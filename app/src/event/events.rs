use crate::event::event_bus::{Dispatcher, DispatcherBuilder};

/*
    Here all events get registered
*/
pub fn events() -> Dispatcher {
    let b = DispatcherBuilder::default();

    //b.register(Arc::new(OnboardingSubscriber { repo, mailer }));

    // -- Alternative lightweight subscriber --
    //b.on::<MobileRegistrationEvent, _>(-100, |ev, _| {
    //    tracing::debug!(user_id = ev.user_id, "mobile registration");
    //});

    b.build()
}
