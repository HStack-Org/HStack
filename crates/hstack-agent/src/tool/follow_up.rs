use async_trait::async_trait;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;

/// The FollowUp tool records the required clarification so the agent can still terminate via identity.
pub struct FollowUpTool;

#[async_trait]
impl Tool for FollowUpTool {
    fn name(&self) -> &str {
        "follow_up"
    }

    fn description(&self) -> &str {
        "Records the clarifying question and missing-information rationale when the request is underspecified or ambiguous."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The clarifying question to ask the user."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional short explanation of what is missing or ambiguous."
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|question| !question.is_empty())
            .ok_or_else(|| Error::Provider("follow_up requires a non-empty 'question' string".to_string()))?
            .to_string();

        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_string);

        Ok(AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "follow_up".to_string(),
            serde_json::json!({
                "question": question,
                "reason": reason,
            }),
        )))
    }
}