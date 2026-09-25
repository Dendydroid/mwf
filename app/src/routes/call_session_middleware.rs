use crate::app::AppState;
use crate::session::{SessionToken, UserSession};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::CookieJar;
use std::sync::Arc;
use time::Duration;
use tokio::sync::RwLock;
use tracing::info;

const CALL_IDENTIFIER: &str = "call_id";

pub async fn call_session_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> impl IntoResponse {
    let ttl = Duration::seconds(state.settings.cache_settings.call_session_ttl_seconds as i64);
 // todo

    response
}
