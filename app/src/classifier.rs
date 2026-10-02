use crate::settings::AppSettings;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::time::Duration;

/// The model answers in well under a second on the CPU; this is for a stalled service.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClassifierError {
    #[error("the classifier did not answer within {}s", TIMEOUT.as_secs())]
    Timeout,

    #[error("classifier request failed: {0}")]
    Transport(reqwest::Error),
}

/// Client of the classifier service in `docker/classifier`.
#[derive(Clone)]
pub struct ClassifierClient {
    http: reqwest::Client,
    url: String,
}

/// What the classifier found in an utterance.
#[derive(Deserialize)]
pub struct Extraction {
    pub language: Option<DetectedLanguage>,
    // Every label asked for, the likeliest first
    pub labels: Vec<LabelScore>,
    pub entities: Vec<FoundValue>,
}

#[derive(Deserialize)]
pub struct DetectedLanguage {
    // ISO 639-1
    pub code: String,
    pub confidence: f64,
}

#[derive(Deserialize)]
pub struct LabelScore {
    pub label: String,
    pub score: f64,
}

/// A form field's value as the caller said it.
#[derive(Deserialize)]
pub struct FoundValue {
    pub field: String,
    pub text: String,
    pub score: f64,
    // `text` as YYYY-MM-DD, when it names a day
    pub date: Option<String>,
}

impl ClassifierClient {
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .expect("Could not build the classifier HTTP client"),
            url: format!("{}/extract", settings.classifier_url().trim_end_matches('/')),
        }
    }

    /// Scores every label (name, what the caller does) for `text`, and finds the values of
    /// the form fields (name, description) in it.
    pub async fn extract(
        &self,
        text: &str,
        labels: &[(&str, &str)],
        fields: &[(&str, &str)],
    ) -> Result<Extraction, ClassifierError> {
        let object = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, description)| (name.to_string(), json!(description)))
                .collect::<Map<String, Value>>()
        };
        let body = json!({ "text": text, "labels": object(labels), "fields": object(fields) });

        self.http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(map_err)?
            .json()
            .await
            .map_err(map_err)
    }
}

fn map_err(e: reqwest::Error) -> ClassifierError {
    if e.is_timeout() {
        ClassifierError::Timeout
    } else {
        ClassifierError::Transport(e)
    }
}
