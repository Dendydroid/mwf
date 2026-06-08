use crate::cache::Cache;
use crate::db::Database;
use crate::factory::Factory;
use crate::session::SessionStore;
use crate::settings::AppSettings;
use sqlx::Postgres;

pub struct AppState {
    pub db: Database<Postgres>,
    pub cache: Cache,
    pub session: SessionStore,
    pub settings: AppSettings,
}

impl AppState {
    pub async fn new() -> Self {
        let settings = AppSettings::load();
        let (cache, session) = Factory::create_cache_and_session(&settings).await;

        Self {
            db: Database::<Postgres>::new(&settings).await,
            cache,
            session,
            settings,
        }
    }
}
