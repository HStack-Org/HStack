use async_trait::async_trait;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::HStackWorld;
use crate::tool::Tool;

/// Allows the agent to store internal thoughts or intermediate reasoning in working memory.
pub struct ScratchThought;

#[async_trait]
impl Tool for ScratchThought {
    fn name(&self) -> &str {
        "scratch_thought"
    }

    fn description(&self) -> &str {
        "Saves a technical thought or intermediate result to the agent's working memory."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thought": { "type": "string", "description": "The thought or reasoning step." },
                "metadata": { "type": "object", "description": "Optional structured data." }
            },
            "required": ["thought"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld) -> Result<AgentAction, Error> {
        let thought = args
            .get("thought")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let metadata = args.get("metadata").cloned().unwrap_or(Value::Null);

        Ok(AgentAction::UpdateWorkingMemory(
            WorkingMemoryDelta::AddTechnicalNoise(format!("thought:{thought}"), metadata),
        ))
    }
}
