
/*
    Every text the assistant says to a caller or shows a machine is in `config/llm_vocabulary.toml`.
    The code has no text of its own: it finds one here by its key, which is an enum's variant or a
    form field's name.
*/
use std::collections::HashMap;
use std::sync::LazyLock;
use config::Config;
use lingua::Language;
use serde::{Deserialize, Serialize};
use time::Date;
use crate::domain::call::{flat_enum, CallerIntent, FormSupported};
use crate::domain::form::ValidationError;
use crate::settings::is_prompt_loop_mode;

const FILE: &str = "config/llm_vocabulary.toml";

static VOCABULARY: LazyLock<Vocabulary> = LazyLock::new(Vocabulary::load);

/// The vocabulary, read from its file the first time it is asked for.
pub fn vocabulary() -> &'static Vocabulary {
    &VOCABULARY
}

/// `text` with each `{name}` replaced by its value.
pub fn fill_in(text: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(text.to_string(), |text, (name, value)| text.replace(&format!("{{{name}}}"), value))
}

flat_enum! {
    /// A sentence code says to the caller as it is, whatever the form. Its wording in every
    /// language is under `[phrases.<variant in snake_case>]`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Phrase {
        // The caller said yes to a value that was read back
        Thanks,
        // The caller said no to it
        Sorry,
        // The third no to the value of one field
        OfferHuman,
        // A value that could not be read, or an answer that gave none
        NotUnderstood,
        // A request the form cannot take
        CannotHelpInForm,
        Goodbye,
        Transfer,
        // A machine failed, so the turn was taken back
        TurnFailedInMainMenu,
        TurnFailedInForm,
    }
}

impl Phrase {
    pub fn say(self, language: Language) -> &'static str {
        vocabulary().phrases[&self].say(language)
    }
}

/// A text in every language the callers speak.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized<T = String> {
    de: T,
    en: T,
    ru: T,
    uk: T,
}

impl<T> Localized<T> {
    pub fn in_language(&self, language: Language) -> &T {
        // No arm for "any other": a language the detector learns needs its texts here first
        match language {
            Language::German => &self.de,
            Language::English => &self.en,
            Language::Russian => &self.ru,
            Language::Ukrainian => &self.uk,
        }
    }

    fn in_every_language(&self) -> [&T; 4] {
        [&self.de, &self.en, &self.ru, &self.uk]
    }
}

impl Localized {
    /// The sentence as the caller hears it.
    pub fn say(&self, language: Language) -> &str {
        self.in_language(language)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    phrases: HashMap<Phrase, Localized>,
    validation: HashMap<ValidationError, Localized>,
    pub dates: Dates,
    forms: HashMap<FormSupported, FormVocabulary>,
    // By the intent's label
    intents: HashMap<String, IntentVocabulary>,
    pub machines: Machines,
    pub output_values: OutputValues,
    pub prompt: Prompt,
    pub instructions: Instructions,
    pub facts: Facts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dates {
    // With {day}, {month} and {year}
    format: Localized,
    // January to December, in the form the wording needs
    months: Localized<Vec<String>>,
}

impl Dates {
    /// A date as it is said, e.g. "13. Juni 1991".
    pub fn spoken(&self, date: Date, language: Language) -> String {
        let month = &self.months.in_language(language)[usize::from(u8::from(date.month())) - 1];

        fill_in(
            self.format.say(language),
            &[("day", &date.day().to_string()), ("month", month), ("year", &date.year().to_string())],
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormVocabulary {
    // Said when the form is started, before the question of its first step
    pub started: Localized,
    // Said when the caller has confirmed the last field
    pub completed: Localized,
    // Said when the caller stops filling it in
    pub cancelled: Localized,
    // By the field's name
    fields: HashMap<String, FieldVocabulary>,
}

impl FormVocabulary {
    pub fn field(&self, name: &str) -> &FieldVocabulary {
        self.fields
            .get(name)
            .unwrap_or_else(|| panic!("{FILE} has no texts for the form field `{name}`"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldVocabulary {
    // What the machines read about the field in <form_state>
    pub description: String,
    // The question that asks the caller for it
    pub ask: Localized,
    // The question that reads its value back, with {value}
    pub confirm: Localized,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentVocabulary {
    // How an intent matcher is told to match it
    pub description: String,
    // How it is named when the caller is told what the assistant can help with. Not named without one
    pub offer: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineVocabulary {
    pub role: String,
    pub rules: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    pub main_menu_context_extractor: MachineVocabulary,
    pub main_menu_intent_matcher: MachineVocabulary,
    pub main_menu_response_formulator: MachineVocabulary,
    pub form_context_extractor: FormContextExtractorVocabulary,
    pub form_intent_matcher: MachineVocabulary,
    pub form_agreement_checker: MachineVocabulary,
    pub form_response_formulator: MachineVocabulary,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormContextExtractorVocabulary {
    pub role: String,
    // With {language}, {day} and {time} of the examples
    pub rules: Vec<String>,
    pub examples: Localized<SpokenExamples>,
}

/// How a day and a time of day are written when the caller says them: the language, and examples
/// of each as they go into the rule.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokenExamples {
    pub language: String,
    pub day: String,
    pub time: String,
}

/// What a machine is told about a key of its answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputValues {
    pub machine_reasoning: String,
    pub caller_intent: String,
    pub is_question: String,
    pub hint: String,
    pub spoken_response: String,
    pub spoken_answer: String,
}

/// The rule a machine gets for each key of its answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
    // With {key} and {description}
    pub set_value: String,
    // With {key}
    pub only_allowed_values: String,
}

/// What a response formulator is told to say.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instructions {
    pub service_unavailable: String,
    pub repeat: String,
    pub end_call: String,
    pub transfer: String,
    // With {offers}
    pub unsupported: String,
    // With {offers}
    pub greeting: String,
    pub calendar_help: String,
    // With {form} and {values}
    pub last_filled_out_form: String,
    pub no_filled_out_form: String,
    pub form_question: String,
}

/// Facts a response formulator answers from, in words.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    // With {sky}, {temperature}, {windspeed} and {direction}
    pub weather: String,
    // With {rate} and {date}
    pub exchange_rate: String,
    pub today: String,
    // Clockwise from north, in steps of 22.5 degrees
    compass: Vec<String>,
    // By the WMO weather code, and `other`
    sky: HashMap<String, String>,
}

const OTHER_SKY: &str = "other";

impl Facts {
    pub fn sky(&self, weather_code: u64) -> &str {
        self.sky.get(&weather_code.to_string()).unwrap_or(&self.sky[OTHER_SKY])
    }

    /// Where a wind from `degrees` comes from, in words.
    pub fn direction(&self, degrees: f64) -> &str {
        let step = 360.0 / self.compass.len() as f64;

        &self.compass[(degrees.rem_euclid(360.0) / step).round() as usize % self.compass.len()]
    }
}

impl Vocabulary {
    fn load() -> Self {
        // Next to settings.toml, and found the same way
        let prefix = if is_prompt_loop_mode() { "app/" } else { "" };

        Config::builder()
            .add_source(config::File::with_name(&format!("{prefix}{FILE}")))
            .build()
            .unwrap_or_else(|e| panic!("{FILE} could not be read: {e}"))
            .try_deserialize()
            .unwrap_or_else(|e| panic!("{FILE} does not have the texts the code asks for: {e}"))
    }

    pub fn form(&self, form: FormSupported) -> &FormVocabulary {
        self.forms
            .get(&form)
            .unwrap_or_else(|| panic!("{FILE} has no texts for the form `{form}`"))
    }

    pub fn intent(&self, label: &str) -> &IntentVocabulary {
        self.intents
            .get(label)
            .unwrap_or_else(|| panic!("{FILE} has no texts for the intent `{label}`"))
    }

    pub fn validation(&self, error: ValidationError) -> &Localized {
        &self.validation[&error]
    }

    /// Panics with every text the code can ask for and the file does not have, so that is known
    /// when the app starts and not in the middle of a call.
    pub fn check(&self) {
        let mut problems = Vec::new();

        for phrase in Phrase::ALL {
            if !self.phrases.contains_key(phrase) {
                problems.push(format!("no [phrases.{}]", key(phrase)));
            }
        }
        for error in ValidationError::ALL {
            if !self.validation.contains_key(error) {
                problems.push(format!("no [validation.{}]", key(error)));
            }
        }
        for intent in CallerIntent::all() {
            if !self.intents.contains_key(&intent.to_string()) {
                problems.push(format!("no [intents.\"{intent}\"]"));
            }
        }
        for form in FormSupported::ALL {
            let Some(texts) = self.forms.get(form) else {
                problems.push(format!("no [forms.{form}]"));
                continue;
            };
            // Building the form asks for the texts of each of its fields
            for field in form.build().fields {
                let confirm = &texts.field(&field.name).confirm;
                if confirm.in_every_language().iter().any(|text| !text.contains("{value}")) {
                    problems.push(format!("[forms.{form}.fields.{}.confirm] has a text without {{value}}", field.name));
                }
            }
        }
        if self.dates.months.in_every_language().iter().any(|months| months.len() != 12) {
            problems.push("[dates.months] needs twelve months in every language".to_string());
        }
        if self.facts.compass.is_empty() {
            problems.push("[facts] compass is empty".to_string());
        }
        if !self.facts.sky.contains_key(OTHER_SKY) {
            problems.push(format!("no {OTHER_SKY} in [facts.sky]"));
        }

        assert!(problems.is_empty(), "{FILE}: {}", problems.join("; "));
    }
}

/// A variant as its key is written in the file.
fn key<T: Serialize>(variant: &T) -> String {
    serde_json::to_value(variant)
        .ok()
        .and_then(|name| name.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    #[test]
    fn file_has_every_text_the_code_asks_for() {
        vocabulary().check();
    }

    #[test]
    fn date_is_said_in_the_callers_language() {
        let date = Date::from_calendar_date(1991, Month::June, 13).unwrap();
        let spoken = |language| vocabulary().dates.spoken(date, language);

        assert_eq!(spoken(Language::German), "13. Juni 1991");
        assert_eq!(spoken(Language::English), "June 13, 1991");
        assert_eq!(spoken(Language::Russian), "13 июня 1991 года");
        assert_eq!(spoken(Language::Ukrainian), "13 червня 1991 року");
    }

    #[test]
    fn wind_direction_is_the_nearest_of_the_compass() {
        let facts = &vocabulary().facts;

        assert_eq!(facts.direction(0.0), "north");
        assert_eq!(facts.direction(287.0), "west-northwest");
        assert_eq!(facts.direction(350.0), "north");
        // A weather code that is not listed
        assert_eq!(facts.sky(3), "overcast");
        assert_eq!(facts.sky(4), facts.sky[OTHER_SKY]);
    }

    #[test]
    fn placeholders_are_filled_in() {
        assert_eq!(fill_in("{a} and {b}, {a}", &[("a", "1"), ("b", "2")]), "1 and 2, 1");
    }
}
