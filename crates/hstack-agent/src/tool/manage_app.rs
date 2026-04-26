use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{app_is_available, AppId, WorkspaceDelta};

pub struct ManageAppTool;

#[async_trait]
impl Tool for ManageAppTool {
    fn name(&self) -> &str {
        "manage_app"
    }

    fn description(&self) -> &str {
        "Opens, closes, focuses, pins, unpins, or scrolls workspace apps through the dock control surface."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["open", "close", "focus", "pin", "unpin", "scroll_up", "scroll_down"] },
                "app_id": { "type": "string", "enum": ["scratchpad", "websearch", "stack-search", "compute", "file-tree", "editor", "file-search", "jobs"] },
                "lines": { "type": "integer", "minimum": 1, "description": "Optional scroll amount for scroll actions." }
            },
            "required": ["action", "app_id"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("manage_app requires an 'action' string".to_string()))?;
        let app_id = parse_app_id(args.get("app_id"))?;
        if !app_is_available(app_id) {
            return Err(Error::Provider(format!(
                "manage_app cannot operate on unavailable app '{}'",
                app_id.label()
            )));
        }
        let lines = args.get("lines").and_then(Value::as_i64).unwrap_or(5).max(1) as isize;

        let delta = match action {
            "open" => WorkspaceDelta::OpenApp(app_id),
            "close" => WorkspaceDelta::CloseApp(app_id),
            "focus" => WorkspaceDelta::FocusApp(app_id),
            "pin" => WorkspaceDelta::PinApp(app_id),
            "unpin" => WorkspaceDelta::UnpinApp(app_id),
            "scroll_up" => WorkspaceDelta::ScrollApp { app_id, delta: -lines },
            "scroll_down" => WorkspaceDelta::ScrollApp { app_id, delta: lines },
            _ => return Err(Error::Provider("manage_app received unsupported action".to_string())),
        };

        Ok(AgentAction::UpdateWorkspace(delta))
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
        _ => Err(Error::Provider("manage_app requires a valid 'app_id'".to_string())),
    }
}