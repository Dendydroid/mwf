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
    pub call_session_ttl_seconds: u32,
}

#[derive(Debug, Deserialize)]
pub struct CallSettings {
    pub default_language: String,
}

/// Knobs for the inference calls themselves. The *where* and the *which model*
/// live with the other deployment values below, because those change per
/// environment; these two change per taste and belong in `settings.toml`.
#[derive(Debug, Deserialize)]
pub struct LlmSettings {
    /// Whole-request budget for one classification, in seconds. Without it a
    /// stalled vLLM would hold an HTTP handler open indefinitely, and a caller
    /// waiting on the line notices that long before a connection pool does.
    ///
    /// It has to be generous rather than tight: a turn here normally takes one
    /// to three seconds, but a cold or contended GPU has been measured taking
    /// over thirty, and failing that turn is worse than waiting for it.
    pub timeout_seconds: u64,
    /// 0 by default: intent matching has to be reproducible, not creative.
    pub temperature: f32,
    /// Ceiling on generated tokens per turn - the other half of not timing out.
    /// See `ChatCompletionRequest::max_tokens`.
    pub max_tokens: u32,
}

/// What `--local-stt-mode` hears the caller with. Paths are relative to the working directory,
/// which is the project root in that mode; `run-local-stt.sh` downloads what they name.
#[cfg(feature = "local-stt")]
#[derive(Debug, Deserialize)]
pub struct SttSettings {
    /// The Silero model that tells speech from silence.
    pub vad_model: String,
    /// Directory of the Parakeet model that writes down what was said.
    pub model_dir: String,
    pub num_threads: i32,
    /// How long the caller has to be silent for the utterance to be over. Shorter answers
    /// sooner and cuts in on a caller who stops to think.
    pub end_of_utterance_silence_seconds: f32,
}

/// What `--local-stt-mode` answers the caller with.
#[cfg(feature = "local-stt")]
#[derive(Debug, Deserialize)]
pub struct TtsSettings {
    pub num_threads: i32,
    /// The directory of a Piper voice per language (ISO 639-1). An answer in a language that
    /// has none is not spoken.
    pub voices: std::collections::HashMap<String, String>,
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
    /// Base URL of the OpenAI-compatible inference server, ending in `/v1`
    /// (`LLM_URL`). It is defaulted in `config/settings.toml`, so the tests and
    /// a bare `cargo run` work without one being exported.
    llm_url: String,
    /// Which model to ask for (`LLM_MODEL`). vLLM serves exactly one model and
    /// rejects a request naming a different one, so this has to match how it
    /// was started - which is why both read the same variable in
    /// `docker-compose.yml`.
    llm_model: String,
    /// Only set when vLLM itself was started with `--api-key`. Optional, and an
    /// empty value counts as unset, because an empty assignment is how the
    /// `.env` files spell "not configured".
    llm_api_key: Option<String>,
    /// The file holding the system prompt (`LLM_SYSTEM_PROMPT_FILE`), relative
    /// to the working directory - the same way `config/settings.toml` is, so
    /// `prompts/system_prompt.txt` means `app/prompts/...` on a dev machine and
    /// `/app/prompts/...` in the container without either having to know.
    ///
    /// The path is read once at startup; the file it points at is re-read every
    /// turn. So swapping prompts by editing the file is instant, and swapping
    /// them by pointing this somewhere else needs a restart.
    llm_system_prompt_file: String,
    /// Directory for rolling log files (`LOG_DIR`). Unset means stdout only.
    ///
    /// Optional rather than defaulted on purpose: in the container it is
    /// `/var/log/app`, which `docker-compose.yml` mounts to `./logs` on the
    /// host, but a bare `cargo run` on a dev machine has no business creating a
    /// log directory nobody asked for. The local STT mode is the exception: it
    /// writes to `LOCAL_STT_LOG_DIR` when this is unset.
    log_dir: Option<String>,
    /// Where completed forms are POSTed (`FORM_SUBMIT_URL`). Unset or empty sends nothing.
    form_submit_url: Option<String>,
    pub cache_settings: CacheSettings,
    pub database_settings: DatabaseSettings,
    pub llm_settings: LlmSettings,
    pub call_settings: CallSettings,
    #[cfg(feature = "local-stt")]
    pub stt_settings: SttSettings,
    #[cfg(feature = "local-stt")]
    pub tts_settings: TtsSettings,
}

const PROMPT_LOOP_FLAG: &str = "--prompt-loop";
const LOCAL_STT_FLAG: &str = "--local-stt-mode";

// Where the local STT mode writes its log without a `LOG_DIR`: ./logs of the project root it
// is run from, which the container's log is mounted to as well
const LOCAL_STT_LOG_DIR: &str = "logs";

pub fn is_prompt_loop_mode() -> bool {
    std::env::args().any(|arg| arg == PROMPT_LOOP_FLAG)
}

pub fn is_local_stt_mode() -> bool {
    std::env::args().any(|arg| arg == LOCAL_STT_FLAG)
}

/// The modes without HTTP. Both are run from the project root: they read `.env.stdin` after
/// `.env` and find the config under `app/`.
pub fn is_local_mode() -> bool {
    is_prompt_loop_mode() || is_local_stt_mode()
}

impl AppSettings {
    pub fn load() -> AppSettings {
        match cfg!(test) {
            true => dotenvy::from_filename_override(".env.test").expect(".env.test does not exist"),
            false => dotenvy::dotenv().expect(".env does not exist"),
        };

        if is_local_mode() {
            dotenvy::from_filename_override(".env.stdin").expect(".env.stdin does not exist");
        }

        let prefix_stdin = if is_local_mode() { "app/" } else { "" };
        let settings = Config::builder()
            .add_source(config::File::with_name(&format!("{}config/settings.toml", prefix_stdin)))
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
    pub fn llm_url(&self) -> &str {
        &self.llm_url
    }
    pub fn llm_model(&self) -> &str {
        &self.llm_model
    }
    pub fn llm_system_prompt_file(&self) -> &str {
        &self.llm_system_prompt_file
    }
    pub fn log_dir(&self) -> Option<&str> {
        self.log_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .or(is_local_stt_mode().then_some(LOCAL_STT_LOG_DIR))
    }
    pub fn form_submit_url(&self) -> Option<&str> {
        self.form_submit_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
    }
    pub fn llm_api_key(&self) -> Option<&str> {
        self.llm_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }
}
