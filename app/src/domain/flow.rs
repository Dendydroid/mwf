use schemars::JsonSchema;
use serde::Deserialize;
use crate::domain::call::CallerIntent;
use crate::domain::machine::{AllowedValue, Machine, OutputFormat, ValueSchema};
use crate::vllm::Answer;

#[derive(JsonSchema, Deserialize, Default, Debug)]
struct Reasoning(String);

impl ValueSchema for Reasoning {
    fn valid_value_description(&self) -> &'static str {
        "A brief 1-sentence analysis of the conversation state based on rules and context."
    }
}

pub mod main_menu_flow {
    use reqwest::Client;
    use schemars::JsonSchema;
    use lingua::Language;
    use serde::Deserialize;
    use tracing::error;
    use crate::domain::call::{CallAction, CallerIntent, FormSupported, GetInformationSupported};
    use crate::domain::call_session::{CallSession, CallState};
    use crate::domain::flow::Reasoning;
    use crate::domain::flow::form_flow::next_step;
    use crate::domain::machine::{AllowedValue, Described, Machine, OutputFormat, ValueSchema};
    use crate::vllm::Answer;

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
    }

    impl ExtractedMainMenuIntent {
        /// For an intent that did not come from the LLM.
        pub fn new(
            machine_reasoning: String,
            caller_intent: CallerIntent,
        ) -> Self {
            Self {
                machine_reasoning: Reasoning(machine_reasoning),
                caller_intent: IntendedMainMenuAction(caller_intent.to_string()),
            }
        }

        pub fn machine_reasoning(&self) -> &str {
            &self.machine_reasoning.0
        }

        pub fn caller_intent(&self) -> &str {
            &self.caller_intent.0
        }

        /// `None` when the model answered with a label that is not an intent.
        pub fn intent(&self) -> Option<CallerIntent> {
            CallerIntent::from_label(&self.caller_intent.0)
        }
    }

    impl OutputFormat for ExtractedMainMenuIntent {
        fn iter_schemas(&self) -> Box<dyn Iterator<Item=(&'static str, &dyn ValueSchema)> + '_> {
            Box::new(vec![
                ("machine_reasoning", &self.machine_reasoning as &dyn ValueSchema),
                ("caller_intent", &self.caller_intent as &dyn ValueSchema),
            ].into_iter())
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
                "The caller is ending the call. Say a short, friendly goodbye and ask nothing.".to_string()

                // TODO: End call logic
            }
            CallerIntent::TransferToHuman => {
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
    use crate::domain::form::{Form, FormField, StepState};

    pub fn next_step(form: &Form) -> String {
        match form.current_field() {
            Some(FormField { description, value: Some(value), state: StepState::AwaitingConfirmation, .. }) => {
                format!("Next, ask the caller to confirm that {description} is {value}.")
            }
            Some(field) => format!("Next, ask the caller for: {}.", field.description),
            None => String::new(),
        }
    }
}