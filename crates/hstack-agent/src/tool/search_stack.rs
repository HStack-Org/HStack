use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{AppId, SearchResultRecord, WorkspaceDelta};

/// Allows the agent to search the HStack world for relevant tickets.
pub struct SearchStack;

#[async_trait]
impl Tool for SearchStack {
    fn name(&self) -> &str {
        "search_stack"
    }

    fn description(&self) -> &str {
        "Searches only the user's local HStack world for matching tickets, notes, habits, tasks, or events. Not for general web or world knowledge."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Query over local HStack content only." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| Error::Provider("search_stack requires a non-empty 'query' string".to_string()))?;
        let stack_snapshot = world.get_stack_snapshot().await.map_err(Error::World)?;
        let query_lower = query.to_lowercase();
        let results = stack_snapshot
            .projected_agent_tickets(&memory.proposed_stack_actions)
            .into_iter()
            .filter(|ticket| {
                ticket.title.to_lowercase().contains(&query_lower)
                    || ticket
                        .notes
                        .as_ref()
                        .map(|notes| notes.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        let records: Result<Vec<SearchResultRecord>, Error> = results
            .into_iter()
            .map(|ticket| {
                let metadata = serde_json::to_value(&ticket)
                    .map_err(|e| Error::Serialization(format!("Failed to serialize search_stack result: {e}")))?;
                Ok(SearchResultRecord {
                    title: ticket.title,
                    url: None,
                    snippet: ticket.notes.unwrap_or_default(),
                    metadata,
                })
            })
            .collect();

        Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::PublishSearchResults {
            app_id: AppId::StackSearch,
            query: query.to_string(),
            results: records?,
        }))
    }
}
