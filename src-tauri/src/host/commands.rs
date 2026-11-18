use crate::core::input_gate::{DragTriggerShortcut, IdleShortcut};
use crate::core::logging::{
    collect_macos_sw_vers, format_diagnostics_markdown, global_logger, log_event,
    DiagnosticsReportInput, DIAGNOSTICS_TAIL_LINES,
};
use crate::core::naming::conversion_stem;
use crate::core::preferences::{AppPreferences, RecentConversion};
use crate::core::quick_actions::{self, QuickActionsStatus};
use crate::core::startup;
use crate::core::updater::{self, UpdateCheckResult};
use crate::engines::bridge::{
    self, ClipboardInspect, PermissionsStatus, ThumbnailResult,
};
use crate::engines::create_new::{
    built_in_kinds, create_new_item, ensure_templates_dir_initialized, scan_user_templates,
    CreateNewResult, NewFileKindInfo, UserTemplateInfo,
};
use crate::engines::{convert_files, detect_tools, ConversionResult, DetectedTools, EngineError};
use crate::host::tray::{open_templates_folder_in_finder, reveal_logs_in_finder, sync_tray_and_windows};
use crate::host::{
    hide_wheel, inspect_paths_with_prefs, present_wheel_at, query_finder_context_or_fallback,
    show_standard_window, HostState, InspectFilesResult, SettingsNavigatePayload,
    WheelStatePayload,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFrameworksStatus {
    #[serde(rename = "imageIO")]
    pub image_io: bool,
    pub pdf_kit: bool,
    pub vision_ocr: bool,
    pub av_foundation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusPayload {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub pandoc: Option<PathBuf>,
    pub bridge: Option<PathBuf>,
    pub pages: bool,
    pub numbers: bool,
    pub keynote: bool,
    pub native_frameworks: NativeFrameworksStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFileKindsResponse {
    pub kinds: Vec<NewFileKindInfo>,
    pub templates: Vec<UserTemplateInfo>,
    pub clipboard: ClipboardInspect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionCompletePayload {
    pub success: bool,
    pub results: Vec<ConversionResult>,
    pub error: Option<String>,
}

struct ConversionBusyGuard {
    state: Arc<HostState>,
}

impl Drop for ConversionBusyGuard {
    fn drop(&mut self) {
        self.state.is_converting.store(false, Ordering::Relaxed);
        if let Ok(mut guard) = self.state.active_cancel.lock() {
            *guard = None;
        }
    }
}

#[tauri::command]
pub fn get_wheel_state(state: State<'_, Arc<HostState>>) -> Result<WheelStatePayload, String> {
    state
        .wheel
        .lock()
        .map(|g| g.payload.clone())
        .map_err(|_| "Wheel state lock poisoned".to_string())
}

#[tauri::command]
pub async fn open_wheel_mode(
    mode: String,
    files: Vec<String>,
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<WheelStatePayload, String> {
    if !matches!(mode.as_str(), "convert" | "new") {
        return Err(format!("Invalid wheel mode: {mode}"));
    }
    let state_arc = Arc::clone(&*state);
    tokio::task::spawn_blocking(move || {
        let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
        let (_sel, folder) = query_finder_context_or_fallback(&prefs);
        present_wheel_at(
            &app,
            &state_arc,
            &mode,
            files,
            false,
            Some(folder),
            None,
            None,
            None,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn switch_wheel_mode(
    mode: String,
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<WheelStatePayload, String> {
    let target_mode = match mode.as_str() {
        "convert" | "new" => mode,
        other => return Err(format!("Invalid wheel mode: {other}")),
    };

    let state_arc = Arc::clone(&*state);
    tokio::task::spawn_blocking(move || {
        let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
        let mut target_folder = state_arc
            .wheel
            .lock()
            .map(|g| g.payload.target_folder.clone())
            .unwrap_or_else(|_| prefs.resolved_fallback_folder().to_string_lossy().into_owned());

        if target_mode == "new" {
            let (_sel, fresh_folder) = query_finder_context_or_fallback(&prefs);
            target_folder = fresh_folder;
        }

        let payload = {
            let mut guard = state_arc
                .wheel
                .lock()
                .map_err(|_| "Wheel state lock poisoned".to_string())?;
            guard.payload.mode = target_mode;
            guard.payload.target_folder = target_folder;
            guard.payload.clone()
        };

        if let Some(win) = tauri::Manager::get_webview_window(&app, "wheel") {
            let _ = win.emit("wheel:state", &payload);
        }
        Ok(payload)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn inspect_files(
    paths: Vec<String>,
    state: State<'_, Arc<HostState>>,
) -> Result<InspectFilesResult, String> {
    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    Ok(inspect_paths_with_prefs(&paths, &prefs))
}

#[tauri::command]
pub async fn get_thumbnail(path: String, size: Option<u32>) -> Result<ThumbnailResult, String> {
    let sz = size.unwrap_or(128).clamp(16, 512);
    tokio::task::spawn_blocking(move || bridge::thumbnail(Path::new(&path), sz))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

pub const LONG_CONVERSION_NOTIFY_THRESHOLD_SECS: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationContent {
    pub title: String,
    pub body: String,
    pub reveal_path: Option<PathBuf>,
}

pub fn build_conversion_notification(
    prefs: &AppPreferences,
    results: &[ConversionResult],
    top_level_error: Option<&str>,
    cancelled: bool,
    elapsed: std::time::Duration,
    from_batch_or_tray: bool,
) -> Option<NotificationContent> {
    if !prefs.notifications_enabled || cancelled {
        return None;
    }
    if !from_batch_or_tray && elapsed.as_secs() < LONG_CONVERSION_NOTIFY_THRESHOLD_SECS {
        return None;
    }

    let outputs: Vec<&str> = results
        .iter()
        .flat_map(|r| r.outputs.iter().map(String::as_str))
        .collect();
    let err_count = results.iter().filter(|r| r.error.is_some()).count();

    if outputs.is_empty() && (top_level_error.is_some() || err_count > 0) {
        let msg = top_level_error
            .or_else(|| results.iter().find_map(|r| r.error.as_deref()))
            .unwrap_or("Conversion failed.");
        let short_msg: String = msg.chars().take(180).collect();
        return Some(NotificationContent {
            title: "FLIP Conversion Failed".to_string(),
            body: short_msg,
            reveal_path: None,
        });
    }

    if outputs.is_empty() {
        return None;
    }

    let first_out = PathBuf::from(outputs[0]);
    let first_name = first_out
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("converted file");

    let (title, body) = if err_count > 0 {
        (
            "FLIP Conversion Finished with Errors".to_string(),
            format!("{} succeeded, {} failed", outputs.len(), err_count),
        )
    } else if outputs.len() == 1 {
        (
            "FLIP Conversion Complete".to_string(),
            format!("Saved {first_name}"),
        )
    } else {
        (
            "FLIP Conversion Complete".to_string(),
            format!("Created {} files (e.g. {first_name})", outputs.len()),
        )
    };

    Some(NotificationContent {
        title,
        body,
        reveal_path: Some(first_out),
    })
}

pub fn spawn_completion_notification(
    prefs: &AppPreferences,
    results: &[ConversionResult],
    top_level_error: Option<&str>,
    cancelled: bool,
    elapsed: std::time::Duration,
    from_batch_or_tray: bool,
) {
    let Some(content) = build_conversion_notification(
        prefs,
        results,
        top_level_error,
        cancelled,
        elapsed,
        from_batch_or_tray,
    ) else {
        return;
    };

    std::thread::spawn(move || {
        if let Err(err) = bridge::notify(
            &content.title,
            &content.body,
            content.reveal_path.as_deref(),
        ) {
            log_event("notification.error", Some(json!({ "error": err.to_string() })));
        }
    });
}

#[tauri::command]
pub async fn execute_conversion(
    paths: Vec<String>,
    formats: Vec<String>,
    window: tauri::WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<ConversionCompletePayload, String> {
    if state
        .is_converting
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Another conversion is already in progress.".to_string());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut guard) = state.active_cancel.lock() {
        *guard = Some(Arc::clone(&cancel));
    }
    let from_batch = window.label() == "batch";
    let from_wheel = window.label() == "wheel";
    let wheel_was_open = if let Ok(mut wheel_guard) = state.wheel.lock() {
        if wheel_guard.is_open {
            if from_wheel {
                wheel_guard.acted_epoch = wheel_guard.open_epoch;
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    let state_arc = Arc::clone(&*state);
    let _busy_guard = ConversionBusyGuard {
        state: Arc::clone(&state_arc),
    };

    let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    let prefs_for_notify = prefs.clone();
    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let app_for_progress = app.clone();
    let cancel_for_job = Arc::clone(&cancel);
    let started = std::time::Instant::now();

    let outcome = tokio::task::spawn_blocking(move || {
        convert_files(
            &path_bufs,
            &formats,
            &prefs,
            &cancel_for_job,
            Some(&|percent, label| {
                let _ = app_for_progress.emit(
                    "conversion:progress",
                    json!({
                        "percent": percent,
                        "label": label,
                    }),
                );
            }),
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    let elapsed = started.elapsed();
    let was_cancelled = matches!(outcome, Err(EngineError::Cancelled));

    let payload = match outcome {
        Ok(results) => {
            record_recent_conversion(&app, &state_arc, &results);

            let first_err = results.iter().find_map(|r| r.error.clone());
            let all_ok = first_err.is_none();
            ConversionCompletePayload {
                success: all_ok,
                results,
                error: first_err,
            }
        }
        Err(EngineError::Cancelled) => ConversionCompletePayload {
            success: false,
            results: Vec::new(),
            error: Some("Conversion cancelled".to_string()),
        },
        Err(err) => ConversionCompletePayload {
            success: false,
            results: Vec::new(),
            error: Some(err.to_string()),
        },
    };

    let _ = app.emit("conversion:complete", &payload);
    spawn_completion_notification(
        &prefs_for_notify,
        &payload.results,
        payload.error.as_deref(),
        was_cancelled,
        elapsed,
        from_batch || !wheel_was_open,
    );
    Ok(payload)
}

#[tauri::command]
pub fn cancel_conversion(state: State<'_, Arc<HostState>>) -> Result<(), String> {
    if let Ok(guard) = state.active_cancel.lock() {
        if let Some(flag) = guard.as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn execute_create_new(
    kind_id: String,
    target_folder: Option<String>,
    state: State<'_, Arc<HostState>>,
) -> Result<CreateNewResult, String> {
    let state_arc = Arc::clone(&*state);
    let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();

    let created = tokio::task::spawn_blocking(move || {
        let dir_opt = target_folder
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Path::new);
        create_new_item(&kind_id, dir_opt, &prefs, true).map(|res| (res, prefs.open_after_creating))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let (result, open_after) = created;
    if open_after && !Path::new(&result.path).is_dir() {
        let _ = Command::new("/usr/bin/open").arg(&result.path).spawn();
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_new_file_kinds(
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<NewFileKindsResponse, String> {
    let state_arc = Arc::clone(&*state);
    tokio::task::spawn_blocking(move || {
        let clip = bridge::clipboard_inspect().unwrap_or(ClipboardInspect {
            available: false,
            kind: "empty".to_string(),
            extension: String::new(),
            types: Vec::new(),
        });
        let kinds = built_in_kinds(clip.available);
        let tmpl_dir = AppPreferences::templates_dir();
        let _ = ensure_templates_dir_initialized(&tmpl_dir);
        let templates = scan_user_templates(&tmpl_dir);
        sync_tray_and_windows(&app, &state_arc, &tmpl_dir, false);
        NewFileKindsResponse {
            kinds,
            templates,
            clipboard: clip,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

pub fn record_recent_conversion(
    app: &AppHandle,
    state: &HostState,
    results: &[ConversionResult],
) {
    let added = if let Ok(mut prefs_guard) = state.prefs.lock() {
        let now = Utc::now().to_rfc3339();
        let mut added = false;
        for r in results {
            if r.error.is_none() {
                let source_name = Path::new(&r.source)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(String::from)
                    .unwrap_or_else(|| conversion_stem(Path::new(&r.source)));
                for out in &r.outputs {
                    prefs_guard.push_recent(RecentConversion {
                        source_name: source_name.clone(),
                        target_format: r.format.clone(),
                        output_path: out.clone(),
                        timestamp_utc: now.clone(),
                    });
                    added = true;
                }
            }
        }
        if added {
            let _ = prefs_guard.save();
        }
        added
    } else {
        false
    };

    if added {
        sync_tray_and_windows(app, state, &AppPreferences::templates_dir(), true);
    }
}

#[tauri::command]
pub fn get_preferences(state: State<'_, Arc<HostState>>) -> Result<AppPreferences, String> {
    let mut guard = state
        .prefs
        .lock()
        .map_err(|_| "Preferences lock poisoned".to_string())?;
    startup::sync_preferences_with_launch_agent(&mut guard);
    Ok(guard.clone())
}

pub fn merge_and_validate_preferences(
    mut prefs: AppPreferences,
    existing: &AppPreferences,
) -> Result<AppPreferences, String> {
    DragTriggerShortcut::parse(Some(&prefs.drag_trigger)).map_err(|e| e.to_string())?;
    IdleShortcut::parse(&prefs.idle_shortcut).map_err(|e| e.to_string())?;
    prefs.sanitize();
    prefs.onboarding_completed = prefs.onboarding_completed || existing.onboarding_completed;
    if !existing.recent_conversions.is_empty() {
        prefs.recent_conversions = existing.recent_conversions.clone();
    }
    Ok(prefs)
}

#[tauri::command]
pub fn save_preferences(
    prefs: AppPreferences,
    state: State<'_, Arc<HostState>>,
) -> Result<AppPreferences, String> {
    let saved = {
        let mut guard = state
            .prefs
            .lock()
            .map_err(|_| "Preferences lock poisoned".to_string())?;
        let merged = merge_and_validate_preferences(prefs, &guard)?;
        let desired_login = merged.launch_at_login;
        let actual_login = startup::is_launch_at_login_enabled();
        if desired_login != guard.launch_at_login || desired_login != actual_login {
            startup::set_launch_at_login(desired_login)?;
        }
        merged.save().map_err(|e| e.to_string())?;
        *guard = merged.clone();
        merged
    };

    state.sync_monitor_config();
    Ok(saved)
}

#[tauri::command]
pub async fn get_engine_status(state: State<'_, Arc<HostState>>) -> Result<EngineStatusPayload, String> {
    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    let tools: DetectedTools = tokio::task::spawn_blocking(move || detect_tools(Some(&prefs)))
        .await
        .map_err(|e| e.to_string())?;

    Ok(EngineStatusPayload {
        ffmpeg: tools.ffmpeg,
        ffprobe: tools.ffprobe,
        pandoc: tools.pandoc,
        bridge: tools.bridge,
        pages: tools.pages,
        numbers: tools.numbers,
        keynote: tools.keynote,
        native_frameworks: NativeFrameworksStatus {
            image_io: true,
            pdf_kit: true,
            vision_ocr: true,
            av_foundation: true,
        },
    })
}

#[tauri::command]
pub async fn get_permissions_status() -> Result<PermissionsStatus, String> {
    tokio::task::spawn_blocking(|| bridge::permissions(false, false, false))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn request_permission(
    kind: String,
    state: State<'_, Arc<HostState>>,
) -> Result<PermissionsStatus, String> {
    let state_arc = Arc::clone(&*state);
    let status = tokio::task::spawn_blocking(move || match kind.as_str() {
        "accessibility_prompt" => bridge::permissions(true, false, false),
        "open_accessibility" => bridge::permissions(false, true, false),
        "open_automation" => bridge::permissions(false, false, true),
        "open_input_monitoring" => {
            let val = bridge::run_bridge_json(
                &[
                    "permissions".to_string(),
                    "--open-input-monitoring".to_string(),
                ],
                15,
                None,
            )?;
            serde_json::from_value(val).map_err(|e| bridge::BridgeError::InvalidOutput(e.to_string()))
        }
        other => Err(bridge::BridgeError::CommandFailed(
            "permissions".to_string(),
            format!("Unknown permission kind: {other}"),
        )),
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    state_arc.send_monitor_command(json!({ "cmd": "recheck_permissions" }));
    Ok(status)
}

#[tauri::command]
pub fn play_wheel_feedback(kind: String, state: State<'_, Arc<HostState>>) -> Result<(), String> {
    let (sound_on, haptics_on) = state
        .prefs
        .lock()
        .map(|g| (g.sound_enabled, g.haptics_enabled))
        .unwrap_or((true, true));

    let normalized = kind.trim().to_ascii_lowercase();
    let effective_kind: String = match normalized.as_str() {
        "tick" => {
            if sound_on {
                "tick".to_string()
            } else if haptics_on {
                "alignment".to_string()
            } else {
                return Ok(());
            }
        }
        "error" => {
            if sound_on {
                "error".to_string()
            } else {
                return Ok(());
            }
        }
        "alignment" | "level" | "generic" | "success" if haptics_on => normalized,
        "alignment" | "level" | "generic" | "success" => return Ok(()),
        _ => return Ok(()),
    };

    let has_monitor = state
        .monitor_tx
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    if has_monitor {
        state.send_monitor_command(json!({
            "cmd": "feedback",
            "kind": effective_kind,
        }));
        return Ok(());
    }

    let wav = state.wav_path.lock().ok().and_then(|g| g.clone());
    std::thread::spawn(move || {
        let _ = bridge::feedback(&effective_kind, wav.as_deref());
    });
    Ok(())
}

#[tauri::command]
pub fn dismiss_wheel(
    reason: String,
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<(), String> {
    hide_wheel(&app, &state, &reason);
    Ok(())
}

#[tauri::command]
pub async fn open_window(
    label: String,
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<(), String> {
    let state_arc = Arc::clone(&*state);
    if label == "wheel" {
        tokio::task::spawn_blocking(move || {
            let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
            let (selected, folder) = query_finder_context_or_fallback(&prefs);
            let mode = if selected.is_empty() { "new" } else { "convert" };
            present_wheel_at(
                &app,
                &state_arc,
                mode,
                selected,
                false,
                Some(folder),
                None,
                None,
                None,
            )
            .map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?
    } else {
        show_standard_window(&app, &state_arc, &label)
    }
}

#[tauri::command]
pub fn open_templates_folder(
    app: AppHandle,
    state: State<'_, Arc<HostState>>,
) -> Result<(), String> {
    open_templates_folder_in_finder()?;
    sync_tray_and_windows(&app, &state, &AppPreferences::templates_dir(), false);
    Ok(())
}

#[tauri::command]
pub fn reveal_logs() -> Result<(), String> {
    reveal_logs_in_finder()
}

#[tauri::command]
pub async fn reveal_in_finder(paths: Vec<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&Path> = paths.iter().map(Path::new).collect();
        bridge::finder_reveal(&refs, false)
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_recent_conversions(
    state: State<'_, Arc<HostState>>,
) -> Result<Vec<RecentConversion>, String> {
    state
        .prefs
        .lock()
        .map(|g| g.recent_conversions.clone())
        .map_err(|_| "Preferences lock poisoned".to_string())
}

#[tauri::command]
pub async fn get_quick_actions_status() -> Result<QuickActionsStatus, String> {
    tokio::task::spawn_blocking(|| Ok(quick_actions::get_quick_actions_status()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn install_finder_quick_actions(
    install_cli: Option<bool>,
) -> Result<QuickActionsStatus, String> {
    let with_cli = install_cli.unwrap_or(false);
    tokio::task::spawn_blocking(move || quick_actions::install_quick_actions(with_cli))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn check_for_updates(repo_url: Option<String>) -> Result<UpdateCheckResult, String> {
    tokio::task::spawn_blocking(move || updater::check_github_updates(repo_url.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_release_url(url: String) -> Result<(), String> {
    let validated = updater::validate_release_html_url(&url)?;
    Command::new("/usr/bin/open")
        .arg(&validated)
        .spawn()
        .map_err(|e| format!("Failed to open GitHub release in browser: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn consume_settings_navigation(
    state: State<'_, Arc<HostState>>,
) -> Result<Option<SettingsNavigatePayload>, String> {
    let mut pending_guard = state
        .pending_settings_nav
        .lock()
        .map_err(|_| "Settings navigation lock poisoned".to_string())?;
    state.settings_mounted.store(true, Ordering::SeqCst);
    Ok(pending_guard.take())
}

#[tauri::command]
pub async fn get_diagnostics_report(state: State<'_, Arc<HostState>>) -> Result<String, String> {
    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let macos_version = collect_macos_sw_vers();
        let tools = detect_tools(Some(&prefs));
        let perms_res = bridge::permissions(false, false, false).map_err(|e| e.to_string());
        let log_lines = global_logger().tail_events(DIAGNOSTICS_TAIL_LINES);
        let home = dirs::home_dir();
        format_diagnostics_markdown(&DiagnosticsReportInput {
            app_version: updater::current_version(),
            macos_version: &macos_version,
            tools: &tools,
            permissions: perms_res.as_ref().map_err(|e| e.as_str()),
            prefs: &prefs,
            log_lines: &log_lines,
            home_dir: home.as_deref(),
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn completion_notification_threshold_and_preference_logic() {
        let mut prefs = AppPreferences::default();
        let ok_one = vec![ConversionResult {
            source: "/tmp/photo.heic".to_string(),
            format: "PNG".to_string(),
            outputs: vec!["/tmp/photo.png".to_string()],
            error: None,
        }];

        // Wheel quick conversion (< 3s) stays silent
        assert!(build_conversion_notification(
            &prefs,
            &ok_one,
            None,
            false,
            Duration::from_millis(2999),
            false
        )
        .is_none());

        // Wheel long conversion (>= 3s) triggers notification with output reveal path
        let long_wheel = build_conversion_notification(
            &prefs,
            &ok_one,
            None,
            false,
            Duration::from_secs(3),
            false,
        )
        .unwrap();
        assert_eq!(long_wheel.title, "FLIP Conversion Complete");
        assert_eq!(long_wheel.body, "Saved photo.png");
        assert_eq!(long_wheel.reveal_path, Some(PathBuf::from("/tmp/photo.png")));

        // Batch / Tray quick conversion still notifies when enabled
        let batch_quick = build_conversion_notification(
            &prefs,
            &ok_one,
            None,
            false,
            Duration::from_millis(120),
            true,
        )
        .unwrap();
        assert_eq!(batch_quick.title, "FLIP Conversion Complete");

        // Cancelled job never notifies
        assert!(build_conversion_notification(
            &prefs,
            &ok_one,
            Some("Conversion cancelled"),
            true,
            Duration::from_secs(10),
            true
        )
        .is_none());

        // Multi-output and failure cases
        let multi = vec![
            ConversionResult {
                source: "/tmp/a.png".to_string(),
                format: "JPG".to_string(),
                outputs: vec!["/tmp/a.jpg".to_string(), "/tmp/b.jpg".to_string()],
                error: None,
            },
        ];
        let multi_note = build_conversion_notification(
            &prefs,
            &multi,
            None,
            false,
            Duration::from_secs(4),
            true,
        )
        .unwrap();
        assert!(multi_note.body.contains("Created 2 files"));

        let err_res = vec![ConversionResult {
            source: "/tmp/bad.png".to_string(),
            format: "JPG".to_string(),
            outputs: Vec::new(),
            error: Some("Corrupt image".to_string()),
        }];
        let err_note = build_conversion_notification(
            &prefs,
            &err_res,
            Some("Corrupt image"),
            false,
            Duration::from_secs(4),
            true,
        )
        .unwrap();
        assert_eq!(err_note.title, "FLIP Conversion Failed");
        assert_eq!(err_note.body, "Corrupt image");
        assert!(err_note.reveal_path.is_none());

        // Disabled preference suppresses all notifications
        prefs.notifications_enabled = false;
        assert!(build_conversion_notification(
            &prefs,
            &ok_one,
            None,
            false,
            Duration::from_secs(10),
            true
        )
        .is_none());
    }
}
