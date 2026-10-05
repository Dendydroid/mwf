use std::sync::Arc;
use crate::domain::call_session::CallSession;
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use tracing::log::error;
use crate::domain::flow::{FlowContext, IntentMatched};
use crate::domain::flow::form_flow::ExtractedFormIntent;
use crate::domain::flow::main_menu_flow::ExtractedMainMenuIntent;
use crate::domain::machine::Machine;
use crate::event::intent_matched::IntentMatchedEvent;
use crate::vllm::VllmClient;

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
                        error!("vLLM error matching main menu intent: {:?}", vllm_error);

                        event.call_session.write().await.fail_turn(&event.utterance);
                    }
                }
            },
            FlowContext::Form(form_value) => {
                let matched = Machine::form_intent_matcher()
                    .query::<ExtractedFormIntent>(
                        &self.llm,
                        &event.utterance,
                        &session,
                    )
                    .await;

                // Released first: the next handler locks the session itself.
                drop(session);

                match matched {
                    Ok(intent) => {
                        tracing::log::info!("Extracted FORM INTENT context: {:?}", intent);

                        let mut intent_matched_event = IntentMatchedEvent::new(
                            &event.utterance,
                            IntentMatched::Form(intent, form_value.clone()),
                            event.call_session.clone(),
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