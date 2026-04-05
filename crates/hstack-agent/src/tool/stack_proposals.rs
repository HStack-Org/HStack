use async_trait::async_trait;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;

pub struct CreateTicketTool;
pub struct DeleteTicketTool;
pub struct DeleteAllTicketsTool;
pub struct EditTicketTool;
pub struct AddCommuteTool;
pub struct GetDirectionsTool;
pub struct RemoveCommuteTool;
pub struct StartLiveDirectionsTool;
pub struct CreateCountdownTool;

#[async_trait]
impl Tool for CreateTicketTool {
    fn name(&self) -> &str { "create_ticket" }
    fn description(&self) -> &str { "Create a new ticket proposal in the user's stack." }
    fn parameters(&self) -> Value { tool_parameters("create_ticket") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let settings = world.get_user_settings().await.map_err(Error::World)?;
        build_stack_action(hstack_core::agent_proposals::build_create_ticket_actions(&args, &settings))
    }
}

#[async_trait]
impl Tool for DeleteTicketTool {
    fn name(&self) -> &str { "delete_ticket" }
    fn description(&self) -> &str { "Delete a ticket proposal from the user's stack given its ID string." }
    fn parameters(&self) -> Value { tool_parameters("delete_ticket") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let tickets = projected_tickets(world, memory).await?;
        build_stack_action(hstack_core::agent_proposals::build_delete_ticket_actions(&args, &tickets))
    }
}

#[async_trait]
impl Tool for DeleteAllTicketsTool {
    fn name(&self) -> &str { "delete_all_tickets" }
    fn description(&self) -> &str { "Delete all ticket proposals from the projected stack." }
    fn parameters(&self) -> Value { tool_parameters("delete_all_tickets") }
    async fn execute(&self, _args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let tickets = projected_tickets(world, memory).await?;
        Ok(compound_stack_actions(hstack_core::agent_proposals::build_delete_all_ticket_actions(&tickets)))
    }
}

#[async_trait]
impl Tool for EditTicketTool {
    fn name(&self) -> &str { "edit_ticket" }
    fn description(&self) -> &str { "Edit an existing ticket proposal in the user's stack." }
    fn parameters(&self) -> Value { tool_parameters("edit_ticket") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let tickets = projected_tickets(world, memory).await?;
        let settings = world.get_user_settings().await.map_err(Error::World)?;
        build_stack_action(hstack_core::agent_proposals::build_edit_ticket_actions(&args, &tickets, &settings))
    }
}

#[async_trait]
impl Tool for AddCommuteTool {
    fn name(&self) -> &str { "add_commute" }
    fn description(&self) -> &str { "Register a recurring commute proposal for the user." }
    fn parameters(&self) -> Value { tool_parameters("add_commute") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let settings = world.get_user_settings().await.map_err(Error::World)?;
        build_stack_action(hstack_core::agent_proposals::build_add_commute_actions(&args, &settings))
    }
}

#[async_trait]
impl Tool for GetDirectionsTool {
    fn name(&self) -> &str { "get_directions" }
    fn description(&self) -> &str { "Create a directions commute proposal between two places." }
    fn parameters(&self) -> Value { tool_parameters("get_directions") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let settings = world.get_user_settings().await.map_err(Error::World)?;
        build_stack_action(hstack_core::agent_proposals::build_get_directions_actions(&args, &settings))
    }
}

#[async_trait]
impl Tool for RemoveCommuteTool {
    fn name(&self) -> &str { "remove_commute" }
    fn description(&self) -> &str { "Remove a commute proposal by its ticket ID." }
    fn parameters(&self) -> Value { tool_parameters("remove_commute") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let tickets = projected_tickets(world, memory).await?;
        build_stack_action(hstack_core::agent_proposals::build_remove_commute_actions(&args, &tickets))
    }
}

#[async_trait]
impl Tool for StartLiveDirectionsTool {
    fn name(&self) -> &str { "start_live_directions" }
    fn description(&self) -> &str { "Start a live directions commute proposal with a deadline." }
    fn parameters(&self) -> Value { tool_parameters("start_live_directions") }
    async fn execute(&self, args: Value, world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let settings = world.get_user_settings().await.map_err(Error::World)?;
        build_stack_action(hstack_core::agent_proposals::build_start_live_directions_actions(&args, &settings))
    }
}

#[async_trait]
impl Tool for CreateCountdownTool {
    fn name(&self) -> &str { "create_countdown" }
    fn description(&self) -> &str { "Create a countdown ticket proposal with a live timer." }
    fn parameters(&self) -> Value { tool_parameters("create_countdown") }
    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        Ok(compound_stack_actions(hstack_core::agent_proposals::build_create_countdown_actions(&args)))
    }
}

async fn projected_tickets(world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<Vec<hstack_core::ticket::Ticket>, Error> {
    let snapshot = world.get_stack_snapshot().await.map_err(Error::World)?;
    Ok(snapshot.projected_agent_tickets(&memory.proposed_stack_actions))
}

fn build_stack_action(result: Result<Vec<hstack_core::sync::SyncAction>, String>) -> Result<AgentAction, Error> {
    result
        .map(compound_stack_actions)
        .map_err(Error::Provider)
}

fn compound_stack_actions(actions: Vec<hstack_core::sync::SyncAction>) -> AgentAction {
    let mut mapped = actions
        .into_iter()
        .map(AgentAction::UpdateStack)
        .collect::<Vec<_>>();
    if mapped.len() == 1 {
        mapped.remove(0)
    } else {
        AgentAction::Compound(mapped)
    }
}

fn tool_parameters(name: &str) -> Value {
    for tool in hstack_core::ticket::tool_schemas() {
        if tool.function.name == name {
            return tool.function.parameters;
        }
    }
    serde_json::json!({"type":"object","properties":{}})
}