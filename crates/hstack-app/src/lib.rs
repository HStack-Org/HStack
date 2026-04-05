#![deny(clippy::unwrap_used, clippy::expect_used)]

// Public client entrypoint.
// Review docs/public-private-contract.md before coupling client behavior to private-only backend capabilities.
mod app_state;
mod location_utils;
mod agent_integration;
mod secure_store;
mod sync_runtime;
mod voice_runtime;

use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;
use rustls::crypto::ring::default_provider;
use hstack_core::error::Error as CoreError;
use hstack_core::provider::{Message, ProviderConfig};
use hstack_core::sync::SyncAction;
use secure_store::SecureStore;
use sync_runtime::NativeSyncRuntimeState;
pub(crate) use app_state::{append_pending_action, apply_sync_update_state, get_settings, load_sync_session, load_tickets_state, SyncSessionInfo};


use hstack_agent::provider::{OpenAiProvider, GeminiProvider};
use hstack_agent::{Agent, AgentPromptProfile, build_base_prompt};
use hstack_agent::manager::SimpleContextManager;
use tauri::Emitter;
use crate::agent_integration::{TauriAgentControl, TauriHStackWorld};

#[tauri::command]
async fn chat_local(app: AppHandle, message: String, _history: Vec<Message>) -> Result<Vec<Message>, String> {
    println!("--- CHAT LOCAL RECEIVED MESSAGE: {message} ---");
    let settings = match get_settings(app.clone()).await {
        Ok(s) => s,
        Err(e) => return Err(e),
    };

    let active_provider = match settings.active_provider() {
        Some(p) => p,
        None => return Err("No active provider configured".to_string()),
    };
    
    // Resolve full config by fetching key from the app's secure store
    let api_key = SecureStore::get_key(&app, &active_provider.id)?;
    let config = ProviderConfig {
        name: active_provider.name.clone(),
        kind: active_provider.kind.clone(),
        endpoint: active_provider.endpoint.clone(),
        api_key,
        model_name: active_provider.model_name.clone(),
        rate_limit: active_provider.rate_limit.clone(),
    };

    let app_clone = app.clone();
    
    tauri::async_runtime::spawn(async move {
        let mut memory = match crate::app_state::load_agent_memory(app_clone.clone()).await {
            Ok(mem) => mem,
            Err(e) => {
                println!("Failed to load agent memory: {e}");
                return;
            }
        };
        
        let provider: Box<dyn hstack_agent::provider::LlmProvider> = match config.kind {
            hstack_core::provider::ProviderKind::OpenAiCompatible => Box::new(OpenAiProvider::new(config.clone(), None)),
            hstack_core::provider::ProviderKind::Gemini => Box::new(GeminiProvider::new(config.clone(), None)),
        };

        let tools = hstack_agent::tool::compose_tools(&[
            "create_ticket", "delete_ticket", "delete_all_tickets", "edit_ticket",
            "add_commute", "get_directions", "remove_commute", "start_live_directions", 
            "create_countdown", "identity", "follow_up", "search_stack", "scratch_thought",
            "light_compute", "manage_app", "inspect_app", "scratchpad_search", "scratchpad_edit"
        ]).unwrap_or_default();
        
        let agent = Agent {
            provider,
            manager: Box::new(SimpleContextManager),
            control: Box::new(TauriAgentControl::new()),
            tools,
            base_prompt: build_base_prompt(AgentPromptProfile::DebugInteractive),
        };

        memory.push_message(Message {
            role: hstack_core::provider::Role::User,
            content: Some(message.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        let world = TauriHStackWorld { app: app_clone.clone() };

        let run_result = agent.run(&world, &mut memory).await;

        match &run_result {
            Ok((answer, _deltas)) => {
                let _ = app_clone.emit("AGENT_ANSWER", serde_json::json!({
                    "answer": answer,
                }));
            }
            Err(e) => {
                println!("Agent run failed: {e}");
                let _ = app_clone.emit("AGENT_ANSWER", serde_json::json!({
                    "answer": null,
                    "error": format!("{e}"),
                }));
            }
        }

        if let Err(e) = crate::app_state::save_agent_memory(app_clone.clone(), memory).await {
            println!("Failed to save agent memory: {e}");
        }
        
        let _ = app_clone.emit("AGENT_PROPOSALS_SYNC", ());
        let _ = app_clone.emit("AGENT_DONE", ());
    });

    Ok(vec![])
}

fn install_rustls_crypto_provider() -> Result<(), String> {
    default_provider()
        .install_default()
        .map(|_| ())
        .map_err(|error| format!("failed to install rustls crypto provider: {error:?}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = install_rustls_crypto_provider() {
        panic!("{error}");
    }

    let app = match tauri::Builder::default()
        .manage(NativeSyncRuntimeState::default())
        .manage(voice_runtime::VoiceRuntimeState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            app_state::get_settings,
            app_state::save_settings,
            app_state::upsert_provider,
            app_state::delete_provider,
            app_state::warm_secure_store,
            app_state::get_voice_secret_status,
            app_state::save_voice_direct_api_key,
            app_state::clear_voice_direct_api_key,
            chat_local,
            app_state::get_tickets,
            app_state::apply_sync_update,
            app_state::get_user_locale,
            app_state::get_sync_session,
            app_state::save_sync_session,
            app_state::clear_sync_session,
            app_state::complete_onboarding,
            app_state::get_agent_proposals,
            app_state::accept_agent_proposals,
            app_state::reject_agent_proposals,
            sync_runtime::start_native_sync,
            sync_runtime::stop_native_sync,
            sync_runtime::get_sync_connection_status,
            sync_runtime::queue_sync_action,
            sync_runtime::sync_refresh_now,
            voice_runtime::start_voice_transcription,
            voice_runtime::append_voice_audio_chunk,
            voice_runtime::stop_voice_transcription
        ])
        .build(tauri::generate_context!()) {
            Ok(app) => app,
            Err(error) => panic!("error while building tauri application: {error}"),
        };

    app.run(|app_handle, event| {
            #[cfg(desktop)]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
            }

            #[cfg(not(desktop))]
            {
                let _ = (&app_handle, &event);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{extract_first_json_value, format_saved_locations_for_prompt, infer_commute_payload_from_event, resolve_commute_location, validate_plan, PlannerAction, PlannerPlan};
    use crate::location_utils::normalize_legacy_commute_payload;
    use crate::planner_support::{PlannerCommitment, PlannerDependencyImpact};
    use hstack_core::settings::{SavedLocation, UserSettings};
    use hstack_core::ticket::{tool_schemas, TicketLocation, TicketPayload};
    use serde::Serialize;
    use serde_json::{json, Value};

    fn must_extract_json_value(input: &str) -> Value {
        match extract_first_json_value(input) {
            Some(value) => value,
            None => panic!("expected fenced JSON to parse"),
        }
    }

    fn assert_plan_is_valid(plan: PlannerPlan) {
        if let Err(error) = validate_plan(plan, &tool_schemas()) {
            panic!("expected valid planner plan: {error}");
        }
    }

    fn expect_plan_validation_error(plan: PlannerPlan) -> String {
        match validate_plan(plan, &tool_schemas()) {
            Ok(_) => panic!("expected validation to fail"),
            Err(error) => error,
        }
    }

    fn must_infer_commute_payload(event_id: &str, payload: &TicketPayload) -> TicketPayload {
        match infer_commute_payload_from_event(event_id, payload) {
            Some(commute) => commute,
            None => panic!("expected a commute to be inferred"),
        }
    }

    fn expect_commute_location_error(args: &Value, settings: &UserSettings) -> String {
        match resolve_commute_location(args, "origin_location", "origin", "origin location", settings) {
            Ok(_) => panic!("expected location resolution to fail"),
            Err(error) => error,
        }
    }

    fn must_json_value<T: Serialize>(value: T) -> Value {
        match serde_json::to_value(value) {
            Ok(json) => json,
            Err(error) => panic!("expected value to serialize in test: {error}"),
        }
    }

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

    fn sample_plan() -> PlannerPlan {
        PlannerPlan {
            user_goal: "Reschedule prep work before a birthday dinner".to_string(),
            grounded_facts: vec![
                "Birthday dinner is a dated event already in the stack".to_string(),
                "Buy flowers depends on that dinner happening on time".to_string(),
            ],
            time_constraints: vec!["Dinner is next Friday at 19:00".to_string()],
            existing_tickets_relevant: vec!["event-birthday-dinner".to_string(), "task-buy-flowers".to_string()],
            dependent_tickets_impacted: vec![PlannerDependencyImpact {
                ticket_id: "task-buy-flowers".to_string(),
                title: Some("Buy flowers".to_string()),
                reason: "It is anchored to the dinner date".to_string(),
                action_required: true,
            }],
            new_commitments_detected: vec![PlannerCommitment {
                r#type: Some("EVENT".to_string()),
                title: Some("Birthday dinner".to_string()),
                rrule: Some("DTSTART:20260320T190000Z".to_string()),
                duration_minutes: Some(120),
            }],
            proactive_opportunities: vec!["Move flower pickup earlier in the day".to_string()],
            assumptions_to_apply: vec!["Use the existing event as the anchor".to_string()],
            tool_actions: vec![
                PlannerAction {
                    tool: "create_ticket".to_string(),
                    arguments: json!({
                        "type": "EVENT",
                        "title": "Birthday dinner",
                        "rrule": "DTSTART:20260320T190000Z",
                        "duration_minutes": 120,
                    }),
                },
                PlannerAction {
                    tool: "edit_ticket".to_string(),
                    arguments: json!({
                        "ticket_id": "task-buy-flowers",
                        "rrule": "DTSTART:20260320T140000Z"
                    }),
                },
            ],
            user_reply_strategy: "Explain the reschedule and confirm the new sequence briefly.".to_string(),
        }
    }

    #[test]
    fn extracts_json_from_fenced_planner_output() {
        let parsed = must_extract_json_value("```json\n{\"user_goal\":\"Plan\"}\n```");

        assert_eq!(parsed.get("user_goal").and_then(|value| value.as_str()), Some("Plan"));
    }

    #[test]
    fn validates_dependency_aware_plan() {
        let plan = sample_plan();

        assert_plan_is_valid(plan);
    }

    #[test]
    fn rejects_tool_actions_without_grounded_facts() {
        let mut plan = sample_plan();
        plan.grounded_facts.clear();

        let error = expect_plan_validation_error(plan);
        assert!(error.contains("grounded facts"));
    }

    #[test]
    fn rejects_commitment_details_without_title() {
        let mut plan = sample_plan();
        plan.new_commitments_detected[0].title = None;

        let error = expect_plan_validation_error(plan);
        assert!(error.contains("no title"));
    }

    #[test]
    fn rejects_duplicate_dependent_ticket_entries() {
        let mut plan = sample_plan();
        plan.dependent_tickets_impacted.push(PlannerDependencyImpact {
            ticket_id: "task-buy-flowers".to_string(),
            title: Some("Buy flowers".to_string()),
            reason: "Duplicate reference".to_string(),
            action_required: false,
        });

        let error = expect_plan_validation_error(plan);
        assert!(error.contains("more than once"));
    }

    #[test]
    fn rejects_edit_without_action_required_flag() {
        let mut plan = sample_plan();
        plan.dependent_tickets_impacted[0].action_required = false;

        let error = expect_plan_validation_error(plan);
        assert!(error.contains("action_required=true"));
    }

    #[test]
    fn accepts_zero_duration_commitment_when_create_ticket_omits_duration() {
        let mut plan = sample_plan();
        plan.new_commitments_detected[0] = PlannerCommitment {
            r#type: Some("TASK".to_string()),
            title: Some("Walk the cat".to_string()),
            rrule: None,
            duration_minutes: Some(0),
        };
        plan.tool_actions = vec![PlannerAction {
            tool: "create_ticket".to_string(),
            arguments: json!({
                "type": "TASK",
                "title": "Walk the cat"
            }),
        }];
        plan.dependent_tickets_impacted.clear();

        assert_plan_is_valid(plan);
    }

    #[test]
    fn normalizes_text_commute_locations_to_address_text() {
        let settings = UserSettings::default();
        let args = json!({
            "origin": "221B Baker Street, London"
        });

        let (display, location) = match resolve_commute_location(&args, "origin_location", "origin", "origin location", &settings) {
            Ok(value) => value,
            Err(error) => panic!("expected strict location normalization to succeed: {error}"),
        };

        assert_eq!(display, "221B Baker Street, London");
        assert_eq!(location, TicketLocation::AddressText {
            address: "221B Baker Street, London".to_string(),
            label: None,
        });
    }

    #[test]
    fn rejects_mismatched_text_and_structured_commute_locations() {
        let settings = UserSettings::default();
        let args = json!({
            "origin": "Current position",
            "origin_location": {
                "location_type": "address_text",
                "address": "10 Downing Street, London"
            }
        });

        let error = expect_commute_location_error(&args, &settings);

        assert!(error.contains("does not match structured location"));
    }

    #[test]
    fn rejects_saved_location_labels_as_raw_text() {
        let settings = settings_with_home();
        let args = json!({
            "origin": "Home"
        });

        let error = expect_commute_location_error(&args, &settings);

        assert!(error.contains("use location_id"));
    }

    #[test]
    fn rejects_ambiguous_raw_location_text() {
        let settings = UserSettings::default();
        let args = json!({
            "origin": "my place"
        });

        let error = expect_commute_location_error(&args, &settings);

        assert!(error.contains("ambiguous"));
    }

    #[test]
    fn formats_saved_locations_for_prompt_context() {
        let rendered = format_saved_locations_for_prompt(&settings_with_home().saved_locations);

        assert!(rendered.contains("loc-home"));
        assert!(rendered.contains("Home"));
        assert!(rendered.contains("12 Rue de Rivoli, Paris"));
    }

    #[test]
    fn infers_commute_from_scheduled_event_with_location() {
        let payload = TicketPayload::Event {
            title: "Team dinner".to_string(),
            scheduled_time_iso: Some("2026-03-28T19:30:00+00:00".to_string()),
            rrule: None,
            duration_minutes: Some(90),
            location: Some(TicketLocation::AddressText {
                address: "12 Rue de Rivoli, Paris".to_string(),
                label: Some("Restaurant".to_string()),
            }),
            status: None,
            priority: None,
            completed: Some(false),
        };

        let commute = must_infer_commute_payload("event-1", &payload);

        match commute {
            TicketPayload::Commute {
                departure_time,
                destination,
                destination_location,
                related_event_id,
                scheduled_time_iso,
                ..
            } => {
                assert_eq!(destination, "12 Rue de Rivoli, Paris");
                assert_eq!(related_event_id.as_deref(), Some("event-1"));
                assert_eq!(scheduled_time_iso.as_deref(), Some("2026-03-28T19:30:00+00:00"));
                assert_eq!(must_json_value(departure_time), json!({
                    "departure_type": "relative_to_arrival",
                    "buffer_minutes": 10
                }));
                assert!(matches!(destination_location, Some(TicketLocation::AddressText { .. })));
            }
            other => panic!("expected commute payload, got {other:?}"),
        }
    }

    #[test]
    fn does_not_infer_commute_without_structured_destination() {
        let payload = TicketPayload::Event {
            title: "Deep work".to_string(),
            scheduled_time_iso: Some("2026-03-28T09:00:00+00:00".to_string()),
            rrule: None,
            duration_minutes: Some(120),
            location: None,
            status: None,
            priority: None,
            completed: Some(false),
        };

        assert!(infer_commute_payload_from_event("event-2", &payload).is_none());
    }

    #[test]
    fn normalizes_legacy_scheduled_commute_to_relative_departure() {
        let mut payload = TicketPayload::Commute {
            title: "Commute to dinner".to_string(),
            label: Some("event_commute".to_string()),
            origin: "Current position".to_string(),
            origin_location: Some(TicketLocation::CurrentPosition {
                label: Some("Current position".to_string()),
            }),
            destination: "12 Rue de Rivoli, Paris".to_string(),
            destination_location: Some(TicketLocation::AddressText {
                address: "12 Rue de Rivoli, Paris".to_string(),
                label: None,
            }),
            departure_time: None,
            scheduled_time_iso: Some("2026-04-04T20:00:00+00:00".to_string()),
            rrule: None,
            deadline: Some("20:00".to_string()),
            days: None,
            related_event_id: Some("event-1".to_string()),
            live: None,
            minutes_remaining: None,
            directions: None,
            priority: None,
            completed: Some(false),
        };

        normalize_legacy_commute_payload(&mut payload);

        match payload {
            TicketPayload::Commute { departure_time, .. } => {
                assert_eq!(must_json_value(departure_time), json!({
                    "departure_type": "relative_to_arrival",
                    "buffer_minutes": 10
                }));
            }
            other => panic!("expected commute payload, got {other:?}"),
        }
    }
}
