use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use sqlx::types::chrono::Local;
use std::collections::HashMap;

/// Dispatched on every request once its call session is loaded. Handlers fill
/// `initial_context`, which then becomes the session's `initial_context`.
pub struct CallSessionLoaded {
    pub call_id: String,
    pub initial_context: HashMap<String, String>,
}

impl CallSessionLoaded {
    pub fn new(call_id: &str) -> Self {
        Self {
            call_id: call_id.to_string(),
            initial_context: HashMap::new(),
        }
    }
}

impl Event for CallSessionLoaded {}

pub struct InitialContextHandler;

impl EventHandler<CallSessionLoaded> for InitialContextHandler {
    fn handle(&self, event: &mut CallSessionLoaded, _: &Dispatcher) {
        event.initial_context.insert(
            "current_time".to_string(),
            Local::now().format("%A, %Y-%m-%d %H:%M %:z").to_string(),
        );
    }
}
