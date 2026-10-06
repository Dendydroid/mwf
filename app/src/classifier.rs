use lingua::Language::{English, German, Russian, Ukrainian};
use lingua::{LanguageDetector, LanguageDetectorBuilder};
use std::sync::LazyLock;

// Only the languages the callers speak: fewer to choose from is fewer to confuse
static DETECTOR: LazyLock<LanguageDetector> =
    LazyLock::new(|| LanguageDetectorBuilder::from_languages(&[English, German, Russian, Ukrainian]).build());

// A shorter text is not asked about: a bare "13. Juni 1991" is read as English, "голова болит" as Ukrainian
const MIN_WORDS: usize = 3;

/// The language of `text` as its ISO 639-1 code, or `None` when it cannot be told: also for a text
/// of fewer than three words, which a form gets a lot of and the detector reads wrongly too often.
pub fn detect_language(text: &str) -> Option<String> {
    if words(text) < MIN_WORDS {
        return None;
    }

    DETECTOR.detect_language_of(text).map(|language| language.iso_code_639_1().to_string())
}

/// How many words of two letters or more `text` has. Numbers are not words.
fn words(text: &str) -> usize {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().count() >= 2)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_answer_does_not_say_which_language_the_caller_speaks() {
        for short in ["13. Juni 1991", "Hans Müller", "Ja", "John Smith", "голова болит", "3 pm"] {
            assert_eq!(detect_language(short), None, "{short}");
        }

        assert_eq!(detect_language("Er ist am 13. Juni 1991 geboren.").as_deref(), Some("de"));
        assert_eq!(detect_language("The patient's name is John Smith.").as_deref(), Some("en"));
    }
}
