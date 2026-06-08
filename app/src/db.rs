use crate::settings::AppSettings;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Pool, Postgres};
use std::time::Duration;

#[derive(Debug)]
pub struct Database<T: sqlx::Database> {
    pool: Pool<T>,
}

impl<T: sqlx::Database> Database<T> {
    pub async fn new(settings: &AppSettings) -> Database<Postgres> {
        let pool = PgPoolOptions::new()
            .max_connections(settings.database_settings.max_connections)
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(
                settings.database_settings.timeout_seconds,
            ))
            .connect(settings.database_url())
            .await;

        if pool.is_err() {
            panic!("Could not connect to database: {}", pool.err().unwrap());
        }

        Database {
            pool: pool.unwrap(),
        }
    }

    pub fn pool(&self) -> Pool<T> {
        self.pool.clone()
    }
}
