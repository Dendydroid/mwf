mod app;
mod cache;
mod db;
mod domain;
mod error;
mod event;
mod factory;
mod routes;
mod session;
mod settings;
mod vllm;

use crate::app::AppState;
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use axum::Router;
use std::any::TypeId;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::log::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const PROMPT_LOOP_FLAG: &str = "--prompt-loop";

const LOG_FILE_PREFIX: &str = "app.log";

fn init_tracing(settings: &AppSettings) -> Option<WorkerGuard> {
    let (file_layer, guard) = match settings.log_dir() {
        Some(directory) => {
            // The container mounts this path, but a host-side run may not have
            // created it, and a rolling appender panics rather than creating it.
            std::fs::create_dir_all(directory)
                .unwrap_or_else(|e| panic!("Could not create log directory {directory}: {e}"));

            let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
                directory,
                LOG_FILE_PREFIX,
            ));

            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_ansi(false),
                ),
                Some(guard),
            )
        }
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()))
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();

    guard
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Settings first, because `LOG_DIR` decides where logging goes and nothing
    // before this point can be logged anywhere. `_log_guard` must outlive every
    // log line, so it is bound for the whole of `main` - see `init_tracing`.
    let settings = AppSettings::load();
    let log_dir = settings.log_dir().map(str::to_string);
    let _log_guard = init_tracing(&settings);

    if let Some(directory) = &log_dir {
        info!("Writing logs to {}/{}.<date>", directory, LOG_FILE_PREFIX);
    }

    let state = Arc::new(AppState::new(settings).await);

    // sqlx::migrate!("../migrations")
    //     .run(&state.db.pool())
    //     .await?;

    info!("Database connected and migrations applied");

    let app = Router::<Arc<AppState>>::new()
        .merge(routes::api::router(state.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()) // tighten in prod
        .with_state(state.clone());

    let addr: SocketAddr = format!("0.0.0.0:{}", state.settings.http_port()).parse()?;
    info!("Listening on {}", addr);
    info!(
        "Mimic client on http://localhost:{}{}",
        state.settings.http_port(),
        routes::api::MIMIC_CLIENT_PATH
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
