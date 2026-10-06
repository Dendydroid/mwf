use std::fmt::Display;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::types::chrono::Local;
use time::format_description::well_known::Iso8601;
use time::Date;
use crate::app::AppState;
use crate::domain::call::FormSupported;
use crate::domain::flow::main_menu_flow::HintMap;

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
    // A time of day kept the same way, e.g. "in the morning"
    SpokenTime,
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
            FormFieldKind::SpokenTime => (!text.is_empty()).then(|| FormFieldValue::SpokenTime(text.to_string())),
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
    SpokenTime(String),
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
            FormFieldValue::SpokenTime(_) => FormFieldKind::SpokenTime,
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
            FormFieldValue::SpokenTime(s) => f.write_str(s),
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

    /// The fields the caller can be giving a value for. While the current field has no value they were
    /// asked for it, so the fields after it are left out: an answer can fit one of those as well, such
    /// as a day said with a time. Once its value is read back, any field can follow.
    pub fn answerable_fields(&self) -> Vec<&str> {
        let asked_for = self
            .fields
            .iter()
            .position(|field| field.state != StepState::Completed)
            .filter(|&current| self.fields[current].state == StepState::Queued);
        let answerable = asked_for.map_or(self.fields.len(), |current| current + 1);

        self.fields[..answerable].iter().map(|field| field.name.as_str()).collect()
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
                // Not "Preferred date": German replies then asked for "den bevorzugten Termin für den Termin"
                .field("appointment_date", "Date of the appointment", FormFieldKind::SpokenDate)
                .field("appointment_time", "Preferred time of the appointment", FormFieldKind::SpokenTime),
        }
    }

    /// Asks the validators of a field about a value the field is about to take. They are found by
    /// the field's name and not kept in the field, which is saved with the call between its turns.
    pub async fn validate(self, app_state: &AppState, field: &str, value: &FormFieldValue) -> Result<(), ValidationError> {
        match (self, field) {
            (FormSupported::DoctorAppointment, "date_of_birth") => {
                date_of_birth_not_in_the_future(app_state, value).await
            }
            _ => Ok(()),
        }
    }

    /// Called once the caller has confirmed every field of the form, with the hints of the call: the
    /// form's callback says what of its values goes back into them. Found by the form's kind and not
    /// kept in the form, like the validators. The session is saved after it, never before.
    pub fn on_completed(self, hint_map: &mut HintMap, form: &Form) {
        match self {
            FormSupported::DoctorAppointment => doctor_appointment_completed(hint_map, form),
        }
    }
}

/// Why a validator refused a value, written for the response formulator: what to tell the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError(pub String);

/*
    Validators: each gets the app state and the value its field is about to take
*/

async fn date_of_birth_not_in_the_future(_: &AppState, value: &FormFieldValue) -> Result<(), ValidationError> {
    // Today on the clock `current_time` is taken from, read the way the form reads a date
    let today = Date::parse(&Local::now().date_naive().to_string(), &Iso8601::DATE).ok();

    match value {
        FormFieldValue::Date(date) if today.is_some_and(|today| *date > today) => {
            Err(ValidationError("A date of birth cannot be in the future. Tell the caller that.".to_string()))
        }
        _ => Ok(()),
    }
}

/*
    Completion callbacks: each gets the hints of the call and its completed form
*/

fn doctor_appointment_completed(hint_map: &mut HintMap, form: &Form) {
    let value = |name: &str| form.find_field(name).and_then(|field| field.value.as_ref()).map(ToString::to_string);

    hint_map.patient_full_name = value("patient_name");
    hint_map.date_of_birth_iso_8601 = value("date_of_birth");
    hint_map.appointment_spoken_date = value("appointment_date");
    hint_map.appointment_spoken_time = value("appointment_time");
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

    #[test]
    fn later_fields_cannot_be_answered_while_the_current_one_is_asked_for() {
        let mut form = FormSupported::DoctorAppointment.build();
        assert_eq!(form.answerable_fields(), ["patient_name"]);

        // Read back to the caller, who can go on to any field from there
        form.fill_field("patient_name", FormFieldValue::String("John Smith".into()));
        assert_eq!(form.answerable_fields().len(), form.fields.len());

        form.confirm_current();
        assert_eq!(form.answerable_fields(), ["patient_name", "date_of_birth"]);
    }

    #[test]
    fn completed_doctor_appointment_goes_back_into_the_hints() {
        let mut form = FormSupported::DoctorAppointment.build();
        for (name, value) in [
            ("patient_name", "John Smith"),
            ("date_of_birth", "1991-06-13"),
            ("reason", "headache"),
            ("appointment_date", "next sunday"),
            ("appointment_time", "at ten"),
        ] {
            let kind = form.find_field(name).unwrap().kind;
            form.fill_field(name, kind.parse(value).unwrap());
            form.confirm_current();
        }

        // What the caller said before the form, and corrected in it
        let mut hint_map = HintMap {
            caller_full_name: Some("Anna Smith".into()),
            patient_full_name: Some("Jon Smith".into()),
            appointment_spoken_date: Some("next saturday".into()),
            appointment_spoken_time: Some("in the morning".into()),
            ..Default::default()
        };
        form.kind.on_completed(&mut hint_map, &form);

        assert_eq!(
            hint_map,
            HintMap {
                caller_full_name: Some("Anna Smith".into()),
                patient_full_name: Some("John Smith".into()),
                date_of_birth_iso_8601: Some("1991-06-13".into()),
                appointment_spoken_date: Some("next sunday".into()),
                appointment_spoken_time: Some("at ten".into()),
                ..Default::default()
            }
        );
    }
}
