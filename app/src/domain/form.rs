use std::fmt::Display;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Iso8601;
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
    // A date kept as the caller said it, e.g. "next saturday", for whoever receives the form to work out
    SpokenDate,
}

impl FormFieldKind {
    /// Reads a value the way the context extractor is told to write it: digits for
    /// numbers, true or false for bool, YYYY-MM-DD for dates.
    pub fn parse(self, text: &str) -> Option<FormFieldValue> {
        let text = text.trim();

        match self {
            FormFieldKind::String => (!text.is_empty()).then(|| FormFieldValue::String(text.to_string())),
            FormFieldKind::UnsignedInteger => text.parse().ok().map(FormFieldValue::UnsignedInteger),
            FormFieldKind::Integer => text.parse().ok().map(FormFieldValue::Integer),
            FormFieldKind::Float => text.replace(',', ".").parse().ok().map(FormFieldValue::Float),
            FormFieldKind::Bool => match text.to_lowercase().as_str() {
                "true" | "yes" => Some(FormFieldValue::Bool(true)),
                "false" | "no" => Some(FormFieldValue::Bool(false)),
                _ => None,
            },
            FormFieldKind::Date => Date::parse(text, &Iso8601::DATE).ok().map(FormFieldValue::Date),
            FormFieldKind::SpokenDate => (!text.is_empty()).then(|| FormFieldValue::SpokenDate(text.to_string())),
        }
    }

    /// Whether an answer meant for one kind would pass for the other: any two
    /// dates, any two numbers, or the same kind.
    fn is_like(self, other: FormFieldKind) -> bool {
        use FormFieldKind::*;

        match (self, other) {
            (Date | SpokenDate, Date | SpokenDate) => true,
            (UnsignedInteger | Integer | Float, UnsignedInteger | Integer | Float) => true,
            _ => self == other,
        }
    }
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
    SpokenDate(String),
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
            FormFieldValue::SpokenDate(_) => FormFieldKind::SpokenDate,
        }
    }
}

impl Display for FormFieldValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormFieldValue::String(s) => f.write_str(s),
            FormFieldValue::UnsignedInteger(n) => write!(f, "{n}"),
            FormFieldValue::Integer(n) => write!(f, "{n}"),
            FormFieldValue::Float(n) => write!(f, "{n}"),
            FormFieldValue::Bool(b) => write!(f, "{b}"),
            FormFieldValue::Date(d) => write!(f, "{d}"),
            FormFieldValue::SpokenDate(s) => f.write_str(s),
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

    /// Adds the next step. Two steps next to each other must not take the same
    /// kind of value: an answer meant for one also fits the other, so the caller
    /// or the context extractor can put it in the wrong field, e.g. a corrected date
    /// of birth taken for the appointment date that is asked for right after it.
    pub fn field(mut self, name: &str, description: &str, kind: FormFieldKind) -> Self {
        if let Some(previous) = self.fields.last() {
            assert!(
                !previous.kind.is_like(kind),
                "{:?} form: `{}` and `{name}` are next to each other and take the same kind of value",
                self.kind,
                previous.name,
            );
        }

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

    pub fn find_field(&self, name: &str) -> Option<&FormField> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub fn is_filled(&self) -> bool {
        self.current_field().is_none()
    }

    /// Whether the field named `name` comes after the current one.
    pub fn is_ahead(&self, name: &str) -> bool {
        let current = self.fields.iter().position(|field| field.state != StepState::Completed);
        let target = self.fields.iter().position(|field| field.name == name);

        matches!((current, target), (Some(current), Some(target)) if target > current)
    }

    /// Sets a field's value, which the caller then has to confirm. A completed
    /// field is reopened, and since every field before it is completed too, it
    /// becomes the current one. `false` when there is no such field or the
    /// value is of the wrong kind.
    pub fn fill_field(&mut self, name: &str, value: FormFieldValue) -> bool {
        match self.fields.iter_mut().find(|field| field.name == name) {
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

    /// `name: value` for every field, e.g. for telling the caller what was filled.
    pub fn values_summary(&self) -> String {
        self.fields
            .iter()
            .map(|field| match &field.value {
                Some(value) => format!("{}: {value}", field.name),
                None => format!("{}: none", field.name),
            })
            .collect::<Vec<_>>()
            .join(", ")
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
                .field("reason", "Reason for the visit", FormFieldKind::String)
                .field("appointment_date", "Preferred date of the appointment", FormFieldKind::SpokenDate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Form::field` asserts that no two neighbouring steps take the same kind of value.
    #[test]
    fn every_form_builds() {
        for form in FormSupported::ALL {
            form.build();
        }
    }
}
