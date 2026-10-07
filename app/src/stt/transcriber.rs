use super::SAMPLE_RATE;
use crate::settings::SttSettings;
use anyhow::anyhow;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, VadModelConfig, VoiceActivityDetector};
use std::time::Instant;
use tracing::info;

/// How many samples `hear` takes at a time. The detector's model is made for exactly this many.
pub const WINDOW: usize = 512;

const VAD_THRESHOLD: f32 = 0.5;
// Anything shorter is a click or a cough
const MIN_SPEECH_SECONDS: f32 = 0.25;
// The detector ends an utterance itself when the caller speaks for longer without a pause
const MAX_SPEECH_SECONDS: f32 = 30.0;
// How much sound the detector keeps for the utterances it has not handed out yet
const VAD_BUFFER_SECONDS: f32 = 60.0;

// The detector cuts an utterance close, and a short answer that has lost its first sound is
// written down wrong more often: of 24 short answers in light noise 8 came out right as cut and
// 11 with this much sound from before and after what the detector calls speech
const LEAD_IN: usize = (0.3 * SAMPLE_RATE as f32) as usize;
const TAIL: usize = (0.2 * SAMPLE_RATE as f32) as usize;
// The most sound that is kept for that: the longest utterance and the silence that ends it
const KEPT_SOUND: usize = ((MAX_SPEECH_SECONDS + 5.0) * SAMPLE_RATE as f32) as usize;

const RECOGNIZER_KIND: &str = "nemo_transducer";
const ENCODER_FILE: &str = "encoder.int8.onnx";
const DECODER_FILE: &str = "decoder.int8.onnx";
const JOINER_FILE: &str = "joiner.int8.onnx";
const TOKENS_FILE: &str = "tokens.txt";

/// Cuts 16 kHz mono sound into utterances and writes each one down.
pub struct Transcriber {
    vad: VoiceActivityDetector,
    recognizer: OfflineRecognizer,
    // The latest sound the detector was given, and how many samples it was given before that
    // since it was last reset: it says where an utterance is by counting from there
    heard: Vec<f32>,
    heard_before: usize,
}

impl Transcriber {
    pub fn load(settings: &SttSettings) -> anyhow::Result<Self> {
        let mut vad_config = VadModelConfig::default();
        vad_config.silero_vad.model = Some(settings.vad_model.clone());
        vad_config.silero_vad.threshold = VAD_THRESHOLD;
        vad_config.silero_vad.min_silence_duration = settings.end_of_utterance_silence_seconds;
        vad_config.silero_vad.min_speech_duration = MIN_SPEECH_SECONDS;
        vad_config.silero_vad.max_speech_duration = MAX_SPEECH_SECONDS;
        vad_config.silero_vad.window_size = WINDOW as i32;
        vad_config.sample_rate = SAMPLE_RATE;
        vad_config.num_threads = 1;

        let vad = VoiceActivityDetector::create(&vad_config, VAD_BUFFER_SECONDS)
            .ok_or_else(|| not_loaded("voice activity detector", &settings.vad_model))?;

        let model_file = |name: &str| Some(format!("{}/{name}", settings.model_dir));

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer.encoder = model_file(ENCODER_FILE);
        config.model_config.transducer.decoder = model_file(DECODER_FILE);
        config.model_config.transducer.joiner = model_file(JOINER_FILE);
        config.model_config.tokens = model_file(TOKENS_FILE);
        config.model_config.model_type = Some(RECOGNIZER_KIND.to_string());
        config.model_config.num_threads = settings.num_threads;

        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| not_loaded("recognizer", &settings.model_dir))?;

        Ok(Self {
            vad,
            recognizer,
            heard: Vec::new(),
            heard_before: 0,
        })
    }

    /// Takes the next `WINDOW` samples. Returns what the caller said when these are the ones
    /// that make their silence long enough for the utterance to be over.
    pub fn hear(&mut self, window: &[f32]) -> Option<String> {
        self.vad.accept_waveform(window);
        self.heard.extend_from_slice(window);

        // Dropped in one go now and then, not sample by sample
        if self.heard.len() > 2 * KEPT_SOUND {
            let old = self.heard.len() - KEPT_SOUND;
            self.heard.drain(..old);
            self.heard_before += old;
        }

        while let Some(utterance) = self.vad.front() {
            self.vad.pop();

            let started = Instant::now();
            let text = self.transcribe(self.with_lead_in_and_tail(utterance.start() as usize, utterance.samples().len()));

            info!(
                "Transcribed {:.1} s of speech in {} ms: {text:?}",
                utterance.samples().len() as f32 / SAMPLE_RATE as f32,
                started.elapsed().as_millis()
            );

            // A noise the detector took for speech has no words
            if !text.is_empty() {
                return Some(text);
            }
        }

        None
    }

    /// Forgets the utterance that is being heard and those not handed out yet.
    pub fn forget(&mut self) {
        self.vad.reset();
        self.vad.clear();
        self.heard.clear();
        self.heard_before = 0;
    }

    /// The utterance that starts at the detector's sample `start` and is `length` long, with
    /// the sound just before and after it.
    fn with_lead_in_and_tail(&self, start: usize, length: usize) -> &[f32] {
        let kept = |sample: usize| sample.saturating_sub(self.heard_before).min(self.heard.len());

        &self.heard[kept(start.saturating_sub(LEAD_IN))..kept(start + length + TAIL)]
    }

    fn transcribe(&self, samples: &[f32]) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.recognizer.decode(&stream);

        stream
            .get_result()
            .map(|result| result.text.trim().to_string())
            .unwrap_or_default()
    }
}

/// sherpa-onnx has said on stderr what is wrong with the model; mostly it is not downloaded.
fn not_loaded(model: &str, path: &str) -> anyhow::Error {
    anyhow!("Could not load the {model} from {path}. ./run-local-stt.sh downloads the speech models")
}
