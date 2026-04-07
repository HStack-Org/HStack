use hstack_core::location_utils as shared;
use hstack_core::settings::{SavedLocation, UserSettings};
use hstack_core::ticket::{CommuteDepartureTime, Ticket, TicketLocation, TicketPayload};
use serde_json::Value;

#[allow(dead_code)]
pub(crate) const DEFAULT_COMMUTE_BUFFER_MINUTES: i64 = shared::DEFAULT_COMMUTE_BUFFER_MINUTES;

#[allow(dead_code)]
pub(crate) fn parse_optional_deserialized_arg<T>(args: &Value, key: &str, label: &str) -> Result<Option<T>, String>
where
    T: for<'de> serde::Deserialize<'de>,
{
    shared::parse_optional_deserialized_arg(args, key, label)
}

#[allow(dead_code)]
pub(crate) fn parse_departure_time_arg(
    args: &Value,
    key: &str,
    label: &str,
) -> Result<Option<CommuteDepartureTime>, String> {
    shared::parse_departure_time_arg(args, key, label)
}

#[allow(dead_code)]
pub(crate) fn format_saved_locations_for_prompt(saved_locations: &[SavedLocation]) -> String {
    shared::format_saved_locations_for_prompt(saved_locations)
}

#[allow(dead_code)]
pub(crate) fn resolve_event_location(
    args: &Value,
    settings: &UserSettings,
) -> Result<Option<TicketLocation>, String> {
    shared::resolve_event_location(args, settings)
}

#[allow(dead_code)]
pub(crate) fn resolve_commute_location(
    args: &Value,
    object_key: &str,
    text_key: &str,
    label: &str,
    settings: &UserSettings,
) -> Result<(String, TicketLocation), String> {
    shared::resolve_commute_location(args, object_key, text_key, label, settings)
}

#[allow(dead_code)]
pub(crate) fn infer_commute_payload_from_event(
    event_id: &str,
    payload: &TicketPayload,
) -> Option<TicketPayload> {
    shared::infer_commute_payload_from_event(event_id, payload)
}

#[allow(dead_code)]
pub(crate) fn normalize_legacy_commute_payload(payload: &mut TicketPayload) {
    shared::normalize_legacy_commute_payload(payload)
}

pub(crate) fn normalize_projected_tickets(tickets: Vec<Ticket>) -> Vec<Ticket> {
    shared::normalize_projected_tickets(tickets)
}

#[allow(dead_code)]
pub(crate) fn find_related_commute_id(tickets: &[Ticket], event_id: &str) -> Option<String> {
    shared::find_related_commute_id(tickets, event_id)
}
