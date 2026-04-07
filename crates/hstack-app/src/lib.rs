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
use rustls::crypto::ring::default_provider;
use hstack_core::provider::{Message, ProviderConfig};
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

    let builder = tauri::Builder::default()
        .manage(NativeSyncRuntimeState::default())
        .manage(voice_runtime::VoiceRuntimeState::default())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|_app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;

            _app.deep_link()
                    .register_all()
                    .map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })?;
            }

            Ok(())
        })
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
        ]);

    let app = match builder
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
    use crate::location_utils::{
        format_saved_locations_for_prompt,
        infer_commute_payload_from_event,
        normalize_legacy_commute_payload,
        resolve_commute_location,
    };
    use hstack_core::settings::{SavedLocation, UserSettings};
    use hstack_core::ticket::{TicketLocation, TicketPayload};
    use serde::Serialize;
    use serde_json::{json, Value};

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
