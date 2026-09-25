
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
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use crate::domain::call::CallerIntent;
use crate::domain::call_session::CallMemory;
use crate::vllm::{VllmClient, VllmError};

struct Machine {
    role: String,

    rules: Vec<String>,
}

impl Machine {
    pub async fn query<T>(
        &self,
        llm: &VllmClient,
        input: &str,
        context: &HashMap<String, String>,
        call_memory: Option<&CallMemory>,
    ) -> Result<T, VllmError>
    where
        T: OutputFormat + Default + JsonSchema + DeserializeOwned,
    {
        let system = self
            .render_system_prompt::<T>(context, call_memory)
            .expect("Could not render the system prompt");
        let user = render_xml(|w| {
            w.create_element("utterance").write_text_content(escaped(input))?;
            Ok(())
        })
        .expect("Could not render the utterance");

        let content = llm.query(&system, &user).await?;

        serde_json::from_str(&content).map_err(|source| VllmError::Malformed { source, content })
    }

    /// `role` and `rules` are written verbatim, so they can refer to other
    /// sections by tag (`<allowed_values>`). Everything else is escaped, since
    /// it comes from the caller or a backend and must not be able to close a tag.
    fn render_system_prompt<T>(
        &self,
        context: &HashMap<String, String>,
        call_memory: Option<&CallMemory>,
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

            if let Some(memory) = call_memory {
                w.create_element("conversation_history").write_inner_content(|w| {
                    for turn in &memory.conversation {
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
            let kind = property.get("type").cloned().unwrap_or_else(|| property.clone());
            (name.clone(), kind)
        })
        .collect();

    format!("{:#}", Value::Object(format))
}

enum AllowedValue {
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

trait ValueSchema {
    fn allowed_values(&self) -> Vec<AllowedValue> {
        vec![]
    }

    fn uses_strict_allowed_values(&self) -> bool {
        !self.allowed_values().is_empty()
    }

    fn valid_value_description(&self) -> &'static str;
}

trait OutputFormat {
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
        "Caller's intended action"
    }
}

#[derive(JsonSchema, Deserialize, Default)]
struct ExtractedIntent {
    machine_reasoning: Reasoning,
    detected_language: DetectedLanguage,
    caller_intent: IntendedAction,
}

impl OutputFormat for ExtractedIntent {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(vec![
            ("machine_reasoning", &self.machine_reasoning as &dyn ValueSchema),
            ("detected_language", &self.detected_language as &dyn ValueSchema),
            ("caller_intent", &self.caller_intent as &dyn ValueSchema),
        ].into_iter())
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
struct FormulatedResponse {
    spoken_response: SpokenResponse,
}

impl OutputFormat for FormulatedResponse {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(vec![
            ("spoken_response", &self.spoken_response as &dyn ValueSchema),
        ].into_iter())
    }
}
