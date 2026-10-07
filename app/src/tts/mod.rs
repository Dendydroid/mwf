/*
    Text to speech. One Piper voice per language, run on this machine through sherpa-onnx, and
    played sentence by sentence: the first one is heard while the next is still being made.
*/
mod speaker;
pub mod voice;

use crate::settings::TtsSettings;
use crate::vocabulary::vocabulary;
use anyhow::anyhow;
use lingua::{IsoCode639_1, Language};
use speaker::Speaker;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
use std::time::Instant;
use tracing::{info, warn};
use voice::Voice;

pub struct TextToSpeech {
    voices: HashMap<Language, Voice>,
    speaker: Speaker,
}

impl TextToSpeech {
    /// Loads the voices and opens the loudspeaker.
    pub fn start(settings: &TtsSettings) -> anyhow::Result<Self> {
        let mut voices = HashMap::new();

        for (language, dir) in &settings.voices {
            let iso_code = IsoCode639_1::from_str(language)
                .map_err(|_| anyhow!("[tts_settings.voices] has a voice for `{language}`, which the callers do not speak"))?;

            voices.insert(Language::from_iso_code_639_1(&iso_code), Voice::load(dir, settings.num_threads)?);
        }

        Ok(Self {
            voices,
            speaker: Speaker::open()?,
        })
    }

    /// Says `text` in the voice of `language` and blocks until it has been heard. A language
    /// that has no voice is not spoken.
    pub fn say(&self, text: &str, language: Language) {
        let Some(voice) = self.voices.get(&language) else {
            warn!("No voice for {language}, so this is not spoken: {text}");
            return;
        };

        let started = Instant::now();
        let first_sound_after = Rc::new(Cell::new(None));

        let play = {
            let speaker = self.speaker.clone();
            let sample_rate = voice.sample_rate();
            let first_sound_after = Rc::clone(&first_sound_after);

            move |sentence: &[f32]| {
                if first_sound_after.get().is_none() {
                    first_sound_after.set(Some(started.elapsed()));
                }

                speaker.play(sentence, sample_rate);
            }
        };

        let Some(samples) = voice.say(&as_the_voice_reads(text, language), play) else {
            warn!("The voice for {language} could not say: {text}");
            return;
        };

        info!(
            "Speaking in {language}: first sound after {} ms, {:.1} s of speech made in {} ms",
            first_sound_after.get().unwrap_or_default().as_millis(),
            samples as f32 / voice.sample_rate().get() as f32,
            started.elapsed().as_millis()
        );

        self.speaker.wait_until_heard();
    }
}

/// `text` as a voice has to be given it. A voice takes the dot of "13. Juni" for the end of a
/// sentence: it says "dreizehn" and pauses. Before a word in lower case it reads the dot as
/// the one of an ordinal, so a month after a day's dot is written small, which sounds the same.
fn as_the_voice_reads(text: &str, language: Language) -> String {
    let mut read = text.to_string();

    for month in vocabulary().dates.months(language) {
        let after_a_day: Vec<usize> = read
            .match_indices(&format!(". {month}"))
            .filter(|(dot, _)| read[..*dot].ends_with(|before: char| before.is_ascii_digit()))
            .map(|(dot, _)| dot + ". ".len())
            .collect();

        // From the last one, so that a month that is longer or shorter in lower case moves none
        for at in after_a_day.into_iter().rev() {
            read.replace_range(at..at + month.len(), &month.to_lowercase());
        }
    }

    read
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_after_a_days_dot_is_written_small() {
        assert_eq!(
            as_the_voice_reads("Ich habe als Geburtsdatum den 13. Juni 1991 notiert. Ist das richtig?", Language::German),
            "Ich habe als Geburtsdatum den 13. juni 1991 notiert. Ist das richtig?"
        );
        assert_eq!(
            as_the_voice_reads("Vom 3. März bis zum 21. März.", Language::German),
            "Vom 3. märz bis zum 21. märz."
        );

        // A sentence that begins with a month, and a date without a day's dot
        for unchanged in ["Das war im Mai. Juni ist besser.", "Zimmer 5. Bitte warten Sie."] {
            assert_eq!(as_the_voice_reads(unchanged, Language::German), unchanged);
        }
        let english = "I have the patient's date of birth as June 13, 1991. Is that right?";
        assert_eq!(as_the_voice_reads(english, Language::English), english);
    }
}
