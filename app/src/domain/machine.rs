
/*
    A single machine has a role, an output rule for each allowed value, other rules and produces strict json response output
*/
use std::collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::domain::call::CallerIntent;
use crate::domain::call_session::CallMemory;

struct Machine {
    role: String,

    rules: Vec<String>,
}

impl Machine {
    pub fn query<T>(
        input: &str,
        context: HashMap<String, String>,
        call_memory: Option<&CallMemory>,
    ) -> T {

    }
}

enum AllowedValue {
    PositiveNumber(u64),
    Number(i64),
    String(String),
    Bool(bool),
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
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=&dyn ValueSchema> + '_>;
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

#[derive(JsonSchema, Deserialize)]
struct ExtractedIntent {
    machine_reasoning: Reasoning,
    detected_language: DetectedLanguage,
    caller_intent: IntendedAction,
}

impl OutputFormat for ExtractedIntent {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=&dyn ValueSchema> + '_> {
        Box::new(vec![
            &self.machine_reasoning as &dyn ValueSchema,
            &self.detected_language as &dyn ValueSchema,
            &self.caller_intent as &dyn ValueSchema,
        ].into_iter())
    }
}
