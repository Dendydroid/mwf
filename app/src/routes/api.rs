use crate::app::AppState;
use crate::error::ApiError;
use crate::session::UserSession;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub const ASSISTANT_ENDPOINT_PATH: &str = "/assistant/handle-request";
pub const MIMIC_CLIENT_PATH: &str = "/mimic-client";
const MIMIC_CLIENT_PAGE: &str = include_str!("../../../client/index.html");

pub fn router() -> Router<Arc<AppState>> {
    Router::<Arc<AppState>>::new()
        .route("/version", get(version))
        .route("/test-session", get(session))
        .route(ASSISTANT_ENDPOINT_PATH, post(handle_assistant_request))
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

#[derive(Serialize)]
struct AssistantResponse {
    answer: String,

    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

async fn handle_assistant_request(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AssistantRequestPayload>,
) -> Result<Json<AssistantResponse>, ApiError> {
    let request_text = payload.request_text.trim();
    if request_text.is_empty() {
        return Err(ApiError::BadRequest(
            "request_text must not be empty".to_string(),
        ));
    }

    info!(
        request_text,
        language = payload.language.as_deref().unwrap_or("unspecified"),
        "handling assistant request"
    );

    let decision = state.llm.classify(request_text).await?;

    // The whole decision, whatever shape the schema currently has, rather than
    // a hand-picked three fields that would silently stop covering it the next
    // time the prompt gains a key. `selected_function` is pulled out as well
    // because it is what anyone reading the log greps for first - and softly,
    // since the schema is free to rename it.
    info!(
        selected_function = decision.field_str("selected_function").unwrap_or("none"),
        decision = %decision.fields_as_json(),
        "classified assistant request"
    );

    Ok(Json(AssistantResponse {
        answer: decision.humanlike_sentence_answer,
        fields: decision.fields,
    }))
}
