
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
    use crate::app::AppState;
    use crate::domain::call::{CallAction, CallerIntent, FormSupported, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::Reasoning;
    use crate::domain::flow::form_flow::next_step;
    use crate::domain::form::{Form, FormFieldValue};
    use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;

    #[derive(JsonSchema, Deserialize, Serialize, Debug, Default, Clone, PartialEq)]
    pub struct HintMap {
        pub caller_full_name: Option<String>,
        pub patient_full_name: Option<String>,
        pub date_of_birth_iso_8601: Option<String>,
        pub appointment_spoken_date: Option<String>,
        pub appointment_spoken_time: Option<String>,
        // The form the caller filled out last in this call. Set by the turn that completes a form,
        // never by the context extractor, so it is kept out of the extractor's schema.
        #[schemars(skip)]
        pub last_filled_out_form: Option<Form>,
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

        /// Puts what the caller already said into a form that was just started. Those values wait for the
        /// caller's confirmation like the ones given inside the form, so the caller is only asked whether
        /// they are right. A hint the field's validators refuse is left out, and the field is asked for.
        pub async fn prefill(&self, form: &mut Form, app_state: &AppState) {
            for (name, value) in self.form_values(form) {
                if form.kind.validate(app_state, name, &value).await.is_ok() {
                    form.fill_field(name, value);
                }
            }
        }

        /// The hints the form has a field for, each as its field's kind reads it. A hint the kind
        /// cannot read is left out.
        fn form_values(&self, form: &Form) -> Vec<(&'static str, FormFieldValue)> {
            let hints = match form.kind {
                FormSupported::DoctorAppointment => [
                    ("patient_name", self.patient_full_name.as_deref()),
                    ("date_of_birth", self.date_of_birth_iso_8601.as_deref()),
                    ("appointment_date", self.appointment_spoken_date.as_deref()),
                    ("appointment_time", self.appointment_spoken_time.as_deref()),
                ],
            };

            hints
                .into_iter()
                .filter_map(|(name, hint)| {
                    let value = form
                        .find_field(name)
                        .zip(hint)
                        .and_then(|(field, text)| field.kind.parse(text))?;

                    Some((name, value))
                })
                .collect()
        }
    }

    impl ValueSchema for Option<String> {
        fn accepts_null(&self) -> bool {
            true
        }

        fn valid_value_description(&self) -> &'static str {
            "what <utterance> says for it, or null when <utterance> does not say it"
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
                    "If value cannot be derived from <utterance>, set the key to null.".into(),
                    "Write `date_of_birth_iso_8601` as YYYY-MM-DD.".into(),
                    "Write `appointment_spoken_date` and `appointment_spoken_time` in the caller's own words, without working a date or time out.".into(),
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
                    "When introducing an abbreviation or acronym, expand it to its full form on first mention (e.g., 'API (Application Programming Interface)'). For common everyday terms like ID, HTML, or USB, keep them as abbreviations.",
                    "No markdown, lists, special characters or emojis: the reply is read out by a speech synthesizer",
                    "Follow <response_context> instructions when answering and take its content into consideration"
                ]
                    .map(String::from)
                    .to_vec()
            }
        }
    }

    pub async fn main_menu_intent_context_handler(
        intent: &ExtractedMainMenuIntent,
        session: &mut CallSession,
        http_client: &Client,
        app_state: &AppState,
    ) {
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
            CallerIntent::StartForm { form } => start_form(
                form,
                app_state,
                session,
            ).await,
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

    async fn start_form(form: FormSupported, app_state: &AppState, session: &mut CallSession) -> String {
        let mut built = form.build();
        session.data.call_memory.hint_map.prefill(&mut built, app_state).await;
        let context = format!("Started the {form} form.");
        session.call_turn_context.insert("next_step".into(), next_step(&built));
        session.data.state = CallState::FormInProgress(built);

        context
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::domain::form::StepState;

        #[test]
        fn hints_fill_a_started_form_and_wait_for_confirmation() {
            let hint_map = HintMap {
                patient_full_name: Some("John Smith".into()),
                // Not a date, so the field is asked for
                date_of_birth_iso_8601: Some("in June".into()),
                appointment_spoken_date: Some("next saturday".into()),
                appointment_spoken_time: Some("in the morning".into()),
                ..Default::default()
            };

            let mut form = FormSupported::DoctorAppointment.build();
            for (name, value) in hint_map.form_values(&form) {
                form.fill_field(name, value);
            }

            let state = |name: &str| form.find_field(name).unwrap().state;
            assert_eq!(state("patient_name"), StepState::AwaitingConfirmation);
            assert_eq!(state("date_of_birth"), StepState::Queued);
            assert_eq!(state("reason"), StepState::Queued);
            assert_eq!(state("appointment_date"), StepState::AwaitingConfirmation);
            assert_eq!(state("appointment_time"), StepState::AwaitingConfirmation);
            assert_eq!(
                form.find_field("appointment_time").unwrap().value,
                Some(FormFieldValue::SpokenTime("in the morning".into()))
            );
            assert_eq!(
                next_step(&form),
                "Next, ask the caller to confirm that Full name of the patient is John Smith."
            );
        }
    }
}

pub mod form_flow {
    use std::collections::HashSet;
    use reqwest::Client;
    use schemars::JsonSchema;
    use lingua::Language;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use tracing::error;
    use crate::app::AppState;
    use crate::domain::call::{CallAction, CallerIntent, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::Reasoning;
    use crate::domain::form::{Form, FormField, FormFieldKind, FormFieldValue, StepState, ValidationError};
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
            `spoken_date` write the caller's words for the day in English and lowercase, such as \"next saturday\" \
            or \"tomorrow\", without working the date out. For a field of kind `spoken_time` write the caller's \
            words for the time of day the same way, such as \"in the morning\" or \"3 pm\". Write numbers as \
            digits, yes and no as true and false, and text as the caller said it"
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

        /// Holds `form_field` to the fields the caller can be answering. Asked for the day, the model files
        /// "tomorrow at 3 pm" or a bare "morgen" under the time, and no rule talked it out of that.
        fn fit_schema(schema: &mut Value, call_session: &CallSession) {
            if let CallState::FormInProgress(form) = &call_session.data.state {
                schema["properties"]["form_field"]["enum"] = json!(form.answerable_fields());
            }
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
                    "One exception is a value the caller points to instead of saying it, such as \"the same as before\": write the value they point to.".into(),
                    "The other is a value the caller changes only a part of: write the whole value with that part changed and the rest as it is in <form_state>. When <form_state> has 2001-04-09 and the caller says \"no, the tenth\" or \"yes, but in 2002\", write 2001-04-10 or 2002-04-09. When it has Maria Rossi and the caller says \"no, the last name is Russo\", write Maria Russo.".into(),
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
                    "Take <response_context> information into consideration when answering",
                    "When <next_step> is set in <context>, end the reply with that step and ask nothing \
                    else. To have a value confirmed, say what the value is for, read it back and ask plainly \
                    whether it is right",
                    "When introducing an abbreviation or acronym, expand it to its full form on first mention (e.g., 'API (Application Programming Interface)'). For common everyday terms like ID, HTML, or USB, keep them as abbreviations.",
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
        utterance: &str,
        session: &mut CallSession,
        http_client: &Client,
        app_state: &AppState,
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
            // A correction never accepts the value that was read back, whichever field it is for.
            CallerIntent::CorrectFormFieldValue => {
                validate_and_fill(app_state, form, form_value.form_field(), given_value, false).await
            }
            CallerIntent::ProvideFormFieldValue | CallerIntent::ReferToContextForFormFieldValue => {
                validate_and_fill(app_state, form, form_value.form_field(), given_value, true).await
            }
            // "Yes, but it is 1992": an agreement that comes with another value does not confirm the one read back.
            CallerIntent::ConfirmYes if replaces_read_back(form, form_value.form_field(), given_value, utterance) => {
                validate_and_fill(app_state, form, form_value.form_field(), given_value, false).await
            }
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

                session.call_turn_context.insert("next_step".into(), next_step(form));

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

    /// The field the extractor named, or the current one when it named none that exists.
    fn target<'a>(form: &'a Form, field: Option<&str>) -> Option<&'a FormField> {
        field
            .and_then(|name| form.find_field(name))
            .or_else(|| form.current_field())
    }

    /// Whether the field the value is for has this value.
    fn holds(form: &Form, field: Option<&str>, value: &str) -> bool {
        target(form, field)
            .and_then(|field| field.value.as_ref())
            .is_some_and(|held| held.to_string().to_lowercase() == value.to_lowercase())
    }

    /// Whether the caller gave `value` in place of the one that was read back to them, in an utterance the
    /// matcher took for an agreement. That has to be sure, because the extractor also writes the value the
    /// form has once more in its own words: it named the field, the field's kind can read the value, and
    /// what is new in a text are words the caller just said. Never for a yes or no field, where the
    /// agreement itself is written as a value.
    fn replaces_read_back(form: &Form, field: Option<&str>, value: Option<&str>, utterance: &str) -> bool {
        let Some((current, value)) = form.current_field().zip(value) else {
            return false;
        };
        let (Some(held), Some(given)) = (current.value.as_ref(), current.kind.parse(value)) else {
            return false;
        };
        if current.state != StepState::AwaitingConfirmation || field != Some(current.name.as_str()) {
            return false;
        }

        match given {
            FormFieldValue::Bool(_) => false,
            FormFieldValue::String(_) | FormFieldValue::SpokenDate(_) | FormFieldValue::SpokenTime(_) => {
                let (held, said) = (words(&held.to_string()), words(utterance));
                let new: Vec<_> = words(value).into_iter().filter(|word| !held.contains(word)).collect();

                !new.is_empty() && new.iter().all(|word| said.contains(word))
            }
            // A date or a number is either the same one or another.
            given => given != *held,
        }
    }

    /// The words of a text, in lowercase.
    fn words(text: &str) -> HashSet<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// `fill`, once the validators of the field the value is for had their say about it.
    async fn validate_and_fill(
        app_state: &AppState,
        form: &mut Form,
        field: Option<&str>,
        value: Option<&str>,
        moves_on: bool,
    ) -> String {
        let read = target(form, field)
            .zip(value)
            .and_then(|(target, text)| Some((target, target.kind.parse(text)?)));

        let refused = match read {
            Some((target, parsed)) => form.kind.validate(app_state, &target.name, &parsed).await.err(),
            None => None,
        };

        fill(form, field, value, moves_on, refused)
    }

    /// Fills the field the value is for. `moves_on` says whether a value for a later field accepts the
    /// one read back for the current field. `refused` is what the field's validators have against the value.
    fn fill(
        form: &mut Form,
        field: Option<&str>,
        value: Option<&str>,
        moves_on: bool,
        refused: Option<ValidationError>,
    ) -> String {
        let Some(target) = target(form, field) else {
            return "The form has no field left to fill.".to_string();
        };
        let (name, kind) = (target.name.clone(), target.kind);
        let was_read_back = target.state == StepState::AwaitingConfirmation
            && form.current_field().is_some_and(|current| current.name == name);

        let Some(text) = value else {
            return format!("The caller gave no value for {name}.");
        };

        // A refused value is not recorded. In place of the one that was read back it still says
        // that one is wrong, like a value that cannot be read.
        if let Some(ValidationError(reason)) = refused {
            if was_read_back {
                form.reject_current();
            }

            return format!("\"{text}\" was not recorded for {name}. {reason}");
        }

        // Parsed with the field's own kind, so filling it cannot fail.
        match kind.parse(text) {
            Some(parsed) => {
                // Answering a later field means the caller moved on from the value read back for the
                // current one, which accepts it.
                if moves_on && form.is_ahead(&name) {
                    form.confirm_current();
                }
                form.fill_field(&name, parsed);

                format!("Recorded \"{text}\" for {name}.")
            }
            // Another value in place of the one that was read back says that one is wrong, even when
            // the new one cannot be read. Keeping it would only have the same answer come again.
            None if was_read_back => reject(form),
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
            // A yes to a yes-or-no field is its value, not a confirmation. Validators are not asked about it.
            (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("true"), false, None),
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
            (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("false"), false, None),
            _ => format!("The caller said no, but {name} has no value to reject."),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::domain::call::FormSupported;

        /// The doctor form with the name confirmed and the date of birth read back to the caller.
        fn form() -> Form {
            let mut form = FormSupported::DoctorAppointment.build();
            fill(&mut form, Some("patient_name"), Some("John Smith"), true, None);
            form.confirm_current();
            fill(&mut form, Some("date_of_birth"), Some("1991-06-13"), true, None);

            form
        }

        #[test]
        fn agreement_with_another_date_replaces_the_one_read_back() {
            let form = form();
            let field = Some("date_of_birth");

            assert!(replaces_read_back(&form, field, Some("1992-06-13"), "yes, but it's 1992"));
            // The same date, a part of a date, and a date the extractor named no field for
            assert!(!replaces_read_back(&form, field, Some("1991-06-13"), "yes, June 13th 1991"));
            assert!(!replaces_read_back(&form, field, Some("1992"), "yes, but it's 1992"));
            assert!(!replaces_read_back(&form, None, Some("1992-06-13"), "yes, but it's 1992"));
        }

        #[test]
        fn agreement_with_another_text_replaces_it_only_in_words_the_caller_said() {
            let mut form = FormSupported::DoctorAppointment.build();
            fill(&mut form, None, Some("John Smith"), true, None);
            let field = Some("patient_name");

            assert!(replaces_read_back(&form, field, Some("John Smyth"), "Yes, but it's Smyth with a y"));
            // The extractor writing the name once more in its own way, and a name with nothing new in it
            assert!(!replaces_read_back(&form, field, Some("Jon Smith"), "yes"));
            assert!(!replaces_read_back(&form, field, Some("Smith"), "yes, Smith"));
        }

        #[test]
        fn unreadable_value_in_place_of_the_one_read_back_drops_it() {
            let mut form = form();
            fill(&mut form, Some("date_of_birth"), Some("1992"), false, None);

            let field = form.find_field("date_of_birth").unwrap();
            assert_eq!(
                (field.state, &field.value, field.confirmation_failed_counter),
                (StepState::Queued, &None, 1)
            );
        }

        #[test]
        fn refused_value_is_not_recorded_and_drops_the_one_read_back() {
            let refused = || Some(ValidationError("Tell the caller why.".into()));
            let field = |form: &Form, name: &str| form.find_field(name).unwrap().clone();

            let mut replaced = form();
            let context = fill(&mut replaced, Some("date_of_birth"), Some("2091-06-13"), false, refused());
            assert_eq!(context, "\"2091-06-13\" was not recorded for date_of_birth. Tell the caller why.");
            assert_eq!(
                (field(&replaced, "date_of_birth").state, field(&replaced, "date_of_birth").value),
                (StepState::Queued, None)
            );

            // For a later field it neither accepts the value read back nor drops it
            let mut moved_on = form();
            fill(&mut moved_on, Some("reason"), Some("headache"), true, refused());
            assert_eq!(field(&moved_on, "date_of_birth").state, StepState::AwaitingConfirmation);
            assert_eq!(field(&moved_on, "reason").value, None);
        }

        #[test]
        fn only_moving_on_accepts_the_value_read_back() {
            let state = |form: &Form| form.find_field("date_of_birth").unwrap().state;

            let mut corrected = form();
            fill(&mut corrected, Some("reason"), Some("headache"), false, None);
            assert_eq!(state(&corrected), StepState::AwaitingConfirmation);

            let mut moved_on = form();
            fill(&mut moved_on, Some("reason"), Some("headache"), true, None);
            assert_eq!(state(&moved_on), StepState::Completed);
        }
    }
}