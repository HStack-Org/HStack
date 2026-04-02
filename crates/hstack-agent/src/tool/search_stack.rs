use async_trait::async_trait;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::HStackWorld;
use crate::tool::Tool;

/// Allows the agent to search the HStack world for relevant tickets.
pub struct SearchStack;

#[async_trait]
impl Tool for SearchStack {
    fn name(&self) -> &str {
        "search_stack"
    }

    fn description(&self) -> &str {
        "Searches the user's stack (HStackWorld) for relevant tickets or habits."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, world: &dyn HStackWorld) -> Result<AgentAction, Error> {
        let query = args.get("query").and_then(Value::as_str).unwrap_or("");
        let results = world.search_tickets(query).await.map_err(Error::World)?;

        Ok(AgentAction::UpdateWorkingMemory(
            WorkingMemoryDelta::AddTechnicalNoise(
                format!("search_stack:{query}"),
                serde_json::to_value(results).unwrap_or(Value::Null),
            ),
        ))
    }
}
