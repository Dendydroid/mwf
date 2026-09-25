use crate::event::call_session::{CallSessionLoaded, InitialContextHandler};
use crate::event::event_bus::{Dispatcher, DispatcherBuilder};
use std::sync::Arc;

/*
    Here all events get registered
*/
pub fn events() -> Dispatcher {
    let mut b = DispatcherBuilder::default();

    b.add::<CallSessionLoaded, _>(Arc::new(InitialContextHandler));

    //b.register(Arc::new(OnboardingSubscriber { repo, mailer }));

    // -- Alternative lightweight subscriber --
    //b.on::<MobileRegistrationEvent, _>(-100, |ev, _| {
    //    tracing::debug!(user_id = ev.user_id, "mobile registration");
    //});

    b.build()
}
