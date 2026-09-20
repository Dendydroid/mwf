use crate::domain::call::SupportedLanguage;
use crate::event::event_bus::Event;

pub struct CallerSpokeEvent {
    pub transcript: String,
    pub detected_language: SupportedLanguage,
}

impl Event for CallerSpokeEvent {}
