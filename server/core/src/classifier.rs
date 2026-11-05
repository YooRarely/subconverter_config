use fancy_regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

const DEFAULT_EMOJI_TXT: &str = include_str!("../config/emoji.txt");
const DEFAULT_EMOJI_TOML: &str = include_str!("../config/emoji.toml");

#[derive(Debug, Deserialize)]
struct EmojiTomlConfig {
    #[serde(default)]
    emoji: Vec<EmojiTomlItem>,
}

#[derive(Debug, Deserialize)]
struct EmojiTomlItem {
    r#match: String,
    emoji: String,
}

#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub name: String,
    pub emoji: String,
    pub group_name: String,
}

pub struct CountryRule {
    pub regex: Regex,
    pub emoji: String,
}

pub struct Classifier {
    rules: Vec<CountryRule>,
    emoji_to_name: HashMap<String, String>,
    countries: Vec<CountryInfo>,
}

pub const MERGE_GROUP_NAME: &str = "🚀 节点选择";
pub const UNKNOWN_GROUP_NAME: &str = "❓ 未知";

#[derive(Debug, Clone)]
pub struct GroupedNodes {
    /// 有序的国家节点组：(组名, 该组下的节点名称列表)
    pub country_groups: Vec<(String, Vec<String>)>,
    /// 未知国家的节点列表
    pub unknown_nodes: Vec<String>,
}

impl Classifier {
    /// 从配置文件或内置默认数据初始化分类器
    pub fn new() -> Self {
        // 1. 读取 emoji.txt（优先读取本地文件，失败则使用内置常量）
        let emoji_txt = std::fs::read_to_string("config/emoji.txt")
            .unwrap_or_else(|_| DEFAULT_EMOJI_TXT.to_string());

        let mut countries = Vec::new();
        let mut emoji_to_name = HashMap::new();

        for line in emoji_txt.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains('/') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(2, '/').collect();
            let name = parts[0].trim().to_string();
            let emoji = parts[1].trim().to_string();
            // 关键：在 emoji 与国家名称之间插入零宽空格 \u{200b} (ZWS)
            // 彻底防止策略组名与只有单节点的机场节点名完全同名导致的 Clash 自环死循环 (loop is detected in ProxyGroup)
            let group_name = format!("{}\u{200b} {}", emoji, name);

            emoji_to_name.insert(emoji.clone(), name.clone());
            countries.push(CountryInfo {
                name,
                emoji,
                group_name,
            });
        }

        // 2. 读取 emoji.toml
        let emoji_toml_str = std::fs::read_to_string("config/emoji.toml")
            .unwrap_or_else(|_| DEFAULT_EMOJI_TOML.to_string());

        let toml_data: EmojiTomlConfig = toml::from_str(&emoji_toml_str).unwrap_or_else(|e| {
            warn!("解析 emoji.toml 失败，回退到内置默认规则: {}", e);
            toml::from_str(DEFAULT_EMOJI_TOML).expect("内置 emoji.toml 解析失败")
        });

        let mut rules = Vec::new();
        for item in toml_data.emoji {
            match Regex::new(&item.r#match) {
                Ok(re) => {
                    rules.push(CountryRule {
                        regex: re,
                        emoji: item.emoji,
                    });
                }
                Err(e) => {
                    warn!("编译正则失败: pattern={}, err={}", item.r#match, e);
                }
            }
        }

        info!(
            "国家分类器已加载: {} 个已知地区, {} 条正则规则",
            countries.len(),
            rules.len()
        );

        Self {
            rules,
            emoji_to_name,
            countries,
        }
    }

    /// 根据节点名称判断属于哪个 Emoji（若未知则返回 None）
    pub fn classify_emoji(&self, node_name: &str) -> Option<String> {
        // 1. 优先按照规则列表进行正则匹配（按顺序）
        for rule in &self.rules {
            if let Ok(matched) = rule.regex.is_match(node_name) {
                if matched {
                    return Some(rule.emoji.clone());
                }
            }
        }

        // 2. 检查节点名称本身是否直接包含已知国家的 Emoji
        for country in &self.countries {
            if node_name.contains(&country.emoji) {
                return Some(country.emoji.clone());
            }
        }

        None
    }

    /// 对节点列表进行分组
    pub fn group_nodes(&self, node_names: &[String]) -> GroupedNodes {
        // 记录每个 emoji 对应的节点列表
        let mut emoji_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut unknown_nodes: Vec<String> = Vec::new();

        for name in node_names {
            if let Some(emoji) = self.classify_emoji(name) {
                emoji_map.entry(emoji).or_default().push(name.clone());
            } else {
                unknown_nodes.push(name.clone());
            }
        }

        // 按照 countries 的预定顺序构建分组
        let mut country_groups = Vec::new();
        for country in &self.countries {
            if let Some(nodes) = emoji_map.remove(&country.emoji) {
                if !nodes.is_empty() {
                    country_groups.push((country.group_name.clone(), nodes));
                }
            }
        }

        // 处理剩余可能匹配到了 emoji 但不在 countries 列表里的情况（容错）
        for (emoji, nodes) in emoji_map {
            if !nodes.is_empty() {
                let name = self
                    .emoji_to_name
                    .get(&emoji)
                    .cloned()
                    .unwrap_or_else(|| "节点".to_string());
                let group_name = format!("{}\u{200b} {}", emoji, name);
                country_groups.push((group_name, nodes));
            }
        }

        GroupedNodes {
            country_groups,
            unknown_nodes,
        }
    }
}
