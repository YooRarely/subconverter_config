use tracing::info;

use crate::store;

pub async fn register(key: &str, url: &str) {
    store::cache()
        .await
        .set(format!("subscribe:{}", key), url.to_string())
        .await;
	info!("添加订阅成功: {} = {}", key, url);
}

/// 查询登记内容 (Get)
pub async fn lookup(key: &str) -> Option<String> {
    match store::cache()
        .await
        .get::<String>(format!("subscribe:{}", key))
        .await
    {
        Some(url) => {

            Some(url.to_string())
        }
        None => {
            info!("订阅地址: {} 不存在", key);
            None
        }
    }
}

/// 注销登记 (Delete)
pub async fn unregister(key: &str) {
    store::cache()
        .await
        .delete(format!("subscribe:{}", key))
        .await;
}
