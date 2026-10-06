use std::fmt::Display;
use serde::{Deserialize, Serialize};
use crate::domain::machine::Described;
use crate::vocabulary::vocabulary;

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

pub(crate) use flat_enum;

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
        // Questions about the form the caller filled out last in the call, answered with its summary
        LastFilledOutFormInformation,
    }
}

flat_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// How the intent matchers are told to match it: its `description` in the vocabulary, under its label.
    fn description(&self) -> &'static str {
        &vocabulary().intent(&self.to_string()).description
    }
}

impl Display for GetInformationSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            GetInformationSupported::GetCurrentWeatherInBerlin => "get_current_weather_in_berlin",
            GetInformationSupported::GetCurrentUAHPerEUR => "get_current_uah_per_eur",
            GetInformationSupported::CalendarHelp => "calendar_help",
            GetInformationSupported::FormInformation => "form_information",
            GetInformationSupported::LastFilledOutFormInformation => "last_filled_out_form_information",
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
