use crate::core::input_gate::{clamp_wheel_placement, ScreenRect};
use crate::core::logging::log_event;
use crate::engines::bridge;
use crate::host::{
    hide_wheel, inspect_paths_with_prefs, present_wheel_at, query_finder_context_or_fallback,
    HostState, WHEEL_WINDOW_SIZE,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitorScreenGeom {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible_x: f64,
    pub visible_y: f64,
    pub visible_width: f64,
    pub visible_height: f64,
    pub scale: f64,
}

/// Centers a `wheel_size x wheel_size` window at `(cursor_x, cursor_y)` while clamping
/// completely inside the active monitor's visible frame (`visibleX/Y/Width/Height`).
pub fn clamp_to_screen(
    cursor_x: f64,
    cursor_y: f64,
    wheel_size: i32,
    screen: &MonitorScreenGeom,
) -> (i32, i32) {
    let area = ScreenRect {
        x: screen.visible_x.round() as i32,
        y: screen.visible_y.round() as i32,
        width: screen.visible_width.round().max(1.0) as i32,
        height: screen.visible_height.round().max(1.0) as i32,
    };
    let size = wheel_size.max(0);
    if area.width >= size && area.height >= size {
        let placed = clamp_wheel_placement(
            cursor_x.round() as i32,
            cursor_y.round() as i32,
            size,
            area,
        );
        return (placed.x, placed.y);
    }
    let max_x = (area.x + area.width - size).max(area.x);
    let max_y = (area.y + area.height - size).max(area.y);
    let x = (cursor_x.round() as i32 - size / 2).clamp(area.x, max_x);
    let y = (cursor_y.round() as i32 - size / 2).clamp(area.y, max_y);
    (x, y)
}

pub fn should_hide_on_drag_end(
    is_open: bool,
    is_drag: bool,
    is_converting: bool,
    current_open_epoch: u64,
    acted_epoch: u64,
    drag_end_epoch: u64,
) -> bool {
    is_open
        && is_drag
        && !is_converting
        && current_open_epoch == drag_end_epoch
        && acted_epoch != drag_end_epoch
}

fn parse_screen(val: &Value) -> Option<MonitorScreenGeom> {
    val.get("screen")
        .and_then(|s| serde_json::from_value::<MonitorScreenGeom>(s.clone()).ok())
}

fn parse_string_list(val: &Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn resync_monitor_wheel_state(state: &HostState) {
    let cur_open = state.wheel.lock().map(|g| g.is_open).unwrap_or(false);
    state.send_monitor_command(json!({ "cmd": "wheel_state", "open": cur_open }));
}

fn handle_monitor_event(app: &AppHandle, state: &Arc<HostState>, raw: &str) {
    let Ok(val) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let Some(ev) = val.get("event").and_then(|v| v.as_str()) else {
        return;
    };

    match ev {
        "ready" => {
            log_event("monitor.ready", Some(val));
            state.sync_monitor_config();
        }
        "ax_status" => {
            log_event("monitor.ax_status", Some(val));
        }
        "drag_start" => {
            let files = parse_string_list(&val, "files");
            if let Ok(mut gate) = state.order_gate.lock() {
                if let (Some(x), Some(y)) = (
                    val.get("x").and_then(|v| v.as_f64()),
                    val.get("y").and_then(|v| v.as_f64()),
                ) {
                    gate.mouse_down(x.round() as i32, y.round() as i32);
                }
            }
            let (is_open, is_drag) = state
                .wheel
                .lock()
                .map(|g| (g.is_open, g.payload.is_drag))
                .unwrap_or((false, false));
            if is_open && is_drag && !files.is_empty() {
                let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                let inspected = inspect_paths_with_prefs(&files, &prefs);
                if let Ok(mut guard) = state.wheel.lock() {
                    guard.payload.files = inspected.files.clone();
                    guard.payload.compatible_formats = inspected.compatible_formats;
                    guard.payload.batch_actions = inspected.batch_actions;
                    guard.payload.availability_message = inspected.availability_message;
                    guard.payload.family = inspected.family;
                }
                if let Some(win) = app.get_webview_window("wheel") {
                    let _ = win.emit("wheel:drag-files", json!({ "files": inspected.files }));
                }
            }
        }
        "drag_end" => {
            if let Ok(mut gate) = state.order_gate.lock() {
                gate.mouse_up();
            }
            let x_opt = val.get("x").and_then(|v| v.as_f64());
            let y_opt = val.get("y").and_then(|v| v.as_f64());
            let x = x_opt.unwrap_or(-1.0);
            let y = y_opt.unwrap_or(-1.0);
            let (is_open, is_drag, wx, wy, open_epoch) = state
                .wheel
                .lock()
                .map(|g| (g.is_open, g.payload.is_drag, g.window_x, g.window_y, g.open_epoch))
                .unwrap_or((false, false, 0, 0, 0));

            if is_open && is_drag && !state.is_converting.load(Ordering::Relaxed) {
                let inside_wheel = x >= (wx as f64 - 24.0)
                    && x <= (wx as f64 + WHEEL_WINDOW_SIZE as f64 + 24.0)
                    && y >= (wy as f64 - 24.0)
                    && y <= (wy as f64 + WHEEL_WINDOW_SIZE as f64 + 24.0);
                if let (Some(px), Some(py)) = (x_opt, y_opt) {
                    let local_x = px - wx as f64;
                    let local_y = py - wy as f64;
                    if let Some(win) = app.get_webview_window("wheel") {
                        let _ = win.emit(
                            "wheel:pointer-release",
                            json!({ "button": 0, "localX": local_x, "localY": local_y }),
                        );
                    }
                }
                let grace = if inside_wheel {
                    Duration::from_millis(380)
                } else {
                    Duration::from_millis(60)
                };
                let app_h = app.clone();
                let state_arc = Arc::clone(state);
                std::thread::spawn(move || {
                    std::thread::sleep(grace);
                    let converting = state_arc.is_converting.load(Ordering::Relaxed);
                    let should_hide = state_arc
                        .wheel
                        .lock()
                        .map(|g| {
                            should_hide_on_drag_end(
                                g.is_open,
                                g.payload.is_drag,
                                converting,
                                g.open_epoch,
                                g.acted_epoch,
                                open_epoch,
                            )
                        })
                        .unwrap_or(false);
                    if should_hide {
                        hide_wheel(&app_h, &state_arc, "drag-ended");
                    }
                });
            }
        }
        "trigger_drag_wheel" => {
            if state.is_converting.load(Ordering::Relaxed) {
                resync_monitor_wheel_state(state);
                return;
            }
            let files = parse_string_list(&val, "files");
            if files.is_empty() {
                resync_monitor_wheel_state(state);
                return;
            }
            let x = val.get("x").and_then(|v| v.as_f64());
            let y = val.get("y").and_then(|v| v.as_f64());
            let screen = parse_screen(&val);
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                if present_wheel_at(
                    &app_h,
                    &state_arc,
                    "convert",
                    files,
                    true,
                    None,
                    x,
                    y,
                    screen.as_ref(),
                )
                .is_err()
                {
                    resync_monitor_wheel_state(&state_arc);
                }
            });
        }
        "trigger_idle_wheel" => {
            if state.is_converting.load(Ordering::Relaxed) {
                return;
            }
            let x = val.get("x").and_then(|v| v.as_f64());
            let y = val.get("y").and_then(|v| v.as_f64());
            let screen = parse_screen(&val);
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                let (selected, folder) = query_finder_context_or_fallback(&prefs);
                let mode = if selected.is_empty() { "new" } else { "convert" };
                let _ = present_wheel_at(
                    &app_h,
                    &state_arc,
                    mode,
                    selected,
                    false,
                    Some(folder),
                    x,
                    y,
                    screen.as_ref(),
                );
            });
        }
        "trigger_mouse_wheel" => {
            if state.is_converting.load(Ordering::Relaxed) {
                return;
            }
            let dragging = val.get("dragging").and_then(|v| v.as_bool()).unwrap_or(false);
            let drag_files = parse_string_list(&val, "files");
            let x = val.get("x").and_then(|v| v.as_f64());
            let y = val.get("y").and_then(|v| v.as_f64());
            let screen = parse_screen(&val);
            let app_h = app.clone();
            let state_arc = Arc::clone(state);
            std::thread::spawn(move || {
                if dragging && !drag_files.is_empty() {
                    let _ = present_wheel_at(
                        &app_h,
                        &state_arc,
                        "convert",
                        drag_files,
                        true,
                        None,
                        x,
                        y,
                        screen.as_ref(),
                    );
                } else {
                    let prefs = state_arc.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                    let (selected, folder) = query_finder_context_or_fallback(&prefs);
                    let mode = if selected.is_empty() { "new" } else { "convert" };
                    let _ = present_wheel_at(
                        &app_h,
                        &state_arc,
                        mode,
                        selected,
                        false,
                        Some(folder),
                        x,
                        y,
                        screen.as_ref(),
                    );
                }
            });
        }
        "mouse_move" => {
            let Some(x) = val.get("x").and_then(|v| v.as_f64()) else {
                return;
            };
            let Some(y) = val.get("y").and_then(|v| v.as_f64()) else {
                return;
            };
            let (is_open, wx, wy) = state
                .wheel
                .lock()
                .map(|g| (g.is_open, g.window_x, g.window_y))
                .unwrap_or((false, 0, 0));
            if is_open {
                let local_x = x - wx as f64;
                let local_y = y - wy as f64;
                if let Some(win) = app.get_webview_window("wheel") {
                    let _ = win.emit(
                        "wheel:pointer-move",
                        json!({ "localX": local_x, "localY": local_y }),
                    );
                }
            }
        }
        "mouse_release" => {
            let button = val.get("button").and_then(|v| v.as_i64()).unwrap_or(2);
            let Some(x) = val.get("x").and_then(|v| v.as_f64()) else {
                return;
            };
            let Some(y) = val.get("y").and_then(|v| v.as_f64()) else {
                return;
            };
            let (is_open, wx, wy) = state
                .wheel
                .lock()
                .map(|g| (g.is_open, g.window_x, g.window_y))
                .unwrap_or((false, 0, 0));
            if is_open {
                let local_x = x - wx as f64;
                let local_y = y - wy as f64;
                if let Some(win) = app.get_webview_window("wheel") {
                    let _ = win.emit(
                        "wheel:pointer-release",
                        json!({ "button": button, "localX": local_x, "localY": local_y }),
                    );
                }
            }
        }
        "escape" => {
            let is_open = state.wheel.lock().map(|g| g.is_open).unwrap_or(false);
            if !is_open {
                return;
            }
            if state.is_converting.load(Ordering::Relaxed) {
                if let Ok(guard) = state.active_cancel.lock() {
                    if let Some(flag) = guard.as_ref() {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                if let Some(win) = app.get_webview_window("wheel") {
                    let _ = win.emit("wheel:escape", json!({ "alreadyHidden": false }));
                }
            } else {
                if let Some(win) = app.get_webview_window("wheel") {
                    let _ = win.emit("wheel:escape", json!({ "alreadyHidden": true }));
                }
                hide_wheel(app, state, "escape-key");
            }
        }
        _ => {}
    }
}

pub fn start_monitor_supervisor(app: AppHandle, state: Arc<HostState>) {
    std::thread::spawn(move || {
        let mut backoff_ms = 500u64;
        while !state.shutting_down.load(Ordering::Relaxed) {
            let bin = match bridge::resolve_bridge_binary() {
                Ok(b) => b,
                Err(e) => {
                    log_event("monitor.bridge_missing", Some(json!({ "error": e.to_string() })));
                    std::thread::sleep(Duration::from_millis(backoff_ms));
                    backoff_ms = (backoff_ms * 2).min(5000);
                    continue;
                }
            };

            let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
            let wav = state.wav_path.lock().ok().and_then(|g| g.clone());

            let mut cmd = Command::new(&bin);
            cmd.arg("monitor")
                .arg("--drag-key")
                .arg(&prefs.drag_trigger)
                .arg("--idle-shortcut")
                .arg(&prefs.idle_shortcut)
                .arg("--mouse-button")
                .arg(
                    prefs
                        .mouse_trigger_button
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "-1".to_string()),
                );
            if let Some(w) = wav {
                cmd.arg("--wav").arg(w);
            }
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    log_event("monitor.spawn_error", Some(json!({ "error": e.to_string() })));
                    std::thread::sleep(Duration::from_millis(backoff_ms));
                    backoff_ms = (backoff_ms * 2).min(5000);
                    continue;
                }
            };

            backoff_ms = 500;
            let (tx, rx) = mpsc::channel::<String>();
            if let Ok(mut guard) = state.monitor_tx.lock() {
                *guard = Some(tx);
            }

            let stdin_opt = child.stdin.take();
            let writer_thread = std::thread::spawn(move || {
                let Some(mut stdin) = stdin_opt else { return };
                for line in rx {
                    if writeln!(stdin, "{line}").is_err() || stdin.flush().is_err() {
                        break;
                    }
                }
            });

            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if state.shutting_down.load(Ordering::Relaxed) {
                        break;
                    }
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        handle_monitor_event(&app, &state, trimmed);
                    }
                }
            }

            if let Ok(mut guard) = state.monitor_tx.lock() {
                *guard = None;
            }
            let _ = child.kill();
            let status = child.wait();
            let _ = writer_thread.join();

            if !state.shutting_down.load(Ordering::Relaxed) {
                log_event(
                    "monitor.exited",
                    Some(json!({
                        "code": status.ok().and_then(|s| s.code()),
                    })),
                );
                std::thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms * 2).min(5000);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_to_screen_keeps_wheel_inside_primary_and_secondary_visible_frames() {
        let primary = MonitorScreenGeom {
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

        // Centered cursor needs no clamping
        assert_eq!(clamp_to_screen(720.0, 450.0, 440, &primary), (500, 230));

        // Near top-left menu bar clamps cleanly below visibleY=25
        assert_eq!(clamp_to_screen(12.0, 15.0, 440, &primary), (0, 25));

        // Near bottom-right dock corner clamps inside visible bounds
        assert_eq!(
            clamp_to_screen(1430.0, 895.0, 440, &primary),
            (1440 - 440, 25 + 875 - 440)
        );

        // Secondary display positioned to the left (negative X) & higher DPI scale
        let secondary_left = MonitorScreenGeom {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            visible_x: -1920.0,
            visible_y: 25.0,
            visible_width: 1920.0,
            visible_height: 1055.0,
            scale: 1.0,
        };
        assert_eq!(
            clamp_to_screen(-1900.0, 30.0, 440, &secondary_left),
            (-1920, 25)
        );
        assert_eq!(
            clamp_to_screen(-10.0, 1070.0, 440, &secondary_left),
            (-440, 25 + 1055 - 440)
        );
    }

    #[test]
    fn clamp_to_screen_handles_fractional_and_small_screens() {
        let tiny = MonitorScreenGeom {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 360.0,
            visible_x: 0.0,
            visible_y: 25.0,
            visible_width: 400.0,
            visible_height: 335.0,
            scale: 2.0,
        };
        assert_eq!(clamp_to_screen(200.0, 180.0, 440, &tiny), (0, 25));
    }

    #[test]
    fn drag_end_grace_respects_acted_and_open_epochs() {
        // Normal un-acted drag ends -> should hide
        assert!(should_hide_on_drag_end(true, true, false, 3, 2, 3));
        // Still converting -> must not hide
        assert!(!should_hide_on_drag_end(true, true, true, 3, 3, 3));
        // Fast (<220ms) conversion finished or errored in this wheel session -> must NOT hide wheel!
        assert!(!should_hide_on_drag_end(true, true, false, 3, 3, 3));
        // Wheel re-opened since drag_end was scheduled -> stale timer must NOT hide new wheel!
        assert!(!should_hide_on_drag_end(true, true, false, 4, 3, 3));
        // Idle mode -> drag_end must not hide
        assert!(!should_hide_on_drag_end(true, false, false, 3, 2, 3));
    }
}
