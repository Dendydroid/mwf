mod app;
mod cache;
mod db;
mod factory;
mod routes;
mod session;
mod settings;

use crate::app::AppState;
use crate::routes::middleware::session_middleware;
use axum::{middleware, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::log::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(AppState::new().await);

    // sqlx::migrate!("../migrations")
    //     .run(&state.db.pool())
    //     .await?;

    info!("Database connected and migrations applied");

    let app = Router::<Arc<AppState>>::new()
        .merge(routes::api::router())
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()) // tighten in prod
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_middleware,
        ))
        .with_state(state.clone());

    let addr: SocketAddr = format!("0.0.0.0:{}", state.settings.http_port()).parse()?;
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
