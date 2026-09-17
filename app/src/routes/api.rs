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

/// Where a spoken sentence is sent to be understood.
///
/// A named constant because the mimic client page hard-codes the same path, and
/// the test at the bottom of this file asserts the two still agree - a silent
/// rename here would otherwise only show up as a 404 in someone's browser.
pub(crate) const ASSISTANT_ENDPOINT_PATH: &str = "/assistant/handle-request";

/// Where the mimic client page is served from.
pub(crate) const MIMIC_CLIENT_PATH: &str = "/mimic-client";

/// The mimic client, baked into the binary at compile time.
///
/// It lives in `client/` rather than inside the `app` crate because it is a
/// *consumer* of this API, not a part of it; the server only hands the file
/// over.
///
/// It is served from here rather than opened from disk for one hard reason: the
/// browser speech APIs need a "secure context". `https` and `http://localhost`
/// qualify, a `file://` page does not reliably - Chrome will not remember a
/// microphone permission for one. Serving it from this process puts the page on
/// `http://localhost:<APP_PORT>`, same-origin with the endpoint above, so
/// neither the microphone nor CORS is in the way.
///
/// `include_str!` rather than `tower_http`'s `ServeDir`: no extra runtime path
/// that has to resolve identically on a dev machine and inside the container,
/// and Cargo still rebuilds when the HTML changes. The one thing it does need is
/// `client/` being present at build time - see the `COPY` in the `Dockerfile`.
const MIMIC_CLIENT_PAGE: &str = include_str!("../../../client/index.html");

pub fn router() -> Router<Arc<AppState>> {
    Router::<Arc<AppState>>::new()
        .route("/version", get(version))
        .route("/test-session", get(session))
        .route(ASSISTANT_ENDPOINT_PATH, post(handle_assistant_request))
        .route(MIMIC_CLIENT_PATH, get(mimic_client))
}

/// Serves the mimic client page. See [`MIMIC_CLIENT_PAGE`] for why it exists.
///
/// `Html` is what sets `content-type: text/html; charset=utf-8`; a bare `&str`
/// would be sent as `text/plain` and the browser would show the source instead
/// of running it.
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

/// One thing the caller said, in their own words.
#[derive(Deserialize)]
struct AssistantRequestPayload {
    request_text: String,

    /// BCP-47 tag of the language the caller is speaking, e.g. `"en-US"`.
    ///
    /// Optional, and it deliberately does **not** change the prompt: the model
    /// detects the language itself and reports it back as `language_detected`,
    /// so making this steer anything would be two sources of truth for the same
    /// fact. It is accepted and logged because the browser has to pick a
    /// recognition language before it will listen anyway, so it knows something
    /// we otherwise would not - which is worth having when a transcript and the
    /// detected language disagree.
    #[serde(default)]
    language: Option<String>,
}

/// One classified turn.
///
/// `answer` is the sentence a caller hears. Everything else the schema asked for
/// is flattened in beside it, under the model's own key names and in the
/// model's own order - so a prompt that starts asking for `anger_level` answers
/// with `anger_level` here, with no change to this file.
///
/// That is the point: the schema in `app/prompts/system_prompt.txt` is the thing
/// being iterated on, and a response struct that re-declares each key would make
/// every experiment a Rust edit and a rebuild.
///
/// ```json
/// {
///   "answer": "Sure, I can help with that.",
///   "selected_function": "cancel_subscription",
///   "confidence": 0.95,
///   "language_detected": "en",
///   "anger_level": 0.1
/// }
/// ```
///
/// The one name to avoid in the schema is `answer` itself, which would collide
/// with the field above.
#[derive(Serialize)]
struct AssistantResponse {
    answer: String,

    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

/// Feed one sentence to the model under the shared system prompt and answer
/// with what it decided.
///
/// The endpoint is intentionally stateless: the prompt classifies a single
/// utterance against a fixed function list, so a turn carries everything the
/// decision needs. The cookie session that the middleware attaches is therefore
/// not read here - when this grows into multi-question flows, that session is
/// where the collected answers would go.
///
/// Note what is *not* an error. An utterance the model cannot place is a normal
/// outcome on a phone line, not a failure: it comes back as `fallback_human`
/// with a 200. Only an unusable request or a broken inference server becomes an
/// [`ApiError`].
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The page is a separate file that no compiler checks against this router,
    /// so the one thing tying them together is that both spell the path the same
    /// way. This is that tie, made to fail loudly.
    #[test]
    fn mimic_client_page_posts_to_the_endpoint_this_router_serves() {
        assert!(
            MIMIC_CLIENT_PAGE.contains(ASSISTANT_ENDPOINT_PATH),
            "client/index.html no longer references {ASSISTANT_ENDPOINT_PATH}"
        );
    }
}
