use config::{Config, Environment};
use serde::Deserialize;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
pub struct DatabaseSettings {
    pub max_connections: u32,
    pub timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
pub struct CacheSettings {
    pub session_ttl_days: u32,
}

#[derive(Debug, Deserialize)]
pub struct AppSettings {
    #[serde(rename = "app_env")]
    env: String,
    #[serde(rename = "app_port")]
    port: u16,
    database_url: String,
    #[serde(rename = "redis_url")]
    cache_url: String,
    pub cache_settings: CacheSettings,
    pub database_settings: DatabaseSettings,
}

impl AppSettings {
    pub fn load() -> AppSettings {
        match cfg!(test) {
            true => dotenvy::from_filename_override(".env.test").expect(".env.test does not exist"),
            false => dotenvy::dotenv().expect(".env does not exist"),
        };

        let settings = Config::builder()
            .add_source(config::File::with_name("config/settings.toml"))
            .add_source(Environment::default())
            .build()
            .expect("Application config build failed");

        settings
            .try_deserialize()
            .expect("Configuration deserialization failed")
    }

    pub fn database_url(&self) -> &str {
        &self.database_url
    }
    pub fn cache_url(&self) -> &str {
        &self.cache_url
    }
    pub fn env(&self) -> &str {
        &self.env
    }
    pub fn http_port(&self) -> u16 {
        self.port
    }
    pub fn version(&self) -> &str {
        APP_VERSION
    }
}
