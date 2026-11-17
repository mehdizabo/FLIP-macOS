use flip_lib::core::policy::FormatFamily;
use flip_lib::core::preferences::{AppPreferences, RecentConversion};
use flip_lib::engines::bridge::{
    resolve_bridge_binary_from, ClipboardInspect, PermissionsStatus, ThumbnailResult,
};
use flip_lib::engines::create_new::{CreateNewResult, NewFileKindInfo, UserTemplateInfo};
use flip_lib::engines::ConversionResult;
use flip_lib::host::commands::{
    ConversionCompletePayload, EngineStatusPayload, NativeFrameworksStatus, NewFileKindsResponse,
};
use flip_lib::host::monitor::{clamp_to_screen, should_hide_on_drag_end, MonitorScreenGeom};
use flip_lib::host::{inspect_paths_with_prefs, resolve_wav_path_from, WheelStatePayload};
use serde_json::Value;
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

    let resolved_wav =
        resolve_wav_path_from(&[bundled_wav.clone()], Some(&fake_exe), &fake_manifest).unwrap();
    assert_eq!(resolved_wav, bundled_wav);

    // Also resolves via exe-relative Contents/Resources when BaseDirectory list is empty
    let resolved_wav_exe_fallback =
        resolve_wav_path_from(&[], Some(&fake_exe), &fake_manifest).unwrap();
    assert_eq!(resolved_wav_exe_fallback, bundled_wav);

    // 3. Sibling MacOS/flip-macos-bridge takes precedence if present
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
