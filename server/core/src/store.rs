use moka::future::Cache;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::env;

pub struct CacheManager {
    local: Cache<String, String>,
    redis: Option<ConnectionManager>,
}

impl CacheManager {
    pub async fn new() -> Self {
        let local = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(3600))
            .build();

        let mut redis = None;
        if let Ok(redis_url) = env::REDIS_CONNECTION_STRING.try_get() {
            let redis_url = redis_url.trim();
            if !redis_url.is_empty() {
                match redis::Client::open(redis_url) {
                    Ok(client) => match ConnectionManager::new(client).await {
                        Ok(mgr) => {
                            info!("成功连接到 Redis 缓存: {}", redis_url);
                            redis = Some(mgr);
                        }
                        Err(e) => {
                            warn!("连接 Redis 失败，回退到纯内存缓存: {:?}", e);
                        }
                    },
                    Err(e) => {
                        warn!("Redis URL 解析失败，回退到纯内存缓存: {:?}", e);
                    }
                }
            }
        }

        Self { local, redis }
    }

    pub async fn set(&self, key: &str, val: String, expire_secs: Option<u64>) {
        self.local.insert(key.to_string(), val.clone()).await;

        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let res: Result<(), redis::RedisError> = match expire_secs {
                Some(secs) if secs > 0 => conn.set_ex(key, val, secs).await,
                _ => conn.set(key, val).await,
            };
            if let Err(e) = res {
                warn!("写入 Redis 失败 (key={}): {:?}", key, e);
            }
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        // 先查本地内存缓存
        if let Some(val) = self.local.get(key).await {
            return Some(val);
        }

        // 本地没有，再查 Redis
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            match conn.get::<_, Option<String>>(key).await {
                Ok(Some(val)) => {
                    self.local.insert(key.to_string(), val.clone()).await;
                    return Some(val);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("读取 Redis 失败 (key={}): {:?}", key, e);
                }
            }
        }

        None
    }

    pub async fn delete(&self, key: &str) {
        self.local.remove(key).await;

        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let _: Result<(), _> = conn.del(key).await;
        }
    }
}

static CACHE_INSTANCE: OnceCell<Arc<CacheManager>> = OnceCell::const_new();

pub async fn cache() -> &'static Arc<CacheManager> {
    CACHE_INSTANCE
        .get_or_init(|| async { Arc::new(CacheManager::new().await) })
        .await
}
