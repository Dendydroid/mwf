
/*
    A single machine has a role, an output rule for each allowed value, other rules and produces strict json response output
*/
use std::collections::HashMap;
use std::fmt::Display;
use std::io;
use quick_xml::escape::partial_escape;
use quick_xml::events::BytesText;
use quick_xml::Writer;
use schemars::generate::SchemaSettings;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::info;
use crate::domain::call::CallerIntent;
use crate::domain::call_session::{CallSession, CallTurn};
use crate::vllm::{VllmClient, VllmError};

pub struct Machine {
    role: String,

    rules: Vec<String>,
}

impl Machine {
    /// `context` is added to the session's own context (initial context, form
    /// state), and wins where both have the same key.
    pub async fn query<T>(
        &self,
        llm: &VllmClient,
        input: &str,
        context: &HashMap<String, String>,
        call_session: &CallSession,
    ) -> Result<T, VllmError>
    where
        T: OutputFormat + Default + JsonSchema + DeserializeOwned,
    {
        let mut full_context = call_session.context();
        full_context.extend(context.iter().map(|(key, value)| (key.clone(), value.clone())));

        let system = self
            .render_system_prompt::<T>(&full_context, call_session.get_conversation())
            .expect("Could not render the system prompt");
        let user = render_xml(|w| {
            w.create_element("utterance").write_text_content(escaped(input))?;
            Ok(())
        })
        .expect("Could not render the utterance");

        let call_id = call_session.call_id.as_str();
        let output = std::any::type_name::<T>().rsplit("::").next().unwrap_or_default();

        info!(call_id, output, "Machine prompt\n{system}\n{user}");

        let content = llm.query(&system, &user).await?;

        info!(call_id, output, content = content.as_str(), "Machine answered");

        serde_json::from_str(&content).map_err(|source| VllmError::Malformed { source, content })
    }

    /// `role` and `rules` are written verbatim, so they can refer to other
    /// sections by tag (`<allowed_values>`). Everything else is escaped, since
    /// it comes from the caller or a backend and must not be able to close a tag.
    fn render_system_prompt<T>(
        &self,
        context: &HashMap<String, String>,
        conversation: &[CallTurn],
    ) -> io::Result<String>
    where
        T: OutputFormat + Default + JsonSchema,
    {
        let output = T::default();
        let fields: Vec<_> = output.iter_schemas().collect();

        render_xml(|w| {
            w.create_element("role").write_text_content(verbatim(&self.role))?;

            w.create_element("rules").write_inner_content(|w| {
                for (name, schema) in &fields {
                    let mut rule = format!("Set `{name}` to: {}", schema.valid_value_description());
                    if schema.uses_strict_allowed_values() {
                        rule.push_str(&format!(". Use only a value listed in <allowed_values><{name}>"));
                    }
                    w.create_element("rule").write_text_content(verbatim(&rule))?;
                }

                for rule in &self.rules {
                    w.create_element("rule").write_text_content(verbatim(rule))?;
                }

                Ok(())
            })?;

            if fields.iter().any(|(_, schema)| schema.uses_strict_allowed_values()) {
                w.create_element("allowed_values").write_inner_content(|w| {
                    for (name, schema) in fields.iter().filter(|(_, schema)| schema.uses_strict_allowed_values()) {
                        w.create_element(*name).write_inner_content(|w| {
                            for value in schema.allowed_values() {
                                w.create_element("allowed_value")
                                    .write_text_content(escaped(&value.to_string()))?;
                            }

                            Ok(())
                        })?;
                    }

                    Ok(())
                })?;
            }

            w.create_element("json_output_format")
                .write_text_content(escaped(&format!("\n{}\n", json_output_format::<T>())))?;

            if !conversation.is_empty() {
                w.create_element("conversation_history").write_inner_content(|w| {
                    for turn in conversation {
                        w.write_serializable("call_turn", turn).map_err(io::Error::other)?;
                    }

                    Ok(())
                })?;
            }

            if !context.is_empty() {
                // Sorted, so the same context always renders the same prompt.
                let mut entries: Vec<_> = context.iter().collect();
                entries.sort();

                w.create_element("context").write_inner_content(|w| {
                    for (key, value) in entries {
                        w.create_element(key.as_str()).write_text_content(escaped(value))?;
                    }

                    Ok(())
                })?;
            }

            Ok(())
        })
    }
}

fn render_xml(write: impl FnOnce(&mut Writer<Vec<u8>>) -> io::Result<()>) -> io::Result<String> {
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    write(&mut writer)?;

    String::from_utf8(writer.into_inner()).map_err(io::Error::other)
}

/// Escapes only `<`, `>` and `&`: quotes stay readable for the model.
fn escaped(text: &str) -> BytesText<'_> {
    BytesText::from_escaped(partial_escape(text))
}

fn verbatim(text: &str) -> BytesText<'_> {
    BytesText::from_escaped(text)
}

/// Each output field with its JSON type, e.g. `{"detected_language": "string"}`.
fn json_output_format<T: JsonSchema>() -> String {
    let schema = SchemaSettings::default()
        .with(|s| s.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();

    let format: Map<String, Value> = schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, property)| {
            // An `Option` field is `["string", "null"]` here, which the model copies as an
            // array. It is left out when it has no value, so only its own type is shown.
            let kind = match property.get("type") {
                Some(Value::Array(kinds)) => kinds
                    .iter()
                    .find(|kind| kind.as_str() != Some("null"))
                    .cloned()
                    .unwrap_or_default(),
                Some(kind) => kind.clone(),
                None => property.clone(),
            };
            (name.clone(), kind)
        })
        .collect();

    format!("{:#}", Value::Object(format))
}

pub enum AllowedValue {
    PositiveNumber(u64),
    Number(i64),
    String(String),
    Bool(bool),
}

impl Display for AllowedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllowedValue::PositiveNumber(n) => write!(f, "{n}"),
            AllowedValue::Number(n) => write!(f, "{n}"),
            AllowedValue::String(s) => f.write_str(s),
            AllowedValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

pub trait ValueSchema {
    fn allowed_values(&self) -> Vec<AllowedValue> {
        vec![]
    }

    fn uses_strict_allowed_values(&self) -> bool {
        !self.allowed_values().is_empty()
    }

    fn valid_value_description(&self) -> &'static str;
}

pub trait OutputFormat {
    /// Each field's JSON key with its schema.
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_>;
}


/*
----------------------------------------------------------------------------------------------------
*/

// Output for machine #1 intent matcher

#[derive(JsonSchema, Deserialize, Default)]
struct Reasoning(String);

impl ValueSchema for Reasoning {
    fn valid_value_description(&self) -> &'static str {
        "A brief 1-sentence analysis based on rules and context"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
struct DetectedLanguage(String);

impl ValueSchema for DetectedLanguage {
    fn valid_value_description(&self) -> &'static str {
        "ISO 639-1 two-letter code"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
struct IntendedAction(String);

impl ValueSchema for IntendedAction {
    fn allowed_values(&self) -> Vec<AllowedValue> {
        CallerIntent::all()
            .iter()
            .map(|i| AllowedValue::String(String::from(format!("{}", i))))
            .collect()
    }

    fn valid_value_description(&self) -> &'static str {
        "Caller's intended action, or `unsupported` when none of the other values fits"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
struct FormFieldName(Option<String>);

impl ValueSchema for FormFieldName {
    fn valid_value_description(&self) -> &'static str {
        "Name of the field in <form_state> that `form_field_value` is for. Set it only when the utterance gives \
        a form value, otherwise leave `form_field` out"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
struct ExtractedValue(Option<String>);

impl ValueSchema for ExtractedValue {
    fn valid_value_description(&self) -> &'static str {
        "The value the caller gave for that field. Set it only when the caller gave one, otherwise leave \
        `form_field_value` out. Write dates as YYYY-MM-DD, resolving relative \
        dates such as \"next Tuesday\" against `current_time` in <context>, numbers as digits, yes and no \
        as true and false, and text as the caller said it"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
pub struct ExtractedIntent {
    machine_reasoning: Reasoning,
    detected_language: DetectedLanguage,
    caller_intent: IntendedAction,
    #[serde(default)]
    form_field: FormFieldName,
    #[serde(default)]
    form_field_value: ExtractedValue,
}

impl ExtractedIntent {
    pub fn reasoning(&self) -> &str {
        &self.machine_reasoning.0
    }

    pub fn language(&self) -> &str {
        &self.detected_language.0
    }

    pub fn intent_label(&self) -> &str {
        &self.caller_intent.0
    }

    /// `None` when the model answered with a label that is not an intent.
    pub fn intent(&self) -> Option<CallerIntent> {
        CallerIntent::from_label(&self.caller_intent.0)
    }

    pub fn form_field(&self) -> Option<&str> {
        self.form_field.0.as_deref()
    }

    pub fn form_field_value(&self) -> Option<&str> {
        self.form_field_value.0.as_deref()
    }
}

impl OutputFormat for ExtractedIntent {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(vec![
            ("machine_reasoning", &self.machine_reasoning as &dyn ValueSchema),
            ("detected_language", &self.detected_language as &dyn ValueSchema),
            ("caller_intent", &self.caller_intent as &dyn ValueSchema),
            ("form_field", &self.form_field as &dyn ValueSchema),
            ("form_field_value", &self.form_field_value as &dyn ValueSchema),
        ].into_iter())
    }
}

impl Machine {
    /// Machine #1: what the caller wants, and which form value they gave.
    pub fn intent_matcher() -> Self {
        Self {
            role: "You are the intent matcher of a phone assistant. Classify what the caller wants with \
                their latest <utterance>, detect its language and extract the form value it gives."
                .to_string(),
            rules: [
                "Classify only the latest <utterance>. Use <conversation_history> to understand short or \
                elliptical answers such as \"yes\", \"the second one\" or a bare name",
                "Use a `get_information[...]` or `start_form[...]` value only when the request matches it \
                exactly. A similar but different request, such as another city, another currency or another \
                kind of booking, is `unsupported`",
                "When <form_state> is not in <context>, never use `provide_form_field_value`, \
                `correct_form_field_value`, `refer_to_context_for_form_field_value` or `cancel_form`",
                "When the `current_field` of <form_state> is `awaiting_confirmation`: agreement is \
                `confirm_yes`, a plain denial is `confirm_no`, and a denial that also gives the right value is \
                `correct_form_field_value` with that value",
                "When the `current_field` of <form_state> is `queued`, an answer to it is \
                `provide_form_field_value`, and so is a plain yes or no when its kind is `bool`",
                "A new value for a field that is already `completed` is `correct_form_field_value`, with \
                `form_field` set to that field",
                "A value the caller points to instead of saying it, such as \"the same as before\", is \
                `refer_to_context_for_form_field_value`, with the value resolved from <conversation_history>",
                "A request unrelated to the form in progress is classified on its own, as if there were no form",
                "`repeat` when the caller did not hear or understand the last answer, `end_call` when they say \
                goodbye or answer that they need nothing else, `transfer_to_human` when they ask for a person, \
                an operator or an agent",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}


/*
----------------------------------------------------------------------------------------------------
*/

// Output for machine #2 response formulator

#[derive(JsonSchema, Deserialize, Default)]
struct SpokenResponse(String);

impl ValueSchema for SpokenResponse {
    fn valid_value_description(&self) -> &'static str {
        "A natural, concise spoken reply of 1 to 2 sentences in plain text for text-to-speech, \
        with numbers, currencies and dates written the way they are spoken"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
pub struct FormulatedResponse {
    spoken_response: SpokenResponse,
}

impl FormulatedResponse {
    pub fn into_spoken_response(self) -> String {
        self.spoken_response.0
    }
}

impl OutputFormat for FormulatedResponse {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(vec![
            ("spoken_response", &self.spoken_response as &dyn ValueSchema),
        ].into_iter())
    }
}

impl Machine {
    /// Machine #2: the sentence the caller hears.
    pub fn response_formulator() -> Self {
        Self {
            role: "You are the voice of a phone assistant. Reply to the caller's <utterance> from what \
                <context> gives you, as natural speech."
                .to_string(),
            rules: [
                "Reply in the language given by <language> in <context>",
                "Rely strictly on the facts and instructions in <backend_context>; never invent details it \
                does not state",
                "Never ask more than one question in a reply",
                "When there is no <form_state> and you delivered information or a form was completed or \
                cancelled, ask whether the caller needs anything else",
                "Say forms, fields and information in plain words, never as their snake_case names",
                "No markdown, lists, special characters or emojis: the reply is read out by a speech \
                synthesizer",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}
