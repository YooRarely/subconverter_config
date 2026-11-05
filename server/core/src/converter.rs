use anyhow::{Context, Result, anyhow};
use serde_yaml_ng::Value;
use tracing::info;

use crate::classifier::{Classifier, MERGE_GROUP_NAME, UNKNOWN_GROUP_NAME};
use crate::rules::RulesBuilder;

pub struct Converter {
    classifier: Classifier,
}

impl Converter {
    pub fn new() -> Self {
        Self {
            classifier: Classifier::new(),
        }
    }

    /// 核心转换函数：传入原始订阅返回的 YAML 文本，返回重构 proxy-groups 与 rules 后的 YAML 文本
    pub async fn convert(&self, raw_yaml: &str, force_refresh_rules: bool) -> Result<String> {
        let mut doc: Value = serde_yaml_ng::from_str(raw_yaml)
            .context("解析原始配置 YAML 失败，请检查输入格式")?;

        let map = doc
            .as_mapping_mut()
            .ok_or_else(|| anyhow!("YAML 顶级结构必须为 Mapping (键值对字典)"))?;

        // 1. 检查并提取 proxies 节点列表中的名称（绝对不修改 proxies 节点本身的内容）
        let proxies_key = Value::String("proxies".to_string());
        let proxies_seq = map
            .get(&proxies_key)
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| anyhow!("配置中未找到有效的 proxies 节点列表"))?;

        let mut node_names = Vec::new();
        for p in proxies_seq {
            if let Some(name) = p
                .get(&Value::String("name".to_string()))
                .and_then(|v| v.as_str())
            {
                node_names.push(name.to_string());
            }
        }

        info!("提取到 {} 个代理节点", node_names.len());

        // 2. 根据名称进行国家分类
        let grouped = self.classifier.group_nodes(&node_names);

        // 3. 构建新的 proxy-groups
        let mut groups_seq: Vec<Value> = Vec::new();

        // 收集所有被纳入主选择组的策略组名
        let mut merge_subgroups: Vec<Value> = Vec::new();

        // 遍历所有有节点的国家组
        for (group_name, nodes) in &grouped.country_groups {
            merge_subgroups.push(Value::String(group_name.clone()));

            let mut country_group = serde_yaml_ng::Mapping::new();
            country_group.insert(
                Value::String("name".to_string()),
                Value::String(group_name.clone()),
            );
            country_group.insert(
                Value::String("type".to_string()),
                Value::String("url-test".to_string()),
            );
            country_group.insert(
                Value::String("url".to_string()),
                Value::String("http://www.gstatic.com/generate_204".to_string()),
            );
            country_group.insert(
                Value::String("interval".to_string()),
                Value::Number(300.into()),
            );
            country_group.insert(
                Value::String("tolerance".to_string()),
                Value::Number(50.into()),
            );
            country_group.insert(
                Value::String("lazy".to_string()),
                Value::Bool(true),
            );

            let node_seq: Vec<Value> = nodes
                .iter()
                .map(|n| Value::String(n.clone()))
                .collect();
            country_group.insert(
                Value::String("proxies".to_string()),
                Value::Sequence(node_seq),
            );

            groups_seq.push(Value::Mapping(country_group));
        }

        // 处理未知国家组（只有在存在未知节点时才生成）
        if !grouped.unknown_nodes.is_empty() {
            merge_subgroups.push(Value::String(UNKNOWN_GROUP_NAME.to_string()));

            let mut unknown_group = serde_yaml_ng::Mapping::new();
            unknown_group.insert(
                Value::String("name".to_string()),
                Value::String(UNKNOWN_GROUP_NAME.to_string()),
            );
            unknown_group.insert(
                Value::String("type".to_string()),
                Value::String("select".to_string()),
            );

            let unknown_seq: Vec<Value> = grouped
                .unknown_nodes
                .iter()
                .map(|n| Value::String(n.clone()))
                .collect();
            unknown_group.insert(
                Value::String("proxies".to_string()),
                Value::Sequence(unknown_seq),
            );

            groups_seq.push(Value::Mapping(unknown_group));
        }

        // 构建 🚀 节点选择 主组
        let mut merge_group = serde_yaml_ng::Mapping::new();
        merge_group.insert(
            Value::String("name".to_string()),
            Value::String(MERGE_GROUP_NAME.to_string()),
        );
        merge_group.insert(
            Value::String("type".to_string()),
            Value::String("select".to_string()),
        );
        merge_group.insert(
            Value::String("proxies".to_string()),
            Value::Sequence(merge_subgroups),
        );

        // 将主组插入在 proxy-groups 的第一个位置
        groups_seq.insert(0, Value::Mapping(merge_group));

        // 替换/插入 proxy-groups 字段
        map.insert(
            Value::String("proxy-groups".to_string()),
            Value::Sequence(groups_seq),
        );

        // 4. 重写 rules 规则列表
        let all_rules = RulesBuilder::build_all_rules(force_refresh_rules).await;
        let rules_seq: Vec<Value> = all_rules
            .into_iter()
            .map(Value::String)
            .collect();

        // 替换/插入 rules 字段
        map.insert(
            Value::String("rules".to_string()),
            Value::Sequence(rules_seq),
        );

        // 确保 mode 字段为 rule
        let mode_key = Value::String("mode".to_string());
        if !map.contains_key(&mode_key) {
            map.insert(mode_key, Value::String("rule".to_string()));
        }

        // 5. 序列化回 YAML 文本并输出
        let output_yaml = serde_yaml_ng::to_string(&doc)
            .context("序列化转换后的配置为 YAML 失败")?;

        Ok(output_yaml)
    }
}
