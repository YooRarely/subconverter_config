use anyhow::{ Result};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

use async_trait::async_trait;

use crate::RemoteSource;

#[derive(Clone)] // ConnectionManager 允许廉价克隆
pub struct RedisManager {
    client: ConnectionManager,
}
#[async_trait]
impl RemoteSource for RedisManager {
    async fn set_raw(&self, key: &str, json: String, expire_secs: u64) -> Result<()> {
        let mut conn = self.client.clone();
        match expire_secs {
            0 => conn.set(key, json).await?,
            _ => conn.set_ex(key, json, expire_secs).await?,
        }
        Ok(())
    }
    // 从远端读取
    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.client.clone();
        let data: Option<String> = conn.get(key).await?;
        Ok(data)
    }
    async fn delete(&self, key: &str) -> Result<()> {
        let mut conn = self.client.clone();
        let _: () = conn.del(key).await?;
        Ok(())
    }
}

impl RedisManager {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;
        Ok(Self { client: manager })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use serde::{Deserialize, Serialize, de::DeserializeOwned};
        use tokio;
    impl RedisManager {
        /// 核心方法：存入任何可序列化的对象
        pub async fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
            self.set_ex(key, value, 0).await
        }
        pub async fn set_ex<T: Serialize>(
            &self,
            key: &str,
            value: &T,
            expire_secs: u64,
        ) -> Result<()> {
            let payload =
                serde_json::to_string(value).context("Failed to serialize data for Redis")?;
            self.set_raw(key, payload, expire_secs).await
        }
        /// 核心方法：读取并自动解析为特定类型
        pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
            match self.get_raw(key).await? {
                Some(json) => {
                    let obj =
                        serde_json::from_str(&json).context("Failed to deserialize Redis data")?;
                    Ok(Some(obj))
                }
                None => Ok(None),
            }
        }
    }
    // 定义测试结构体，实现 PartialEq 方便断言比较
    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct UserProfile {
        id: u64,
        username: String,
        active: bool,
    }

    #[tokio::test]
    async fn test_zeabur_redis_integration() -> Result<()> {
        // 1. 使用你提供的 Zeabur Redis 连接字符串
        let redis_url = "redis://:9nJAm35rXElMHY86P2Dhzfec170s4gTG@cgk1.clusters.zeabur.com:22037";

        // 初始化 RedisManager
        let cache = RedisManager::new(redis_url)
            .await
            .context("无法连接到 Zeabur Redis 实例，请检查网络或配置")?;

        // 定义唯一的测试 Key，防止污染线上其他数据
        let test_key = "test:user:999";
        let test_data = UserProfile {
            id: 999,
            username: "Zeabur_Tester".to_string(),
            active: true,
        };

        println!("--- 开始 Redis 集成测试 ---");

        // 2. 测试写入 (SET)
        // 设置 120 秒过期，确保测试完即便清理失败，数据也会自动消失
        cache.set_ex(test_key, &test_data, 120).await?;
        println!("✅ 数据已成功写入 Zeabur Redis");

        // 3. 测试读取 (GET)
        let result: Option<UserProfile> = cache.get(test_key).await?;

        assert!(result.is_some(), "应当能查找到刚才存入的数据");
        let fetched_data = result.unwrap();
        assert_eq!(fetched_data.username, "Zeabur_Tester");
        assert_eq!(fetched_data, test_data);
        println!("✅ 数据读取一致性校验通过: {:?}", fetched_data);

        // 4. 测试删除 (DELETE)
        cache.delete(test_key).await?;
        let after_delete: Option<UserProfile> = cache.get(test_key).await?;
        assert!(after_delete.is_none(), "删除后数据应当为空");
        println!("✅ 数据清理完成");

        println!("--- 测试圆满结束 ---");
        Ok(())
    }

    #[tokio::test]
    async fn test_auth_and_connection_error() {
        // 测试一个错误的 URL 看看错误处理是否正常
        let wrong_url = "redis://:wrong_pass@cgk1.clusters.zeabur.com:22037";
        let result = RedisManager::new(wrong_url).await;

        // 注意：ConnectionManager 在 new 的时候不一定会立即校验权限
        // 通常在第一次执行命令（如 set）时才会暴露认证错误
        if let Ok(mgr) = result {
            let probe = mgr.set_ex("probe", &"data", 1).await;
            assert!(probe.is_err(), "使用错误密码应当报错");
            println!("✅ 错误处理校验通过");
        }
    }
}
