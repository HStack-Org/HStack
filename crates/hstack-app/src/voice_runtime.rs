use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use hstack_core::voice::{AudioFormat, ManagedVoiceAuthMessage, VoiceMode, MANAGED_VOICE_WEBSOCKET_PATH};
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::Request, protocol::frame::CloseFrame, Message},
};

use crate::{app_state::VOICE_DIRECT_API_KEY_KEY, get_settings, load_sync_session, secure_store::SecureStore};

pub const VOICE_EVENT: &str = "hstack:voice-event";

struct VoiceController {
    command_tx: mpsc::UnboundedSender<VoiceRuntimeCommand>,
    generation: u64,
}

#[derive(Default)]
struct VoiceRuntimeControllerState {
    controller: Option<VoiceController>,
    generation: u64,
}

pub struct VoiceRuntimeState {
    inner: Arc<Mutex<VoiceRuntimeControllerState>>,
}

impl Default for VoiceRuntimeState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VoiceRuntimeControllerState::default())),
        }
    }
}

enum VoiceRuntimeCommand {
    AppendAudio { audio_base64: String },
    Stop,
}

enum VoiceTransportConfig {
    Direct {
        request: Request<()>,
        audio_format: AudioFormat,
        target_streaming_delay_ms: Option<u32>,
    },
    Managed {
        ws_url: String,
        auth_message: ManagedVoiceAuthMessage,
    },
}

#[derive(Clone, Serialize)]
struct VoiceEventPayload {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_mode: Option<String>,
}

fn emit_voice_event(app: &AppHandle, payload: VoiceEventPayload) {
    let _ = app.emit(VOICE_EVENT, payload);
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("voice base URL must not be empty".to_string());
    }

    Ok(trimmed.trim_end_matches('/').to_string())
}

fn build_managed_voice_ws_url(base_url: &str, websocket_path: Option<&str>) -> Result<String, String> {
    let mut url = Url::parse(base_url).map_err(|error| format!("invalid remote voice base URL: {error}"))?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        "wss" => "wss",
        "ws" => "ws",
        other => return Err(format!("unsupported remote voice URL scheme '{other}'")),
    };

    url.set_scheme(scheme)
        .map_err(|_| "failed to convert remote voice URL to websocket scheme".to_string())?;
    url.set_path(websocket_path.unwrap_or(MANAGED_VOICE_WEBSOCKET_PATH));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn build_direct_voice_request(endpoint: &str, model_name: &str, api_key: &str) -> Result<Request<()>, String> {
    let normalized = normalize_base_url(endpoint)?;
    let mut url = Url::parse(&normalized).map_err(|error| format!("invalid direct voice endpoint: {error}"))?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        "wss" => "wss",
        "ws" => "ws",
        other => return Err(format!("unsupported direct voice URL scheme '{other}'")),
    };

    url.set_scheme(scheme)
        .map_err(|_| "failed to convert direct voice URL to websocket scheme".to_string())?;
    url.set_path("/v1/audio/transcriptions/realtime");
    {
        let mut query = url.query_pairs_mut();
        query.clear();
        query.append_pair("model", model_name);
    }
    url.set_fragment(None);

    let mut request = url
        .to_string()
        .into_client_request()
        .map_err(|error| format!("failed to build direct voice request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {api_key}")
            .parse()
            .map_err(|error| format!("failed to encode direct voice authorization header: {error}"))?,
    );
    Ok(request)
}

async fn resolve_voice_transport(app: &AppHandle, remote_base_url: Option<String>) -> Result<VoiceTransportConfig, String> {
    let settings = get_settings(app.clone()).await?;
    let voice_settings = settings.voice.clone();

    if matches!(voice_settings.mode, VoiceMode::Disabled) {
        return Err("voice input is disabled in settings".to_string());
    }

    let direct_api_key = SecureStore::get_key(app, VOICE_DIRECT_API_KEY_KEY)?;
    let direct_api_key_present = !direct_api_key.trim().is_empty();
    let sync_session = load_sync_session(app.clone()).await?;

    let audio_format = AudioFormat::default();

    match voice_settings.mode {
        VoiceMode::Auto => {
            let base_url = remote_base_url
                .ok_or_else(|| "managed voice requires a resolved remote base URL".to_string())?;
            Ok(VoiceTransportConfig::Managed {
                ws_url: build_managed_voice_ws_url(base_url.as_str(), Some(MANAGED_VOICE_WEBSOCKET_PATH))?,
                auth_message: ManagedVoiceAuthMessage {
                    token: sync_session
                        .token
                        .clone()
                        .ok_or_else(|| "managed voice session is missing token".to_string())?,
                    audio_format,
                    target_streaming_delay_ms: voice_settings.target_streaming_delay_ms,
                },
            })
        }
        VoiceMode::DirectOnly => {
            if !direct_api_key_present {
                return Err("no direct voice API key is configured".to_string());
            }

            Ok(VoiceTransportConfig::Direct {
                request: build_direct_voice_request(
                    &voice_settings.direct_api_base_url,
                    &voice_settings.direct_model_name,
                    direct_api_key.trim(),
                )?,
                audio_format,
                target_streaming_delay_ms: voice_settings.target_streaming_delay_ms,
            })
        }
        VoiceMode::Disabled => Err("voice input is disabled in settings".to_string()),
    }
}

fn cleanup_controller(state: &VoiceRuntimeState, generation: u64) {
    if let Ok(mut inner) = state.inner.lock() {
        if inner.controller.as_ref().map(|controller| controller.generation) == Some(generation) {
            inner.controller = None;
        }
    }
}

fn parse_error_message(payload: &Value) -> String {
    payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| "voice transcription failed".to_string())
}

fn describe_close_frame(frame: Option<CloseFrame>) -> String {
    match frame {
        Some(frame) if frame.reason.is_empty() => {
            format!("voice websocket closed by remote endpoint (code {})", frame.code)
        }
        Some(frame) => format!(
            "voice websocket closed by remote endpoint (code {}): {}",
            frame.code,
            frame.reason
        ),
        None => "voice websocket closed by remote endpoint".to_string(),
    }
}

async fn run_voice_session(
    app: AppHandle,
    state: VoiceRuntimeState,
    generation: u64,
    transport: VoiceTransportConfig,
    mut command_rx: mpsc::UnboundedReceiver<VoiceRuntimeCommand>,
) {
    let run_result = run_voice_session_inner(&app, &transport, &mut command_rx).await;

    if let Err(message) = run_result {
        eprintln!("voice runtime error: {message}");
        emit_voice_event(
            &app,
            VoiceEventPayload {
                event_type: "error".to_string(),
                text: None,
                message: Some(message),
                selected_mode: None,
            },
        );
    }

    emit_voice_event(
        &app,
        VoiceEventPayload {
            event_type: "stopped".to_string(),
            text: None,
            message: None,
            selected_mode: None,
        },
    );

    cleanup_controller(&state, generation);
}

async fn run_voice_session_inner(
    app: &AppHandle,
    transport: &VoiceTransportConfig,
    command_rx: &mut mpsc::UnboundedReceiver<VoiceRuntimeCommand>,
) -> Result<(), String> {
    let (mut socket, selected_mode) = match transport {
        VoiceTransportConfig::Direct {
            request,
            audio_format,
            target_streaming_delay_ms,
        } => {
            let (mut socket, _) = connect_async(request.clone())
                .await
                .map_err(|error| format!("failed to connect direct voice transport: {error}"))?;

            let session_update = json!({
                "type": "session.update",
                "session": {
                    "audio_format": audio_format,
                    "target_streaming_delay_ms": target_streaming_delay_ms,
                }
            });

            socket
                .send(Message::Text(session_update.to_string()))
                .await
                .map_err(|error| format!("failed to initialize direct voice session: {error}"))?;

            (socket, "direct".to_string())
        }
        VoiceTransportConfig::Managed { ws_url, auth_message } => {
            let (mut socket, _) = connect_async(ws_url)
                .await
                .map_err(|error| format!("failed to connect managed voice transport: {error}"))?;
            let auth_json = serde_json::to_string(&json!({
                "type": "auth",
                "token": auth_message.token,
                "audio_format": auth_message.audio_format,
                "target_streaming_delay_ms": auth_message.target_streaming_delay_ms,
            }))
            .map_err(|error| format!("failed to serialize managed voice auth: {error}"))?;

            socket
                .send(Message::Text(auth_json))
                .await
                .map_err(|error| format!("failed to authenticate managed voice transport: {error}"))?;

            (socket, "managed".to_string())
        }
    };

    emit_voice_event(
        app,
        VoiceEventPayload {
            event_type: "started".to_string(),
            text: None,
            message: None,
            selected_mode: Some(selected_mode),
        },
    );

    let mut stop_sent = false;
    let mut appended_audio = false;

    loop {
        tokio::select! {
            command = command_rx.recv(), if !stop_sent => {
                match command {
                    Some(VoiceRuntimeCommand::AppendAudio { audio_base64 }) => {
                        appended_audio = true;
                        let append_message = json!({
                            "type": "input_audio.append",
                            "audio": audio_base64,
                        });

                        socket
                            .send(Message::Text(append_message.to_string()))
                            .await
                            .map_err(|error| format!("failed to stream audio chunk: {error}"))?;
                    }
                    Some(VoiceRuntimeCommand::Stop) | None => {
                        if appended_audio {
                            socket
                                .send(Message::Text(json!({ "type": "input_audio.flush" }).to_string()))
                                .await
                                .map_err(|error| format!("failed to flush voice audio: {error}"))?;
                            socket
                                .send(Message::Text(json!({ "type": "input_audio.end" }).to_string()))
                                .await
                                .map_err(|error| format!("failed to end voice audio: {error}"))?;
                        } else {
                            let _ = socket.close(None).await;
                            break;
                        }
                        stop_sent = true;
                    }
                }
            }
            message = socket.next() => {
                let Some(message_result) = message else {
                    break;
                };

                let message = message_result
                    .map_err(|error| format!("voice websocket error: {error}"))?;

                match message {
                    Message::Text(text) => {
                        let payload: Value = serde_json::from_str(&text)
                            .map_err(|error| format!("invalid voice event payload: {error}"))?;
                        let message_type = payload
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default();

                        match message_type {
                            "session.created" => {
                                emit_voice_event(
                                    app,
                                    VoiceEventPayload {
                                        event_type: "ready".to_string(),
                                        text: None,
                                        message: None,
                                        selected_mode: None,
                                    },
                                );
                            }
                            "transcription.text.delta" => {
                                let text = payload
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                emit_voice_event(
                                    app,
                                    VoiceEventPayload {
                                        event_type: "partial".to_string(),
                                        text: Some(text),
                                        message: None,
                                        selected_mode: None,
                                    },
                                );
                            }
                            "transcription.done" => {
                                let final_text = payload
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .map(ToString::to_string);
                                emit_voice_event(
                                    app,
                                    VoiceEventPayload {
                                        event_type: "done".to_string(),
                                        text: final_text,
                                        message: None,
                                        selected_mode: None,
                                    },
                                );
                                break;
                            }
                            "error" => {
                                return Err(parse_error_message(&payload));
                            }
                            _ => {}
                        }
                    }
                    Message::Close(frame) => {
                        if !stop_sent {
                            return Err(describe_close_frame(frame));
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = socket.close(None).await;
    Ok(())
}

#[tauri::command]
pub async fn start_voice_transcription(
    app: AppHandle,
    remote_base_url: Option<String>,
    state: State<'_, VoiceRuntimeState>,
) -> Result<(), String> {
    let transport = resolve_voice_transport(&app, remote_base_url).await?;
    let (command_tx, command_rx) = mpsc::unbounded_channel();

    let generation = {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "voice runtime lock failure".to_string())?;

        if inner.controller.is_some() {
            return Err("voice transcription is already running".to_string());
        }

        inner.generation += 1;
        let generation = inner.generation;
        inner.controller = Some(VoiceController {
            command_tx,
            generation,
        });
        generation
    };

    let runtime_state = VoiceRuntimeState {
        inner: state.inner.clone(),
    };

    tokio::spawn(run_voice_session(app, runtime_state, generation, transport, command_rx));
    Ok(())
}

#[tauri::command]
pub async fn append_voice_audio_chunk(
    audio_base64: String,
    state: State<'_, VoiceRuntimeState>,
) -> Result<(), String> {
    let command_tx = {
        let inner = state
            .inner
            .lock()
            .map_err(|_| "voice runtime lock failure".to_string())?;
        inner
            .controller
            .as_ref()
            .map(|controller| controller.command_tx.clone())
            .ok_or_else(|| "voice transcription is not running".to_string())?
    };

    command_tx
        .send(VoiceRuntimeCommand::AppendAudio { audio_base64 })
        .map_err(|_| "voice runtime is unavailable".to_string())
}

#[tauri::command]
pub async fn stop_voice_transcription(state: State<'_, VoiceRuntimeState>) -> Result<(), String> {
    let command_tx = {
        let inner = state
            .inner
            .lock()
            .map_err(|_| "voice runtime lock failure".to_string())?;
        inner
            .controller
            .as_ref()
            .map(|controller| controller.command_tx.clone())
            .ok_or_else(|| "voice transcription is not running".to_string())?
    };

    command_tx
        .send(VoiceRuntimeCommand::Stop)
        .map_err(|_| "voice runtime is unavailable".to_string())
}