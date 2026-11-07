use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

// 内置基础规则列表与 TOML 配置（公共保底，完全自包含）
const DEFAULT_RULES_TOML: &str = include_str!("../../config/rules.toml");
const DEFAULT_AD_RULE: &str = include_str!("../../config/rules/AdRule.list");
const DEFAULT_LAN_RULE: &str = include_str!("../../config/rules/LocalAreaNetwork.list");
const DEFAULT_CORE_GLOBAL: &str = include_str!("../../config/rules/Global.list");
const DEFAULT_CORE_CHINA: &str = include_str!("../../config/rules/China.list");

// 远程规则原文常驻内存缓存：URL -> String
static REMOTE_CACHE: Lazy<RwLock<HashMap<String, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

// 全局预编译常驻规则链（启动时一次性加载编译进内存，日常客户端请求 0ms 纯只读返回）
static COMPILED_RULES: Lazy<RwLock<Vec<String>>> = Lazy::new(|| RwLock::new(Vec::new()));

#[derive(Debug, Deserialize)]
pub struct RulesetConfig {
    #[serde(default)]
    pub rulesets: Vec<RulesetItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RulesetItem {
    pub group: String,
    pub ruleset: String,
    #[serde(default)]
    pub remote: Option<String>,
}

pub struct RulesBuilder;

impl RulesBuilder {
    /// 日常请求入口：直接从内存只读获取已预编译好的规则链（0 延迟、0 外部请求、0 磁盘扫描）
    pub async fn build_all_rules(force_refresh: bool) -> Vec<String> {
        if force_refresh {
            return Self::init_or_reload(true).await;
        }

        {
            let rules_lock = COMPILED_RULES.read().await;
            if !rules_lock.is_empty() {
                return rules_lock.clone();
            }
        }

        // 若尚未初始化 (如冷启动第一次请求)，执行首次编译加载
        Self::init_or_reload(false).await
    }

    /// 核心编译引擎：程序启动时预热加载，或接收到 /refresh 时重新拉取编译
    pub async fn init_or_reload(force_fetch_remote: bool) -> Vec<String> {
        info!("--- 开始编译构建规则链 (强制刷新远程: {}) ---", force_fetch_remote);
        let mut rules = Vec::new();

        let toml_str = Self::load_rules_toml();
        let config: RulesetConfig = toml::from_str(&toml_str).unwrap_or_else(|e| {
            warn!("解析 rules.toml 失败，回退到内置默认规则配置: {}", e);
            toml::from_str(DEFAULT_RULES_TOML).expect("内置 rules.toml 解析失败")
        });

        for item in &config.rulesets {
            let target_group = &item.group;
            let ruleset_val = item.ruleset.trim();

            if let Some(inline) = ruleset_val.strip_prefix("[]") {
                // 1. 内联原生规则：例如 []GEOIP,CN,no-resolve 或 []MATCH / []FINAL
                let inline = inline.trim();
                if inline.is_empty() {
                    continue;
                }
                let inline = if inline.eq_ignore_ascii_case("FINAL") {
                    "MATCH"
                } else {
                    inline
                };

                if let Some(prefix) = inline.strip_suffix(",no-resolve") {
                    rules.push(format!("{},{},no-resolve", prefix.trim(), target_group));
                } else {
                    rules.push(format!("{},{}", inline, target_group));
                }
            } else if ruleset_val.starts_with("http://") || ruleset_val.starts_with("https://") {
                // 2. 远程私人规则 (URL 直接驱动)：从 GitHub 抓取并缓存，绝不扫描本地磁盘
                let content = Self::load_remote_ruleset(ruleset_val, force_fetch_remote).await;
                Self::append_rules(&mut rules, &content, target_group);
            } else if let Some(ref remote_url) = item.remote {
                // 2.2 兼容配置了 remote 的相对路径规则：优先走 remote 抓取
                let content = Self::load_remote_ruleset(remote_url, force_fetch_remote).await;
                Self::append_rules(&mut rules, &content, target_group);
            } else {
                // 3. 不变公共规则 (相对路径)：严格从服务端内部 server/config/ 资产加载
                let content = Self::load_local_public_ruleset(ruleset_val);
                Self::append_rules(&mut rules, &content, target_group);
            }
        }

        info!("✅ 规则链编译构建完毕，有效规则共 {} 条", rules.len());

        // 原子写入全局内存缓存
        {
            let mut write_lock = COMPILED_RULES.write().await;
            *write_lock = rules.clone();
        }

        rules
    }

    /// 触发刷新接口专用：重新从 GitHub 拉取编译，并覆写全局内存规则链
    pub async fn refresh_private_rules() -> (bool, bool) {
        info!("--- 收到 /refresh 请求：重新从 GitHub 拉取私人规则并重新编译规则链 ---");
        let rules = Self::init_or_reload(true).await;
        (!rules.is_empty(), true)
    }

    fn load_rules_toml() -> String {
        let candidates = [
            "config/rules.toml",
            "server/config/rules.toml",
            "../config/rules.toml",
            "../../config/rules.toml",
        ];
        for p in &candidates {
            if let Ok(c) = std::fs::read_to_string(p) {
                return c;
            }
        }
        DEFAULT_RULES_TOML.to_string()
    }

    /// 抓取远程规则清单（带内存常驻缓存与网络容灾保底，绝不扫描本地磁盘）
    async fn load_remote_ruleset(url: &str, force_fetch: bool) -> String {
        if !force_fetch {
            let cache = REMOTE_CACHE.read().await;
            if let Some(text) = cache.get(url) {
                return text.clone();
            }
        }

        if Self::fetch_and_cache(url).await {
            let cache = REMOTE_CACHE.read().await;
            if let Some(text) = cache.get(url) {
                return text.clone();
            }
        }

        // 容灾：网络暂时超时或离线时使用已有的旧缓存
        {
            let cache = REMOTE_CACHE.read().await;
            if let Some(text) = cache.get(url) {
                return text.clone();
            }
        }

        // 终极静态编译期兜底
        if url.contains("China") {
            DEFAULT_CORE_CHINA.to_string()
        } else if url.contains("Global") {
            DEFAULT_CORE_GLOBAL.to_string()
        } else {
            String::new()
        }
    }

    /// 从服务端内部 server/config/ 目录读取不变的公共规则资产（绝不向上越界查找）
    fn load_local_public_ruleset(file_path: &str) -> String {
        let paths_to_try = [
            format!("server/config/{}", file_path),
            format!("config/{}", file_path),
            file_path.to_string(),
        ];

        for p in &paths_to_try {
            if Path::new(p).exists() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    return content;
                }
            }
        }

        // 静态编译期公共兜底
        if file_path.contains("AdRule") {
            DEFAULT_AD_RULE.to_string()
        } else if file_path.contains("LocalAreaNetwork") {
            DEFAULT_LAN_RULE.to_string()
        } else if file_path.contains("China") {
            DEFAULT_CORE_CHINA.to_string()
        } else if file_path.contains("Global") {
            DEFAULT_CORE_GLOBAL.to_string()
        } else {
            String::new()
        }
    }

    async fn fetch_and_cache(remote_url: &str) -> bool {
        // 破除 GitHub Fastly CDN 的 300 秒 (5分钟) 强行缓存延迟：附带时间戳并禁用缓存回源
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let bust_url = if remote_url.contains('?') {
            format!("{}&_t={}", remote_url, timestamp)
        } else {
            format!("{}?_t={}", remote_url, timestamp)
        };

        info!("正在从远程 GitHub 拉取私人规则 (实时回源防缓存): {}", bust_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let req = client
            .get(&bust_url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
            .header(reqwest::header::PRAGMA, "no-cache");

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    let mut cache = REMOTE_CACHE.write().await;
                    cache.insert(remote_url.to_string(), text);
                    info!("✅ 成功同步并缓存远程规则 (实时生效): {}", remote_url);
                    return true;
                }
            }
            Ok(resp) => {
                warn!("从远程拉取规则返回异常状态码 ({}): {}", resp.status(), remote_url);
            }
            Err(e) => {
                warn!("从远程拉取规则网络失败: {} ({})", remote_url, e);
            }
        }
        false
    }

    fn append_rules(dest: &mut Vec<String>, raw_content: &str, policy: &str) {
        const SUPPORTED_TYPES: &[&str] = &[
            "DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD", "DOMAIN-REGEX",
            "GEOSITE", "GEOIP", "IP-CIDR", "IP-CIDR6", "SRC-GEOIP",
            "SRC-IP-CIDR", "SRC-PORT", "DST-PORT", "IN-PORT", "IN-TYPE",
            "IN-USER", "IN-NAME", "PROCESS-NAME", "PROCESS-PATH", "MATCH", "FINAL",
        ];

        for line in raw_content.lines() {
            let mut line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }

            if line.starts_with("- ") {
                line = line[2..].trim();
            }

            if line.starts_with("URL-REGEX,") {
                if let Some(domain_regex) = Self::convert_url_regex(&line[10..]) {
                    dest.push(format!("DOMAIN-REGEX,{},{}", domain_regex, policy));
                }
                continue;
            }

            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.is_empty() {
                continue;
            }

            let rule_type = parts[0].to_uppercase();
            if !SUPPORTED_TYPES.contains(&rule_type.as_str()) {
                continue;
            }

            if let Some(prefix) = line.strip_suffix(",no-resolve") {
                dest.push(format!("{},{},no-resolve", prefix.trim(), policy));
            } else {
                dest.push(format!("{},{}", line, policy));
            }
        }
    }

    fn convert_url_regex(raw_pattern: &str) -> Option<String> {
        let mut s = raw_pattern.trim();
        if s.starts_with('^') {
            s = &s[1..];
        }

        for proto in &[
            "https?:\\/\\/", "http?:\\/\\/", "https:\\/\\/", "http:\\/\\/",
            "https?://", "http?://", "https://", "http://",
        ] {
            if s.starts_with(proto) {
                s = &s[proto.len()..];
                break;
            }
        }

        let mut end_idx = s.len();
        for sep in &["\\/", "/"] {
            if let Some(pos) = s.find(sep) {
                if pos < end_idx {
                    end_idx = pos;
                }
            }
        }

        let domain_part = s[..end_idx].trim();
        if domain_part.is_empty() || domain_part == ".+" || domain_part == ".*" {
            None
        } else {
            Some(domain_part.to_string())
        }
    }
}
