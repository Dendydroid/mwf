use crate::cache::Cache;
use crate::db::Database;
use crate::domain::call_session::CallLocks;
use crate::event::event_bus::Dispatcher;
use crate::event::events::events;
use crate::factory::Factory;
use crate::session::SessionStore;
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use sqlx::Postgres;
use std::sync::Arc;
use reqwest::Client;

pub struct AppState {
    pub db: Database<Postgres>,
    pub cache: Cache,
    pub session: SessionStore,
    pub llm: VllmClient,
    pub http_client: Client,
    pub settings: AppSettings,
    pub event_dispatcher: Dispatcher,
    pub call_locks: CallLocks,
}

impl AppState {
    pub async fn new(settings: AppSettings) -> Self {
        let (cache, session) = Factory::create_cache_and_session(&settings).await;
        let http_client = Client::new();
        let event_dispatcher = events(&settings, &http_client);

        Self {
            db: Database::<Postgres>::new(&settings).await,
            cache,
            session,
            llm: VllmClient::new(&settings),
            http_client,
            settings,
            event_dispatcher,
            call_locks: CallLocks::default(),
        }
    }
}
