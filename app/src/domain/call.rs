use crate::app::AppState;
use std::collections::HashMap;
use std::fmt::{format, Display};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

impl GetInformationSupported {
    /// Whether the caller is told this is something the assistant can help with.
    pub fn is_offered(self) -> bool {
        !matches!(self, GetInformationSupported::CalendarHelp | GetInformationSupported::FormInformation)
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

    /// Labels of every concrete intent, as rendered by `Display`.
    pub fn all_labels() -> Vec<String> {
        Self::all().iter().map(ToString::to_string).collect()
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

    /// Backend context for the response formulator on `Unsupported`: a polite
    /// refusal that lists what the caller can ask for instead.
    pub fn unsupported_backend_context() -> String {
        let supported = Self::supported_requests()
            .iter()
            .filter(|intent| !matches!(intent, CallerIntent::GetInformation { selected } if !selected.is_offered()))
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "The caller asked for something that is not supported. Politely say so \
            and list, in plain words, what you can help with instead: {supported}"
        )
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
