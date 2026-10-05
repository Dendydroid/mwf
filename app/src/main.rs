mod app;
mod cache;
mod classifier;
mod db;
mod domain;
mod error;
mod event;
mod factory;
mod routes;
mod session;
mod settings;
mod vllm;

#[cfg(test)]
mod tests;

use crate::app::AppState;
use crate::settings::AppSettings;
use crate::vllm::{Answer, VllmClient};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use schemars::JsonSchema;
use sqlx::types::chrono::Local;
use tokio::sync::RwLock;
use tower_http::compression::CompressionLayer;
use tower_http::CompressionLevel::Default;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::log::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;
use crate::domain::call_session::{CallSession, CallTurn};
use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
use crate::event::call_session::CallSessionLoaded;
use crate::event::caller_spoke::CallerSpokeEvent;
use crate::event::context_extracted::HintMap;

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

pub fn is_prompt_loop_mode() -> bool {
    std::env::args().any(|arg| arg == PROMPT_LOOP_FLAG)
}


async fn run_prompt_loop(app_state: Arc<AppState>) -> anyhow::Result<()> {
    info!("Starting prompt loop mode. Type your input and press Enter. Type 'exit' to quit.");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let call_id = &Uuid::new_v4().to_string();
    let ttl_seconds = u64::from(app_state.settings.cache_settings.call_session_ttl_seconds);
    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        stdin.read_line(&mut input)?;

        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        if input.eq_ignore_ascii_case("exit") {
            info!("Exiting prompt loop.");
            break;
        }

        let call_session = Arc::new(
            RwLock::new(
                CallSession::from_or_new(call_id, app_state.cache.clone(), &app_state.settings).await
            )
        );

        let mut session_loaded = CallSessionLoaded::new(&call_id);
        app_state.event_dispatcher.dispatch(&mut session_loaded).await;
        call_session.write().await.call_turn_context = session_loaded.initial_context;

        let mut caller_spoke_event = CallerSpokeEvent::new(
            call_id,
            input,
            Arc::clone(&call_session)
        );

        app_state.event_dispatcher.dispatch(&mut caller_spoke_event).await;

        let actual_response = &call_session.read().await.data.last_spoken_response.clone().unwrap();

        if !call_session.write().await.save(ttl_seconds).await {
            error!("Could not save call session {call_id}");
        }

        println!("Response: {}", actual_response);
    }

    Ok(())
}

// TODO: add readback trait with description

// TODO: analytics collect calls on a separate endpoint and a separate small LLM I wrote extracts all the context defined in context SCHEMA which can be updated on the fly and that info can be used to build graphs, pie charts and whatever to showcase the problematic areas which can be improved.
// Also this can be used for training the trainable models on some edge cases. valuable data.

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

    // Check if we should run in prompt loop mode
    if is_prompt_loop_mode() {
        info!("Running in prompt loop mode");
        return run_prompt_loop(state.clone()).await;
    }

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
