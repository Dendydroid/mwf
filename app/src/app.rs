use crate::cache::Cache;
use crate::db::Database;
use crate::factory::Factory;
use crate::session::SessionStore;
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use sqlx::Postgres;

pub struct AppState {
    pub db: Database<Postgres>,
    pub cache: Cache,
    pub session: SessionStore,
    /// The inference server handle. Built once here rather than per request so
    /// every utterance reuses the same connection pool - see `VllmClient`.
    pub llm: VllmClient,
    pub settings: AppSettings,
}

impl AppState {
    /// Takes the settings rather than loading them, because logging has to be
    /// set up from `log_dir` *before* this runs - otherwise the first thing the
    /// process does (connecting to Postgres and Redis) is also the part whose
    /// failures would never reach the log file.
    pub async fn new(settings: AppSettings) -> Self {
        let (cache, session) = Factory::create_cache_and_session(&settings).await;

        Self {
            db: Database::<Postgres>::new(&settings).await,
            cache,
            session,
            llm: VllmClient::new(&settings),
            settings,
        }
    }
}
