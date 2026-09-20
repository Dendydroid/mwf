use crate::app::AppState;
use crate::domain::call::{HandlerResult, InstructionRegistry, InstructionType};
use std::sync::Arc;

const BERLIN_WEATHER_URL: &str =
    "https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41&current_weather=true";

pub fn instructions() -> InstructionRegistry {
    InstructionRegistry::default().add_instruction(
        "get_current_berlin_weather",
        InstructionType::GetInformation,
        "User wants to know current weather in Berlin, Germany",
        get_current_berlin_weather,
    )
}

async fn get_current_berlin_weather(state: Arc<AppState>) -> HandlerResult {
    let forecast: serde_json::Value = state
        .http_client
        .get(BERLIN_WEATHER_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(forecast["current_weather"].to_string())
}
