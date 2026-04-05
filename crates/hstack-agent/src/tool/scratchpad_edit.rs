use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::WorkspaceDelta;

pub struct ScratchpadEditTool;

#[async_trait]
impl Tool for ScratchpadEditTool {
    fn name(&self) -> &str {
        "scratchpad_edit"
    }

    fn description(&self) -> &str {
        "Edits the scratchpad document by appending, replacing, inserting, or deleting lines."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["append", "replace", "insert", "delete"] },
                "start_line": { "type": "integer", "minimum": 0 },
                "delete_count": { "type": "integer", "minimum": 0 },
                "new_lines": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("scratchpad_edit requires an 'operation' string".to_string()))?;
        let start_line = args.get("start_line").and_then(Value::as_u64).unwrap_or(0) as usize;
        let new_lines = match args.get("new_lines") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| Error::Provider("scratchpad_edit 'new_lines' must contain strings".to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(Error::Provider("scratchpad_edit 'new_lines' must be an array of strings".to_string())),
        };

        let delta = match operation {
            "append" => WorkspaceDelta::ScratchpadPatch {
                start_line: usize::MAX,
                delete_count: 0,
                new_lines,
            },
            "replace" => WorkspaceDelta::ScratchpadPatch {
                start_line,
                delete_count: args.get("delete_count").and_then(Value::as_u64).unwrap_or(new_lines.len() as u64) as usize,
                new_lines,
            },
            "insert" => WorkspaceDelta::ScratchpadPatch {
                start_line,
                delete_count: 0,
                new_lines,
            },
            "delete" => WorkspaceDelta::ScratchpadPatch {
                start_line,
                delete_count: args.get("delete_count").and_then(Value::as_u64).unwrap_or(1) as usize,
                new_lines: Vec::new(),
            },
            _ => return Err(Error::Provider("scratchpad_edit received unsupported operation".to_string())),
        };

        Ok(AgentAction::UpdateWorkspace(delta))
    }
}