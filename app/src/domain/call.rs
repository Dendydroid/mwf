use crate::app::AppState;
use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crate::vllm::SystemPrompt;

pub enum CallerIntent {
    SmallTalk,
    GetInformation,
    StartForm,
    ProvideFormFieldValue,
    CancelForm,
    PointToContextForFormFieldValue,
}

impl Display for CallerIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            CallerIntent::SmallTalk => "small_talk",
            CallerIntent::GetInformation => "get_information",
            CallerIntent::StartForm => "start_form",
            CallerIntent::ProvideFormFieldValue => "provide_form_field_value",
            CallerIntent::CancelForm => "cancel_form",
            CallerIntent::PointToContextForFormFieldValue => "point_to_context_for_form_field_value",
        })
    }
}

