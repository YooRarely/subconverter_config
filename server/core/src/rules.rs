use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
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

// 全局预编译常驻规则链状态（启动时一次性加载编译进内存，日常客户端请求与看板查询均为 0ms 纯只读返回）
static COMPILED_STATE: Lazy<RwLock<Option<RulesOverviewResponse>>> = Lazy::new(|| RwLock::new(None));

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

#[derive(Debug, Clone, Serialize)]
pub struct RulesetOverviewItem {
    pub order: usize,
    pub name: String,
    pub group: String,
    pub source_type: String, // "remote" | "local" | "inline"
    pub source: String,
    pub count: usize,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RulesOverviewResponse {
    pub success: bool,
    pub total_rules: usize,
    pub rulesets: Vec<RulesetOverviewItem>,
}

pub struct RulesBuilder;

impl RulesBuilder {
    /// 控制台看板专用：直接从内存获取结构化分流规则文件概览（0ms 纯内存只读）
    pub async fn get_rules_overview() -> RulesOverviewResponse {
        {
            let lock = COMPILED_STATE.read().await;
            if let Some(ref overview) = *lock {
                return overview.clone();
            }
        }

        // 冷启动未预热时自动编译加载一次
        let _ = Self::init_or_reload(false).await;
        let lock = COMPILED_STATE.read().await;
        lock.clone().unwrap_or(RulesOverviewResponse {
            success: true,
            total_rules: 0,
            rulesets: Vec::new(),
        })
    }

    /// 日常请求入口：直接从内存只读获取已预编译好的拍平规则链（0 延迟、0 外部请求、0 磁盘扫描）
    pub async fn build_all_rules(force_refresh: bool) -> Vec<String> {
        if force_refresh {
            return Self::init_or_reload(true).await;
        }

        {
            let lock = COMPILED_STATE.read().await;
            if let Some(ref overview) = *lock {
                return overview.rulesets.iter().flat_map(|rs| rs.rules.clone()).collect();
            }
        }

        // 若尚未初始化 (如冷启动第一次请求)，执行首次编译加载
        Self::init_or_reload(false).await
    }

    /// 核心编译引擎：程序启动时预热加载，或接收到 /refresh 时重新拉取编译
    pub async fn init_or_reload(force_fetch_remote: bool) -> Vec<String> {
        info!("--- 开始编译构建规则链 (强制刷新远程: {}) ---", force_fetch_remote);
        let toml_str = Self::load_rules_toml();
        let config: RulesetConfig = toml::from_str(&toml_str).unwrap_or_else(|e| {
            warn!("解析 rules.toml 失败，回退到内置默认规则配置: {}", e);
            toml::from_str(DEFAULT_RULES_TOML).expect("内置 rules.toml 解析失败")
        });

        let mut overview_items = Vec::new();
        let mut total = 0;

        for (idx, item) in config.rulesets.iter().enumerate() {
            let target_group = item.group.clone();
            let ruleset_val = item.ruleset.trim();

            if let Some(inline) = ruleset_val.strip_prefix("[]") {
                // 1. 内联原生规则：例如 []GEOIP,CN,no-resolve 或 []MATCH / []FINAL
                let inline = inline.trim();
                if inline.is_empty() {
                    continue;
                }
                let rule_str = if inline.eq_ignore_ascii_case("FINAL") {
                    format!("MATCH,{}", target_group)
                } else if let Some(prefix) = inline.strip_suffix(",no-resolve") {
                    format!("{},{},no-resolve", prefix.trim(), target_group)
                } else {
                    format!("{},{}", inline, target_group)
                };

                let name = if rule_str.starts_with("GEOIP") {
                    "内联原生规则 · GEOIP 国内直连".to_string()
                } else {
                    "终极兜底规则 · MATCH".to_string()
                };

                overview_items.push(RulesetOverviewItem {
                    order: idx + 1,
                    name,
                    group: target_group,
                    source_type: "inline".to_string(),
                    source: ruleset_val.to_string(),
                    count: 1,
                    rules: vec![rule_str],
                });
                total += 1;
            } else if ruleset_val.starts_with("http://") || ruleset_val.starts_with("https://") {
                // 2. 远程私人规则 (URL 直接驱动)：从 GitHub 抓取并缓存，绝不扫描本地磁盘
                let content = Self::load_remote_ruleset(ruleset_val, force_fetch_remote).await;
                let mut parsed_rules = Vec::new();
                Self::append_rules(&mut parsed_rules, &content, &target_group);
                let count = parsed_rules.len();
                total += count;

                let name = if ruleset_val.contains("China") {
                    "私人直连规则清单 (China.list)".to_string()
                } else if ruleset_val.contains("Global") {
                    "私人代理规则清单 (Global.list)".to_string()
                } else {
                    format!("远程规则清单 ({})", Path::new(ruleset_val).file_name().and_then(|f| f.to_str()).unwrap_or(ruleset_val))
                };

                overview_items.push(RulesetOverviewItem {
                    order: idx + 1,
                    name,
                    group: target_group,
                    source_type: "remote".to_string(),
                    source: ruleset_val.to_string(),
                    count,
                    rules: parsed_rules,
                });
            } else {
                // 3. 不变公共规则 (相对路径)：严格从服务端内部 server/config/ 资产加载
                let content = Self::load_local_public_ruleset(ruleset_val);
                let mut parsed_rules = Vec::new();
                Self::append_rules(&mut parsed_rules, &content, &target_group);
                let count = parsed_rules.len();
                total += count;

                let name = if ruleset_val.contains("AdRule") {
                    "广告拦截过滤清单 (AdRule.list)".to_string()
                } else if ruleset_val.contains("LocalAreaNetwork") {
                    "局域网直连清单 (LocalAreaNetwork.list)".to_string()
                } else if ruleset_val.contains("China") {
                    "公共国内直连清单 (China.list)".to_string()
                } else if ruleset_val.contains("Global") {
                    "公共国外代理清单 (Global.list)".to_string()
                } else {
                    format!("公共规则清单 ({})", ruleset_val)
                };

                overview_items.push(RulesetOverviewItem {
                    order: idx + 1,
                    name,
                    group: target_group,
                    source_type: "local".to_string(),
                    source: ruleset_val.to_string(),
                    count,
                    rules: parsed_rules,
                });
            }
        }

        let all_rules: Vec<String> = overview_items.iter().flat_map(|rs| rs.rules.clone()).collect();
        info!("✅ 规则链编译构建完毕，有效规则共 {} 条 ({} 个规则集)", all_rules.len(), overview_items.len());

        let overview_resp = RulesOverviewResponse {
            success: true,
            total_rules: total,
            rulesets: overview_items,
        };

        // 原子写入全局常驻内存状态
        {
            let mut write_lock = COMPILED_STATE.write().await;
            *write_lock = Some(overview_resp);
        }

        all_rules
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
