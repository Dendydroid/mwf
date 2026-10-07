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

/// What a voice says, the recognizer hears: text to speech into speech to text, without the
/// sound card. Needs the speech models, which `./run-local-stt.sh` downloads:
/// `cargo test -p app --features local-stt -- --ignored speech`
#[cfg(feature = "local-stt")]
#[test]
#[ignore = "needs the speech models"]
fn speech_survives_the_round_trip() {
    use crate::domain::call::FormSupported;
    use crate::stt::transcriber::{Transcriber, WINDOW};
    use crate::stt::SAMPLE_RATE;
    use crate::tts::voice::Voice;
    use lingua::{IsoCode639_1, Language};
    use sherpa_onnx::LinearResampler;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::str::FromStr;

    // The paths are relative to the project root, and the tests run in app/
    let from_root = |path: &str| format!("../{path}");
    let letters = |text: &str| text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();

    let mut settings = AppSettings::load();
    settings.stt_settings.vad_model = from_root(&settings.stt_settings.vad_model);
    settings.stt_settings.model_dir = from_root(&settings.stt_settings.model_dir);

    let mut transcriber = Transcriber::load(&settings.stt_settings).unwrap();
    let question = &vocabulary().form(FormSupported::DoctorAppointment).field("date_of_birth").ask;

    for (language, dir) in &settings.tts_settings.voices {
        let voice = Voice::load(&from_root(dir), settings.tts_settings.num_threads).unwrap();
        let language = Language::from_iso_code_639_1(&IsoCode639_1::from_str(language).unwrap());
        let text = question.say(language);

        let said = Rc::new(RefCell::new(Vec::new()));
        voice
            .say(text, {
                let said = Rc::clone(&said);
                move |sentence| said.borrow_mut().extend_from_slice(sentence)
            })
            .unwrap();

        // At the recognizer's sample rate, followed by the silence that ends an utterance
        let mut sound = LinearResampler::create(voice.sample_rate().get() as i32, SAMPLE_RATE)
            .unwrap()
            .resample(&said.borrow(), true);
        sound.extend(vec![0.0; 2 * SAMPLE_RATE as usize]);

        let heard: Vec<String> = sound.chunks_exact(WINDOW).filter_map(|window| transcriber.hear(window)).collect();

        assert_eq!(letters(&heard.join(" ")), letters(text), "{language}");

        // As between two turns: the next voice starts from nothing
        transcriber.forget();
    }
}
