use crate::app::AppState;
use crate::session::UserSession;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub fn router() -> Router<Arc<AppState>> {
    Router::<Arc<AppState>>::new()
        .route("/version", get(version))
        .route("/test-session", get(session))
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
