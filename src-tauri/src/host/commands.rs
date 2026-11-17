use crate::core::input_gate::{DragTriggerShortcut, IdleShortcut};
use crate::core::naming::conversion_stem;
use crate::core::preferences::{AppPreferences, RecentConversion};
use crate::engines::bridge::{
    self, ClipboardInspect, PermissionsStatus, ThumbnailResult,
};
use crate::engines::create_new::{
    built_in_kinds, create_new_item, ensure_templates_dir_initialized, scan_user_templates,
    CreateNewResult, NewFileKindInfo, UserTemplateInfo,
};
use crate::engines::{convert_files, detect_tools, ConversionResult, DetectedTools, EngineError};
use crate::host::tray::{open_templates_folder_in_finder, reveal_logs_in_finder};
use crate::host::{
    hide_wheel, inspect_paths_with_prefs, present_wheel_at, query_finder_context_or_fallback,
    show_standard_window, HostState, InspectFilesResult, WheelStatePayload,
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

#[tauri::command]
pub async fn execute_conversion(
    paths: Vec<String>,
    formats: Vec<String>,
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
    if let Ok(mut wheel_guard) = state.wheel.lock() {
        if wheel_guard.is_open {
            wheel_guard.acted_epoch = wheel_guard.open_epoch;
        }
    }

    let state_arc = Arc::clone(&*state);
    let _busy_guard = ConversionBusyGuard {
        state: Arc::clone(&state_arc),
    };

    let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let app_for_progress = app.clone();
    let cancel_for_job = Arc::clone(&cancel);

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

    let payload = match outcome {
        Ok(results) => {
            // Record recent conversions in preferences
            if let Ok(mut prefs_guard) = state_arc.prefs.lock() {
                let now = Utc::now().to_rfc3339();
                let mut added = false;
                for r in &results {
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
            }

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
pub async fn get_new_file_kinds(_state: State<'_, Arc<HostState>>) -> Result<NewFileKindsResponse, String> {
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
        NewFileKindsResponse {
            kinds,
            templates,
            clipboard: clip,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_preferences(state: State<'_, Arc<HostState>>) -> Result<AppPreferences, String> {
    state
        .prefs
        .lock()
        .map(|g| g.clone())
        .map_err(|_| "Preferences lock poisoned".to_string())
}

#[tauri::command]
pub fn save_preferences(
    mut prefs: AppPreferences,
    state: State<'_, Arc<HostState>>,
) -> Result<AppPreferences, String> {
    DragTriggerShortcut::parse(Some(&prefs.drag_trigger)).map_err(|e| e.to_string())?;
    IdleShortcut::parse(&prefs.idle_shortcut).map_err(|e| e.to_string())?;
    prefs.sanitize();

    {
        let mut guard = state
            .prefs
            .lock()
            .map_err(|_| "Preferences lock poisoned".to_string())?;
        prefs.onboarding_completed = prefs.onboarding_completed || guard.onboarding_completed;
        if prefs.recent_conversions.is_empty() && !guard.recent_conversions.is_empty() {
            prefs.recent_conversions = guard.recent_conversions.clone();
        }
        prefs.save().map_err(|e| e.to_string())?;
        *guard = prefs.clone();
    }

    state.sync_monitor_config();
    Ok(prefs)
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
        "alignment" | "level" | "generic" | "success" => {
            if haptics_on {
                normalized
            } else {
                return Ok(());
            }
        }
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
    hide_wheel(&app, &*state, &reason);
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
pub fn open_templates_folder() -> Result<(), String> {
    open_templates_folder_in_finder()
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
