use crate::core::logging::global_logger;
use crate::core::preferences::AppPreferences;
use crate::engines::create_new::ensure_templates_dir_initialized;
use crate::host::{
    present_wheel_at, query_finder_context_or_fallback, show_standard_window, HostState,
};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

pub fn build_tray_template_icon() -> Image<'static> {
    let size: usize = 36;
    let mut rgba = vec![0u8; size * size * 4];
    let center = (size as f64 - 1.0) / 2.0;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - center;
            let dy = y as f64 - center;
            let r = (dx * dx + dy * dy).sqrt();

            // Outer segmented radial wheel ring (r in 8.8..15.2) with 6 clean sectors
            let ring_outer_cov = (15.2 - r).clamp(0.0, 1.0);
            let ring_inner_cov = (r - 8.6).clamp(0.0, 1.0);
            let ring_cov = ring_outer_cov.min(ring_inner_cov);

            let angle_deg = (dy.atan2(dx).to_degrees() + 360.0 + 90.0) % 360.0;
            let sector_pos = angle_deg % 60.0;
            let gap_dist = sector_pos.min(60.0 - sector_pos);
            // Smooth 1px anti-aliased gap between the 6 wheel sectors
            let angular_cov = ((gap_dist - 4.0) * (r * std::f64::consts::PI / 180.0)).clamp(0.0, 1.0);

            // Center dot hub (r <= 3.6)
            let hub_cov = (3.8 - r).clamp(0.0, 1.0);

            let alpha = (ring_cov * angular_cov).max(hub_cov);
            let idx = (y * size + x) * 4;
            rgba[idx] = 0;
            rgba[idx + 1] = 0;
            rgba[idx + 2] = 0;
            rgba[idx + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Image::new_owned(rgba, size as u32, size as u32)
}

pub fn open_templates_folder_in_finder() -> Result<(), String> {
    let dir = AppPreferences::templates_dir();
    ensure_templates_dir_initialized(&dir).map_err(|e| e.to_string())?;
    Command::new("/usr/bin/open")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("Failed to open Templates folder: {e}"))?;
    Ok(())
}

pub fn reveal_logs_in_finder() -> Result<(), String> {
    let dir = global_logger().directory();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let events_file = dir.join("events.jsonl");
    if events_file.is_file() {
        Command::new("/usr/bin/open")
            .arg("-R")
            .arg(&events_file)
            .spawn()
            .map_err(|e| format!("Failed to reveal logs: {e}"))?;
    } else {
        Command::new("/usr/bin/open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("Failed to open log directory: {e}"))?;
    }
    Ok(())
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_wheel = MenuItem::with_id(app, "open_wheel", "Open FLIP Wheel", true, None::<&str>)?;
    let new_file = MenuItem::with_id(
        app,
        "new_file",
        "New File in Finder Folder…",
        true,
        None::<&str>,
    )?;
    let batch = MenuItem::with_id(
        app,
        "batch",
        "Batch / Multiple Formats…",
        true,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        "Settings & Shortcuts…",
        true,
        None::<&str>,
    )?;
    let templates = MenuItem::with_id(
        app,
        "templates",
        "Open Templates Folder",
        true,
        None::<&str>,
    )?;
    let logs = MenuItem::with_id(app, "logs", "Reveal Diagnostic Logs", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit FLIP", true, None::<&str>)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &open_wheel,
            &new_file,
            &batch,
            &sep1,
            &settings,
            &templates,
            &logs,
            &sep2,
            &quit,
        ])
        .build()?;

    let icon = build_tray_template_icon();

    TrayIconBuilder::with_id("flip-tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("FLIP — Local File Converter")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let state = Arc::clone(&*app.state::<Arc<HostState>>());
            let id = event.id().as_ref().to_string();
            match id.as_str() {
                "open_wheel" => {
                    let app_h = app.clone();
                    std::thread::spawn(move || {
                        let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                        let (selected, folder) = query_finder_context_or_fallback(&prefs);
                        let mode = if selected.is_empty() { "new" } else { "convert" };
                        let _ = present_wheel_at(
                            &app_h,
                            &state,
                            mode,
                            selected,
                            false,
                            Some(folder),
                            None,
                            None,
                            None,
                        );
                    });
                }
                "new_file" => {
                    let app_h = app.clone();
                    std::thread::spawn(move || {
                        let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                        let (_selected, folder) = query_finder_context_or_fallback(&prefs);
                        let _ = present_wheel_at(
                            &app_h,
                            &state,
                            "new",
                            Vec::new(),
                            false,
                            Some(folder),
                            None,
                            None,
                            None,
                        );
                    });
                }
                "batch" => {
                    let app_h = app.clone();
                    std::thread::spawn(move || {
                        let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                        let (selected, _) = query_finder_context_or_fallback(&prefs);
                        if !selected.is_empty() {
                            if let Ok(mut w) = state.wheel.lock() {
                                if !w.is_open {
                                    w.payload.files = selected.clone();
                                }
                            }
                        }
                        let _ = show_standard_window(&app_h, &state, "batch");
                        if !selected.is_empty() {
                            if let Some(win) = app_h.get_webview_window("batch") {
                                let _ = win.emit(
                                    "batch:files",
                                    serde_json::json!({ "files": selected, "results": null }),
                                );
                            }
                        }
                    });
                }
                "settings" => {
                    let _ = show_standard_window(app, &state, "settings");
                }
                "templates" => {
                    let _ = open_templates_folder_in_finder();
                }
                "logs" => {
                    let _ = reveal_logs_in_finder();
                }
                "quit" => {
                    state.shutting_down.store(true, Ordering::Relaxed);
                    state.send_monitor_command(serde_json::json!({ "cmd": "quit" }));
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_template_icon_has_transparent_background_and_visible_ring() {
        let img = build_tray_template_icon();
        assert_eq!(img.width(), 36);
        assert_eq!(img.height(), 36);
        let rgba = img.rgba();
        assert_eq!(rgba.len(), 36 * 36 * 4);
        // Corner pixel (0,0) must be fully transparent for template image
        assert_eq!(rgba[3], 0);
        // Center pixel (18,18) must be opaque black template pixel
        let center_idx = (18 * 36 + 18) * 4;
        assert!(rgba[center_idx + 3] > 200);
        assert_eq!(rgba[center_idx], 0);
    }
}
