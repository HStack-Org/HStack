use serde::{Deserialize, Serialize};

use crate::location_utils::normalize_projected_tickets;
use crate::sync::{project_state, SyncAction};
use crate::ticket::Ticket;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StackSnapshot {
    pub base_tickets: Vec<Ticket>,
    pub pending_actions: Vec<SyncAction>,
}

impl StackSnapshot {
    pub fn new(base_tickets: Vec<Ticket>, pending_actions: Vec<SyncAction>) -> Self {
        Self {
            base_tickets,
            pending_actions,
        }
    }

    pub fn projected_host_tickets(&self) -> Vec<Ticket> {
        normalize_projected_tickets(project_state(
            self.base_tickets.clone(),
            &self.pending_actions,
        ))
    }

    pub fn projected_agent_tickets(&self, proposed_actions: &[SyncAction]) -> Vec<Ticket> {
        let host_projection = self.projected_host_tickets();
        normalize_projected_tickets(project_state(host_projection, proposed_actions))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::StackSnapshot;
    use crate::sync::{SyncAction, SyncActionType};
    use crate::ticket::{Ticket, TicketPayload, TicketStatus, TicketType};

    #[test]
    fn projects_host_and_agent_layers_separately() {
        let snapshot = StackSnapshot::new(
            vec![Ticket {
                id: "base-1".to_string(),
                title: "Base task".to_string(),
                r#type: TicketType::Task,
                status: TicketStatus::Idle,
                payload: TicketPayload::Task {
                    title: "Base task".to_string(),
                    scheduled_time_iso: None,
                    rrule: None,
                    duration_minutes: None,
                    status: None,
                    priority: None,
                    completed: Some(false),
                },
                notes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            vec![SyncAction {
                action_id: "pending-1".to_string(),
                r#type: SyncActionType::Create,
                entity_id: "host-1".to_string(),
                entity_type: "TASK".to_string(),
                status: Some(TicketStatus::Idle),
                payload: Some(TicketPayload::Task {
                    title: "Host pending".to_string(),
                    scheduled_time_iso: None,
                    rrule: None,
                    duration_minutes: None,
                    status: None,
                    priority: None,
                    completed: Some(false),
                }),
                notes: None,
                timestamp: Utc::now().to_rfc3339(),
            }],
        );

        let host_projection = snapshot.projected_host_tickets();
        assert_eq!(host_projection.len(), 2);
        assert!(host_projection.iter().any(|ticket| ticket.id == "host-1"));

        let agent_projection = snapshot.projected_agent_tickets(&[SyncAction {
            action_id: "proposal-1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "agent-1".to_string(),
            entity_type: "TASK".to_string(),
            status: Some(TicketStatus::Idle),
            payload: Some(TicketPayload::Task {
                title: "Agent proposal".to_string(),
                scheduled_time_iso: None,
                rrule: None,
                duration_minutes: None,
                status: None,
                priority: None,
                completed: Some(false),
            }),
            notes: None,
            timestamp: Utc::now().to_rfc3339(),
        }]);

        assert_eq!(agent_projection.len(), 3);
        assert!(agent_projection.iter().any(|ticket| ticket.id == "host-1"));
        assert!(agent_projection.iter().any(|ticket| ticket.id == "agent-1"));
    }
}