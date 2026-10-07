use anyhow::anyhow;
use rodio::SampleRate;
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsVitsModelConfig};

// What every Piper voice comes with, next to its model
const TOKENS_FILE: &str = "tokens.txt";
const PHONEMES_DIR: &str = "espeak-ng-data";
const MODEL_EXTENSION: &str = "onnx";

/// A Piper voice: one speaker of one language.
pub struct Voice {
    model: OfflineTts,
    sample_rate: SampleRate,
}

impl Voice {
    /// Loads the voice in the directory `dir`.
    pub fn load(dir: &str, num_threads: i32) -> anyhow::Result<Self> {
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: model_file(dir),
                    tokens: Some(format!("{dir}/{TOKENS_FILE}")),
                    data_dir: Some(format!("{dir}/{PHONEMES_DIR}")),
                    ..Default::default()
                },
                num_threads,
                ..Default::default()
            },
            ..Default::default()
        };

        // sherpa-onnx has said on stderr what is wrong with the voice; mostly it is not downloaded
        let not_loaded =
            || anyhow!("Could not load the voice in {dir}. ./run-local-stt.sh downloads the speech models");

        let model = OfflineTts::create(&config).ok_or_else(not_loaded)?;
        let sample_rate = u32::try_from(model.sample_rate())
            .ok()
            .and_then(SampleRate::new)
            .ok_or_else(not_loaded)?;

        Ok(Self { model, sample_rate })
    }

    /// The sample rate of what the voice says, which is mono.
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Says `text`, handing each sentence to `on_sentence` as soon as it is made. Returns
    /// how many samples that was in all, or `None` when the voice could not say it.
    pub fn say(&self, text: &str, mut on_sentence: impl FnMut(&[f32]) + 'static) -> Option<usize> {
        let made = move |samples: &[f32], _progress: f32| {
            on_sentence(samples);

            // Go on with the next sentence
            true
        };

        self.model
            .generate_with_config(text, &GenerationConfig::default(), Some(made))
            .map(|speech| speech.samples().len())
    }
}

/// The model of the Piper voice in `dir`: its one `.onnx` file, named after the voice.
fn model_file(dir: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == MODEL_EXTENSION))
        .map(|path| path.to_string_lossy().into_owned())
}
