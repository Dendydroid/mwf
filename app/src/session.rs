use rand::RngCore;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::{Display, Formatter, Write};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use time::Duration;
use tokio::sync::{RwLock, RwLockReadGuard};

static SESSION_LAST_SESSION_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UserSession {
    pub token_id: u64,
    user_chosen_name: String,
}

impl PartialEq<UserSession> for RwLockReadGuard<'_, UserSession> {
    fn eq(&self, other: &UserSession) -> bool {
        other.token_id == self.token_id && other.user_chosen_name == self.user_chosen_name
    }
}

impl Display for UserSession {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(serde_json::to_string(&self).unwrap().as_str())
    }
}

impl UserSession {
    pub fn new() -> Self {
        let new_last_uid_lock = SESSION_LAST_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            token_id: new_last_uid_lock,
            user_chosen_name: String::from(""),
        }
    }

    pub fn change_name(&mut self, name: String) {
        self.user_chosen_name = name;
    }

    pub fn get_name(&self) -> String {
        self.user_chosen_name.clone()
    }
}

pub struct SessionStore {
    manager: ConnectionManager,
}

#[derive(Clone, PartialEq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn generate_session_id() -> Self {
        let mut bytes = [0u8; 32];

        rand::thread_rng().fill_bytes(&mut bytes);

        Self(hex::encode(bytes))
    }

    pub fn from(token: &str) -> Self {
        Self(token.into())
    }
}

impl Display for SessionToken {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Deref for SessionToken {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SessionStore {
    pub fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    fn session_key(&self, token: &SessionToken) -> String {
        format!("session:{}", token)
    }

    pub async fn save_session(
        &self,
        token: &SessionToken,
        session: &UserSession,
        ttl_days: Duration,
    ) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        let key = self.session_key(token);

        let binary_data = rmp_serde::to_vec(session)?;

        let ttl_seconds = ttl_days.as_seconds_f64() as u64;
        conn.set_ex::<_, _, ()>(&key, binary_data, ttl_seconds)
            .await?;

        Ok(())
    }

    /// Fetch and refresh a session (Sliding Window TTL)
    pub async fn get_and_refresh_session(
        &self,
        token: &SessionToken,
        ttl: Duration,
    ) -> Option<UserSession> {
        let mut conn = self.manager.clone();
        let key = self.session_key(token);

        // Fetch the raw binary bytes from Redis
        let binary_data: Option<Vec<u8>> = conn.get(&key).await.ok()?;
        let bytes = binary_data?;

        // Deserialize binary data back into our Rust struct
        let session: UserSession = rmp_serde::from_slice(&bytes).ok()?;

        // Efficiently extend the session expiration time because the user is active
        let ttl_seconds = ttl.as_seconds_f64() as u64;
        let _: Result<(), _> = conn.expire(&key, ttl_seconds as i64).await;

        Some(session)
    }

    /// Explicit logout
    pub async fn destroy_session(&self, token: &SessionToken) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        let key = self.session_key(token);

        conn.del::<_, ()>(&key).await?;
        Ok(())
    }
}
