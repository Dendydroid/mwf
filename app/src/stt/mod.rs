/*
    Speech to text. The microphone is cut into utterances by a voice activity detector (Silero)
    and each utterance is written down by a recognizer (Parakeet TDT v3, which finds out the
    language itself). Both run on this machine, through sherpa-onnx.
*/
mod microphone;
pub mod transcriber;

use crate::settings::SttSettings;
use microphone::Microphone;
use tracing::warn;
use transcriber::Transcriber;

/// The sample rate the voice activity detector and the recognizer are made for.
pub const SAMPLE_RATE: i32 = 16_000;

// A microphone that is switched on is never without any noise for this long: five seconds
const DEAD_SILENCE_WINDOWS: usize = 5 * SAMPLE_RATE as usize / transcriber::WINDOW;

pub struct SpeechToText {
    microphone: Microphone,
    transcriber: Transcriber,
    // How many windows in a row held no sound at all. `None` once that has been said
    dead_silent_windows: Option<usize>,
}

impl SpeechToText {
    /// Loads the models and opens the microphone.
    pub fn start(settings: &SttSettings) -> anyhow::Result<Self> {
        Ok(Self {
            transcriber: Transcriber::load(settings)?,
            microphone: Microphone::open()?,
            dead_silent_windows: Some(0),
        })
    }

    /// What the caller says next, once they have gone silent. Blocks until then. While `muted`
    /// says so, nothing the microphone records is heard.
    pub fn listen(&mut self, muted: impl Fn() -> bool) -> anyhow::Result<String> {
        let mut was_muted = false;

        loop {
            // Read while muted too: the device keeps what is not taken from it
            let window = self.microphone.read(transcriber::WINDOW)?;

            if muted() {
                was_muted = true;
                continue;
            }

            // What was being said when the microphone was muted is no part of what is said now
            if was_muted {
                self.transcriber.forget();
                was_muted = false;
            }

            self.notice_dead_silence(&window);

            if let Some(utterance) = self.transcriber.hear(&window) {
                return Ok(utterance);
            }
        }
    }

    /// Says, once, that the microphone gives nothing: the caller would wait for an answer that
    /// cannot come.
    fn notice_dead_silence(&mut self, window: &[f32]) {
        let Some(windows) = self.dead_silent_windows else {
            return;
        };

        self.dead_silent_windows = match window.iter().all(|&sample| sample == 0.0) {
            false => Some(0),
            true if windows + 1 < DEAD_SILENCE_WINDOWS => Some(windows + 1),
            true => {
                warn!(
                    "Nothing at all comes from the microphone `{}`. Is it the default input device you speak into, and is it switched on?",
                    self.microphone.name()
                );

                None
            }
        };
    }

    /// Forgets everything recorded up to now: what the microphone picked up of the assistant's
    /// own voice is not something the caller said.
    pub fn forget(&mut self) {
        self.microphone.skip_recorded();
        self.transcriber.forget();
    }
}
