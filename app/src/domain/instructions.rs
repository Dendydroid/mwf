use crate::app::AppState;
use crate::domain::call::{Instruction, InstructionRegistry, InstructionType};

pub fn instructions() -> InstructionRegistry {
    let registry = InstructionRegistry::default();
    
    registry
        .add_instruction(
            "get_current_berlin_weather",
            InstructionType::GetInformation,
            "User wants to know current weather in Berlin, Germany",
            |state: &mut AppState| {
                let body = reqwest::get("https://httpbin.org/get")
                    .await?
                    .text()
                    .await?;
                let response = state
                    .http_client
                    .get("https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41&current_weather=true");
            }
        )
}
