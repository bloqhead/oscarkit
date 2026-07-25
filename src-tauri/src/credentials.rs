//! "Save password" support — the OS-native credential store (macOS Keychain,
//! Windows Credential Manager, Linux Secret Service via D-Bus), not a
//! plaintext file. Keyed by screen name, since this app only ever remembers
//! one set of credentials at a time (the last-used account), not a list.

const KEYCHAIN_SERVICE: &str = "com.bloqhead.oscarkit";

fn entry_for(screen_name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, screen_name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_password(screen_name: String, password: String) -> Result<(), String> {
    entry_for(&screen_name)?.set_password(&password).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_saved_password(screen_name: String) -> Result<Option<String>, String> {
    match entry_for(&screen_name)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn delete_saved_password(screen_name: String) -> Result<(), String> {
    match entry_for(&screen_name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
