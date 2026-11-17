pub mod cli;
pub mod core;
pub mod engines;
pub mod host;

use crate::core::logging::log_event;
use crate::core::preferences::AppPreferences;
use crate::core::quick_actions::{parse_flip_url, FlipUrlAction};
use crate::core::startup;
use crate::engines::bridge;
use crate::engines::convert_files;
use crate::engines::create_new::{create_new_item, ensure_templates_dir_initialized};
use host::{
    attach_window_close_guard, commands::record_recent_conversion, monitor, present_wheel_at,
    query_finder_context_or_fallback, resolve_wav_path, show_standard_window, tray, HostState,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Url};

struct UrlConversionBusyGuard {
    state: Arc<HostState>,
}

impl Drop for UrlConversionBusyGuard {
    fn drop(&mut self) {
        self.state.is_converting.store(false, Ordering::Relaxed);
        if let Ok(mut guard) = self.state.active_cancel.lock() {
            *guard = None;
        }
    }
}

fn handle_flip_url_action(app: &AppHandle, state: &Arc<HostState>, action: FlipUrlAction) {
    match action {
        FlipUrlAction::OpenSettings => {
            let _ = show_standard_window(app, state, "settings");
        }
        FlipUrlAction::OpenBatch { files } => {
            if !files.is_empty() {
                if let Ok(mut w) = state.wheel.lock() {
                    if !w.is_open {
                        w.payload.files = files.clone();
                    }
                }
            }
            let _ = show_standard_window(app, state, "batch");
            if !files.is_empty() {
                if let Some(win) = app.get_webview_window("batch") {
                    let _ = win.emit(
                        "batch:files",
                        json!({ "files": files, "results": null }),
                    );
                }
            }
        }
        FlipUrlAction::OpenWheel {
            mode,
            files,
            folder,
        } => {
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                let (resolved_mode, resolved_files, resolved_folder) = if mode == "new" {
                    let target = match folder {
                        Some(f) if Path::new(&f).is_dir() => f,
                        _ => query_finder_context_or_fallback(&prefs).1,
                    };
                    ("new", Vec::new(), Some(target))
                } else if mode == "convert" {
                    let (ctx_files, ctx_folder) = if files.is_empty() {
                        query_finder_context_or_fallback(&prefs)
                    } else {
                        (files, prefs.resolved_fallback_folder().to_string_lossy().into_owned())
                    };
                    ("convert", ctx_files, folder.or(Some(ctx_folder)))
                } else {
                    let (ctx_files, ctx_folder) = if files.is_empty() {
                        query_finder_context_or_fallback(&prefs)
                    } else {
                        (files, prefs.resolved_fallback_folder().to_string_lossy().into_owned())
                    };
                    let eff_mode = if ctx_files.is_empty() {
                        "new"
                    } else {
                        "convert"
                    };
                    (eff_mode, ctx_files, folder.or(Some(ctx_folder)))
                };
                let _ = present_wheel_at(
                    &app_h,
                    &state_arc,
                    resolved_mode,
                    resolved_files,
                    false,
                    resolved_folder,
                    None,
                    None,
                    None,
                );
            });
        }
        FlipUrlAction::CreateNew {
            kind,
            folder,
            no_rename,
        } => {
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                if let Some(kind_id) = kind {
                    let mut prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                    if no_rename {
                        prefs.auto_rename_on_create = false;
                    }
                    let dir_opt = folder.as_deref().map(Path::new);
                    if let Err(err) = create_new_item(&kind_id, dir_opt, &prefs, true) {
                        log_event(
                            "url.create_new.error",
                            Some(json!({ "kind": kind_id, "error": err.to_string() })),
                        );
                        let _ = bridge::feedback("error", None);
                    }
                } else {
                    let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                    let target = match folder {
                        Some(f) if Path::new(&f).is_dir() => f,
                        _ => query_finder_context_or_fallback(&prefs).1,
                    };
                    let _ = present_wheel_at(
                        &app_h,
                        &state_arc,
                        "new",
                        Vec::new(),
                        false,
                        Some(target),
                        None,
                        None,
                        None,
                    );
                }
            });
        }
        FlipUrlAction::Convert { files, formats } => {
            if files.is_empty() {
                return;
            }
            if state
                .is_converting
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                let _ = bridge::feedback("error", None);
                return;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            if let Ok(mut guard) = state.active_cancel.lock() {
                *guard = Some(Arc::clone(&cancel));
            }
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                let _busy = UrlConversionBusyGuard {
                    state: Arc::clone(&state_arc),
                };
                let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                let path_bufs: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
                match convert_files(&path_bufs, &formats, &prefs, &cancel, None) {
                    Ok(results) => {
                        record_recent_conversion(&app_h, &state_arc, &results);
                        let has_err = results.iter().any(|r| r.error.is_some());
                        let _ = bridge::feedback(if has_err { "error" } else { "success" }, None);
                    }
                    Err(err) => {
                        log_event(
                            "url.convert.error",
                            Some(json!({ "error": err.to_string() })),
                        );
                        let _ = bridge::feedback("error", None);
                    }
                }
            });
        }
    }
}

fn handle_opened_urls(app: &AppHandle, state: &Arc<HostState>, urls: &[Url]) {
    let mut file_paths: Vec<String> = Vec::new();
    for url in urls {
        if url.scheme().eq_ignore_ascii_case("flip") {
            match parse_flip_url(url.as_str()) {
                Ok(action) => handle_flip_url_action(app, state, action),
                Err(err) => {
                    log_event(
                        "url.parse.error",
                        Some(json!({ "url": url.as_str(), "error": err })),
                    );
                }
            }
        } else if url.scheme() == "file" {
            if let Ok(path) = url.to_file_path() {
                file_paths.push(path.to_string_lossy().into_owned());
            }
        }
    }

    if !file_paths.is_empty() {
        if file_paths.len() == 1 {
            let p = Path::new(&file_paths[0]);
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            let is_iwork = matches!(ext.as_str(), "pages" | "numbers" | "key");
            if p.is_dir() && !is_iwork {
                handle_flip_url_action(
                    app,
                    state,
                    FlipUrlAction::OpenWheel {
                        mode: "new".to_string(),
                        files: Vec::new(),
                        folder: Some(file_paths[0].clone()),
                    },
                );
                return;
            }
        }
        handle_flip_url_action(
            app,
            state,
            FlipUrlAction::OpenWheel {
                mode: "convert".to_string(),
                files: file_paths,
                folder: None,
            },
        );
    }
}

pub fn run() {
    run_with_options(None, false);
}

pub fn run_with_options(initial_url: Option<String>, is_login_item: bool) {
    if is_login_item && startup::is_gui_instance_running() {
        return;
    }
    let _gui_lock = startup::acquire_gui_lock();

    let mut prefs = AppPreferences::load();
    prefs.sanitize();
    startup::sync_preferences_with_launch_agent(&mut prefs);
    let _ = ensure_templates_dir_initialized(&AppPreferences::templates_dir());
    let host_state = Arc::new(HostState::new(prefs));
    let state_for_setup = Arc::clone(&host_state);
    let state_for_exit = Arc::clone(&host_state);
    let has_initial_url = initial_url.is_some();

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

            if let Some(raw_url) = initial_url.clone() {
                let app_h = app.handle().clone();
                let st = Arc::clone(&state_for_setup);
                if let Ok(parsed) = Url::parse(&raw_url) {
                    handle_opened_urls(&app_h, &st, &[parsed]);
                } else if let Ok(action) = parse_flip_url(&raw_url) {
                    handle_flip_url_action(&app_h, &st, action);
                }
            }

            // Show onboarding automatically on first launch or when permissions are missing
            if !has_initial_url {
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
            }

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
            host::commands::get_quick_actions_status,
            host::commands::install_finder_quick_actions,
        ])
        .build(tauri::generate_context!())
        .expect("error while building FLIP tauri application");

    app.run(move |app_handle, event| {
        match event {
            #[cfg(target_os = "macos")]
            RunEvent::Opened { urls } => {
                handle_opened_urls(app_handle, &state_for_exit, &urls);
            }
            RunEvent::Exit => {
                state_for_exit.shutting_down.store(true, Ordering::Relaxed);
                state_for_exit.send_monitor_command(serde_json::json!({ "cmd": "quit" }));
            }
            _ => {}
        }
    });
}
