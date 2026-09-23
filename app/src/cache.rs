use redis::aio::ConnectionManager;
use redis::{AsyncCommands, ErrorKind, FromRedisValue, ToRedisArgs};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::log::error;

#[derive(Clone)]
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

    pub async fn set_ex<V: ToRedisArgs + Send + Sync>(
        &mut self,
        key: String,
        value: V,
        ttl_seconds: u64,
    ) -> bool {
        let mut conn = self.manager.clone();
        conn.set_ex::<_, _, bool>(key, value, ttl_seconds)
            .await
            .unwrap_or_else(|e| {
                error!("Redis responded with error {e} when trying to set value");

                false
            })
    }

    pub async fn get_serde<T: DeserializeOwned>(&mut self, key: String) -> Option<T> {
        let mut conn = self.manager.clone();

        let bytes = match conn.get::<_, Option<Vec<u8>>>(&key).await {
            Ok(v) => v?,
            Err(e) => {
                error!("Redis responded with error {e} when trying to get value");

                return None;
            }
        };

        match rmp_serde::from_slice(&bytes) {
            Ok(value) => Some(value),
            // A decode failure means the value under that key was written by an
            // older shape of the type. Treat it as a miss so the caller rebuilds
            // it instead of failing the request.
            Err(e) => {
                error!("Could not decode cached value under {key}: {e}");

                None
            }
        }
    }

    pub async fn set_serde<V: Serialize + ?Sized>(&mut self, key: String, value: &V) -> bool {
        match rmp_serde::to_vec(value) {
            Ok(bytes) => self.set(key, bytes).await,
            Err(e) => {
                error!("Could not encode value for {key}: {e}");

                false
            }
        }
    }

    pub async fn set_serde_ex<V: Serialize + ?Sized>(
        &mut self,
        key: String,
        value: &V,
        ttl_seconds: u64,
    ) -> bool {
        match rmp_serde::to_vec(value) {
            Ok(bytes) => self.set_ex(key, bytes, ttl_seconds).await,
            Err(e) => {
                error!("Could not encode value for {key}: {e}");

                false
            }
        }
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

pub enum CacheKey {
    CallSessionKey,
}

impl CacheKey {
    pub fn get_key(self, unique_id: &str) -> String {
        match self {
            CacheKey::CallSessionKey => format!("call_id.{unique_id}"),
        }
    }
}
