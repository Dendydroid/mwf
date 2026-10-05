use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use reqwest::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::domain::call::{CallAction, CallerIntent, FormSupported};
use crate::domain::call_session::{CallSession, CallState, CallTurn, Transcript};
use crate::domain::form::{FormField, FormFieldKind, StepState};
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use crate::event::form::FormCompleted;
use tracing::info;
use tracing::log::error;
use crate::domain::flow::main_menu_flow::{main_menu_intent_context_handler, ExtractedMainMenuIntent, FormulatedResponse};
use crate::domain::machine::{Machine, OutputFormat, ValueSchema};
use crate::event::caller_intent::IntentExtracted;
use crate::event::context_extracted::FlowContext;
use crate::vllm::VllmClient;

pub enum IntentMatched {
    MainMenu(ExtractedMainMenuIntent),
    Form,
}

pub struct IntentMatchedEvent {
    pub utterance: String,
    pub intent: IntentMatched,
    // Handlers are built once at startup, so the call's session comes with the event.
    call_session: Arc<tokio::sync::RwLock<CallSession>>,
}

impl IntentMatchedEvent {
    pub fn new(utterance: &str, intent: IntentMatched, call_session: Arc<tokio::sync::RwLock<CallSession>>) -> Self {
        Self {
            utterance: utterance.to_string(),
            intent,
            call_session,
        }
    }
}

impl Event for IntentMatchedEvent {}

pub struct IntentMatchedHandler {
    pub llm: Arc<VllmClient>,
    pub http_client: Arc<Client>,
}

impl EventHandler<IntentMatchedEvent> for IntentMatchedHandler {
    async fn handle(&self, event: &mut IntentMatchedEvent, dispatcher: &Dispatcher) {
        let mut session = event.call_session.write().await;
        match &event.intent {
            IntentMatched::MainMenu(main_menu_intent) => {

                main_menu_intent_context_handler(
                    &main_menu_intent,
                    &mut session,
                    &self.http_client
                ).await;

                match Machine::main_menu_response_formulator()
                    .query::<FormulatedResponse>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await {
                    Ok(formulated_response) => {
                        tracing::log::info!("Created FORMULATED_RESPONSE: {:?}", formulated_response);

                        save_last_exchange(&mut session, &event.utterance, &formulated_response.into_spoken_response())
                    }
                    Err(vllm_error) => {
                        error!("vLLM error extracting main menu context: {:?}", vllm_error);

                    }
                }
            },
            IntentMatched::Form => todo!(),
        }
    }
}

fn save_last_exchange(session: &mut CallSession, utterance: &str, spoken_response: &str) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    let language = session.data.language.iso_code_639_1().to_string();

    let transcript = |text: &str| Transcript {
        language: language.clone(),
        transcript: text.to_string(),
        timestamp,
    };

    session.save_call_turn(CallTurn {
        caller_transcript: transcript(utterance),
        llm_transcript: transcript(spoken_response),
    });

    session.data.last_spoken_response = spoken_response.to_string();
}