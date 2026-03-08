
use reqwest;
use serde_yaml_ng::Value;
use tracing::{error};

use crate::{env, store};
/// 主入口：传入原始配置，返回修改后的配置
pub async fn inject(mut config: Value) -> Value {
	
    // let direct_rules = env::var("DIRECT_RULES").unwrap_or_else(|_| "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/China.list".into());
    // let global_rules = env::var("GLOBAL_RULES").unwrap_or_else(|_| "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/Global.list".into());
    let rules_vec = RuleFetcher::new()
        .add_task(env::DIRECT_RULES, "DIRECT")
		.add_task(env::GLOBAL_RULES, "🚀 节点选择")
        .collect()
        .await;
	insert_yaml(&mut config,&rules_vec);
    config
}
async fn fetch_rules(url: &str, policy: &str) -> Option<std::sync::Arc<Vec<String>>> {
    let response = store::cache().await
        .get_or_set_async::<Vec<String>>((format!("rules::{}", url).as_str(), None), || async move {
            let client = reqwest::Client::new();
            let response = match client.get(url).send().await {
                Ok(res) => res.text().await.unwrap_or_default(),
                Err(e) => {
                    error!("下载规则失败: {}", e);
                    return None;
                }
            };
            let new_rules_raw: Vec<String> = response
                .lines()
                .map(|line| line.trim())
                // 过滤：去掉注释行 (#) 和空行
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                // 拼接策略：例如 "DOMAIN-KEYWORD,mihoyo" -> "DOMAIN-KEYWORD,mihoyo,DIRECT"
                .map(|line| format!("{},{}", line, policy))
                .collect();
            if new_rules_raw.is_empty() {
                return None;
            }
            Some(new_rules_raw)
        })
        .await;
    response
}

fn insert_yaml(config: &mut Value, new_rules_raw: &Vec<String>) {
    let mut processed_rules: Vec<Value> = new_rules_raw
        .into_iter()
        .map(|s| Value::String(s.clone()))
        .collect();
    if let Some(rules_seq) = config.get_mut("rules").and_then(|v| v.as_sequence_mut()) {
        // 置顶：新规则在前，原规则在后
        processed_rules.extend(rules_seq.drain(..));
        *rules_seq = processed_rules;
    } else
    // 如果原配置没 rules 字段，直接新建
    if let Some(map) = config.as_mapping_mut() {
        map.insert(
            Value::String("rules".to_string()),
            Value::Sequence(processed_rules),
        );
    }
}
struct RuleFetcher {
    tasks: Vec<tokio::task::JoinHandle<Option<std::sync::Arc<Vec<String>>>>>,
}

impl RuleFetcher {
    fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    // 立即启动异步任务，不阻塞
    fn add_task(mut self, url: impl Into<String>, policy: impl Into<String>) -> Self {
        let (url, policy) = (url.into(), policy.into());
        let handle = tokio::spawn(async move {
            // 在异步闭包内部调用你原本的 fetch_rules
            fetch_rules(&url, &policy).await
        });
        self.tasks.push(handle);
        self
    }

    // 最后统一等待
    async fn collect(self) -> Vec<String> {
        let mut all_results = Vec::new();
        for handle in self.tasks {
            if let Ok(Some(rules_arc)) = handle.await {
                all_results.extend(rules_arc.iter().cloned());
            }
        }
        all_results
    }
}
