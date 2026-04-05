use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::WorkspaceDelta;

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

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let thought = args
            .get("thought")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|thought| !thought.is_empty())
            .ok_or_else(|| Error::Provider("scratch_thought requires a non-empty 'thought' string".to_string()))?
            .to_string();
        let metadata = match args.get("metadata") {
            None => Value::Null,
            Some(Value::Object(map)) => Value::Object(map.clone()),
            Some(Value::Null) => Value::Null,
            Some(_) => {
                return Err(Error::Provider(
                    "scratch_thought 'metadata' must be an object when provided".to_string(),
                ))
            }
        };

        Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::ScratchpadAppend { thought, metadata }))
    }
}
