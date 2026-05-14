use chrono::Utc;
use hstack_agent::filesystem::LocalSandboxedFilesystem;
use hstack_agent::workspace::AppLifecycle;
use hstack_core::filesystem::FilesystemPolicy;
use hstack_core::settings::{SavedProvider, SyncMode, UserSettings};
use hstack_core::sync::{project_state, reconcile_state, SyncAction, SyncActionType};
use hstack_core::ticket::{Ticket, TicketPayload, TicketStatus};
use hstack_core::virtual_fs::VirtualPath;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tauri_plugin_store::StoreExt;
use uuid::Uuid;

use crate::location_utils::normalize_projected_tickets;
use crate::secure_store::SecureStore;
use crate::sync_runtime::SYNC_TICKETS_CHANGED_EVENT;

const SYNC_TOKEN_KEY: &str = "hstack-sync-token";
pub(crate) const VOICE_DIRECT_API_KEY_KEY: &str = "hstack-voice-direct-api-key";
const AGENT_WORKSPACE_STORE: &str = "agent_workspace.json";
const AGENT_FILESYSTEM_MOUNT_KEY: &str = "filesystem_mount_root";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SyncSessionInfo {
    pub(crate) user_id: Option<i64>,
    pub(crate) user_name: Option<String>,
    pub(crate) token: Option<String>,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct VoiceSecretStatus {
    pub(crate) direct_api_key_present: bool,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct AgentFilesystemMountState {
    pub(crate) host_path: Option<String>,
    pub(crate) folder_picker_supported: bool,
}

fn agent_folder_picker_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows", target_os = "linux"))
}

fn load_agent_filesystem_mount_path(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let store = app.store(AGENT_WORKSPACE_STORE)
        .map_err(|e| format!("Agent workspace store failure: {e}"))?;

    match store.get(AGENT_FILESYSTEM_MOUNT_KEY) {
        Some(value) => {
            let raw = serde_json::from_value::<Option<String>>(value)
                .map_err(|e| format!("Failed to parse agent filesystem mount: {e}"))?;
            Ok(raw.map(PathBuf::from))
        }
        None => Ok(None),
    }
}

fn save_agent_filesystem_mount_path(app: &AppHandle, path: Option<&Path>) -> Result<(), String> {
    let store = app.store(AGENT_WORKSPACE_STORE)
        .map_err(|e| format!("Agent workspace store failure: {e}"))?;

    let value = path.map(|mounted| mounted.to_string_lossy().to_string());
    store.set(AGENT_FILESYSTEM_MOUNT_KEY, serde_json::json!(value));
    store.save().map_err(|e| format!("Failed to save agent filesystem mount: {e}"))
}

pub(crate) fn get_agent_filesystem_mount_path(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    load_agent_filesystem_mount_path(app)
}

fn reset_agent_filesystem_workspace(memory: &mut hstack_agent::memory::WorkingMemory) {
    memory.workspace.filesystem_cwd = VirtualPath::root();
    memory.workspace.filesystem_mount_host_path = None;
    memory.workspace.file_tree = hstack_agent::workspace::FileTreeApp::default();
    memory.workspace.editor = hstack_agent::workspace::EditorApp::default();
    memory.workspace.file_search = hstack_agent::workspace::FilesystemSearchApp::default();
    memory.workspace.jobs = hstack_agent::workspace::JobApp::default();
}

pub(crate) fn sync_agent_filesystem_mount_into_memory(
    memory: &mut hstack_agent::memory::WorkingMemory,
    mounted_root: Option<&Path>,
) -> Result<(), String> {
    reset_agent_filesystem_workspace(memory);

    let Some(mounted_root) = mounted_root else {
        return Ok(());
    };

    let backend = LocalSandboxedFilesystem::new(
        mounted_root.to_path_buf(),
        FilesystemPolicy::project_sandbox(VirtualPath::root()),
    )
    .map_err(|e| format!("Failed to initialize mounted filesystem root: {e}"))?;
    let entries = backend
        .list_dir(&VirtualPath::root(), None)
        .map_err(|e| format!("Failed to list mounted filesystem root: {e}"))?;

    memory.workspace.filesystem_mount_host_path = Some(mounted_root.to_string_lossy().to_string());
    memory.workspace.file_tree.lifecycle = AppLifecycle::OpenMounted;
    memory.workspace.file_tree.cwd = VirtualPath::root();
    memory.workspace.file_tree.entries = entries;
    Ok(())
}

fn mount_state_from_path(path: Option<PathBuf>) -> AgentFilesystemMountState {
    AgentFilesystemMountState {
        host_path: path.map(|mounted| mounted.to_string_lossy().to_string()),
        folder_picker_supported: agent_folder_picker_supported(),
    }
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Result<UserSettings, String> {
    let store = match app.store("settings.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Settings store failure: {e}")),
    };

    let settings_val = match store.get("user_settings") {
        Some(val) => val,
        None => serde_json::json!(UserSettings::default()),
    };

    match serde_json::from_value(settings_val) {
        Ok(s) => Ok(s),
        Err(e) => Err(format!("Settings parse failure: {e}")),
    }
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: UserSettings) -> Result<(), String> {
    let store = match app.store("settings.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Settings store failure: {e}")),
    };

    store.set("user_settings", serde_json::json!(settings));
    match store.save() {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Settings save failure: {e}")),
    }
}

#[tauri::command]
pub async fn upsert_provider(
    app: AppHandle,
    provider: SavedProvider,
    api_key: Option<String>,
) -> Result<(), String> {
    if let Some(key) = api_key {
        SecureStore::set_key(&app, &provider.id, &key)?;
    }

    let mut settings = get_settings(app.clone()).await?;

    if let Some(pos) = settings.providers.iter().position(|p| p.id == provider.id) {
        settings.providers[pos] = provider.clone();
    } else {
        settings.providers.push(provider.clone());
    }

    if settings.default_provider_id.is_none() {
        settings.default_provider_id = Some(provider.id.clone());
    }

    save_settings(app, settings).await
}

#[tauri::command]
pub async fn delete_provider(app: AppHandle, id: String) -> Result<(), String> {
    let _ = SecureStore::delete_key(&app, &id);

    let mut settings = get_settings(app.clone()).await?;
    settings.providers.retain(|p| p.id != id);

    if settings.default_provider_id.as_deref() == Some(&id) {
        settings.default_provider_id = settings.providers.first().map(|p| p.id.clone());
    }

    save_settings(app, settings).await
}

pub(crate) async fn append_pending_action(
    app: &AppHandle,
    action_type: SyncActionType,
    entity_id: String,
    entity_type: String,
    payload: Option<TicketPayload>,
    status: Option<TicketStatus>,
    notes: Option<String>,
) -> Result<(), String> {
    let store = match app.store("pending_actions.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("History store failure: {e}")),
    };

    let mut actions: Vec<SyncAction> = match store.get("pending") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => Vec::new(),
    };

    actions.push(SyncAction {
        action_id: Uuid::new_v4().to_string(),
        r#type: action_type,
        entity_id,
        entity_type,
        status,
        payload,
        notes,
        timestamp: Utc::now().to_rfc3339(),
    });

    if actions.len() > 50 {
        actions.drain(0..actions.len() - 50);
    }

    store.set("pending", serde_json::json!(actions));
    match store.save() {
        Ok(_) => {
            let _ = app.emit(SYNC_TICKETS_CHANGED_EVENT, serde_json::json!({}));
            Ok(())
        }
        Err(e) => Err(format!("Failed to save pending action: {e}")),
    }
}

pub(crate) async fn load_tickets_state_raw(app: AppHandle) -> Result<(Vec<Ticket>, Vec<SyncAction>), String> {
    let base_store = match app.store("base_state.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Base state store failure: {e}")),
    };
    let base_tickets: Vec<Ticket> = match base_store.get("tickets") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => Vec::new(),
    };

    let pending_store = match app.store("pending_actions.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Pending actions store failure: {e}")),
    };
    let pending_actions: Vec<SyncAction> = match pending_store.get("pending") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => Vec::new(),
    };

    Ok((base_tickets, pending_actions))
}

pub(crate) async fn load_tickets_state(app: AppHandle) -> Result<Vec<Ticket>, String> {
    let (base_tickets, pending_actions) = load_tickets_state_raw(app).await?;
    Ok(normalize_projected_tickets(project_state(base_tickets, &pending_actions)))
}

#[tauri::command]
pub async fn get_tickets(app: AppHandle) -> Result<Vec<Ticket>, String> {
    load_tickets_state(app).await
}

pub(crate) async fn load_agent_memory(app: AppHandle) -> Result<hstack_agent::memory::WorkingMemory, String> {
    let store = match app.store("agent_memory.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Agent memory store failure: {e}")),
    };
    match store.get("memory") {
        Some(val) => serde_json::from_value(val).map_err(|e| format!("Failed to parse agent memory: {e}")),
        None => Ok(hstack_agent::memory::WorkingMemory::default()),
    }
}

pub(crate) async fn save_agent_memory(app: AppHandle, memory: hstack_agent::memory::WorkingMemory) -> Result<(), String> {
    let store = match app.store("agent_memory.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Agent memory store failure: {e}")),
    };
    store.set("memory", serde_json::json!(memory));
    store.save().map_err(|e| format!("Failed to save agent memory: {e}"))
}

#[tauri::command]
pub async fn get_agent_proposals(app: AppHandle) -> Result<Vec<SyncAction>, String> {
    let memory = load_agent_memory(app).await?;
    Ok(memory.proposed_stack_actions)
}

#[tauri::command]
pub async fn get_agent_filesystem_mount(app: AppHandle) -> Result<AgentFilesystemMountState, String> {
    let path = load_agent_filesystem_mount_path(&app)?;
    Ok(mount_state_from_path(path))
}

#[tauri::command]
pub async fn pick_agent_filesystem_mount(app: AppHandle) -> Result<AgentFilesystemMountState, String> {
    if !agent_folder_picker_supported() {
        return Err("Directory mounting is not supported on this platform yet".to_string());
    }

    use tauri_plugin_dialog::{DialogExt, FilePath};

    let selected = app.dialog().file().blocking_pick_folder();
    let Some(selected) = selected else {
        let existing = load_agent_filesystem_mount_path(&app)?;
        return Ok(mount_state_from_path(existing));
    };

    let chosen = match selected {
        FilePath::Path(path) => path,
        FilePath::Url(url) => {
            return Err(format!(
                "Directory mounting requires a local filesystem path, got URI '{url}'"
            ))
        }
    };

    let canonical = std::fs::canonicalize(&chosen)
        .map_err(|e| format!("Failed to resolve selected directory: {e}"))?;
    if !canonical.is_dir() {
        return Err("Selected mount is not a directory".to_string());
    }

    save_agent_filesystem_mount_path(&app, Some(canonical.as_path()))?;
    let mut memory = load_agent_memory(app.clone()).await?;
    sync_agent_filesystem_mount_into_memory(&mut memory, Some(canonical.as_path()))?;
    save_agent_memory(app.clone(), memory).await?;

    let _ = app.emit("AGENT_WORKSPACE_SYNC", ());
    let _ = app.emit("AGENT_FILESYSTEM_MOUNT_SYNC", ());
    Ok(mount_state_from_path(Some(canonical)))
}

#[tauri::command]
pub async fn clear_agent_filesystem_mount(app: AppHandle) -> Result<(), String> {
    save_agent_filesystem_mount_path(&app, None)?;
    let mut memory = load_agent_memory(app.clone()).await?;
    sync_agent_filesystem_mount_into_memory(&mut memory, None)?;
    save_agent_memory(app.clone(), memory).await?;
    let _ = app.emit("AGENT_WORKSPACE_SYNC", ());
    let _ = app.emit("AGENT_FILESYSTEM_MOUNT_SYNC", ());
    Ok(())
}

#[tauri::command]
pub async fn get_agent_workspace(app: AppHandle) -> Result<hstack_agent::workspace::WorkspaceState, String> {
    let memory = load_agent_memory(app).await?;
    Ok(memory.workspace)
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct AgentSessionState {
    pub(crate) messages: Vec<hstack_agent::provider::Message>,
}

#[tauri::command]
pub async fn get_agent_session(app: AppHandle) -> Result<AgentSessionState, String> {
    let memory = load_agent_memory(app).await?;
    Ok(AgentSessionState {
        messages: memory.messages,
    })
}

#[tauri::command]
pub async fn accept_agent_proposals(app: AppHandle) -> Result<(), String> {
    let mut memory = load_agent_memory(app.clone()).await?;
    let actions = std::mem::take(&mut memory.proposed_stack_actions);
    save_agent_memory(app.clone(), memory).await?;

    for action in actions {
        append_pending_action(
            &app,
            action.r#type,
            action.entity_id,
            action.entity_type,
            action.payload,
            action.status,
            action.notes,
        ).await?;
    }
    
    let _ = crate::sync_runtime::trigger_flush(&app);
    Ok(())
}

#[tauri::command]
pub async fn reject_agent_proposals(app: AppHandle) -> Result<(), String> {
    let mut memory = load_agent_memory(app.clone()).await?;
    memory.proposed_stack_actions.clear();
    save_agent_memory(app, memory).await
}

pub(crate) async fn apply_sync_update_state(
    app: AppHandle,
    new_base_tickets: Vec<Ticket>,
) -> Result<(), String> {
    let base_store = match app.store("base_state.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Base state store failure: {e}")),
    };
    base_store.set("tickets", serde_json::json!(new_base_tickets));
    let _ = base_store.save();

    let pending_store = match app.store("pending_actions.json") {
        Ok(s) => s,
        Err(e) => return Err(format!("Pending actions store failure: {e}")),
    };

    let pending_actions: Vec<SyncAction> = match pending_store.get("pending") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => Vec::new(),
    };

    let remaining_actions = reconcile_state(&new_base_tickets, pending_actions);

    pending_store.set("pending", serde_json::json!(remaining_actions));
    match pending_store.save() {
        Ok(_) => {
            let _ = app.emit(SYNC_TICKETS_CHANGED_EVENT, serde_json::json!({}));
            Ok(())
        }
        Err(e) => Err(format!("Failed to update pending actions after sync: {e}")),
    }
}

#[tauri::command]
pub async fn apply_sync_update(app: AppHandle, new_base_tickets: Vec<Ticket>) -> Result<(), String> {
    apply_sync_update_state(app, new_base_tickets).await
}

#[tauri::command]
pub async fn get_user_locale(app: AppHandle) -> Result<(String, bool), String> {
    let settings = get_settings(app).await?;
    let locale = settings.locale.unwrap_or_else(|| "en-US".to_string());
    let hour12 = settings.hour12.unwrap_or(true);
    Ok((locale, hour12))
}

pub(crate) async fn load_sync_session(app: AppHandle) -> Result<SyncSessionInfo, String> {
    let settings = get_settings(app.clone()).await?;
    let token = SecureStore::get_key(&app, SYNC_TOKEN_KEY)?;

    if token.is_empty() {
        return Ok(SyncSessionInfo {
            user_id: None,
            user_name: None,
            token: None,
        });
    }

    Ok(SyncSessionInfo {
        user_id: settings.sync_user_id,
        user_name: settings.sync_user_name,
        token: Some(token),
    })
}

#[tauri::command]
pub async fn get_sync_session(app: AppHandle) -> Result<SyncSessionInfo, String> {
    load_sync_session(app).await
}

#[tauri::command]
pub async fn save_sync_session(
    app: AppHandle,
    user_id: i64,
    user_name: String,
    token: String,
) -> Result<(), String> {
    SecureStore::set_key(&app, SYNC_TOKEN_KEY, &token)?;

    let mut settings = get_settings(app.clone()).await?;
    settings.sync_user_id = Some(user_id);
    settings.sync_user_name = Some(user_name);
    save_settings(app, settings).await
}

#[tauri::command]
pub async fn clear_sync_session(app: AppHandle) -> Result<(), String> {
    let _ = SecureStore::delete_key(&app, SYNC_TOKEN_KEY);

    let mut settings = get_settings(app.clone()).await?;
    settings.sync_user_id = None;
    settings.sync_user_name = None;
    save_settings(app, settings).await
}

#[tauri::command]
pub async fn get_voice_secret_status(app: AppHandle) -> Result<VoiceSecretStatus, String> {
    let api_key = SecureStore::get_key(&app, VOICE_DIRECT_API_KEY_KEY)?;
    Ok(VoiceSecretStatus {
        direct_api_key_present: !api_key.trim().is_empty(),
    })
}

#[tauri::command]
pub async fn warm_secure_store(app: AppHandle) -> Result<(), String> {
    let settings = get_settings(app.clone()).await?;
    let mut ids = vec![SYNC_TOKEN_KEY.to_string(), VOICE_DIRECT_API_KEY_KEY.to_string()];
    ids.extend(settings.providers.iter().map(|provider| provider.id.clone()));
    ids.sort();
    ids.dedup();
    SecureStore::warm_keys(&app, &ids)
}

#[tauri::command]
pub async fn save_voice_direct_api_key(app: AppHandle, api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("voice api key must not be empty".to_string());
    }

    SecureStore::set_key(&app, VOICE_DIRECT_API_KEY_KEY, trimmed)
}

#[tauri::command]
pub async fn clear_voice_direct_api_key(app: AppHandle) -> Result<(), String> {
    SecureStore::delete_key(&app, VOICE_DIRECT_API_KEY_KEY)
}

#[tauri::command]
pub async fn complete_onboarding(app: AppHandle, mode: String) -> Result<(), String> {
    let mut settings = get_settings(app.clone()).await?;
    settings.onboarding_complete = true;
    settings.sync_mode = match mode.as_str() {
        "CloudOfficial" => SyncMode::CloudOfficial,
        "CloudCustom" => SyncMode::CloudCustom,
        _ => SyncMode::LocalOnly,
    };
    save_settings(app, settings).await
}
