use crate::cache::Cache;
use crate::session::SessionStore;
use crate::settings::AppSettings;
use redis::aio::ConnectionManager;

pub struct Factory {}

impl Factory {
    pub async fn create_cache_and_session(settings: &AppSettings) -> (Cache, SessionStore) {
        let client = redis::Client::open(settings.cache_url()).expect("Invalid Redis URL");

        let connection_manager = ConnectionManager::new(client)
            .await
            .expect("Could not initialize Redis connection manager");

        (
            Cache::new(connection_manager.clone()),
            SessionStore::new(connection_manager.clone()),
        )
    }
}
