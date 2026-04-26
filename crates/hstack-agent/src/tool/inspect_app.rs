use async_trait::async_trait;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{app_is_available, AppId};

pub struct InspectAppTool;

#[async_trait]
impl Tool for InspectAppTool {
    fn name(&self) -> &str {
        "inspect_app"
    }

    fn description(&self) -> &str {
        "Inspects the current visible viewport and lifecycle state of a workspace app."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "app_id": { "type": "string", "enum": ["scratchpad", "websearch", "stack-search", "compute", "file-tree", "editor", "file-search", "jobs"] }
            },
            "required": ["app_id"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let app_id = parse_app_id(args.get("app_id"))?;
        if !app_is_available(app_id) {
            return Err(Error::Provider(format!(
                "inspect_app cannot inspect unavailable app '{}'",
                app_id.label()
            )));
        }
        let label = app_id.label();
        Ok(AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            format!("inspect_app:{label}"),
            memory.workspace.inspect_app(app_id),
        )))
    }
}

fn parse_app_id(value: Option<&Value>) -> Result<AppId, Error> {
    match value.and_then(Value::as_str) {
        Some("scratchpad") => Ok(AppId::Scratchpad),
        Some("websearch") => Ok(AppId::WebSearch),
        Some("stack-search") => Ok(AppId::StackSearch),
        Some("compute") => Ok(AppId::Compute),
        Some("file-tree") => Ok(AppId::FileTree),
        Some("editor") => Ok(AppId::Editor),
        Some("file-search") => Ok(AppId::FileSearch),
        Some("jobs") => Ok(AppId::Jobs),
        _ => Err(Error::Provider("inspect_app requires a valid 'app_id'".to_string())),
    }
}