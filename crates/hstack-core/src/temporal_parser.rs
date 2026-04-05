use chrono::{DateTime, FixedOffset, Local, LocalResult, NaiveDateTime, TimeZone};
use rrule::RRuleSet;
use std::str::FromStr;

/// Parses an agent-generated RRULE/DTSTART string into an absolute DateTime and normalized RRULE.
/// Expects standard iCal format, e.g., "DTSTART:20260319T090000\nRRULE:FREQ=WEEKLY;BYDAY=MO"
pub fn parse_agent_rrule(rrule_input: &str) -> Result<(DateTime<FixedOffset>, Option<String>), String> {
    let normalized = normalize_rrule_input(rrule_input);

    let dtstart_value = normalized
        .lines()
        .find_map(|line| line.strip_prefix("DTSTART:"))
        .map(str::trim)
        .ok_or_else(|| format!("Agent generated invalid RFC 5545 string: missing DTSTART (Input: {rrule_input})"))?;

    let start_time = parse_dtstart_value(dtstart_value)
        .map_err(|e| format!("Agent generated invalid RFC 5545 string: {e} (Input: {rrule_input})"))?;

    if !normalized.contains("RRULE:") {
        return Ok((start_time, None));
    }

    RRuleSet::from_str(&normalized)
        .map_err(|e| format!("Agent generated invalid RFC 5545 string: {e} (Input: {rrule_input})"))?;

    Ok((start_time, Some(normalized)))
}

fn normalize_rrule_input(rrule_input: &str) -> String {
    let mut normalized = rrule_input.trim().replace(" RRULE:", "\nRRULE:");

    if let Some(start_idx) = normalized.find("DTSTART:") {
        let content_start = start_idx + 8;
        let content_end = normalized[content_start..]
            .find(|c: char| c.is_whitespace() || c == '\n')
            .map(|end_idx| content_start + end_idx)
            .unwrap_or(normalized.len());

        if normalized[content_start..content_end].ends_with('Z') {
            normalized.remove(content_end - 1);
        }
    }

    normalized
}

fn parse_dtstart_value(value: &str) -> Result<DateTime<FixedOffset>, String> {
    let trimmed = value.trim_end_matches('Z');
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(trimmed, "%Y%m%dT%H%M"))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(trimmed, "%Y%m%d")
                .map(|d| d.and_hms_opt(10, 0, 0).expect("valid hms"))
        })
        .map_err(|e| format!("invalid DTSTART value: {e}"))?;

    match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.fixed_offset()),
        LocalResult::Ambiguous(first, _) => Ok(first.fixed_offset()),
        LocalResult::None => Err("invalid local DTSTART value".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_agent_rrule;
    use chrono::{Datelike, FixedOffset, Timelike};

    fn must_parse_schedule(input: &str) -> (chrono::DateTime<FixedOffset>, Option<String>) {
        match parse_agent_rrule(input) {
            Ok(parsed) => parsed,
            Err(error) => panic!("schedule should parse in test: {error}"),
        }
    }

    #[test]
    fn parses_dtstart_only_as_one_time_schedule() {
        let (start, rrule) = must_parse_schedule("DTSTART:20260326T100000");

        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 3);
        assert_eq!(start.day(), 26);
        assert_eq!(start.hour(), 10);
        assert_eq!(start.minute(), 0);
        assert_eq!(start.second(), 0);
        assert_eq!(rrule, None);
    }

    #[test]
    fn preserves_local_wall_time_even_when_llm_includes_z() {
        let (start, rrule) = must_parse_schedule("DTSTART:20260326T100000Z");

        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 3);
        assert_eq!(start.day(), 26);
        assert_eq!(start.hour(), 10);
        assert_eq!(start.minute(), 0);
        assert_eq!(start.second(), 0);
        assert_eq!(rrule, None);
    }

    #[test]
    fn preserves_recurrence_when_rrule_is_present() {
        let (start, rrule) = must_parse_schedule("DTSTART:20260324T083000 RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR");

        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 3);
        assert_eq!(start.day(), 24);
        assert_eq!(start.hour(), 8);
        assert_eq!(start.minute(), 30);
        assert_eq!(start.second(), 0);
        let normalized = match rrule {
            Some(value) => value,
            None => panic!("rrule should be preserved for recurring schedules"),
        };
        assert!(normalized.starts_with("DTSTART:20260324T083000\nRRULE:FREQ=WEEKLY"));
        assert!(normalized.contains("BYDAY=MO,WE,FR"));
    }
}