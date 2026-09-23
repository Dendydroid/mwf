use std::default::Default;
use crate::cache::{Cache, CacheKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sqlx::types::chrono;
use uuid::Uuid;
use chrono::DateTime;
use sqlx::types::chrono::Local;
use tracing::log::error;

const CALL_MEMORY_SLOT_KEY: &str = "call_memory";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CallState {
    Idle,
    FormInProgress,
}

impl CallSession {
    pub async fn from_or_new(call_id: &str, mut cache: Cache) -> Self {
        let slots: HashMap<String, SlotValue> =
            cache
            .get_serde(CacheKey::CallSessionKey.get_key(call_id))
            .await
            .unwrap_or_default();

        let call_memory = Self::read_call_memory(call_id, &slots);

        Self {
            cache,
            call_id: call_id.into(),
            slots,
            initial_context: Default::default(),
            state: CallState::Idle,
            call_memory,
        }
    }

    pub async fn save_slots(&mut self, ttl_seconds: u64) -> bool {
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

    fn read_call_memory(call_id: &str, slots: &HashMap<String, SlotValue>) -> CallMemory {
        let Some(slot) = slots.get(CALL_MEMORY_SLOT_KEY) else {
            return CallMemory::default();
        };

        let SlotValue::JsonObject(encoded) = slot else {
            error!("Call memory slot for {call_id} holds {slot:?}, expected a JsonObject");

            return CallMemory::default();
        };

        serde_json::from_str(encoded).unwrap_or_else(|e| {
            error!("Could not decode call memory for {call_id}: {e}");

            CallMemory::default()
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
