use crate::app::AppState;
use crate::domain::call::{CallAction, CallerIntent, GetInformationSupported};
use crate::domain::call_session::{CallSession, CallTurn, Transcript};
use crate::domain::machine::{ExtractedIntent, FormulatedResponse, Machine};
use crate::error::ApiError;
use crate::event::caller_intent::IntentExtracted;
use crate::routes::call_session_middleware::call_session_middleware;
use crate::session::UserSession;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{middleware, Extension, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

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
    selected_function: String,
    language_detected: String,
    action: CallAction,
    reasoning: String,
}

const BACKEND_CONTEXT_KEY: &str = "backend_context";
const LANGUAGE_CONTEXT_KEY: &str = "language";
const FALLBACK_LANGUAGE: &str = "en";

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
    let mut session = call_session.write().await;
    let call_id = session.call_id.clone();

    info!(
        call_id,
        utterance,
        language = payload.language.as_deref().unwrap_or("unspecified"),
        "Handling assistant request"
    );

    // 1. Machine #1: what does the caller want?
    let extracted: ExtractedIntent = Machine::intent_matcher()
        .query(&state.llm, utterance, &HashMap::new(), &session)
        .await?;

    let intent = extracted.intent().unwrap_or_else(|| {
        warn!(call_id, label = extracted.intent_label(), "Unknown intent, treating it as unsupported");

        CallerIntent::Unsupported
    });
    let language = match extracted.language() {
        "" => FALLBACK_LANGUAGE.to_string(),
        language => language.to_string(),
    };

    // 2. Facts for `GetInformation`, fetched here because event handlers cannot await.
    let information = match intent {
        CallerIntent::GetInformation { selected } => {
            match selected.fetch(&state.http_client).await {
                Ok(information) => Some(information),
                Err(e) => {
                    error!(call_id, "Could not fetch {selected}: {e:#}");

                    None
                }
            }
        }
        _ => None,
    };

    // 3. State changes: forms and steps.
    let previous_state = session.state.clone();

    let mut event = IntentExtracted::new(&call_id, intent, std::mem::take(&mut session.state));
    event.form_field = extracted.form_field().map(str::to_string);
    event.form_field_value = extracted.form_field_value().map(str::to_string);
    event.information = information;

    state.event_dispatcher.dispatch(&mut event);

    session.state = event.state;

    // 4. Machine #2: the answer. A repeat is the last answer word for word.
    let last_answer = session.last_answer().map(str::to_string);
    let answer = match (intent, last_answer) {
        (CallerIntent::Repeat, Some(last_answer)) => last_answer,
        _ => {
            let context = HashMap::from([
                (BACKEND_CONTEXT_KEY.to_string(), event.backend_context),
                (LANGUAGE_CONTEXT_KEY.to_string(), language.clone()),
            ]);

            let machine = match intent {
                CallerIntent::GetInformation { selected: GetInformationSupported::FormInformation } => {
                    Machine::form_informant()
                }
                CallerIntent::GetInformation { selected: GetInformationSupported::CalendarHelp } => {
                    Machine::calendar_refuser()
                }
                _ => Machine::response_formulator(),
            };

            match machine
                .query::<FormulatedResponse>(&state.llm, utterance, &context, &session)
                .await
            {
                Ok(formulated) => formulated.into_spoken_response(),
                Err(e) => {
                    // The caller never hears how this turn went, so none of it is kept.
                    session.state = previous_state;

                    return Err(e.into());
                }
            }
        }
    };

    // 5. Remember the turn.
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let transcript = |text: &str| Transcript {
        language: language.clone(),
        transcript: text.to_string(),
        timestamp,
    };

    session.save_call_turn(CallTurn {
        caller_transcript: transcript(utterance),
        llm_transcript: transcript(&answer),
    });

    info!(call_id, intent = %intent, action = ?event.action, answer, "Answered assistant request");

    Ok(Json(AssistantResponse {
        answer,
        selected_function: intent.to_string(),
        language_detected: language,
        action: event.action,
        reasoning: extracted.reasoning().to_string(),
    }))
}
