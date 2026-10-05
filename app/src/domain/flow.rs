
/*
    A call is always in one flow, like in one menu of a game: the main menu, or a form once it is started.
    Each flow has its own machines, so they only know what the caller can say in it.
*/
use schemars::JsonSchema;
use serde::Deserialize;
use crate::domain::flow::form_flow::{ExtractedFormIntent, ExtractedFormValue};
use crate::domain::flow::main_menu_flow::{ExtractedMainMenuIntent, HintMap};
use crate::domain::machine::{OutputFormat, ValueSchema};

/// What the context extractor of the flow took from the utterance.
pub enum FlowContext {
    MainMenu(HintMap),
    Form(ExtractedFormValue),
}

/// What the intent matcher of the flow chose. In a form it comes with the extracted value,
/// which only counts when the intent says the caller gave one.
pub enum IntentMatched {
    MainMenu(ExtractedMainMenuIntent),
    Form(ExtractedFormIntent, ExtractedFormValue),
}

#[derive(JsonSchema, Deserialize, Default, Debug)]
struct Reasoning(String);

impl ValueSchema for Reasoning {
    fn valid_value_description(&self) -> &'static str {
        "A brief 1-sentence analysis of the conversation state based on rules and context."
    }
}

#[derive(JsonSchema, Deserialize, Default, Debug)]
struct SpokenResponse(String);

impl ValueSchema for SpokenResponse {
    fn valid_value_description(&self) -> &'static str {
        "A natural, concise spoken reply of 1 to 2 sentences in plain text for text-to-speech, \
        with numbers, currencies and dates written the way they are spoken"
    }
}

#[derive(JsonSchema, Deserialize, Default, Debug)]
pub struct FormulatedResponse {
    spoken_response: SpokenResponse,
}

impl FormulatedResponse {
    pub fn into_spoken_response(self) -> String {
        self.spoken_response.0
    }
}

impl OutputFormat for FormulatedResponse {
    fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
        Box::new(vec![
            ("spoken_response", &self.spoken_response as &dyn ValueSchema),
        ].into_iter())
    }
}

pub mod main_menu_flow {
    use reqwest::Client;
    use schemars::JsonSchema;
    use lingua::Language;
    use serde::{Deserialize, Serialize};
    use tracing::error;
    use crate::domain::call::{CallAction, CallerIntent, FormSupported, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::Reasoning;
    use crate::domain::flow::form_flow::next_step;
    use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;

    #[derive(JsonSchema, Deserialize, Serialize, Debug, Default, Clone, PartialEq)]
    pub struct HintMap {
        caller_full_name: Option<String>,
        patient_full_name: Option<String>,
        date_of_birth_iso_8601: Option<String>,
        appointment_spoken_date: Option<String>,
        appointment_spoken_time: Option<String>,
    }

    impl HintMap {
        /// Takes the values `other` has, and keeps its own where `other` has none.
        pub fn merge(&mut self, other: HintMap) {
            self.caller_full_name = other.caller_full_name.or(self.caller_full_name.take());
            self.patient_full_name = other.patient_full_name.or(self.patient_full_name.take());
            self.date_of_birth_iso_8601 = other.date_of_birth_iso_8601.or(self.date_of_birth_iso_8601.take());
            self.appointment_spoken_date = other.appointment_spoken_date.or(self.appointment_spoken_date.take());
            self.appointment_spoken_time = other.appointment_spoken_time.or(self.appointment_spoken_time.take());
        }
    }

    impl ValueSchema for Option<String> {
        fn valid_value_description(&self) -> &'static str {
            "either non-empty string or null type."
        }
    }

    impl OutputFormat for HintMap {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(
                [
                    ("caller_full_name", &self.caller_full_name as &dyn ValueSchema),
                    ("patient_full_name", &self.patient_full_name as &dyn ValueSchema),
                    ("date_of_birth_iso_8601", &self.date_of_birth_iso_8601 as &dyn ValueSchema),
                    ("appointment_spoken_date", &self.appointment_spoken_date as &dyn ValueSchema),
                    ("appointment_spoken_time", &self.appointment_spoken_time as &dyn ValueSchema),
                ]
                    .into_iter(),
            )
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct IntendedMainMenuAction(String);

    impl ValueSchema for IntendedMainMenuAction {
        fn allowed_values(&self) -> Vec<AllowedValue> {
            use CallerIntent::*;

            vec![
                Unsupported,
                GetInformation {selected: GetInformationSupported::CalendarHelp},
                GetInformation {selected: GetInformationSupported::GetCurrentUAHPerEUR},
                GetInformation {selected: GetInformationSupported::GetCurrentWeatherInBerlin},
                StartForm {form: FormSupported::DoctorAppointment},
                Repeat,
                EndCall,
                TransferToHuman,
            ]
                .iter()
                .map(AllowedValue::described)
                .collect()
        }

        fn valid_value_description(&self) -> &'static str {
            "Caller's intended action, or `unsupported` when none of the other values fits"
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct ExtractedMainMenuIntent {
        machine_reasoning: Reasoning,
        caller_intent: IntendedMainMenuAction,
        // How sure the matcher was of `caller_intent`, 0 to 1. Not part of the answer.
        #[serde(skip)]
        confidence: Option<f64>,
    }

    impl ExtractedMainMenuIntent {
        pub fn machine_reasoning(&self) -> &str {
            &self.machine_reasoning.0
        }

        pub fn caller_intent(&self) -> &str {
            &self.caller_intent.0
        }

        pub fn confidence(&self) -> Option<f64> {
            self.confidence
        }
    }

    impl OutputFormat for ExtractedMainMenuIntent {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("machine_reasoning", &self.machine_reasoning as &dyn ValueSchema),
                ("caller_intent", &self.caller_intent as &dyn ValueSchema),
            ].into_iter())
        }

        fn read_answer(&mut self, answer: &Answer) {
            self.confidence = answer.probability_of("caller_intent");
        }
    }

    impl Machine {
        pub fn main_menu_context_extractor() -> Self {
            Self {
                role: "You are a context extractor machine. You receive a <utterance> on the input and provide the values in a predefined <json_output_format> as output.".into(),
                rules: vec![
                    "Each extracted value must only be set once in one property of <json_output_format>.".into(),
                    "If value cannot be derived from <utterance>, remove the key from <json_output_format>.".into(),
                ],
            }
        }

        pub fn main_menu_intent_matcher() -> Self {
            Self {
                role: "You are the intent matcher. Classify what the caller wants with their latest <utterance>.".into(),
                rules: [
                    "Classify only the latest <utterance>. Use <conversation_history> to understand short or \
                elliptical answers such as \"yes\", \"the second one\" or a bare name",
                ]
                    .map(String::from)
                    .to_vec(),
            }
        }

        pub fn main_menu_response_formulator() -> Self {
            Self {
                role: "You are the voice of a phone assistant. Reply to the caller's <utterance> from what \
                <context> gives you, as natural speech."
                    .to_string(),
                rules: [
                    "Reply in the language given by <language> in <context>",
                    "Only ask one question in reply",
                    "No markdown, lists, special characters or emojis: the reply is read out by a speech synthesizer",
                    "Follow <response_context> instructions when answering and take its content into consideration"
                ]
                    .map(String::from)
                    .to_vec()
            }
        }
    }
    
    pub async fn main_menu_intent_context_handler(intent: &ExtractedMainMenuIntent, session: &mut CallSession, http_client: &Client) {
        let caller_intent = CallerIntent::from_label(intent.caller_intent())
            .unwrap_or(CallerIntent::Unsupported);

        session.call_turn_outcome = CallTurnOutcome::matched(caller_intent, intent.machine_reasoning(), intent.confidence());

        let has_conversation_history = session.data.call_memory.conversation.len() > 0;

        let backend_context = match caller_intent {
            CallerIntent::GetInformation { selected } => match selected.fetch(&http_client).await {
                Ok(information) => information,
                Err(e) => {
                    error!(call_id = %session.call_id, "Could not fetch {selected}: {e:#}");

                    "Politely apologize, state that the requested service is unavailable at the moment.".into()
                }
            },
            CallerIntent::StartForm { form } => start_form(&mut session.data.state, form),
            CallerIntent::Repeat if has_conversation_history => "The caller asked to hear your last reply from <conversation_history>."
                .to_string(),
            CallerIntent::EndCall => {
                session.call_turn_outcome.action = CallAction::EndCall;

                "The caller is ending the call. Say a short, friendly goodbye and ask nothing.".to_string()

                // TODO: End call logic
            }
            CallerIntent::TransferToHuman => {
                session.call_turn_outcome.action = CallAction::TransferToHuman;

                "Tell the caller you are transferring them to a human agent now, and ask nothing.".to_string()

                // TODO: Transfer to human logic
            }
            // CallerIntent::Unsupported | CallerIntent::Repeat if has_conversation_just_started
            _ => {
                let not_listed = [
                    CallerIntent::Unsupported,
                    CallerIntent::GetInformation { selected: GetInformationSupported::CalendarHelp },
                    CallerIntent::Repeat,
                    CallerIntent::EndCall,
                    CallerIntent::TransferToHuman,
                ]
                    .map(|intent| intent.to_string());

                let supported = IntendedMainMenuAction::default()
                    .allowed_values()
                    .iter()
                    .map(ToString::to_string)
                    .filter(|value| !not_listed.contains(value))
                    .collect::<Vec<_>>()
                    .join(", ");

                format!(
                    "The caller asked for something that is not supported. Politely say so \
                    and list, in plain words, what you can help with instead: {supported}."
                )
            },
        };

        session.call_turn_context.insert(
            "response_context".into(),
            backend_context
        );
    }

    /// What the caller hears when the turn failed on our side. Fixed text, because the machine
    /// that would phrase it is the one that failed. Lists what `IntendedMainMenuAction` offers.
    pub fn failed_turn_response(language: Language) -> &'static str {
        match language {
            Language::German => "Entschuldigung, bei uns ist ein Fehler aufgetreten, bitte versuchen Sie es noch \
                einmal. Ich kann Ihnen das aktuelle Wetter in Berlin oder den Wechselkurs von Hrywnja zu Euro \
                nennen oder einen Arzttermin für Sie buchen.",
            Language::Russian => "Извините, на нашей стороне произошла ошибка, пожалуйста, попробуйте ещё раз. \
                Я могу подсказать текущую погоду в Берлине, курс гривны к евро или записать вас на приём к врачу.",
            Language::Ukrainian => "Вибачте, на нашому боці сталася помилка, будь ласка, спробуйте ще раз. \
                Я можу підказати поточну погоду в Берліні, курс гривні до євро або записати вас на прийом до лікаря.",
            _ => "Sorry, an error happened on our side, please try again. I can tell you the current weather in \
                Berlin, the hryvnia to euro exchange rate, or book a doctor's appointment.",
        }
    }

    fn start_form(state: &mut CallState, form: FormSupported) -> String {
        let built = form.build();
        let context = format!("Started the {form} form. {}", next_step(&built));
        *state = CallState::FormInProgress(built);

        context
    }
}

pub mod form_flow {
    use reqwest::Client;
    use schemars::JsonSchema;
    use lingua::Language;
    use serde::Deserialize;
    use tracing::error;
    use crate::domain::call::{CallAction, CallerIntent, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::Reasoning;
    use crate::domain::form::{Form, FormField, FormFieldKind, StepState};
    use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;

    /// After this many rejected values for one field, the caller is offered a human.
    const MAX_CONFIRMATION_FAILURES: u8 = 3;

    #[derive(JsonSchema, Deserialize, Default, Debug, Clone)]
    struct FormFieldName(Option<String>);

    impl ValueSchema for FormFieldName {
        fn valid_value_description(&self) -> &'static str {
            "Name of the field in <form_state> that `form_field_value` is for. Set it only when the utterance gives \
            a form value, otherwise leave `form_field` out"
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug, Clone)]
    struct ExtractedValue(Option<String>);

    impl ValueSchema for ExtractedValue {
        fn valid_value_description(&self) -> &'static str {
            "The value the caller gave for that field. Set it only when the caller gave one, otherwise leave \
            `form_field_value` out. For a field of kind `date` write YYYY-MM-DD. For a field of kind \
            `spoken_date` write the caller's words in English and lowercase, such as \"next saturday\" or \
            \"tomorrow evening\", without working the date out. Write numbers as digits, yes and no as true and false, \
            and text as the caller said it"
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug, Clone)]
    pub struct ExtractedFormValue {
        #[serde(default)]
        form_field: FormFieldName,
        #[serde(default)]
        form_field_value: ExtractedValue,
    }

    impl ExtractedFormValue {
        /// An empty name counts as left out.
        pub fn form_field(&self) -> Option<&str> {
            self.form_field.0.as_deref().filter(|name| !name.is_empty())
        }

        /// Same as [`Self::form_field`].
        pub fn form_field_value(&self) -> Option<&str> {
            self.form_field_value.0.as_deref().filter(|value| !value.is_empty())
        }
    }

    impl OutputFormat for ExtractedFormValue {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("form_field", &self.form_field as &dyn ValueSchema),
                ("form_field_value", &self.form_field_value as &dyn ValueSchema),
            ].into_iter())
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct IntendedFormAction(String);

    impl ValueSchema for IntendedFormAction {
        fn allowed_values(&self) -> Vec<AllowedValue> {
            use CallerIntent::*;

            vec![
                Unsupported,
                ProvideFormFieldValue,
                CorrectFormFieldValue,
                ReferToContextForFormFieldValue,
                ConfirmYes,
                ConfirmNo,
                CancelForm,
                GetInformation {selected: GetInformationSupported::FormInformation},
                Repeat,
                EndCall,
                TransferToHuman,
            ]
                .iter()
                .map(AllowedValue::described)
                .collect()
        }

        fn valid_value_description(&self) -> &'static str {
            "Caller's intended action, or `unsupported` when none of the other values fits"
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct ExtractedFormIntent {
        machine_reasoning: Reasoning,
        caller_intent: IntendedFormAction,
        // How sure the matcher was of `caller_intent`, 0 to 1. Not part of the answer.
        #[serde(skip)]
        confidence: Option<f64>,
    }

    impl ExtractedFormIntent {
        pub fn machine_reasoning(&self) -> &str {
            &self.machine_reasoning.0
        }

        pub fn caller_intent(&self) -> &str {
            &self.caller_intent.0
        }

        pub fn confidence(&self) -> Option<f64> {
            self.confidence
        }
    }

    impl OutputFormat for ExtractedFormIntent {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("machine_reasoning", &self.machine_reasoning as &dyn ValueSchema),
                ("caller_intent", &self.caller_intent as &dyn ValueSchema),
            ].into_iter())
        }

        fn read_answer(&mut self, answer: &Answer) {
            self.confidence = answer.probability_of("caller_intent");
        }
    }

    impl Machine {
        pub fn form_context_extractor() -> Self {
            Self {
                role: "You are a context extractor machine. The caller is filling in the form in <form_state>. You receive a <utterance> on the input and provide the form value it gives in a predefined <json_output_format> as output.".into(),
                rules: vec![
                    "Extract the value only from what <utterance> itself says. Never copy a value that is already in <form_state> or in <conversation_history>.".into(),
                    "The only exception is a value the caller points to instead of saying it, such as \"the same as before\": write the value they point to.".into(),
                    "The value is for the `current_field` of <form_state>, unless the caller is changing a value they gave for another field.".into(),
                    "If <utterance> gives no form value, such as a plain agreement or denial, a question or a request, remove both keys from <json_output_format>.".into(),
                ],
            }
        }

        pub fn form_intent_matcher() -> Self {
            Self {
                role: "You are the intent matcher. The caller is filling in the form in <form_state>. Classify what the caller wants with their latest <utterance>.".into(),
                rules: [
                    "Classify only the latest <utterance>. Use <conversation_history> to understand short or \
                    elliptical answers such as \"yes\", \"the second one\" or a bare name",
                    "Look at the state of the `current_field` of <form_state> first: when it is `queued` the caller \
                    was asked for its value, when it is `awaiting_confirmation` the caller was asked whether its \
                    value is right",
                    "Use `confirm_yes` only for a clear agreement. A word that neither agrees, denies nor gives a \
                    value is `unsupported`",
                    "A request that is not about this form, such as the weather, an exchange rate, a question about \
                    the calendar or another booking, is `unsupported`",
                ]
                    .map(String::from)
                    .to_vec(),
            }
        }

        pub fn form_response_formulator() -> Self {
            Self {
                role: "You are the voice of a phone assistant that is filling in a form with the caller. Reply to \
                the caller's <utterance> from what <context> gives you, as natural speech."
                    .to_string(),
                rules: [
                    "Reply in the language given by <language> in <context>",
                    "Only ask one question in reply",
                    "No markdown, lists, special characters or emojis: the reply is read out by a speech synthesizer",
                    "Follow <response_context> instructions when answering and take its content into consideration",
                    "When <response_context> ends with a next step, end the reply with that step and ask nothing \
                    else. To have a value confirmed, say what the value is for, read it back and ask plainly \
                    whether it is right",
                    "Say forms, fields and values in plain words, never as their snake_case names or states",
                    "Answer a question about the form only from the fields of <form_state>: a field without a value \
                    has not been given yet, and a field that is not `completed` has not been confirmed yet",
                    "The form is only sent once every field is confirmed, so never say it is booked or done before \
                    <response_context> says it is complete",
                ]
                    .map(String::from)
                    .to_vec()
            }
        }
    }

    /// Returns the form once the caller has confirmed its last field: the call is back in the main menu then.
    pub async fn form_intent_context_handler(
        intent: &ExtractedFormIntent,
        form_value: &ExtractedFormValue,
        session: &mut CallSession,
        http_client: &Client,
    ) -> Option<Form> {
        let caller_intent = CallerIntent::from_label(intent.caller_intent())
            .unwrap_or(CallerIntent::Unsupported);

        session.call_turn_outcome = CallTurnOutcome::matched(caller_intent, intent.machine_reasoning(), intent.confidence());

        let CallState::FormInProgress(form) = &mut session.data.state else {
            return None;
        };

        // The extractor tends to write a value the form already has once more: the caller did not give that one.
        let given_value = form_value
            .form_field_value()
            .filter(|value| !holds(form, form_value.form_field(), value));

        let mut backend_context = match caller_intent {
            CallerIntent::GetInformation { selected } => match selected.fetch(&http_client).await {
                Ok(information) => information,
                Err(e) => {
                    error!(call_id = %session.call_id, "Could not fetch {selected}: {e:#}");

                    "Politely apologize, state that the requested service is unavailable at the moment.".into()
                }
            },
            // A correction that comes without a value is only the denial.
            CallerIntent::CorrectFormFieldValue if given_value.is_none() => reject(form),
            CallerIntent::ProvideFormFieldValue
            | CallerIntent::ReferToContextForFormFieldValue
            | CallerIntent::CorrectFormFieldValue => fill(form, form_value.form_field(), given_value),
            CallerIntent::ConfirmYes => confirm(form),
            CallerIntent::ConfirmNo => reject(form),
            CallerIntent::CancelForm => {
                let cancelled = format!(
                    "The {} form was cancelled, nothing of it was kept. Say so and ask whether the caller needs \
                    anything else.",
                    form.kind
                );
                session.data.state = CallState::Idle;

                cancelled
            }
            CallerIntent::Repeat => format!(
                "The caller asked to hear your last reply again. Say it once more: {}",
                session.data.last_spoken_response.as_deref().unwrap_or_default()
            ),
            CallerIntent::EndCall => {
                session.data.state = CallState::Idle;
                session.call_turn_outcome.action = CallAction::EndCall;

                "The caller is ending the call. Say a short, friendly goodbye and ask nothing.".to_string()
            }
            CallerIntent::TransferToHuman => {
                session.call_turn_outcome.action = CallAction::TransferToHuman;

                "Tell the caller you are transferring them to a human agent now, and ask nothing.".to_string()
            }
            // CallerIntent::Unsupported
            _ => "The caller said something you cannot help with while the form is being filled in, or that \
                does not answer your question. Do not answer it, only say politely in a few words that you \
                cannot help with that."
                .to_string(),
        };

        // A repeated reply already ends with the step, and after a transfer the assistant asks nothing.
        let asks_next_step = !matches!(caller_intent, CallerIntent::Repeat | CallerIntent::TransferToHuman);

        let completed_form = match &session.data.state {
            CallState::FormInProgress(form) if form.is_filled() => Some(form.clone()),
            CallState::FormInProgress(form) if asks_next_step => {
                backend_context = format!("{backend_context} {}", next_step(form));

                None
            }
            _ => None,
        };

        if completed_form.is_some() {
            session.data.state = CallState::Idle;
        }

        session.call_turn_context.insert(
            "response_context".into(),
            backend_context
        );

        completed_form
    }

    /// What the caller hears when the turn failed on our side while a form is being filled in.
    /// Fixed text like in the main menu. The form is as it was, so they only have to say it again.
    pub fn failed_turn_response(language: Language) -> &'static str {
        match language {
            Language::German => "Entschuldigung, bei uns ist ein Fehler aufgetreten, bitte wiederholen Sie das \
                noch einmal.",
            Language::Russian => "Извините, на нашей стороне произошла ошибка, пожалуйста, повторите ещё раз.",
            Language::Ukrainian => "Вибачте, на нашому боці сталася помилка, будь ласка, повторіть ще раз.",
            _ => "Sorry, an error happened on our side, please say that again.",
        }
    }

    pub fn next_step(form: &Form) -> String {
        match form.current_field() {
            Some(FormField { description, value: Some(value), state: StepState::AwaitingConfirmation, .. }) => {
                format!("Next, ask the caller to confirm that {description} is {value}.")
            }
            Some(field) => format!("Next, ask the caller for: {}.", field.description),
            None => String::new(),
        }
    }

    /// Whether the field the extractor named, or the current one when it named none that exists, has this value.
    fn holds(form: &Form, field: Option<&str>, value: &str) -> bool {
        field
            .and_then(|name| form.find_field(name))
            .or_else(|| form.current_field())
            .and_then(|field| field.value.as_ref())
            .is_some_and(|held| held.to_string().to_lowercase() == value.to_lowercase())
    }

    /// Fills the field the extractor named, or the current one when it named none that exists.
    fn fill(form: &mut Form, field: Option<&str>, value: Option<&str>) -> String {
        let Some(target) = field
            .and_then(|name| form.find_field(name))
            .or_else(|| form.current_field())
        else {
            return "The form has no field left to fill.".to_string();
        };
        let (name, kind) = (target.name.clone(), target.kind);

        let Some(text) = value else {
            return format!("The caller gave no value for {name}.");
        };

        // Parsed with the field's own kind, so filling it cannot fail.
        match kind.parse(text) {
            Some(parsed) => {
                // Answering a later field means the caller moved on from the value read back for the
                // current one, which accepts it.
                if form.is_ahead(&name) {
                    form.confirm_current();
                }
                form.fill_field(&name, parsed);

                format!("Recorded \"{text}\" for {name}.")
            }
            None => format!("Could not understand \"{text}\" as a {kind:?} value for {name}."),
        }
    }

    fn confirm(form: &mut Form) -> String {
        let Some(field) = form.current_field() else {
            return "The form has nothing to confirm.".to_string();
        };
        let (name, kind, state) = (field.name.clone(), field.kind, field.state);

        match (state, kind) {
            (StepState::AwaitingConfirmation, _) => {
                form.confirm_current();

                if form.is_filled() {
                    format!(
                        "The caller confirmed {name}. The {} form is complete with {}. Tell the caller it is done \
                        and ask whether they need anything else.",
                        form.kind,
                        form.values_summary()
                    )
                } else {
                    format!("The caller confirmed {name}. Thank them.")
                }
            }
            // A yes to a yes-or-no field is its value, not a confirmation.
            (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("true")),
            _ => format!("The caller said yes, but {name} has no value to confirm yet."),
        }
    }

    fn reject(form: &mut Form) -> String {
        let Some(field) = form.current_field() else {
            return "The form has nothing to reject.".to_string();
        };
        let (name, kind, state) = (field.name.clone(), field.kind, field.state);

        match (state, kind) {
            (StepState::AwaitingConfirmation, _) => {
                form.reject_current();

                let failures = form.current_field().map_or(0, |field| field.confirmation_failed_counter);
                if failures >= MAX_CONFIRMATION_FAILURES {
                    format!(
                        "The caller rejected the value for {name} {failures} times. Apologize and offer to transfer \
                        them to a human agent."
                    )
                } else {
                    format!("The caller said the value for {name} is wrong, so it was dropped. Apologize for the mistake.")
                }
            }
            (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("false")),
            _ => format!("The caller said no, but {name} has no value to reject."),
        }
    }
}