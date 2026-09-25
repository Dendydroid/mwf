use std::default::Default;
use crate::cache::{Cache, CacheKey};
use crate::domain::form::Form;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sqlx::types::chrono;
use uuid::Uuid;
use chrono::DateTime;
use sqlx::types::chrono::Local;
use tracing::log::error;

const CALL_MEMORY_SLOT_KEY: &str = "call_memory";
const CALL_STATE_SLOT_KEY: &str = "call_state";
const FORM_STATE_CONTEXT_KEY: &str = "form_state";

pub struct CallSession {
    cache: Cache,

    pub call_id: String,

    pub slots: HashMap<String, SlotValue>,

    // Current time, service name, phone number, etc.
    pub initial_context: HashMap<String, String>,

    pub state: CallState,

    pub call_memory: CallMemory,
}


#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum SlotValue {
    Number(u64),
    String(String),
    Bool(bool),
    // YYYY-MM-DD
    Iso8601DateString(String),
    JsonObject(String),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum CallState {
    #[default]
    Idle,
    FormInProgress(Form),
}

impl CallSession {
    pub async fn from_or_new(call_id: &str, mut cache: Cache) -> Self {
        let slots: HashMap<String, SlotValue> =
            cache
            .get_serde(CacheKey::CallSessionKey.get_key(call_id))
            .await
            .unwrap_or_default();

        let state = Self::read_json_slot(call_id, &slots, CALL_STATE_SLOT_KEY);
        let call_memory = Self::read_json_slot(call_id, &slots, CALL_MEMORY_SLOT_KEY);

        Self {
            cache,
            call_id: call_id.into(),
            slots,
            initial_context: Default::default(),
            state,
            call_memory,
        }
    }

    pub async fn save_slots(&mut self, ttl_seconds: u64) -> bool {
        // `state` is changed in place, so it is encoded here rather than on every change.
        match serde_json::to_string(&self.state) {
            Ok(encoded) => self.set_slot_value(CALL_STATE_SLOT_KEY, SlotValue::JsonObject(encoded)),
            Err(e) => error!("Could not encode call state for {}: {e}", self.call_id),
        }

        self
            .cache
            .set_serde_ex(
                CacheKey::CallSessionKey.get_key(&self.call_id),
                &self.slots,
                ttl_seconds,
            )
            .await
    }

    pub fn get_slot_value(&self, key: &str) -> Option<&SlotValue> {
        self.slots.get(key)
    }

    pub fn set_slot_value(&mut self, key: &str, value: SlotValue) {
        self.slots.insert(String::from(key), value);
    }

    pub fn save_call_turn(&mut self, call_turn: CallTurn) {
        self.call_memory.conversation.push(call_turn);

        match serde_json::to_string(&self.call_memory) {
            Ok(encoded) => self.set_slot_value(CALL_MEMORY_SLOT_KEY, SlotValue::JsonObject(encoded)),
            Err(e) => error!("Could not encode call memory for {}: {e}", self.call_id),
        }
    }

    pub fn get_conversation(&self) -> &[CallTurn] {
        &self.call_memory.conversation
    }

    /// Everything that goes into `<context>` this turn.
    pub fn context(&self) -> HashMap<String, String> {
        let mut context = self.initial_context.clone();

        if let CallState::FormInProgress(form) = &self.state {
            context.insert(FORM_STATE_CONTEXT_KEY.to_string(), form.context_value());
        }

        context
    }

    fn read_json_slot<T: DeserializeOwned + Default>(
        call_id: &str,
        slots: &HashMap<String, SlotValue>,
        key: &str,
    ) -> T {
        let Some(slot) = slots.get(key) else {
            return T::default();
        };

        let SlotValue::JsonObject(encoded) = slot else {
            error!("Slot {key} for {call_id} holds {slot:?}, expected a JsonObject");

            return T::default();
        };

        serde_json::from_str(encoded).unwrap_or_else(|e| {
            error!("Could not decode slot {key} for {call_id}: {e}");

            T::default()
        })
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
    pub conversation: Vec<CallTurn>
}

impl Default for CallMemory {
    fn default() -> Self {
        Self {
            conversation: Vec::default(),
        }
    }
}
