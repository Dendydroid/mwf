use crate::app::AppState;
use std::collections::HashMap;

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

pub struct Instruction {
    instruction_type: InstructionType,
    handler: Box<dyn Fn(&mut AppState)>,
    unique_key: String,
    llm_description: String,
}

impl Instruction {
    fn new(
        unique_key: &str,
        instruction_type: InstructionType,
        llm_description: &str,
        handler: fn(&mut AppState),
    ) -> Self {
        Self {
            instruction_type,
            handler: Box::new(handler),
            unique_key: String::from(unique_key),
            llm_description: String::from(llm_description),
        }
    }
}

pub struct InstructionRegistry {
    instructions: HashMap<String, Instruction>,
}

impl InstructionRegistry {
    pub fn add_instruction(
        mut self,
        unique_key: &str,
        instruction_type: InstructionType,
        llm_description: &str,
        handler: fn(&mut AppState) -> (),
    ) -> Self {
        self.instructions.insert(
            String::from(unique_key),
            Instruction::new(unique_key, instruction_type, llm_description, handler),
        );

        self
    }
}

impl Default for InstructionRegistry {
    fn default() -> Self {
        InstructionRegistry {instructions: HashMap::new()}
    }
}
