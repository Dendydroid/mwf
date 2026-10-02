use crate::domain::form::Form;
use crate::event::event_bus::{Dispatcher, Event, EventHandler};
use reqwest::Client;
use serde_json::json;
use sqlx::types::chrono::Local;
use std::time::Duration;
use tracing::{error, info, warn};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Dispatched once the caller has confirmed every field of a form.
pub struct FormCompleted {
    pub call_id: String,
    pub form: Form,
}

impl FormCompleted {
    pub fn new(call_id: &str, form: Form) -> Self {
        Self {
            call_id: call_id.to_string(),
            form,
        }
    }
}

impl Event for FormCompleted {}

/// POSTs a completed form to `FORM_SUBMIT_URL` in the background, so the call
/// goes on without waiting for it.
pub struct FormSubmitter {
    pub http: Client,
    pub url: Option<String>,
}

impl EventHandler<FormCompleted> for FormSubmitter {
    fn handle(&self, event: &mut FormCompleted, _: &Dispatcher) {
        let call_id = event.call_id.clone();

        let Some(url) = self.url.clone() else {
            warn!(call_id, "FORM_SUBMIT_URL is not set, the completed form was not sent");

            return;
        };

        // `completed_at` is what values said relative to the call, such as "next saturday", are relative to.
        let payload = json!({
            "call_id": call_id,
            "completed_at": Local::now().to_rfc3339(),
            "form": event.form,
        });

        info!("Form was submitted!: {}", payload);

        let request = self.http.post(url).timeout(TIMEOUT).json(&payload);

        tokio::spawn(async move {
            match request.send().await.and_then(|response| response.error_for_status()) {
                Ok(_) => info!(call_id, "Form submitted"),
                Err(e) => error!(call_id, "Could not submit the form: {e}"),
            }
        });
    }
}
