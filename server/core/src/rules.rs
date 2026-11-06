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

// 远程规则常驻内存缓存：remote_url -> String
static REMOTE_CACHE: Lazy<RwLock<HashMap<String, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct RulesetConfig {
    #[serde(default)]
    pub rulesets: Vec<RulesetItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RulesetItem {
    pub group: String,
    pub ruleset: String,
    pub remote: Option<String>,
}

pub struct RulesBuilder;

impl RulesBuilder {
    /// 加载并生成全部 Clash 规则列表（根据 server/config/rules.toml 配置的顺序动态构建）
    /// force_refresh 为 true 时会强制穿透缓存从 GitHub 重新拉取远程规则
    pub async fn build_all_rules(force_refresh: bool) -> Vec<String> {
        let mut rules = Vec::new();

        // 1. 读取并解析 rules.toml
        let toml_str = Self::load_rules_toml();
        let config: RulesetConfig = toml::from_str(&toml_str).unwrap_or_else(|e| {
            warn!("解析 rules.toml 失败，回退到内置默认规则配置: {}", e);
            toml::from_str(DEFAULT_RULES_TOML).expect("内置 rules.toml 解析失败")
        });

        for item in &config.rulesets {
            let target_group = &item.group;
            let ruleset_val = item.ruleset.trim();

            if let Some(inline) = ruleset_val.strip_prefix("[]") {
                // 内联原生规则：例如 []GEOIP,CN,no-resolve 或 []MATCH
                let inline = inline.trim();
                if inline.is_empty() {
                    continue;
                }
                if let Some(prefix) = inline.strip_suffix(",no-resolve") {
                    rules.push(format!("{},{},no-resolve", prefix.trim(), target_group));
                } else {
                    rules.push(format!("{},{}", inline, target_group));
                }
            } else {
                // 文件规则清单
                let content = Self::load_ruleset_content(
                    ruleset_val,
                    item.remote.as_deref(),
                    force_refresh,
                )
                .await;

                Self::append_rules(&mut rules, &content, target_group);
            }
        }

        info!("规则链已根据 rules.toml 生成完毕，有效规则共 {} 条", rules.len());
        rules
    }

    /// 主动触发远程规则刷新：重新从 GitHub 拉取 rules.toml 中所有定义了 remote 地址的规则
    pub async fn refresh_private_rules() -> (bool, bool) {
        info!("--- 触发主动刷新：正在从 GitHub 拉取最新私人规则 ---");
        let toml_str = Self::load_rules_toml();
        let config: RulesetConfig = toml::from_str(&toml_str).unwrap_or_else(|_| {
            toml::from_str(DEFAULT_RULES_TOML).unwrap()
        });

        let mut china_ok = false;
        let mut global_ok = false;

        for item in config.rulesets {
            if let Some(ref remote_url) = item.remote {
                let ok = Self::fetch_and_cache(remote_url).await;
                if item.ruleset.contains("China") {
                    china_ok = china_ok || ok;
                }
                if item.ruleset.contains("Global") {
                    global_ok = global_ok || ok;
                }
            }
        }

        (china_ok, global_ok)
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

    async fn load_ruleset_content(
        file_path: &str,
        remote_url_opt: Option<&str>,
        force_refresh: bool,
    ) -> String {
        // 1. 本地磁盘优先探查
        let filename = Path::new(file_path).file_name().and_then(|f| f.to_str()).unwrap_or(file_path);
        let paths_to_try = [
            file_path.to_string(),
            format!("config/{}", file_path),
            format!("server/config/{}", file_path),
            format!("server/config/rules/{}", filename),
            format!("config/rules/{}", filename),
            format!("../config/rules/{}", filename),
            format!("../../config/rules/{}", filename),
            format!("rules/{}", filename),
            format!("../rules/{}", filename),
        ];

        for p in &paths_to_try {
            if Path::new(p).exists() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    return content;
                }
            }
        }

        // 2. 远程拉取 (如 Render 环境无本地磁盘文件)
        if let Some(remote_url) = remote_url_opt {
            if !force_refresh {
                let cache = REMOTE_CACHE.read().await;
                if let Some(text) = cache.get(remote_url) {
                    return text.clone();
                }
            }

            if Self::fetch_and_cache(remote_url).await {
                let cache = REMOTE_CACHE.read().await;
                if let Some(text) = cache.get(remote_url) {
                    return text.clone();
                }
            }

            // 旧缓存保底
            {
                let cache = REMOTE_CACHE.read().await;
                if let Some(text) = cache.get(remote_url) {
                    return text.clone();
                }
            }
        }

        // 3. 内置静态兜底
        if filename.contains("AdRule") {
            DEFAULT_AD_RULE.to_string()
        } else if filename.contains("LocalAreaNetwork") {
            DEFAULT_LAN_RULE.to_string()
        } else if filename.contains("China") {
            DEFAULT_CORE_CHINA.to_string()
        } else if filename.contains("Global") {
            DEFAULT_CORE_GLOBAL.to_string()
        } else {
            String::new()
        }
    }

    async fn fetch_and_cache(remote_url: &str) -> bool {
        info!("正在从远程 GitHub 拉取: {}", remote_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match client.get(remote_url).send().await {
            Ok(res) if res.status().is_success() => {
                if let Ok(text) = res.text().await {
                    let mut cache = REMOTE_CACHE.write().await;
                    cache.insert(remote_url.to_string(), text);
                    info!("成功拉取并覆盖更新了规则缓存: {}", remote_url);
                    return true;
                }
            }
            Ok(res) => {
                warn!("从 GitHub 拉取返回非 200 状态码 ({}): {}", res.status(), remote_url);
            }
            Err(e) => {
                warn!("从 GitHub 拉取网络错误: {} ({})", e, remote_url);
            }
        }
        false
    }

    /// 辅助方法：解析规则文本、过滤非 Clash 规则并追加策略
    fn append_rules(dest: &mut Vec<String>, raw_content: &str, policy: &str) {
        const SUPPORTED_TYPES: &[&str] = &[
            "DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD", "DOMAIN-REGEX",
            "GEOSITE", "GEOIP", "IP-CIDR", "IP-CIDR6", "SRC-GEOIP",
            "SRC-IP-CIDR", "SRC-PORT", "DST-PORT", "IN-PORT", "IN-TYPE",
            "IN-USER", "IN-NAME", "PROCESS-NAME", "PROCESS-PATH", "MATCH", "FINAL",
        ];

        for line in raw_content.lines() {
            let mut line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(pos) = line.find("//") {
                line = line[..pos].trim();
            }
            if line.is_empty() {
                continue;
            }

            let mut parts = line.splitn(2, ',');
            let rule_type = parts.next().unwrap_or("").trim().to_uppercase();
            let body = parts.next().unwrap_or("").trim();

            if rule_type == "URL-REGEX" {
                if let Some(domain_regex) = Self::convert_url_regex(body) {
                    dest.push(format!("DOMAIN-REGEX,{},{}", domain_regex, policy));
                }
                continue;
            }

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