use crate::provider::{Tool, ToolFunction};

pub fn tool_schemas() -> Vec<Tool> {
    vec![
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "create_ticket".to_string(),
                description: "Create a new ticket in the user's stack. Must specify the type of ticket (HABIT, EVENT, or TASK) and the title payload. Any of these ticket types may include an RRULE/DTSTART schedule when the user gives timing information.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "description": "The type of the ticket. MUST be exactly one of: HABIT, EVENT, TASK"
                        },
                        "title": {
                            "type": "string",
                            "description": "The title or description of the ticket"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Optional: Detailed context, research results, or user preferences for this specific ticket. Use Markdown formatting."
                        },
                        "rrule": {
                            "type": "string",
                            "description": "Optional: RFC 5545 scheduling string for any time-bearing ticket type. Use 'DTSTART:YYYYMMDDTHHMMSS' for a one-time scheduled ticket, or 'DTSTART:YYYYMMDDTHHMMSS RRULE:FREQ=WEEKLY;BYDAY=MO' for a recurring ticket. Examples: DTSTART:20260320T090000 (tomorrow 9am), DTSTART:20260324T090000 RRULE:FREQ=WEEKLY;BYDAY=MO (every Monday)"
                        },
                        "duration_minutes": {
                            "type": "integer",
                            "description": "Optional: Estimated duration in minutes."
                        },
                        "location": {
                            "type": "object",
                            "description": "Optional structured EVENT location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "todo", "in_progress", "blocked", "done", "cancelled", "mandatory", "optional", "nice_to_have", "active", "paused", "archived"],
                            "description": "Optional: Ticket-specific status. TASK supports backlog/todo/in_progress/blocked/done/cancelled. EVENT supports mandatory/optional/nice_to_have/cancelled. HABIT supports active/paused/optional/archived."
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["low", "medium", "high", "urgent"],
                            "description": "Optional: Shared priority indicator for the ticket."
                        }
                    },
                    "required": ["type", "title"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "delete_ticket".to_string(),
                description: "Delete a ticket from the user's stack given its ID string.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ticket_id": {
                            "type": "string",
                            "description": "The exact ID of the ticket to delete"
                        }
                    },
                    "required": ["ticket_id"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "delete_all_tickets".to_string(),
                description: "Deletes the entire stack of tickets for the user. Use this when the user wants to 'clear everything' or 'get rid of all tickets'.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "edit_ticket".to_string(),
                description: "Edit an existing ticket in the user's stack. You can change its type, title, notes, duration, or RRULE/DTSTART timing for any scheduled ticket type.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ticket_id": {
                            "type": "string",
                            "description": "The ID of the ticket to edit"
                        },
                        "type": {
                            "type": "string",
                            "description": "The new type (HABIT, EVENT, or TASK). Skip if no change."
                        },
                        "title": {
                            "type": "string",
                            "description": "The new title/description. Skip if no change."
                        },
                        "notes": {
                            "type": "string",
                            "description": "The new detailed notes for this ticket. Skip if no change."
                        },
                        "rrule": {
                            "type": "string",
                            "description": "The new RFC 5545 schedule for this ticket. Skip if no change. Format: 'DTSTART:YYYYMMDDTHHMMSS' for one-time scheduling or 'DTSTART:YYYYMMDDTHHMMSS RRULE:FREQ=WEEKLY;BYDAY=MO' for recurrence. Valid for HABIT, EVENT, and TASK tickets."
                        },
                        "duration_minutes": {
                            "type": "integer",
                            "description": "The new duration. Skip if no change."
                        },
                        "location": {
                            "type": "object",
                            "description": "Optional structured EVENT location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "departure_time": {
                            "type": "object",
                            "description": "Optional COMMUTE departure semantics. Use relative_to_arrival for dynamic departure (arrival minus route duration minus buffer), or fixed for absolute departure scheduling.",
                            "properties": {
                                "departure_type": {
                                    "type": "string",
                                    "enum": ["relative_to_arrival", "fixed"]
                                },
                                "buffer_minutes": { "type": "integer" },
                                "departure_time_iso": { "type": "string" },
                                "departure_rrule": { "type": "string" }
                            }
                        },
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "todo", "in_progress", "blocked", "done", "cancelled", "mandatory", "optional", "nice_to_have", "active", "paused", "archived"],
                            "description": "Optional: The new ticket-specific status."
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["low", "medium", "high", "urgent"],
                            "description": "Optional: The new shared priority indicator."
                        }
                    },
                    "required": ["ticket_id"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "add_commute".to_string(),
                description: "Register a recurring commute for the user. Use this when the user says they regularly travel from one place to another at a specific time (e.g., 'I go from X to Y every morning at 9:30'). This will create a scheduled commute that automatically provides transit directions before the deadline.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "A short label for the commute, e.g. 'morning_commute', 'evening_commute', 'work_commute'"
                        },
                        "origin": {
                            "type": "string",
                            "description": "The full starting address or place name"
                        },
                        "origin_location": {
                            "type": "object",
                            "description": "Optional structured origin location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "destination": {
                            "type": "string",
                            "description": "The full destination address or place name"
                        },
                        "destination_location": {
                            "type": "object",
                            "description": "Optional structured destination location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "deadline": {
                            "type": "string",
                            "description": "The time the user needs to arrive, in HH:MM 24-hour format (e.g. '09:30', '18:00')"
                        },
                        "days": {
                            "type": "string",
                            "description": "Comma-separated days of the week this commute applies, e.g. 'monday,tuesday,wednesday,thursday,friday'. Default is weekdays."
                        },
                        "departure_time": {
                            "type": "object",
                            "description": "Optional COMMUTE departure semantics. Relative commutes depart at arrival minus live route duration minus buffer. Fixed commutes depart at the explicit scheduled time.",
                            "properties": {
                                "departure_type": {
                                    "type": "string",
                                    "enum": ["relative_to_arrival", "fixed"]
                                },
                                "buffer_minutes": { "type": "integer" },
                                "departure_time_iso": { "type": "string" },
                                "departure_rrule": { "type": "string" }
                            }
                        }
                    },
                    "required": ["label", "origin", "destination", "deadline"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "get_directions".to_string(),
                description: "Get real-time transit directions between two places. This creates a persistent COMMUTE ticket in the user's stack that renders expanded (in-focus) with step-by-step instructions.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "origin": {
                            "type": "string",
                            "description": "The starting address or place name"
                        },
                        "origin_location": {
                            "type": "object",
                            "description": "Optional structured origin location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "destination": {
                            "type": "string",
                            "description": "The destination address or place name"
                        },
                        "destination_location": {
                            "type": "object",
                            "description": "Optional structured destination location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        }
                    },
                    "required": ["origin", "destination"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "remove_commute".to_string(),
                description: "Remove/delete a registered commute by its ticket ID.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ticket_id": {
                            "type": "string",
                            "description": "The ID of the commute ticket to remove"
                        }
                    },
                    "required": ["ticket_id"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "start_live_directions".to_string(),
                description: "Start a live directions tracker for an URGENT or ONE-TIME trip with a deadline. This creates a persistent COMMUTE ticket with `live: true` that stays in-focus and updates every 5 minutes until the deadline passes.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "origin": {
                            "type": "string",
                            "description": "The user's current location / starting address"
                        },
                        "origin_location": {
                            "type": "object",
                            "description": "Optional structured origin location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "destination": {
                            "type": "string",
                            "description": "Where the user needs to go"
                        },
                        "destination_location": {
                            "type": "object",
                            "description": "Optional structured destination location. Use saved_location with location_id for a saved place, or use a concrete non-ambiguous address/place payload.",
                            "properties": {
                                "location_type": {
                                    "type": "string",
                                    "enum": ["saved_location", "address_text", "coordinates", "place_id", "current_position"]
                                },
                                "location_id": { "type": "string" },
                                "address": { "type": "string" },
                                "latitude": { "type": "number" },
                                "longitude": { "type": "number" },
                                "place_id": { "type": "string" },
                                "provider": { "type": "string" },
                                "label": { "type": "string" }
                            }
                        },
                        "minutes_until_deadline": {
                            "type": "integer",
                            "description": "How many minutes from now the user needs to arrive. e.g. if they say 'in 30 mins' this is 30."
                        }
                    },
                    "required": ["origin", "destination", "minutes_until_deadline"]
                }),
            },
        },
        Tool {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: "create_countdown".to_string(),
                description: "Create a countdown timer (personal or agent-related). Use this for any task with a time limit (e.g., 'eat in 30 mins', 'IDE refactoring for 10 mins'). This creates a COUNTDOWN ticket with a live timer that auto-deletes when it expires.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "A short description of the task or timer, e.g., 'Refactoring code' or 'Time to leave'"
                        },
                        "duration_minutes": {
                            "type": "integer",
                            "description": "Number of minutes until the deadline."
                        }
                    },
                    "required": ["title", "duration_minutes"]
                }),
            },
        }
    ]
}