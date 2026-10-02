
/*
    Machine #1 without an LLM: the classifier service scores a label per intent and finds the
    form values in the utterance, and the code here picks the intent with the call's state
*/
use crate::classifier::{ClassifierClient, ClassifierError, FoundValue};
use crate::domain::call::{CallerIntent, FormSupported, GetInformationSupported};
use crate::domain::call_session::{CallSession, CallState};
use crate::domain::form::{Form, FormField, FormFieldKind, StepState};
use crate::domain::machine::ExtractedIntent;

/// Below this the best label is not trusted, and the utterance is `unsupported`.
const LABEL_THRESHOLD: f64 = 0.3;
/// Ending the call cannot be taken back, so it takes a surer label: a bare "yes" or "no"
/// scores about 0.4 for "goodbye".
const END_CALL_THRESHOLD: f64 = 0.6;
/// Form values found with less than this are ignored.
const VALUE_THRESHOLD: f64 = 0.4;
/// After the first turn a language detected with less than this is ignored for the previous
/// turn's: a bare name or "ok" passes for several languages.
const LANGUAGE_THRESHOLD: f64 = 0.8;

/// Machine #1: what the caller wants, and which form value they gave.
pub async fn match_intent(
    classifier: &ClassifierClient,
    utterance: &str,
    session: &CallSession,
) -> Result<ExtractedIntent, ClassifierError> {
    let form = match &session.state {
        CallState::FormInProgress(form) => Some(form),
        CallState::Idle => None,
    };

    let labels = labels(form.and_then(Form::current_field));
    let asked: Vec<_> = labels.iter().map(|(name, description, _)| (*name, *description)).collect();
    let fields: Vec<_> = form
        .iter()
        .flat_map(|form| &form.fields)
        .map(|field| (field.name.as_str(), field.description.as_str()))
        .collect();

    let extraction = classifier.extract(utterance, &asked, &fields).await?;

    let (best, score) = extraction.labels.first().map_or(("", 0.0), |best| (best.label.as_str(), best.score));
    let intent = labels
        .iter()
        .find(|(name, ..)| *name == best)
        .map(|(.., intent)| *intent)
        .filter(|intent| score >= LABEL_THRESHOLD && (*intent != CallerIntent::EndCall || score >= END_CALL_THRESHOLD))
        .unwrap_or(CallerIntent::Unsupported);

    let (intent, form_value) = match form {
        Some(form) => decide(form, utterance, intent, &extraction.entities),
        None => (intent, None),
    };

    let previous = session.get_conversation().last().map(|turn| turn.caller_transcript.language.clone());
    let language = match extraction.language {
        Some(detected) if detected.confidence >= LANGUAGE_THRESHOLD || previous.is_none() => detected.code,
        _ => previous.unwrap_or_default(),
    };

    let mut reasoning = extraction
        .labels
        .iter()
        .take(3)
        .map(|label| format!("{} {:.2}", label.label, label.score))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some((field, value)) = &form_value {
        reasoning.push_str(&format!("; {field} = \"{value}\""));
    }

    Ok(ExtractedIntent::new(reasoning, language, intent, form_value, score))
}

/// The labels the classifier scores, for the form's current field: a short name, what the
/// caller does, and its intent.
fn labels(current: Option<&FormField>) -> Vec<(&'static str, &'static str, CallerIntent)> {
    let mut labels: Vec<_> = CallerIntent::all()
        .into_iter()
        .filter(|intent| is_asked(*intent, current))
        .filter_map(|intent| label(intent).map(|(name, description)| (name, description, intent)))
        .collect();

    // Last, because the classifier favours the labels listed first and these take anything
    labels.push((
        "other request",
        "asks about another city, another currency or something else the assistant does not offer",
        CallerIntent::Unsupported,
    ));
    labels.push(("greeting", "says hello, greets or makes small talk", CallerIntent::Unsupported));

    labels
}

/// A yes or a no is only asked about while a value waits for confirmation, and an answer only
/// while a field is asked for: offered anytime, the two take most utterances.
fn is_asked(intent: CallerIntent, current: Option<&FormField>) -> bool {
    use CallerIntent::*;

    let is_awaiting = current.is_some_and(|field| {
        field.state == StepState::AwaitingConfirmation || field.kind == FormFieldKind::Bool
    });

    match intent {
        ConfirmYes | ConfirmNo => is_awaiting,
        ProvideFormFieldValue => current.is_some_and(|field| field.state == StepState::Queued),
        CancelForm | GetInformation { selected: GetInformationSupported::FormInformation } => current.is_some(),
        _ => true,
    }
}

fn label(intent: CallerIntent) -> Option<(&'static str, &'static str)> {
    use CallerIntent::*;
    use GetInformationSupported::*;

    Some(match intent {
        GetInformation { selected: GetCurrentWeatherInBerlin } => ("weather", "asks about the weather in Berlin"),
        GetInformation { selected: GetCurrentUAHPerEUR } => (
            "exchange rate",
            "asks about the exchange rate of the euro to the Ukrainian hryvnia",
        ),
        GetInformation { selected: CalendarHelp } => ("calendar question", "asks which date or day of the week a day is"),
        GetInformation { selected: FormInformation } => (
            "booking question",
            "asks what details were recorded or whether the booking is done",
        ),
        // Any appointment: it is the only one there is, and callers rarely say "doctor"
        StartForm { form: FormSupported::DoctorAppointment } => ("doctor appointment", "wants to book an appointment"),
        ProvideFormFieldValue => ("answer", "answers the assistant's question with a name, a date or a reason"),
        CancelForm => ("cancel", "wants to cancel the booking"),
        ConfirmYes => ("yes", "agrees, confirms or says yes"),
        ConfirmNo => ("no", "disagrees, denies or says no"),
        Repeat => ("repeat", "did not hear or understand the last answer and asks to repeat it"),
        EndCall => ("goodbye", "says goodbye or that they need nothing else"),
        TransferToHuman => ("human", "asks to talk to a person, an operator or an agent"),
        // Its labels go last, see `labels`
        Unsupported => return None,
        // Decided from the form value found, not by a label of their own. A value pointed to
        // ("the same as before") is not supported: the classifier sees no history.
        CorrectFormFieldValue | ReferToContextForFormFieldValue => return None,
    })
}

/// A form value in the utterance turns an answer, a yes, a no or an unsupported utterance into
/// the form intent it means for the current field.
fn decide(
    form: &Form,
    utterance: &str,
    intent: CallerIntent,
    found: &[FoundValue],
) -> (CallerIntent, Option<(String, String)>) {
    use CallerIntent::*;

    let Some(current) = form.current_field() else {
        return (intent, None);
    };

    let value = form_value(form, current, found)
        .filter(|_| matches!(intent, ProvideFormFieldValue | ConfirmYes | ConfirmNo | Unsupported));
    let Some((field, value)) = value else {
        // A free text answer with no value found in it, such as a reason: all of it is the value
        if intent == ProvideFormFieldValue && current.state == StepState::Queued && current.kind == FormFieldKind::String {
            return (intent, Some((current.name.clone(), utterance.trim().to_string())));
        }

        return (intent, None);
    };

    let is_ahead = form.is_ahead(&field.name);
    if intent == ConfirmYes && current.state == StepState::AwaitingConfirmation && !is_ahead {
        // "yes, Taras" confirms the value read back
        return (ConfirmYes, None);
    }

    let is_correction = field.state == StepState::Completed || current.state == StepState::AwaitingConfirmation;
    let intent = if is_correction && !is_ahead { CorrectFormFieldValue } else { ProvideFormFieldValue };

    (intent, Some((field.name.clone(), value)))
}

/// The form value the utterance gave: one for the current field first, then the likeliest.
fn form_value<'a>(form: &'a Form, current: &'a FormField, found: &[FoundValue]) -> Option<(&'a FormField, String)> {
    let is_date = |kind| matches!(kind, FormFieldKind::Date | FormFieldKind::SpokenDate);

    found
        .iter()
        .filter(|found| found.score >= VALUE_THRESHOLD)
        .filter_map(|found| {
            let mut field = form.find_field(&found.field)?;
            // A date is for the date being asked for: without context the classifier takes
            // "the 15th of June" for the appointment as often as for the birth
            if is_date(field.kind) && is_date(current.kind) {
                field = current;
            }

            let value = match field.kind {
                FormFieldKind::Date => found.date.clone()?,
                kind => as_value(kind, &found.text),
            };

            Some((field, value, found.score))
        })
        .max_by(|a, b| (a.0.name == current.name).cmp(&(b.0.name == current.name)).then(a.2.total_cmp(&b.2)))
        .map(|(field, value, _)| (field, value))
}

/// A value as the form takes it: a spoken date in lowercase, like the LLM writes it.
fn as_value(kind: FormFieldKind, text: &str) -> String {
    match kind {
        FormFieldKind::SpokenDate => text.trim().to_lowercase(),
        _ => text.trim().to_string(),
    }
}
