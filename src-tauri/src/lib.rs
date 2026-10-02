//! guft desktop shell: hardening at startup, a small command layer, and the window.
//!
//! The UI only ever sees what these commands return. Keys, plaintext at rest and
//! the network never cross into the WebView.
mod commands;
mod dto;
#[cfg(desktop)]
mod sandbox;

use std::time::Duration;

use guft_app::{AppOptions, Hub, TorBackend};
use guft_net::NetConfig;
use tauri::{Emitter, Manager, WindowEvent};

pub type Shell = Hub<TorBackend>;

pub struct State {
    pub hub: Shell,
    pub data_dir: std::path::PathBuf,
    /// Where saved files go.
    pub downloads: std::path::PathBuf,
}

/// The hub starts background tasks, so it is built inside the runtime Tauri will use.
fn make_hub(dir: std::path::PathBuf) -> Shell {
    tauri::async_runtime::block_on(async { Hub::new(dir, AppOptions::default(), |d| TorBackend::new(d, NetConfig::default())) })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer makes some Wayland compositors drop the connection
    // ("Error 71 (Protocol error)"). A chat UI doesn't need it, so default it off.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Desktop: confine the process before any secret exists and before the WebView spawns its
    // helpers (they inherit this). On a phone the system already sandboxes the app, and its
    // data folder is only known once the app is running (see `setup`).
    #[cfg(desktop)]
    let state = {
        let dirs = sandbox::Dirs::resolve();
        sandbox::apply(&dirs);
        State { hub: make_hub(dirs.profile.clone()), data_dir: dirs.profile.clone(), downloads: dirs.downloads.clone() }
    };

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.manage(state);

    builder

        // Only the microphone, and only for voice notes (the UI asks when you tap record).
        // Everything else a page could ask for (camera, screen, location, ...) is refused.
        .on_permission_request(|_, kind| match kind {
            tauri::webview::PermissionKind::Microphone => tauri::webview::PermissionResponse::Allow,
            _ => tauri::webview::PermissionResponse::Deny,
        })
        .setup(|tauri_app| {
            #[cfg(mobile)]
            {
                let base = tauri_app.path().app_data_dir()?;
                let profile = base.join("profile");
                std::fs::create_dir_all(&profile)?;
                tauri_app.manage(State { hub: make_hub(profile.clone()), data_dir: profile, downloads: base.join("downloads") });
            }
            if cfg!(debug_assertions) {
                eprintln!("guft: serving {}", if tauri::is_dev() { "the dev server (localhost:5173)" } else { "the bundled UI" });
            }
            // Forward app events to the window; they carry ids, never message text.
            let handle = tauri_app.handle().clone();
            let mut rx = tauri_app.state::<State>().hub.subscribe();
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
                    let _ = tokio::time::timeout(Duration::from_secs(10), state.hub.lock()).await;
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
            commands::room_invitations,
            commands::accept_room_invite,
            commands::decline_room_invite,
            commands::room_invite,
            commands::add_to_room,
            commands::leave_room,
            commands::remove_member,
            commands::rename_room,
            commands::send_room_text,
            commands::send_room_file,
            commands::new_invite,
            commands::add_contact,
            commands::invite_is_room,
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
