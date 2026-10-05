use crate::app::AppState;
use crate::domain::call::CallAction;
use crate::domain::call_session::CallSession;
use crate::error::ApiError;
use crate::event::caller_spoke::CallerSpokeEvent;
use crate::routes::call_session_middleware::call_session_middleware;
use crate::session::UserSession;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{middleware, Extension, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub const ASSISTANT_ENDPOINT_PATH: &str = "/assistant/handle-request";
pub const MIMIC_CLIENT_PATH: &str = "/mimic-client";
const MIMIC_CLIENT_PAGE: &str = include_str!("../../../client/index.html");

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::<Arc<AppState>>::new()
        .route("/version", get(version))
        .route("/test-session", get(session))
        // Only the assistant needs a call session. The page, `/version` and CORS
        // preflights get through without an `x-call-id`.
        .route(
            ASSISTANT_ENDPOINT_PATH,
            post(handle_assistant_request)
                .route_layer(middleware::from_fn_with_state(state, call_session_middleware)),
        )
        .route(MIMIC_CLIENT_PATH, get(mimic_client))
}

async fn mimic_client() -> Html<&'static str> {
    Html(MIMIC_CLIENT_PAGE)
}

#[derive(Serialize)]
struct VersionResponse {
    version: String,
    environment: String,
}

async fn version(State(state): State<Arc<AppState>>) -> Result<Json<VersionResponse>, StatusCode> {
    Ok(Json(VersionResponse {
        version: state.settings.version().into(),
        environment: state.settings.env().into(),
    }))
}

#[derive(Serialize)]
struct TestSessionResponse {
    user_session: UserSession,
}

#[derive(Deserialize)]
struct TestSessionQuery {
    new_name: Option<String>,
}

async fn session(
    State(_): State<Arc<AppState>>,
    Query(query): Query<TestSessionQuery>,
    req: Request,
) -> Result<Json<TestSessionResponse>, StatusCode> {
    let mut user_session = req
        .extensions()
        .get::<Arc<RwLock<UserSession>>>()
        .expect("No session present in request")
        .write()
        .await;

    match query.new_name {
        Some(new_name) => user_session.change_name(new_name),
        None => (),
    }

    Ok(Json(TestSessionResponse {
        user_session: user_session.clone(),
    }))
}

#[derive(Deserialize)]
struct AssistantRequestPayload {
    request_text: String,

    #[serde(default)]
    language: Option<String>,
}

/// `answer` is what the caller hears; the mimic client shows every other key
/// as a badge, led by `selected_function`, and speaks in `language_detected`.
#[derive(Serialize)]
struct AssistantResponse {
    answer: String,
    // `None` when the turn failed before an intent was matched
    selected_function: Option<String>,
    // How sure the intent matcher was of `selected_function`, 0 to 1
    confidence: Option<f64>,
    language_detected: String,
    action: CallAction,
    reasoning: Option<String>,
}

async fn handle_assistant_request(
    State(state): State<Arc<AppState>>,
    Extension(call_session): Extension<Arc<RwLock<CallSession>>>,
    Json(payload): Json<AssistantRequestPayload>,
) -> Result<Json<AssistantResponse>, ApiError> {
    let utterance = payload.request_text.trim();

    if utterance.is_empty() {
        return Err(ApiError::BadRequest(
            "request_text must not be empty".to_string(),
        ));
    }

    // Turns of one call are handled one at a time by the call lock in `call_session_middleware`.
    let call_id = call_session.read().await.call_id.clone();

    info!(
        call_id,
        utterance,
        language = payload.language.as_deref().unwrap_or("unspecified"),
        "Handling assistant request"
    );

    let mut caller_spoke_event = CallerSpokeEvent::new(
        &call_id,
        utterance,
        Arc::clone(&call_session)
    );

    state.event_dispatcher.dispatch(&mut caller_spoke_event).await;

    let session = call_session.read().await;
    let outcome = &session.call_turn_outcome;
    let selected_function = outcome.caller_intent.map(|intent| intent.to_string());
    let answer = session.data.last_spoken_response.clone().unwrap_or_default();

    info!(
        call_id,
        intent = selected_function,
        confidence = outcome.confidence,
        action = ?outcome.action,
        answer,
        "Answered assistant request"
    );

    Ok(Json(AssistantResponse {
        answer,
        selected_function,
        confidence: outcome.confidence,
        language_detected: session.data.language.iso_code_639_1().to_string(),
        action: outcome.action,
        reasoning: outcome.machine_reasoning.clone(),
    }))
}
