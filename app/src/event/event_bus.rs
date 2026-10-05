use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub trait Event: Send + 'static {
    fn is_propagation_stopped(&self) -> bool {
        false
    }
    fn stop_propagation(&mut self) {}
}

pub trait EventHandler<E: Event>: Send + Sync + 'static {
    fn priority(&self) -> i32 {
        0
    }

    fn handle(&self, event: &mut E, dispatcher: &Dispatcher) -> impl Future<Output = ()> + Send;
}

type HandlerFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

type BoxedHandler<E> = Box<dyn for<'a> Fn(&'a mut E, &'a Dispatcher) -> HandlerFuture<'a> + Send + Sync>;

type HandlerList<E> = Vec<(i32, BoxedHandler<E>)>;

#[derive(Default)]
pub struct DispatcherBuilder {
    slots: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl DispatcherBuilder {
    pub fn add<E, H>(&mut self, handler: Arc<H>) -> &mut Self
    where
        E: Event,
        H: EventHandler<E>,
    {
        let priority = handler.priority();
        self.push::<E>(
            priority,
            Box::new(move |event, dispatcher| {
                // the future outlives this call, so it owns its handler
                let handler = handler.clone();

                Box::pin(async move { handler.handle(event, dispatcher).await })
            }),
        )
    }

    pub fn on<E, F>(&mut self, priority: i32, f: F) -> &mut Self
    where
        E: Event,
        F: Fn(&mut E, &Dispatcher) + Send + Sync + 'static,
    {
        self.push::<E>(
            priority,
            Box::new(move |event, dispatcher| {
                f(event, dispatcher);

                Box::pin(async {})
            }),
        )
    }

    pub fn register<S: Subscriber>(&mut self, s: Arc<S>) -> &mut Self {
        S::subscriptions(self, s);
        self
    }

    fn push<E: Event>(&mut self, priority: i32, h: BoxedHandler<E>) -> &mut Self {
        let slot = self
            .slots
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(HandlerList::<E>::new()));

        let list = slot
            .downcast_mut::<HandlerList<E>>()
            .expect("slot is keyed by TypeId::of::<E>()");

        list.push((priority, h));
        // stable sort: equal priorities keep registration order, like Symfony
        list.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
        self
    }

    pub fn build(self) -> Dispatcher {
        Dispatcher { slots: self.slots }
    }
}

pub struct Dispatcher {
    slots: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
}

impl Dispatcher {
    pub async fn dispatch<E: Event>(&self, event: &mut E) {
        let Some(slot) = self.slots.get(&TypeId::of::<E>()) else {
            return;
        };
        let list = slot
            .downcast_ref::<HandlerList<E>>()
            .expect("Slot is keyed by TypeId::of::<E>()");

        for (_, handler) in list {
            if event.is_propagation_stopped() {
                break;
            }
            handler(event, self).await; // handlers may dispatch nested events
        }
    }
}

pub trait Subscriber: Sized + Send + Sync + 'static {
    fn subscriptions(b: &mut DispatcherBuilder, this: Arc<Self>);
}

/*
pub struct UserCreatedEvent { pub user_id: u64 }

impl Event for UserCreatedEvent {}

pub struct OnboardingSubscriber { repo: Arc<UserRepo>, mailer: Arc<Mailer> }

impl OnboardingSubscriber {
    fn onboard(&self, user_id: u64) { /* ... */ }
}

impl<E: UserCreatedEvent> EventHandler<E> for OnboardingSubscriber {
    async fn handle(&self, ev: &mut E, _: &Dispatcher) {
        self.onboard(ev.user_id(), ev.source());
    }
}

impl Subscriber for OnboardingSubscriber {
    fn subscriptions(b: &mut DispatcherBuilder, this: Arc<Self>) {
        b.add::<UserCreatedEvent, _>(this.clone());
        b.add::<MigratedRegistrationEvent, _>(this);
    }
}

// --- boot ---
let mut b = DispatcherBuilder::default();
b.register(Arc::new(OnboardingSubscriber { repo, mailer }));
// -- Alternative lightweight subscriber --
b.on::<MobileRegistrationEvent, _>(-100, |ev, _| {
    tracing::debug!(user_id = ev.user_id, "mobile registration");
});
let dispatcher = b.build();

// --- dispatch ---
let mut ev = MobileRegistrationEvent { user_id: 42 };
dispatcher.dispatch(&mut ev).await;

*/
