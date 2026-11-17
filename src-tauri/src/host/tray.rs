use crate::core::logging::{global_logger, log_event};
use crate::core::preferences::{AppPreferences, RecentConversion};
use crate::engines::bridge;
use crate::engines::create_new::{create_new_item, ensure_templates_dir_initialized};
use crate::host::{
    present_wheel_at, query_finder_context_or_fallback, show_standard_window, HostState,
};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItem, PredefinedMenuItem, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

pub const NEW_FILE_TRAY_ITEMS: &[(&str, &str)] = &[
    ("folder", "Folder"),
    ("txt", "Plain Text (.txt)"),
    ("md", "Markdown (.md)"),
    ("rtf", "Rich Text (.rtf)"),
    ("docx", "Word Document (.docx)"),
    ("xlsx", "Spreadsheet (.xlsx)"),
    ("pptx", "Presentation (.pptx)"),
    ("html", "HTML (.html)"),
    ("json", "JSON (.json)"),
    ("sh", "Shell Script (.sh)"),
    ("clipboard", "From Clipboard"),
];

pub const TRAY_UTILITY_ITEMS: &[(&str, &str)] = &[
    ("settings", "Settings & Shortcuts…"),
    ("check_updates", "Check for Updates…"),
    ("templates", "Open Templates Folder"),
    ("logs", "Reveal Diagnostic Logs"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentMenuEntry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    let count = raw.chars().count();
    if count <= max_chars {
        return raw.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = raw.chars().take(keep).collect();
    out.push('…');
    out
}

pub fn format_recent_menu_label(item: &RecentConversion) -> String {
    let out_name = Path::new(&item.output_path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&item.output_path);
    let full = format!("{} → {} ({})", item.source_name, item.target_format, out_name);
    truncate_chars(&full, 68)
}

pub fn escape_menu_mnemonic(raw: &str) -> String {
    raw.replace('&', "&&")
}

pub fn recent_menu_entries(recent: &[RecentConversion]) -> Vec<RecentMenuEntry> {
    if recent.is_empty() {
        return vec![RecentMenuEntry {
            id: "recent_empty".to_string(),
            label: "No Recent Conversions".to_string(),
            enabled: false,
        }];
    }
    let mut seen = std::collections::HashSet::new();
    recent
        .iter()
        .filter(|item| seen.insert(item.output_path.as_str()))
        .take(10)
        .map(|item| RecentMenuEntry {
            id: format!("recent_path:{}", item.output_path),
            label: format_recent_menu_label(item),
            enabled: true,
        })
        .collect()
}

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
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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

pub fn build_tray_menu(app: &AppHandle, recent: &[RecentConversion]) -> tauri::Result<Menu<Wry>> {
    let open_wheel = MenuItem::with_id(app, "open_wheel", "Open FLIP Wheel", true, None::<&str>)?;

    let mut new_file_builder =
        SubmenuBuilder::with_id(app, "new_file_menu", "New File in Finder Folder")
            .item(&MenuItem::with_id(
                app,
                "new_file_wheel",
                "Open New File Wheel…",
                true,
                None::<&str>,
            )?)
            .separator();

    for (kind_id, label) in NEW_FILE_TRAY_ITEMS {
        let item = MenuItem::with_id(
            app,
            format!("new_kind:{kind_id}"),
            *label,
            true,
            None::<&str>,
        )?;
        new_file_builder = new_file_builder.item(&item);
    }
    let new_file_submenu = new_file_builder.build()?;

    let mut recent_builder = SubmenuBuilder::with_id(app, "recent_menu", "Recent Conversions");
    for entry in recent_menu_entries(recent) {
        let item = MenuItem::with_id(
            app,
            entry.id,
            escape_menu_mnemonic(&entry.label),
            entry.enabled,
            None::<&str>,
        )?;
        recent_builder = recent_builder.item(&item);
    }
    let recent_submenu = recent_builder.build()?;

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
        escape_menu_mnemonic("Settings & Shortcuts…"),
        true,
        None::<&str>,
    )?;
    let check_updates = MenuItem::with_id(
        app,
        "check_updates",
        "Check for Updates…",
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

    MenuBuilder::new(app)
        .items(&[
            &open_wheel,
            &new_file_submenu,
            &recent_submenu,
            &batch,
            &sep1,
            &settings,
            &check_updates,
            &templates,
            &logs,
            &sep2,
            &quit,
        ])
        .build()
}

pub fn refresh_tray_menu(app: &AppHandle, recent: &[RecentConversion]) {
    if let Some(tray) = app.tray_by_id("flip-tray") {
        match build_tray_menu(app, recent) {
            Ok(menu) => {
                if let Err(err) = tray.set_menu(Some(menu)) {
                    log_event(
                        "tray.refresh.error",
                        Some(json!({ "error": err.to_string() })),
                    );
                }
            }
            Err(err) => {
                log_event("tray.build.error", Some(json!({ "error": err.to_string() })));
            }
        }
    }
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let recent = app
        .state::<Arc<HostState>>()
        .prefs
        .lock()
        .map(|g| g.recent_conversions.clone())
        .unwrap_or_default();

    let menu = build_tray_menu(app, &recent)?;
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
            if let Some(kind) = id.strip_prefix("new_kind:") {
                let kind = kind.to_string();
                std::thread::spawn(move || {
                    let prefs = state.prefs.lock().map(|g| g.clone()).unwrap_or_default();
                    match create_new_item(&kind, None, &prefs, true) {
                        Ok(res) => {
                            if prefs.open_after_creating && !Path::new(&res.path).is_dir() {
                                let _ = Command::new("/usr/bin/open").arg(&res.path).spawn();
                            }
                        }
                        Err(err) => {
                            log_event(
                                "tray.create_new.error",
                                Some(json!({ "kind": kind, "error": err.to_string() })),
                            );
                            let _ = bridge::feedback("error", None);
                        }
                    }
                });
                return;
            }

            if let Some(path_str) = id.strip_prefix("recent_path:") {
                let path_str = path_str.to_string();
                std::thread::spawn(move || {
                    let p = Path::new(&path_str);
                    if p.exists() || fs::symlink_metadata(p).is_ok() {
                        if let Err(err) = bridge::finder_reveal(&[p], false) {
                            log_event(
                                "tray.recent.reveal_error",
                                Some(json!({ "path": path_str, "error": err.to_string() })),
                            );
                            let _ = Command::new("/usr/bin/open").arg("-R").arg(p).spawn();
                        }
                    } else if let Some(parent) = p.parent().filter(|d| d.is_dir()) {
                        let _ = Command::new("/usr/bin/open").arg(parent).spawn();
                    } else {
                        log_event("tray.recent.missing", Some(json!({ "path": path_str })));
                        let _ = bridge::feedback("error", None);
                    }
                });
                return;
            }

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
                "new_file" | "new_file_wheel" => {
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
                "check_updates" => {
                    let had_window = app.get_webview_window("settings").is_some();
                    let _ = show_standard_window(app, &state, "settings");
                    let payload = json!({ "tab": "about", "checkUpdates": true });
                    if let Some(win) = app.get_webview_window("settings") {
                        let _ = win.emit("settings:navigate", &payload);
                    }
                    if !had_window {
                        let app_h = app.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(300));
                            if let Some(win) = app_h.get_webview_window("settings") {
                                let _ = win.emit("settings:navigate", &payload);
                            }
                        });
                    }
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

    #[test]
    fn tray_new_file_and_recent_submenu_specs_match_ux_contract() {
        let labels: Vec<&str> = NEW_FILE_TRAY_ITEMS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            vec![
                "Folder",
                "Plain Text (.txt)",
                "Markdown (.md)",
                "Rich Text (.rtf)",
                "Word Document (.docx)",
                "Spreadsheet (.xlsx)",
                "Presentation (.pptx)",
                "HTML (.html)",
                "JSON (.json)",
                "Shell Script (.sh)",
                "From Clipboard",
            ]
        );

        let empty = recent_menu_entries(&[]);
        assert_eq!(empty.len(), 1);
        assert!(!empty[0].enabled);
        assert_eq!(empty[0].label, "No Recent Conversions");

        let items: Vec<RecentConversion> = (0..12)
            .map(|i| RecentConversion {
                source_name: format!("file_{i}.heic"),
                target_format: "PNG".into(),
                output_path: format!("/tmp/file_{i}.png"),
                timestamp_utc: "2026-11-17T20:00:00Z".into(),
            })
            .collect();
        let entries = recent_menu_entries(&items);
        assert_eq!(entries.len(), 10);
        assert!(entries[0].enabled);
        assert_eq!(entries[0].id, "recent_path:/tmp/file_0.png");
        assert!(entries[0].label.contains("file_0.heic → PNG (file_0.png)"));

        // Long UTF-8 filename truncation must stay char-boundary safe
        let long_item = RecentConversion {
            source_name: "è🎉".repeat(30),
            target_format: "WEBP".into(),
            output_path: format!("/tmp/{}.webp", "ä".repeat(30)),
            timestamp_utc: "2026-11-17T20:00:00Z".into(),
        };
        let label = format_recent_menu_label(&long_item);
        assert!(label.ends_with('…'));
        assert!(label.chars().count() <= 68);

        // Ampersands must be escaped for muda macOS mnemonic stripping, and duplicate paths filtered
        assert_eq!(
            escape_menu_mnemonic("Settings & Shortcuts…"),
            "Settings && Shortcuts…"
        );
        assert_eq!(
            escape_menu_mnemonic("R&D.pdf → MD (R&D.md)"),
            "R&&D.pdf → MD (R&&D.md)"
        );
        let dup_items = vec![
            RecentConversion {
                source_name: "R&D.pdf".into(),
                target_format: "MD".into(),
                output_path: "/tmp/R&D.md".into(),
                timestamp_utc: "2026-11-17T20:00:00Z".into(),
            },
            RecentConversion {
                source_name: "R&D.pdf".into(),
                target_format: "MD".into(),
                output_path: "/tmp/R&D.md".into(),
                timestamp_utc: "2026-11-17T19:00:00Z".into(),
            },
        ];
        assert_eq!(recent_menu_entries(&dup_items).len(), 1);
        assert!(TRAY_UTILITY_ITEMS
            .iter()
            .any(|(id, label)| *id == "check_updates" && *label == "Check for Updates…"));
    }
}
