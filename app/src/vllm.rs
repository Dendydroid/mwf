use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Debug)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Serialize, Debug)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub response_format: ResponseFormat,
}

#[derive(Deserialize, Debug)]
pub struct ActionDecision {
    pub selected_function: String,
    pub confidence: f32,
    pub language_detected: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChoiceMessage {
    content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Choice {
    message: ChoiceMessage,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChatResponse {
    choices: Vec<Choice>,
}