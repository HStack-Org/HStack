use async_trait::async_trait;
use tauri::AppHandle;

use hstack_agent::memory::HStackWorld;
use hstack_agent::AgentControlSystem;
use hstack_core::stack_snapshot::StackSnapshot;
use hstack_core::sync::SyncAction;

use crate::app_state::load_tickets_state_raw;

/// Adapter linking `hstack-agent`'s view of the world to Tauri's local state.
#[derive(Clone)]
pub struct TauriHStackWorld {
    pub app: AppHandle,
}

#[async_trait]
impl HStackWorld for TauriHStackWorld {
    async fn get_stack_snapshot(&self) -> Result<StackSnapshot, String> {
        let (base_tickets, pending_actions) = load_tickets_state_raw(self.app.clone()).await?;
        Ok(StackSnapshot::new(base_tickets, pending_actions))
    }
}

/// A silent controller for the agent loop natively running in Tauri.
pub struct TauriAgentControl;

impl TauriAgentControl {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentControlSystem for TauriAgentControl {
    async fn validate_stack_action(
        &self,
        _action: &SyncAction,
    ) -> Result<(), hstack_agent::error::Error> {
        Ok(())
    }
}
