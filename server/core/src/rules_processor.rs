use std::env;

use reqwest;
use serde_yaml_ng::Value;
use tracing::{error, info};

/// 主入口：传入原始配置，返回修改后的配置
pub async fn apply_custom_rules(mut config: Value) -> Value {
    let direct_rules = env::var("DIRECT_RULES").unwrap_or_else(|_| "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/China.list".into());
    let global_rules = env::var("GLOBAL_RULES").unwrap_or_else(|_| "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/Global.list".into());
    // 你可以轻松地在这里添加多组规则
    // 参数：(配置文件对象, GitHub链接, 策略名称)
    config = fetch_and_patch(config, &direct_rules, "DIRECT").await;
    config = fetch_and_patch(config, &global_rules, "🚀 节点选择").await;
    config
}

/// 核心逻辑：下载 list，处理格式，插入到 rules 顶部
async fn fetch_and_patch(mut config: Value, url: &str, policy: &str) -> Value {
    info!("正在从远程获取规则: {}", url);

    let client = reqwest::Client::new();
    let response = match client.get(url).send().await {
        Ok(res) => res.text().await.unwrap_or_default(),
        Err(e) => {
            error!("下载规则失败: {}", e);
            return config; // 失败则返回原配置，不中断程序
        }
    };

    // 1. 解析 List 文件
    let new_rules_raw: Vec<String> = response
        .lines()
        .map(|line| line.trim())
        // 过滤：去掉注释行 (#) 和空行
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        // 拼接策略：例如 "DOMAIN-KEYWORD,mihoyo" -> "DOMAIN-KEYWORD,mihoyo,DIRECT"
        .map(|line| format!("{},{}", line, policy))
        .collect();

    if new_rules_raw.is_empty() {
        return config;
    }

    // 2. 插入到 YAML
    if let Some(rules_seq) = config.get_mut("rules").and_then(|v| v.as_sequence_mut()) {
        let mut processed_rules: Vec<Value> =
            new_rules_raw.into_iter().map(Value::String).collect();

        // 置顶：新规则在前，原规则在后
        processed_rules.extend(rules_seq.drain(..));
        *rules_seq = processed_rules;
    } else {
        // 如果原配置没 rules 字段，直接新建
        if let Some(map) = config.as_mapping_mut() {
            let processed_rules: Vec<Value> =
                new_rules_raw.into_iter().map(Value::String).collect();
            map.insert(
                Value::String("rules".to_string()),
                Value::Sequence(processed_rules),
            );
        }
    }

    config
}
