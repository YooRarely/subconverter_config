use tracing::info;

use crate::store;

const PREFIX: &str = "subscribe:";

/// 注册订阅 URL 到缓存 / Redis
pub async fn register(key: &str, url: &str) {
    let cache_key = format!("{}{}", PREFIX, key);
    // 永久存储于 Redis（若无 Redis 则保存在当前进程内存中）
    store::cache().await.set(&cache_key, url.to_string(), None).await;
    info!("登记订阅成功: {} = {}", key, url);
}

/// 查询已登记的订阅 URL
pub async fn lookup(key: &str) -> Option<String> {
    let cache_key = format!("{}{}", PREFIX, key);
    match store::cache().await.get(&cache_key).await {
        Some(url) => Some(url),
        None => {
            info!("未找到订阅: {}", key);
            None
        }
    }
}

/// 注销登记
pub async fn unregister(key: &str) {
    let cache_key = format!("{}{}", PREFIX, key);
    store::cache().await.delete(&cache_key).await;
    info!("注销订阅成功: {}", key);
}
