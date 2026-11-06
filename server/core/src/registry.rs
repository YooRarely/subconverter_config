use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::store;

const PREFIX: &str = "subscribe:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionItem {
    pub key: String,
    pub url: String,               // 机场订阅原链接
    #[serde(default)]
    pub raw_yaml: String,          // 手动缓存的真节点 YAML 底包 (非空且 is_manual_cached 时直接下发)
    #[serde(default)]
    pub user_info: String,         // 流量信息 (subscription-userinfo: upload=...; download=...; total=...)
    #[serde(default)]
    pub updated_at: u64,           // 上次更新时间戳 (秒)
    #[serde(default)]
    pub node_count: usize,         // 节点个数
    #[serde(default)]
    pub is_manual_cached: bool,    // 显式标志：是否开启了手动真节点快照模式 (模式 B)
    #[serde(default)]
    pub last_status: String,       // 运行状态: "ok" 或具体错误 "error: ..."
    #[serde(default)]
    pub mode: String,              // 兼容旧字段
}

impl SubscriptionItem {
    /// 是否为手动快照直发模式
    pub fn is_snapshot_mode(&self) -> bool {
        self.is_manual_cached && !self.raw_yaml.trim().is_empty()
    }
}

/// 将原始 YAML 清洗为只保留 proxies 和顶级基础字段，剔除旧的 proxy-groups 和 rules
pub fn clean_raw_yaml(full_yaml: &str) -> Result<(String, usize), String> {
    let mut doc: Value = serde_yaml_ng::from_str(full_yaml)
        .map_err(|e| format!("YAML 格式解析失败: {}", e))?;

    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| "YAML 顶级结构必须为字典 (Mapping)".to_string())?;

    // 剔除旧的 proxy-groups 和 rules
    map.remove(&Value::String("proxy-groups".to_string()));
    map.remove(&Value::String("rules".to_string()));

    // 统计节点数量
    let node_count = map
        .get(&Value::String("proxies".to_string()))
        .and_then(|v| v.as_sequence())
        .map(|s| s.len())
        .unwrap_or(0);

    if node_count == 0 {
        return Err("配置中未找到有效的 proxies 节点列表或节点数为 0".to_string());
    }

    let cleaned_yaml = serde_yaml_ng::to_string(&doc)
        .map_err(|e| format!("导出纯净 YAML 失败: {}", e))?;

    Ok((cleaned_yaml, node_count))
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 保存或更新订阅项
pub async fn save_subscription(mut item: SubscriptionItem) {
    let cache_key = format!("{}{}", PREFIX, item.key);
    item.updated_at = current_timestamp();

    if let Ok(json_str) = serde_json::to_string(&item) {
        store::cache().await.set(&cache_key, json_str, None).await;
        info!(
            "订阅配置已保存 (key={}, 快照模式={}, 节点数={})",
            item.key,
            item.is_snapshot_mode(),
            item.node_count
        );
    }
}

/// 快速注册 URL（未缓存快照，下发时在线拉取）
pub async fn register(key: &str, url: &str) {
    let item = SubscriptionItem {
        key: key.to_string(),
        url: url.to_string(),
        raw_yaml: String::new(),
        user_info: String::new(),
        updated_at: current_timestamp(),
        node_count: 0,
        is_manual_cached: false,
        last_status: "pending".to_string(),
        mode: "url".to_string(),
    };
    save_subscription(item).await;
}

/// 获取订阅配置（兼容旧格式纯字符串数据）
pub async fn lookup_item(key: &str) -> Option<SubscriptionItem> {
    let cache_key = format!("{}{}", PREFIX, key);
    let val = store::cache().await.get(&cache_key).await?;

    // 尝试反序列化为新版结构体
    if let Ok(item) = serde_json::from_str::<SubscriptionItem>(&val) {
        return Some(item);
    }

    // 兼容旧格式：之前直接存 URL 字符串的情况
    if val.starts_with("http://") || val.starts_with("https://") {
        return Some(SubscriptionItem {
            key: key.to_string(),
            url: val,
            raw_yaml: String::new(),
            user_info: String::new(),
            updated_at: 0,
            node_count: 0,
            is_manual_cached: false,
            last_status: "ok".to_string(),
            mode: "url".to_string(),
        });
    }

    None
}

/// 快捷查询 URL（兼容原有旧调用）
pub async fn lookup(key: &str) -> Option<String> {
    lookup_item(key).await.map(|item| item.url)
}

/// 在线实时拉取成功时，异步记录最新元数据（节点数、流量信息、状态），绝不覆盖/冻结 raw_yaml！
pub async fn record_fetch_success(key: &str, node_count: usize, user_info: &str, raw_yaml: &str) {
    if let Some(mut item) = lookup_item(key).await {
        item.node_count = node_count;
        if !user_info.trim().is_empty() {
            item.user_info = user_info.to_string();
        }
        if !raw_yaml.trim().is_empty() {
            item.raw_yaml = raw_yaml.to_string();
        }
        item.last_status = "ok".to_string();
        item.updated_at = current_timestamp();
        let has_traffic = !item.user_info.is_empty();
        save_subscription(item).await;
        info!(
            "已异步同步订阅 [{}] 的实时状态看板与文件缓存 (节点数: {}, 流量: {})",
            key,
            node_count,
            if has_traffic { "已记录" } else { "无" }
        );
    }
}

/// 在线实时拉取失败时，异步记录错误信息到状态看板
pub async fn record_fetch_error(key: &str, err_msg: &str) {
    if let Some(mut item) = lookup_item(key).await {
        item.last_status = err_msg.to_string();
        item.updated_at = current_timestamp();
        save_subscription(item).await;
        info!("已异步记录订阅 [{}] 的拉取异常状态: {}", key, err_msg);
    }
}

/// 清空指定订阅的真节点快照（清空后变更为纯在线实时拉取状态）
pub async fn clear_subscription_cache(key: &str) -> bool {
    if let Some(mut item) = lookup_item(key).await {
        item.raw_yaml = String::new();
        item.is_manual_cached = false;
        item.mode = "url".to_string();
        save_subscription(item).await;
        info!("订阅 [{}] 的真节点快照已清空，变更为纯实时在线拉取状态", key);
        return true;
    }
    false
}

/// 列出所有当前保存的订阅配置项
pub async fn list_all() -> Vec<SubscriptionItem> {
    let keys = store::cache().await.get_keys_by_prefix(PREFIX).await;
    let mut list = Vec::new();

    for full_key in keys {
        if let Some(real_key) = full_key.strip_prefix(PREFIX) {
            if let Some(item) = lookup_item(real_key).await {
                list.push(item);
            }
        }
    }

    // 按更新时间降序排列
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    list
}

/// 注销 / 删除订阅
pub async fn unregister(key: &str) {
    let cache_key = format!("{}{}", PREFIX, key);
    store::cache().await.delete(&cache_key).await;
    info!("注销订阅成功: {}", key);
}
