use hstack_core::ticket::Ticket;
use hstack_core::sync::SyncAction;
use hstack_core::stack_snapshot::StackSnapshot;
use hstack_core::settings::UserSettings;
use hstack_core::provider::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use async_trait::async_trait;

use crate::workspace::WorkspaceState;

/// Represents the agent's short-term scratchpad and reasoning history.
/// All tool results and intermediate thoughts live here first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemory {
    pub messages: Vec<Message>,
    pub technical_noise: Vec<Value>, // Raw tool outputs, etc.
    pub proposed_stack_actions: Vec<SyncAction>,
    pub workspace: WorkspaceState,
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            technical_noise: Vec::new(),
            proposed_stack_actions: Vec::new(),
            workspace: WorkspaceState::default(),
        }
    }

    pub fn push_message(&mut self, message: Message) {
        self.messages.push(message);
        let budget = self.workspace.budget.short_term_budget;
        self.messages = crate::workspace::retain_short_term_messages(&self.messages, budget);
    }
}

/// Represents the long-term, user-canonical state of the world.
/// This trait allows the harness to remain independent of the storage layer (Tauri vs Database).
#[async_trait]
pub trait HStackWorld: Send + Sync {
    /// Returns the host-owned stack state: canonical base plus the host's pending sync buffer.
    async fn get_stack_snapshot(&self) -> Result<StackSnapshot, String>;

    /// Returns the host-visible projected tickets, excluding the agent's local proposal buffer.
    async fn get_tickets(&self) -> Result<Vec<Ticket>, String> {
        Ok(self.get_stack_snapshot().await?.projected_host_tickets())
    }
    
    /// Returns a subset of tickets based on a search query or filter.
    async fn search_tickets(&self, query: &str) -> Result<Vec<Ticket>, String> {
        let query = query.to_lowercase();
        Ok(self
            .get_tickets()
            .await?
            .into_iter()
            .filter(|ticket| {
                ticket.title.to_lowercase().contains(&query)
                    || ticket
                        .notes
                        .as_ref()
                        .map(|notes| notes.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .collect())
    }

    async fn get_user_settings(&self) -> Result<UserSettings, String> {
        Ok(UserSettings::default())
    }
}

/// A simple in-memory implementation of HStackWorld for testing and basic use.
pub struct InMemoryWorld {
    pub tickets: Vec<Ticket>,
}

#[async_trait]
impl HStackWorld for InMemoryWorld {
    async fn get_stack_snapshot(&self) -> Result<StackSnapshot, String> {
        Ok(StackSnapshot::new(self.tickets.clone(), Vec::new()))
    }
}
