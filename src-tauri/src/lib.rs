mod commands;
mod session_actor;

use std::sync::Mutex;

use commands::SessionState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMA-BUF renderer crashes with "Error 71 (Protocol error)
    // dispatching to Wayland display" on NVIDIA + Wayland — a known
    // WebKitGTK/NVIDIA driver interaction, not something fixable from here.
    // Must be set before the webview initializes (i.e. before the Builder
    // below touches GTK), and only matters on Linux.
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SessionState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::send_message,
            commands::add_buddy,
            commands::remove_buddy,
            commands::set_away_message,
            commands::request_user_info,
            commands::send_warning,
            commands::add_to_block_list,
            commands::remove_from_block_list,
            commands::logout,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
