use moka::future::Cache;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use std::collections::HashSet;
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
            if redis_url.is_empty() {
                info!("💡 未配置 REDIS_CONNECTION_STRING，已跳过 Redis 连接，秒级启动纯内存高速缓存");
            } else {
                match redis::Client::open(redis_url) {
                    Ok(client) => {
                        info!("正在连接 Redis (1.5秒超时保护): {} ...", redis_url);
                        match tokio::time::timeout(
                            Duration::from_millis(1500),
                            ConnectionManager::new(client),
                        )
                        .await
                        {
                            Ok(Ok(mgr)) => {
                                info!("✅ 成功连接到 Redis 缓存: {}", redis_url);
                                redis = Some(mgr);
                            }
                            Ok(Err(e)) => {
                                warn!("⚠️ 连接 Redis 握手失败，自动回退到纯内存缓存: {:?}", e);
                            }
                            Err(_) => {
                                warn!("⚠️ 连接 Redis 超过 1.5 秒超时，已自动跳过并回退到纯内存缓存");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("⚠️ Redis URL 解析失败，回退到纯内存缓存: {:?}", e);
                    }
                }
            }
        } else {
            info!("💡 未提供 REDIS_CONNECTION_STRING，使用纯内存高速缓存");
        }

        Self { local, redis }
    }

    pub fn is_redis_connected(&self) -> bool {
        self.redis.is_some()
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

    /// 获取包含指定前缀的所有键名
    pub async fn get_keys_by_prefix(&self, prefix: &str) -> Vec<String> {
        let mut keys_set = HashSet::new();

        // 1. 扫描 Redis (如果有)
        if let Some(ref mgr) = self.redis {
            let mut conn = mgr.clone();
            let query = format!("{}*", prefix);
            if let Ok(redis_keys) = conn.keys::<_, Vec<String>>(query).await {
                for k in redis_keys {
                    keys_set.insert(k);
                }
            }
        }

        // 2. 扫描本地内存
        for (k, _) in self.local.iter() {
            if k.starts_with(prefix) {
                keys_set.insert((*k).clone());
            }
        }

        keys_set.into_iter().collect()
    }
}

static CACHE_INSTANCE: OnceCell<Arc<CacheManager>> = OnceCell::const_new();

pub async fn cache() -> &'static Arc<CacheManager> {
    CACHE_INSTANCE
        .get_or_init(|| async { Arc::new(CacheManager::new().await) })
        .await
}
