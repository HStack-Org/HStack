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
        "Edits the scratchpad document by appending or replacing a row span with a replacement text block."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["append", "replace", "insert", "delete"] },
                "row_start": { "type": "integer", "minimum": 0 },
                "row_end": { "type": "integer", "minimum": 0, "description": "Exclusive end row for replace/delete spans." },
                "replacement_text": { "type": "string", "description": "Replacement text block. Embedded newlines are allowed." }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("scratchpad_edit requires an 'operation' string".to_string()))?;
        let new_lines = parse_replacement_lines(&args, operation)?;

        let row_start = parse_row_start(&args, operation)?;
        let row_end = parse_row_end(&args, operation, row_start)?;
        if row_end < row_start {
            return Err(Error::Provider("scratchpad_edit requires row_end >= row_start".to_string()));
        }
        let delete_count = row_end - row_start;

        let delta = match operation {
            "append" => WorkspaceDelta::ScratchpadPatch {
                start_line: usize::MAX,
                delete_count: 0,
                new_lines,
            },
            "replace" => WorkspaceDelta::ScratchpadPatch {
                start_line: row_start,
                delete_count,
                new_lines,
            },
            "insert" => WorkspaceDelta::ScratchpadPatch {
                start_line: row_start,
                delete_count: 0,
                new_lines,
            },
            "delete" => WorkspaceDelta::ScratchpadPatch {
                start_line: row_start,
                delete_count,
                new_lines: Vec::new(),
            },
            _ => return Err(Error::Provider("scratchpad_edit received unsupported operation".to_string())),
        };

        Ok(AgentAction::UpdateWorkspace(delta))
    }
}

fn parse_replacement_lines(args: &Value, operation: &str) -> Result<Vec<String>, Error> {
    let Some(replacement_text) = args.get("replacement_text") else {
        return match operation {
            "append" | "replace" | "insert" => Err(Error::Provider(
                "scratchpad_edit requires string 'replacement_text' for append, replace, and insert"
                    .to_string(),
            )),
            "delete" => Ok(Vec::new()),
            _ => Err(Error::Provider("scratchpad_edit received unsupported operation".to_string())),
        };
    };

    let replacement_text = replacement_text
        .as_str()
        .ok_or_else(|| Error::Provider("scratchpad_edit 'replacement_text' must be a string".to_string()))?;
    Ok(split_replacement_text(replacement_text))
}

fn split_replacement_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if text.ends_with('\n') {
        let _ = lines.pop();
    }
    lines
}

fn parse_row_start(args: &Value, operation: &str) -> Result<usize, Error> {
    match operation {
        "append" => Ok(0),
        "replace" | "insert" | "delete" => args
            .get("row_start")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("scratchpad_edit requires integer 'row_start'".to_string())),
        _ => Err(Error::Provider("scratchpad_edit received unsupported operation".to_string())),
    }
}

fn parse_row_end(args: &Value, operation: &str, row_start: usize) -> Result<usize, Error> {
    match operation {
        "append" | "insert" => Ok(row_start),
        "replace" | "delete" => args
            .get("row_end")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("scratchpad_edit requires integer 'row_end' for replace and delete".to_string())),
        _ => Err(Error::Provider("scratchpad_edit received unsupported operation".to_string())),
    }
}