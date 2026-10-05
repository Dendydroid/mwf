use std::sync::Arc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::domain::call::{CallAction, CallerIntent, FormSupported};
use crate::domain::call_session::{CallSession, CallState};
use crate::domain::form::{Form, FormField, FormFieldKind, StepState};
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use crate::event::form::FormCompleted;
use tracing::info;
use tracing::log::error;
use crate::domain::flow::main_menu_flow::ExtractedMainMenuIntent;
use crate::domain::machine::{Machine, OutputFormat, ValueSchema};
use crate::event::caller_intent::IntentExtracted;
use crate::event::intent_matched::{IntentMatched, IntentMatchedEvent};
use crate::vllm::VllmClient;

#[derive(JsonSchema, Deserialize, Serialize, Debug, Default, Clone, PartialEq)]
pub struct HintMap {
    caller_full_name: Option<String>,
    patient_full_name: Option<String>,
    date_of_birth_iso_8601: Option<String>,
    appointment_spoken_date: Option<String>,
    appointment_spoken_time: Option<String>,
}

impl HintMap {
    /// Takes the values `other` has, and keeps its own where `other` has none.
    pub fn merge(&mut self, other: HintMap) {
        self.caller_full_name = other.caller_full_name.or(self.caller_full_name.take());
        self.patient_full_name = other.patient_full_name.or(self.patient_full_name.take());
        self.date_of_birth_iso_8601 = other.date_of_birth_iso_8601.or(self.date_of_birth_iso_8601.take());
        self.appointment_spoken_date = other.appointment_spoken_date.or(self.appointment_spoken_date.take());
        self.appointment_spoken_time = other.appointment_spoken_time.or(self.appointment_spoken_time.take());
    }
}

impl ValueSchema for Option<String> {
    fn valid_value_description(&self) -> &'static str {
        "either non-empty string or null type."
    }
}

impl OutputFormat for HintMap {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(
            [
                ("caller_full_name", &self.caller_full_name as &dyn ValueSchema),
                ("patient_full_name", &self.patient_full_name as &dyn ValueSchema),
                ("date_of_birth_iso_8601", &self.date_of_birth_iso_8601 as &dyn ValueSchema),
                ("appointment_spoken_date", &self.appointment_spoken_date as &dyn ValueSchema),
                ("appointment_spoken_time", &self.appointment_spoken_time as &dyn ValueSchema),
            ]
                .into_iter(),
        )
    }
}

pub enum FlowContext {
    MainMenu(HintMap),
    Form(Form),
}

pub struct ContextExtractedEvent {
    pub utterance: String,
    pub context: FlowContext,
    // Handlers are built once at startup, so the call's session comes with the event.
    call_session: Arc<tokio::sync::RwLock<CallSession>>,
}

impl ContextExtractedEvent {
    pub fn new(utterance: &str, context: FlowContext, call_session: Arc<tokio::sync::RwLock<CallSession>>) -> Self {
        Self {
            utterance: utterance.to_string(),
            context,
            call_session,
        }
    }
}

impl Event for ContextExtractedEvent {}

pub struct ContextExtractedHandler {
    pub llm: Arc<VllmClient>,
}

impl EventHandler<ContextExtractedEvent> for ContextExtractedHandler {
    async fn handle(&self, event: &mut ContextExtractedEvent, dispatcher: &Dispatcher) {
        let mut session = event.call_session.write().await;
        match &event.context {
            FlowContext::MainMenu(hint_map) => {
                session.merge_filled_hint_map_values(hint_map.clone());

                tracing::log::info!("Updated HINT MAP: {:?}", session.data.call_memory.hint_map);

                let matched = Machine::main_menu_intent_matcher()
                    .query::<ExtractedMainMenuIntent>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await;

                // Released first: the next handler locks the session itself.
                drop(session);

                match matched {
                    Ok(intent) => {
                        tracing::log::info!("Extracted INTENT context: {:?}", intent);

                        let mut intent_matched_event = IntentMatchedEvent::new(
                            &event.utterance,
                            IntentMatched::MainMenu(intent),
                            event.call_session.clone(),
                        );

                        dispatcher.dispatch(&mut intent_matched_event).await;
                    }
                    Err(vllm_error) => {
                        error!("vLLM error extracting main menu context: {:?}", vllm_error);

                    }
                }
            },
            FlowContext::Form(_) => todo!(),
        }
    }
}