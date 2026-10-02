use crate::settings::AppSettings;
use serde::Deserialize;
use serde_json::{json, Value};
use std::ops::Range;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum VllmError {
    #[error("vLLM did not answer within {budget_seconds}s")]
    Timeout { budget_seconds: u64 },

    #[error("vLLM request failed: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("vLLM context window full: {0}")]
    ContextWindowFull(String),

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

    /// Sends a system + user prompt, returns the model's raw JSON answer. vLLM only lets
    /// the model write tokens that keep the answer valid for `schema`, named `name`.
    pub async fn query(&self, system: &str, user: &str, name: &str, schema: &Value) -> Result<Answer, VllmError> {
        let body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": name, "schema": schema, "strict": true },
            },
            // Each token's log probability, for how sure the model was of a value.
            "logprobs": true,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });

        let mut request = self.http.post(&self.url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.map_err(|e| self.map_err(e))?;

        // vLLM answers a prompt plus `max_tokens` longer than its context with a 400
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            let body: Value = response.json().await.unwrap_or_default();
            let message = body["error"]["message"].as_str().unwrap_or_default();
            if message.contains("maximum context length") {
                return Err(VllmError::ContextWindowFull(message.to_string()));
            }
            return Err(VllmError::BadResponse(body));
        }

        let response: Value = response
            .error_for_status()
            .map_err(|e| self.map_err(e))?
            .json()
            .await
            .map_err(|e| self.map_err(e))?;

        let choice = &response["choices"][0];
        let Some(content) = choice["message"]["content"].as_str() else {
            return Err(VllmError::BadResponse(response));
        };

        Ok(Answer {
            content: content.to_string(),
            tokens: serde_json::from_value(choice["logprobs"]["content"].clone()).unwrap_or_default(),
        })
    }

    /// Same as [`Self::query`], parsed into a `Value`.
    pub async fn query_json(&self, system: &str, user: &str, name: &str, schema: &Value) -> Result<Value, VllmError> {
        let content = self.query(system, user, name, schema).await?.content;
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

/// The model's answer, with the log probability of every token it wrote.
pub struct Answer {
    pub content: String,

    tokens: Vec<Token>,
}

#[derive(Deserialize)]
struct Token {
    token: String,
    // The token's UTF-8 bytes: `token` itself can end in half a character
    bytes: Option<Vec<u8>>,
    logprob: f64,
}

impl Answer {
    /// How sure the model was of the string it wrote for `key`: the probability of the
    /// tokens that spell it. `None` without logprobs or when `key` has no string value.
    pub fn probability_of(&self, key: &str) -> Option<f64> {
        if self.tokens.is_empty() {
            return None;
        }
        let value = string_value_bytes(&self.content, key)?;

        let mut offset = 0;
        let mut logprob = 0.0;
        for token in &self.tokens {
            let len = token.bytes.as_ref().map_or(token.token.len(), Vec::len);
            if offset < value.end && offset + len > value.start {
                logprob += token.logprob;
            }
            offset += len;
        }

        Some(logprob.exp())
    }
}

/// Where the raw string value of `key` is in `json`, in bytes, without its quotes.
fn string_value_bytes(json: &str, key: &str) -> Option<Range<usize>> {
    let after_key = json.find(&format!("\"{key}\""))? + key.len() + 2;
    let value = json[after_key..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let start = json.len() - value.len();

    let mut escaped = false;
    for (i, c) in json[start..].char_indices() {
        match c {
            '"' if !escaped => return Some(start..start + i),
            '\\' if !escaped => escaped = true,
            _ => escaped = false,
        }
    }

    None
}
