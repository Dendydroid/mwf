use redis::aio::ConnectionManager;
use redis::{
    AsyncCommands, Client, Commands, Connection, ErrorKind, FromRedisValue, RedisResult,
    ToRedisArgs, Value,
};
use tracing::log::error;

pub struct Cache {
    manager: ConnectionManager,
}

impl Cache {
    pub fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub async fn get<T: FromRedisValue>(&mut self, key: String) -> Option<T> {
        let mut conn = self.manager.clone();
        match conn.get::<_, T>(key).await {
            Ok(v) => Option::from(v),
            Err(e) if e.kind() == ErrorKind::TypeError => None,
            Err(e) => {
                error!("Redis responded with error {e} when trying to get value");

                None
            }
        }
    }

    pub async fn set<V: ToRedisArgs + Send + Sync>(&mut self, key: String, value: V) -> bool {
        let mut conn = self.manager.clone();
        conn.set::<_, _, bool>(key, value)
            .await
            .unwrap_or_else(|e| {
                error!("Redis responded with error {e} when trying to get value");

                false
            })
    }

    pub async fn del(&mut self, key: String) -> bool {
        let mut conn = self.manager.clone();
        match conn.del::<_, bool>(key).await {
            Ok(v) => v,
            Err(e) if e.kind() == ErrorKind::TypeError => false,
            Err(e) => {
                error!("Redis responded with error {e} when trying to get value");

                false
            }
        }
    }
}
