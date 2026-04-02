use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::HStackWorld;

pub mod exa_search;
pub mod identity;
pub mod light_compute;
pub mod scratch_thought;
pub mod search_stack;

pub use exa_search::ExaSearchTool;
pub use identity::IdentityTool;
pub use light_compute::LightComputeTool;
pub use scratch_thought::ScratchThought;
pub use search_stack::SearchStack;

/// The interface for all agentic tools.
/// Tools produce an AgentAction (the transition function `a`).
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    
    /// Executes the tool logic and returns the resulting action (transition).
    async fn execute(&self, args: Value, world: &dyn HStackWorld) -> Result<AgentAction, Error>;
}

/// Returns all built-in tool names available for composition.
pub fn available_tools() -> &'static [&'static str] {
    &[
        "identity",
        "search_stack",
        "scratch_thought",
        "exa_search",
        "light_compute",
    ]
}

/// Builds a toolset from ordered names, allowing composition per runtime/preset.
pub fn compose_tools(names: &[&str]) -> Result<Vec<Box<dyn Tool>>, Error> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for &name in names {
        let tool = match name {
            "identity" => Box::new(IdentityTool) as Box<dyn Tool>,
            "search_stack" => Box::new(SearchStack) as Box<dyn Tool>,
            "scratch_thought" => Box::new(ScratchThought) as Box<dyn Tool>,
            "exa_search" => {
                // Do not expose Exa search if credentials are unavailable.
                if std::env::var("EXA_API_KEY").ok().filter(|v| !v.trim().is_empty()).is_none() {
                    continue;
                }
                Box::new(ExaSearchTool::new()) as Box<dyn Tool>
            }
            "light_compute" => Box::new(LightComputeTool::new()) as Box<dyn Tool>,
            _ => {
                return Err(Error::Internal(format!(
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
