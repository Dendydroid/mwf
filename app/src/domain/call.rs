use crate::app::AppState;
use std::collections::HashMap;
use std::fmt::{format, Display};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use schemars::JsonSchema;
use crate::vllm::SystemPrompt;

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
    SmallTalk,
    // FOR CLAUDE: Result of info goes into backend_context
    GetInformation {selected: GetInformationSupported},
    // FOR CLAUDE: When we start a form it is crucial that we in handler set the first step and its already used in backend_context to already ask first step
    StartForm {form: FormSupported},
    // FOR CLAUDE: This has to update call state, step state, advance step or possibly finish form and the info about the form state has to go to backend_context too
    ProvideFormFieldValue,
    // FOR CLAUDE: The info that form isnt needed anymore has to go to backend context stating that form was cancelled and immediately we follow up with question what else can i help with
    CancelForm,
    // FOR CLAUDE: For this handler has to add to backend_context that we need to search conversation history for answer
    ReferToContextForFormFieldValue,
    ConfirmYes,
    ConfirmNo,
    CorrectFormFieldValue,
}

flat_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum GetInformationSupported {
        GetCurrentWeatherInBerlin,
        GetCurrentUAHPerEUR,
    }
}

// FOR CLAUDE: lets have builder use this enum to build particular form shapes chaining properties. and have one global mod.rs function to get all supported forms which will go into app state as property supported_forms

flat_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        match SmallTalk {
            SmallTalk
            | GetInformation { .. }
            | StartForm { .. }
            | ProvideFormFieldValue
            | CancelForm
            | ReferToContextForFormFieldValue
            | ConfirmYes
            | ConfirmNo
            | CorrectFormFieldValue => {}
        }

        let mut all = vec![
            SmallTalk,
            ProvideFormFieldValue,
            CancelForm,
            ReferToContextForFormFieldValue,
            ConfirmYes,
            ConfirmNo,
            CorrectFormFieldValue,
        ];
        all.extend(GetInformationSupported::ALL.iter().map(|&selected| GetInformation { selected }));
        all.extend(FormSupported::ALL.iter().map(|&form| StartForm { form }));
        all
    }

    /// Labels of every concrete intent, as rendered by `Display`.
    pub fn all_labels() -> Vec<String> {
        Self::all().iter().map(ToString::to_string).collect()
    }
}

impl Display for CallerIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Written arm by arm rather than as one match, because the variants that
        // carry a payload spell it into the label and the bare ones do not.
        match self {
            CallerIntent::SmallTalk => f.write_str("small_talk"),
            CallerIntent::GetInformation {selected} => write!(f, "get_information[{selected}]"),
            CallerIntent::StartForm {form} => write!(f, "start_form[{form}]"),
            CallerIntent::ProvideFormFieldValue => f.write_str("provide_form_field_value"),
            CallerIntent::CancelForm => f.write_str("cancel_form"),
            CallerIntent::ReferToContextForFormFieldValue => f.write_str("refer_to_context_for_form_field_value"),
            CallerIntent::ConfirmYes => f.write_str("confirm_yes"),
            CallerIntent::ConfirmNo => f.write_str("confirm_no"),
            CallerIntent::CorrectFormFieldValue => f.write_str("correct_form_field_value"),
        }
    }
}

impl Display for GetInformationSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            GetInformationSupported::GetCurrentWeatherInBerlin => "get_current_weather_in_berlin",
            GetInformationSupported::GetCurrentUAHPerEUR => "get_current_uah_per_eur",
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
