
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
use serde_json::{json, Map, Value};
use tracing::info;
use crate::domain::call_session::{CallSession, CallTurn};
use crate::vllm::{Answer, VllmClient, VllmError};

pub struct Machine {
    pub(crate) role: String,

    pub(crate) rules: Vec<String>,
}

impl Machine {
    /// `<context>` is the session's own context (initial context, form state).
    pub async fn query<T>(
        &self,
        llm: &VllmClient,
        input: &str,
        call_session: &CallSession,
    ) -> Result<T, VllmError>
    where
        T: OutputFormat + Default + JsonSchema + DeserializeOwned,
    {
        // Sorted, so the same context always renders the same prompt.
        let mut context: Vec<_> = call_session.context().into_iter().collect();
        context.sort();

        let system = self
            .render_system_prompt::<T>(&context, call_session.get_conversation())
            .expect("Could not render the system prompt");
        let user = render_xml(|w| {
            w.create_element("utterance").write_text_content(escaped(input))?;
            Ok(())
        })
        .expect("Could not render the utterance");

        let call_id = call_session.call_id.as_str();
        let output = std::any::type_name::<T>().rsplit("::").next().unwrap_or_default();

        info!(call_id, output, "Machine prompt\n{system}\n{user}");

        let answer = llm.query(&system, &user, output, &json_schema::<T>()).await?;

        info!(call_id, output, content = answer.content.as_str(), "Machine answered");

        let mut parsed: T = serde_json::from_str(&answer.content)
            .map_err(|source| VllmError::Malformed { source, content: answer.content.clone() })?;
        parsed.read_answer(&answer);

        Ok(parsed)
    }

    /// `role` and `rules` are written verbatim, so they can refer to other
    /// sections by tag (`<allowed_values>`). Everything else is escaped, since
    /// it comes from the caller or a backend and must not be able to close a tag.
    fn render_system_prompt<T>(
        &self,
        context: &[(String, String)],
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
                                let Some(description) = value.description() else {
                                    w.create_element("allowed_value")
                                        .write_text_content(escaped(&value.to_string()))?;

                                    continue;
                                };

                                // Written verbatim, like a rule: it can refer to other sections by tag.
                                w.create_element("allowed_value").write_inner_content(|w| {
                                    w.create_element("value").write_text_content(escaped(&value.to_string()))?;
                                    w.create_element("description").write_text_content(verbatim(description))?;

                                    Ok(())
                                })?;
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
                w.create_element("context").write_inner_content(|w| {
                    for (key, value) in context {
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
    format!("{:#}", Value::Object(field_types::<T>()))
}

/// The schema vLLM holds the answer to: the output's fields and no others, no empty
/// strings, and only the allowed values of a field that has them.
fn json_schema<T: OutputFormat + Default + JsonSchema>() -> Value {
    let output = T::default();
    let schemas: HashMap<_, _> = output.iter_schemas().collect();

    let properties: Map<String, Value> = field_types::<T>()
        .into_iter()
        .map(|(name, kind)| {
            let mut property = json!({ "type": kind });
            if kind == "string" {
                property["minLength"] = json!(1);
            }
            if let Some(schema) = schemas.get(name.as_str()).filter(|schema| schema.uses_strict_allowed_values()) {
                property["enum"] = schema.allowed_values().iter().map(AllowedValue::to_json).collect();
            }
            (name, property)
        })
        .collect();

    json!({
        "type": "object",
        "properties": properties,
        "required": root_schema::<T>().get("required").cloned().unwrap_or_else(|| json!([])),
        "additionalProperties": false,
    })
}

/// Each field's JSON type, in struct order.
fn field_types<T: JsonSchema>() -> Map<String, Value> {
    root_schema::<T>()
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
        .collect()
}

fn root_schema<T: JsonSchema>() -> Value {
    SchemaSettings::default()
        .with(|s| s.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

pub enum AllowedValue {
    PositiveNumber(u64),
    Number(i64),
    String(String),
    Bool(bool),
    // A string value with how to match it, see `AllowedValue::described`.
    Described(String, &'static str),
}

/// A value that tells the model how to match it, such as an intent.
pub trait Described {
    fn description(&self) -> &'static str;
}

impl Display for AllowedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllowedValue::PositiveNumber(n) => write!(f, "{n}"),
            AllowedValue::Number(n) => write!(f, "{n}"),
            AllowedValue::String(s) => f.write_str(s),
            AllowedValue::Bool(b) => write!(f, "{b}"),
            AllowedValue::Described(s, _) => f.write_str(s),
        }
    }
}

impl AllowedValue {
    pub fn described<T: Display + Described>(value: &T) -> Self {
        AllowedValue::Described(value.to_string(), value.description())
    }

    fn description(&self) -> Option<&'static str> {
        match self {
            AllowedValue::Described(_, description) => Some(description),
            _ => None,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            AllowedValue::Described(s, _) => json!(s),
            AllowedValue::PositiveNumber(n) => json!(n),
            AllowedValue::Number(n) => json!(n),
            AllowedValue::String(s) => json!(s),
            AllowedValue::Bool(b) => json!(b),
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

    /// Called once the answer is parsed, for what else it tells, such as how sure the model was.
    fn read_answer(&mut self, _answer: &Answer) {}
}
