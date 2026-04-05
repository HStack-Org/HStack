use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[path = "ticket_tool_schemas.rs"]
mod tool_schemas;

pub use tool_schemas::tool_schemas;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TicketType {
    Task,
    Habit,
    Event,
    Commute,
    Countdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Idle,
    InFocus,
    Completed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TicketPriority {
    Low,
    Medium,
    High,
    Urgent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskWorkflowStatus {
    Backlog,
    Todo,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventAttendanceStatus {
    Mandatory,
    Optional,
    NiceToHave,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HabitWorkflowStatus {
    Active,
    Paused,
    Optional,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedTicketSchedule {
    pub scheduled_time_iso: Option<String>,
    pub rrule: Option<String>,
    pub duration_minutes: Option<i64>,
}

impl SharedTicketSchedule {
    pub fn is_scheduled(&self) -> bool {
        self.scheduled_time_iso.is_some() || self.rrule.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "location_type", rename_all = "snake_case")]
pub enum TicketLocation {
    SavedLocation {
        location_id: String,
        label: Option<String>,
    },
    Coordinates {
        latitude: f64,
        longitude: f64,
        label: Option<String>,
    },
    AddressText {
        address: String,
        label: Option<String>,
    },
    PlaceId {
        place_id: String,
        provider: Option<String>,
        label: Option<String>,
    },
    CurrentPosition {
        label: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "departure_type", rename_all = "snake_case")]
pub enum CommuteDepartureTime {
    RelativeToArrival {
        buffer_minutes: i64,
    },
    Fixed {
        departure_time_iso: Option<String>,
        departure_rrule: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum TicketPayload {
    Commute {
        title: String,
        label: Option<String>,
        origin: String,
        origin_location: Option<TicketLocation>,
        destination: String,
        destination_location: Option<TicketLocation>,
        departure_time: Option<CommuteDepartureTime>,
        scheduled_time_iso: Option<String>,
        rrule: Option<String>,
        deadline: Option<String>,
        days: Option<String>,
        related_event_id: Option<String>,
        live: Option<bool>,
        minutes_remaining: Option<i64>,
        directions: Option<Value>,
        priority: Option<TicketPriority>,
        completed: Option<bool>,
    },
    Countdown {
        title: String,
        duration_minutes: i64,
        expires_at: Option<String>,
        priority: Option<TicketPriority>,
    },
    Event {
        title: String,
        scheduled_time_iso: Option<String>,
        rrule: Option<String>,
        duration_minutes: Option<i64>,
        location: Option<TicketLocation>,
        status: Option<EventAttendanceStatus>,
        priority: Option<TicketPriority>,
        completed: Option<bool>,
    },
    Habit {
        title: String,
        scheduled_time_iso: Option<String>,
        rrule: Option<String>,
        status: Option<HabitWorkflowStatus>,
        priority: Option<TicketPriority>,
        completed: Option<bool>,
    },
    Task {
        title: String,
        scheduled_time_iso: Option<String>,
        rrule: Option<String>,
        duration_minutes: Option<i64>,
        status: Option<TaskWorkflowStatus>,
        priority: Option<TicketPriority>,
        completed: Option<bool>,
    },
    Generic(Value), // Fallback for unknown payloads during migration
}

impl<'de> Deserialize<'de> for TicketPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(TicketPayload::Generic(Value::deserialize(deserializer)?))
    }
}

pub fn decode_ticket_payload_for_type(type_: &TicketType, value: Value) -> Result<TicketPayload, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "ticket payload must be a JSON object".to_string())?;

    match type_ {
        TicketType::Commute => Ok(TicketPayload::Commute {
            title: object.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            label: object.get("label").and_then(Value::as_str).map(str::to_string),
            origin: object.get("origin").and_then(Value::as_str).ok_or_else(|| "commute payload missing origin".to_string())?.to_string(),
            origin_location: parse_optional_json_field(object, "origin_location")?,
            destination: object.get("destination").and_then(Value::as_str).ok_or_else(|| "commute payload missing destination".to_string())?.to_string(),
            destination_location: parse_optional_json_field(object, "destination_location")?,
            departure_time: parse_optional_json_field(object, "departure_time")?,
            scheduled_time_iso: object.get("scheduled_time_iso").and_then(Value::as_str).map(str::to_string),
            rrule: object.get("rrule").and_then(Value::as_str).map(str::to_string),
            deadline: object.get("deadline").and_then(Value::as_str).map(str::to_string),
            days: object.get("days").and_then(Value::as_str).map(str::to_string),
            related_event_id: object.get("related_event_id").and_then(Value::as_str).map(str::to_string),
            live: object.get("live").and_then(Value::as_bool),
            minutes_remaining: object.get("minutes_remaining").and_then(Value::as_i64),
            directions: object.get("directions").cloned().filter(|item| !item.is_null()),
            priority: parse_optional_json_field(object, "priority")?,
            completed: object.get("completed").and_then(Value::as_bool),
        }),
        TicketType::Countdown => Ok(TicketPayload::Countdown {
            title: object.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            duration_minutes: object.get("duration_minutes").and_then(Value::as_i64).ok_or_else(|| "countdown payload missing duration_minutes".to_string())?,
            expires_at: object.get("expires_at").and_then(Value::as_str).map(str::to_string),
            priority: parse_optional_json_field(object, "priority")?,
        }),
        TicketType::Event => Ok(TicketPayload::Event {
            title: object.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            scheduled_time_iso: object.get("scheduled_time_iso").and_then(Value::as_str).map(str::to_string),
            rrule: object.get("rrule").and_then(Value::as_str).map(str::to_string),
            duration_minutes: object.get("duration_minutes").and_then(Value::as_i64),
            location: parse_optional_json_field(object, "location")?,
            status: parse_optional_json_field(object, "status")?,
            priority: parse_optional_json_field(object, "priority")?,
            completed: object.get("completed").and_then(Value::as_bool),
        }),
        TicketType::Habit => Ok(TicketPayload::Habit {
            title: object.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            scheduled_time_iso: object.get("scheduled_time_iso").and_then(Value::as_str).map(str::to_string),
            rrule: object.get("rrule").and_then(Value::as_str).map(str::to_string),
            status: parse_optional_json_field(object, "status")?,
            priority: parse_optional_json_field(object, "priority")?,
            completed: object.get("completed").and_then(Value::as_bool),
        }),
        TicketType::Task => Ok(TicketPayload::Task {
            title: object.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            scheduled_time_iso: object.get("scheduled_time_iso").and_then(Value::as_str).map(str::to_string),
            rrule: object.get("rrule").and_then(Value::as_str).map(str::to_string),
            duration_minutes: object.get("duration_minutes").and_then(Value::as_i64),
            status: parse_optional_json_field(object, "status")?,
            priority: parse_optional_json_field(object, "priority")?,
            completed: object.get("completed").and_then(Value::as_bool),
        }),
    }
}

fn parse_optional_json_field<T>(object: &Map<String, Value>, key: &str) -> Result<Option<T>, String>
where
    T: for<'de> Deserialize<'de>,
{
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| format!("invalid {key} value")),
    }
}

#[derive(Deserialize)]
struct RawTicket {
    id: String,
    #[serde(rename = "type")]
    r#type: TicketType,
    status: TicketStatus,
    payload: Value,
    notes: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    title: String,
}

impl TicketPayload {
    pub fn shared_schedule(&self) -> Option<SharedTicketSchedule> {
        match self {
            TicketPayload::Event {
                scheduled_time_iso,
                rrule,
                duration_minutes,
                ..
            }
            | TicketPayload::Task {
                scheduled_time_iso,
                rrule,
                duration_minutes,
                ..
            } => Some(SharedTicketSchedule {
                scheduled_time_iso: scheduled_time_iso.clone(),
                rrule: rrule.clone(),
                duration_minutes: *duration_minutes,
            }),
            TicketPayload::Commute {
                departure_time,
                scheduled_time_iso,
                rrule,
                ..
            } => Some(SharedTicketSchedule {
                scheduled_time_iso: match departure_time {
                    Some(CommuteDepartureTime::Fixed {
                        departure_time_iso,
                        ..
                    }) => departure_time_iso.clone().or_else(|| scheduled_time_iso.clone()),
                    _ => scheduled_time_iso.clone(),
                },
                rrule: match departure_time {
                    Some(CommuteDepartureTime::Fixed { departure_rrule, .. }) => {
                        departure_rrule.clone().or_else(|| rrule.clone())
                    }
                    _ => rrule.clone(),
                },
                duration_minutes: None,
            }),
            TicketPayload::Habit {
                scheduled_time_iso,
                rrule,
                ..
            } => Some(SharedTicketSchedule {
                scheduled_time_iso: scheduled_time_iso.clone(),
                rrule: rrule.clone(),
                duration_minutes: None,
            }),
            _ => None,
        }
    }

    pub fn set_shared_schedule(&mut self, schedule: SharedTicketSchedule) -> bool {
        match self {
            TicketPayload::Event {
                scheduled_time_iso,
                rrule,
                duration_minutes,
                ..
            }
            | TicketPayload::Task {
                scheduled_time_iso,
                rrule,
                duration_minutes,
                ..
            } => {
                *scheduled_time_iso = schedule.scheduled_time_iso;
                *rrule = schedule.rrule;
                *duration_minutes = schedule.duration_minutes;
                true
            }
            TicketPayload::Commute {
                departure_time,
                scheduled_time_iso,
                rrule,
                ..
            } => {
                *scheduled_time_iso = schedule.scheduled_time_iso;
                *rrule = schedule.rrule;
                if let Some(CommuteDepartureTime::Fixed {
                    departure_time_iso,
                    departure_rrule,
                }) = departure_time
                {
                    *departure_time_iso = scheduled_time_iso.clone();
                    *departure_rrule = rrule.clone();
                }
                true
            }
            TicketPayload::Habit {
                scheduled_time_iso,
                rrule,
                ..
            } => {
                *scheduled_time_iso = schedule.scheduled_time_iso;
                *rrule = schedule.rrule;
                true
            }
            _ => false,
        }
    }

    pub fn get_title(&self) -> &str {
        match self {
            TicketPayload::Commute { title, .. } => title,
            TicketPayload::Countdown { title, .. } => title,
            TicketPayload::Event { title, .. } => title,
            TicketPayload::Habit { title, .. } => title,
            TicketPayload::Task { title, .. } => title,
            TicketPayload::Generic(v) => v.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled"),
        }
    }

    pub fn set_title(&mut self, new_title: String) {
        match self {
            TicketPayload::Commute { title, .. } => *title = new_title,
            TicketPayload::Countdown { title, .. } => *title = new_title,
            TicketPayload::Event { title, .. } => *title = new_title,
            TicketPayload::Habit { title, .. } => *title = new_title,
            TicketPayload::Task { title, .. } => *title = new_title,
            TicketPayload::Generic(v) => {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("title".to_string(), serde_json::Value::String(new_title));
                }
            }
        }
    }

    pub fn apply_partial_update(&mut self, updates: &Map<String, Value>) {
        match self {
            TicketPayload::Commute {
                title,
                label,
                origin,
                origin_location,
                destination,
                destination_location,
                departure_time,
                scheduled_time_iso,
                rrule,
                deadline,
                days,
                related_event_id,
                live,
                minutes_remaining,
                directions,
                priority,
                completed,
            } => {
                apply_string_field(updates, "title", title);
                apply_option_string_field(updates, "label", label);
                apply_string_field(updates, "origin", origin);
                apply_option_deserialized_field(updates, "origin_location", origin_location);
                apply_string_field(updates, "destination", destination);
                apply_option_deserialized_field(updates, "destination_location", destination_location);
                apply_option_deserialized_field(updates, "departure_time", departure_time);
                apply_option_string_field(updates, "scheduled_time_iso", scheduled_time_iso);
                apply_option_string_field(updates, "rrule", rrule);
                apply_option_string_field(updates, "deadline", deadline);
                apply_option_string_field(updates, "days", days);
                apply_option_string_field(updates, "related_event_id", related_event_id);
                apply_option_bool_field(updates, "live", live);
                apply_option_i64_field(updates, "minutes_remaining", minutes_remaining);
                apply_option_value_field(updates, "directions", directions);
                apply_option_deserialized_field(updates, "priority", priority);
                apply_option_bool_field(updates, "completed", completed);
            }
            TicketPayload::Countdown {
                title,
                duration_minutes,
                expires_at,
                priority,
            } => {
                apply_string_field(updates, "title", title);
                apply_i64_field(updates, "duration_minutes", duration_minutes);
                apply_option_string_field(updates, "expires_at", expires_at);
                apply_option_deserialized_field(updates, "priority", priority);
            }
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
                apply_string_field(updates, "title", title);
                apply_option_string_field(updates, "scheduled_time_iso", scheduled_time_iso);
                apply_option_string_field(updates, "rrule", rrule);
                apply_option_i64_field(updates, "duration_minutes", duration_minutes);
                apply_option_deserialized_field(updates, "location", location);
                apply_option_deserialized_field(updates, "status", status);
                apply_option_deserialized_field(updates, "priority", priority);
                apply_option_bool_field(updates, "completed", completed);
            }
            TicketPayload::Habit {
                title,
                scheduled_time_iso,
                rrule,
                status,
                priority,
                completed,
            } => {
                apply_string_field(updates, "title", title);
                apply_option_string_field(updates, "scheduled_time_iso", scheduled_time_iso);
                apply_option_string_field(updates, "rrule", rrule);
                apply_option_deserialized_field(updates, "status", status);
                apply_option_deserialized_field(updates, "priority", priority);
                apply_option_bool_field(updates, "completed", completed);
            }
            TicketPayload::Task {
                title,
                scheduled_time_iso,
                rrule,
                duration_minutes,
                status,
                priority,
                completed,
            } => {
                apply_string_field(updates, "title", title);
                apply_option_string_field(updates, "scheduled_time_iso", scheduled_time_iso);
                apply_option_string_field(updates, "rrule", rrule);
                apply_option_i64_field(updates, "duration_minutes", duration_minutes);
                apply_option_deserialized_field(updates, "status", status);
                apply_option_deserialized_field(updates, "priority", priority);
                apply_option_bool_field(updates, "completed", completed);
            }
            TicketPayload::Generic(value) => {
                if let Some(object) = value.as_object_mut() {
                    for (key, field_value) in updates {
                        object.insert(key.clone(), field_value.clone());
                    }
                }
            }
        }
    }

    pub fn matches_partial_update(&self, updates: &Map<String, Value>) -> bool {
        let mut projected = self.clone();
        projected.apply_partial_update(updates);
        &projected == self
    }
}

fn apply_string_field(updates: &Map<String, Value>, key: &str, target: &mut String) {
    if let Some(value) = updates.get(key).and_then(Value::as_str) {
        *target = value.to_string();
    }
}

fn apply_option_string_field(updates: &Map<String, Value>, key: &str, target: &mut Option<String>) {
    if let Some(value) = updates.get(key) {
        *target = value.as_str().map(|item| item.to_string());
    }
}

fn apply_i64_field(updates: &Map<String, Value>, key: &str, target: &mut i64) {
    if let Some(value) = updates.get(key).and_then(Value::as_i64) {
        *target = value;
    }
}

fn apply_option_i64_field(updates: &Map<String, Value>, key: &str, target: &mut Option<i64>) {
    if let Some(value) = updates.get(key) {
        *target = value.as_i64();
    }
}

fn apply_option_bool_field(updates: &Map<String, Value>, key: &str, target: &mut Option<bool>) {
    if let Some(value) = updates.get(key) {
        *target = value.as_bool();
    }
}

fn apply_option_value_field(updates: &Map<String, Value>, key: &str, target: &mut Option<Value>) {
    if let Some(value) = updates.get(key) {
        *target = if value.is_null() { None } else { Some(value.clone()) };
    }
}

fn apply_option_deserialized_field<T>(updates: &Map<String, Value>, key: &str, target: &mut Option<T>)
where
    T: for<'de> Deserialize<'de>,
{
    if let Some(value) = updates.get(key) {
        *target = if value.is_null() {
            None
        } else {
            serde_json::from_value(value.clone()).ok()
        };
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub r#type: TicketType,
    pub status: TicketStatus,
    pub payload: TicketPayload,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl<'de> Deserialize<'de> for Ticket {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawTicket::deserialize(deserializer)?;
        let payload = decode_ticket_payload_for_type(&raw.r#type, raw.payload)
            .map_err(de::Error::custom)?;

        Ok(Ticket {
            id: raw.id,
            title: raw.title,
            r#type: raw.r#type,
            status: raw.status,
            payload,
            notes: raw.notes,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        })
    }
}

impl Ticket {
    pub fn new(title: String, type_: TicketType, payload: TicketPayload, notes: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            r#type: type_,
            status: TicketStatus::Idle,
            payload,
            notes,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::{
        CommuteDepartureTime,
        decode_ticket_payload_for_type,
        EventAttendanceStatus,
        HabitWorkflowStatus,
        SharedTicketSchedule,
        TicketLocation,
        TaskWorkflowStatus,
        Ticket,
        TicketPayload,
        TicketPriority,
        TicketStatus,
        TicketType,
    };

    fn must_decode_payload(ticket_type: &TicketType, value: Value) -> TicketPayload {
        match decode_ticket_payload_for_type(ticket_type, value) {
            Ok(payload) => payload,
            Err(error) => panic!("payload should decode in test: {error}"),
        }
    }

    fn must_object(value: &Value) -> &Map<String, Value> {
        match value.as_object() {
            Some(object) => object,
            None => panic!("updates should be an object"),
        }
    }

    fn must_shared_schedule(payload: &TicketPayload) -> SharedTicketSchedule {
        match payload.shared_schedule() {
            Some(schedule) => schedule,
            None => panic!("ticket should expose a shared schedule"),
        }
    }

    fn must_deserialize_ticket(value: Value) -> Ticket {
        match serde_json::from_value(value) {
            Ok(ticket) => ticket,
            Err(error) => panic!("ticket should deserialize in test: {error}"),
        }
    }

    #[test]
    fn decodes_event_payload_from_explicit_ticket_type() {
        let payload = must_decode_payload(&TicketType::Event, serde_json::json!({
            "title": "Yoga",
            "duration_minutes": 60,
            "location": {
                "location_type": "address_text",
                "address": "42 Rue Oberkampf, Paris",
                "label": "Studio"
            },
            "status": "mandatory",
            "priority": "high"
        }));

        match payload {
            TicketPayload::Event { title, duration_minutes, scheduled_time_iso, rrule, location, status, priority, completed } => {
                assert_eq!(title, "Yoga");
                assert_eq!(duration_minutes, Some(60));
                assert_eq!(scheduled_time_iso, None);
                assert_eq!(rrule, None);
                assert_eq!(location, Some(TicketLocation::AddressText {
                    address: "42 Rue Oberkampf, Paris".to_string(),
                    label: Some("Studio".to_string()),
                }));
                assert_eq!(status, Some(EventAttendanceStatus::Mandatory));
                assert_eq!(priority, Some(TicketPriority::High));
                assert_eq!(completed, None);
            }
            other => panic!("expected event payload, got {other:?}"),
        }
    }

    #[test]
    fn decodes_countdown_payload_from_explicit_ticket_type() {
        let payload = must_decode_payload(&TicketType::Countdown, serde_json::json!({
            "title": "Refactor auth",
            "duration_minutes": 45,
            "expires_at": "2026-03-20T22:00:00+00:00"
        }));

        match payload {
            TicketPayload::Countdown {
                title,
                duration_minutes,
                expires_at,
                priority,
            } => {
                assert_eq!(title, "Refactor auth");
                assert_eq!(duration_minutes, 45);
                assert_eq!(expires_at.as_deref(), Some("2026-03-20T22:00:00+00:00"));
                assert_eq!(priority, None);
            }
            other => panic!("expected countdown payload, got {other:?}"),
        }
    }

    #[test]
    fn applies_partial_update_to_task_status_and_priority() {
        let mut payload = TicketPayload::Task {
            title: "Prepare launch notes".to_string(),
            scheduled_time_iso: None,
            rrule: None,
            duration_minutes: Some(30),
            status: None,
            priority: None,
            completed: Some(false),
        };

        let updates = serde_json::json!({
            "status": "in_progress",
            "priority": "urgent"
        });

        payload.apply_partial_update(must_object(&updates));

        match payload {
            TicketPayload::Task { status, priority, .. } => {
                assert_eq!(status, Some(TaskWorkflowStatus::InProgress));
                assert_eq!(priority, Some(TicketPriority::Urgent));
            }
            other => panic!("expected task payload, got {other:?}"),
        }
    }

    #[test]
    fn decodes_commute_locations_when_present() {
        let payload = must_decode_payload(&TicketType::Commute, serde_json::json!({
            "title": "Trip to office",
            "origin": "Current location",
            "origin_location": {
                "location_type": "current_position",
                "label": "Current location"
            },
            "destination": "1 Infinite Loop, Cupertino",
            "destination_location": {
                "location_type": "address_text",
                "address": "1 Infinite Loop, Cupertino",
                "label": "Office"
            }
        }));

        match payload {
            TicketPayload::Commute { origin_location, destination_location, .. } => {
                assert_eq!(origin_location, Some(TicketLocation::CurrentPosition {
                    label: Some("Current location".to_string()),
                }));
                assert_eq!(destination_location, Some(TicketLocation::AddressText {
                    address: "1 Infinite Loop, Cupertino".to_string(),
                    label: Some("Office".to_string()),
                }));
            }
            other => panic!("expected commute payload, got {other:?}"),
        }
    }

    #[test]
    fn decodes_saved_location_references() {
        let payload = must_decode_payload(&TicketType::Event, serde_json::json!({
            "title": "Dinner",
            "location": {
                "location_type": "saved_location",
                "location_id": "loc-home",
                "label": "Home"
            }
        }));

        match payload {
            TicketPayload::Event { location, .. } => {
                assert_eq!(location, Some(TicketLocation::SavedLocation {
                    location_id: "loc-home".to_string(),
                    label: Some("Home".to_string()),
                }));
            }
            other => panic!("expected event payload, got {other:?}"),
        }
    }

    #[test]
    fn decodes_commute_departure_time() {
        let payload = must_decode_payload(&TicketType::Commute, serde_json::json!({
            "title": "Trip to office",
            "origin": "Home",
            "destination": "Office",
            "departure_time": {
                "departure_type": "relative_to_arrival",
                "buffer_minutes": 12
            },
            "scheduled_time_iso": "2026-03-25T09:00:00+00:00"
        }));

        match payload {
            TicketPayload::Commute { departure_time, scheduled_time_iso, .. } => {
                assert_eq!(departure_time, Some(CommuteDepartureTime::RelativeToArrival {
                    buffer_minutes: 12,
                }));
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-25T09:00:00+00:00"));
            }
            other => panic!("expected commute payload, got {other:?}"),
        }
    }

    #[test]
    fn exposes_shared_schedule_for_scheduled_ticket_types() {
        let payload = TicketPayload::Task {
            title: "Prepare launch notes".to_string(),
            scheduled_time_iso: Some("2026-03-25T09:00:00+00:00".to_string()),
            rrule: Some("DTSTART:20260325T090000Z\nRRULE:FREQ=WEEKLY;BYDAY=WE".to_string()),
            duration_minutes: Some(45),
            status: None,
            priority: None,
            completed: Some(false),
        };

        let schedule = must_shared_schedule(&payload);

        assert!(schedule.is_scheduled());
        assert_eq!(schedule.scheduled_time_iso.as_deref(), Some("2026-03-25T09:00:00+00:00"));
        assert_eq!(schedule.duration_minutes, Some(45));
    }

    #[test]
    fn applies_shared_schedule_updates_without_changing_payload_kind() {
        let mut payload = TicketPayload::Habit {
            title: "Morning reading".to_string(),
            scheduled_time_iso: None,
            rrule: None,
            status: Some(HabitWorkflowStatus::Active),
            priority: Some(TicketPriority::Medium),
            completed: Some(false),
        };

        let changed = payload.set_shared_schedule(SharedTicketSchedule {
            scheduled_time_iso: Some("2026-03-27T07:00:00+00:00".to_string()),
            rrule: Some("DTSTART:20260327T070000Z\nRRULE:FREQ=DAILY".to_string()),
            duration_minutes: Some(15),
        });

        assert!(changed);

        match payload {
            TicketPayload::Habit {
                scheduled_time_iso,
                rrule,
                status,
                priority,
                ..
            } => {
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-27T07:00:00+00:00"));
                assert_eq!(rrule.as_deref(), Some("DTSTART:20260327T070000Z\nRRULE:FREQ=DAILY"));
                assert_eq!(status, Some(HabitWorkflowStatus::Active));
                assert_eq!(priority, Some(TicketPriority::Medium));
            }
            other => panic!("expected habit payload, got {other:?}"),
        }
    }

    #[test]
    fn decodes_habit_status_and_priority_when_present() {
        let payload = must_decode_payload(&TicketType::Habit, serde_json::json!({
            "title": "Morning reading",
            "status": "active",
            "priority": "medium"
        }));

        match payload {
            TicketPayload::Habit { status, priority, .. } => {
                assert_eq!(status, Some(HabitWorkflowStatus::Active));
                assert_eq!(priority, Some(TicketPriority::Medium));
            }
            other => panic!("expected habit payload, got {other:?}"),
        }
    }

    #[test]
    fn deserializes_ticket_using_its_type_discriminator() {
        let ticket: Ticket = must_deserialize_ticket(serde_json::json!({
            "id": "ticket-1",
            "type": "EVENT",
            "status": "idle",
            "payload": {
                "title": "Yoga",
                "scheduled_time_iso": "2026-03-26T09:00:00+00:00",
                "duration_minutes": 60,
                "location": {
                    "location_type": "address_text",
                    "address": "42 Rue Oberkampf, Paris"
                },
                "status": "mandatory",
                "priority": "high",
                "rrule": null,
                "completed": false
            },
            "notes": null,
            "created_at": "2026-03-20T22:00:00+00:00",
            "updated_at": "2026-03-20T22:00:00+00:00",
            "title": "Yoga"
        }));

        assert_eq!(ticket.r#type, TicketType::Event);
        assert_eq!(ticket.status, TicketStatus::Idle);
        match ticket.payload {
            TicketPayload::Event { title, scheduled_time_iso, duration_minutes, location, status, priority, .. } => {
                assert_eq!(title, "Yoga");
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-26T09:00:00+00:00"));
                assert_eq!(duration_minutes, Some(60));
                assert_eq!(location, Some(TicketLocation::AddressText {
                    address: "42 Rue Oberkampf, Paris".to_string(),
                    label: None,
                }));
                assert_eq!(status, Some(EventAttendanceStatus::Mandatory));
                assert_eq!(priority, Some(TicketPriority::High));
            }
            other => panic!("expected event payload, got {other:?}"),
        }
    }
}
