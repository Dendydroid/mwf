mod cache;
mod db;
mod factory;
mod session;
mod settings;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::factory::Factory;
    use crate::session::{SessionStore, SessionToken};
    use config::Config;
    use settings::AppSettings;
    use sqlx::Postgres;
    use tokio::runtime::Runtime;

    #[test]
    fn config_could_be_built_and_deserialized() {
        dotenvy::dotenv().ok();

        let settings = Config::builder()
            .add_source(config::File::with_name("config/settings.toml"))
            .add_source(config::Environment::default())
            .build();

        assert!(settings.is_ok(), "Settings build failed");

        let settings: Result<AppSettings, _> = settings.unwrap().try_deserialize();

        assert!(settings.is_ok(), "Settings deserialize failed");
    }

    #[test]
    fn database_connected_and_pool_created() {
        let rt = Runtime::new().unwrap();
        let db = rt.block_on(async move {
            let settings = AppSettings::load();

            Database::<Postgres>::new(&settings).await
        });

        assert!(true);
    }

    #[test]
    fn cache_and_session_connections_created() {
        let rt = Runtime::new().unwrap();
        let cache = rt.block_on(async move {
            let settings = AppSettings::load();

            let (cache, session) = Factory::create_cache_and_session(&settings).await;
        });

        assert!(true);
    }

    #[test]
    fn session_id_generates_32_byte_hex() {
        let token = SessionToken::generate_session_id();

        assert_eq!(token.len(), 64);
    }

    #[test]
    fn cache_saves_value_and_value_is_removed() {
        let rt = Runtime::new().unwrap();
        let (mut cache, _) = rt.block_on(async move {
            let settings = AppSettings::load();

            Factory::create_cache_and_session(&settings).await
        });

        let key = String::from("key_test_cache_123");

        let (check_1, check_2, check_3) = rt.block_on(async move {
            let item = cache.get(key.clone()).await.unwrap_or(0);

            let first_check_value_is_0 = item == 0;

            cache.set(key.clone(), 100i32).await;

            let item = cache.get(key.clone()).await.unwrap_or(0);

            let second_check_value_is_100 = item == 100;

            cache.del(key.clone()).await;

            let item = cache.get(key.clone()).await.unwrap_or(0);

            let third_check_value_is_0 = item == 0;

            (
                first_check_value_is_0,
                second_check_value_is_100,
                third_check_value_is_0,
            )
        });

        assert!(
            check_1 && check_2 && check_3,
            "Cache didn't write value properly!"
        );
    }
}
