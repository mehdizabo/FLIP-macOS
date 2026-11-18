pub mod cli;
pub mod core;
pub mod engines;
pub mod host;

use crate::core::ipc_socket::{self, IpcCommand, IpcResponse, IpcServerHandle};
use crate::core::logging::log_event;
use crate::core::preferences::AppPreferences;
use crate::core::quick_actions::{parse_flip_url, FlipUrlAction};
use crate::core::startup;
use crate::engines::bridge;
use crate::engines::convert_files;
use crate::engines::create_new::{create_new_item, ensure_templates_dir_initialized};
use host::{
    attach_window_close_guard,
    commands::{record_recent_conversion, spawn_completion_notification},
    configure_wheel_overlay, monitor, present_wheel_at, query_finder_context_or_fallback,
    resolve_wav_path, show_standard_window, tray, wheel_overlay_config, HostState,
    WheelStatePayload,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

fn dispatch_open_batch(
    app: &AppHandle,
    state: &Arc<HostState>,
    files: Vec<String>,
) -> Result<(), String> {
    state.opened_from_url.store(true, Ordering::Relaxed);
    if !files.is_empty() {
        if let Ok(mut w) = state.wheel.lock() {
            if !w.is_open {
                w.payload.files = files.clone();
            }
        }
    }
    show_standard_window(app, state, "batch")?;
    if !files.is_empty() {
        if let Some(win) = app.get_webview_window("batch") {
            let _ = win.emit(
                "batch:files",
                json!({ "files": files, "results": null }),
            );
        }
    }
    Ok(())
}

fn dispatch_open_wheel(
    app: &AppHandle,
    state: &Arc<HostState>,
    mode: &str,
    files: Vec<String>,
    folder: Option<String>,
) -> Result<WheelStatePayload, String> {
    state.opened_from_url.store(true, Ordering::Relaxed);
    let normalized_mode = mode.trim().to_ascii_lowercase();
    if !matches!(normalized_mode.as_str(), "convert" | "new" | "auto" | "") {
        return Err(format!("Invalid wheel mode: {mode}"));
    }
    if let Some(ref f) = folder {
        if !Path::new(f).is_dir() {
            return Err(format!("Target folder does not exist: {f}"));
        }
    }

    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    let (resolved_mode, resolved_files, resolved_folder) = if normalized_mode == "new" {
        let target = match folder {
            Some(f) => f,
            None => query_finder_context_or_fallback(&prefs).1,
        };
        ("new", Vec::new(), Some(target))
    } else if normalized_mode == "convert" {
        let (ctx_files, ctx_folder) = if files.is_empty() {
            query_finder_context_or_fallback(&prefs)
        } else {
            (
                files,
                prefs.resolved_fallback_folder().to_string_lossy().into_owned(),
            )
        };
        ("convert", ctx_files, folder.or(Some(ctx_folder)))
    } else {
        let (ctx_files, ctx_folder) = if files.is_empty() {
            query_finder_context_or_fallback(&prefs)
        } else {
            (
                files,
                prefs.resolved_fallback_folder().to_string_lossy().into_owned(),
            )
        };
        let eff_mode = if ctx_files.is_empty() {
            "new"
        } else {
            "convert"
        };
        (eff_mode, ctx_files, folder.or(Some(ctx_folder)))
    };

    present_wheel_at(
        app,
        state,
        resolved_mode,
        resolved_files,
        false,
        resolved_folder,
        None,
        None,
        None,
    )
}

fn handle_flip_url_action(app: &AppHandle, state: &Arc<HostState>, action: FlipUrlAction) {
    state.opened_from_url.store(true, Ordering::Relaxed);
    match action {
        FlipUrlAction::OpenSettings => {
            let _ = show_standard_window(app, state, "settings");
        }
        FlipUrlAction::OpenBatch { files } => {
            let _ = dispatch_open_batch(app, state, files);
        }
        FlipUrlAction::OpenWheel {
            mode,
            files,
            folder,
        } => {
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                let _ = dispatch_open_wheel(&app_h, &state_arc, &mode, files, folder);
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
                    if let Some(ref f) = folder {
                        if !Path::new(f).is_dir() {
                            log_event(
                                "url.create_new.error",
                                Some(json!({ "kind": kind_id, "error": format!("Target folder does not exist: {f}") })),
                            );
                            let _ = bridge::feedback("error", None);
                            return;
                        }
                    }
                    let mut prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                    if no_rename {
                        prefs.auto_rename_on_create = false;
                    }
                    let dir_opt = folder.as_deref().map(Path::new);
                    match create_new_item(&kind_id, dir_opt, &prefs, true) {
                        Ok(res) => {
                            if prefs.open_after_creating && !Path::new(&res.path).is_dir() {
                                let _ = std::process::Command::new("/usr/bin/open")
                                    .arg(&res.path)
                                    .spawn();
                            }
                        }
                        Err(err) => {
                            log_event(
                                "url.create_new.error",
                                Some(json!({ "kind": kind_id, "error": err.to_string() })),
                            );
                            let _ = bridge::feedback("error", None);
                        }
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
                let started = std::time::Instant::now();
                match convert_files(&path_bufs, &formats, &prefs, &cancel, None) {
                    Ok(results) => {
                        record_recent_conversion(&app_h, &state_arc, &results);
                        let first_err = results.iter().find_map(|r| r.error.as_deref());
                        let _ = bridge::feedback(if first_err.is_some() { "error" } else { "success" }, None);
                        spawn_completion_notification(
                            &prefs,
                            &results,
                            first_err,
                            false,
                            started.elapsed(),
                            true,
                        );
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        log_event(
                            "url.convert.error",
                            Some(json!({ "error": msg })),
                        );
                        let _ = bridge::feedback("error", None);
                        spawn_completion_notification(
                            &prefs,
                            &[],
                            Some(&msg),
                            matches!(err, crate::engines::EngineError::Cancelled),
                            started.elapsed(),
                            true,
                        );
                    }
                }
            });
        }
    }
}

fn handle_opened_urls(app: &AppHandle, state: &Arc<HostState>, urls: &[Url]) {
    if !urls.is_empty() {
        state.opened_from_url.store(true, Ordering::Relaxed);
    }
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

fn handle_ipc_command(app: &AppHandle, state: &Arc<HostState>, cmd: IpcCommand) -> IpcResponse {
    match cmd {
        IpcCommand::Ping => IpcResponse::ok("pong"),
        IpcCommand::Activate => {
            state.opened_from_url.store(true, Ordering::Relaxed);
            for lbl in ["onboarding", "settings", "batch"] {
                if let Some(win) = app.get_webview_window(lbl) {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.unminimize();
                        let _ = win.show();
                        let _ = win.set_focus();
                        return IpcResponse::ok(format!("activated {lbl}"));
                    }
                }
            }
            match dispatch_open_wheel(app, state, "auto", Vec::new(), None) {
                Ok(_) => IpcResponse::ok("activated wheel"),
                Err(err) => IpcResponse::err(err),
            }
        }
        IpcCommand::OpenUrl { url } => {
            if let Ok(parsed) = Url::parse(&url) {
                if parsed.scheme().eq_ignore_ascii_case("flip") {
                    match parse_flip_url(parsed.as_str()) {
                        Ok(action) => {
                            handle_flip_url_action(app, state, action);
                            IpcResponse::ok("opened flip url")
                        }
                        Err(err) => IpcResponse::err(err),
                    }
                } else if parsed.scheme() == "file" {
                    handle_opened_urls(app, state, &[parsed]);
                    IpcResponse::ok("opened file url")
                } else {
                    IpcResponse::err(format!("Unsupported URL scheme: {}", parsed.scheme()))
                }
            } else {
                match parse_flip_url(&url) {
                    Ok(action) => {
                        handle_flip_url_action(app, state, action);
                        IpcResponse::ok("opened flip url")
                    }
                    Err(err) => IpcResponse::err(err),
                }
            }
        }
        IpcCommand::OpenWheel {
            mode,
            files,
            folder,
        } => {
            let eff_mode = mode.as_deref().unwrap_or("auto");
            match dispatch_open_wheel(app, state, eff_mode, files, folder) {
                Ok(_) => IpcResponse::ok("opened wheel"),
                Err(err) => IpcResponse::err(err),
            }
        }
        IpcCommand::OpenSettings => {
            state.opened_from_url.store(true, Ordering::Relaxed);
            match show_standard_window(app, state, "settings") {
                Ok(()) => IpcResponse::ok("opened settings"),
                Err(err) => IpcResponse::err(err),
            }
        }
        IpcCommand::OpenBatch { files } => match dispatch_open_batch(app, state, files) {
            Ok(()) => IpcResponse::ok("opened batch"),
            Err(err) => IpcResponse::err(err),
        },
        IpcCommand::Quit => {
            state.shutting_down.store(true, Ordering::Relaxed);
            state.send_monitor_command(serde_json::json!({ "cmd": "quit" }));
            let app_h = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(25));
                app_h.exit(0);
            });
            IpcResponse::ok("quitting")
        }
    }
}

struct IpcServerCleanupGuard {
    slot: Arc<Mutex<Option<IpcServerHandle>>>,
}

impl Drop for IpcServerCleanupGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.lock() {
            if let Some(mut server) = guard.take() {
                server.stop();
            }
        }
    }
}

fn try_forward_to_running_instance(
    socket_path: &Path,
    initial_url: Option<&str>,
    is_login_item: bool,
) -> bool {
    if is_login_item {
        return startup::is_gui_instance_running() || ipc_socket::is_server_listening(socket_path);
    }

    let cmd = match initial_url {
        Some(url) => IpcCommand::OpenUrl {
            url: url.to_string(),
        },
        None => IpcCommand::Activate,
    };

    if let Ok(resp) = ipc_socket::send_command(socket_path, &cmd) {
        if resp.ok {
            return true;
        }
    }

    if startup::is_gui_instance_running() {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(resp) = ipc_socket::send_command(socket_path, &cmd) {
                if resp.ok {
                    return true;
                }
            }
        }
        return true;
    }

    false
}

pub fn run() {
    run_with_options(None, false);
}

pub fn run_with_options(initial_url: Option<String>, is_login_item: bool) {
    let socket_path = ipc_socket::default_socket_path();
    if try_forward_to_running_instance(&socket_path, initial_url.as_deref(), is_login_item) {
        return;
    }

    let _gui_lock = startup::acquire_gui_lock();
    if _gui_lock.is_none() {
        if !is_login_item {
            let cmd = match initial_url.as_deref() {
                Some(url) => IpcCommand::OpenUrl {
                    url: url.to_string(),
                },
                None => IpcCommand::Activate,
            };
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Ok(resp) = ipc_socket::send_command(&socket_path, &cmd) {
                    if resp.ok {
                        return;
                    }
                }
            }
        }
        return;
    }

    let mut prefs = AppPreferences::load();
    prefs.sanitize();
    startup::sync_preferences_with_launch_agent(&mut prefs);
    let _ = ensure_templates_dir_initialized(&AppPreferences::templates_dir());
    let host_state = Arc::new(HostState::new(prefs));
    let state_for_setup = Arc::clone(&host_state);
    let state_for_exit = Arc::clone(&host_state);
    let has_initial_url = initial_url.is_some();

    let ipc_server_slot: Arc<Mutex<Option<IpcServerHandle>>> = Arc::new(Mutex::new(None));
    let ipc_for_setup = Arc::clone(&ipc_server_slot);
    let ipc_for_exit = Arc::clone(&ipc_server_slot);
    let _ipc_cleanup_guard = IpcServerCleanupGuard {
        slot: Arc::clone(&ipc_server_slot),
    };

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
            tray::start_templates_watcher(app.handle().clone(), Arc::clone(&state_for_setup));

            for label in ["wheel", "settings", "batch", "onboarding"] {
                if let Some(win) = app.get_webview_window(label) {
                    attach_window_close_guard(&win, app.handle(), &state_for_setup);
                    if label == "wheel" {
                        configure_wheel_overlay(&win, wheel_overlay_config(true, false));
                    }
                }
            }

            monitor::start_monitor_supervisor(app.handle().clone(), Arc::clone(&state_for_setup));

            let app_for_ipc = app.handle().clone();
            let state_for_ipc = Arc::clone(&state_for_setup);
            match ipc_socket::start_server(&socket_path, move |cmd| {
                handle_ipc_command(&app_for_ipc, &state_for_ipc, cmd)
            }) {
                Ok(handle) => {
                    if let Ok(mut guard) = ipc_for_setup.lock() {
                        *guard = Some(handle);
                    }
                }
                Err(err) => {
                    log_event(
                        "ipc.server.error",
                        Some(json!({ "error": err.to_string() })),
                    );
                }
            }

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

                    // Give cold-launched CLI socket handoff a brief window before presenting onboarding.
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

                    let opened_url = state_for_onboarding.opened_from_url.load(Ordering::Relaxed);
                    let wheel_open = state_for_onboarding
                        .wheel
                        .lock()
                        .map(|w| w.is_open)
                        .unwrap_or(false);
                    if !opened_url && !wheel_open && (first_launch || missing_perms) {
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
            host::commands::check_for_updates,
            host::commands::open_release_url,
            host::commands::consume_settings_navigation,
            host::commands::get_diagnostics_report,
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
                if let Ok(mut guard) = ipc_for_exit.lock() {
                    if let Some(mut server) = guard.take() {
                        server.stop();
                    }
                }
            }
            _ => {}
        }
    });
}
