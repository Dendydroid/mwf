mod app;
mod cache;
mod db;
mod error;
mod factory;
mod routes;
mod session;
mod settings;
mod vllm;

use crate::app::AppState;
use crate::routes::middleware::session_middleware;
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use axum::{middleware, Router};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::log::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// The flag that runs the terminal loop instead of the HTTP server.
const PROMPT_LOOP_FLAG: &str = "--prompt-loop";

/// Base name of the rolling log files. `tracing_appender` appends the date, so
/// the mounted directory fills with `app.log.2026-09-17`, one per day.
const LOG_FILE_PREFIX: &str = "app.log";

/// Sets up logging, and - when `LOG_DIR` is configured - a second copy of every
/// line into a daily rolling file there.
///
/// Two layers rather than one because they serve different readers: `docker
/// logs` wants the stdout stream, and anything looking at yesterday's incident
/// wants a file that outlives the container. The file layer turns ANSI colour
/// off, which the terminal layer keeps - escape codes are what make a log file
/// unreadable in an editor and unmatchable by `grep`.
///
/// The returned guard must stay alive for as long as the process logs: writing
/// is done on a background thread and dropping the guard is what flushes it.
/// Dropping it early silently truncates the file at that point.
fn init_tracing(settings: &AppSettings) -> Option<WorkerGuard> {
    let (file_layer, guard) = match settings.log_dir() {
        Some(directory) => {
            // The container mounts this path, but a host-side run may not have
            // created it, and a rolling appender panics rather than creating it.
            std::fs::create_dir_all(directory)
                .unwrap_or_else(|e| panic!("Could not create log directory {directory}: {e}"));

            let (writer, guard) = tracing_appender::non_blocking(
                tracing_appender::rolling::daily(directory, LOG_FILE_PREFIX),
            );

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

/// The original read-a-line, classify, print loop - kept, because trying a
/// sentence against the model from a terminal is still the quickest way to see
/// what the prompt does.
///
/// Two things changed. It now goes through [`VllmClient`], so it uses the same
/// prompt, the same sampling and the same parsing as the HTTP endpoint - trying
/// something here now actually tells you what `/assistant/handle-request` would
/// answer. And it no longer runs on every startup: it read `stdin` forever,
/// which meant `main` never reached `axum::serve` and the server never came up.
/// It runs only behind `--prompt-loop`.
async fn ai_prompt_loop(llm: &VllmClient) {
    println!("Talking to {} - Ctrl-C to stop.", llm.model());

    loop {
        let mut next_prompt = String::new();

        println!("Caller says >>>");
        match io::stdin().read_line(&mut next_prompt) {
            // Zero bytes means the input stream closed (Ctrl-D, or a piped file
            // running out). Looping on that spins forever, so it ends the loop.
            Ok(0) => break,
            Ok(_) => (),
            Err(e) => {
                eprintln!("Error reading input: {e}");

                continue;
            }
        }

        let utterance = next_prompt.trim();
        if utterance.is_empty() {
            continue;
        }

        match llm.classify(utterance).await {
            // The fields are printed as the JSON object they are, because the
            // schema decides what is in them - a fixed format string here would
            // stop showing whatever was added to the prompt last.
            Ok(decision) => println!(
                "Model answers >>> {}\n              >>> {}",
                decision.humanlike_sentence_answer,
                decision.fields_as_json()
            ),
            Err(e) => eprintln!("Model failed >>> {e}"),
        }
    }
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

    // The terminal loop is an alternative to serving, not a step before it: it
    // owns stdin and never returns on its own.
    if std::env::args().any(|argument| argument == PROMPT_LOOP_FLAG) {
        ai_prompt_loop(&state.llm).await;

        return Ok(());
    }

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
    info!(
        "Mimic client on http://localhost:{}{}",
        state.settings.http_port(),
        routes::api::MIMIC_CLIENT_PATH
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
