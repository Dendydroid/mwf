
/*
    A call is always in one flow, like in one menu of a game: the main menu, or a form once it is started.
    Each flow has its own machines, so they only know what the caller can say in it.

    Every text a flow says or shows a machine is in the vocabulary (`config/llm_vocabulary.toml`).
*/
use schemars::JsonSchema;
use serde::Deserialize;
use crate::domain::flow::form_flow::{ExtractedFormIntent, ExtractedFormValues};
use crate::domain::flow::main_menu_flow::{ExtractedMainMenuIntent, HintMap};
use crate::domain::machine::{OutputFormat, ValueSchema};
use crate::vocabulary::vocabulary;

/// What the context extractor of the flow took from the utterance.
pub enum FlowContext {
    MainMenu(HintMap),
    Form(ExtractedFormValues),
}

/// What the intent matcher of the flow chose. In a form it comes with the extracted values,
/// which only count when the intent says the caller gave some.
pub enum IntentMatched {
    MainMenu(ExtractedMainMenuIntent),
    Form(ExtractedFormIntent, ExtractedFormValues),
}

/// Who words what the caller hears in a turn.
pub enum Reply {
    /// Code: sentences of the vocabulary, said as they are.
    Said(String),
    /// The flow's response formulator, from what `<response_context>` tells it. `then` is what
    /// code says after it.
    Formulated { then: Option<String> },
}

/// Sentences as one reply, without the ones that are empty.
pub fn sentences(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(JsonSchema, Deserialize, Default, Debug)]
struct Reasoning(String);

impl ValueSchema for Reasoning {
    fn valid_value_description(&self) -> &'static str {
        &vocabulary().output_values.machine_reasoning
    }
}

#[derive(JsonSchema, Deserialize, Default, Debug)]
struct SpokenResponse(String);

impl ValueSchema for SpokenResponse {
    fn valid_value_description(&self) -> &'static str {
        &vocabulary().output_values.spoken_response
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
    use serde::{Deserialize, Serialize};
    use tracing::error;
    use crate::app::AppState;
    use crate::domain::call::{CallAction, CallerIntent, FormSupported, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::{sentences, Reasoning, Reply};
    use crate::domain::flow::form_flow::next_step;
    use crate::domain::form::{Form, FormFieldValue};
    use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;
    use crate::vocabulary::{fill_in, vocabulary};

    #[derive(JsonSchema, Deserialize, Serialize, Debug, Default, Clone, PartialEq)]
    pub struct HintMap {
        pub caller_full_name: Option<String>,
        pub patient_full_name: Option<String>,
        pub date_of_birth_iso_8601: Option<String>,
        pub appointment_spoken_date: Option<String>,
        pub appointment_spoken_time: Option<String>,
        pub appointment_reason: Option<String>,
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
            self.appointment_reason = other.appointment_reason.or(self.appointment_reason.take());
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

        /// What the formulator is told when the caller asks about the form they filled out last in the
        /// call: its summary, only if there is one.
        pub fn last_filled_out_form_information(&self) -> String {
            let instructions = &vocabulary().instructions;

            match &self.last_filled_out_form {
                Some(form) => fill_in(
                    &instructions.last_filled_out_form,
                    &[("form", &form.kind.to_string()), ("values", &form.values_summary())],
                ),
                None => instructions.no_filled_out_form.clone(),
            }
        }

        /// The hints the form has a field for, each as its field's kind reads it. A hint the kind
        /// cannot read is left out.
        fn form_values(&self, form: &Form) -> Vec<(&'static str, FormFieldValue)> {
            let hints = match form.kind {
                FormSupported::DoctorAppointment => [
                    ("patient_name", self.patient_full_name.as_deref()),
                    ("date_of_birth", self.date_of_birth_iso_8601.as_deref()),
                    ("reason", self.appointment_reason.as_deref()),
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
            &vocabulary().output_values.hint
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
                    ("appointment_reason", &self.appointment_reason as &dyn ValueSchema),
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
                Greeting,
                GetInformation {selected: GetInformationSupported::CalendarHelp},
                GetInformation {selected: GetInformationSupported::GetCurrentUAHPerEUR},
                GetInformation {selected: GetInformationSupported::GetCurrentWeatherInBerlin},
                GetInformation {selected: GetInformationSupported::LastFilledOutFormInformation},
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
            &vocabulary().output_values.caller_intent
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
            Self::from(&vocabulary().machines.main_menu_context_extractor)
        }

        pub fn main_menu_intent_matcher() -> Self {
            Self::from(&vocabulary().machines.main_menu_intent_matcher)
        }

        pub fn main_menu_response_formulator() -> Self {
            Self::from(&vocabulary().machines.main_menu_response_formulator)
        }
    }

    /// Returns who words the reply: code on the turn that starts a form, the formulator otherwise.
    pub async fn main_menu_intent_context_handler(
        intent: &ExtractedMainMenuIntent,
        session: &mut CallSession,
        http_client: &Client,
        app_state: &AppState,
    ) -> Reply {
        let caller_intent = CallerIntent::from_label(intent.caller_intent())
            .unwrap_or(CallerIntent::Unsupported);

        session.call_turn_outcome = CallTurnOutcome::matched(caller_intent, intent.machine_reasoning(), intent.confidence());

        let has_conversation_history = session.data.call_memory.conversation.len() > 0;
        let instructions = &vocabulary().instructions;

        let backend_context = match caller_intent {
            // Not fetched: the call's own hints have the form.
            CallerIntent::GetInformation { selected: GetInformationSupported::LastFilledOutFormInformation } => {
                session.data.call_memory.hint_map.last_filled_out_form_information()
            }
            CallerIntent::GetInformation { selected } => match selected.fetch(&http_client).await {
                Ok(information) => information,
                Err(e) => {
                    error!(call_id = %session.call_id, "Could not fetch {selected}: {e:#}");

                    instructions.service_unavailable.clone()
                }
            },
            // A form's replies are code's, its first one too.
            CallerIntent::StartForm { form } => return Reply::Said(start_form(
                form,
                app_state,
                session,
            ).await),
            CallerIntent::Repeat if has_conversation_history => instructions.repeat.clone(),
            CallerIntent::EndCall => {
                session.call_turn_outcome.action = CallAction::EndCall;

                instructions.end_call.clone()

                // TODO: End call logic
            }
            CallerIntent::TransferToHuman => {
                session.call_turn_outcome.action = CallAction::TransferToHuman;

                instructions.transfer.clone()

                // TODO: Transfer to human logic
            }
            CallerIntent::Greeting => fill_in(&instructions.greeting, &[("offers", &offers())]),
            // CallerIntent::Unsupported | CallerIntent::Repeat if has_conversation_just_started
            _ => fill_in(&instructions.unsupported, &[("offers", &offers())]),
        };

        session.call_turn_context.insert(
            "response_context".into(),
            backend_context
        );

        Reply::Formulated { then: None }
    }

    /// What the menu has an `offer` for in the vocabulary, each as it is named there.
    fn offers() -> String {
        IntendedMainMenuAction::default()
            .allowed_values()
            .iter()
            .filter_map(|value| vocabulary().intent(&value.to_string()).offer.as_deref())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Starts the form with what the caller already said in it. Returns what the caller hears: that
    /// it is started, and the question of its first step.
    async fn start_form(form: FormSupported, app_state: &AppState, session: &mut CallSession) -> String {
        let language = session.data.language;
        let mut built = form.build();
        session.data.call_memory.hint_map.prefill(&mut built, app_state).await;
        let reply = sentences(&[form.vocabulary().started.say(language), &next_step(&built, language)]);
        session.data.state = CallState::FormInProgress(built);

        reply
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use lingua::Language;
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
            // The first step reads the name back, in the vocabulary's words
            let confirm = form.kind.vocabulary().field("patient_name").confirm.say(Language::English);
            assert_eq!(next_step(&form, Language::English), fill_in(confirm, &[("value", "John Smith")]));
        }
    }
}

pub mod form_flow {
    use std::collections::{HashMap, HashSet};
    use reqwest::Client;
    use schemars::JsonSchema;
    use lingua::Language;
    use serde::Deserialize;
    use serde_json::{json, Map, Value};
    use tracing::error;
    use crate::app::AppState;
    use crate::domain::call::{CallAction, CallerIntent, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState, CallTurnOutcome};
    use crate::domain::flow::{sentences, Reasoning, Reply};
    use crate::domain::form::{Form, FormField, FormFieldKind, FormFieldValue, StepState, ValidationError};
    use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;
    use crate::vocabulary::{fill_in, vocabulary, Phrase};

    /// After this many rejected values for one field, the caller is offered a human.
    const MAX_CONFIRMATION_FAILURES: u8 = 3;

    /// What the utterance gives for the fields of the form, by the field's name: several values at
    /// once, like `HintMap` in the main menu, with the fields of the form in progress as its keys.
    #[derive(JsonSchema, Deserialize, Default, Debug, Clone)]
    pub struct ExtractedFormValues(HashMap<String, Option<String>>);

    impl ExtractedFormValues {
        /// What the utterance gives for the field. An empty value counts as none.
        pub fn value_of(&self, field: &str) -> Option<&str> {
            self.0.get(field)?.as_deref().filter(|value| !value.is_empty())
        }
    }

    impl OutputFormat for ExtractedFormValues {
        /// The keys are not known before the call's form is: `fit_schema` adds them, and the
        /// machine's rules say what to set them to.
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(std::iter::empty())
        }

        /// One key per field of the form the utterance can give a value for, each a string or null like a
        /// hint. See `Form::answerable_fields` for the field that is left out.
        fn fit_schema(schema: &mut Value, call_session: &CallSession) {
            if let CallState::FormInProgress(form) = &call_session.data.state {
                let names = form.answerable_fields();

                schema["properties"] = names
                    .iter()
                    .map(|name| (name.to_string(), json!({ "type": ["string", "null"], "minLength": 1 })))
                    .collect::<Map<_, _>>()
                    .into();
                schema["required"] = json!(names);
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
            &vocabulary().output_values.caller_intent
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

        /// Whether the matcher took the utterance for an agreement.
        pub fn is_agreement(&self) -> bool {
            self.caller_intent() == CallerIntent::ConfirmYes.to_string()
        }

        /// The answer for an utterance the matcher took for an agreement and the agreement checker
        /// found to be a question: the caller did not agree, they asked about the form. The
        /// confidence was the matcher's in the agreement.
        pub fn as_question(mut self) -> Self {
            let question = CallerIntent::GetInformation { selected: GetInformationSupported::FormInformation };
            self.caller_intent = IntendedFormAction(question.to_string());
            self.confidence = None;

            self
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

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    struct IsQuestion(bool);

    impl ValueSchema for IsQuestion {
        fn valid_value_description(&self) -> &'static str {
            &vocabulary().output_values.is_question
        }
    }

    /// What the agreement checker says of an utterance the matcher took for an agreement.
    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct CheckedAgreement {
        is_question: IsQuestion,
    }

    impl CheckedAgreement {
        pub fn is_question(&self) -> bool {
            self.is_question.0
        }
    }

    impl OutputFormat for CheckedAgreement {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("is_question", &self.is_question as &dyn ValueSchema),
            ].into_iter())
        }
    }

    #[derive(JsonSchema, Deserialize, Default, Debug)]
    struct SpokenAnswer(String);

    impl ValueSchema for SpokenAnswer {
        fn valid_value_description(&self) -> &'static str {
            &vocabulary().output_values.spoken_answer
        }
    }

    /// The form formulator's answer to a question about the form. Code asks the step's question
    /// after it.
    #[derive(JsonSchema, Deserialize, Default, Debug)]
    pub struct FormulatedAnswer {
        spoken_response: SpokenAnswer,
    }

    impl FormulatedAnswer {
        /// Its first sentence. Told to answer in one sentence and ask nothing, the model still goes
        /// on with a question or a step of its own in a third of its answers.
        pub fn into_spoken_answer(self) -> String {
            first_sentence(&self.spoken_response.0).to_string()
        }
    }

    impl OutputFormat for FormulatedAnswer {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("spoken_response", &self.spoken_response as &dyn ValueSchema),
            ].into_iter())
        }
    }

    /// The first sentence of `text`. A full stop after one or two letters or digits does not end
    /// it: the number of a day or a title, as in "13. Juni" and "Dr. Müller".
    fn first_sentence(text: &str) -> &str {
        let mut word_before = 0;

        for (at, c) in text.char_indices() {
            let after = at + c.len_utf8();
            let ends_here = matches!(c, '.' | '!' | '?')
                && text[after..].chars().next().map_or(true, char::is_whitespace)
                && !(c == '.' && (1..=2).contains(&word_before));

            if ends_here {
                return &text[..after];
            }
            word_before = if c.is_alphanumeric() { word_before + 1 } else { 0 };
        }

        text
    }

    impl Machine {
        /// `language` is the one the caller speaks: the days and times they say are kept in the
        /// language the vocabulary's examples for it are in.
        pub fn form_context_extractor(language: Language) -> Self {
            let texts = &vocabulary().machines.form_context_extractor;
            let examples = texts.examples.in_language(language);
            let said_as = [
                ("language", examples.language.as_str()),
                ("day", examples.day.as_str()),
                ("time", examples.time.as_str()),
            ];

            Self {
                role: texts.role.clone(),
                rules: texts.rules.iter().map(|rule| fill_in(rule, &said_as)).collect(),
            }
        }

        pub fn form_intent_matcher() -> Self {
            Self::from(&vocabulary().machines.form_intent_matcher)
        }

        /// Only asked about an utterance the matcher took for an agreement, and shown that utterance
        /// alone (`query_utterance_alone`): a question is never an agreement.
        pub fn form_agreement_checker() -> Self {
            Self::from(&vocabulary().machines.form_agreement_checker)
        }

        /// Only asked when the caller has a question about the form: every other reply of a form
        /// is code's. `language` is the one it answers in, named in its rules.
        pub fn form_response_formulator(language: Language) -> Self {
            let texts = &vocabulary().machines.form_response_formulator;
            let language = language.to_string();

            Self {
                role: texts.role.clone(),
                rules: texts.rules.iter().map(|rule| fill_in(rule, &[("language", &language)])).collect(),
            }
        }
    }

    /// What a turn of a form comes to.
    pub struct FormTurn {
        pub reply: Reply,
        // The form, once the caller has confirmed its last field: the call is back in the main menu then
        pub completed: Option<Form>,
    }

    impl FormTurn {
        fn said(reply: impl Into<String>) -> Self {
            Self {
                reply: Reply::Said(reply.into()),
                completed: None,
            }
        }
    }

    /// Changes the form by what the caller wants, and words the reply: what the turn has to say first,
    /// then the question of the step the form is on. Only a question about the form is left to the
    /// formulator.
    pub async fn form_intent_context_handler(
        intent: &ExtractedFormIntent,
        form_values: &ExtractedFormValues,
        utterance: &str,
        session: &mut CallSession,
        http_client: &Client,
        app_state: &AppState,
    ) -> FormTurn {
        let caller_intent = CallerIntent::from_label(intent.caller_intent())
            .unwrap_or(CallerIntent::Unsupported);

        session.call_turn_outcome = CallTurnOutcome::matched(caller_intent, intent.machine_reasoning(), intent.confidence());

        let language = session.data.language;

        let CallState::FormInProgress(form) = &mut session.data.state else {
            return FormTurn { reply: Reply::Formulated { then: None }, completed: None };
        };

        let given = given_values(form, form_values);
        // What of it is for the field the caller is on
        let for_current: Vec<_> = given
            .iter()
            .filter(|(name, _)| form.current_field().is_some_and(|current| current.name == *name))
            .cloned()
            .collect();

        // What the caller hears before the question of the step
        let said_first = match caller_intent {
            // The formulator answers the caller's question, and the step's question is asked after it.
            CallerIntent::GetInformation { selected } => {
                let instruction = match selected.fetch(&http_client).await {
                    Ok(information) => information,
                    Err(e) => {
                        error!(call_id = %session.call_id, "Could not fetch {selected}: {e:#}");

                        vocabulary().instructions.service_unavailable.clone()
                    }
                };
                session.call_turn_context.insert("response_context".into(), instruction);

                return FormTurn {
                    reply: Reply::Formulated { then: Some(next_step(form, language)) },
                    completed: None,
                };
            }
            // A correction that comes without a value is only the denial.
            CallerIntent::CorrectFormFieldValue if given.is_empty() => reject(form, language),
            // A correction never accepts the value that was read back, whichever fields it is for.
            CallerIntent::CorrectFormFieldValue => validate_and_fill(app_state, form, &given, false, language).await,
            // An answer fills what the caller has not confirmed yet: a confirmed value takes a correction.
            CallerIntent::ProvideFormFieldValue | CallerIntent::ReferToContextForFormFieldValue => {
                let unconfirmed: Vec<_> = given
                    .iter()
                    .filter(|(name, _)| form.find_field(name).is_some_and(|field| field.state != StepState::Completed))
                    .cloned()
                    .collect();

                validate_and_fill(app_state, form, &unconfirmed, true, language).await
            }
            // "Yes, but it is 1992": an agreement that comes with another value does not confirm the one read
            // back. Only that value is taken: on an agreement the extractor's other values are not the caller's.
            CallerIntent::ConfirmYes if replaces_read_back(form, &for_current, utterance) => {
                validate_and_fill(app_state, form, &for_current, false, language).await
            }
            CallerIntent::ConfirmYes => confirm(form, language),
            CallerIntent::ConfirmNo => reject(form, language),
            CallerIntent::CancelForm => {
                let cancelled = form.kind.vocabulary().cancelled.say(language);
                session.data.state = CallState::Idle;

                return FormTurn::said(cancelled);
            }
            // The last reply once more: it ends with the step's question already.
            CallerIntent::Repeat => {
                let again = session.data.last_spoken_response.clone();

                return FormTurn::said(again.unwrap_or_else(|| next_step(form, language)));
            }
            CallerIntent::EndCall => {
                session.data.state = CallState::Idle;
                session.call_turn_outcome.action = CallAction::EndCall;

                return FormTurn::said(Phrase::Goodbye.say(language));
            }
            // The form is kept, and after a transfer the assistant asks nothing.
            CallerIntent::TransferToHuman => {
                session.call_turn_outcome.action = CallAction::TransferToHuman;

                return FormTurn::said(Phrase::Transfer.say(language));
            }
            // CallerIntent::Unsupported
            _ => vec![Phrase::CannotHelpInForm.say(language)],
        };

        // The caller confirmed the last field: the form's own closing sentence is all they hear.
        if form.is_filled() {
            let completed = form.clone();
            session.data.state = CallState::Idle;

            return FormTurn {
                reply: Reply::Said(completed.kind.vocabulary().completed.say(language).to_string()),
                completed: Some(completed),
            };
        }

        let step = next_step(form, language);
        let mut reply: Vec<&str> = said_first;
        reply.push(&step);

        FormTurn::said(sentences(&reply))
    }

    /// The question of the step the caller is on, as they hear it: for the field's value, or whether
    /// the value it has is right. Always about the current field, the first one that is not confirmed.
    pub fn next_step(form: &Form, language: Language) -> String {
        let Some(field) = form.current_field() else {
            return String::new();
        };
        let texts = form.kind.vocabulary().field(&field.name);

        match field {
            FormField { value: Some(value), state: StepState::AwaitingConfirmation, .. } => {
                fill_in(texts.confirm.say(language), &[("value", &value.spoken(language))])
            }
            _ => texts.ask.say(language).to_string(),
        }
    }

    /// The values the caller gave, each with the name of its field, in the order of the form's fields.
    /// The extractor tends to write a value the form already has once more: the caller did not give that one.
    fn given_values(form: &Form, values: &ExtractedFormValues) -> Vec<(String, String)> {
        form.fields
            .iter()
            .filter_map(|field| {
                let value = values.value_of(&field.name).filter(|value| !holds(field, value))?;

                Some((field.name.clone(), value.to_string()))
            })
            .collect()
    }

    /// Whether the field has this value.
    fn holds(field: &FormField, value: &str) -> bool {
        field
            .value
            .as_ref()
            .is_some_and(|held| held.to_string().to_lowercase() == value.to_lowercase())
    }

    /// Whether the caller gave a value in place of the one that was read back to them, in an utterance the
    /// matcher took for an agreement. `for_current` is what the extractor wrote for the current field.
    /// That has to be sure, because the extractor also writes the value the form has once more in its own
    /// words: the field's kind can read the value, and what is new in a text are words the caller just
    /// said. Never for a yes or no field, where the agreement itself is written as a value.
    fn replaces_read_back(form: &Form, for_current: &[(String, String)], utterance: &str) -> bool {
        let Some((current, (_, value))) = form.current_field().zip(for_current.first()) else {
            return false;
        };
        let (Some(held), Some(given)) = (current.value.as_ref(), current.kind.parse(value)) else {
            return false;
        };
        if current.state != StepState::AwaitingConfirmation {
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

    /// `fill`, once the validators of each field a value is for had their say about it.
    async fn validate_and_fill(
        app_state: &AppState,
        form: &mut Form,
        given: &[(String, String)],
        moves_on: bool,
        language: Language,
    ) -> Vec<&'static str> {
        let mut refused = Vec::new();

        for (name, text) in given {
            refused.push(match form.find_field(name).and_then(|field| field.kind.parse(text)) {
                Some(parsed) => form.kind.validate(app_state, name, &parsed).await.err(),
                None => None,
            });
        }

        fill(form, given, moves_on, refused, language)
    }

    /// Fills every field the caller gave a value for, each then waiting for their confirmation. `moves_on`
    /// says whether values for later fields accept the one read back for the current field. `refused` is
    /// what the validators of each value's field have against it, in the order of `given`.
    ///
    /// Returns what the caller is told before the step's question, each sentence once: why a value was
    /// not taken. A recorded value needs no word: the step's question reads it back.
    fn fill(
        form: &mut Form,
        given: &[(String, String)],
        moves_on: bool,
        refused: Vec<Option<ValidationError>>,
        language: Language,
    ) -> Vec<&'static str> {
        let Some(current) = form.current_field() else {
            return vec![];
        };
        let (current, was_read_back) = (current.name.clone(), current.state == StepState::AwaitingConfirmation);

        if given.is_empty() {
            return vec![Phrase::NotUnderstood.say(language)];
        }

        let mut said = Vec::new();
        let mut read = Vec::new();

        for ((name, text), refused) in given.iter().zip(refused) {
            let Some(kind) = form.find_field(name).map(|field| field.kind) else {
                continue;
            };

            // Parsed with the field's own kind, so filling it cannot fail.
            match (refused, kind.parse(text)) {
                // A refused value is not recorded. In place of the one that was read back it still says
                // that one is wrong, like a value that cannot be read.
                (Some(error), _) => {
                    if *name == current {
                        form.reject_current();
                    }

                    said.push(error.say(language));
                }
                (None, Some(parsed)) => read.push((name, parsed)),
                // Another value in place of the one that was read back says that one is wrong, even when
                // the new one cannot be read. Keeping it would only have the same answer come again.
                (None, None) if *name == current && was_read_back => said.extend(reject(form, language)),
                (None, None) => said.push(Phrase::NotUnderstood.say(language)),
            }
        }

        // Answering later fields without a word about the current one means the caller moved on from the
        // value read back for it, which accepts it.
        let says_current = given.iter().any(|(name, _)| *name == current);
        if moves_on && !says_current && read.iter().any(|(name, _)| form.is_ahead(name)) {
            form.confirm_current();
        }

        for (name, parsed) in read {
            form.fill_field(name, parsed);
        }

        let mut once = Vec::new();
        for sentence in said {
            if !once.contains(&sentence) {
                once.push(sentence);
            }
        }

        once
    }

    /// The caller agreed. Returns what they are told before the step's question.
    fn confirm(form: &mut Form, language: Language) -> Vec<&'static str> {
        let Some(field) = form.current_field() else {
            return vec![];
        };
        let (name, kind, state) = (field.name.clone(), field.kind, field.state);

        match (state, kind) {
            (StepState::AwaitingConfirmation, _) => {
                form.confirm_current();

                vec![Phrase::Thanks.say(language)]
            }
            // A yes to a yes-or-no field is its value, not a confirmation. Validators are not asked about it.
            (StepState::Queued, FormFieldKind::Bool) => fill(form, &[(name, "true".to_string())], false, vec![None], language),
            // A yes with nothing to confirm: the step's question is all there is to say.
            _ => vec![],
        }
    }

    /// The caller denied. Returns what they are told before the step's question.
    fn reject(form: &mut Form, language: Language) -> Vec<&'static str> {
        let Some(field) = form.current_field() else {
            return vec![];
        };
        let (name, kind, state) = (field.name.clone(), field.kind, field.state);

        match (state, kind) {
            (StepState::AwaitingConfirmation, _) => {
                form.reject_current();

                let failures = form.current_field().map_or(0, |field| field.confirmation_failed_counter);
                if failures >= MAX_CONFIRMATION_FAILURES {
                    vec![Phrase::OfferHuman.say(language)]
                } else {
                    vec![Phrase::Sorry.say(language)]
                }
            }
            (StepState::Queued, FormFieldKind::Bool) => fill(form, &[(name, "false".to_string())], false, vec![None], language),
            // A no with nothing to reject: the step's question is all there is to say.
            _ => vec![],
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::domain::call::FormSupported;

        const LANGUAGE: Language = Language::English;

        fn given(values: &[(&str, &str)]) -> Vec<(String, String)> {
            values.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
        }

        /// `fill` with no validator against any of the values.
        fn fill_all(form: &mut Form, values: &[(&str, &str)], moves_on: bool) -> Vec<&'static str> {
            fill(form, &given(values), moves_on, values.iter().map(|_| None).collect(), LANGUAGE)
        }

        /// The doctor form with the name confirmed and the date of birth read back to the caller.
        fn form() -> Form {
            let mut form = FormSupported::DoctorAppointment.build();
            fill_all(&mut form, &[("patient_name", "John Smith")], true);
            form.confirm_current();
            fill_all(&mut form, &[("date_of_birth", "1991-06-13")], true);

            form
        }

        fn state(form: &Form, name: &str) -> StepState {
            form.find_field(name).unwrap().state
        }

        /// The question that asks for the field, and the one that reads `value` back for it, as the
        /// vocabulary words them.
        fn asks_for(form: &Form, name: &str) -> String {
            form.kind.vocabulary().field(name).ask.say(LANGUAGE).to_string()
        }

        fn reads_back(form: &Form, name: &str, value: &str) -> String {
            fill_in(form.kind.vocabulary().field(name).confirm.say(LANGUAGE), &[("value", value)])
        }

        #[test]
        fn one_utterance_fills_several_fields_and_each_waits_for_its_confirmation() {
            let mut form = FormSupported::DoctorAppointment.build();
            let said_first = fill_all(
                &mut form,
                &[("patient_name", "John Smith"), ("reason", "headache"), ("appointment_time", "at noon")],
                true,
            );

            // Recorded values need no word: the first of them is read back
            assert!(said_first.is_empty());
            assert_eq!(next_step(&form, LANGUAGE), reads_back(&form, "patient_name", "John Smith"));

            // The fields in between are still asked for, the filled ones only have to be confirmed
            form.confirm_current();
            assert_eq!(next_step(&form, LANGUAGE), asks_for(&form, "date_of_birth"));
            fill_all(&mut form, &[("date_of_birth", "1991-06-13")], true);
            assert_eq!(next_step(&form, LANGUAGE), reads_back(&form, "date_of_birth", "June 13, 1991"));
            form.confirm_current();
            assert_eq!(next_step(&form, LANGUAGE), reads_back(&form, "reason", "headache"));
            form.confirm_current();
            assert_eq!(next_step(&form, LANGUAGE), asks_for(&form, "appointment_date"));
            assert_eq!(state(&form, "appointment_time"), StepState::AwaitingConfirmation);
        }

        #[test]
        fn values_the_form_already_holds_were_not_given() {
            let form = form();
            let values: ExtractedFormValues = serde_json::from_str(
                r#"{"patient_name": "john smith", "date_of_birth": "1991-06-13", "reason": "headache", "appointment_date": null}"#,
            )
            .unwrap();

            assert_eq!(given_values(&form, &values), given(&[("reason", "headache")]));
        }

        #[test]
        fn agreement_with_another_date_replaces_the_one_read_back() {
            let form = form();
            let date = |value| given(&[("date_of_birth", value)]);

            assert!(replaces_read_back(&form, &date("1992-06-13"), "yes, but it's 1992"));
            // The same date, a part of a date, and nothing written for the field
            assert!(!replaces_read_back(&form, &date("1991-06-13"), "yes, June 13th 1991"));
            assert!(!replaces_read_back(&form, &date("1992"), "yes, but it's 1992"));
            assert!(!replaces_read_back(&form, &[], "yes, but it's 1992"));
        }

        #[test]
        fn agreement_with_another_text_replaces_it_only_in_words_the_caller_said() {
            let mut form = FormSupported::DoctorAppointment.build();
            fill_all(&mut form, &[("patient_name", "John Smith")], true);
            let name = |value| given(&[("patient_name", value)]);

            assert!(replaces_read_back(&form, &name("John Smyth"), "Yes, but it's Smyth with a y"));
            // The extractor writing the name once more in its own way, and a name with nothing new in it
            assert!(!replaces_read_back(&form, &name("Jon Smith"), "yes"));
            assert!(!replaces_read_back(&form, &name("Smith"), "yes, Smith"));
        }

        #[test]
        fn unreadable_value_in_place_of_the_one_read_back_drops_it() {
            let mut form = form();
            let said_first = fill_all(&mut form, &[("date_of_birth", "1992")], false);

            let field = form.find_field("date_of_birth").unwrap();
            assert_eq!(
                (field.state, &field.value, field.confirmation_failed_counter),
                (StepState::Queued, &None, 1)
            );
            // The caller hears the apology of a denial, and is asked for the date again
            assert_eq!(said_first, vec![Phrase::Sorry.say(LANGUAGE)]);
            assert_eq!(next_step(&form, LANGUAGE), asks_for(&form, "date_of_birth"));
        }

        #[test]
        fn refused_value_is_not_recorded_and_drops_the_one_read_back() {
            let refused = || vec![Some(ValidationError::DateOfBirthInTheFuture)];
            let field = |form: &Form, name: &str| form.find_field(name).unwrap().clone();

            let mut replaced = form();
            let said_first = fill(&mut replaced, &given(&[("date_of_birth", "2091-06-13")]), false, refused(), LANGUAGE);
            // The caller hears the validator's own sentence
            assert_eq!(said_first, vec![ValidationError::DateOfBirthInTheFuture.say(LANGUAGE)]);
            assert_eq!(
                (field(&replaced, "date_of_birth").state, field(&replaced, "date_of_birth").value),
                (StepState::Queued, None)
            );

            // For a later field it neither accepts the value read back nor drops it
            let mut moved_on = form();
            fill(&mut moved_on, &given(&[("reason", "headache")]), true, refused(), LANGUAGE);
            assert_eq!(field(&moved_on, "date_of_birth").state, StepState::AwaitingConfirmation);
            assert_eq!(field(&moved_on, "reason").value, None);
        }

        #[test]
        fn only_moving_on_accepts_the_value_read_back() {
            let mut corrected = form();
            fill_all(&mut corrected, &[("reason", "headache")], false);
            assert_eq!(state(&corrected, "date_of_birth"), StepState::AwaitingConfirmation);

            // Several later fields accept it once: none of them is taken for read back and accepted too
            let mut moved_on = form();
            fill_all(&mut moved_on, &[("reason", "headache"), ("appointment_date", "tomorrow")], true);
            assert_eq!(state(&moved_on, "date_of_birth"), StepState::Completed);
            assert_eq!(state(&moved_on, "reason"), StepState::AwaitingConfirmation);
            assert_eq!(state(&moved_on, "appointment_date"), StepState::AwaitingConfirmation);

            // A new value for the field itself is not moving on from it
            let mut replaced = form();
            fill_all(&mut replaced, &[("date_of_birth", "1992-06-13"), ("reason", "headache")], true);
            assert_eq!(state(&replaced, "date_of_birth"), StepState::AwaitingConfirmation);
            assert_eq!(state(&replaced, "reason"), StepState::AwaitingConfirmation);
        }

        #[test]
        fn yes_and_no_are_answered_with_a_sentence_of_the_vocabulary() {
            let mut form = form();
            assert_eq!(confirm(&mut form, LANGUAGE), vec![Phrase::Thanks.say(LANGUAGE)]);
            // Nothing waits for a yes or a no now: the step's question is all there is to say
            assert!(confirm(&mut form, LANGUAGE).is_empty());
            assert!(reject(&mut form, LANGUAGE).is_empty());

            // The third no to one field offers a human
            for said_first in [Phrase::Sorry, Phrase::Sorry, Phrase::OfferHuman] {
                fill_all(&mut form, &[("reason", "headache")], true);
                assert_eq!(reject(&mut form, LANGUAGE), vec![said_first.say(LANGUAGE)]);
            }
        }

        #[test]
        fn value_that_cannot_be_read_is_said_not_to_be_understood() {
            let mut form = FormSupported::DoctorAppointment.build();
            // Not a date. The reason said with it is recorded all the same
            let said_first = fill_all(&mut form, &[("date_of_birth", "in June"), ("reason", "headache")], true);
            assert_eq!(said_first, vec![Phrase::NotUnderstood.say(LANGUAGE)]);
            assert_eq!(state(&form, "date_of_birth"), StepState::Queued);
            assert_eq!(state(&form, "reason"), StepState::AwaitingConfirmation);

            // An answer that gave no value at all
            assert_eq!(fill_all(&mut form, &[], true), vec![Phrase::NotUnderstood.say(LANGUAGE)]);
        }

        #[test]
        fn agreement_that_was_a_question_is_a_question_about_the_form() {
            let matched = |intent: &str| -> ExtractedFormIntent {
                serde_json::from_value(json!({ "machine_reasoning": "The caller asks whether it is confirmed.", "caller_intent": intent })).unwrap()
            };

            // Only what the matcher took for an agreement is checked
            assert!(matched("confirm_yes").is_agreement());
            assert!(!matched("confirm_no").is_agreement());
            assert!(!matched("get_information[form_information]").is_agreement());

            let question = matched("confirm_yes").as_question();
            assert_eq!(
                CallerIntent::from_label(question.caller_intent()),
                Some(CallerIntent::GetInformation { selected: GetInformationSupported::FormInformation })
            );
            assert!(!question.is_agreement());
            // What the matcher wrote stays, but for how sure it was of the agreement
            assert_eq!(question.machine_reasoning(), "The caller asks whether it is confirmed.");
            assert_eq!(question.confidence(), None);

            // The checker's answer is true or false and nothing else
            let checked = |answer: &str| serde_json::from_str::<CheckedAgreement>(answer).map(|checked| checked.is_question());
            assert_eq!(checked(r#"{"is_question": true}"#).ok(), Some(true));
            assert_eq!(checked(r#"{"is_question": false}"#).ok(), Some(false));
            assert!(checked(r#"{"is_question": "yes"}"#).is_err());
        }

        #[test]
        fn answer_to_a_question_about_the_form_is_its_first_sentence() {
            assert_eq!(first_sentence("Ich habe Hans Müller notiert. Ist das korrekt?"), "Ich habe Hans Müller notiert.");
            assert_eq!(first_sentence("Noch nicht, wir buchen den Termin gleich."), "Noch nicht, wir buchen den Termin gleich.");
            // The number of a day and a title do not end a sentence
            assert_eq!(
                first_sentence("Das Geburtsdatum ist der 13. Juni 1991. Stimmt das?"),
                "Das Geburtsdatum ist der 13. Juni 1991."
            );
            assert_eq!(first_sentence("Ich habe Dr. Müller notiert. Und nun?"), "Ich habe Dr. Müller notiert.");
            assert_eq!(first_sentence("Hans Müller"), "Hans Müller");
        }
    }
}
