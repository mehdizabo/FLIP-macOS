pub mod core;
pub mod engines;
pub mod host;

use crate::core::preferences::AppPreferences;
use crate::engines::bridge;
use crate::engines::create_new::ensure_templates_dir_initialized;
use host::{
    attach_window_close_guard, monitor, resolve_wav_path, show_standard_window, tray, HostState,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Manager, RunEvent};

pub fn run() {
    let mut prefs = AppPreferences::load();
    prefs.sanitize();
    let _ = ensure_templates_dir_initialized(&AppPreferences::templates_dir());
    let host_state = Arc::new(HostState::new(prefs));
    let state_for_setup = Arc::clone(&host_state);
    let state_for_exit = Arc::clone(&host_state);

    let app = tauri::Builder::default()
        .manage(host_state)
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if let Some(wav) = resolve_wav_path(app.handle()) {
                if let Ok(mut guard) = state_for_setup.wav_path.lock() {
                    *guard = Some(wav);
                }
            }

            tray::setup_tray(app.handle())?;

            for label in ["wheel", "settings", "batch", "onboarding"] {
                if let Some(win) = app.get_webview_window(label) {
                    attach_window_close_guard(&win, app.handle(), &state_for_setup);
                    if label == "wheel" {
                        let _ = win.set_visible_on_all_workspaces(true);
                    }
                }
            }

            monitor::start_monitor_supervisor(app.handle().clone(), Arc::clone(&state_for_setup));

            // Show onboarding automatically on first launch or when permissions are missing
            let app_handle = app.handle().clone();
            let state_for_onboarding = Arc::clone(&state_for_setup);
            tauri::async_runtime::spawn(async move {
                let first_launch = state_for_onboarding
                    .prefs
                    .lock()
                    .map(|p| !p.onboarding_completed)
                    .unwrap_or(true);
                let missing_perms = tokio::task::spawn_blocking(|| {
                    bridge::permissions(false, false, false)
                        .map(|p| !p.accessibility && p.automation_finder != "granted")
                        .unwrap_or(false)
                })
                .await
                .unwrap_or(false);

                if first_launch || missing_perms {
                    let _ = show_standard_window(&app_handle, &state_for_onboarding, "onboarding");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            host::commands::get_wheel_state,
            host::commands::open_wheel_mode,
            host::commands::switch_wheel_mode,
            host::commands::inspect_files,
            host::commands::get_thumbnail,
            host::commands::execute_conversion,
            host::commands::cancel_conversion,
            host::commands::execute_create_new,
            host::commands::get_new_file_kinds,
            host::commands::get_preferences,
            host::commands::save_preferences,
            host::commands::get_engine_status,
            host::commands::get_permissions_status,
            host::commands::request_permission,
            host::commands::play_wheel_feedback,
            host::commands::dismiss_wheel,
            host::commands::open_window,
            host::commands::open_templates_folder,
            host::commands::reveal_logs,
            host::commands::reveal_in_finder,
            host::commands::get_recent_conversions,
        ])
        .build(tauri::generate_context!())
        .expect("error while building FLIP tauri application");

    app.run(move |_app_handle, event| {
        if let RunEvent::Exit = event {
            state_for_exit.shutting_down.store(true, Ordering::Relaxed);
            state_for_exit.send_monitor_command(serde_json::json!({ "cmd": "quit" }));
        }
    });
}
