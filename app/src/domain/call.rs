use crate::app::AppState;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crate::vllm::SystemPrompt;

pub enum SupportedLanguage {
    DE,
    EN,
}

pub enum CallState {
    Idle,
    FormInProgress,
}

pub enum InstructionType {
    GetInformation,
    StartForm,
    CancelForm,
}

/// What a handler hands back: text the assistant can turn into an answer.
pub type HandlerResult = anyhow::Result<String>;

type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send>>;
type Handler = Box<dyn Fn(Arc<AppState>) -> HandlerFuture + Send + Sync>;

pub struct Instruction {
    instruction_type: InstructionType,
    handler: Handler,
    unique_key: String,
    llm_description: String,
}

impl Instruction {
    fn new<F, Fut>(
        unique_key: &str,
        instruction_type: InstructionType,
        llm_description: &str,
        handler: F,
    ) -> Self
    where
        F: Fn(Arc<AppState>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        Self {
            instruction_type,
            handler: Box::new(move |state| Box::pin(handler(state))),
            unique_key: String::from(unique_key),
            llm_description: String::from(llm_description),
        }
    }

    pub async fn run(&self, state: Arc<AppState>) -> HandlerResult {
        (self.handler)(state).await
    }

    pub fn unique_key(&self) -> &str {
        &self.unique_key
    }

    pub fn is_get_information(&self) -> bool {
        matches!(self.instruction_type, InstructionType::GetInformation)
    }

    /// The file, inside the prompts directory, holding the prompt that turns
    /// this instruction's result into an answer. Derived from the key so that
    /// adding an instruction means adding one file next to the others, with no
    /// name to register anywhere.
    pub fn answer_prompt_file(&self) -> String {
        format!("{}_prompt.txt", self.unique_key)
    }
}

pub struct InstructionRegistry {
    instructions: HashMap<String, Instruction>,
}

impl InstructionRegistry {
    pub fn add_instruction<F, Fut>(
        mut self,
        unique_key: &str,
        instruction_type: InstructionType,
        llm_description: &str,
        handler: F,
    ) -> Self
    where
        F: Fn(Arc<AppState>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.instructions.insert(
            String::from(unique_key),
            Instruction::new(unique_key, instruction_type, llm_description, handler),
        );

        self
    }

    pub fn get(&self, unique_key: &str) -> Option<&Instruction> {
        self.instructions.get(unique_key)
    }

    pub fn get_instruction_list_for_llm(&self, system_prompt: &str) -> String {
        let mut instructions_formatted = String::new();

        for (key, instruction) in &self.instructions {
            instructions_formatted += &*String::from(
                format!(
                    "`{}`: {}",
                    key,
                    instruction.llm_description,
                )
            );

            instructions_formatted += "\n\r";
        }

        String::from(
            system_prompt.replace(
                "<instructions>",
                &*instructions_formatted
            )
        )
    }
}

impl Default for InstructionRegistry {
    fn default() -> Self {
        InstructionRegistry {instructions: HashMap::new()}
    }
}
