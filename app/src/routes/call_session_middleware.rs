use crate::app::AppState;
use crate::domain::call_session::CallSession;
use crate::error::ApiError;
use crate::event::call_session::CallSessionLoaded;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::error;

const CALL_IDENTIFIER: &str = "x-call-id";

pub async fn call_session_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let ttl_seconds = u64::from(state.settings.cache_settings.call_session_ttl_seconds);

    let call_id = req
        .headers()
        .get(CALL_IDENTIFIER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|call_id| !call_id.is_empty())
        .map(str::to_string);

    let Some(call_id) = call_id else {
        return ApiError::BadRequest(format!("{CALL_IDENTIFIER} header is required")).into_response();
    };

    // Held until the session is saved, so the call's next turn loads what this one saved.
    let _turn = state.call_locks.lock(&call_id).await;

    let mut call_session = CallSession::from_or_new(&call_id, state.cache.clone()).await;

    let mut loaded = CallSessionLoaded::new(&call_id);
    state.event_dispatcher.dispatch(&mut loaded);
    call_session.initial_context = loaded.initial_context;

    let call_session = Arc::new(RwLock::new(call_session));

    req.extensions_mut().insert(Arc::clone(&call_session));

    let response = next.run(req).await;

    // Saved on every request, not only on change, so the TTL counts from the
    // last turn of the call rather than the first.
    if !call_session.write().await.save_slots(ttl_seconds).await {
        error!("Could not save call session {call_id}");
    }

    response
}
