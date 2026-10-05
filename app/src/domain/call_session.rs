use std::default::Default;
use crate::cache::{Cache, CacheKey};
use crate::domain::form::Form;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::sync::OwnedMutexGuard;
use sqlx::types::chrono;
use uuid::Uuid;
use chrono::DateTime;
use lingua::{IsoCode639_1, Language};
use sqlx::types::chrono::Local;
use crate::event::context_extracted::HintMap;
use crate::settings::AppSettings;

const FORM_STATE_CONTEXT_KEY: &str = "form_state";
const LANGUAGE_CONTEXT_KEY: &str = "language";

pub struct CallSession {
    cache: Cache,

    pub call_id: String,

    // Current time, service name, phone number, what was done on backend, info from backend
    pub call_turn_context: HashMap<String, String>,

    pub data: CallData,
}

/// What is kept between the turns of a call: loaded from cache when the session
/// is built and saved back at the end of the request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CallData {
    pub language: Language,

    pub state: CallState,

    pub call_memory: CallMemory,

    pub last_spoken_response: Option<String>,
}

impl CallData {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            state: Default::default(),
            call_memory: Default::default(),
            last_spoken_response: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum CallState {
    #[default]
    Idle,
    FormInProgress(Form),
}

impl CallSession {
    pub async fn from_or_new(
        call_id: &str,
        mut cache: Cache,
        settings: &AppSettings
    ) -> Self {
        let data = cache
            .get_serde(CacheKey::CallSessionKey.get_key(call_id))
            .await
            .unwrap_or_else(|| CallData::new(
                Language::from_iso_code_639_1(
                    &IsoCode639_1::from_str(&settings.call_settings.default_language)
                        .expect("Couldn't resolve default language from settings")
                )
            ));

        Self {
            cache,
            call_id: call_id.into(),
            call_turn_context: Default::default(),
            data,
        }
    }

    pub async fn save(&mut self, ttl_seconds: u64) -> bool {
        self
            .cache
            .set_serde_ex(
                CacheKey::CallSessionKey.get_key(&self.call_id),
                &self.data,
                ttl_seconds,
            )
            .await
    }

    pub fn save_call_turn(&mut self, call_turn: CallTurn) {
        self.data.call_memory.conversation.push(call_turn);
    }

    pub fn merge_filled_hint_map_values(&mut self, hint_map_input: HintMap) {
        self.data.call_memory.hint_map.merge(hint_map_input);
    }

    pub fn get_conversation(&self) -> &[CallTurn] {
        &self.data.call_memory.conversation
    }

    /// Everything that goes into `<context>` this turn.
    pub fn context(&self) -> HashMap<String, String> {
        let mut context = self.call_turn_context.clone();

        if let CallState::FormInProgress(form) = &self.data.state {
            context.insert(FORM_STATE_CONTEXT_KEY.to_string(), form.context_value());
        }

        context.insert(
            LANGUAGE_CONTEXT_KEY.to_string(),
            self.data.language.to_string()
        );

        context
    }

    pub fn set_language(&mut self, language: Language) {
        self.data.language = language;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    // ISO 639-1
    pub language: String,
    pub transcript: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallTurn {
    pub caller_transcript: Transcript,
    pub llm_transcript: Transcript,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallMemory {
    pub conversation: Vec<CallTurn>,
    pub hint_map: HintMap,
}

impl Default for CallMemory {
    fn default() -> Self {
        Self {
            conversation: Vec::default(),
            hint_map: Default::default(),
        }
    }
}

/*
----------------------------------------------------------------------------------------------------
*/

/// One lock per call, so the turns of a call are handled one at a time in this
/// process: a turn takes it before its session is loaded and holds it until the
/// session is saved.
#[derive(Default)]
pub struct CallLocks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl CallLocks {
    pub async fn lock(&self, call_id: &str) -> CallLock<'_> {
        let lock = self.0.lock().unwrap().entry(call_id.to_string()).or_default().clone();

        CallLock {
            locks: self,
            call_id: call_id.to_string(),
            _guard: lock.lock_owned().await,
        }
    }
}

pub struct CallLock<'a> {
    locks: &'a CallLocks,
    call_id: String,
    _guard: OwnedMutexGuard<()>,
}

impl Drop for CallLock<'_> {
    /// Forgets the call's lock once no other turn is waiting for it.
    fn drop(&mut self) {
        let mut locks = self.locks.0.lock().unwrap();

        // The map's reference and this turn's `_guard`, which is dropped after this.
        if locks.get(&self.call_id).is_some_and(|lock| Arc::strong_count(lock) == 2) {
            locks.remove(&self.call_id);
        }
    }
}
