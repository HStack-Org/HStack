use chrono::Utc;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::location_utils::{
    find_related_commute_id, infer_commute_payload_from_event, parse_departure_time_arg,
    parse_optional_deserialized_arg, resolve_commute_location, resolve_event_location,
    DEFAULT_COMMUTE_BUFFER_MINUTES,
};
use crate::settings::UserSettings;
use crate::sync::{SyncAction, SyncActionType};
use crate::temporal_parser::parse_agent_rrule;
use crate::ticket::{
    CommuteDepartureTime, EventAttendanceStatus, HabitWorkflowStatus, TaskWorkflowStatus, Ticket,
    TicketPayload, TicketPriority, TicketStatus,
};

pub fn build_create_ticket_actions(args: &Value, settings: &UserSettings) -> Result<Vec<SyncAction>, String> {
    let ticket_type_str = args
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("TASK")
        .to_uppercase();
    let notes = args.get("notes").and_then(Value::as_str).map(str::to_string);
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_string();
    let duration_minutes = args.get("duration_minutes").and_then(Value::as_i64);
    let priority = parse_optional_deserialized_arg::<TicketPriority>(args, "priority", "priority")?;
    let event_location = resolve_event_location(args, settings)?;

    let mut scheduled_time_iso = None;
    let mut rrule_out = None;
    if let Some(rrule_input) = args.get("rrule").and_then(Value::as_str) {
        let (start_datetime, rrule_str) = parse_agent_rrule(rrule_input)?;
        scheduled_time_iso = Some(start_datetime.to_rfc3339());
        rrule_out = rrule_str;
    }

    let payload = match ticket_type_str.as_str() {
        "HABIT" => TicketPayload::Habit {
            title,
            scheduled_time_iso,
            rrule: rrule_out,
            status: parse_optional_deserialized_arg::<HabitWorkflowStatus>(args, "status", "habit status")?,
            priority,
            completed: Some(false),
        },
        "EVENT" => TicketPayload::Event {
            title,
            scheduled_time_iso,
            rrule: rrule_out,
            duration_minutes,
            location: event_location,
            status: parse_optional_deserialized_arg::<EventAttendanceStatus>(args, "status", "event status")?,
            priority,
            completed: Some(false),
        },
        _ => TicketPayload::Task {
            title,
            scheduled_time_iso,
            rrule: rrule_out,
            duration_minutes,
            status: parse_optional_deserialized_arg::<TaskWorkflowStatus>(args, "status", "task status")?,
            priority,
            completed: Some(false),
        },
    };

    let entity_id = Uuid::new_v4().to_string();
    let mut actions = vec![new_sync_action(
        SyncActionType::Create,
        entity_id.clone(),
        ticket_type_str.clone(),
        Some(payload.clone()),
        Some(TicketStatus::Idle),
        notes,
    )];

    if ticket_type_str == "EVENT" {
        if let Some(commute_payload) = infer_commute_payload_from_event(&entity_id, &payload) {
            actions.push(new_sync_action(
                SyncActionType::Create,
                Uuid::new_v4().to_string(),
                "COMMUTE".to_string(),
                Some(commute_payload),
                Some(TicketStatus::Idle),
                None,
            ));
        }
    }

    Ok(actions)
}

pub fn build_delete_ticket_actions(args: &Value, tickets: &[Ticket]) -> Result<Vec<SyncAction>, String> {
    let ticket_id = args
        .get("ticket_id")
        .and_then(Value::as_str)
        .filter(|ticket_id| !ticket_id.is_empty())
        .ok_or_else(|| "ticket_id missing".to_string())?;

    let actual_entity_type = tickets
        .iter()
        .find(|ticket| ticket.id == ticket_id)
        .map(ticket_entity_type)
        .unwrap_or_else(|| "TASK".to_string());

    Ok(vec![new_sync_action(
        SyncActionType::Delete,
        ticket_id.to_string(),
        actual_entity_type,
        None,
        None,
        None,
    )])
}

pub fn build_delete_all_ticket_actions(tickets: &[Ticket]) -> Vec<SyncAction> {
    tickets
        .iter()
        .map(|ticket| {
            new_sync_action(
                SyncActionType::Delete,
                ticket.id.clone(),
                ticket_entity_type(ticket),
                None,
                None,
                None,
            )
        })
        .collect()
}

pub fn build_edit_ticket_actions(
    args: &Value,
    tickets: &[Ticket],
    settings: &UserSettings,
) -> Result<Vec<SyncAction>, String> {
    let ticket_id = args
        .get("ticket_id")
        .and_then(Value::as_str)
        .filter(|ticket_id| !ticket_id.is_empty())
        .ok_or_else(|| "ticket_id missing".to_string())?;

    let existing_ticket = tickets.iter().find(|ticket| ticket.id == ticket_id);
    let updated_entity_type = if let Some(new_type) = args.get("type").and_then(Value::as_str) {
        new_type.to_uppercase()
    } else {
        existing_ticket
            .map(ticket_entity_type)
            .unwrap_or_else(|| "TASK".to_string())
    };

    let notes = args.get("notes").and_then(Value::as_str).map(str::to_string);
    let mut payload_updates = Map::new();
    if let Some(title) = args.get("title") {
        payload_updates.insert("title".to_string(), title.clone());
    }
    if let Some(duration_minutes) = args.get("duration_minutes") {
        payload_updates.insert("duration_minutes".to_string(), duration_minutes.clone());
    }
    if let Some(rrule_input) = args.get("rrule").and_then(Value::as_str) {
        let (start_datetime, rrule_str) = parse_agent_rrule(rrule_input)?;
        payload_updates.insert(
            "scheduled_time_iso".to_string(),
            serde_json::json!(start_datetime.to_rfc3339()),
        );
        payload_updates.insert(
            "rrule".to_string(),
            match rrule_str {
                Some(rrule) => serde_json::json!(rrule),
                None => Value::Null,
            },
        );
    }
    if args.get("priority").is_some() {
        payload_updates.insert(
            "priority".to_string(),
            serde_json::to_value(parse_optional_deserialized_arg::<TicketPriority>(args, "priority", "priority")?)
                .map_err(|error| format!("failed to serialize priority: {error}"))?,
        );
    }
    if args.get("status").is_some() {
        let status_value = match updated_entity_type.as_str() {
            "TASK" => serde_json::to_value(parse_optional_deserialized_arg::<TaskWorkflowStatus>(args, "status", "task status")?)
                .map_err(|error| format!("failed to serialize task status: {error}"))?,
            "EVENT" => serde_json::to_value(parse_optional_deserialized_arg::<EventAttendanceStatus>(args, "status", "event status")?)
                .map_err(|error| format!("failed to serialize event status: {error}"))?,
            "HABIT" => serde_json::to_value(parse_optional_deserialized_arg::<HabitWorkflowStatus>(args, "status", "habit status")?)
                .map_err(|error| format!("failed to serialize habit status: {error}"))?,
            _ => {
                return Err(
                    "status is currently only supported for TASK, EVENT, and HABIT tickets"
                        .to_string(),
                )
            }
        };
        payload_updates.insert("status".to_string(), status_value);
    }
    if args.get("location").is_some() {
        if updated_entity_type != "EVENT" {
            return Err("location is currently only supported for EVENT tickets".to_string());
        }
        payload_updates.insert(
            "location".to_string(),
            serde_json::to_value(resolve_event_location(args, settings)?)
                .map_err(|error| format!("failed to serialize event location: {error}"))?,
        );
    }
    if args.get("departure_time").is_some() {
        if updated_entity_type != "COMMUTE" {
            return Err("departure_time is currently only supported for COMMUTE tickets".to_string());
        }
        payload_updates.insert(
            "departure_time".to_string(),
            serde_json::to_value(parse_departure_time_arg(args, "departure_time", "departure time")?)
                .map_err(|error| format!("failed to serialize departure time: {error}"))?,
        );
    }

    let mut actions = vec![new_sync_action(
        SyncActionType::Update,
        ticket_id.to_string(),
        updated_entity_type.clone(),
        if payload_updates.is_empty() {
            None
        } else {
            Some(TicketPayload::Generic(Value::Object(payload_updates.clone())))
        },
        None,
        notes,
    )];

    let commute_follow_up = infer_commute_follow_up(ticket_id, &updated_entity_type, existing_ticket, tickets, &payload_updates);
    if let Some(action) = commute_follow_up {
        actions.push(action);
    }

    Ok(actions)
}

pub fn build_add_commute_actions(args: &Value, settings: &UserSettings) -> Result<Vec<SyncAction>, String> {
    let label = args.get("label").and_then(Value::as_str).unwrap_or("commute");
    let deadline = args.get("deadline").and_then(Value::as_str).unwrap_or("09:00");
    let days = args
        .get("days")
        .and_then(Value::as_str)
        .unwrap_or("monday,tuesday,wednesday,thursday,friday");
    let (origin, origin_location) = resolve_commute_location(args, "origin_location", "origin", "origin location", settings)?;
    let (destination, destination_location) = resolve_commute_location(
        args,
        "destination_location",
        "destination",
        "destination location",
        settings,
    )?;
    let departure_time = match parse_departure_time_arg(args, "departure_time", "departure time")? {
        Some(value) => value,
        None => CommuteDepartureTime::RelativeToArrival {
            buffer_minutes: DEFAULT_COMMUTE_BUFFER_MINUTES,
        },
    };

    let payload = TicketPayload::Commute {
        title: format!(
            "{}: {}... -> {}... @ {}",
            label,
            &origin[..std::cmp::min(15, origin.len())],
            &destination[..std::cmp::min(15, destination.len())],
            deadline
        ),
        label: Some(label.to_string()),
        origin,
        origin_location: Some(origin_location),
        destination,
        destination_location: Some(destination_location),
        departure_time: Some(departure_time),
        scheduled_time_iso: None,
        rrule: None,
        deadline: Some(deadline.to_string()),
        days: Some(days.to_string()),
        related_event_id: None,
        live: None,
        minutes_remaining: None,
        directions: None,
        priority: None,
        completed: Some(false),
    };

    Ok(vec![new_sync_action(
        SyncActionType::Create,
        Uuid::new_v4().to_string(),
        "COMMUTE".to_string(),
        Some(payload),
        Some(TicketStatus::Idle),
        None,
    )])
}

pub fn build_get_directions_actions(args: &Value, settings: &UserSettings) -> Result<Vec<SyncAction>, String> {
    let (origin, origin_location) = resolve_commute_location(args, "origin_location", "origin", "origin location", settings)?;
    let (destination, destination_location) = resolve_commute_location(
        args,
        "destination_location",
        "destination",
        "destination location",
        settings,
    )?;
    let payload = TicketPayload::Commute {
        title: format!(
            "Directions: {}... -> {}...",
            &origin[..std::cmp::min(15, origin.len())],
            &destination[..std::cmp::min(15, destination.len())]
        ),
        label: None,
        origin,
        origin_location: Some(origin_location),
        destination,
        destination_location: Some(destination_location),
        departure_time: None,
        scheduled_time_iso: None,
        rrule: None,
        deadline: None,
        days: None,
        related_event_id: None,
        live: None,
        minutes_remaining: None,
        directions: Some(serde_json::json!({
            "steps": [],
            "total_duration": "Enriching via Server...",
            "total_duration_minutes": Value::Null,
            "error": Value::Null,
        })),
        priority: None,
        completed: None,
    };

    Ok(vec![new_sync_action(
        SyncActionType::Create,
        Uuid::new_v4().to_string(),
        "COMMUTE".to_string(),
        Some(payload),
        Some(TicketStatus::InFocus),
        None,
    )])
}

pub fn build_remove_commute_actions(args: &Value, tickets: &[Ticket]) -> Result<Vec<SyncAction>, String> {
    build_delete_ticket_actions(args, tickets)
}

pub fn build_start_live_directions_actions(args: &Value, settings: &UserSettings) -> Result<Vec<SyncAction>, String> {
    let minutes = args
        .get("minutes_until_deadline")
        .and_then(Value::as_i64)
        .unwrap_or(30);
    let (origin, origin_location) = resolve_commute_location(args, "origin_location", "origin", "origin location", settings)?;
    let (destination, destination_location) = resolve_commute_location(
        args,
        "destination_location",
        "destination",
        "destination location",
        settings,
    )?;
    let payload = TicketPayload::Commute {
        title: format!("Trip to {}", &destination[..std::cmp::min(40, destination.len())]),
        label: None,
        origin,
        origin_location: Some(origin_location),
        destination,
        destination_location: Some(destination_location),
        departure_time: None,
        scheduled_time_iso: None,
        rrule: None,
        deadline: None,
        days: None,
        related_event_id: None,
        live: Some(true),
        minutes_remaining: Some(minutes),
        directions: Some(serde_json::json!({
            "steps": [],
            "total_duration": "Enriching via Server...",
            "total_duration_minutes": Value::Null,
            "error": Value::Null,
        })),
        priority: None,
        completed: None,
    };

    Ok(vec![new_sync_action(
        SyncActionType::Create,
        Uuid::new_v4().to_string(),
        "COMMUTE".to_string(),
        Some(payload),
        Some(TicketStatus::InFocus),
        None,
    )])
}

pub fn build_create_countdown_actions(args: &Value) -> Vec<SyncAction> {
    let title = args.get("title").and_then(Value::as_str).unwrap_or("Countdown");
    let duration_minutes = args.get("duration_minutes").and_then(Value::as_i64).unwrap_or(30);
    let payload = TicketPayload::Countdown {
        title: title.to_string(),
        duration_minutes,
        expires_at: Some((Utc::now() + chrono::Duration::minutes(duration_minutes)).to_rfc3339()),
        priority: None,
    };

    vec![new_sync_action(
        SyncActionType::Create,
        Uuid::new_v4().to_string(),
        "COUNTDOWN".to_string(),
        Some(payload),
        Some(TicketStatus::Idle),
        None,
    )]
}

fn infer_commute_follow_up(
    ticket_id: &str,
    updated_entity_type: &str,
    existing_ticket: Option<&Ticket>,
    tickets: &[Ticket],
    payload_updates: &Map<String, Value>,
) -> Option<SyncAction> {
    if updated_entity_type != "EVENT" {
        return None;
    }

    let mut projected_payload = existing_ticket?.payload.clone();
    projected_payload.apply_partial_update(payload_updates);
    let existing_commute_id = find_related_commute_id(tickets, ticket_id);

    match (existing_commute_id, infer_commute_payload_from_event(ticket_id, &projected_payload)) {
        (Some(commute_id), Some(payload)) => Some(new_sync_action(
            SyncActionType::Update,
            commute_id,
            "COMMUTE".to_string(),
            Some(payload),
            None,
            None,
        )),
        (None, Some(payload)) => Some(new_sync_action(
            SyncActionType::Create,
            Uuid::new_v4().to_string(),
            "COMMUTE".to_string(),
            Some(payload),
            Some(TicketStatus::Idle),
            None,
        )),
        (Some(commute_id), None) => Some(new_sync_action(
            SyncActionType::Delete,
            commute_id,
            "COMMUTE".to_string(),
            None,
            None,
            None,
        )),
        (None, None) => None,
    }
}

fn ticket_entity_type(ticket: &Ticket) -> String {
    match ticket.r#type {
        crate::ticket::TicketType::Task => "TASK",
        crate::ticket::TicketType::Habit => "HABIT",
        crate::ticket::TicketType::Event => "EVENT",
        crate::ticket::TicketType::Commute => "COMMUTE",
        crate::ticket::TicketType::Countdown => "COUNTDOWN",
    }
    .to_string()
}

fn new_sync_action(
    action_type: SyncActionType,
    entity_id: String,
    entity_type: String,
    payload: Option<TicketPayload>,
    status: Option<TicketStatus>,
    notes: Option<String>,
) -> SyncAction {
    SyncAction {
        action_id: Uuid::new_v4().to_string(),
        r#type: action_type,
        entity_id,
        entity_type,
        status,
        payload,
        notes,
        timestamp: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_create_ticket_actions, build_delete_all_ticket_actions, build_edit_ticket_actions};
    use crate::settings::{SavedLocation, UserSettings};
    use crate::ticket::{Ticket, TicketLocation, TicketPayload, TicketStatus, TicketType};

    fn settings_with_home() -> UserSettings {
        UserSettings {
            saved_locations: vec![SavedLocation {
                id: "loc-home".to_string(),
                label: "Home".to_string(),
                location: TicketLocation::AddressText {
                    address: "12 Rue de Rivoli, Paris".to_string(),
                    label: None,
                },
            }],
            ..UserSettings::default()
        }
    }

    #[test]
    fn create_event_produces_event_and_inferred_commute_actions() {
        let actions = build_create_ticket_actions(
            &json!({
                "type": "EVENT",
                "title": "Dinner",
                "rrule": "DTSTART:20260320T190000Z",
                "location": {
                    "location_type": "saved_location",
                    "location_id": "loc-home"
                }
            }),
            &settings_with_home(),
        );

        match actions {
            Ok(actions) => assert_eq!(actions.len(), 2),
            Err(error) => panic!("expected create event actions: {error}"),
        }
    }

    #[test]
    fn edit_event_emits_related_commute_update_when_present() {
        let tickets = vec![
            Ticket {
                id: "event-1".to_string(),
                title: "Dinner".to_string(),
                r#type: TicketType::Event,
                status: TicketStatus::Idle,
                payload: TicketPayload::Event {
                    title: "Dinner".to_string(),
                    scheduled_time_iso: Some("2026-03-20T19:00:00+00:00".to_string()),
                    rrule: None,
                    duration_minutes: Some(120),
                    location: Some(TicketLocation::SavedLocation {
                        location_id: "loc-home".to_string(),
                        label: Some("Home".to_string()),
                    }),
                    status: None,
                    priority: None,
                    completed: Some(false),
                },
                notes: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            Ticket {
                id: "commute-1".to_string(),
                title: "Commute to Dinner".to_string(),
                r#type: TicketType::Commute,
                status: TicketStatus::Idle,
                payload: TicketPayload::Commute {
                    title: "Commute to Dinner".to_string(),
                    label: Some("event_commute".to_string()),
                    origin: "Current position".to_string(),
                    origin_location: Some(TicketLocation::CurrentPosition { label: None }),
                    destination: "12 Rue de Rivoli, Paris".to_string(),
                    destination_location: Some(TicketLocation::SavedLocation {
                        location_id: "loc-home".to_string(),
                        label: Some("Home".to_string()),
                    }),
                    departure_time: None,
                    scheduled_time_iso: Some("2026-03-20T19:00:00+00:00".to_string()),
                    rrule: None,
                    deadline: None,
                    days: None,
                    related_event_id: Some("event-1".to_string()),
                    live: None,
                    minutes_remaining: None,
                    directions: None,
                    priority: None,
                    completed: Some(false),
                },
                notes: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        ];

        let actions = build_edit_ticket_actions(
            &json!({
                "ticket_id": "event-1",
                "title": "Later dinner"
            }),
            &tickets,
            &settings_with_home(),
        );

        match actions {
            Ok(actions) => assert_eq!(actions.len(), 2),
            Err(error) => panic!("expected edit actions: {error}"),
        }
    }

    #[test]
    fn delete_all_projects_delete_actions_for_every_ticket() {
        let tickets = vec![
            Ticket::new(
                "One".to_string(),
                TicketType::Task,
                TicketPayload::Task {
                    title: "One".to_string(),
                    scheduled_time_iso: None,
                    rrule: None,
                    duration_minutes: None,
                    status: None,
                    priority: None,
                    completed: Some(false),
                },
                None,
            ),
            Ticket::new(
                "Two".to_string(),
                TicketType::Countdown,
                TicketPayload::Countdown {
                    title: "Two".to_string(),
                    duration_minutes: 10,
                    expires_at: None,
                    priority: None,
                },
                None,
            ),
        ];

        let actions = build_delete_all_ticket_actions(&tickets);
        assert_eq!(actions.len(), 2);
    }
}