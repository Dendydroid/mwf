//! The vLLM side of the assistant: the prompt, the wire types of the
//! OpenAI-compatible `/chat/completions` API, and the client that joins them.
//!
//! Everything that talks to the model goes through here, so the HTTP endpoint
//! (`routes::api::handle_assistant_request`) and the terminal loop
//! (`main::ai_prompt_loop`) cannot drift apart: they share one system prompt,
//! one set of sampling parameters and one response parser by construction
//! rather than by convention.

use crate::settings::AppSettings;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// The instructions the model is given for every utterance, read from a file.
///
/// **Why a file and not a `const`.** Trying a different prompt used to mean
/// editing Rust and waiting out a release build; the prompt is the part of this
/// service that actually gets iterated on, so it is now
/// `app/prompts/system_prompt.txt` (see `LLM_SYSTEM_PROMPT_FILE`) and an edit is
/// just an edit. The API and the terminal loop still share one prompt, because
/// they share this one object.
///
/// **Why it re-reads per turn.** The file is checked on every classification
/// rather than cached at startup, so an edit applies to the next sentence with
/// nothing to restart - which is the whole point of moving it out of the binary.
/// The cost is one small read next to a GPU call that takes seconds; it is done
/// with `tokio::fs`, so it never blocks the runtime. What it buys back is the
/// edit-and-say-it-again loop that prompt work is made of.
///
/// **Why the last good text is kept.** An editor saving in two steps, a rename,
/// a delete - any of those makes the file unreadable for a moment. Failing turns
/// over that would be absurd, so a failed read logs a warning and the previous
/// text is used. Startup is the exception: a prompt that cannot be read *at all*
/// is a misconfiguration, and the process refuses to start rather than answer
/// callers with an empty system message.
#[derive(Debug)]
pub struct SystemPrompt {
    path: PathBuf,
    /// The last text that loaded successfully. `Arc` so a turn can take a cheap
    /// snapshot and let go of the lock immediately.
    last_good: RwLock<Arc<String>>,
}

impl SystemPrompt {
    /// Reads the prompt once, failing loudly if it cannot be.
    pub fn load(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let text = Self::read(&path)?;

        info!(path = %path.display(), bytes = text.len(), "Loaded system prompt");

        Ok(Self {
            path,
            last_good: RwLock::new(Arc::new(text)),
        })
    }

    /// The prompt to use right now: the file if it can be read, the last text
    /// that could otherwise.
    pub async fn current(&self) -> Arc<String> {
        let text = match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => text,
            Err(e) => {
                warn!(path = %self.path.display(), error = %e,
                      "Could not read the system prompt, using the last one that loaded");

                return self.last_good.read().await.clone();
            }
        };

        let text = text.trim_end();
        if text.is_empty() {
            warn!(path = %self.path.display(),
                  "The system prompt file is empty, using the last one that loaded");

            return self.last_good.read().await.clone();
        }

        {
            // Read lock first: unchanged is the common case by far, and it is
            // the one that must not serialise concurrent turns behind a writer.
            let current = self.last_good.read().await;
            if current.as_str() == text {
                return current.clone();
            }
        }

        let updated = Arc::new(text.to_string());
        *self.last_good.write().await = updated.clone();

        info!(path = %self.path.display(), bytes = updated.len(),
              "System prompt changed on disk, using the new one from this turn on");

        updated
    }

    /// The file's contents, trimmed of the trailing newline every editor adds
    /// and rejected when there is nothing left.
    fn read(path: &Path) -> anyhow::Result<String> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Could not read the system prompt at {}", path.display()))?;

        let text = text.trim_end().to_string();
        if text.is_empty() {
            anyhow::bail!("The system prompt at {} is empty", path.display());
        }

        Ok(text)
    }
}

#[derive(Serialize, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Debug)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Serialize, Debug)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub response_format: ResponseFormat,
    /// Hard ceiling on how much the model may generate for one turn.
    ///
    /// Without it vLLM will happily generate until the end of the context
    /// window, which is what turned one slow turn into a 30-second stall and a
    /// 502: the answer this schema needs is about 50 tokens, but nothing was
    /// stopping a repetition loop from running to 4096. A cap converts that
    /// worst case from "occupies the GPU for minutes" into a truncated answer
    /// that [`VllmError::Truncated`] reports precisely.
    pub max_tokens: u32,
}

/// What the model answers with - the JSON of the [`SystemPrompt`]'s schema,
/// parsed.
///
/// **Only one key is named here, and that is deliberate.** The schema is
/// something to iterate on: adding `anger_level` or `is_hanging_up` to the
/// prompt should make it appear in the API response and in the client, without
/// touching Rust and without a redeploy of anything but the prompt. A struct
/// with one field per key cannot do that - serde drops what it has no field for,
/// so every new key would silently vanish exactly where it was wanted.
///
/// So the split is by *who needs the value*:
///
/// * `humanlike_sentence_answer` is the sentence a caller hears. This service
///   has to know which key that is, because it is the one thing it does with the
///   answer, so it stays named - and a schema that stops asking for it fails the
///   turn loudly ("missing field") rather than answering silence.
/// * Everything else is carried through untouched, in the order the model
///   emitted it (hence `serde_json`'s `preserve_order`, so badges appear in the
///   order the schema lists them rather than alphabetically). `selected_function`
///   and `confidence` are in here too: this service reads them for the log, but
///   it does not need them to *work*, and nothing breaks if they are renamed.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActionDecision {
    pub humanlike_sentence_answer: String,

    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl ActionDecision {
    /// A string-valued field, when the schema currently asks for one under that
    /// name. Used for logging, which is why a missing or differently-typed key
    /// is `None` rather than an error - a log line is never worth failing a
    /// turn for.
    pub fn field_str(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// The decision's non-answer fields as one JSON object, for a log line that
    /// stays correct however the schema changes.
    pub fn fields_as_json(&self) -> Value {
        Value::Object(self.fields.clone())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChoiceMessage {
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Choice {
    pub message: ChoiceMessage,
    /// `"stop"` when the model finished on its own, `"length"` when it hit
    /// `max_tokens`. Optional because it is only a diagnostic here - but it is
    /// the difference between "the model emitted bad JSON" and "we cut the JSON
    /// off ourselves", which are opposite bugs with the same symptom.
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
}

/// Why a turn could not be classified.
///
/// Each variant is a different problem for whoever reads the log, which is the
/// entire point of splitting them: the first 502 this endpoint ever produced
/// logged only `error sending request for url (...)`, because that is all
/// `reqwest`'s `Display` prints - it hides the cause, so a 30-second timeout and
/// a refused connection read identically. Timeouts are now their own variant
/// with the budget in the message.
#[derive(Debug, thiserror::Error)]
pub enum VllmError {
    /// The request outlived `llm_settings.timeout_seconds`. Nearly always a slow
    /// or overloaded inference server rather than a broken one, so it is worth
    /// telling apart from the variant below.
    #[error("vLLM did not answer within {budget_seconds}s")]
    Timeout { budget_seconds: u64 },

    #[error("vLLM request failed: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("vLLM returned no choices")]
    NoChoices,

    /// The answer was cut off at `max_tokens`, so it cannot parse - and it is
    /// our cap that did it, not the model misbehaving. Raise
    /// `llm_settings.max_tokens` if real answers legitimately run this long.
    #[error("vLLM answer was cut off at the {max_tokens}-token cap (content: {content})")]
    Truncated { max_tokens: u32, content: String },

    #[error("vLLM answered with content that is not the expected JSON: {source} (content: {content})")]
    Malformed {
        source: serde_json::Error,
        content: String,
    },
}

/// A thin, cloneable handle on the inference server.
///
/// `reqwest::Client` already owns the connection pool and is cheap to clone, so
/// this is built once into [`crate::app::AppState`] and shared by every request
/// instead of being created per call - a per-call client throws the pool away
/// each time and pays for a fresh TCP handshake on every utterance.
#[derive(Clone)]
pub struct VllmClient {
    /// Shared rather than owned: the client is cloned, and every clone has to
    /// see the same prompt - including the same reload of it.
    prompt: Arc<SystemPrompt>,
    http: reqwest::Client,
    /// Fully-qualified completions URL, resolved once at startup.
    completions_url: String,
    model: String,
    api_key: Option<String>,
    temperature: f32,
    max_tokens: u32,
    /// Kept only so a timeout can name its own budget in the log.
    timeout_seconds: u64,
}

impl VllmClient {
    pub fn new(settings: &AppSettings) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(settings.llm_settings.timeout_seconds))
            .build()
            .expect("Could not build the vLLM HTTP client");

        // Fail here rather than per turn: a server that starts without a usable
        // prompt would answer every caller with an empty system message.
        let prompt = SystemPrompt::load(settings.llm_system_prompt_file())
            .unwrap_or_else(|e| panic!("{e:#}"));

        Self {
            prompt: Arc::new(prompt),
            http,
            completions_url: Self::completions_url(settings.llm_url()),
            model: settings.llm_model().to_string(),
            api_key: settings.llm_api_key().map(str::to_string),
            temperature: settings.llm_settings.temperature,
            max_tokens: settings.llm_settings.max_tokens,
            timeout_seconds: settings.llm_settings.timeout_seconds,
        }
    }

    /// Turns the configured base URL into the completions endpoint.
    ///
    /// `LLM_URL` is written the way the OpenAI-compatible world writes it - the
    /// base ending in `/v1` - so the path is appended here rather than baked
    /// into the environment, where every deployment would have to repeat it
    /// correctly.
    fn completions_url(base_url: &str) -> String {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// One utterance in, one decision out.
    ///
    /// `temperature` is configurable but defaults to 0: this is a routing
    /// decision, not prose, so the same sentence has to land on the same
    /// function every time. `response_format` asks vLLM to constrain decoding to
    /// JSON, which is what makes parsing the content below safe enough to do
    /// directly.
    pub async fn classify(&self, utterance: &str) -> Result<ActionDecision, VllmError> {
        let payload = ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: self.prompt.current().await.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: utterance.to_string(),
                },
            ],
            temperature: self.temperature,
            response_format: ResponseFormat {
                kind: "json_object".to_string(),
            },
            max_tokens: self.max_tokens,
        };

        let mut request = self.http.post(&self.completions_url).json(&payload);

        // vLLM only checks this when it was started with `--api-key`; sending a
        // bearer token it does not expect is harmless, sending none when it does
        // expect one is a 401 - so the header follows the configuration.
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .send()
            .await
            .map_err(|error| self.classify_transport_error(error))?
            .error_for_status()?;
        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|error| self.classify_transport_error(error))?;

        let choice = chat_response
            .choices
            .into_iter()
            .next()
            .ok_or(VllmError::NoChoices)?;

        let truncated = choice.finish_reason.as_deref() == Some("length");
        let content = choice.message.content;

        debug!(content, truncated, "vLLM answered");

        serde_json::from_str(&content).map_err(|source| {
            // Same symptom, two causes. Reporting them as one sends whoever
            // reads the log hunting for a model problem when the fix is a
            // config value.
            if truncated {
                VllmError::Truncated {
                    max_tokens: self.max_tokens,
                    content,
                }
            } else {
                VllmError::Malformed { source, content }
            }
        })
    }

    /// Separates "we gave up waiting" from every other transport failure.
    ///
    /// `reqwest`'s own message for a timeout is `error sending request for url
    /// (...)`, identical to the one for a refused connection, so without this
    /// the log cannot tell a slow GPU from a missing container.
    fn classify_transport_error(&self, error: reqwest::Error) -> VllmError {
        if error.is_timeout() {
            VllmError::Timeout {
                budget_seconds: self.timeout_seconds,
            }
        } else {
            VllmError::Transport(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_url_is_appended_to_the_configured_base() {
        assert_eq!(
            VllmClient::completions_url("http://vllm:8000/v1"),
            "http://vllm:8000/v1/chat/completions"
        );
    }

    #[test]
    fn completions_url_tolerates_a_trailing_slash() {
        assert_eq!(
            VllmClient::completions_url("http://localhost:8000/v1/"),
            "http://localhost:8000/v1/chat/completions"
        );
    }

    #[test]
    fn decision_parses_the_schema_the_system_prompt_asks_for() {
        let decision: ActionDecision = serde_json::from_str(
            r#"{"selected_function":"check_invoices","confidence":0.92,
                "language_detected":"en","humanlike_sentence_answer":"Sure, let me look at your invoices."}"#,
        )
        .expect("the prompt's own schema must parse");

        assert_eq!(
            decision.humanlike_sentence_answer,
            "Sure, let me look at your invoices."
        );
        assert_eq!(decision.field_str("selected_function"), Some("check_invoices"));
        assert_eq!(decision.field_str("language_detected"), Some("en"));
    }

    /// The reason `fields` is a map and not a struct: a key added to the prompt
    /// has to survive the round trip on its own. If this test ever needs a Rust
    /// change to keep passing, extending the schema has stopped being free.
    #[test]
    fn keys_the_code_has_never_heard_of_survive_the_round_trip() {
        let decision: ActionDecision = serde_json::from_str(
            r#"{"selected_function":"small_talk","confidence":0.95,"language_detected":"en",
                "humanlike_sentence_answer":"Hello!","anger_level":0.05,
                "is_greeting":true,"is_hanging_up":false,"escalation_note":"none"}"#,
        )
        .expect("a schema with new keys must still parse");

        assert_eq!(decision.fields["anger_level"], 0.05);
        assert_eq!(decision.fields["is_greeting"], true);
        assert_eq!(decision.fields["is_hanging_up"], false);
        assert_eq!(decision.fields["escalation_note"], "none");

        // And back out again, because the endpoint re-serialises them.
        let json = serde_json::to_value(&decision).unwrap();
        assert_eq!(json["anger_level"], 0.05);
        assert_eq!(json["humanlike_sentence_answer"], "Hello!");
    }

    /// Badges are rendered in the order the keys arrive, so that order has to be
    /// the schema's, not the alphabet's. This is what `serde_json`'s
    /// `preserve_order` feature buys, and it fails loudly if that feature is
    /// ever dropped from `Cargo.toml`.
    #[test]
    fn fields_keep_the_order_the_model_emitted_them_in() {
        let decision: ActionDecision = serde_json::from_str(
            r#"{"selected_function":"small_talk","confidence":0.95,
                "humanlike_sentence_answer":"Hi","anger_level":0.1,"is_greeting":true}"#,
        )
        .unwrap();

        let order: Vec<&str> = decision.fields.keys().map(String::as_str).collect();

        assert_eq!(
            order,
            vec![
                "selected_function",
                "confidence",
                "anger_level",
                "is_greeting"
            ],
            "keys came back in a different order than the schema lists them"
        );
    }

    /// The one key the code does depend on. A schema that drops it has broken
    /// the only thing this service promises, so failing here - loudly, with
    /// serde's own "missing field" message - beats answering with an empty
    /// sentence.
    #[test]
    fn a_decision_without_the_answer_key_is_rejected() {
        let error = serde_json::from_str::<ActionDecision>(
            r#"{"selected_function":"small_talk","confidence":0.95}"#,
        )
        .expect_err("the answer key is not optional");

        assert!(
            error.to_string().contains("humanlike_sentence_answer"),
            "the error should name the missing key, got: {error}"
        );
    }

    #[test]
    fn request_serializes_to_the_openai_chat_shape() {
        let payload = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![Message {
                role: "system".to_string(),
                content: "a system prompt".to_string(),
            }],
            temperature: 0.0,
            response_format: ResponseFormat {
                kind: "json_object".to_string(),
            },
            max_tokens: 256,
        };

        let json = serde_json::to_value(&payload).unwrap();

        // `type` rather than `kind`: the field is renamed because that is what
        // the OpenAI-compatible API expects, and nothing else would be honoured.
        assert_eq!(json["response_format"]["type"], "json_object");
        assert_eq!(json["messages"][0]["role"], "system");
        // Unbounded generation is what let one turn run until the client
        // timeout and answer 502, so the cap has to actually reach vLLM.
        assert_eq!(json["max_tokens"], 256);
    }

    /// A cut-off answer and a model that cannot produce JSON look identical -
    /// both are "this string will not parse" - so `finish_reason` is what tells
    /// them apart, and it has to survive deserialization to do that.
    #[test]
    fn choice_carries_the_finish_reason() {
        let response: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"selected"},"finish_reason":"length"}]}"#,
        )
        .expect("vLLM's own response shape must parse");

        assert_eq!(response.choices[0].finish_reason.as_deref(), Some("length"));
    }

    /// A scratch prompt file, named after the test using it so the tests can
    /// run in parallel without sharing one.
    fn scratch_prompt(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ai_assistant_prompt_{name}.txt"));
        std::fs::write(&path, contents).expect("could not write the scratch prompt");

        path
    }

    /// The point of the whole exercise: edit the file, and the next turn uses
    /// the new text. No restart, no rebuild.
    #[tokio::test]
    async fn an_edited_prompt_file_is_picked_up_on_the_next_turn() {
        let path = scratch_prompt("edited", "first prompt\n");
        let prompt = SystemPrompt::load(&path).expect("the scratch prompt must load");

        assert_eq!(prompt.current().await.as_str(), "first prompt");

        std::fs::write(&path, "second prompt\n").unwrap();

        assert_eq!(
            prompt.current().await.as_str(),
            "second prompt",
            "an edit to the file has to reach the next classification"
        );

        std::fs::remove_file(&path).ok();
    }

    /// Editors save in more than one step, and a prompt that vanishes for a few
    /// milliseconds must not take turns down with it.
    #[tokio::test]
    async fn a_prompt_that_cannot_be_read_falls_back_to_the_last_good_one() {
        let path = scratch_prompt("unreadable", "the good prompt\n");
        let prompt = SystemPrompt::load(&path).expect("the scratch prompt must load");

        assert_eq!(prompt.current().await.as_str(), "the good prompt");

        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            prompt.current().await.as_str(),
            "the good prompt",
            "a missing file must not blank the system prompt"
        );

        // An empty file is the other half of a half-finished save.
        std::fs::write(&path, "   \n").unwrap();

        assert_eq!(
            prompt.current().await.as_str(),
            "the good prompt",
            "an empty file must not blank the system prompt either"
        );

        std::fs::remove_file(&path).ok();
    }

    /// Starting with no usable prompt is a misconfiguration, not something to
    /// paper over: every caller would get an empty system message.
    #[test]
    fn a_missing_prompt_file_fails_at_startup() {
        let error = SystemPrompt::load(std::env::temp_dir().join("ai_assistant_no_such_prompt.txt"))
            .expect_err("a missing prompt file must not start the server");

        assert!(
            error.to_string().contains("Could not read the system prompt"),
            "the error should say what it could not read, got: {error:#}"
        );
    }

    /// The file the repository ships has to be the one the configuration points
    /// at, and it has to still ask for the one key the code requires - this is
    /// what catches a prompt edited into a shape the endpoint cannot answer.
    #[test]
    fn the_shipped_prompt_file_loads_and_asks_for_the_answer_key() {
        let prompt = SystemPrompt::load("prompts/system_prompt.txt")
            .expect("the prompt shipped in app/prompts must load");
        let text = prompt.last_good.try_read().unwrap().clone();

        assert!(
            text.contains("humanlike_sentence_answer"),
            "the shipped prompt no longer asks for the key the endpoint speaks"
        );
    }

    /// The failure that started all of this: a turn that ran past the budget
    /// came back as `error sending request for url (...)`, indistinguishable
    /// from a refused connection, and as a 502 that says "upstream is down"
    /// when it was merely slow.
    ///
    /// The listener accepts the connection and then says nothing, which is what
    /// a vLLM busy generating looks like from here.
    #[tokio::test]
    async fn a_server_that_never_answers_is_reported_as_a_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            // Accepted sockets are held, never answered, and never closed -
            // closing would produce a transport error instead of a timeout.
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let client = VllmClient {
            prompt: Arc::new(
                SystemPrompt::load(scratch_prompt("timeout", "a system prompt\n")).unwrap(),
            ),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(1))
                .build()
                .unwrap(),
            completions_url: format!("http://{address}/v1/chat/completions"),
            model: "test-model".to_string(),
            api_key: None,
            temperature: 0.0,
            max_tokens: 256,
            timeout_seconds: 1,
        };

        let error = client
            .classify("anything")
            .await
            .expect_err("a server that never answers cannot produce a decision");

        assert!(
            matches!(error, VllmError::Timeout { budget_seconds: 1 }),
            "expected a timeout naming its budget, got: {error}"
        );
        assert_eq!(error.to_string(), "vLLM did not answer within 1s");
    }

    /// vLLM omits `finish_reason` on a streamed-in-progress choice, and older
    /// OpenAI-compatible servers omit it entirely - neither is a reason to fail
    /// a turn that otherwise parsed.
    #[test]
    fn choice_without_a_finish_reason_still_parses() {
        let response: ChatResponse =
            serde_json::from_str(r#"{"choices":[{"message":{"content":"{}"}}]}"#)
                .expect("a missing finish_reason must not break parsing");

        assert!(response.choices[0].finish_reason.is_none());
    }
}
