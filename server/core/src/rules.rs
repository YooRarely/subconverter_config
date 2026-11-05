use once_cell::sync::Lazy;
use std::path::Path;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

// 内置基础规则列表（万年不变的公共底库，完全自包含）
const DEFAULT_PRIVATE_CHINA: &str = "";
const DEFAULT_PRIVATE_GLOBAL: &str = "";
const DEFAULT_AD_RULE: &str = include_str!("../rules/default/AdRule.list");
const DEFAULT_LAN_RULE: &str = include_str!("../rules/default/LocalAreaNetwork.list");
const DEFAULT_CORE_GLOBAL: &str = include_str!("../rules/default/Global.list");
const DEFAULT_CORE_CHINA: &str = include_str!("../rules/default/China.list");

// 远程 GitHub 私人规则地址（用于 Render 仅部署 server2 目录时动态拉取最新规则）
pub const REMOTE_CHINA_URL: &str =
    "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/China.list";
pub const REMOTE_GLOBAL_URL: &str =
    "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/Global.list";

// 私人规则常驻内存缓存（平时永远常驻内存，零网络开销；仅在显式触发刷新时重新拉取）
static CHINA_CACHE: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static GLOBAL_CACHE: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));

pub struct RulesBuilder;

impl RulesBuilder {
    /// 加载并生成全部 Clash 规则列表
    /// force_refresh 为 true 时会强制穿透缓存从 GitHub 重新拉取私人规则
    pub async fn build_all_rules(force_refresh: bool) -> Vec<String> {
        let mut rules = Vec::new();

        // 1. 私人【直连】规则清单 (China.list) -> 最高优先级
        // 本地优先读本地文件；若在 Render 环境中无本地文件，则常驻内存缓存，收到刷新信号时实时拉取
        let china_rules = Self::load_private_rule(
            "rules/China.list",
            REMOTE_CHINA_URL,
            DEFAULT_PRIVATE_CHINA,
            &CHINA_CACHE,
            force_refresh,
        )
        .await;
        Self::append_rules(&mut rules, &china_rules, "DIRECT");

        // 2. 私人【代理】规则清单 (Global.list) -> 最高优先级
        let global_rules = Self::load_private_rule(
            "rules/Global.list",
            REMOTE_GLOBAL_URL,
            DEFAULT_PRIVATE_GLOBAL,
            &GLOBAL_CACHE,
            force_refresh,
        )
        .await;
        Self::append_rules(&mut rules, &global_rules, "🚀 节点选择");

        // 3. 广告拦截规则清单 (default/AdRule.list) -> REJECT (静态大文件，内置极速加载)
        let ad_rules = Self::load_file_or_default("rules/default/AdRule.list", DEFAULT_AD_RULE);
        Self::append_rules(&mut rules, &ad_rules, "REJECT");

        // 4. 局域网直连 (default/LocalAreaNetwork.list) -> DIRECT
        let lan_rules =
            Self::load_file_or_default("rules/default/LocalAreaNetwork.list", DEFAULT_LAN_RULE);
        Self::append_rules(&mut rules, &lan_rules, "DIRECT");

        // 5. 核心分流：被墙国外服务 (default/Global.list) -> 🚀 节点选择
        let core_global =
            Self::load_file_or_default("rules/default/Global.list", DEFAULT_CORE_GLOBAL);
        Self::append_rules(&mut rules, &core_global, "🚀 节点选择");

        // 6. 国内直连 (default/China.list) -> DIRECT
        let core_china = Self::load_file_or_default("rules/default/China.list", DEFAULT_CORE_CHINA);
        Self::append_rules(&mut rules, &core_china, "DIRECT");

        // 7. GEOIP 国内直连
        rules.push("GEOIP,CN,DIRECT,no-resolve".to_string());

        // 8. 终极兜底走主选择组
        rules.push("MATCH,🚀 节点选择".to_string());

        info!("规则链已生成完毕，有效规则共 {} 条", rules.len());
        rules
    }

    /// 主动触发私人规则刷新：立刻从 GitHub 重新拉取最新 China.list 和 Global.list 覆盖内存缓存
    pub async fn refresh_private_rules() -> (bool, bool) {
        info!("--- 触发主动刷新：正在从 GitHub 拉取最新私人规则 ---");
        let china_ok = Self::fetch_and_cache(REMOTE_CHINA_URL, &CHINA_CACHE).await;
        let global_ok = Self::fetch_and_cache(REMOTE_GLOBAL_URL, &GLOBAL_CACHE).await;
        (china_ok, global_ok)
    }

    /// 辅助方法：读取高频变动的私人规则
    async fn load_private_rule(
        file_path: &str,
        remote_url: &str,
        default_builtin: &'static str,
        cache_lock: &RwLock<Option<String>>,
        force_refresh: bool,
    ) -> String {
        // 1. 本地优先：检查本地磁盘（若在本地运行，直接读取本地文件，不需要网络）
        let paths_to_try = [
            file_path.to_string(),
            format!("core/{}", file_path),
            format!("../{}", file_path),
            format!("../core/{}", file_path),
        ];

        for p in &paths_to_try {
            if Path::new(p).exists() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    return content;
                }
            }
        }

        // 2. 本地不存在（Render 云端环境）：
        // 若非强制刷新，优先直接返回常驻内存中的缓存（0毫秒，零网络开销）
        if !force_refresh {
            let cache = cache_lock.read().await;
            if let Some(ref text) = *cache {
                return text.clone();
            }
        }

        // 3. 缓存为空或强制刷新：从 GitHub 重新拉取
        if Self::fetch_and_cache(remote_url, cache_lock).await {
            let cache = cache_lock.read().await;
            if let Some(ref text) = *cache {
                return text.clone();
            }
        }

        // 4. 若远程请求失败但曾有旧缓存，使用旧缓存保底
        {
            let cache = cache_lock.read().await;
            if let Some(ref text) = *cache {
                return text.clone();
            }
        }

        // 5. 终极保底：内置编译期数据
        default_builtin.to_string()
    }

    /// 从 GitHub 下载并覆盖写入内存缓存
    async fn fetch_and_cache(remote_url: &str, cache_lock: &RwLock<Option<String>>) -> bool {
        info!("正在从远程 GitHub 拉取: {}", remote_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match client.get(remote_url).send().await {
            Ok(res) if res.status().is_success() => {
                if let Ok(text) = res.text().await {
                    let mut cache = cache_lock.write().await;
                    *cache = Some(text);
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

    /// 辅助方法：读取万年不变的基础大规则集（本地优先 -> 内置保底）
    fn load_file_or_default(file_path: &str, default_content: &'static str) -> String {
        let paths_to_try = [
            file_path.to_string(),
            format!("core/{}", file_path),
            format!("../{}", file_path),
            format!("../core/{}", file_path),
        ];

        for p in &paths_to_try {
            if Path::new(p).exists() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    return content;
                }
            }
        }

        default_content.to_string()
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
