use std::sync::Arc;
use crate::app::AppState;
use crate::domain::call_session::CallSession;
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use tracing::log::error;
use crate::domain::flow::{FlowContext, IntentMatched};
use crate::domain::flow::form_flow::{CheckedAgreement, ExtractedFormIntent};
use crate::domain::flow::main_menu_flow::ExtractedMainMenuIntent;
use crate::domain::machine::Machine;
use crate::event::intent_matched::IntentMatchedEvent;
use crate::vllm::VllmClient;

pub struct ContextExtractedEvent {
    pub utterance: String,
    pub context: FlowContext,
    // Handlers are built once at startup, so the call's session comes with the event.
    call_session: Arc<tokio::sync::RwLock<CallSession>>,
    // So does the app state: the handlers are built before it, as a part of it.
    app_state: Arc<AppState>,
}

impl ContextExtractedEvent {
    pub fn new(
        utterance: &str,
        context: FlowContext,
        call_session: Arc<tokio::sync::RwLock<CallSession>>,
        app_state: Arc<AppState>,
    ) -> Self {
        Self {
            utterance: utterance.to_string(),
            context,
            call_session,
            app_state,
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

                // Only from here on: the context extractor takes its values for hints the caller gave.
                session.add_recently_completed_form_context();

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
                            event.app_state.clone(),
                        );

                        dispatcher.dispatch(&mut intent_matched_event).await;
                    }
                    Err(vllm_error) => {
                        error!("vLLM error matching main menu intent: {:?}", vllm_error);

                        event.call_session.write().await.fail_turn(&event.utterance);
                    }
                }
            },
            FlowContext::Form(form_values) => {
                let matched = Machine::form_intent_matcher()
                    .query::<ExtractedFormIntent>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await;

                // A question is never an agreement. The matcher takes one for it when it asks whether
                // something is right or confirmed, so what it took for an agreement is checked.
                let matched = match matched {
                    Ok(intent) if intent.is_agreement() => Machine::form_agreement_checker()
                        .query_utterance_alone::<CheckedAgreement>(
                            &self.llm,
                            &event.utterance,
                            &session,
                        )
                        .await
                        .map(|checked| if checked.is_question() { intent.as_question() } else { intent }),
                    matched => matched,
                };

                // Released first: the next handler locks the session itself.
                drop(session);

                match matched {
                    Ok(intent) => {
                        tracing::log::info!("Extracted FORM INTENT context: {:?}", intent);

                        let mut intent_matched_event = IntentMatchedEvent::new(
                            &event.utterance,
                            IntentMatched::Form(intent, form_values.clone()),
                            event.call_session.clone(),
                            event.app_state.clone(),
                        );

                        dispatcher.dispatch(&mut intent_matched_event).await;
                    }
                    Err(vllm_error) => {
                        error!("vLLM error matching form intent: {:?}", vllm_error);

                        event.call_session.write().await.fail_turn(&event.utterance);
                    }
                }
            },
        }
    }
}