/*
    The window of `--local-stt-mode`: the call's id with a button for a new one, and the mute
    button. Everything else about a call is in the logs.
*/
use anyhow::anyhow;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

const WINDOW_TITLE: &str = "Assistant: local STT";
const WINDOW_SIZE: [f32; 2] = [470.0, 96.0];

const CALL_ID_LABEL: &str = "x-call-id";
const CALL_ID_WIDTH: f32 = 290.0;
const NEW_CALL_ID_LABEL: &str = "New";
const MUTE_LABEL: &str = "Mute";
const UNMUTE_LABEL: &str = "Unmute";
const MUTE_BUTTON_HEIGHT: f32 = 44.0;

// The voice loop mutes and closes too, so the window looks at the controls this often
const REFRESH: Duration = Duration::from_millis(200);

/// What the window sets and the voice loop goes by.
pub struct Controls {
    muted: AtomicBool,
    call_id: Mutex<String>,
    closed: AtomicBool,
}

impl Controls {
    /// A new call that hears nothing until it is unmuted.
    pub fn new() -> Self {
        Self {
            muted: AtomicBool::new(true),
            call_id: Mutex::new(new_call_id()),
            closed: AtomicBool::new(false),
        }
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn mute(&self) {
        self.muted.store(true, Ordering::Relaxed);
    }

    /// The call an utterance belongs to: what the field holds, or a new id when it is empty.
    pub fn call_id(&self) -> String {
        let mut call_id = self.call_id.lock().unwrap();

        if call_id.trim().is_empty() {
            *call_id = new_call_id();
        }

        call_id.trim().to_string()
    }

    /// Closes the window, which ends the program.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

fn new_call_id() -> String {
    Uuid::new_v4().to_string()
}

/// Shows the window and blocks until it is closed. Must be called on the main thread.
pub fn run(controls: Arc<Controls>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };

    eframe::run_ui_native(WINDOW_TITLE, options, move |ui, _frame| {
        if controls.closed.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut call_id = controls.call_id.lock().unwrap();

                ui.label(CALL_ID_LABEL);
                ui.add(egui::TextEdit::singleline(&mut *call_id).desired_width(CALL_ID_WIDTH));

                if ui.button(NEW_CALL_ID_LABEL).clicked() {
                    *call_id = new_call_id();
                }
            });

            let muted = controls.is_muted();
            let label = if muted { UNMUTE_LABEL } else { MUTE_LABEL };
            let button = egui::Button::new(label);

            if ui.add_sized([ui.available_width(), MUTE_BUTTON_HEIGHT], button).clicked() {
                controls.muted.store(!muted, Ordering::Relaxed);
            }
        });

        ui.ctx().request_repaint_after(REFRESH);
    })
    .map_err(|error| anyhow!("The window could not be shown: {error}"))
}
