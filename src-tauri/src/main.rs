//! guft desktop shell: hardening at startup, a small command layer, and the window.
//!
//! The UI only ever sees what these commands return. Keys, plaintext at rest and
//! the network never cross into the WebView.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dto;
mod sandbox;

use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppOptions, TorBackend};
use guft_net::NetConfig;
use tauri::{Emitter, Manager, WindowEvent};

pub type Shell = App<TorBackend>;

pub struct State {
    pub app: Arc<Shell>,
    pub data_dir: std::path::PathBuf,
}

fn main() {
    // WebKitGTK's DMABUF renderer makes some Wayland compositors drop the connection
    // ("Error 71 (Protocol error)"). A chat UI doesn't need it, so default it off.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let dirs = sandbox::Dirs::resolve();
    // Before any secret exists and before the WebView spawns its helpers (they inherit this).
    sandbox::apply(&dirs);

    let backend = TorBackend::new(&dirs.profile, NetConfig::default());
    let app = Arc::new(App::new(dirs.profile.clone(), backend, AppOptions::default()));
    let state = State { app: app.clone(), data_dir: dirs.profile.clone() };

    tauri::Builder::default()
        .manage(state)
        // Only the microphone, and only for voice notes (the UI asks when you tap record).
        // Everything else a page could ask for (camera, screen, location, ...) is refused.
        .on_permission_request(|_, kind| match kind {
            tauri::webview::PermissionKind::Microphone => tauri::webview::PermissionResponse::Allow,
            _ => tauri::webview::PermissionResponse::Deny,
        })
        .setup(move |tauri_app| {
            if cfg!(debug_assertions) {
                eprintln!("guft: serving {}", if tauri::is_dev() { "the dev server (localhost:5173)" } else { "the bundled UI" });
            }
            // Forward app events to the window; they carry ids, never message text.
            let handle = tauri_app.handle().clone();
            let mut rx = app.subscribe();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let _ = handle.emit("guft://event", dto::EventDto::from(event));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if cfg!(debug_assertions) && std::env::var_os("GUFT_DEBUG_EVENTS").is_some() {
                eprintln!("window event: {event:?}");
            }
            // Closing the window locks the vault first, so keys never outlive the UI.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<State>();
                    let _ = tokio::time::timeout(Duration::from_secs(10), state.app.lock()).await;
                    handle.exit(0);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::create_profile,
            commands::unlock,
            commands::lock,
            commands::touch,
            commands::chats,
            commands::rooms,
            commands::messages,
            commands::mark_read,
            commands::send_text,
            commands::send_file,
            commands::create_room,
            commands::start_temp_chat,
            commands::room_invite,
            commands::add_to_room,
            commands::leave_room,
            commands::remove_member,
            commands::rename_room,
            commands::send_room_text,
            commands::send_room_file,
            commands::new_invite,
            commands::add_contact,
            commands::safety_number,
            commands::set_verified,
            commands::remove_contact,
            commands::delete_chat,
            commands::delete_message,
            commands::rename_contact,
            commands::save_file,
            commands::file_bytes,
            commands::set_online_when_locked,
            commands::set_idle_minutes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running guft");
}
