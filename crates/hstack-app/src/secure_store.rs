use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;

#[cfg(not(mobile))]
use keyring::Entry;
#[cfg(mobile)]
use tauri_plugin_store::StoreExt;

static KEY_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
#[cfg(mobile)]
const MOBILE_STORE_PATH: &str = "secure_store.json";
#[cfg(mobile)]
const MOBILE_STORE_KEY: &str = "entries";

fn get_cache() -> &'static Mutex<HashMap<String, String>> {
    KEY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct SecureStore;

impl SecureStore {
    const SERVICE_NAME: &'static str = "hstack-llm-service";
    #[cfg(not(mobile))]
    const DESKTOP_BUNDLE_ACCOUNT: &'static str = "hstack-secure-store-v1";

    #[cfg(not(mobile))]
    fn desktop_entry(account: &str) -> Result<Entry, String> {
        Entry::new(Self::SERVICE_NAME, account)
            .map_err(|e| format!("OS Keychain access error: {e}"))
    }

    #[cfg(not(mobile))]
    fn load_desktop_bundle_entries() -> Result<Option<HashMap<String, String>>, String> {
        let entry = Self::desktop_entry(Self::DESKTOP_BUNDLE_ACCOUNT)?;

        match entry.get_password() {
            Ok(raw) => serde_json::from_str::<HashMap<String, String>>(&raw)
                .map(Some)
                .map_err(|e| format!("Failed to parse bundled secret store from OS Keychain: {e}")),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to retrieve bundled secret store from OS Keychain: {e}")),
        }
    }

    #[cfg(not(mobile))]
    fn save_desktop_bundle_entries(entries: &HashMap<String, String>) -> Result<(), String> {
        let entry = Self::desktop_entry(Self::DESKTOP_BUNDLE_ACCOUNT)?;
        let serialized = serde_json::to_string(entries)
            .map_err(|e| format!("Failed to serialize bundled secret store: {e}"))?;

        entry
            .set_password(&serialized)
            .map_err(|e| format!("Failed to save bundled secret store to OS Keychain: {e}"))
    }

    #[cfg(not(mobile))]
    fn load_legacy_desktop_key(id: &str) -> Result<Option<String>, String> {
        let entry = Self::desktop_entry(id)?;

        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to retrieve secret from OS Keychain: {e}")),
        }
    }

    #[cfg(not(mobile))]
    fn delete_legacy_desktop_key(id: &str) -> Result<(), String> {
        let entry = Self::desktop_entry(id)?;

        match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Failed to delete secret from OS Keychain: {e}")),
        }
    }

    #[cfg(mobile)]
    fn mobile_entries(app: &AppHandle) -> Result<HashMap<String, String>, String> {
        let store = app
            .store(MOBILE_STORE_PATH)
            .map_err(|e| format!("Secure store open failure: {}", e))?;

        match store.get(MOBILE_STORE_KEY) {
            Some(value) => serde_json::from_value(value)
                .map_err(|e| format!("Secure store parse failure: {}", e)),
            None => Ok(HashMap::new()),
        }
    }

    #[cfg(mobile)]
    fn save_mobile_entries(app: &AppHandle, entries: HashMap<String, String>) -> Result<(), String> {
        let store = app
            .store(MOBILE_STORE_PATH)
            .map_err(|e| format!("Secure store open failure: {}", e))?;

        store.set(MOBILE_STORE_KEY, serde_json::json!(entries));
        store
            .save()
            .map_err(|e| format!("Secure store save failure: {}", e))
    }

    pub fn set_key(_app: &AppHandle, id: &str, key: &str) -> Result<(), String> {
        #[cfg(not(mobile))]
        {
            let mut entries = Self::load_desktop_bundle_entries()?.unwrap_or_default();
            entries.insert(id.to_string(), key.to_string());
            Self::save_desktop_bundle_entries(&entries)?;
        }

        #[cfg(mobile)]
        {
            let mut entries = Self::mobile_entries(_app)?;
            entries.insert(id.to_string(), key.to_string());
            Self::save_mobile_entries(_app, entries)?;
        }
            
        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(id.to_string(), key.to_string());
        }
        Ok(())
    }

    pub fn get_key(_app: &AppHandle, id: &str) -> Result<String, String> {
        if let Ok(cache) = get_cache().lock() {
            if let Some(key) = cache.get(id) {
                return Ok(key.clone());
            }
        }

        #[cfg(not(mobile))]
        {
            let mut bundle_entries = Self::load_desktop_bundle_entries()?.unwrap_or_default();

            if let Some(key) = bundle_entries.get(id) {
                if let Ok(mut cache) = get_cache().lock() {
                    cache.extend(bundle_entries.clone());
                }
                return Ok(key.clone());
            }

            match Self::load_legacy_desktop_key(id)? {
                Some(password) => {
                    bundle_entries.insert(id.to_string(), password.clone());
                    Self::save_desktop_bundle_entries(&bundle_entries)?;

                    if let Ok(mut cache) = get_cache().lock() {
                        cache.extend(bundle_entries);
                    }

                    Ok(password)
                }
                None => Ok("".to_string()),
            }
        }

        #[cfg(mobile)]
        {
            let entries = Self::mobile_entries(_app)?;
            let value = entries.get(id).cloned().unwrap_or_default();

            if let Ok(mut cache) = get_cache().lock() {
                cache.insert(id.to_string(), value.clone());
            }

            Ok(value)
        }
    }

    pub fn delete_key(_app: &AppHandle, id: &str) -> Result<(), String> {
        #[cfg(not(mobile))]
        {
            let mut entries = Self::load_desktop_bundle_entries()?.unwrap_or_default();
            entries.remove(id);
            Self::save_desktop_bundle_entries(&entries)?;
            let _ = Self::delete_legacy_desktop_key(id);
        }

        #[cfg(mobile)]
        {
            let mut entries = Self::mobile_entries(_app)?;
            entries.remove(id);
            Self::save_mobile_entries(_app, entries)?;
        }

        if let Ok(mut cache) = get_cache().lock() {
            cache.remove(id);
        }

        Ok(())
    }

    pub fn warm_keys(_app: &AppHandle, ids: &[String]) -> Result<(), String> {
        #[cfg(not(mobile))]
        {
            let mut bundle_entries = Self::load_desktop_bundle_entries()?.unwrap_or_default();
            let mut changed = false;

            for id in ids {
                if bundle_entries.contains_key(id) {
                    continue;
                }

                if let Some(value) = Self::load_legacy_desktop_key(id)? {
                    bundle_entries.insert(id.clone(), value);
                    let _ = Self::delete_legacy_desktop_key(id);
                    changed = true;
                }
            }

            if changed {
                Self::save_desktop_bundle_entries(&bundle_entries)?;
            }

            if let Ok(mut cache) = get_cache().lock() {
                cache.extend(bundle_entries);
            }

            Ok(())
        }

        #[cfg(mobile)]
        {
            let entries = Self::mobile_entries(_app)?;

            if let Ok(mut cache) = get_cache().lock() {
                cache.extend(entries);
            }

            let _ = ids;
            Ok(())
        }
    }
}
