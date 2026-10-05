use crate::event::call_session_loaded::{CallSessionLoadedEvent, InitialContextHandler};
use crate::event::caller_spoke::{CallerSpokeEvent, CallerSpokeHandler};
use crate::event::context_extracted::{ContextExtractedEvent, ContextExtractedHandler};
use crate::event::event_bus::{Dispatcher, DispatcherBuilder};
use crate::event::form_completed::{FormCompletedEvent, FormSubmitter};
use crate::event::intent_matched::{IntentMatchedEvent, IntentMatchedHandler};
use crate::settings::AppSettings;
use crate::vllm::VllmClient;
use reqwest::Client;
use std::sync::Arc;

/*
    Here all events get registered
*/
pub fn events(settings: &AppSettings, http: &Client, llm: &VllmClient) -> Dispatcher {
    let mut b = DispatcherBuilder::default();
    let llm = Arc::new(llm.clone());

    b.add::<CallSessionLoadedEvent, _>(Arc::new(InitialContextHandler));

    b.add::<CallerSpokeEvent, _>(Arc::new(CallerSpokeHandler { llm: llm.clone() }));
    b.add::<ContextExtractedEvent, _>(Arc::new(ContextExtractedHandler { llm: llm.clone() }));
    b.add::<IntentMatchedEvent, _>(Arc::new(IntentMatchedHandler {
        llm,
        http_client: Arc::new(http.clone()),
    }));

    b.add::<FormCompletedEvent, _>(Arc::new(FormSubmitter {
        http: http.clone(),
        url: settings.form_submit_url().map(str::to_string),
    }));

    //b.register(Arc::new(OnboardingSubscriber { repo, mailer }));

    // -- Alternative lightweight subscriber --
    //b.on::<MobileRegistrationEvent, _>(-100, |ev, _| {
    //    tracing::debug!(user_id = ev.user_id, "mobile registration");
    //});

    b.build()
}
