use flip_lib::core::logging::redact_user_paths;
use flip_lib::core::policy::FormatFamily;
use flip_lib::core::preferences::{expand_tilde_path, AppPreferences, RecentConversion};
use flip_lib::core::quick_actions::QuickActionsStatus;
use flip_lib::core::updater::{
    check_github_updates, is_curl_http_404, validate_release_html_url, UpdateCheckResult,
};
use flip_lib::engines::bridge::{
    resolve_bridge_binary_from, ClipboardInspect, PermissionsStatus, ThumbnailResult,
};
use flip_lib::engines::create_new::{CreateNewResult, NewFileKindInfo, UserTemplateInfo};
use flip_lib::engines::ConversionResult;
use flip_lib::host::commands::{
    merge_and_validate_preferences, ConversionCompletePayload, EngineStatusPayload,
    NativeFrameworksStatus, NewFileKindsResponse,
};
use flip_lib::host::monitor::{clamp_to_screen, should_hide_on_drag_end, MonitorScreenGeom};
use flip_lib::host::{
    inspect_paths_with_prefs, resolve_wav_path_from, wheel_overlay_config, HostState,
    SettingsNavigatePayload, WheelStatePayload,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn tauri_config_and_capabilities_match_window_labels_and_resources() {
    let tauri_conf_raw = fs::read_to_string(manifest_dir().join("tauri.conf.json")).unwrap();
    let tauri_conf: Value = serde_json::from_str(&tauri_conf_raw).unwrap();

    let windows = tauri_conf["app"]["windows"].as_array().unwrap();
    let labels: Vec<&str> = windows
        .iter()
        .filter_map(|w| w["label"].as_str())
        .collect();
    assert_eq!(labels, vec!["wheel", "settings", "batch", "onboarding"]);

    let resources = tauri_conf["bundle"]["resources"].as_object().unwrap();
    assert_eq!(
        resources.get("resources/flip-tick.wav").and_then(|v| v.as_str()),
        Some("resources/flip-tick.wav")
    );
    assert_eq!(
        resources
            .get("../native-bridge/bin/flip-macos-bridge")
            .and_then(|v| v.as_str()),
        Some("flip-macos-bridge")
    );

    let cap_raw = fs::read_to_string(manifest_dir().join("capabilities/default.json")).unwrap();
    let cap: Value = serde_json::from_str(&cap_raw).unwrap();
    let cap_windows: Vec<&str> = cap["windows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(cap_windows, vec!["wheel", "settings", "batch", "onboarding"]);
    let perms: Vec<&str> = cap["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(perms.contains(&"core:default"));

    let wheel_win = windows.iter().find(|w| w["label"] == "wheel").unwrap();
    assert_eq!(wheel_win["visibleOnAllWorkspaces"], true);
    assert_eq!(wheel_win["alwaysOnTop"], true);
    assert_eq!(wheel_win["focus"], false);
    assert_eq!(wheel_win["focusable"], false);

    let drag_cfg = wheel_overlay_config(true, true);
    assert!(!drag_cfg.focusable);
    assert!(!drag_cfg.make_key);
    assert!(drag_cfg.can_join_all_spaces);
    assert!(drag_cfg.full_screen_auxiliary);
    assert!(!drag_cfg.hides_on_deactivate);
    assert_eq!(drag_cfg.window_level, 101);

    let idle_cfg = wheel_overlay_config(false, true);
    assert!(idle_cfg.focusable);
    assert!(idle_cfg.make_key);
    assert!(idle_cfg.can_join_all_spaces);
    assert!(idle_cfg.full_screen_auxiliary);
}

#[test]
fn ipc_payloads_serialize_with_exact_typescript_camel_case_keys() {
    let wheel_payload = WheelStatePayload {
        mode: "convert".into(),
        files: vec!["/tmp/a.png".into()],
        is_drag: true,
        target_folder: "/tmp".into(),
        compatible_formats: vec!["JPG".into()],
        batch_actions: vec![],
        origin_x: 220.0,
        origin_y: 220.0,
        availability_message: String::new(),
        family: Some(FormatFamily::IWork),
    };
    let wv = serde_json::to_value(&wheel_payload).unwrap();
    for key in [
        "mode",
        "files",
        "isDrag",
        "targetFolder",
        "compatibleFormats",
        "batchActions",
        "originX",
        "originY",
        "availabilityMessage",
        "family",
    ] {
        assert!(wv.get(key).is_some(), "WheelStatePayload missing `{key}`");
    }
    assert_eq!(wv["family"], "iwork");

    let engine_status = EngineStatusPayload {
        ffmpeg: Some(PathBuf::from("/opt/homebrew/bin/ffmpeg")),
        ffprobe: Some(PathBuf::from("/opt/homebrew/bin/ffprobe")),
        pandoc: None,
        bridge: Some(PathBuf::from("/Applications/FLIP.app/Contents/Resources/flip-macos-bridge")),
        pages: true,
        numbers: false,
        keynote: false,
        native_frameworks: NativeFrameworksStatus {
            image_io: true,
            pdf_kit: true,
            vision_ocr: true,
            av_foundation: true,
        },
    };
    let ev = serde_json::to_value(&engine_status).unwrap();
    assert_eq!(ev["nativeFrameworks"]["imageIO"], true);
    assert_eq!(ev["nativeFrameworks"]["pdfKit"], true);
    assert_eq!(ev["nativeFrameworks"]["visionOcr"], true);
    assert_eq!(ev["nativeFrameworks"]["avFoundation"], true);

    let new_kinds = NewFileKindsResponse {
        kinds: vec![NewFileKindInfo {
            id: "txt".into(),
            name: "Plain Text".into(),
            extension: Some("txt".into()),
            category: "document".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        }],
        templates: vec![UserTemplateInfo {
            id: "template:/tmp/a.md".into(),
            name: "a".into(),
            group: None,
            extension: Some("md".into()),
            source_path: "/tmp/a.md".into(),
        }],
        clipboard: ClipboardInspect {
            available: false,
            kind: "empty".into(),
            extension: String::new(),
            types: vec![],
        },
    };
    let nkv = serde_json::to_value(&new_kinds).unwrap();
    assert!(nkv["kinds"][0].get("disabledReason").is_some());
    assert!(nkv["templates"][0].get("sourcePath").is_some());
    assert!(nkv["clipboard"].get("available").is_some());

    let create_res = CreateNewResult {
        path: "/tmp/Untitled.txt".into(),
        kind_id: "txt".into(),
        revealed: true,
        rename_triggered: true,
        fallback_used: false,
    };
    let crv = serde_json::to_value(&create_res).unwrap();
    for key in ["path", "kindId", "revealed", "renameTriggered", "fallbackUsed"] {
        assert!(crv.get(key).is_some(), "CreateNewResult missing `{key}`");
    }

    let complete = ConversionCompletePayload {
        success: true,
        results: vec![ConversionResult {
            source: "/tmp/a.png".into(),
            format: "JPG".into(),
            outputs: vec!["/tmp/a.jpg".into()],
            error: None,
        }],
        error: None,
    };
    let cv = serde_json::to_value(&complete).unwrap();
    assert_eq!(cv["success"], true);
    assert!(cv["results"][0].get("outputs").is_some());

    let perms = PermissionsStatus {
        accessibility: true,
        automation_finder: "granted".into(),
        input_monitoring: true,
    };
    let pv = serde_json::to_value(&perms).unwrap();
    assert!(pv.get("automationFinder").is_some());
    assert!(pv.get("inputMonitoring").is_some());

    let qa = QuickActionsStatus {
        services_dir: "/Users/demo/Library/Services".into(),
        convert_installed: true,
        new_file_installed: true,
        convert_path: "/Users/demo/Library/Services/Convert with FLIP.workflow".into(),
        new_file_path: "/Users/demo/Library/Services/New File with FLIP.workflow".into(),
        cli_executable: Some("/Applications/FLIP.app/Contents/MacOS/flip".into()),
        cli_symlink_path: Some("/Users/demo/.local/bin/flip".into()),
    };
    let qav = serde_json::to_value(&qa).unwrap();
    for key in [
        "servicesDir",
        "convertInstalled",
        "newFileInstalled",
        "convertPath",
        "newFilePath",
        "cliExecutable",
        "cliSymlinkPath",
    ] {
        assert!(qav.get(key).is_some(), "QuickActionsStatus missing `{key}`");
    }

    let thumb = ThumbnailResult {
        ok: true,
        png_base64: Some("abc".into()),
        width: Some(128),
        height: Some(128),
        error: None,
    };
    let tv = serde_json::to_value(&thumb).unwrap();
    assert!(tv.get("pngBase64").is_some());

    let mut prefs = AppPreferences::default();
    prefs.recent_conversions.push(RecentConversion {
        source_name: "a.png".into(),
        target_format: "JPG".into(),
        output_path: "/tmp/a.jpg".into(),
        timestamp_utc: "2026-11-17T12:00:00Z".into(),
    });
    let prv = serde_json::to_value(&prefs).unwrap();
    for key in [
        "version",
        "soundEnabled",
        "hapticsEnabled",
        "notificationsEnabled",
        "dragTrigger",
        "idleShortcut",
        "mouseTriggerButton",
        "autoRenameOnCreate",
        "openAfterCreating",
        "launchAtLogin",
        "outputSubfolder",
        "fallbackFolder",
        "imageQuality",
        "preserveExif",
        "stripGpsExif",
        "pdfDpi",
        "pdfPageSize",
        "ffmpegPath",
        "pandocPath",
        "preferredFormats",
        "newFileVisibleKinds",
        "recentConversions",
        "onboardingCompleted",
    ] {
        assert!(prv.get(key).is_some(), "AppPreferences missing `{key}`");
    }
    assert!(prv["recentConversions"][0].get("sourceName").is_some());
    assert!(prv["recentConversions"][0].get("targetFormat").is_some());
    assert!(prv["recentConversions"][0].get("outputPath").is_some());
    assert!(prv["recentConversions"][0].get("timestampUtc").is_some());

    let upd = UpdateCheckResult {
        current_version: "0.1.0".into(),
        latest_tag: "v0.5.1".into(),
        release_name: "FLIP v0.5.1".into(),
        html_url: "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1".into(),
        published_at: Some("2026-10-24T12:00:00Z".into()),
        body: "Release notes".into(),
        update_available: true,
        checked_at_utc: "2026-11-17T22:00:00Z".into(),
    };
    let uv = serde_json::to_value(&upd).unwrap();
    for key in [
        "currentVersion",
        "latestTag",
        "releaseName",
        "htmlUrl",
        "publishedAt",
        "body",
        "updateAvailable",
        "checkedAtUtc",
    ] {
        assert!(uv.get(key).is_some(), "UpdateCheckResult missing `{key}`");
    }

    assert!(check_github_updates(Some("http://insecure.example.com")).is_err());
    assert!(validate_release_html_url("https://github.com/thetinyfoundry/FLIP/releases").is_ok());
    assert!(validate_release_html_url("https://evil.example/x").is_err());
    assert!(is_curl_http_404(
        Some(56),
        "curl: (56) The requested URL returned error: 404"
    ));

    let nav = SettingsNavigatePayload {
        tab: "about".into(),
        check_updates: true,
    };
    let nv = serde_json::to_value(&nav).unwrap();
    assert_eq!(nv["tab"], "about");
    assert_eq!(nv["checkUpdates"], true);

    // Cold tray navigation handoff: pending navigation is queued while settings_mounted == false
    // and consumed once when SettingsView initializes.
    let state = HostState::new(AppPreferences::default());
    assert!(!state.settings_mounted.load(std::sync::atomic::Ordering::SeqCst));
    *state.pending_settings_nav.lock().unwrap() = Some(nav.clone());
    let consumed = state.pending_settings_nav.lock().unwrap().take();
    state
        .settings_mounted
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(consumed, Some(nav));
    assert!(state.pending_settings_nav.lock().unwrap().is_none());
    assert!(state.settings_mounted.load(std::sync::atomic::Ordering::SeqCst));

    let redacted = redact_user_paths(
        "/Users/tester/Desktop/a.png /private/var/folders/12/abcd/T/b.txt flip://convert?file=%2FUsers%2Ftester%2Fc.md",
    );
    assert_eq!(
        redacted,
        "~/Desktop/a.png ~/T/b.txt flip://convert?file=~%2Fc.md"
    );
}

#[test]
fn bridge_and_wav_resolve_in_both_dev_and_app_bundle_layouts() {
    // 1. Dev mode (real repo files)
    let dev_bridge = resolve_bridge_binary_from(None, &manifest_dir(), None).unwrap();
    assert_eq!(
        dev_bridge,
        repo_root().join("native-bridge/bin/flip-macos-bridge")
    );
    assert!(dev_bridge.is_file());

    let dev_wav = resolve_wav_path_from(&[], None, &manifest_dir()).unwrap();
    assert_eq!(dev_wav, manifest_dir().join("resources/flip-tick.wav"));
    assert!(dev_wav.is_file());

    // 2. Packaged .app bundle layout (Resources/flip-macos-bridge & Resources/resources/flip-tick.wav)
    let temp = tempdir().unwrap();
    let app_dir = temp.path().join("FLIP.app/Contents");
    let macos_dir = app_dir.join("MacOS");
    let res_dir = app_dir.join("Resources");
    let res_nested = res_dir.join("resources");
    fs::create_dir_all(&macos_dir).unwrap();
    fs::create_dir_all(&res_nested).unwrap();

    let fake_exe = macos_dir.join("flip");
    let bundled_bridge = res_dir.join("flip-macos-bridge");
    let bundled_wav = res_nested.join("flip-tick.wav");
    fs::write(&fake_exe, b"exe").unwrap();
    fs::write(&bundled_bridge, b"bridge").unwrap();
    fs::write(&bundled_wav, b"wav").unwrap();

    // Even with bogus manifest_dir (simulating relocated .app), bundled resources resolve cleanly
    let fake_manifest = temp.path().join("nonexistent-src-tauri");
    let resolved_bridge =
        resolve_bridge_binary_from(Some(&fake_exe), &fake_manifest, None).unwrap();
    assert_eq!(resolved_bridge, bundled_bridge);

    let resolved_wav = resolve_wav_path_from(
        std::slice::from_ref(&bundled_wav),
        Some(&fake_exe),
        &fake_manifest,
    )
    .unwrap();
    assert_eq!(resolved_wav, bundled_wav);

    // Also resolves via exe-relative Contents/Resources when BaseDirectory list is empty
    let resolved_wav_exe_fallback =
        resolve_wav_path_from(&[], Some(&fake_exe), &fake_manifest).unwrap();
    assert_eq!(resolved_wav_exe_fallback, bundled_wav);

    // 3. Symlinked CLI invocation (`~/.local/bin/flip -> FLIP.app/Contents/MacOS/flip`) still finds bundled bridge & wav
    let bin_dir = temp.path().join("local_bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let cli_symlink = bin_dir.join("flip");
    std::os::unix::fs::symlink(&fake_exe, &cli_symlink).unwrap();
    assert_eq!(
        resolve_bridge_binary_from(Some(&cli_symlink), &fake_manifest, None).unwrap(),
        fs::canonicalize(&bundled_bridge).unwrap()
    );
    assert_eq!(
        resolve_wav_path_from(&[], Some(&cli_symlink), &fake_manifest).unwrap(),
        fs::canonicalize(&bundled_wav).unwrap()
    );

    // 4. Sibling MacOS/flip-macos-bridge takes precedence if present
    let sibling_bridge = macos_dir.join("flip-macos-bridge");
    fs::write(&sibling_bridge, b"sibling").unwrap();
    assert_eq!(
        resolve_bridge_binary_from(Some(&fake_exe), &fake_manifest, None).unwrap(),
        sibling_bridge
    );
}

#[test]
fn host_inspect_paths_and_screen_clamp_integration() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();

    let p1 = dir.path().join("one.png");
    let p2 = dir.path().join("two.jpg");
    fs::write(&p1, b"png").unwrap();
    fs::write(&p2, b"jpg").unwrap();

    let res = inspect_paths_with_prefs(
        &[
            p1.to_string_lossy().into_owned(),
            p2.to_string_lossy().into_owned(),
        ],
        &prefs,
    );
    assert_eq!(res.files.len(), 2);
    assert_eq!(res.batch_actions, vec!["COMBINE_PDF"]);
    assert!(res.compatible_formats.contains(&"WEBP".to_string()));
    assert!(!res.compatible_formats.contains(&"PNG".to_string()));
    assert!(!res.compatible_formats.contains(&"JPG".to_string()));
    assert_eq!(res.family, Some(FormatFamily::Image));

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
    assert_eq!(clamp_to_screen(20.0, 20.0, 440, &geom), (0, 25));
    assert!(should_hide_on_drag_end(true, true, false, 5, 4, 5));
    assert!(!should_hide_on_drag_end(true, true, false, 5, 5, 5));
}

#[test]
fn frontend_and_rust_command_and_event_names_match_exactly() {
    let lib_rs = fs::read_to_string(manifest_dir().join("src/lib.rs")).unwrap();
    let ipc_ts = fs::read_to_string(repo_root().join("src/ipc.ts")).unwrap();

    // Extract registered `host::commands::<name>` from tauri::generate_handler![]
    let mut rust_cmds = BTreeSet::new();
    for line in lib_rs.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("host::commands::") {
            let cmd = rest.trim_end_matches(',').trim();
            if !cmd.is_empty() {
                rust_cmds.insert(cmd.to_string());
            }
        }
    }
    assert_eq!(rust_cmds.len(), 27);

    // Extract every invoke<...>("cmd_name") in src/ipc.ts
    let mut ts_cmds = BTreeSet::new();
    let mut search = ipc_ts.as_str();
    while let Some(idx) = search.find("invoke<") {
        let after = &search[idx..];
        if let Some(open_paren) = after.find('(') {
            let args_part = &after[open_paren + 1..];
            if let Some(q1) = args_part.find('"') {
                let rem = &args_part[q1 + 1..];
                if let Some(q2) = rem.find('"') {
                    ts_cmds.insert(rem[..q2].to_string());
                }
            }
        }
        search = &search[idx + 7..];
    }
    assert_eq!(ts_cmds, rust_cmds, "TS invoke commands must match Rust generate_handler!");

    // Verify all host-emitted event names are subscribed in src/ipc.ts
    for ev_name in [
        "wheel:state",
        "wheel:drag-files",
        "wheel:pointer-move",
        "wheel:pointer-release",
        "wheel:escape",
        "conversion:progress",
        "conversion:complete",
        "batch:files",
        "settings:navigate",
        "templates:changed",
    ] {
        assert!(
            ipc_ts.contains(&format!("\"{ev_name}\"")),
            "src/ipc.ts missing event subscription `{ev_name}`"
        );
    }

    // Verify every TS invoke parameter key matches Rust #[tauri::command] camelCase argument names
    let commands_rs = fs::read_to_string(manifest_dir().join("src/host/commands.rs")).unwrap();
    let snake_to_camel = |s: &str| -> String {
        let mut out = String::new();
        let mut up = false;
        for ch in s.chars() {
            if ch == '_' {
                up = true;
            } else if up {
                out.push(ch.to_ascii_uppercase());
                up = false;
            } else {
                out.push(ch);
            }
        }
        out
    };

    let mut rust_param_map = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut rem = commands_rs.as_str();
    while let Some(pos) = rem.find("#[tauri::command]") {
        let after = &rem[pos + "#[tauri::command]".len()..];
        let fn_pos = after.find("fn ").unwrap();
        let after_fn = &after[fn_pos + 3..];
        let open_p = after_fn.find('(').unwrap();
        let name = after_fn[..open_p].trim().to_string();
        let close_p = after_fn.find("->").unwrap();
        let sig_end = after_fn[..close_p].rfind(')').unwrap();
        let raw_params = &after_fn[open_p + 1..sig_end];
        let mut keys = Vec::new();
        for p in raw_params.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some((arg, ty)) = p.split_once(':') {
                let ty = ty.trim();
                if ty.contains("AppHandle") || ty.contains("State<") {
                    continue;
                }
                let clean_arg = arg.trim().trim_start_matches("mut ").trim().trim_start_matches('_');
                keys.push(snake_to_camel(clean_arg));
            }
        }
        rust_param_map.insert(name, keys);
        rem = &after_fn[close_p..];
    }
    assert_eq!(rust_param_map.len(), 27);

    let mut ts_search = ipc_ts.as_str();
    while let Some(idx) = ts_search.find("invoke<") {
        let after = &ts_search[idx..];
        let paren = after.find('(').unwrap();
        let close = after[paren..].find(')').unwrap() + paren;
        let call_args = &after[paren + 1..close];
        let q1 = call_args.find('"').unwrap();
        let q2 = call_args[q1 + 1..].find('"').unwrap() + q1 + 1;
        let cmd_name = &call_args[q1 + 1..q2];
        let actual_keys: Vec<String> = if let Some(b1) = call_args.find('{') {
            let b2 = call_args.rfind('}').unwrap();
            call_args[b1 + 1..b2]
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|entry| entry.split(':').next().unwrap().trim().to_string())
                .collect()
        } else {
            Vec::new()
        };
        let expected_keys = rust_param_map
            .get(cmd_name)
            .unwrap_or_else(|| panic!("Unknown TS command `{cmd_name}`"));
        assert_eq!(
            &actual_keys, expected_keys,
            "Parameter mismatch for command `{cmd_name}`"
        );
        ts_search = &ts_search[idx + 7..];
    }
}

#[test]
fn fallback_folder_tilde_expansion_and_save_preferences_history_preservation() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand_tilde_path("~"), home);
    assert_eq!(expand_tilde_path("~/Desktop"), home.join("Desktop"));

    let prefs = AppPreferences {
        fallback_folder: Some(" ~/Documents/FlipInbox ".to_string()),
        ..Default::default()
    };
    assert_eq!(
        prefs.resolved_fallback_folder(),
        home.join("Documents/FlipInbox")
    );

    // Server-recorded recent conversions and onboarding flag must never be erased
    // by stale non-empty recentConversions coming from an already-open Settings view.
    let server_prefs = AppPreferences {
        onboarding_completed: true,
        recent_conversions: vec![
            RecentConversion {
                source_name: "new.png".into(),
                target_format: "JPG".into(),
                output_path: "/tmp/new.jpg".into(),
                timestamp_utc: "2026-11-17T12:05:00Z".into(),
            },
            RecentConversion {
                source_name: "old.png".into(),
                target_format: "JPG".into(),
                output_path: "/tmp/old.jpg".into(),
                timestamp_utc: "2026-11-17T12:00:00Z".into(),
            },
        ],
        ..Default::default()
    };

    let stale_settings = AppPreferences {
        sound_enabled: false,
        onboarding_completed: false,
        recent_conversions: vec![RecentConversion {
            source_name: "old.png".into(),
            target_format: "JPG".into(),
            output_path: "/tmp/old.jpg".into(),
            timestamp_utc: "2026-11-17T12:00:00Z".into(),
        }],
        ..Default::default()
    };

    let merged = merge_and_validate_preferences(stale_settings, &server_prefs).unwrap();
    assert!(!merged.sound_enabled);
    assert!(merged.onboarding_completed);
    assert_eq!(merged.recent_conversions.len(), 2);
    assert_eq!(merged.recent_conversions[0].source_name, "new.png");

    // Invalid shortcuts must still be rejected cleanly
    let bad_prefs = AppPreferences {
        drag_trigger: "Return".into(),
        ..Default::default()
    };
    assert!(merge_and_validate_preferences(bad_prefs, &server_prefs).is_err());
}

#[test]
fn info_plist_registers_flip_url_scheme() {
    let plist_raw = fs::read_to_string(manifest_dir().join("Info.plist")).unwrap();
    assert!(plist_raw.contains("CFBundleURLTypes"));
    assert!(plist_raw.contains("CFBundleURLSchemes"));
    assert!(plist_raw.contains("<string>flip</string>"));
    if Path::new("/usr/bin/plutil").is_file() {
        let status = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(manifest_dir().join("Info.plist"))
            .status()
            .unwrap();
        assert!(status.success(), "src-tauri/Info.plist must pass plutil -lint");
    }
}
