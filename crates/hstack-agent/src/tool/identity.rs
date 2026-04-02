use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::HStackWorld;
use crate::tool::Tool;

/// The Identity tool allows the agent to signal that it has finished its task.
pub struct IdentityTool;

#[async_trait]
impl Tool for IdentityTool {
    fn name(&self) -> &str {
        "identity"
    }

    fn description(&self) -> &str {
        "Signals that the task is complete and returns the final answer."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string", "description": "The final natural language response to the user." }
            },
            "required": ["answer"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld) -> Result<AgentAction, Error> {
        let answer = args
            .get("answer")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|answer| !answer.is_empty())
            .ok_or_else(|| Error::Provider("identity requires a non-empty 'answer' string".to_string()))?
            .to_string();
        Ok(AgentAction::Stop(answer))
    }
}
