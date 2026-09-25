use crate::settings::AppSettings;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum VllmError {
    #[error("vLLM did not answer within {budget_seconds}s")]
    Timeout { budget_seconds: u64 },

    #[error("vLLM request failed: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("vLLM returned an unexpected response: {0}")]
    BadResponse(Value),

    #[error("vLLM answered with invalid JSON: {source} (content: {content})")]
    Malformed {
        source: serde_json::Error,
        content: String,
    },
}

#[derive(Clone)]
pub struct VllmClient {
    http: reqwest::Client,
    url: String,
    model: String,
    api_key: Option<String>,
    temperature: f32,
    max_tokens: u32,
    timeout_seconds: u64,
}

impl VllmClient {
    pub fn new(settings: &AppSettings) -> Self {
        let timeout_seconds = settings.llm_settings.timeout_seconds;

        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_seconds))
                .build()
                .expect("Could not build the vLLM HTTP client"),
            url: format!("{}/chat/completions", settings.llm_url().trim_end_matches('/')),
            model: settings.llm_model().to_string(),
            api_key: settings.llm_api_key().map(str::to_string),
            temperature: settings.llm_settings.temperature,
            max_tokens: settings.llm_settings.max_tokens,
            timeout_seconds,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Sends a system + user prompt, returns the model's raw JSON answer as a string.
    pub async fn query(&self, system: &str, user: &str) -> Result<String, VllmError> {
        let body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "response_format": { "type": "json_object" },
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });

        let mut request = self.http.post(&self.url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response: Value = request
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| self.map_err(e))?
            .json()
            .await
            .map_err(|e| self.map_err(e))?;

        match response["choices"][0]["message"]["content"].as_str() {
            Some(content) => Ok(content.to_string()),
            None => Err(VllmError::BadResponse(response)),
        }
    }

    /// Same as [`Self::query`], parsed into a `Value`.
    pub async fn query_json(&self, system: &str, user: &str) -> Result<Value, VllmError> {
        let content = self.query(system, user).await?;
        serde_json::from_str(&content).map_err(|source| VllmError::Malformed { source, content })
    }

    fn map_err(&self, e: reqwest::Error) -> VllmError {
        if e.is_timeout() {
            VllmError::Timeout { budget_seconds: self.timeout_seconds }
        } else {
            VllmError::Transport(e)
        }
    }
}
