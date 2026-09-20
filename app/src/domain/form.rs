use std::iter::Map;
use std::slice::Iter;
use time::Date;

#[derive(Debug, PartialEq, Clone)]
pub enum FormFieldValue {
    String(String),
    UnsignedInteger(u64),
    Integer(i64),
    Float(f64),
    Bool(bool),
    Date(Date),
}

pub struct FormField {
    pub label: String,
    pub llm_description: String,
    pub value: Option<FormFieldValue>,
}

pub struct Form {
    pub fields: Vec<FormField>,
    pub is_filled: bool,
}

impl Form {
    pub fn get_steps(&self) -> Vec<Step> {
        self.fields
            .iter()
            .map(|field| Step::new(field))
            .collect::<Vec<Step>>()
    }
}

pub trait PersistentSlot {
    fn unique_slot_cache_key(call_id: &str) -> String;

    fn ttl_seconds() -> u64 {
        600
    }
}

pub enum StepState {
    Queued,
    InProgress,
    Completed,
}

pub struct Step<'a> {
    state: StepState,
    field: &'a FormField,
    is_confirmed_by_caller: bool,
    confirmation_failed_counter: u8,
}

impl<'a> Step<'a> {
    pub fn new(field: &'a FormField) -> Self {
        Self {
            state: StepState::Queued,
            field,
            is_confirmed_by_caller: false,
            confirmation_failed_counter: 0,
        }
    }
}
