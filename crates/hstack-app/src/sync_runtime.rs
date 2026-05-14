use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hstack_core::api_models::{SyncAck, SyncActionInput, SyncActionsMessage};
use hstack_core::sync::{calculate_state_hash, SyncAction, SyncActionType};
use hstack_core::ticket::{decode_ticket_payload_for_type, Ticket, TicketPayload, TicketStatus, TicketType};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_store::StoreExt;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::{http::Request, Message}};
use uuid::Uuid;

use crate::{append_pending_action, apply_sync_update_state, load_sync_session, load_tickets_state, SyncSessionInfo};

pub const SYNC_STATUS_EVENT: &str = "hstack:sync-status";
pub const SYNC_TICKETS_CHANGED_EVENT: &str = "hstack:sync-tickets-changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConnectionStatus {
    pub connected: bool,
    pub phase: String,
    pub message: Option<String>,
    pub transport_owner: String,
}

impl Default for SyncConnectionStatus {
    fn default() -> Self {
        Self {
            connected: false,
            phase: "idle".to_string(),
            message: None,
            transport_owner: "tauri-rust".to_string(),
        }
    }
}

struct NativeSyncController {
    command_tx: mpsc::UnboundedSender<SyncRuntimeCommand>,
}

#[derive(Default)]
struct NativeSyncControllerState {
    controller: Option<NativeSyncController>,
    status: SyncConnectionStatus,
    generation: u64,
}

pub struct NativeSyncRuntimeState {
    inner: Arc<Mutex<NativeSyncControllerState>>,
}

impl Default for NativeSyncRuntimeState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(NativeSyncControllerState::default())),
        }
    }
}

#[derive(Debug)]
enum SyncRuntimeCommand {
    Flush,
    Refresh,
    Shutdown,
}

#[derive(Debug, Deserialize)]
pub struct QueueSyncActionRequest {
    pub action_type: SyncActionType,
    pub entity_id: String,
    pub entity_type: String,
    pub payload: Option<TicketPayload>,
    pub status: Option<TicketStatus>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteSyncConfig {
    base_url: String,
    api_url: String,
    ws_url: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct ServerHelloAck {
    #[serde(rename = "type")]
    _message_type: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ServerHelloOutOfSync {
    #[serde(rename = "type")]
    _message_type: String,
    server_hash: String,
}

#[derive(Debug, Deserialize)]
struct ServerStateUpdated {
    #[serde(rename = "type")]
    _message_type: String,
}

#[derive(Debug, Deserialize)]
struct RemoteTicketRecord {
    id: String,
    #[serde(rename = "type")]
    entity_type: String,
    payload: Value,
    status: TicketStatus,
    #[serde(default)]
    notes: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("sync base URL must not be empty".to_string());
    }

    Ok(trimmed.trim_end_matches('/').to_string())
}

fn build_api_url(base_url: &str, path: &str) -> Result<String, String> {
    let mut url = Url::parse(base_url).map_err(|error| format!("invalid sync base URL: {error}"))?;
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn build_ws_url(base_url: &str, user_id: i64) -> Result<String, String> {
    let mut url = Url::parse(base_url).map_err(|error| format!("invalid sync base URL: {error}"))?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => return Err(format!("unsupported sync URL scheme '{other}'")),
    };

    url.set_scheme(scheme)
        .map_err(|_| "failed to convert sync URL to websocket scheme".to_string())?;
    url.set_path(&format!("/ws/sync/{user_id}"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn resolve_remote_sync_config(base_url: String, session: SyncSessionInfo) -> Result<RemoteSyncConfig, String> {
    let normalized_base_url = normalize_base_url(&base_url)?;
    let user_id = session.user_id.ok_or_else(|| "sync session is missing user_id".to_string())?;
    let token = session.token.ok_or_else(|| "sync session is missing token".to_string())?;

    Ok(RemoteSyncConfig {
        api_url: build_api_url(&normalized_base_url, "/api/tickets")?,
        ws_url: build_ws_url(&normalized_base_url, user_id)?,
        base_url: normalized_base_url,
        token,
    })
}

fn build_ws_request(config: &RemoteSyncConfig) -> Result<Request<()>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    
    let mut request = config.ws_url.as_str().into_client_request()
        .map_err(|error| format!("failed to parse websocket uri: {error}"))?;
        
    let auth_header_value = format!("Bearer {}", config.token)
        .parse()
        .map_err(|error| format!("failed to construct auth header: {error}"))?;
        
    request.headers_mut().insert("Authorization", auth_header_value);
    
    Ok(request)
}

fn parse_remote_ticket_type(value: &str) -> TicketType {
    match value.to_ascii_uppercase().as_str() {
        "HABIT" => TicketType::Habit,
        "EVENT" => TicketType::Event,
        "COMMUTE" => TicketType::Commute,
        "COUNTDOWN" => TicketType::Countdown,
        _ => TicketType::Task,
    }
}

fn map_remote_ticket(record: RemoteTicketRecord) -> Result<Ticket, String> {
    let ticket_type = parse_remote_ticket_type(&record.entity_type);
    let payload = decode_ticket_payload_for_type(&ticket_type, record.payload)?;
    let title = payload.get_title().to_string();

    Ok(Ticket {
        id: record.id,
        title,
        r#type: ticket_type,
        status: record.status,
        payload,
        notes: record.notes,
        created_at: record.created_at,
        updated_at: record.updated_at.unwrap_or(record.created_at),
    })
}

fn pending_action_to_input(action: SyncAction) -> Result<SyncActionInput, String> {
    let action_id = Uuid::parse_str(&action.action_id)
        .map_err(|error| format!("invalid pending action_id '{}': {error}", action.action_id))?;
    let entity_id = Uuid::parse_str(&action.entity_id)
        .map_err(|error| format!("invalid pending entity_id '{}': {error}", action.entity_id))?;

    Ok(SyncActionInput {
        action_id,
        r#type: match action.r#type {
            SyncActionType::Create => "CREATE".to_string(),
            SyncActionType::Update => "UPDATE".to_string(),
            SyncActionType::Delete => "DELETE".to_string(),
        },
        entity_id,
        entity_type: action.entity_type,
        payload: action.payload,
        status: action.status.map(|status| match status {
            TicketStatus::Idle => "idle".to_string(),
            TicketStatus::InFocus => "in_focus".to_string(),
            TicketStatus::Completed => "completed".to_string(),
            TicketStatus::Expired => "expired".to_string(),
        }),
    })
}

fn current_status(runtime_state: &Arc<Mutex<NativeSyncControllerState>>) -> SyncConnectionStatus {
    match runtime_state.lock() {
        Ok(guard) => guard.status.clone(),
        Err(_) => SyncConnectionStatus {
            connected: false,
            phase: "error".to_string(),
            message: Some("sync runtime state lock poisoned".to_string()),
            transport_owner: "tauri-rust".to_string(),
        },
    }
}

fn emit_tickets_changed(app: &AppHandle) {
    let _ = app.emit(SYNC_TICKETS_CHANGED_EVENT, json!({}));
}

fn commute_needs_remote_refresh(ticket: &Ticket) -> bool {
    if ticket.r#type != TicketType::Commute {
        return false;
    }

    match &ticket.payload {
        TicketPayload::Commute { directions, .. } => {
            let Some(Value::Object(directions)) = directions.as_ref() else {
                return true;
            };

            let duration_missing = directions
                .get("total_duration_minutes")
                .is_none_or(Value::is_null);
            let duration_placeholder = directions
                .get("total_duration")
                .and_then(Value::as_str)
                .map(|value| value.trim().is_empty() || value == "Enriching via Server...")
                .unwrap_or(true);

            duration_missing || duration_placeholder
        }
        _ => false,
    }
}

fn build_commute_refresh_url(config: &RemoteSyncConfig, ticket_id: &str) -> Result<String, String> {
    build_api_url(
        &config.base_url,
        &format!("/api/tickets/{ticket_id}/commute/refresh"),
    )
}

fn update_status(
    app: &AppHandle,
    runtime_state: &Arc<Mutex<NativeSyncControllerState>>,
    generation: u64,
    status: SyncConnectionStatus,
) {
    let should_emit = match runtime_state.lock() {
        Ok(mut guard) => {
            if guard.generation != generation {
                false
            } else {
                guard.status = status.clone();
                true
            }
        }
        Err(_) => false,
    };

    if should_emit {
        let _ = app.emit(SYNC_STATUS_EVENT, status);
    }
}

async fn refresh_remote_commute_ticket(config: &RemoteSyncConfig, ticket_id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .post(build_commute_refresh_url(config, ticket_id)?)
        .bearer_auth(&config.token)
        .send()
        .await
        .map_err(|error| format!("commute refresh failed for {ticket_id}: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "commute refresh failed for {ticket_id} with status {}",
            response.status()
        ));
    }

    Ok(())
}

async fn refresh_remote_commutes_if_needed(
    config: &RemoteSyncConfig,
    ticket_ids: Vec<String>,
) -> Result<(), String> {
    for ticket_id in ticket_ids {
        refresh_remote_commute_ticket(config, &ticket_id).await?;
    }

    Ok(())
}

async fn fetch_remote_state(app: &AppHandle, config: &RemoteSyncConfig) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(&config.api_url)
        .bearer_auth(&config.token)
        .send()
        .await
        .map_err(|error| format!("remote state fetch failed: {error}"))?;
    
    println!("--- fetch_remote_state success: received response from server ---");

    if !response.status().is_success() {
        return Err(format!("remote state fetch failed with status {}", response.status()));
    }

    let records = response
        .json::<Vec<RemoteTicketRecord>>()
        .await
        .map_err(|error| format!("remote state decode failed: {error}"))?;

    let mut tickets = Vec::with_capacity(records.len());
    for record in records {
        tickets.push(map_remote_ticket(record)?);
    }

    let commute_refresh_ids = tickets
        .iter()
        .filter(|ticket| commute_needs_remote_refresh(ticket))
        .map(|ticket| ticket.id.clone())
        .collect::<Vec<_>>();

    println!("--- fetch_remote_state: applying sync update to local store ---");
    apply_sync_update_state(app.clone(), tickets).await?;
    println!("--- fetch_remote_state completed successfully ---");
    Ok(commute_refresh_ids)
}

async fn sync_remote_state(app: &AppHandle, config: &RemoteSyncConfig) -> Result<(), String> {
    let commute_refresh_ids = fetch_remote_state(app, config).await?;
    emit_tickets_changed(app);
    
    // Sidecar: refresh commutes in the background so they don't block the main sync outcome
    if !commute_refresh_ids.is_empty() {
        let config_clone = config.clone();
        tokio::spawn(async move {
            if let Err(e) = refresh_remote_commutes_if_needed(&config_clone, commute_refresh_ids).await {
                eprintln!("--- Background commute refresh failed: {e} ---");
            } else {
                println!("--- Background commute refresh success ---");
            }
        });
    }

    Ok(())
}

async fn send_hello(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    app: &AppHandle,
) -> Result<(), String> {
    let tickets = load_tickets_state(app.clone()).await?;
    let client_hash = calculate_state_hash(&tickets).map_err(|error| error.to_string())?;
    let hello = json!({
        "type": "HELLO",
        "client_hash": client_hash,
    });

    socket
        .send(Message::Text(hello.to_string()))
        .await
        .map_err(|error| format!("failed to send sync HELLO: {error}"))
}

fn remove_acked_pending_actions(app: &AppHandle, ack_ids: &[Uuid]) -> Result<(), String> {
    println!("--- remove_acked_pending_actions invoked with {} ack_ids. ---", ack_ids.len());
    let store = app
        .store("pending_actions.json")
        .map_err(|error| format!("pending actions store failure: {error}"))?;

    let mut pending_actions: Vec<SyncAction> = match store.get("pending") {
        Some(value) => serde_json::from_value(value).unwrap_or_default(),
        None => Vec::new(),
    };

    let original_len = pending_actions.len();
    pending_actions.retain(|action| !ack_ids.iter().any(|id| id.to_string() == action.action_id));
    println!("--- remove_acked_pending_actions retained {} out of {} actions ---", pending_actions.len(), original_len);

    if pending_actions.len() < original_len {
        store.set("pending", serde_json::json!(pending_actions));
        let _ = store.save();
        println!("--- pending_actions.json successfully overwritten with {} items remaining ---", pending_actions.len());
    } else {
        println!("--- No actions were matched to be removed from pending_actions.json ---");
    }
    
    Ok(())
}

async fn flush_pending_actions(
    app: &AppHandle,
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> Result<(), String> {
    let store = app
        .store("pending_actions.json")
        .map_err(|error| format!("pending actions store failure: {error}"))?;

    let pending_actions: Vec<SyncAction> = match store.get("pending") {
        Some(value) => {
            let res: Vec<SyncAction> = serde_json::from_value(value).unwrap_or_default();
            println!("--- flush_pending_actions found {} pending actions in store ---", res.len());
            res
        }
        None => {
            println!("--- flush_pending_actions found no pending actions ---");
            Vec::new()
        }
    };

    if pending_actions.is_empty() {
        return Ok(());
    }

    let actions: Vec<_> = pending_actions
        .into_iter()
        .filter_map(|action| {
            match pending_action_to_input(action) {
                Ok(input) => Some(input),
                Err(error) => {
                    eprintln!("--- skipping malformed pending action: {error} ---");
                    None
                }
            }
        })
        .collect();

    if actions.is_empty() {
        println!("--- flush_pending_actions: all actions were malformed, nothing to send ---");
        return Ok(());
    }

    let message = SyncActionsMessage {
        r#type: "SYNC_ACTIONS".to_string(),
        actions,
    };
    let payload = serde_json::to_string(&message)
        .map_err(|error| format!("failed to serialize sync actions: {error}"))?;

    socket
        .send(Message::Text(payload))
        .await
        .map_err(|error| format!("failed to send sync actions: {error}"))
}

async fn reconnect_delay(
    mut delay_seconds: u64,
    command_rx: &mut mpsc::UnboundedReceiver<SyncRuntimeCommand>,
) -> Result<u64, ()> {
    let sleep = tokio::time::sleep(Duration::from_secs(delay_seconds));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            _ = &mut sleep => {
                delay_seconds = (delay_seconds.saturating_mul(2)).min(30);
                return Ok(delay_seconds);
            }
            command = command_rx.recv() => {
                match command {
                    Some(SyncRuntimeCommand::Shutdown) | None => return Err(()),
                    Some(SyncRuntimeCommand::Flush) | Some(SyncRuntimeCommand::Refresh) => continue,
                }
            }
        }
    }
}

async fn run_sync_runtime(
    app: AppHandle,
    runtime_state: Arc<Mutex<NativeSyncControllerState>>,
    generation: u64,
    config: RemoteSyncConfig,
    mut command_rx: mpsc::UnboundedReceiver<SyncRuntimeCommand>,
) {
    let mut reconnect_delay_seconds = 1;
    println!("--- run_sync_runtime thread spawned for url: {} ---", config.ws_url);

    loop {
        update_status(
            &app,
            &runtime_state,
            generation,
            SyncConnectionStatus {
                connected: false,
                phase: "connecting".to_string(),
                message: Some(format!("connecting to {}", config.base_url)),
                transport_owner: "tauri-rust".to_string(),
            },
        );

        let request = match build_ws_request(&config) {
            Ok(request) => request,
            Err(error) => {
                update_status(
                    &app,
                    &runtime_state,
                    generation,
                    SyncConnectionStatus {
                        connected: false,
                        phase: "error".to_string(),
                        message: Some(error),
                        transport_owner: "tauri-rust".to_string(),
                    },
                );
                return;
            }
        };

        match tokio::time::timeout(std::time::Duration::from_secs(10), connect_async(request)).await {
            Ok(Ok((mut socket, _))) => {
                println!("--- WebSocket successfully connected to {} ---", config.ws_url);
                reconnect_delay_seconds = 1;

                update_status(
                    &app,
                    &runtime_state,
                    generation,
                    SyncConnectionStatus {
                        connected: false,
                        phase: "handshaking".to_string(),
                        message: Some("sync websocket connected; waiting for protocol handshake".to_string()),
                        transport_owner: "tauri-rust".to_string(),
                    },
                );

                if let Err(error) = send_hello(&mut socket, &app).await {
                    update_status(
                        &app,
                        &runtime_state,
                        generation,
                        SyncConnectionStatus {
                            connected: false,
                            phase: "reconnecting".to_string(),
                            message: Some(error),
                            transport_owner: "tauri-rust".to_string(),
                        },
                    );
                    if let Ok(next_delay) = reconnect_delay(reconnect_delay_seconds, &mut command_rx).await {
                        reconnect_delay_seconds = next_delay;
                        continue;
                    }
                    return;
                }
                println!("--- HELLO sent successfully, entering handshake wait phase ---");

                let handshake_deadline = tokio::time::sleep(Duration::from_secs(15));
                tokio::pin!(handshake_deadline);
                let mut handshake_completed = false;

                loop {
                    tokio::select! {
                        _ = &mut handshake_deadline, if !handshake_completed => {
                            eprintln!("--- handshake timeout: server did not respond within 15s, reconnecting ---");
                            update_status(
                                &app,
                                &runtime_state,
                                generation,
                                SyncConnectionStatus {
                                    connected: false,
                                    phase: "reconnecting".to_string(),
                                    message: Some("handshake timeout: server did not respond within 15s".to_string()),
                                    transport_owner: "tauri-rust".to_string(),
                                },
                            );
                            let _ = socket.close(None).await;
                            break;
                        }
                        command = command_rx.recv() => {
                            match command {
                                Some(SyncRuntimeCommand::Shutdown) | None => {
                                    let _ = socket.close(None).await;
                                    update_status(
                                        &app,
                                        &runtime_state,
                                        generation,
                                        SyncConnectionStatus {
                                            connected: false,
                                            phase: "stopped".to_string(),
                                            message: None,
                                            transport_owner: "tauri-rust".to_string(),
                                        },
                                    );
                                    return;
                                }
                                Some(SyncRuntimeCommand::Flush) => {
                                    if let Err(error) = flush_pending_actions(&app, &mut socket).await {
                                        update_status(
                                            &app,
                                            &runtime_state,
                                            generation,
                                            SyncConnectionStatus {
                                                connected: false,
                                                phase: "reconnecting".to_string(),
                                                message: Some(error),
                                                transport_owner: "tauri-rust".to_string(),
                                            },
                                        );
                                        break;
                                    }
                                }
                                Some(SyncRuntimeCommand::Refresh) => {
                                    match sync_remote_state(&app, &config).await {
                                        Ok(()) => {}
                                        Err(error) => {
                                            update_status(
                                                &app,
                                                &runtime_state,
                                                generation,
                                                SyncConnectionStatus {
                                                    connected: true,
                                                    phase: "connected".to_string(),
                                                    message: Some(error),
                                                    transport_owner: "tauri-rust".to_string(),
                                                },
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        message = socket.next() => {
                            match message {
                                Some(Ok(Message::Close(_))) | None => {
                                    update_status(
                                        &app,
                                        &runtime_state,
                                        generation,
                                        SyncConnectionStatus {
                                            connected: false,
                                            phase: "reconnecting".to_string(),
                                            message: Some("sync connection closed".to_string()),
                                            transport_owner: "tauri-rust".to_string(),
                                        },
                                    );
                                    break;
                                }
                                Some(Ok(msg)) => {
                                    println!("--- Received raw WebSocket message: {msg:?} ---");
                                    if let Message::Text(text) = msg {
                                        let parsed: Value = match serde_json::from_str(&text) {
                                            Ok(value) => value,
                                            Err(error) => {
                                                update_status(
                                                    &app,
                                                    &runtime_state,
                                                    generation,
                                                    SyncConnectionStatus {
                                                        connected: true,
                                                        phase: "connected".to_string(),
                                                        message: Some(format!("ignored malformed sync message: {error}")),
                                                        transport_owner: "tauri-rust".to_string(),
                                                    },
                                                );
                                                continue;
                                            }
                                        };

                                        let message_type = parsed.get("type").and_then(Value::as_str).unwrap_or_default();
                                        match message_type {
                                            "ACK" => {
                                                println!("--- Sync Handshake: Received ACK/IN_SYNC from server ---");
                                                handshake_completed = true;
                                                if let Ok(ack) = serde_json::from_value::<ServerHelloAck>(parsed.clone()) {
                                                    if ack.status == "IN_SYNC" {
                                                        update_status(
                                                            &app,
                                                            &runtime_state,
                                                            generation,
                                                            SyncConnectionStatus {
                                                                connected: true,
                                                                phase: "connected".to_string(),
                                                                message: None,
                                                                transport_owner: "tauri-rust".to_string(),
                                                            },
                                                        );
                                                        if let Err(error) = flush_pending_actions(&app, &mut socket).await {
                                                            eprintln!("--- flush_pending_actions failed after ACK/IN_SYNC: {error} ---");
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            "OUT_OF_SYNC" => {
                                                println!("--- Sync Handshake: Received OUT_OF_SYNC from server ---");
                                                handshake_completed = true;
                                                let _ = serde_json::from_value::<ServerHelloOutOfSync>(parsed.clone()).map(|message| message.server_hash);
                                                match sync_remote_state(&app, &config).await {
                                                Ok(()) => {
                                                    println!("--- Remote state synchronized, now flushing pending actions ---");
                                                    update_status(
                                                            &app,
                                                            &runtime_state,
                                                            generation,
                                                            SyncConnectionStatus {
                                                                connected: true,
                                                                phase: "connected".to_string(),
                                                                message: None,
                                                                transport_owner: "tauri-rust".to_string(),
                                                            },
                                                        );
                                                        if let Err(error) = flush_pending_actions(&app, &mut socket).await {
                                                            eprintln!("--- flush_pending_actions failed after OUT_OF_SYNC: {error} ---");
                                                            break;
                                                        }
                                                    }
                                                    Err(error) => {
                                                        update_status(
                                                            &app,
                                                            &runtime_state,
                                                            generation,
                                                            SyncConnectionStatus {
                                                                connected: true,
                                                                phase: "connected".to_string(),
                                                                message: Some(error),
                                                                transport_owner: "tauri-rust".to_string(),
                                                            },
                                                        );
                                                    }
                                                }
                                            }
                                            "SYNC_ACK" => {
                                                println!("--- Received SYNC_ACK payload raw: {parsed:?} ---");
                                                match serde_json::from_value::<SyncAck>(parsed.clone()) {
                                                    Ok(ack) => {
                                                        println!("--- SYNC_ACK successfully parsed containing {} ack_action_ids ---", ack.ack_action_ids.len());
                                                        update_status(
                                                            &app,
                                                            &runtime_state,
                                                            generation,
                                                            SyncConnectionStatus {
                                                                connected: true,
                                                                phase: "connected".to_string(),
                                                                message: None,
                                                                transport_owner: "tauri-rust".to_string(),
                                                            },
                                                        );
                                                        if let Err(e) = remove_acked_pending_actions(&app, &ack.ack_action_ids) {
                                                            eprintln!("--- Failed to remove acked pending actions: {e} ---");
                                                        }
                                                        match sync_remote_state(&app, &config).await {
                                                            Ok(()) => {}
                                                            Err(error) => {
                                                                update_status(
                                                                    &app,
                                                                    &runtime_state,
                                                                    generation,
                                                                    SyncConnectionStatus {
                                                                        connected: true,
                                                                        phase: "connected".to_string(),
                                                                        message: Some(error),
                                                                        transport_owner: "tauri-rust".to_string(),
                                                                    },
                                                                );
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!("--- Failed to deserialize SYNC_ACK: {e:?} ---");
                                                    }
                                                }
                                            }
                                            "STATE_UPDATED" => {
                                                println!("--- Received STATE_UPDATED from server ---");
                                                if serde_json::from_value::<ServerStateUpdated>(parsed).is_ok() {
                                                    update_status(
                                                        &app,
                                                        &runtime_state,
                                                        generation,
                                                        SyncConnectionStatus {
                                                            connected: true,
                                                            phase: "connected".to_string(),
                                                            message: None,
                                                            transport_owner: "tauri-rust".to_string(),
                                                        },
                                                    );
                                                    match sync_remote_state(&app, &config).await {
                                                        Ok(()) => {}
                                                        Err(error) => {
                                                            update_status(
                                                                &app,
                                                                &runtime_state,
                                                                generation,
                                                                SyncConnectionStatus {
                                                                    connected: true,
                                                                    phase: "connected".to_string(),
                                                                    message: Some(error),
                                                                    transport_owner: "tauri-rust".to_string(),
                                                                },
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                Some(Err(error)) => {
                                    update_status(
                                        &app,
                                        &runtime_state,
                                        generation,
                                        SyncConnectionStatus {
                                            connected: false,
                                            phase: "reconnecting".to_string(),
                                            message: Some(format!("sync transport error: {error}")),
                                            transport_owner: "tauri-rust".to_string(),
                                        },
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Ok(Err(error)) => {
                println!("--- WebSocket connection failed: {error} ---");
                update_status(
                    &app,
                    &runtime_state,
                    generation,
                    SyncConnectionStatus {
                        connected: false,
                        phase: "reconnecting".to_string(),
                        message: Some(format!("failed to connect native sync websocket: {error}")),
                        transport_owner: "tauri-rust".to_string(),
                    },
                );
            }
            Err(_) => {
                println!("--- WebSocket connection TIMED OUT while connecting to {} ---", config.ws_url);
                update_status(
                    &app,
                    &runtime_state,
                    generation,
                    SyncConnectionStatus {
                        connected: false,
                        phase: "reconnecting".to_string(),
                        message: Some("Connection timed out".to_string()),
                        transport_owner: "tauri-rust".to_string(),
                    },
                );
            }
        }

        match reconnect_delay(reconnect_delay_seconds, &mut command_rx).await {
            Ok(next_delay) => reconnect_delay_seconds = next_delay,
            Err(()) => {
                update_status(
                    &app,
                    &runtime_state,
                    generation,
                    SyncConnectionStatus {
                        connected: false,
                        phase: "stopped".to_string(),
                        message: None,
                        transport_owner: "tauri-rust".to_string(),
                    },
                );
                return;
            }
        }
    }
}

#[tauri::command]
pub async fn start_native_sync(
    app: AppHandle,
    runtime: State<'_, NativeSyncRuntimeState>,
    base_url: String,
) -> Result<(), String> {
    let config = resolve_remote_sync_config(base_url, load_sync_session(app.clone()).await?)?;
    let (command_tx, command_rx) = mpsc::unbounded_channel();

    let generation = {
        let mut guard = runtime
            .inner
            .lock()
            .map_err(|_| "sync runtime state lock poisoned".to_string())?;
        guard.generation = guard.generation.saturating_add(1);
        let generation = guard.generation;
        if let Some(existing) = guard.controller.take() {
            let _ = existing.command_tx.send(SyncRuntimeCommand::Shutdown);
        }
        guard.controller = Some(NativeSyncController { command_tx: command_tx.clone() });
        guard.status = SyncConnectionStatus {
            connected: false,
            phase: "starting".to_string(),
            message: None,
            transport_owner: "tauri-rust".to_string(),
        };
        generation
    };

    let runtime_state = runtime.inner.clone();
    update_status(&app, &runtime_state, generation, current_status(&runtime_state));
    tauri::async_runtime::spawn(run_sync_runtime(app.clone(), runtime_state, generation, config, command_rx));
    let _ = command_tx.send(SyncRuntimeCommand::Refresh);
    Ok(())
}

#[tauri::command]
pub async fn stop_native_sync(
    app: AppHandle,
    runtime: State<'_, NativeSyncRuntimeState>,
) -> Result<(), String> {
    let status = {
        let mut guard = runtime
            .inner
            .lock()
            .map_err(|_| "sync runtime state lock poisoned".to_string())?;
        guard.generation = guard.generation.saturating_add(1);
        if let Some(existing) = guard.controller.take() {
            let _ = existing.command_tx.send(SyncRuntimeCommand::Shutdown);
        }
        guard.status = SyncConnectionStatus::default();
        guard.status.clone()
    };

    let _ = app.emit(SYNC_STATUS_EVENT, status);
    Ok(())
}

#[tauri::command]
pub async fn get_sync_connection_status(
    runtime: State<'_, NativeSyncRuntimeState>,
) -> Result<SyncConnectionStatus, String> {
    Ok(current_status(&runtime.inner))
}

#[tauri::command]
pub async fn queue_sync_action(
    app: AppHandle,
    _runtime: State<'_, NativeSyncRuntimeState>,
    action: QueueSyncActionRequest,
) -> Result<Vec<Ticket>, String> {
    append_pending_action(
        &app,
        action.action_type,
        action.entity_id,
        action.entity_type,
        action.payload,
        action.status,
        action.notes,
    )
    .await?;
    if let Err(e) = trigger_flush(&app) {
        eprintln!("Failed to trigger sync flush: {e}");
    }

    load_tickets_state(app).await
}

pub fn trigger_flush(app: &AppHandle) -> Result<(), String> {
    let runtime = app.try_state::<NativeSyncRuntimeState>().ok_or("Sync runtime not found")?;
    if let Ok(guard) = runtime.inner.lock() {
        if let Some(controller) = &guard.controller {
            let _ = controller.command_tx.send(SyncRuntimeCommand::Flush);
            return Ok(());
        }
    }
    Err("Sync controller not ready".to_string())
}

#[tauri::command]
pub async fn sync_refresh_now(
    app: AppHandle,
) -> Result<Vec<Ticket>, String> {
    let runtime = app.try_state::<NativeSyncRuntimeState>().ok_or("Sync runtime not found")?;
    if let Ok(guard) = runtime.inner.lock() {
        if let Some(controller) = &guard.controller {
            let _ = controller.command_tx.send(SyncRuntimeCommand::Refresh);
        }
    }

    load_tickets_state(app).await
}