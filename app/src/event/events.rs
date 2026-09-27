use crate::event::call_session::{CallSessionLoaded, InitialContextHandler};
use crate::event::caller_intent::{CallStateHandler, IntentExtracted};
use crate::event::event_bus::{Dispatcher, DispatcherBuilder};
use crate::event::form::{FormCompleted, FormSubmitter};
use crate::settings::AppSettings;
use reqwest::Client;
use std::sync::Arc;

/*
    Here all events get registered
*/
pub fn events(settings: &AppSettings, http: &Client) -> Dispatcher {
    let mut b = DispatcherBuilder::default();

    b.add::<CallSessionLoaded, _>(Arc::new(InitialContextHandler));
    b.add::<IntentExtracted, _>(Arc::new(CallStateHandler));
    b.add::<FormCompleted, _>(Arc::new(FormSubmitter {
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
