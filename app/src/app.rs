use crate::cache::Cache;
use crate::db::Database;
use crate::event::event_bus::Dispatcher;
use crate::event::events::events;
use crate::factory::Factory;
use crate::session::SessionStore;
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use sqlx::Postgres;
use std::sync::Arc;
use reqwest::Client;
use crate::domain::call::InstructionRegistry;
use crate::domain::instructions::instructions;

pub struct AppState {
    pub db: Database<Postgres>,
    pub cache: Cache,
    pub session: SessionStore,
    pub llm: VllmClient,
    pub http_client: Client,
    pub settings: AppSettings,
    pub event_dispatcher: Dispatcher,
    pub instructions: InstructionRegistry,
}

impl AppState {
    pub async fn new(settings: AppSettings) -> Self {
        let (cache, session) = Factory::create_cache_and_session(&settings).await;

        Self {
            db: Database::<Postgres>::new(&settings).await,
            cache,
            session,
            llm: VllmClient::new(&settings),
            http_client: Client::new(),
            settings,
            event_dispatcher: events(),
            instructions: instructions(),
        }
    }
}
