use std::sync::Arc;
use reqwest::Client;
use crate::app::AppState;
use crate::domain::call_session::{CallSession, CallState};
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use crate::event::form_completed::FormCompletedEvent;
use tracing::log::error;
use crate::domain::flow::{FormulatedResponse, IntentMatched};
use crate::domain::flow::form_flow::form_intent_context_handler;
use crate::domain::flow::main_menu_flow::main_menu_intent_context_handler;
use crate::domain::machine::Machine;
use crate::vllm::VllmClient;

pub struct IntentMatchedEvent {
    pub utterance: String,
    pub intent: IntentMatched,
    // Handlers are built once at startup, so the call's session comes with the event.
    call_session: Arc<tokio::sync::RwLock<CallSession>>,
    // So does the app state: the handlers are built before it, as a part of it.
    app_state: Arc<AppState>,
}

impl IntentMatchedEvent {
    pub fn new(
        utterance: &str,
        intent: IntentMatched,
        call_session: Arc<tokio::sync::RwLock<CallSession>>,
        app_state: Arc<AppState>,
    ) -> Self {
        Self {
            utterance: utterance.to_string(),
            intent,
            call_session,
            app_state,
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
                    &self.http_client,
                    &event.app_state
                ).await;

                // A turn that starts a form is the form's first one: its reply asks for the first step.
                let formulator = match session.data.state {
                    CallState::Idle => Machine::main_menu_response_formulator(),
                    CallState::FormInProgress(..) => Machine::form_response_formulator(),
                };

                match formulator
                    .query::<FormulatedResponse>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await {
                    Ok(formulated_response) => {
                        tracing::log::info!("Created FORMULATED_RESPONSE: {:?}", formulated_response);

                        session.save_last_exchange(&event.utterance, &formulated_response.into_spoken_response())
                    }
                    Err(vllm_error) => {
                        error!("vLLM error formulating main menu response: {:?}", vllm_error);

                        session.fail_turn(&event.utterance)
                    }
                }
            },
            IntentMatched::Form(form_intent, form_values) => {

                let completed_form = form_intent_context_handler(
                    &form_intent,
                    &form_values,
                    &event.utterance,
                    &mut session,
                    &self.http_client,
                    &event.app_state
                ).await;

                match Machine::form_response_formulator()
                    .query::<FormulatedResponse>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await {
                    Ok(formulated_response) => {
                        tracing::log::info!("Created FORMULATED_RESPONSE: {:?}", formulated_response);

                        session.save_last_exchange(&event.utterance, &formulated_response.into_spoken_response());

                        // Only now, so a form the caller was not told about is not sent either.
                        if let Some(form) = completed_form {
                            tracing::log::info!("Completed FORM: {}", form.context_value());

                            // The form's callback first: everything that saves comes after it.
                            form.kind.on_completed(&mut session.data.call_memory.hint_map, &form);

                            // Saved with the session, for the main menu to know what the caller has filled out.
                            session.data.call_memory.hint_map.last_filled_out_form = Some(form.clone());

                            let mut form_completed_event = FormCompletedEvent::new(&session.call_id, form);

                            // Released first, like before any other dispatch.
                            drop(session);

                            dispatcher.dispatch(&mut form_completed_event).await;
                        }
                    }
                    Err(vllm_error) => {
                        error!("vLLM error formulating form response: {:?}", vllm_error);

                        session.fail_turn(&event.utterance)
                    }
                }
            },
        }
    }
}