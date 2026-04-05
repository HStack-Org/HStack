use async_trait::async_trait;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;

pub struct ScratchpadSearchTool;

#[async_trait]
impl Tool for ScratchpadSearchTool {
    fn name(&self) -> &str {
        "scratchpad_search"
    }

    fn description(&self) -> &str {
        "Searches the scratchpad document like a bounded ctrl+F and returns matching lines."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| Error::Provider("scratchpad_search requires a non-empty 'query' string".to_string()))?;

        Ok(AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            format!("scratchpad_search:{query}"),
            serde_json::json!({
                "matches": memory.workspace.search_scratchpad(query),
            }),
        )))
    }
}