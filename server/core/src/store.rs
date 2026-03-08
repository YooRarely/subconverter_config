use cache::{CacheManager, NoneSource, RemoteSource, redis_cache::RedisManager};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::env;

const CACHE_CAPACITY: u64 = 10000;
const CACHE_TTL: u64 = 10;

static REMOTE_CACHE: OnceCell<Arc<CacheManager>> = OnceCell::const_new();
pub async fn cache() -> &'static Arc<CacheManager> {
    REMOTE_CACHE
        .get_or_init(|| async {
            let remote = match RedisManager::new(&env::REDIS_CONNECTION_STRING).await {
                Ok(mgr) => {
                    info!("Successfully connected to Redis");
                    Arc::new(mgr) as Arc<dyn RemoteSource>
                }
                Err(e) => {
                    warn!(
                        "Redis connection failed: {:?}. Falling back to NoopRemote (Memory Only).",
                        e
                    );
                    Arc::new(NoneSource::default()) as Arc<dyn RemoteSource>
                }
            };
            Arc::new(CacheManager::new(remote, CACHE_CAPACITY, CACHE_TTL))
        })
        .await
}
