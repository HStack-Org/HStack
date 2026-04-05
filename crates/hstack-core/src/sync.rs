use serde::{de, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use crate::ticket::{decode_ticket_payload_for_type, Ticket, TicketType, TicketStatus, TicketPayload};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncActionType {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncAction {
    pub action_id: String,
    pub r#type: SyncActionType,
    pub entity_id: String,
    pub entity_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TicketStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<TicketPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub timestamp: String,
}

#[derive(Deserialize)]
struct RawSyncAction {
    action_id: String,
    #[serde(rename = "type")]
    r#type: SyncActionType,
    entity_id: String,
    entity_type: String,
    #[serde(default)]
    status: Option<TicketStatus>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    notes: Option<String>,
    timestamp: String,
}

impl<'de> Deserialize<'de> for SyncAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSyncAction::deserialize(deserializer)?;
        let payload = match raw.payload {
            None => None,
            Some(value) => match raw.r#type {
                SyncActionType::Create => {
                    let ticket_type = match raw.entity_type.to_uppercase().as_str() {
                        "HABIT" => TicketType::Habit,
                        "EVENT" => TicketType::Event,
                        "COMMUTE" => TicketType::Commute,
                        "COUNTDOWN" => TicketType::Countdown,
                        _ => TicketType::Task,
                    };
                    Some(decode_ticket_payload_for_type(&ticket_type, value).map_err(de::Error::custom)?)
                }
                SyncActionType::Update => Some(TicketPayload::Generic(value)),
                SyncActionType::Delete => None,
            },
        };

        Ok(SyncAction {
            action_id: raw.action_id,
            r#type: raw.r#type,
            entity_id: raw.entity_id,
            entity_type: raw.entity_type,
            status: raw.status,
            payload,
            notes: raw.notes,
            timestamp: raw.timestamp,
        })
    }
}

#[derive(Serialize)]
struct HashStateItem {
    id: String,
    r#type: TicketType,
    payload: TicketPayload,
    status: TicketStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

/// Filters pending actions by removing those whose effects are already reflected in the base state.
/// This is the core of "Approach A": we keep local actions until the server state confirms them.
pub fn reconcile_state(base_tickets: &[Ticket], pending_actions: Vec<SyncAction>) -> Vec<SyncAction> {
    pending_actions
        .into_iter()
        .filter(|action| {
            match action.r#type {
                SyncActionType::Create => {
                    // Keep the action if the ticket is NOT yet in the base state
                    !base_tickets.iter().any(|t| t.id == action.entity_id)
                }
                SyncActionType::Update => {
                    // Find the ticket in the base state
                    match base_tickets.iter().find(|t| t.id == action.entity_id) {
                        // If it doesn't exist in base, it might be updating a pending creation, so keep it
                        None => true,
                        Some(t) => {
                            // Check if the base state matches the intended update
                            let status_matches = action.status.as_ref().is_none_or(|s| s == &t.status);
                            let notes_matches = action.notes.as_ref().is_none_or(|n| {
                                let norm_action = if n.is_empty() { None } else { Some(n.clone()) };
                                let norm_base = t.notes.as_ref().filter(|s| !s.is_empty()).cloned();
                                norm_action == norm_base
                            });
                            
                            let payload_matches = action.payload.as_ref().is_none_or(|p| match p {
                                TicketPayload::Generic(value) => value
                                    .as_object()
                                    .map(|updates| t.payload.matches_partial_update(updates))
                                    .unwrap_or(false),
                                other => other == &t.payload,
                            });

                            // Keep the action if either status or payload or notes doesn't match yet
                            !(status_matches && payload_matches && notes_matches)
                        }
                    }
                }
                SyncActionType::Delete => {
                    // Keep the action if the ticket IS still in the base state
                    base_tickets.iter().any(|t| t.id == action.entity_id)
                }
            }
        })
        .collect()
}

/// Projects the current state by applying a series of sync actions to a base set of tickets.
pub fn project_state(base_tickets: Vec<Ticket>, actions: &[SyncAction]) -> Vec<Ticket> {
    let mut effective_state = base_tickets;

    for action in actions {
        match action.r#type {
            SyncActionType::Create => {
                let type_ = match action.entity_type.to_lowercase().as_str() {
                    "habit" => TicketType::Habit,
                    "event" => TicketType::Event,
                    "commute" => TicketType::Commute,
                    "countdown" => TicketType::Countdown,
                    _ => TicketType::Task,
                };
                let payload = action.payload.clone().unwrap_or(TicketPayload::Generic(serde_json::json!({})));
                let title = payload.get_title().to_string();

                let ticket = Ticket {
                    id: action.entity_id.clone(),
                    r#type: type_,
                    status: action.status.clone().unwrap_or(TicketStatus::Idle),
                    payload,
                    notes: action.notes.clone(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    title,
                };
                effective_state.push(ticket);
            }
            SyncActionType::Update => {
                if let Some(ticket) = effective_state.iter_mut().find(|t| t.id == action.entity_id) {
                    // Apply type morphing
                    ticket.r#type = match action.entity_type.to_uppercase().as_str() {
                        "HABIT" => TicketType::Habit,
                        "EVENT" => TicketType::Event,
                        "COMMUTE" => TicketType::Commute,
                        "COUNTDOWN" => TicketType::Countdown,
                        _ => TicketType::Task,
                    };

                    if let Some(status) = &action.status {
                        ticket.status = status.clone();
                    }
                    if let Some(notes) = &action.notes {
                        ticket.notes = Some(notes.clone());
                    }
                    if let Some(new_payload) = &action.payload {
                        match (&mut ticket.payload, new_payload) {
                            (existing_payload, TicketPayload::Generic(new_obj)) => {
                                if let Some(updates) = new_obj.as_object() {
                                    existing_payload.apply_partial_update(updates);
                                }
                            }
                            // Otherwise replace whole payload for simplicity
                            (_, p) => {
                                ticket.payload = p.clone();
                            }
                        }
                        ticket.title = ticket.payload.get_title().to_string();
                    }
                    ticket.updated_at = chrono::Utc::now();
                }
            }
            SyncActionType::Delete => {
                effective_state.retain(|t| t.id != action.entity_id);
            }
        }
    }
    effective_state
}

/// Calculate the SHA-256 hash of the entire state to check synchronization integrity.
pub fn calculate_state_hash(tasks: &[Ticket]) -> Result<String, Error> {
    // 1. Map to consistent structure for hashing
    let mut state_list: Vec<HashStateItem> = tasks
        .iter()
        .map(|t| HashStateItem {
            id: t.id.clone(),
            r#type: t.r#type.clone(),
            payload: t.payload.clone(),
            status: t.status.clone(),
            notes: t.notes.clone(),
        })
        .collect();

    // 2. Sort by ID for deterministic hashing (as per JS client logic)
    // Note: Python server sorts by created_at, id ASC. 
    // For local-first client, ID sort is usually most stable.
    state_list.sort_by(|a, b| a.id.cmp(&b.id));

    // 3. Serialize to JSON without whitespace (separators=(',', ':') in Python)
    let state_str = match serde_json::to_string(&state_list) {
        Ok(s) => s,
        Err(e) => return Err(Error::Serialization(e)),
    };

    // 4. Compute SHA-256
    let mut hasher = Sha256::new();
    hasher.update(state_str.as_bytes());
    let result = hasher.finalize();

    // 5. Hex encode
    Ok(format!("{result:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::{TicketType, TicketStatus, TicketPayload};

    #[test]
    fn test_reconcile_create() {
        let base_tickets = vec![];
        let pending = vec![SyncAction {
            action_id: "a1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "t1".to_string(),
            entity_type: "TASK".to_string(),
            status: Some(TicketStatus::Idle),
            payload: Some(TicketPayload::Task {
                title: "Test".to_string(),
                scheduled_time_iso: None,
                rrule: None,
                duration_minutes: None,
                status: None,
                priority: None,
                completed: Some(false),
            }),
            notes: None,
            timestamp: "now".to_string(),
        }];

        let reconciled = reconcile_state(&base_tickets, pending);
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].entity_id, "t1");
    }

    #[test]
    fn test_reconcile_already_created() {
        let payload = TicketPayload::Task {
            title: "Test".to_string(),
            scheduled_time_iso: None,
            rrule: None,
            duration_minutes: None,
            status: None,
            priority: None,
            completed: Some(false),
        };
        let base_tickets = vec![Ticket {
            id: "t1".to_string(),
            title: "Test".to_string(),
            r#type: TicketType::Task,
            status: TicketStatus::Idle,
            payload: payload.clone(),
            notes: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        let pending = vec![SyncAction {
            action_id: "a1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "t1".to_string(),
            entity_type: "TASK".to_string(),
            status: Some(TicketStatus::Idle),
            payload: Some(payload),
            notes: None,
            timestamp: "now".to_string(),
        }];

        let reconciled = reconcile_state(&base_tickets, pending);
        assert_eq!(reconciled.len(), 0);
    }

    #[test]
    fn test_project_state_merges_partial_generic_update_into_event_payload() {
        let base_tickets = vec![Ticket {
            id: "party-1".to_string(),
            title: "Untitled".to_string(),
            r#type: TicketType::Event,
            status: TicketStatus::Idle,
            payload: TicketPayload::Event {
                title: "Untitled".to_string(),
                scheduled_time_iso: Some("2026-03-21T18:00:00+00:00".to_string()),
                rrule: Some("DTSTART:20260321T180000Z".to_string()),
                duration_minutes: None,
                location: None,
                status: None,
                priority: None,
                completed: Some(false),
            },
            notes: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        let actions = vec![SyncAction {
            action_id: "a2".to_string(),
            r#type: SyncActionType::Update,
            entity_id: "party-1".to_string(),
            entity_type: "EVENT".to_string(),
            status: None,
            payload: Some(TicketPayload::Generic(serde_json::json!({
                "title": "Jimbo birthday party"
            }))),
            notes: None,
            timestamp: "now".to_string(),
        }];

        let projected = project_state(base_tickets, &actions);
        assert_eq!(projected.len(), 1);

        match &projected[0].payload {
            TicketPayload::Event { title, scheduled_time_iso, rrule, status, priority, .. } => {
                assert_eq!(title, "Jimbo birthday party");
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-21T18:00:00+00:00"));
                assert_eq!(rrule.as_deref(), Some("DTSTART:20260321T180000Z"));
                assert_eq!(status, &None);
                assert_eq!(priority, &None);
            }
            other => panic!("expected event payload, got {other:?}"),
        }
    }

    #[test]
    fn test_reconcile_partial_generic_update_when_base_ticket_already_matches() {
        let base_tickets = vec![Ticket {
            id: "party-1".to_string(),
            title: "Jimbo birthday party".to_string(),
            r#type: TicketType::Event,
            status: TicketStatus::Idle,
            payload: TicketPayload::Event {
                title: "Jimbo birthday party".to_string(),
                scheduled_time_iso: Some("2026-03-21T18:00:00+00:00".to_string()),
                rrule: Some("DTSTART:20260321T180000Z".to_string()),
                duration_minutes: None,
                location: None,
                status: None,
                priority: None,
                completed: Some(false),
            },
            notes: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        let pending = vec![SyncAction {
            action_id: "a3".to_string(),
            r#type: SyncActionType::Update,
            entity_id: "party-1".to_string(),
            entity_type: "EVENT".to_string(),
            status: None,
            payload: Some(TicketPayload::Generic(serde_json::json!({
                "rrule": "DTSTART:20260321T180000Z"
            }))),
            notes: None,
            timestamp: "now".to_string(),
        }];

        let reconciled = reconcile_state(&base_tickets, pending);
        assert!(reconciled.is_empty());
    }

    #[test]
    fn test_deserialized_create_and_update_actions_preserve_scheduled_event_fields() {
        let create_action: SyncAction = match serde_json::from_value(serde_json::json!({
            "action_id": "create-yoga",
            "type": "CREATE",
            "entity_id": "yoga-1",
            "entity_type": "EVENT",
            "payload": {
                "title": "Yoga",
                "duration_minutes": 60
            },
            "status": "idle",
            "timestamp": "2026-03-20T22:26:42.489738+00:00"
        })) {
            Ok(action) => action,
            Err(error) => panic!("create action should deserialize: {error}"),
        };

        let update_action: SyncAction = match serde_json::from_value(serde_json::json!({
            "action_id": "update-yoga",
            "type": "UPDATE",
            "entity_id": "yoga-1",
            "entity_type": "EVENT",
            "payload": {
                "rrule": null,
                "scheduled_time_iso": "2026-03-26T09:00:00+00:00"
            },
            "timestamp": "2026-03-20T22:31:07.811231+00:00"
        })) {
            Ok(action) => action,
            Err(error) => panic!("update action should deserialize: {error}"),
        };

        let projected = project_state(vec![], &[create_action, update_action]);
        assert_eq!(projected.len(), 1);

        let ticket = &projected[0];
        assert_eq!(ticket.title, "Yoga");
        assert_eq!(ticket.r#type, TicketType::Event);

        match &ticket.payload {
            TicketPayload::Event {
                title,
                scheduled_time_iso,
                rrule,
                duration_minutes,
                location,
                status,
                priority,
                completed,
            } => {
                assert_eq!(title, "Yoga");
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-26T09:00:00+00:00"));
                assert_eq!(rrule, &None);
                assert_eq!(duration_minutes, &Some(60));
                assert_eq!(location, &None);
                assert_eq!(status, &None);
                assert_eq!(priority, &None);
                assert_eq!(completed, &None);
            }
            other => panic!("expected event payload, got {other:?}"),
        }
    }
}
