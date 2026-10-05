use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use sqlx::types::chrono::Local;
use std::collections::HashMap;

/// Dispatched on every request once its call session is loaded. Handlers fill
/// `initial_context`, which then becomes the session's `call_turn_context`.
pub struct CallSessionLoadedEvent {
    pub call_id: String,
    pub initial_context: HashMap<String, String>,
}

impl CallSessionLoadedEvent {
    pub fn new(call_id: &str) -> Self {
        Self {
            call_id: call_id.to_string(),
            initial_context: HashMap::new(),
        }
    }
}

impl Event for CallSessionLoadedEvent {}

pub struct InitialContextHandler;

impl EventHandler<CallSessionLoadedEvent> for InitialContextHandler {
    async fn handle(&self, event: &mut CallSessionLoadedEvent, _: &Dispatcher) {
        event.initial_context.insert(
            "current_time".to_string(),
            Local::now().format("%A, %Y-%m-%d %H:%M %:z").to_string(),
        );
    }
}
