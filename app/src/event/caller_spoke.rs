use std::str::FromStr;
use std::sync::Arc;
use lingua::{IsoCode639_1, Language};
use tracing::log::{error, info};
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use crate::app::AppState;
use crate::classifier::detect_language;
use crate::domain::call_session::{CallSession, CallState};
use crate::domain::flow::FlowContext;
use crate::domain::flow::form_flow::ExtractedFormValue;
use crate::domain::flow::main_menu_flow::HintMap;
use crate::domain::machine::Machine;
use crate::event::context_extracted::ContextExtractedEvent;
use crate::vllm::VllmClient;

pub struct CallerSpokeEvent {
    call_id: String,
    utterance: String,
    // Handlers are built once at startup, so the call's session comes with the event.
    call_session: Arc<tokio::sync::RwLock<CallSession>>,
    // So does the app state: the handlers are built before it, as a part of it.
    app_state: Arc<AppState>,
}

impl CallerSpokeEvent {
    pub fn new(
        call_id: &str,
        utterance: &str,
        call_session: Arc<tokio::sync::RwLock<CallSession>>,
        app_state: Arc<AppState>,
    ) -> Self {
        Self {
            call_id: call_id.to_string(),
            utterance: utterance.to_string(),
            call_session,
            app_state,
        }
    }
}

impl Event for CallerSpokeEvent {}

pub struct CallerSpokeHandler {
    pub llm: Arc<VllmClient>,
}

impl EventHandler<CallerSpokeEvent> for CallerSpokeHandler {
    async fn handle(&self, event: &mut CallerSpokeEvent, dispatcher: &Dispatcher) {
        // 1. Detect language (Not trusting language provided in request)
        let mut session = event.call_session.write().await;
        if let Some(language_detected_iso_639_1) = detect_language(&event.utterance) {
            session.set_language(
                Language::from_iso_code_639_1(
                    &IsoCode639_1::from_str(
                        &language_detected_iso_639_1
                    ).expect("Bad ISO-639-1 code was detected")
                )
            )
        }
        // 2. Extract context
        let context = match session.data.state {
            // Main menu extraction
            CallState::Idle => {
                match Machine::main_menu_context_extractor()
                    .query::<HintMap>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await {
                    Ok(hint_map) => {
                        info!("Extracted MAIN_MENU context: {:?}", hint_map);

                        Some(FlowContext::MainMenu(hint_map))
                    }
                    Err(vllm_error) => {
                        error!("vLLM error extracting main menu context: {:?}", vllm_error);

                        session.fail_turn(&event.utterance);

                        None
                    }
                }
            },
            // During form extraction
            CallState::FormInProgress(..) => {
                match Machine::form_context_extractor()
                    .query::<ExtractedFormValue>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await {
                    Ok(form_value) => {
                        info!("Extracted FORM context: {:?}", form_value);

                        Some(FlowContext::Form(form_value))
                    }
                    Err(vllm_error) => {
                        error!("vLLM error extracting form context: {:?}", vllm_error);

                        session.fail_turn(&event.utterance);

                        None
                    }
                }
            }
        };

        // Released first: the next handler locks the session itself.
        drop(session);

        // 3. Dispatch context extracted event
        if let Some(context) = context {
            let mut extracted_context = ContextExtractedEvent::new(
                &event.utterance,
                context,
                event.call_session.clone(),
                event.app_state.clone(),
            );

            dispatcher.dispatch(&mut extracted_context).await;
        }
    }
}