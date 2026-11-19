pub mod commands;
pub mod monitor;
pub mod tray;

use crate::core::input_gate::{ShortcutOrderGate, ShortcutPressGate};
use crate::core::logging::log_event;
use crate::core::policy::{
    availability_message, batch_actions, ext_lower, family_for_ext, order_formats, outputs,
    validate_input_files, FormatFamily, PolicyError,
};
use crate::core::preferences::AppPreferences;
use crate::engines::{bridge, detect_tools};
use monitor::{clamp_to_screen, MonitorScreenGeom};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Size, WebviewWindow, WebviewWindowBuilder, WindowEvent};

pub const WHEEL_WINDOW_SIZE: i32 = 440;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelOverlayConfig {
    pub focusable: bool,
    pub order_front: bool,
    pub make_key: bool,
    pub window_level: isize,
    pub hides_on_deactivate: bool,
    pub can_join_all_spaces: bool,
    pub full_screen_auxiliary: bool,
    pub ignores_cycle: bool,
    pub transient: bool,
    pub logical_bounds: Option<(f64, f64, f64, f64)>,
}

pub fn wheel_overlay_config(is_drag: bool, order_front: bool) -> WheelOverlayConfig {
    WheelOverlayConfig {
        focusable: !is_drag,
        order_front,
        make_key: !is_drag,
        window_level: 101, // NSPopUpMenuWindowLevel
        hides_on_deactivate: false,
        can_join_all_spaces: true,
        full_screen_auxiliary: true,
        ignores_cycle: true,
        transient: true,
        logical_bounds: None,
    }
}

pub fn configure_wheel_overlay(win: &WebviewWindow, cfg: WheelOverlayConfig) {
    let _ = win.set_visible_on_all_workspaces(cfg.can_join_all_spaces);
    let _ = win.set_focusable(cfg.focusable);

    #[cfg(target_os = "macos")]
    {
        let _ = win.with_webview(move |webview| {
            use objc2_app_kit::{
                NSApplication, NSColor, NSPopUpMenuWindowLevel, NSScreen, NSWindow,
                NSWindowAnimationBehavior, NSWindowCollectionBehavior,
            };
            use objc2_foundation::{MainThreadMarker, NSPoint, NSSize};
            let ptr = webview.ns_window().cast::<NSWindow>();
            if ptr.is_null() {
                return;
            }
            // SAFETY: `with_webview` runs on the macOS main thread with a live NSWindow handle.
            unsafe {
                let ns_win = &*ptr;
                ns_win.setOpaque(false);
                ns_win.setBackgroundColor(Some(&NSColor::clearColor()));
                ns_win.setHasShadow(false);
                let mut behavior = NSWindowCollectionBehavior::empty();
                if cfg.can_join_all_spaces {
                    behavior |= NSWindowCollectionBehavior::CanJoinAllSpaces;
                }
                if cfg.full_screen_auxiliary {
                    behavior |= NSWindowCollectionBehavior::FullScreenAuxiliary;
                }
                if cfg.ignores_cycle {
                    behavior |= NSWindowCollectionBehavior::IgnoresCycle;
                }
                if cfg.transient {
                    behavior |= NSWindowCollectionBehavior::Transient;
                }
                ns_win.setCollectionBehavior(behavior);
                let level = if cfg.window_level == 101 {
                    NSPopUpMenuWindowLevel
                } else {
                    cfg.window_level
                };
                ns_win.setLevel(level);
                ns_win.setHidesOnDeactivate(cfg.hides_on_deactivate);
                ns_win.setAnimationBehavior(NSWindowAnimationBehavior::None);

                let mtm = MainThreadMarker::new();
                if let (Some((x, y, width, height)), Some(mtm)) = (cfg.logical_bounds, mtm) {
                    if let Some(primary) = NSScreen::screens(mtm).firstObject() {
                        let primary_height = primary.frame().size.height;
                        ns_win.setContentSize(NSSize::new(width, height));
                        ns_win.setFrameTopLeftPoint(NSPoint::new(x, primary_height - y));
                    }
                }

                if cfg.order_front {
                    if cfg.make_key {
                        ns_win.makeKeyAndOrderFront(None);
                    } else {
                        if let Some(mtm) = mtm {
                            let ns_app = NSApplication::sharedApplication(mtm);
                            if ns_app.isHidden() {
                                ns_app.unhideWithoutActivation();
                            }
                        }
                        ns_win.orderFrontRegardless();
                    }
                }
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        if cfg.order_front {
            let _ = win.show();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelStatePayload {
    pub mode: String,
    pub files: Vec<String>,
    pub is_drag: bool,
    pub target_folder: String,
    pub compatible_formats: Vec<String>,
    pub batch_actions: Vec<String>,
    pub origin_x: f64,
    pub origin_y: f64,
    pub availability_message: String,
    pub family: Option<FormatFamily>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectFilesResult {
    pub files: Vec<String>,
    pub compatible_formats: Vec<String>,
    pub batch_actions: Vec<String>,
    pub availability_message: String,
    pub family: Option<FormatFamily>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsNavigatePayload {
    pub tab: String,
    pub check_updates: bool,
}

#[derive(Debug, Clone)]
pub struct WheelRuntimeState {
    pub payload: WheelStatePayload,
    pub is_open: bool,
    pub window_x: i32,
    pub window_y: i32,
    pub open_epoch: u64,
    pub acted_epoch: u64,
}

pub struct HostState {
    pub prefs: Mutex<AppPreferences>,
    pub wheel: Mutex<WheelRuntimeState>,
    pub is_converting: AtomicBool,
    pub active_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub monitor_tx: Mutex<Option<std::sync::mpsc::Sender<String>>>,
    pub shutting_down: AtomicBool,
    pub opened_from_url: AtomicBool,
    pub settings_mounted: AtomicBool,
    pub pending_settings_nav: Mutex<Option<SettingsNavigatePayload>>,
    pub batch_mounted: AtomicBool,
    pub pending_batch_files: Mutex<Option<Vec<String>>>,
    pub wav_path: Mutex<Option<PathBuf>>,
    pub last_tray_templates: Mutex<Vec<tray::TemplateMenuEntry>>,
    pub order_gate: Mutex<ShortcutOrderGate>,
    pub press_gate: Mutex<ShortcutPressGate>,
}

impl HostState {
    pub fn new(prefs: AppPreferences) -> Self {
        let fallback = prefs.resolved_fallback_folder().to_string_lossy().into_owned();
        Self {
            prefs: Mutex::new(prefs),
            wheel: Mutex::new(WheelRuntimeState {
                payload: WheelStatePayload {
                    mode: "convert".to_string(),
                    files: Vec::new(),
                    is_drag: false,
                    target_folder: fallback,
                    compatible_formats: Vec::new(),
                    batch_actions: Vec::new(),
                    origin_x: (WHEEL_WINDOW_SIZE / 2) as f64,
                    origin_y: (WHEEL_WINDOW_SIZE / 2) as f64,
                    availability_message: "Drop local files here".to_string(),
                    family: None,
                },
                is_open: false,
                window_x: 0,
                window_y: 0,
                open_epoch: 0,
                acted_epoch: 0,
            }),
            is_converting: AtomicBool::new(false),
            active_cancel: Mutex::new(None),
            monitor_tx: Mutex::new(None),
            shutting_down: AtomicBool::new(false),
            opened_from_url: AtomicBool::new(false),
            settings_mounted: AtomicBool::new(false),
            pending_settings_nav: Mutex::new(None),
            batch_mounted: AtomicBool::new(false),
            pending_batch_files: Mutex::new(None),
            wav_path: Mutex::new(None),
            last_tray_templates: Mutex::new(Vec::new()),
            order_gate: Mutex::new(ShortcutOrderGate::default()),
            press_gate: Mutex::new(ShortcutPressGate::default()),
        }
    }

    pub fn send_monitor_command(&self, cmd: serde_json::Value) {
        let Ok(guard) = self.monitor_tx.lock() else {
            return;
        };
        if let Some(tx) = guard.as_ref() {
            if let Ok(line) = serde_json::to_string(&cmd) {
                let _ = tx.send(line);
            }
        }
    }

    pub fn sync_monitor_config(&self) {
        let prefs = self.prefs.lock().map(|g| g.clone()).unwrap_or_default();
        let wav = self
            .wav_path
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|p| p.to_string_lossy().into_owned());
        let wheel_open = self.wheel.lock().map(|g| g.is_open).unwrap_or(false);
        self.send_monitor_command(json!({
            "cmd": "set_config",
            "dragKey": prefs.drag_trigger,
            "idleShortcut": prefs.idle_shortcut,
            "mouseButton": prefs.mouse_trigger_button.map(|b| b as i64).unwrap_or(-1),
            "wavPath": wav,
            "wheelOpen": wheel_open,
        }));
    }

    pub fn mark_onboarding_completed(&self) {
        let Ok(mut guard) = self.prefs.lock() else {
            return;
        };
        if !guard.onboarding_completed {
            guard.onboarding_completed = true;
            let _ = guard.save();
        }
    }
}

pub fn resolve_wav_path(app: &AppHandle) -> Option<PathBuf> {
    let mut resource_candidates = Vec::new();
    if let Ok(res) = app.path().resolve("resources/flip-tick.wav", BaseDirectory::Resource) {
        resource_candidates.push(res);
    }
    if let Ok(res) = app.path().resolve("flip-tick.wav", BaseDirectory::Resource) {
        resource_candidates.push(res);
    }
    let current_exe = std::env::current_exe().ok();
    resolve_wav_path_from(
        &resource_candidates,
        current_exe.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn probe_wav_near_exe(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let res_nested = contents.join("Resources").join("resources").join("flip-tick.wav");
    if res_nested.is_file() {
        return Some(res_nested);
    }
    let res_flat = contents.join("Resources").join("flip-tick.wav");
    if res_flat.is_file() {
        return Some(res_flat);
    }
    None
}

pub fn resolve_wav_path_from(
    resource_candidates: &[PathBuf],
    exe_path: Option<&Path>,
    manifest_dir: &Path,
) -> Option<PathBuf> {
    for candidate in resource_candidates {
        if candidate.is_file() {
            return Some(candidate.clone());
        }
    }
    if let Some(exe) = exe_path {
        if let Some(found) = probe_wav_near_exe(exe) {
            return Some(found);
        }
        if let Ok(canon) = std::fs::canonicalize(exe) {
            if canon != exe {
                if let Some(found) = probe_wav_near_exe(&canon) {
                    return Some(found);
                }
            }
        }
    }
    let dev = manifest_dir.join("resources").join("flip-tick.wav");
    if dev.is_file() {
        return Some(dev);
    }
    None
}

pub fn inspect_paths_with_prefs(paths: &[String], prefs: &AppPreferences) -> InspectFilesResult {
    if paths.is_empty() {
        return InspectFilesResult {
            files: Vec::new(),
            compatible_formats: Vec::new(),
            batch_actions: Vec::new(),
            availability_message: "Drop local files here".to_string(),
            family: None,
        };
    }

    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    match validate_input_files(&path_bufs) {
        Ok(valid) => {
            let caps = detect_tools(Some(prefs)).capabilities();
            let raw_outputs = outputs(&valid, &caps);
            let family = valid
                .first()
                .and_then(|p| family_for_ext(&ext_lower(p)));
            let ordered = order_formats(&raw_outputs, family, Some(prefs));
            let batches = batch_actions(&valid);
            let msg = availability_message(&valid, &caps);
            InspectFilesResult {
                files: valid
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                compatible_formats: ordered,
                batch_actions: batches,
                availability_message: msg,
                family,
            }
        }
        Err(PolicyError::InvalidBatchCount) => InspectFilesResult {
            files: paths.to_vec(),
            compatible_formats: Vec::new(),
            batch_actions: Vec::new(),
            availability_message: PolicyError::InvalidBatchCount.to_string(),
            family: None,
        },
        Err(err) => InspectFilesResult {
            files: paths.to_vec(),
            compatible_formats: Vec::new(),
            batch_actions: Vec::new(),
            availability_message: err.to_string(),
            family: None,
        },
    }
}

pub fn attach_window_close_guard(win: &WebviewWindow, app: &AppHandle, state: &Arc<HostState>) {
    let label = win.label().to_string();
    let app_handle = app.clone();
    let state_arc = Arc::clone(state);
    win.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if label == "wheel" {
                hide_wheel(&app_handle, &state_arc, "window-close");
            } else {
                if label == "onboarding" {
                    state_arc.mark_onboarding_completed();
                }
                if let Some(w) = app_handle.get_webview_window(&label) {
                    let _ = w.hide();
                }
            }
        }
    });
}

pub fn ensure_window(
    app: &AppHandle,
    state: &Arc<HostState>,
    label: &str,
) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(label) {
        return Ok(existing);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .cloned()
        .ok_or_else(|| format!("Unknown window label: {label}"))?;

    let built = WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    attach_window_close_guard(&built, app, state);
    if label == "wheel" {
        configure_wheel_overlay(&built, wheel_overlay_config(true, false));
    }
    Ok(built)
}

pub fn show_standard_window(
    app: &AppHandle,
    state: &Arc<HostState>,
    label: &str,
) -> Result<(), String> {
    if label == "settings" {
        if let Some(ob) = app.get_webview_window("onboarding") {
            if ob.is_visible().unwrap_or(false) {
                state.mark_onboarding_completed();
                let _ = ob.hide();
            }
        }
    }
    let win = ensure_window(app, state, label)?;
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

fn fallback_screen_center(app: &AppHandle, win: &WebviewWindow) -> (f64, f64, MonitorScreenGeom) {
    let monitor = win
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let pos = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        let geom = MonitorScreenGeom {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            visible_x: pos.x,
            visible_y: pos.y + 25.0,
            visible_width: size.width,
            visible_height: (size.height - 25.0).max(1.0),
            scale,
        };
        let cx = geom.visible_x + geom.visible_width / 2.0;
        let cy = geom.visible_y + geom.visible_height / 2.0;
        return (cx, cy, geom);
    }
    let geom = MonitorScreenGeom {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
        visible_x: 0.0,
        visible_y: 25.0,
        visible_width: 1440.0,
        visible_height: 875.0,
        scale: 2.0,
    };
    (720.0, 462.0, geom)
}

#[allow(clippy::too_many_arguments)]
pub fn present_wheel_at(
    app: &AppHandle,
    state: &Arc<HostState>,
    mode: &str,
    files: Vec<String>,
    is_drag: bool,
    target_folder: Option<String>,
    cursor_x: Option<f64>,
    cursor_y: Option<f64>,
    screen: Option<&MonitorScreenGeom>,
) -> Result<WheelStatePayload, String> {
    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
    let inspected = if files.is_empty() {
        InspectFilesResult {
            files: Vec::new(),
            compatible_formats: Vec::new(),
            batch_actions: Vec::new(),
            availability_message: "Drop local files here".to_string(),
            family: None,
        }
    } else {
        inspect_paths_with_prefs(&files, &prefs)
    };

    let resolved_folder = target_folder
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| prefs.resolved_fallback_folder().to_string_lossy().into_owned());

    let win = ensure_window(app, state, "wheel")?;
    let (cx, cy, geom) = match (cursor_x, cursor_y, screen) {
        (Some(x), Some(y), Some(s)) => (x, y, *s),
        _ => {
            let (fcx, fcy, fgeom) = fallback_screen_center(app, &win);
            (cursor_x.unwrap_or(fcx), cursor_y.unwrap_or(fcy), screen.copied().unwrap_or(fgeom))
        }
    };

    let (win_x, win_y) = clamp_to_screen(cx, cy, WHEEL_WINDOW_SIZE, &geom);
    let origin_x = cx - win_x as f64;
    let origin_y = cy - win_y as f64;

    let payload = WheelStatePayload {
        mode: mode.to_string(),
        files: inspected.files,
        is_drag,
        target_folder: resolved_folder,
        compatible_formats: inspected.compatible_formats,
        batch_actions: inspected.batch_actions,
        origin_x,
        origin_y,
        availability_message: inspected.availability_message,
        family: inspected.family,
    };

    if let Ok(mut guard) = state.wheel.lock() {
        guard.payload = payload.clone();
        guard.is_open = true;
        guard.window_x = win_x;
        guard.window_y = win_y;
        guard.open_epoch = guard.open_epoch.wrapping_add(1);
    }

    let _ = win.set_size(Size::Logical(LogicalSize::new(
        WHEEL_WINDOW_SIZE as f64,
        WHEEL_WINDOW_SIZE as f64,
    )));
    let _ = win.set_position(Position::Logical(LogicalPosition::new(
        win_x as f64,
        win_y as f64,
    )));
    let mut overlay_cfg = wheel_overlay_config(is_drag, true);
    overlay_cfg.logical_bounds = Some((
        win_x as f64,
        win_y as f64,
        WHEEL_WINDOW_SIZE as f64,
        WHEEL_WINDOW_SIZE as f64,
    ));
    let _ = win.emit("wheel:state", &payload);
    configure_wheel_overlay(&win, overlay_cfg);
    let _ = win.emit(
        "wheel:pointer-move",
        json!({ "localX": origin_x, "localY": origin_y }),
    );
    if !is_drag {
        let _ = win.set_focus();
    }

    state.send_monitor_command(json!({
        "cmd": "wheel_state",
        "open": true
    }));

    log_event(
        "wheel.open",
        Some(json!({
            "mode": payload.mode,
            "isDrag": payload.is_drag,
            "fileCount": payload.files.len(),
            "x": win_x,
            "y": win_y,
        })),
    );

    Ok(payload)
}

pub fn hide_wheel(app: &AppHandle, state: &Arc<HostState>, reason: &str) {
    if let Ok(mut guard) = state.wheel.lock() {
        guard.is_open = false;
        guard.payload.is_drag = false;
    }
    state.send_monitor_command(json!({
        "cmd": "wheel_state",
        "open": false
    }));
    if let Some(win) = app.get_webview_window("wheel") {
        let _ = win.hide();
        let _ = win.set_focusable(false);
        #[cfg(target_os = "macos")]
        {
            let any_standard_visible = ["settings", "batch", "onboarding"].iter().any(|lbl| {
                app.get_webview_window(lbl)
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(false)
            });
            if !any_standard_visible && reason != "create-new-completed" {
                let _ = win.with_webview(|_| {
                    use objc2_app_kit::NSApplication;
                    use objc2_foundation::MainThreadMarker;
                    if let Some(mtm) = MainThreadMarker::new() {
                        let ns_app = NSApplication::sharedApplication(mtm);
                        if ns_app.isActive() {
                            ns_app.hide(None);
                        }
                    }
                });
            }
        }
    }
    log_event("wheel.dismiss", Some(json!({ "reason": reason })));
}

pub fn query_finder_context_or_fallback(prefs: &AppPreferences) -> (Vec<String>, String) {
    match bridge::finder_context() {
        Ok(ctx) => {
            let folder = if !ctx.target_folder.trim().is_empty()
                && Path::new(&ctx.target_folder).is_dir()
            {
                ctx.target_folder
            } else {
                prefs.resolved_fallback_folder().to_string_lossy().into_owned()
            };
            (ctx.selected_files, folder)
        }
        Err(_) => (
            Vec::new(),
            prefs.resolved_fallback_folder().to_string_lossy().into_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn inspect_paths_validates_local_files_and_rejects_directories() {
        let dir = tempdir().unwrap();
        let prefs = AppPreferences::default();

        let empty = inspect_paths_with_prefs(&[], &prefs);
        assert!(empty.compatible_formats.is_empty());
        assert_eq!(empty.availability_message, "Drop local files here");

        let png = dir.path().join("sample.png");
        fs::write(&png, b"fake").unwrap();
        let res = inspect_paths_with_prefs(&[png.to_string_lossy().into_owned()], &prefs);
        assert_eq!(res.files.len(), 1);
        assert!(res.compatible_formats.contains(&"JPG".to_string()));
        assert!(!res.compatible_formats.contains(&"PNG".to_string()));
        assert_eq!(res.family, Some(FormatFamily::Image));
        assert!(res.availability_message.is_empty());

        let dir_res = inspect_paths_with_prefs(&[dir.path().to_string_lossy().into_owned()], &prefs);
        assert!(dir_res.compatible_formats.is_empty());
        assert!(dir_res.availability_message.contains("Directories cannot be converted"));
    }
}
