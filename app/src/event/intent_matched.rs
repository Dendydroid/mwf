use std::sync::Arc;
use reqwest::Client;
use crate::app::AppState;
use crate::domain::call_session::CallSession;
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use crate::event::form_completed::FormCompletedEvent;
use tracing::log::error;
use crate::domain::flow::{sentences, FormulatedResponse, IntentMatched, Reply};
use crate::domain::flow::form_flow::{form_intent_context_handler, FormulatedAnswer};
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

                let reply = main_menu_intent_context_handler(
                    &main_menu_intent,
                    &mut session,
                    &self.http_client,
                    &event.app_state
                ).await;

                let spoken_response = match reply {
                    // The turn that starts a form: code says that it is started and asks for its first step.
                    Reply::Said(spoken_response) => Ok(spoken_response),
                    Reply::Formulated { .. } => Machine::main_menu_response_formulator()
                        .query::<FormulatedResponse>(
                            &self.llm,
                            &event.utterance,
                            &session,
                        )
                        .await
                        .map(FormulatedResponse::into_spoken_response),
                };

                match spoken_response {
                    Ok(spoken_response) => {
                        tracing::log::info!("Created SPOKEN_RESPONSE: {:?}", spoken_response);

                        session.save_last_exchange(&event.utterance, &spoken_response)
                    }
                    Err(vllm_error) => {
                        error!("vLLM error formulating main menu response: {:?}", vllm_error);

                        session.fail_turn(&event.utterance)
                    }
                }
            },
            IntentMatched::Form(form_intent, form_values) => {

                let form_turn = form_intent_context_handler(
                    &form_intent,
                    &form_values,
                    &event.utterance,
                    &mut session,
                    &self.http_client,
                    &event.app_state
                ).await;

                let spoken_response = match form_turn.reply {
                    // Every reply of a form is code's: what the turn has to say, then the step's question.
                    Reply::Said(spoken_response) => Ok(spoken_response),
                    // All but the answer to a question about the form, which the formulator words. The
                    // step's question is still code's, said after it.
                    Reply::Formulated { then } => Machine::form_response_formulator(session.data.language)
                        .query::<FormulatedAnswer>(
                            &self.llm,
                            &event.utterance,
                            &session,
                        )
                        .await
                        .map(|answer| sentences(&[&answer.into_spoken_answer(), then.as_deref().unwrap_or_default()])),
                };

                match spoken_response {
                    Ok(spoken_response) => {
                        tracing::log::info!("Created SPOKEN_RESPONSE: {:?}", spoken_response);

                        session.save_last_exchange(&event.utterance, &spoken_response);

                        // Only now, so a form the caller was not told about is not sent either.
                        if let Some(form) = form_turn.completed {
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
