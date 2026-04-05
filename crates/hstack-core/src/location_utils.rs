use chrono::Local;
use serde_json::Value;

use crate::settings::{SavedLocation, UserSettings};
use crate::ticket::{CommuteDepartureTime, Ticket, TicketLocation, TicketPayload};

pub const DEFAULT_COMMUTE_BUFFER_MINUTES: i64 = 10;

pub fn parse_optional_deserialized_arg<T>(args: &Value, key: &str, label: &str) -> Result<Option<T>, String>
where
    T: for<'de> serde::Deserialize<'de>,
{
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| format!("invalid {label} value")),
    }
}

fn parse_location_arg(args: &Value, key: &str, label: &str) -> Result<Option<TicketLocation>, String> {
    parse_optional_deserialized_arg::<TicketLocation>(args, key, label)
}

pub fn parse_departure_time_arg(
    args: &Value,
    key: &str,
    label: &str,
) -> Result<Option<CommuteDepartureTime>, String> {
    parse_optional_deserialized_arg::<CommuteDepartureTime>(args, key, label)
}

fn normalize_address_text_location(text: &str, label: &str) -> Result<TicketLocation, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} must not be empty"));
    }

    Ok(TicketLocation::AddressText {
        address: trimmed.to_string(),
        label: None,
    })
}

pub fn location_display_text(location: &TicketLocation) -> String {
    match location {
        TicketLocation::SavedLocation { location_id, label } => {
            label.clone().unwrap_or_else(|| location_id.clone())
        }
        TicketLocation::Coordinates {
            latitude,
            longitude,
            label,
        } => label
            .clone()
            .unwrap_or_else(|| format!("{latitude}, {longitude}")),
        TicketLocation::AddressText { address, .. } => address.clone(),
        TicketLocation::PlaceId {
            label,
            place_id,
            ..
        } => label.clone().unwrap_or_else(|| place_id.clone()),
        TicketLocation::CurrentPosition { label } => {
            label.clone().unwrap_or_else(|| "Current position".to_string())
        }
    }
}

fn normalize_location_key(text: &str) -> String {
    text.trim().to_lowercase()
}

fn find_saved_location_by_id<'a>(settings: &'a UserSettings, location_id: &str) -> Option<&'a SavedLocation> {
    settings
        .saved_locations
        .iter()
        .find(|location| location.id == location_id)
}

fn find_saved_location_by_label<'a>(settings: &'a UserSettings, label: &str) -> Option<&'a SavedLocation> {
    let normalized = normalize_location_key(label);
    settings
        .saved_locations
        .iter()
        .find(|location| normalize_location_key(&location.label) == normalized)
}

fn is_ambiguous_location_text(text: &str) -> bool {
    matches!(
        normalize_location_key(text).as_str(),
        "home"
            | "my home"
            | "house"
            | "my house"
            | "my place"
            | "place"
            | "work"
            | "office"
            | "my office"
            | "gym"
            | "school"
            | "there"
            | "here"
    )
}

fn resolve_saved_location_reference(
    settings: &UserSettings,
    location_id: &str,
    label: Option<String>,
    field_label: &str,
) -> Result<(String, TicketLocation), String> {
    let saved_location = find_saved_location_by_id(settings, location_id)
        .ok_or_else(|| format!("unknown {field_label} location_id '{location_id}'"))?;

    let resolved = match &saved_location.location {
        TicketLocation::SavedLocation { .. } => {
            return Err(format!(
                "saved location '{}' must resolve to a concrete location",
                saved_location.label
            ));
        }
        concrete => location_display_text(concrete),
    };

    Ok((
        resolved,
        TicketLocation::SavedLocation {
            location_id: location_id.to_string(),
            label: label.or_else(|| Some(saved_location.label.clone())),
        },
    ))
}

fn resolve_location_object(
    location: TicketLocation,
    settings: &UserSettings,
    field_label: &str,
) -> Result<(String, TicketLocation), String> {
    match location {
        TicketLocation::SavedLocation { location_id, label } => {
            resolve_saved_location_reference(settings, &location_id, label, field_label)
        }
        other => {
            let rendered = location_display_text(&other);
            if rendered.trim().is_empty() {
                return Err(format!(
                    "{field_label} structured location must render to a non-empty value"
                ));
            }

            Ok((rendered, other))
        }
    }
}

pub fn format_saved_locations_for_prompt(saved_locations: &[SavedLocation]) -> String {
    if saved_locations.is_empty() {
        return "- None".to_string();
    }

    saved_locations
        .iter()
        .map(|saved_location| {
            let rendered = match &saved_location.location {
                TicketLocation::SavedLocation { location_id, .. } => location_id.clone(),
                concrete => location_display_text(concrete),
            };

            format!("- {} | {} | {rendered}", saved_location.id, saved_location.label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn resolve_event_location(
    args: &Value,
    settings: &UserSettings,
) -> Result<Option<TicketLocation>, String> {
    match parse_location_arg(args, "location", "event location")? {
        None => Ok(None),
        Some(location) => {
            resolve_location_object(location, settings, "event location").map(|(_, location)| Some(location))
        }
    }
}

pub fn resolve_commute_location(
    args: &Value,
    object_key: &str,
    text_key: &str,
    label: &str,
    settings: &UserSettings,
) -> Result<(String, TicketLocation), String> {
    let text_value = args.get(text_key).and_then(Value::as_str).map(str::trim);
    let object_value = parse_location_arg(args, object_key, label)?;

    match (text_value, object_value) {
        (Some(text), Some(location)) => {
            if text.is_empty() {
                return Err(format!("{label} text must not be empty"));
            }

            let (rendered, normalized) = resolve_location_object(location, settings, label)?;
            let text_matches_saved_label = matches!(
                &normalized,
                TicketLocation::SavedLocation {
                    label: Some(saved_label),
                    ..
                } if saved_label == text
            );

            if rendered != text && !text_matches_saved_label {
                return Err(format!(
                    "{label} text '{text}' does not match structured location '{rendered}'"
                ));
            }

            Ok((rendered, normalized))
        }
        (Some(text), None) => {
            if find_saved_location_by_label(settings, text).is_some() {
                return Err(format!(
                    "{label} '{text}' matches a saved location; use location_id instead of raw text"
                ));
            }

            if is_ambiguous_location_text(text) {
                return Err(format!(
                    "{label} '{text}' is ambiguous; ask the user which saved place or concrete address they mean"
                ));
            }

            let location = normalize_address_text_location(text, label)?;
            Ok((text.to_string(), location))
        }
        (None, Some(location)) => resolve_location_object(location, settings, label),
        (None, None) => Err(format!("missing {label}")),
    }
}

fn extract_rrule_days(rrule: &str) -> Option<String> {
    let rule_line = rrule.lines().find(|line| line.starts_with("RRULE:"))?;
    let byday = rule_line
        .trim_start_matches("RRULE:")
        .split(';')
        .find_map(|segment| segment.strip_prefix("BYDAY="))?;

    let normalized = byday
        .split(',')
        .filter_map(|token| match token {
            "MO" => Some("monday"),
            "TU" => Some("tuesday"),
            "WE" => Some("wednesday"),
            "TH" => Some("thursday"),
            "FR" => Some("friday"),
            "SA" => Some("saturday"),
            "SU" => Some("sunday"),
            _ => None,
        })
        .collect::<Vec<_>>();

    if normalized.is_empty() {
        None
    } else {
        Some(normalized.join(","))
    }
}

fn deadline_from_scheduled_time(scheduled_time_iso: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(scheduled_time_iso)
        .ok()
        .map(|value| value.with_timezone(&Local).format("%H:%M").to_string())
}

pub fn infer_commute_payload_from_event(
    event_id: &str,
    payload: &TicketPayload,
) -> Option<TicketPayload> {
    let TicketPayload::Event {
        title,
        scheduled_time_iso,
        rrule,
        location,
        ..
    } = payload else {
        return None;
    };

    if scheduled_time_iso.is_none() && rrule.is_none() {
        return None;
    }

    let destination_location = location.clone()?;
    if matches!(destination_location, TicketLocation::CurrentPosition { .. }) {
        return None;
    }

    let destination = location_display_text(&destination_location);
    if destination.trim().is_empty() {
        return None;
    }

    Some(TicketPayload::Commute {
        title: format!("Commute to {title}"),
        label: Some("event_commute".to_string()),
        origin: "Current position".to_string(),
        origin_location: Some(TicketLocation::CurrentPosition {
            label: Some("Current position".to_string()),
        }),
        destination,
        destination_location: Some(destination_location),
        departure_time: Some(CommuteDepartureTime::RelativeToArrival {
            buffer_minutes: DEFAULT_COMMUTE_BUFFER_MINUTES,
        }),
        scheduled_time_iso: scheduled_time_iso.clone(),
        rrule: rrule.clone(),
        deadline: scheduled_time_iso
            .as_deref()
            .and_then(deadline_from_scheduled_time),
        days: rrule.as_deref().and_then(extract_rrule_days),
        related_event_id: Some(event_id.to_string()),
        live: None,
        minutes_remaining: None,
        directions: None,
        priority: None,
        completed: Some(false),
    })
}

pub fn normalize_legacy_commute_payload(payload: &mut TicketPayload) {
    let TicketPayload::Commute {
        departure_time,
        scheduled_time_iso,
        rrule,
        ..
    } = payload else {
        return;
    };

    if departure_time.is_some() {
        return;
    }

    if scheduled_time_iso.is_none() && rrule.is_none() {
        return;
    }

    *departure_time = Some(CommuteDepartureTime::RelativeToArrival {
        buffer_minutes: DEFAULT_COMMUTE_BUFFER_MINUTES,
    });
}

pub fn normalize_projected_tickets(mut tickets: Vec<Ticket>) -> Vec<Ticket> {
    for ticket in &mut tickets {
        normalize_legacy_commute_payload(&mut ticket.payload);
    }
    tickets
}

pub fn find_related_commute_id(tickets: &[Ticket], event_id: &str) -> Option<String> {
    tickets.iter().find_map(|ticket| match &ticket.payload {
        TicketPayload::Commute {
            related_event_id: Some(related_event_id),
            ..
        } if related_event_id == event_id => Some(ticket.id.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        format_saved_locations_for_prompt, infer_commute_payload_from_event,
        normalize_projected_tickets, parse_departure_time_arg, resolve_commute_location,
    };
    use crate::settings::{SavedLocation, UserSettings};
    use crate::ticket::{CommuteDepartureTime, Ticket, TicketLocation, TicketPayload, TicketStatus, TicketType};

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
    fn parses_departure_time_object() {
        let args = json!({
            "departure_time": {
                "departure_type": "relative_to_arrival",
                "buffer_minutes": 15
            }
        });

        let parsed = parse_departure_time_arg(&args, "departure_time", "departure time");
        match parsed {
            Ok(Some(CommuteDepartureTime::RelativeToArrival { buffer_minutes })) => {
                assert_eq!(buffer_minutes, 15);
            }
            Ok(other) => panic!("unexpected parsed departure time: {other:?}"),
            Err(error) => panic!("expected departure time to parse: {error}"),
        }
    }

    #[test]
    fn resolves_text_commute_location_to_address() {
        let args = json!({
            "origin": "221B Baker Street, London"
        });

        let resolved = resolve_commute_location(
            &args,
            "origin_location",
            "origin",
            "origin location",
            &UserSettings::default(),
        );

        match resolved {
            Ok((display, TicketLocation::AddressText { address, .. })) => {
                assert_eq!(display, "221B Baker Street, London");
                assert_eq!(address, "221B Baker Street, London");
            }
            Ok(other) => panic!("unexpected location resolution: {other:?}"),
            Err(error) => panic!("expected address text resolution: {error}"),
        }
    }

    #[test]
    fn rejects_saved_location_label_as_raw_text() {
        let args = json!({
            "origin": "Home"
        });

        let error = match resolve_commute_location(
            &args,
            "origin_location",
            "origin",
            "origin location",
            &settings_with_home(),
        ) {
            Ok(_) => panic!("expected saved label raw text to be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("use location_id instead of raw text"));
    }

    #[test]
    fn formats_saved_locations_for_prompt_output() {
        let rendered = format_saved_locations_for_prompt(&settings_with_home().saved_locations);
        assert!(rendered.contains("loc-home | Home | 12 Rue de Rivoli, Paris"));
    }

    #[test]
    fn infers_commute_from_event_location_and_schedule() {
        let payload = TicketPayload::Event {
            title: "Dinner".to_string(),
            scheduled_time_iso: Some("2026-03-20T19:00:00+00:00".to_string()),
            rrule: None,
            duration_minutes: Some(120),
            location: Some(TicketLocation::AddressText {
                address: "12 Rue de Rivoli, Paris".to_string(),
                label: Some("Home".to_string()),
            }),
            status: None,
            priority: None,
            completed: Some(false),
        };

        let inferred = infer_commute_payload_from_event("event-1", &payload);
        match inferred {
            Some(TicketPayload::Commute { related_event_id, destination, .. }) => {
                assert_eq!(related_event_id.as_deref(), Some("event-1"));
                assert_eq!(destination, "12 Rue de Rivoli, Paris");
            }
            Some(other) => panic!("unexpected inferred payload: {other:?}"),
            None => panic!("expected commute inference"),
        }
    }

    #[test]
    fn normalizes_projected_commutes_missing_departure_time() {
        let tickets = vec![Ticket {
            id: "commute-1".to_string(),
            title: "Trip".to_string(),
            r#type: TicketType::Commute,
            status: TicketStatus::Idle,
            payload: TicketPayload::Commute {
                title: "Trip".to_string(),
                label: None,
                origin: "A".to_string(),
                origin_location: None,
                destination: "B".to_string(),
                destination_location: None,
                departure_time: None,
                scheduled_time_iso: Some("2026-03-20T19:00:00+00:00".to_string()),
                rrule: None,
                deadline: None,
                days: None,
                related_event_id: None,
                live: None,
                minutes_remaining: None,
                directions: None,
                priority: None,
                completed: Some(false),
            },
            notes: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        let normalized = normalize_projected_tickets(tickets);
        match &normalized[0].payload {
            TicketPayload::Commute {
                departure_time: Some(CommuteDepartureTime::RelativeToArrival { buffer_minutes }),
                ..
            } => assert_eq!(*buffer_minutes, 10),
            other => panic!("unexpected normalized payload: {other:?}"),
        }
    }
}