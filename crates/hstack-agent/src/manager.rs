use crate::memory::{HStackWorld, WorkingMemory};
use crate::workspace::{compose_workspace_projection_message, compose_workspace_system_message};
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

/// A simple implementation that emits a fixed role-defining system prompt plus
/// a separate mounted workspace projection.
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

        // SPEC ANCHOR: docs/agent-workspace-viewport-spec.md and
        // docs/agent-harness-invariants.md require a fixed context layout:
        // one role-defining system prompt, then a separately mounted workspace
        // projection. Do not append workspace regions or replay short-term
        // history as additional ad hoc prompt fragments here.
        let mut messages = Vec::new();

        let system_content = compose_workspace_system_message(
            base_prompt,
            memory,
            &tickets,
            &settings,
            &stack_snapshot.pending_actions,
        );

        messages.push(Message {
            role: Role::System,
            content: Some(system_content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        messages.push(Message {
            role: Role::User,
            content: Some(compose_workspace_projection_message(memory, &tickets)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        Ok(messages)
    }
}
