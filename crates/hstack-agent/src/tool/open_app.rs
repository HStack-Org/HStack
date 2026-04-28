use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{app_is_available, AppId, WorkspaceDelta};

pub struct OpenAppTool;

#[async_trait]
impl Tool for OpenAppTool {
    fn name(&self) -> &str {
        "open_app"
    }

    fn description(&self) -> &str {
        "Compatibility alias that opens a workspace app by id. Prefer manage_app for richer dock control."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "app_id": {
                    "type": "string",
                    "enum": [
                        "scratchpad",
                        "websearch",
                        "stack-search",
                        "stack_search",
                        "compute",
                        "cli",
                        "file-tree",
                        "file_tree",
                        "editor",
                        "file-search",
                        "file_search",
                        "jobs"
                    ]
                }
            },
            "required": ["app_id"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let app_id = match args.get("app_id").and_then(Value::as_str) {
            Some("scratchpad") => AppId::Scratchpad,
            Some("websearch") => AppId::WebSearch,
            Some("stack-search") | Some("stack_search") => AppId::StackSearch,
            Some("compute") => AppId::Compute,
            Some("cli") => AppId::Cli,
            Some("file-tree") | Some("file_tree") => AppId::FileTree,
            Some("editor") => AppId::Editor,
            Some("file-search") | Some("file_search") => AppId::FileSearch,
            Some("jobs") => AppId::Jobs,
            _ => return Err(Error::Provider("open_app requires a valid 'app_id'".to_string())),
        };

        if !app_is_available(app_id) {
            return Err(Error::Provider(format!(
                "open_app cannot operate on unavailable app '{}'",
                app_id.label()
            )));
        }

        Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::OpenApp(app_id)))
    }
}