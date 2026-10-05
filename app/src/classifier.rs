use lingua::Language::{English, German, Russian, Ukrainian};
use lingua::{LanguageDetector, LanguageDetectorBuilder};
use std::sync::LazyLock;

// Only the languages the callers speak: fewer to choose from is fewer to confuse
static DETECTOR: LazyLock<LanguageDetector> =
    LazyLock::new(|| LanguageDetectorBuilder::from_languages(&[English, German, Russian, Ukrainian]).build());

/// The language of `text` as its ISO 639-1 code, or `None` when it cannot be told.
pub fn detect_language(text: &str) -> Option<String> {
    DETECTOR.detect_language_of(text).map(|language| language.iso_code_639_1().to_string())
}
