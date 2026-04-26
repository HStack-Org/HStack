use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};

pub mod follow_up;
pub mod editor_edit;
pub mod filesystem_patch;
pub mod inspect_app;
pub mod execution_request;
pub mod identity;
pub mod light_compute;
pub mod manage_app;
pub mod microbash;
pub mod scratchpad_edit;
pub mod scratchpad_search;
pub mod scratch_thought;
pub mod search_stack;
pub mod stack_proposals;
pub mod web_search;

pub use editor_edit::EditorEditTool;
pub use follow_up::FollowUpTool;
pub use filesystem_patch::FilesystemPatchTool;
pub use inspect_app::InspectAppTool;
pub use execution_request::ExecutionRequestTool;
pub use identity::IdentityTool;
pub use light_compute::LightComputeTool;
pub use manage_app::ManageAppTool;
pub use microbash::MicrobashTool;
pub use scratchpad_edit::ScratchpadEditTool;
pub use scratchpad_search::ScratchpadSearchTool;
pub use scratch_thought::ScratchThought;
pub use search_stack::SearchStack;
pub use stack_proposals::{
    AddCommuteTool, CreateCountdownTool, CreateTicketTool, DeleteAllTicketsTool,
    DeleteTicketTool, EditTicketTool, GetDirectionsTool, RemoveCommuteTool,
    StartLiveDirectionsTool,
};
pub use web_search::{web_search_is_available, WebSearchTool};

/// The interface for all agentic tools.
/// Tools produce an AgentAction (the transition function `a`).
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    
    /// Executes the tool logic and returns the resulting action (transition).
    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error>;
}

pub fn light_compute_is_available() -> bool {
    true
}

/// Returns all built-in tool names available for composition.
pub fn available_tools() -> &'static [&'static str] {
    &[
        "identity",
        "follow_up",
        "search_stack",
        "scratch_thought",
        "web_search",
        "light_compute",
        "manage_app",
        "inspect_app",
        "editor_edit",
        "microbash",
        "filesystem_patch",
        "scratchpad_search",
        "execution_request",
        "scratchpad_edit",
        "create_ticket",
        "delete_ticket",
        "delete_all_tickets",
        "edit_ticket",
        "add_commute",
        "get_directions",
        "remove_commute",
        "start_live_directions",
        "create_countdown",
    ]
}

/// Builds a toolset from ordered names, allowing composition per runtime/preset.
pub fn compose_tools(names: &[&str]) -> Result<Vec<Box<dyn Tool>>, Error> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for &name in names {
        let tool = match name {
            "identity" => Box::new(IdentityTool) as Box<dyn Tool>,
            "follow_up" => Box::new(FollowUpTool) as Box<dyn Tool>,
            "search_stack" => Box::new(SearchStack) as Box<dyn Tool>,
            "scratch_thought" => Box::new(ScratchThought) as Box<dyn Tool>,
            "web_search" => {
                if !web_search_is_available() {
                    continue;
                }
                Box::new(WebSearchTool::new()?) as Box<dyn Tool>
            }
            "light_compute" => {
                if !light_compute_is_available() {
                    continue;
                }
                Box::new(LightComputeTool::new()) as Box<dyn Tool>
            }
            "manage_app" => Box::new(ManageAppTool) as Box<dyn Tool>,
            "inspect_app" => Box::new(InspectAppTool) as Box<dyn Tool>,
            "editor_edit" => Box::new(EditorEditTool::new()) as Box<dyn Tool>,
            "microbash" => Box::new(MicrobashTool::new()) as Box<dyn Tool>,
            "filesystem_patch" => Box::new(FilesystemPatchTool::new()) as Box<dyn Tool>,
            "scratchpad_search" => Box::new(ScratchpadSearchTool) as Box<dyn Tool>,
            "execution_request" => Box::new(ExecutionRequestTool) as Box<dyn Tool>,
            "scratchpad_edit" => Box::new(ScratchpadEditTool) as Box<dyn Tool>,
            "create_ticket" => Box::new(CreateTicketTool) as Box<dyn Tool>,
            "delete_ticket" => Box::new(DeleteTicketTool) as Box<dyn Tool>,
            "delete_all_tickets" => Box::new(DeleteAllTicketsTool) as Box<dyn Tool>,
            "edit_ticket" => Box::new(EditTicketTool) as Box<dyn Tool>,
            "add_commute" => Box::new(AddCommuteTool) as Box<dyn Tool>,
            "get_directions" => Box::new(GetDirectionsTool) as Box<dyn Tool>,
            "remove_commute" => Box::new(RemoveCommuteTool) as Box<dyn Tool>,
            "start_live_directions" => Box::new(StartLiveDirectionsTool) as Box<dyn Tool>,
            "create_countdown" => Box::new(CreateCountdownTool) as Box<dyn Tool>,
            _ => {
                return Err(Error::Configuration(format!(
                    "Unknown tool '{}'. Available tools: {}",
                    name,
                    available_tools().join(", ")
                )));
            }
        };
        tools.push(tool);
    }

    Ok(tools)
}
