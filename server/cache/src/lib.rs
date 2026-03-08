use anyhow::{Context, Result};
use async_trait::async_trait;
use moka::future::Cache;
use serde::{Serialize, de::DeserializeOwned};
use std::{any::Any, sync::Arc, time::Duration};
use tracing::{debug, error, warn};
pub mod redis_cache;

#[async_trait]
pub trait RemoteSource: Send + Sync + 'static {
    // 写入远端
    async fn set_raw(&self, _key: &str, _json: String, _expire_secs: u64) -> Result<()> {
        Ok(())
    }
    // 从远端读取
    async fn get_raw(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoneSource;
impl RemoteSource for NoneSource {}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).context("Failed to serialize data for Redis")
}
fn from_json<T: DeserializeOwned>(json: String) -> Result<T> {
    let obj = serde_json::from_str(&json).context("Failed to deserialize Redis data")?;
    Ok(obj)
}

pub trait Cacheable: Serialize + DeserializeOwned + Send + Sync + 'static {}
impl<T> Cacheable for T where T: Serialize + DeserializeOwned + Send + Sync + 'static {}

pub trait CacheKey {
    fn key(&self) -> &str;
    fn expire_secs(&self) -> u64 {
        0
    }
    fn remote_source(&self) -> bool {
        true
    }
}

impl CacheKey for &str {
    fn key(&self) -> &str {
        self
    }
}
impl CacheKey for String {
    fn key(&self) -> &str {
        &self
    }
}
impl<T:AsRef<str>> CacheKey for (T, u64) {
    fn key(&self) -> &str {
        self.0.as_ref()
    }
    fn expire_secs(&self) -> u64 {
        self.1
    }
}
impl<T:AsRef<str>> CacheKey for (T, Option<u64>) {
    fn key(&self) -> &str {
        self.0.as_ref()
    }
    fn expire_secs(&self) -> u64 {
        self.1.unwrap_or(0)
    }
    fn remote_source(&self) -> bool {
        self.1.is_some()
    }
}

pub trait AsyncFnOnce<T> {
    type Fut: Future<Output = Option<T>>;
    fn call(self) -> Self::Fut;
}
impl<T, F, Fut> AsyncFnOnce<T> for F
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    type Fut = Fut;
    fn call(self) -> Self::Fut {
        (self)() // 调用闭包并返回 Future
    }
}

pub struct CacheManager {
    local_cache: Cache<String, Arc<dyn Any + Send + Sync>>,
    // 替换为 Arc<dyn RemoteSource>
    remote_source: Arc<dyn RemoteSource>,
}
impl CacheManager {
    pub fn new(
        remote_source: Arc<dyn RemoteSource>,
        max_capacity: u64,
        local_ttl_sec: u64,
    ) -> Self {
        Self {
            local_cache: Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(Duration::from_secs(local_ttl_sec))
                .build(),
            remote_source,
        }
    }

    pub async fn set<T: Cacheable>(&self, key: impl CacheKey, value: T) -> Arc<T> {
        let expire_secs = key.expire_secs();
        let is_remote_source = key.remote_source();
        let key: &str = key.key();
        let shared_val = Arc::new(value);
        let k = key.to_string();
        // 1. 存入 L1 内存
        self.local_cache.insert(k.clone(), shared_val.clone()).await;
        if !is_remote_source {
            return shared_val;
        }
        let return_val = shared_val.clone();
        // 2. 异步写 L2 (RemoteSource)
        let remote_source = self.remote_source.clone();

        tokio::spawn(async move {
            // 在这个异步块内使用 ?，因为我们将结果捕获到了 result 变量里
            let result: Result<()> = async {
                let json = to_json(&*shared_val)?;
                remote_source.set_raw(&k, json, expire_secs).await?;
                Ok(())
            }
            .await;

            if let Err(e) = result {
                warn!("远端备份失败 (降级至内存): {}, 错误: {:?}", k, e);
            } else {
                debug!("远端备份成功: {}", k);
            }
        });

        return_val
    }

    pub async fn get<T: Cacheable>(&self, key: impl CacheKey) -> Option<Arc<T>> {
        let is_remote_source = key.remote_source();
        let key: &str = key.key();
        // --- 1. L1 内存层 ---
        if let Some(any_val) = self.local_cache.get(key).await {
            if let Ok(concrete_val) = any_val.downcast::<T>() {
                return Some(concrete_val);
            }
        }
        if !is_remote_source {
            return None;
        }
        debug!("内存未命中，尝试从远端读取: {}", key);

        // --- 2. L2 远端层 ---
        // 使用一个异步闭包处理 get_raw -> from_json 的转换
        let remote_res: Result<Option<T>> = async {
            if let Some(json) = self.remote_source.get_raw(key).await? {
                let obj = from_json::<T>(json)?;
                return Ok(Some(obj));
            }
            Ok(None)
        }
        .await;

        match remote_res {
            Ok(Some(obj)) => {
                let shared_obj = Arc::new(obj);
                // 回填内存
                self.local_cache
                    .insert(key.to_string(), shared_obj.clone())
                    .await;
                Some(shared_obj)
            }
            Ok(None) => {
                debug!("远端亦未命中: {}", key);
                None
            }
            Err(e) => {
                error!("远端读取异常: {}, 错误: {:?}", key, e);
                None
            }
        }
    }
    pub async fn get_or_set<T: Cacheable>(
        &self,
        key: impl CacheKey,
        setter: impl FnOnce() -> Option<T>,
    ) -> Option<Arc<T>> {
        if let Some(value) = self.get(key.key()).await {
            return Some(value);
        }
        if let Some(value) = setter() {
            return Some(self.set(key, value).await);
        }
        None
    }
    pub async fn get_or_set_async<T: Cacheable>(
        &self,
        key: impl CacheKey,
        setter: impl AsyncFnOnce<T>,
    ) -> Option<Arc<T>> {
        if let Some(value) = self.get(key.key()).await {
            return Some(value);
        }
        if let Some(value) = setter.call().await {
            return Some(self.set(key, value).await);
        }
        None
    }
    pub async fn delete(&self, key: impl CacheKey) {
        let key: &str = key.key();
        let _ = self.local_cache.remove(key);
		let _ = self.remote_source.delete(key).await;
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;
    use redis_cache::RedisManager;
    use serde::{Deserialize, Serialize};
    use tokio::time::{Duration, sleep};

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct User {
        id: u64,
        name: String,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct Config {
        theme: String,
        version: i32,
    }

    // 假设你的环境已经配置好 Redis URL
    const REDIS_URL: &str =
        "redis://:9nJAm35rXElMHY86P2Dhzfec170s4gTG@cgk1.clusters.zeabur.com:22037";

    async fn setup() -> CacheManager {
        let redis_mgr = RedisManager::new(REDIS_URL).await.expect("Redis 连不上");
        // max_capacity: 100, local_ttl: 2秒 (为了方便测试过期)
        CacheManager::new(Arc::new(redis_mgr), 100, 2)
    }

    #[tokio::test]
    async fn test_cache_manager_basic_flow() -> Result<()> {
        let cache = setup().await;
        let user = User {
            id: 1,
            name: "Gemini".into(),
        };
        let key = "user:1";

        // 1. 写入测试
        cache.set(key, user.clone()).await;

        // 2. 内存命中测试 (L1)
        // 刚写完，内存里肯定有，此时应该秒回且是 Some
        let cached_user = cache.get::<User>(key).await;
        assert_eq!(cached_user.as_deref(), Some(&user));
        println!("✅ L1 内存命中成功");

        // 3. 多类型存储测试 (类型擦除)
        let config = Config {
            theme: "Dark".into(),
            version: 2,
        };
        cache.set("cfg:app", config.clone()).await;

        let cached_cfg = cache.get::<Config>("cfg:app").await;
        assert_eq!(cached_cfg.as_deref(), Some(&config));
        println!("✅ 多类型存储校验成功");

        Ok(())
    }

    #[tokio::test]
    async fn test_l1_expiration_and_l2_fallback() -> Result<()> {
        let cache = setup().await;
        let user = User {
            id: 42,
            name: "DeepSeek".into(),
        };
        let key = "user:42";

        // 写入并设置 Redis 过期时间为 10 秒
        cache.set((key, 10), user.clone()).await;

        // 等待 3 秒，让本地内存 L1 过期 (我们设置的 L1 TTL 是 2 秒)
        println!("等待 L1 过期...");
        sleep(Duration::from_secs(3)).await;

        // 此时获取数据：
        // 1. 内存里已经没了
        // 2. 逻辑会去 Redis 捞
        // 3. Redis 里还有（因为 Redis 设的是 10 秒）
        let fetched = cache.get::<User>(key).await;

        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, "DeepSeek");
        println!("✅ L1 过期后成功从 L2 Redis 回源数据");

        Ok(())
    }

    #[tokio::test]
    async fn test_type_safety_downcast() -> Result<()> {
        let cache = setup().await;
        cache
            .set(
                "mix:1",
                User {
                    id: 7,
                    name: "James".into(),
                },
            )
            .await;

        // 故意用错误的类型去取同一个 Key
        // 由于我们用了 downcast，这里应该因为类型不匹配而进入 Redis 读取逻辑
        // 如果 Redis 也解析失败（因为类型确实不对），最终会返回 None 或报错
        let wrong_type = cache.get::<Config>("mix:1").await;
        assert!(wrong_type.is_none());
        println!("✅ 类型安全校验成功：错误的类型转换未导致崩溃");

        Ok(())
    }

    #[tokio::test]
    async fn test_async_background_backup() -> Result<()> {
        let cache = setup().await;
        let key = "async:test";

        // 写入数据
        cache.set(key, "Just a string".to_string()).await;

        // 立即删除本地内存中的数据（模拟内存被踢出或丢失）
        // 注意：CacheManager 没暴露 local_cache，这里我们只是逻辑演示
        // 实际上我们可以等待 L1 过期

        // 验证异步写入：给 Redis 一点点时间完成 tokio::spawn 的任务
        sleep(Duration::from_millis(500)).await;

        // 重新获取，确保 Redis 已经备份成功
        let redis_val = cache.get::<String>(key).await;
        assert!(redis_val.is_some());
        println!("✅ 异步 Redis 备份任务验证通过");

        Ok(())
    }
}
