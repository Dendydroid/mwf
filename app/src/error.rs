//! One error type, one error body.
//!
//! The two original handlers (`version`, `session`) answer with a bare
//! `StatusCode`, which is fine for endpoints that have essentially one way to
//! fail. The assistant endpoint has three genuinely different ones - the caller
//! sent nothing usable, the inference server is down, the inference server
//! answered with something unparseable - which map to different statuses and to
//! different amounts of detail that are safe to send back.
//!
//! Every failure serialises as `{"error": "..."}`, so the mimic client can
//! handle all of them in one branch instead of guessing at the body per status.

use crate::vllm::VllmError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The request arrived intact but its contents are unusable - an empty
    /// `request_text`, for instance. The message describes what the *caller*
    /// did, so unlike the variant below it is safe to echo back.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// The model could not be reached, or did not answer in the shape the
    /// system prompt asks for. `#[from]` lets handlers use `?` on `classify`.
    #[error("inference failed: {0}")]
    Inference(#[from] VllmError),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // What goes back to the browser is deliberately vaguer than what goes
        // into the log: a `reqwest` error string carries the upstream URL, and a
        // malformed-response error carries whatever the model emitted, which is
        // partly attacker-controlled text. Both belong in the log only.
        let (status, client_message) = match &self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            // A timeout is its own status. It is not "the assistant is down" -
            // the next turn will very likely succeed - and a caller that retries
            // on 504 but gives up on 502 is behaving correctly in both cases.
            ApiError::Inference(VllmError::Timeout { .. }) => (
                StatusCode::GATEWAY_TIMEOUT,
                "the assistant took too long to answer".to_string(),
            ),
            // 502 rather than 500: the failing component is a service behind
            // this one, and that distinction is the whole point when the thing
            // that is down is a GPU box that takes ten minutes to come back.
            ApiError::Inference(_) => (
                StatusCode::BAD_GATEWAY,
                "the assistant is unavailable right now".to_string(),
            ),
        };

        // A 5xx is ours to fix and someone has to look at it; a 4xx is the
        // caller sending us something wrong, which is worth a record but is not
        // an incident.
        if status.is_server_error() {
            tracing::error!(error = %self, %status, "request failed");
        } else {
            tracing::warn!(error = %self, %status, "request rejected");
        }

        (
            status,
            Json(ErrorResponse {
                error: client_message,
            }),
        )
            .into_response()
    }
}
