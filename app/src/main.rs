mod app;
mod cache;
mod classifier;
mod db;
mod domain;
mod error;
mod event;
mod factory;
#[cfg(feature = "local-stt")]
mod gui;
mod routes;
mod session;
mod settings;
#[cfg(feature = "local-stt")]
mod stt;
#[cfg(feature = "local-stt")]
mod tts;
mod vllm;
mod vocabulary;

#[cfg(test)]
mod tests;

use crate::app::AppState;
use crate::settings::{is_local_stt_mode, is_prompt_loop_mode, AppSettings};
use axum::Router;
use std::io;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::log::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;
use crate::domain::call_session::CallSession;
use crate::event::call_session_loaded::CallSessionLoadedEvent;
use crate::event::caller_spoke::CallerSpokeEvent;
use crate::vocabulary::vocabulary;

#[cfg(feature = "local-stt")]
use {
    crate::domain::call::CallAction,
    crate::gui::Controls,
    crate::stt::SpeechToText,
    crate::tts::TextToSpeech,
    std::sync::mpsc,
    std::thread,
    std::time::Instant,
    tokio::runtime::Handle,
};

const LOG_FILE_PREFIX: &str = "app.log";
// The local STT mode writes into the directory the container's log is in, so its file has
// a name of its own
const LOCAL_STT_LOG_FILE_SUFFIX: &str = "stt";

/// `app.log.<date>`, and `app.log.<date>.stt` in the local STT mode.
fn log_file_name() -> String {
    match is_local_stt_mode() {
        true => format!("{LOG_FILE_PREFIX}.<date>.{LOCAL_STT_LOG_FILE_SUFFIX}"),
        false => format!("{LOG_FILE_PREFIX}.<date>"),
    }
}

fn init_tracing(settings: &AppSettings) -> Option<WorkerGuard> {
    let (file_layer, guard) = match settings.log_dir() {
        Some(directory) => {
            // The container mounts this path, but a host-side run may not have
            // created it, and a rolling appender panics rather than creating it.
            std::fs::create_dir_all(directory)
                .unwrap_or_else(|e| panic!("Could not create log directory {directory}: {e}"));

            let mut log_file = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix(LOG_FILE_PREFIX);

            if is_local_stt_mode() {
                log_file = log_file.filename_suffix(LOCAL_STT_LOG_FILE_SUFFIX);
            }

            let (writer, guard) = tracing_appender::non_blocking(
                log_file
                    .build(directory)
                    .unwrap_or_else(|e| panic!("Could not open a log file in {directory}: {e}")),
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

/// One turn of a call without HTTP: the two events a request dispatches. Returns the call's
/// session as the turn leaves it, saved.
async fn take_turn(app_state: &Arc<AppState>, call_id: &str, utterance: &str) -> Arc<RwLock<CallSession>> {
    let ttl_seconds = u64::from(app_state.settings.cache_settings.call_session_ttl_seconds);

    let call_session = Arc::new(
        RwLock::new(
            CallSession::from_or_new(call_id, app_state.cache.clone(), &app_state.settings).await
        )
    );

    let mut session_loaded = CallSessionLoadedEvent::new(call_id);
    app_state.event_dispatcher.dispatch(&mut session_loaded).await;
    call_session.write().await.call_turn_context = session_loaded.initial_context;

    let mut caller_spoke_event = CallerSpokeEvent::new(
        call_id,
        utterance,
        Arc::clone(&call_session),
        Arc::clone(app_state)
    );

    app_state.event_dispatcher.dispatch(&mut caller_spoke_event).await;

    if !call_session.write().await.save(ttl_seconds).await {
        error!("Could not save call session {call_id}");
    }

    call_session
}

async fn run_prompt_loop(app_state: Arc<AppState>) -> anyhow::Result<()> {
    info!("Starting prompt loop mode. Type your input and press Enter. Type 'exit' to quit.");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let call_id = &Uuid::new_v4().to_string();
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

        let call_session = take_turn(&app_state, call_id, input).await;

        let actual_response = &call_session.read().await.data.last_spoken_response.clone().unwrap();

        println!("Response: {}", actual_response);
    }

    Ok(())
}

/// The prompt loop with the microphone for the keyboard and the loudspeaker for the screen:
/// what the caller says is a turn of the call whose id is in the window. The call is held on
/// its own thread, because the window needs the main one.
#[cfg(feature = "local-stt")]
fn run_local_stt_mode(app_state: Arc<AppState>) -> anyhow::Result<()> {
    info!("Starting local STT mode: loading the speech models");

    let controls = Arc::new(Controls::new());
    let runtime = Handle::current();
    let (ready, when_ready) = mpsc::channel();

    thread::spawn({
        let controls = Arc::clone(&controls);

        move || {
            if let Err(error) = hold_call(&app_state, &controls, &runtime, ready) {
                error!("Local STT mode stopped: {error:#}");
            }

            // Without the call the window is of no use
            controls.close();
        }
    });

    // Nothing to unmute before the models are loaded and the sound card is open
    if when_ready.recv().is_err() {
        anyhow::bail!("Local STT mode could not start");
    }

    info!("Local STT mode is ready: unmute in the window and speak");

    gui::run(controls)
}

/// Listens, takes the turn, speaks the answer, and listens again. Returns only with an error.
#[cfg(feature = "local-stt")]
fn hold_call(
    app_state: &Arc<AppState>,
    controls: &Controls,
    runtime: &Handle,
    ready: mpsc::Sender<()>,
) -> anyhow::Result<()> {
    let mut stt = SpeechToText::start(&app_state.settings.stt_settings)?;
    let tts = TextToSpeech::start(&app_state.settings.tts_settings)?;

    let _ = ready.send(());

    loop {
        let utterance = stt.listen(|| controls.is_muted())?;
        let call_id = controls.call_id();

        info!("Heard on call {call_id}: {utterance}");

        let started = Instant::now();
        let call_session = runtime.block_on(take_turn(app_state, &call_id, &utterance));

        let session = call_session.blocking_read();
        let answer = session.data.last_spoken_response.clone().unwrap_or_default();
        let language = session.data.language;
        let action = session.call_turn_outcome.action;
        drop(session);

        info!("Answered on call {call_id} in {} ms: {answer}", started.elapsed().as_millis());

        tts.say(&answer, language);

        // What the phone side does on these: it hangs up
        if action != CallAction::Continue {
            info!("Call {call_id} is over ({action:?}): the microphone is muted");
            controls.mute();
        }

        // The microphone heard the answer too
        stt.forget();
    }
}

// TODO: add readback trait with description

// TODO: analytics collect calls on a separate endpoint and a separate small LLM I wrote extracts all the context defined in context SCHEMA which can be updated on the fly and that info can be used to build graphs, pie charts and whatever to showcase the problematic areas which can be improved.
// Also this can be used for training the trainable models on some edge cases. valuable data.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Said before anything is connected to: the window and the speech models are a build feature
    if is_local_stt_mode() && !cfg!(feature = "local-stt") {
        anyhow::bail!("This build has no local STT mode: start it with ./run-local-stt.sh");
    }

    // Settings first, because `LOG_DIR` decides where logging goes and nothing
    // before this point can be logged anywhere. `_log_guard` must outlive every
    // log line, so it is bound for the whole of `main` - see `init_tracing`.
    let settings = AppSettings::load();
    let log_dir = settings.log_dir().map(str::to_string);
    let _log_guard = init_tracing(&settings);

    if let Some(directory) = &log_dir {
        info!("Writing logs to {}/{}", directory, log_file_name());
    }

    // Before the first call: a text the code asks for and the vocabulary does not have stops the app here.
    vocabulary().check();

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

    #[cfg(feature = "local-stt")]
    if is_local_stt_mode() {
        info!("Running in local STT mode");
        return run_local_stt_mode(state.clone());
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
