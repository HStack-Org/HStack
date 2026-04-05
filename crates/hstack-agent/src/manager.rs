use crate::memory::{HStackWorld, WorkingMemory};
use crate::workspace::{compose_workspace_system_message, short_term_messages, workspace_runtime_snapshot};
use hstack_core::provider::{Message, Role};
use async_trait::async_trait;
use crate::error::Error;

/// Constructs the prompt for the provider by fusing the persistent world (HStackWorld)
/// and the short-term reasoning context (WorkingMemory).
#[async_trait]
pub trait ContextManager: Send + Sync {
    async fn construct_context(
        &self,
        world: &dyn HStackWorld,
        memory: &WorkingMemory,
        base_prompt: &str,
    ) -> Result<Vec<Message>, Error>;
}

/// A simple implementation that appends the entire world state to the system prompt.
pub struct SimpleContextManager;

#[async_trait]
impl ContextManager for SimpleContextManager {
    async fn construct_context(
        &self,
        world: &dyn HStackWorld,
        memory: &WorkingMemory,
        base_prompt: &str,
    ) -> Result<Vec<Message>, Error> {
        let stack_snapshot = world.get_stack_snapshot().await.map_err(Error::World)?;
        let tickets = stack_snapshot.projected_agent_tickets(&memory.proposed_stack_actions);
        let settings = world.get_user_settings().await.map_err(Error::World)?;

        let mut messages = Vec::new();

        let mut system_content = compose_workspace_system_message(
            base_prompt,
            memory,
            &tickets,
            &settings,
            &stack_snapshot.pending_actions,
        );
        let workspace_snapshot = workspace_runtime_snapshot(memory);
        system_content.push_str("\nWORKSPACE SNAPSHOT\n");
        let workspace_snapshot_json = serde_json::to_string_pretty(&workspace_snapshot)
            .map_err(|e| Error::Serialization(format!("Failed to serialize workspace snapshot: {e}")))?;
        system_content.push_str(&workspace_snapshot_json);
        
        messages.push(Message {
            role: Role::System,
            content: Some(system_content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        messages.extend(short_term_messages(memory));
        
        Ok(messages)
    }
}
