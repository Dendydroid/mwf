use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Date;
use crate::domain::call::FormSupported;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldKind {
    String,
    UnsignedInteger,
    Integer,
    Float,
    Bool,
    Date,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldValue {
    String(String),
    UnsignedInteger(u64),
    Integer(i64),
    Float(f64),
    Bool(bool),
    // YYYY-MM-DD
    Date(Date),
}

impl FormFieldValue {
    pub fn kind(&self) -> FormFieldKind {
        match self {
            FormFieldValue::String(_) => FormFieldKind::String,
            FormFieldValue::UnsignedInteger(_) => FormFieldKind::UnsignedInteger,
            FormFieldValue::Integer(_) => FormFieldKind::Integer,
            FormFieldValue::Float(_) => FormFieldKind::Float,
            FormFieldValue::Bool(_) => FormFieldKind::Bool,
            FormFieldValue::Date(_) => FormFieldKind::Date,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    // No value yet
    Queued,
    // Has a value the caller still has to confirm
    AwaitingConfirmation,
    // Confirmed by the caller
    Completed,
}

/// One field of a form, and the step of asking the caller for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub description: String,
    pub kind: FormFieldKind,
    pub value: Option<FormFieldValue>,
    pub state: StepState,
    pub confirmation_failed_counter: u8,
}

/// A form being filled out, one field at a time and in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub kind: FormSupported,
    pub fields: Vec<FormField>,
}

impl Form {
    pub fn new(kind: FormSupported) -> Self {
        Self {
            kind,
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, name: &str, description: &str, kind: FormFieldKind) -> Self {
        self.fields.push(FormField {
            name: name.to_string(),
            description: description.to_string(),
            kind,
            value: None,
            state: StepState::Queued,
            confirmation_failed_counter: 0,
        });

        self
    }

    /// The step the caller is on: the first field that is not confirmed yet.
    pub fn current_field(&self) -> Option<&FormField> {
        self.fields.iter().find(|field| field.state != StepState::Completed)
    }

    fn current_field_mut(&mut self) -> Option<&mut FormField> {
        self.fields.iter_mut().find(|field| field.state != StepState::Completed)
    }

    pub fn is_filled(&self) -> bool {
        self.current_field().is_none()
    }

    /// Sets the current field's value, which the caller then has to confirm.
    /// `false` when the form is already filled or the value is of the wrong kind.
    pub fn fill_current(&mut self, value: FormFieldValue) -> bool {
        match self.current_field_mut() {
            Some(field) if field.kind == value.kind() => {
                field.value = Some(value);
                field.state = StepState::AwaitingConfirmation;

                true
            }
            _ => false,
        }
    }

    /// The caller confirmed the current field's value, so the next field
    /// becomes current. `false` when there was nothing to confirm.
    pub fn confirm_current(&mut self) -> bool {
        match self.current_field_mut() {
            Some(field) if field.state == StepState::AwaitingConfirmation => {
                field.state = StepState::Completed;

                true
            }
            _ => false,
        }
    }

    /// The caller said the current field's value is wrong, so it is asked for
    /// again. `false` when there was nothing to reject.
    pub fn reject_current(&mut self) -> bool {
        match self.current_field_mut() {
            Some(field) if field.state == StepState::AwaitingConfirmation => {
                field.value = None;
                field.state = StepState::Queued;
                field.confirmation_failed_counter = field.confirmation_failed_counter.saturating_add(1);

                true
            }
            _ => false,
        }
    }

    /// The form as it goes into `<context>`.
    pub fn context_value(&self) -> String {
        json!({
            "form": self.kind,
            "current_field": self.current_field().map(|field| &field.name),
            "fields": self.fields,
        })
        .to_string()
    }
}

impl FormSupported {
    pub fn build(self) -> Form {
        match self {
            FormSupported::DoctorAppointment => Form::new(self)
                .field("patient_name", "Full name of the patient", FormFieldKind::String)
                .field("date_of_birth", "Patient's date of birth", FormFieldKind::Date)
                .field("appointment_date", "Preferred date of the appointment", FormFieldKind::Date)
                .field("reason", "Reason for the visit", FormFieldKind::String),
        }
    }
}
