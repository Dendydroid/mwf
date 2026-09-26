use crate::domain::call::{CallAction, CallerIntent, FormSupported};
use crate::domain::call_session::CallState;
use crate::domain::form::{Form, FormFieldKind, StepState};
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use tracing::info;

/// After this many rejected values for one field, the caller is offered a human.
const MAX_CONFIRMATION_FAILURES: u8 = 3;

/// Dispatched once machine #1 has classified an utterance. Handlers change
/// `state` and put what machine #2 needs to know into `backend_context`.
///
/// The event owns everything, because the dispatcher needs `'static` events:
/// the session's state is moved in before dispatching and moved back after.
pub struct IntentExtracted {
    pub call_id: String,
    pub intent: CallerIntent,
    pub form_field: Option<String>,
    pub form_field_value: Option<String>,
    /// What a `GetInformation` intent fetched, `None` when fetching failed.
    /// Fetched before dispatching, because handlers cannot await.
    pub information: Option<String>,
    pub state: CallState,
    pub backend_context: String,
    pub action: CallAction,
}

impl IntentExtracted {
    pub fn new(call_id: &str, intent: CallerIntent, state: CallState) -> Self {
        Self {
            call_id: call_id.to_string(),
            intent,
            form_field: None,
            form_field_value: None,
            information: None,
            state,
            backend_context: String::new(),
            action: CallAction::Continue,
        }
    }
}

impl Event for IntentExtracted {}

pub struct CallStateHandler;

impl EventHandler<IntentExtracted> for CallStateHandler {
    fn handle(&self, event: &mut IntentExtracted, _: &Dispatcher) {
        event.backend_context = match event.intent {
            CallerIntent::Unsupported => CallerIntent::unsupported_backend_context(),
            CallerIntent::GetInformation { selected } => match &event.information {
                Some(information) => format!("Requested information ({selected}): {information}"),
                None => format!(
                    "Could not get {selected} right now. Apologize and offer to help with something else."
                ),
            },
            CallerIntent::StartForm { form } => start_form(&mut event.state, form),
            CallerIntent::ProvideFormFieldValue
            | CallerIntent::ReferToContextForFormFieldValue
            | CallerIntent::CorrectFormFieldValue => match &mut event.state {
                CallState::FormInProgress(form) => fill(
                    form,
                    event.form_field.as_deref(),
                    event.form_field_value.as_deref(),
                ),
                CallState::Idle => {
                    "There is no form in progress, so there is nothing to fill. Ask how you can help.".to_string()
                }
            },
            CallerIntent::ConfirmYes => match &mut event.state {
                CallState::FormInProgress(form) => confirm(form),
                CallState::Idle => "The caller agreed. If you asked whether they need anything else, ask what \
                    they need."
                    .to_string(),
            },
            CallerIntent::ConfirmNo => match &mut event.state {
                CallState::FormInProgress(form) => reject(form),
                CallState::Idle => "The caller said no. Ask how you can help.".to_string(),
            },
            CallerIntent::CancelForm => match std::mem::take(&mut event.state) {
                CallState::FormInProgress(form) => format!(
                    "The {} form was cancelled, nothing of it was kept.",
                    form.kind
                ),
                CallState::Idle => "There is no form to cancel. Ask how you can help.".to_string(),
            },
            CallerIntent::Repeat => "The caller asked to hear your last answer again. Repeat it from the \
                conversation history, or ask how you can help if there is none."
                .to_string(),
            CallerIntent::EndCall => {
                event.state = CallState::Idle;
                event.action = CallAction::EndCall;

                "The caller is ending the call. Say a short, friendly goodbye and ask nothing.".to_string()
            }
            CallerIntent::TransferToHuman => {
                event.action = CallAction::TransferToHuman;

                "Tell the caller you are transferring them to a human agent now, and ask nothing.".to_string()
            }
        };

        if let CallState::FormInProgress(form) = &event.state {
            if form.is_filled() {
                info!(call_id = %event.call_id, form = %form.context_value(), "Form completed");

                event.state = CallState::Idle;
            }
        }
    }
}

fn start_form(state: &mut CallState, form: FormSupported) -> String {
    if let CallState::FormInProgress(current) = state {
        if current.kind == form {
            return format!("The {form} form is already in progress, continue with it.");
        }
    }

    *state = CallState::FormInProgress(form.build());

    format!("Started the {form} form.")
}

/// Fills the field the matcher named, or the current one when it named none
/// that exists.
fn fill(form: &mut Form, field: Option<&str>, value: Option<&str>) -> String {
    let Some(target) = field
        .and_then(|name| form.find_field(name))
        .or_else(|| form.current_field())
    else {
        return "The form has no field left to fill.".to_string();
    };
    let (name, kind) = (target.name.clone(), target.kind);

    let Some(text) = value else {
        return format!("The caller gave no value for {name}. Ask for it again.");
    };

    // Parsed with the field's own kind, so filling it cannot fail.
    match kind.parse(text) {
        Some(parsed) => {
            form.fill_field(&name, parsed);

            format!("Recorded \"{text}\" for {name}.")
        }
        None => format!("Could not understand \"{text}\" as a {kind:?} value for {name}. Ask for it again."),
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
                    "The caller confirmed {name}. The {} form is complete with {}. Tell the caller it is done.",
                    form.kind,
                    form.values_summary()
                )
            } else {
                format!("The caller confirmed {name}.")
            }
        }
        // A yes to a yes-or-no field is its value, not a confirmation.
        (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("true")),
        _ => format!("The caller said yes, but {name} has no value to confirm yet. Ask for it."),
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
                    "The caller rejected the value for {name} {failures} times. Apologize, ask for it once more \
                    and offer to transfer them to a human agent."
                )
            } else {
                format!("The caller said the value for {name} is wrong. Ask for it again.")
            }
        }
        (StepState::Queued, FormFieldKind::Bool) => fill(form, Some(&name), Some("false")),
        _ => format!("The caller said no, but {name} has no value to reject. Ask for it."),
    }
}
