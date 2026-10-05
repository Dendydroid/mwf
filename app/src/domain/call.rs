use std::fmt::Display;
use serde::{Deserialize, Serialize};
use crate::domain::machine::Described;

/// Declares a fieldless enum together with an `ALL` slice of its variants.
/// Because both come from the same variant list, `ALL` can never go stale.
macro_rules! flat_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        $vis enum $name { $($variant),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerIntent {
    Unsupported,
    GetInformation {selected: GetInformationSupported},
    StartForm {form: FormSupported},
    ProvideFormFieldValue,
    CancelForm,
    ReferToContextForFormFieldValue,
    ConfirmYes,
    ConfirmNo,
    CorrectFormFieldValue,
    Repeat,
    EndCall,
    TransferToHuman,
}

/// What the phone side has to do once the answer is spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallAction {
    #[default]
    Continue,
    EndCall,
    TransferToHuman,
}

flat_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum GetInformationSupported {
        GetCurrentWeatherInBerlin,
        GetCurrentUAHPerEUR,
        // Calendar questions, such as the date of next Saturday: answered with a refusal
        CalendarHelp,
        // Questions about the form in progress, answered from its state
        FormInformation,
    }
}

flat_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum FormSupported {
        DoctorAppointment,
    }
}

impl CallerIntent {
    /// Every concrete intent, with payload variants expanded over all their inner values.
    pub fn all() -> Vec<CallerIntent> {
        use CallerIntent::*;

        // Compile-time guard: adding a CallerIntent variant breaks this match,
        // pointing you here to also add it to the list below.
        match Unsupported {
            Unsupported
            | GetInformation { .. }
            | StartForm { .. }
            | ProvideFormFieldValue
            | CancelForm
            | ReferToContextForFormFieldValue
            | ConfirmYes
            | ConfirmNo
            | CorrectFormFieldValue
            | Repeat
            | EndCall
            | TransferToHuman => {}
        }

        let mut all = vec![
            Unsupported,
            ProvideFormFieldValue,
            CancelForm,
            ReferToContextForFormFieldValue,
            ConfirmYes,
            ConfirmNo,
            CorrectFormFieldValue,
            Repeat,
            EndCall,
            TransferToHuman,
        ];
        all.extend(Self::supported_requests());
        all
    }

    /// The intent a label names, as rendered by `Display`.
    pub fn from_label(label: &str) -> Option<CallerIntent> {
        Self::all().into_iter().find(|intent| intent.to_string() == label)
    }

    /// What the caller can ask for: every information request and every form.
    pub fn supported_requests() -> Vec<CallerIntent> {
        GetInformationSupported::ALL
            .iter()
            .map(|&selected| CallerIntent::GetInformation { selected })
            .chain(FormSupported::ALL.iter().map(|&form| CallerIntent::StartForm { form }))
            .collect()
    }
}

impl Display for CallerIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Written arm by arm rather than as one match, because the variants that
        // carry a payload spell it into the label and the bare ones do not.
        match self {
            CallerIntent::Unsupported => f.write_str("unsupported"),
            CallerIntent::GetInformation {selected} => write!(f, "get_information[{selected}]"),
            CallerIntent::StartForm {form} => write!(f, "start_form[{form}]"),
            CallerIntent::ProvideFormFieldValue => f.write_str("provide_form_field_value"),
            CallerIntent::CancelForm => f.write_str("cancel_form"),
            CallerIntent::ReferToContextForFormFieldValue => f.write_str("refer_to_context_for_form_field_value"),
            CallerIntent::ConfirmYes => f.write_str("confirm_yes"),
            CallerIntent::ConfirmNo => f.write_str("confirm_no"),
            CallerIntent::CorrectFormFieldValue => f.write_str("correct_form_field_value"),
            CallerIntent::Repeat => f.write_str("repeat"),
            CallerIntent::EndCall => f.write_str("end_call"),
            CallerIntent::TransferToHuman => f.write_str("transfer_to_human"),
        }
    }
}

impl Described for CallerIntent {
    fn description(&self) -> &'static str {
        use GetInformationSupported::*;

        match self {
            CallerIntent::Unsupported => "None of the other values fits. Also a greeting, small talk, or a \
                request that is similar to a supported one but not the same, such as another city, another \
                currency or another kind of booking",
            CallerIntent::GetInformation { selected: GetCurrentWeatherInBerlin } => "The caller asks about \
                the current weather in Berlin",
            CallerIntent::GetInformation { selected: GetCurrentUAHPerEUR } => "The caller asks for the \
                current exchange rate of the Ukrainian hryvnia to the euro",
            CallerIntent::GetInformation { selected: CalendarHelp } => "A question about the calendar, such \
                as which date next Saturday is",
            CallerIntent::GetInformation { selected: FormInformation } => "A question about the form or the \
                values the caller gave, such as \"what name did you record\" or \"did you book it\"",
            CallerIntent::StartForm { form: FormSupported::DoctorAppointment } => "The caller either explicitly states that he wants to book \
                a doctor's appointment or describes the symptoms which also most probably means he wants a doctor's appointment",
            CallerIntent::ProvideFormFieldValue => "An answer to the `current_field` of <form_state> when it \
                is `queued`, including a plain yes or no when its kind is `bool`",
            CallerIntent::CancelForm => "The caller wants to stop filling in the form in progress",
            CallerIntent::ReferToContextForFormFieldValue => "A value the caller points to instead of saying \
                it, such as \"the same as before\"",
            CallerIntent::ConfirmYes => "Agreement when the `current_field` of <form_state> is \
                `awaiting_confirmation`",
            CallerIntent::ConfirmNo => "A denial that gives no other value, such as \"no\" or \"that is wrong\", \
                when the `current_field` of <form_state> is `awaiting_confirmation`",
            CallerIntent::CorrectFormFieldValue => "A new value for a field that is already `completed`, or \
                a denial that also gives the right value when the `current_field` of <form_state> is \
                `awaiting_confirmation`",
            CallerIntent::Repeat => "The caller did not hear or understand the last answer",
            CallerIntent::EndCall => "The caller says goodbye or answers that they need nothing else",
            CallerIntent::TransferToHuman => "The caller asks for a person, an operator or an agent",
        }
    }
}

impl Display for GetInformationSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            GetInformationSupported::GetCurrentWeatherInBerlin => "get_current_weather_in_berlin",
            GetInformationSupported::GetCurrentUAHPerEUR => "get_current_uah_per_eur",
            GetInformationSupported::CalendarHelp => "calendar_help",
            GetInformationSupported::FormInformation => "form_information",
        })
    }
}

impl Display for FormSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            FormSupported::DoctorAppointment => "doctor_appointment",
        })
    }
}
