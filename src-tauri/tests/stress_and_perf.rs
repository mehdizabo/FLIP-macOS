use flip_lib::core::input_gate::{
    clamp_wheel_placement, DragTriggerShortcut, IdleShortcut, ScreenRect,
};
use flip_lib::core::policy::{
    family_for_ext, outputs, validate_input_files, EngineCapabilities, MAX_BATCH_FILES,
};
use flip_lib::core::preferences::AppPreferences;
use flip_lib::core::staging::JobStaging;
use flip_lib::engines::{convert_files, EngineError};
use flip_lib::host::monitor::{clamp_to_screen, should_hide_on_drag_end, MonitorScreenGeom};
use flip_lib::host::{inspect_paths_with_prefs, HostState, WHEEL_WINDOW_SIZE};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn multi_monitor_geometries() -> Vec<MonitorScreenGeom> {
    vec![
        // Primary 1440x900 Retina (with 25pt menu bar)
        MonitorScreenGeom {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
            visible_x: 0.0,
            visible_y: 25.0,
            visible_width: 1440.0,
            visible_height: 875.0,
            scale: 2.0,
        },
        // Secondary display positioned to the left (negative X, 1x scale)
        MonitorScreenGeom {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            visible_x: -1920.0,
            visible_y: 25.0,
            visible_width: 1920.0,
            visible_height: 1055.0,
            scale: 1.0,
        },
        // Secondary display positioned above primary (negative Y, Retina 2x, notch menu bar)
        MonitorScreenGeom {
            x: 0.0,
            y: -1440.0,
            width: 2560.0,
            height: 1440.0,
            visible_x: 0.0,
            visible_y: -1408.0,
            visible_width: 2560.0,
            visible_height: 1408.0,
            scale: 2.0,
        },
        // Ultrawide display to the right with side Dock (visible_x inset)
        MonitorScreenGeom {
            x: 1440.0,
            y: 0.0,
            width: 3440.0,
            height: 1440.0,
            visible_x: 1510.0,
            visible_y: 25.0,
            visible_width: 3370.0,
            visible_height: 1415.0,
            scale: 1.0,
        },
    ]
}

fn assert_bounded_wheel(win_x: i32, win_y: i32, geom: &MonitorScreenGeom) {
    let min_x = geom.visible_x.round() as i32;
    let min_y = geom.visible_y.round() as i32;
    let max_x = min_x + geom.visible_width.round() as i32 - WHEEL_WINDOW_SIZE;
    let max_y = min_y + geom.visible_height.round() as i32 - WHEEL_WINDOW_SIZE;
    assert!(
        win_x >= min_x && win_x <= max_x,
        "win_x={win_x} out of bounds [{min_x}, {max_x}] for screen {geom:?}"
    );
    assert!(
        win_y >= min_y && win_y <= max_y,
        "win_y={win_y} out of bounds [{min_y}, {max_y}] for screen {geom:?}"
    );
}

#[test]
fn stress_50_cycle_trigger_wheel_escape_and_multimonitor_clamping() {
    let dir = tempdir().unwrap();
    let sample_png = dir.path().join("sample.png");
    let sample_md = dir.path().join("notes.md");
    fs::write(&sample_png, b"fake-png-bytes").unwrap();
    fs::write(&sample_md, b"# Stress Notes\n").unwrap();

    let prefs = AppPreferences::default();
    let drag_shortcut = DragTriggerShortcut::parse(Some(&prefs.drag_trigger)).unwrap();
    let idle_shortcut = IdleShortcut::parse(&prefs.idle_shortcut).unwrap();
    assert_eq!(drag_shortcut.key_code, Some(125));
    assert_eq!(idle_shortcut.canonical_id(), "Option+Shift+Space");

    let state = HostState::new(prefs.clone());
    let screens = multi_monitor_geometries();
    let sample_files = [
        sample_png.to_string_lossy().into_owned(),
        sample_md.to_string_lossy().into_owned(),
    ];

    for cycle in 0..50usize {
        let geom = &screens[cycle % screens.len()];
        // Sample corners, edges, center, and slightly-outside coordinates
        let cursor_x = match cycle % 5 {
            0 => geom.x + 4.0,
            1 => geom.x + geom.width - 4.0,
            2 => geom.x + geom.width / 2.0,
            3 => geom.visible_x - 30.0,
            _ => geom.visible_x + geom.visible_width + 30.0,
        };
        let cursor_y = match (cycle / 5) % 5 {
            0 => geom.y + 4.0,
            1 => geom.y + geom.height - 4.0,
            2 => geom.y + geom.height / 2.0,
            3 => geom.visible_y - 25.0,
            _ => geom.visible_y + geom.visible_height + 25.0,
        };

        let (win_x, win_y) = clamp_to_screen(cursor_x, cursor_y, WHEEL_WINDOW_SIZE, geom);
        assert_bounded_wheel(win_x, win_y, geom);

        // Also validate pure integer `clamp_wheel_placement` parity
        let rect = ScreenRect {
            x: geom.visible_x.round() as i32,
            y: geom.visible_y.round() as i32,
            width: geom.visible_width.round() as i32,
            height: geom.visible_height.round() as i32,
        };
        let placed = clamp_wheel_placement(
            cursor_x.round() as i32,
            cursor_y.round() as i32,
            WHEEL_WINDOW_SIZE,
            rect,
        );
        assert_eq!((placed.x, placed.y, placed.size), (win_x, win_y, WHEEL_WINDOW_SIZE));

        let pattern = cycle % 4;
        let is_drag = pattern != 2;

        // 1. Drive ShortcutPressGate + ShortcutOrderGate through realistic trigger sequences
        {
            let mut press = state.press_gate.lock().unwrap();
            let mut order = state.order_gate.lock().unwrap();

            match pattern {
                0 => {
                    // Drag-first -> key-down + repeats -> Esc cancel / mouse-up -> key-up
                    order.mouse_down(cursor_x.round() as i32, cursor_y.round() as i32);
                    assert!(order.key_down(true));
                    assert_eq!(press.process(true, true), (true, true));
                    assert_eq!(press.process(true, true), (true, false)); // repeat suppressed
                }
                1 => {
                    // Key-first -> mouse-down -> >= 8px drag -> opens once
                    assert!(!order.key_down(false));
                    assert_eq!(press.process(true, false), (false, false));
                    press.reserve_repeats_for_gesture();
                    assert_eq!(press.process(true, false), (true, false)); // repeat consumed
                    order.mouse_down(cursor_x.round() as i32, cursor_y.round() as i32);
                    assert!(!order.move_cursor(cursor_x.round() as i32 + 3, cursor_y.round() as i32 + 2));
                    assert!(order.move_cursor(cursor_x.round() as i32 + 12, cursor_y.round() as i32));
                    assert!(!order.move_cursor(cursor_x.round() as i32 + 28, cursor_y.round() as i32));
                }
                2 => {
                    // Idle mode open -> mode switch -> Esc dismiss (no drag active)
                    assert!(!order.held());
                    assert!(!press.is_down());
                }
                _ => {
                    // Key-first cancelled mid-drag, then immediate re-drag with fresh press
                    assert!(!order.key_down(false));
                    assert_eq!(press.process(true, false), (false, false));
                    order.mouse_down(cursor_x.round() as i32, cursor_y.round() as i32);
                    order.mouse_up();
                    order.key_up();
                    assert_eq!(press.process(false, false), (false, false));

                    order.mouse_down(cursor_x.round() as i32, cursor_y.round() as i32);
                    assert!(order.key_down(true));
                    assert_eq!(press.process(true, true), (true, true));
                }
            }
        }

        // 2. Open wheel state in HostState
        let selected = if pattern == 2 && cycle % 8 == 2 {
            Vec::new()
        } else {
            vec![sample_files[cycle % sample_files.len()].clone()]
        };
        let inspected = inspect_paths_with_prefs(&selected, &prefs);
        let mode = if selected.is_empty() { "new" } else { "convert" };
        let cancel_token = Arc::new(AtomicBool::new(false));
        *state.active_cancel.lock().unwrap() = Some(Arc::clone(&cancel_token));
        state.is_converting.store(pattern == 3, Ordering::Relaxed);

        let epoch_now = {
            let mut wheel = state.wheel.lock().unwrap();
            wheel.open_epoch = wheel.open_epoch.wrapping_add(1);
            wheel.is_open = true;
            wheel.window_x = win_x;
            wheel.window_y = win_y;
            wheel.payload.mode = mode.to_string();
            wheel.payload.files = inspected.files;
            wheel.payload.is_drag = is_drag;
            wheel.payload.compatible_formats = inspected.compatible_formats;
            wheel.payload.batch_actions = inspected.batch_actions;
            wheel.payload.origin_x = cursor_x - win_x as f64;
            wheel.payload.origin_y = cursor_y - win_y as f64;
            wheel.payload.family = inspected.family;
            wheel.open_epoch
        };

        // Verify stale epoch timer from previous cycle would NOT close the current wheel
        assert!(!should_hide_on_drag_end(
            true,
            true,
            false,
            epoch_now,
            epoch_now.saturating_sub(1),
            epoch_now.saturating_sub(1),
        ));

        // 3. Global Escape / drag-end cancellation sequence
        if let Some(flag) = state.active_cancel.lock().unwrap().as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
        assert!(cancel_token.load(Ordering::Relaxed));

        {
            let mut wheel = state.wheel.lock().unwrap();
            if is_drag {
                // While user actually performed a wheel action in this epoch, drag-end must NOT dismiss the wheel
                let hide_after_action = should_hide_on_drag_end(
                    wheel.is_open,
                    wheel.payload.is_drag,
                    false,
                    wheel.open_epoch,
                    epoch_now,
                    epoch_now,
                );
                assert!(!hide_after_action);

                assert!(should_hide_on_drag_end(
                    wheel.is_open,
                    wheel.payload.is_drag,
                    false,
                    wheel.open_epoch,
                    wheel.acted_epoch,
                    epoch_now,
                ));
            }
            wheel.is_open = false;
            wheel.payload.is_drag = false;
        }
        state.is_converting.store(false, Ordering::Relaxed);
        *state.active_cancel.lock().unwrap() = None;

        {
            let mut press = state.press_gate.lock().unwrap();
            let mut order = state.order_gate.lock().unwrap();
            if pattern != 2 {
                if cycle % 5 == 4 {
                    // Escape pressed mid-hold while mouse/key are still held down -> monitor teardown calls reset()
                    assert!(
                        press.is_down() || order.held(),
                        "cycle {cycle}: expected active hold before Escape reset"
                    );
                    press.reset();
                    order.reset();
                } else {
                    // Natural release without calling reset() must leave zero stuck key/mouse flags
                    order.mouse_up();
                    order.key_up();
                    let _ = press.process(false, false);
                }
            }

            assert!(!press.is_down(), "cycle {cycle}: press_gate stuck down");
            assert!(!order.held(), "cycle {cycle}: order_gate stuck held");
            // Normal typing after escape/drag end must pass right through unconsumed
            assert_eq!(press.process(true, false), (false, false));
            assert_eq!(press.process(false, false), (false, false));
        }

        let wheel_after = state.wheel.lock().unwrap();
        assert!(!wheel_after.is_open, "cycle {cycle}: orphaned open wheel state");
        assert!(!wheel_after.payload.is_drag, "cycle {cycle}: orphaned is_drag flag");
    }

    assert!(!state.is_converting.load(Ordering::Relaxed));
    assert!(state.active_cancel.lock().unwrap().is_none());
    assert!(!state.press_gate.lock().unwrap().is_down());
    assert!(!state.order_gate.lock().unwrap().held());
}

fn find_residual_staging_dirs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".flip-job-") || name.starts_with(".flip-probe-") {
            out.push(path);
            continue;
        }
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            find_residual_staging_dirs(&path, out);
        }
    }
}

#[test]
fn stress_20_plus_10_staging_cleanup_policy_latency_and_cli_startup_perf() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    // Create fast local native test inputs
    let json_src = dir.path().join("data.json");
    let md_src = dir.path().join("doc.md");
    let txt_src = dir.path().join("note.txt");
    let png_src = dir.path().join("pixel.png");

    fs::write(&json_src, r#"{"app":"FLIP","version":"0.1.0","items":[1,2,3]}"#).unwrap();
    fs::write(&md_src, "# Perf Check\n\nFast native conversion.\n").unwrap();
    fs::write(&txt_src, "Plain text benchmark payload.\n").unwrap();
    {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([24, 144, 255, 255]));
        img.save_with_format(&png_src, image::ImageFormat::Png).unwrap();
    }

    // 1) 20 sequential successful conversions across engines (Data, Document/EPUB, Image, ZIP)
    let success_jobs: [(&Path, &str); 20] = [
        (&json_src, "YAML"),
        (&json_src, "TOML"),
        (&json_src, "XML"),
        (&json_src, "CSV"),
        (&md_src, "HTML"),
        (&md_src, "TXT"),
        (&md_src, "EPUB"),
        (&md_src, "DOCX"),
        (&txt_src, "MD"),
        (&txt_src, "ODT"),
        (&txt_src, "RTF"),
        (&png_src, "JPG"),
        (&png_src, "WEBP"),
        (&png_src, "BMP"),
        (&png_src, "TIFF"),
        (&png_src, "GIF"),
        (&json_src, "ZIP"),
        (&md_src, "ZIP"),
        (&txt_src, "ZIP"),
        (&png_src, "ZIP"),
    ];
    for (idx, (src, fmt)) in success_jobs.iter().enumerate() {
        let res = convert_files(
            &[(*src).to_path_buf()],
            &[(*fmt).to_string()],
            &prefs,
            &cancel,
            None,
        )
        .unwrap();
        assert_eq!(res.len(), 1);
        assert!(
            res[0].error.is_none(),
            "sequential conversion #{idx} ({} -> {fmt}) failed: {:?}",
            src.display(),
            res[0].error
        );
        assert!(
            Path::new(&res[0].outputs[0]).exists(),
            "missing output for #{idx}"
        );
    }

    // 2) 10 intentionally failing / cancelled conversions that each enter JobStaging::create
    //    (2 mid-flight cancellations triggered after JobStaging::create, 2 corrupt JSON,
    //     2 zip-slip / truncated PK-zip UNZIP, 2 corrupt PNG, 1 corrupt PDF OPTIMIZE_PDF,
    //     1 blank transparent PNG REMOVE_BG), plus 2 read-only JobStaging Drop aborts
    let bad_json = dir.path().join("corrupt.json");
    fs::write(&bad_json, "{not valid json").unwrap();

    let slip_zip = dir.path().join("slip.zip");
    {
        let mut zw = ZipWriter::new(File::create(&slip_zip).unwrap());
        zw.start_file("../escape.txt", SimpleFileOptions::default().compression_method(CompressionMethod::Stored))
            .unwrap();
        zw.write_all(b"evil").unwrap();
        zw.finish().unwrap();
    }

    let truncated_zip = dir.path().join("truncated.zip");
    fs::write(&truncated_zip, b"PK\x03\x04\x14\x00\x00\x00truncated-central-dir").unwrap();

    let corrupt_png = dir.path().join("corrupt_body.png");
    fs::write(&corrupt_png, b"\x89PNG\r\n\x1a\nnot-a-valid-ihdr-chunk").unwrap();

    let corrupt_pdf = dir.path().join("corrupt_body.pdf");
    fs::write(&corrupt_pdf, b"%PDF-1.4\n%corrupt-missing-trailer\n").unwrap();

    let blank_png = dir.path().join("blank_transparent.png");
    {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 0]));
        img.save_with_format(&blank_png, image::ImageFormat::Png).unwrap();
    }

    // 2a-b: 2 mid-flight cancellations triggered inside progress callback AFTER JobStaging::create
    for (src, fmt) in [(&png_src, "JPG"), (&json_src, "YAML")] {
        let cancel_mid = Arc::new(AtomicBool::new(false));
        let cancel_ref = Arc::clone(&cancel_mid);
        let saw_staging_progress = Arc::new(AtomicBool::new(false));
        let saw_ref = Arc::clone(&saw_staging_progress);
        let cb = move |pct: u8, _msg: &str| {
            // base_pct is 2; pct >= 20 only fires from unit_progress inside convert_single_unit after JobStaging::create
            if pct >= 20 {
                saw_ref.store(true, Ordering::Relaxed);
                cancel_ref.store(true, Ordering::Relaxed);
            }
        };
        let err = convert_files(
            std::slice::from_ref(src),
            &[fmt.to_string()],
            &prefs,
            &cancel_mid,
            Some(&cb),
        );
        assert!(saw_staging_progress.load(Ordering::Relaxed), "expected mid-flight progress after JobStaging::create for {fmt}");
        assert!(matches!(err, Err(EngineError::Cancelled)), "expected mid-flight Cancelled for {fmt}, got {err:?}");
    }
    // 2c-d: 2 corrupt JSON conversions (returns unit error inside ConversionResult)
    for fmt in ["YAML", "TOML"] {
        let res = convert_files(std::slice::from_ref(&bad_json), &[fmt.into()], &prefs, &cancel, None).unwrap();
        assert!(res[0].error.is_some() && res[0].outputs.is_empty());
    }
    // 2e-f: 2 zip-slip & truncated PK-zip UNZIP errors
    for bad_zip in [&slip_zip, &truncated_zip] {
        let unzip_err = convert_files(std::slice::from_ref(bad_zip), &["UNZIP".into()], &prefs, &cancel, None).unwrap();
        assert!(unzip_err[0].error.is_some() && unzip_err[0].outputs.is_empty());
    }
    // 2g-h: 2 corrupt PNG image decode failures
    for fmt in ["JPG", "WEBP"] {
        let img_err = convert_files(std::slice::from_ref(&corrupt_png), &[fmt.into()], &prefs, &cancel, None).unwrap();
        assert!(img_err[0].error.is_some() && img_err[0].outputs.is_empty());
    }
    // 2i: 1 corrupt PDF OPTIMIZE_PDF failure
    let pdf_err = convert_files(std::slice::from_ref(&corrupt_pdf), &["OPTIMIZE_PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(pdf_err[0].error.is_some() && pdf_err[0].outputs.is_empty());
    // 2j: 1 blank transparent PNG Vision REMOVE_BG failure
    let bg_err = convert_files(std::slice::from_ref(&blank_png), &["REMOVE_BG".into()], &prefs, &cancel, None).unwrap();
    assert!(bg_err[0].error.is_some() && bg_err[0].outputs.is_empty());

    // Extra: 2 simulated read-only mid-job staging aborts (Drop cleanup without commit)
    for i in 0..2 {
        let staging = JobStaging::create(dir.path()).unwrap();
        let staged_file = staging.staged_path(&format!("aborted_{i}.tmp"));
        fs::write(&staged_file, b"partial").unwrap();
        let mut perms = fs::metadata(&staged_file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&staged_file, perms).unwrap();
        assert!(staging.dir().exists());
        drop(staging);
    }

    // Assert 0 leftover `.flip-job-*` or `.flip-probe-*` directories anywhere in TempDir
    let mut residuals = Vec::new();
    find_residual_staging_dirs(dir.path(), &mut residuals);
    assert!(
        residuals.is_empty(),
        "Found leftover staging directories after 20+10 runs: {residuals:?}"
    );

    // 3) `policy::outputs` latency benchmark (< 5 ms target even for 128-file batch)
    let caps_all = EngineCapabilities::all();
    let caps_native = EngineCapabilities::native_only();
    let max_batch: Vec<PathBuf> = (0..MAX_BATCH_FILES)
        .map(|i| dir.path().join(format!("img_{i}.png")))
        .collect();
    let mixed_batch = vec![
        PathBuf::from("photo.heic"),
        PathBuf::from("scan.pdf"),
        PathBuf::from("clip.mp4"),
    ];
    let single_pdf = vec![PathBuf::from("deck.pdf")];

    let single_start = Instant::now();
    let pdf_targets = outputs(&single_pdf, &caps_all);
    let single_elapsed = single_start.elapsed();
    assert!(pdf_targets.contains(&"EPUB".to_string()));
    assert!(pdf_targets.contains(&"OPTIMIZE_PDF".to_string()));
    assert!(
        single_elapsed < Duration::from_millis(5),
        "single policy::outputs exceeded 5ms: {single_elapsed:?}"
    );

    let batch_start = Instant::now();
    let batch_targets = outputs(&max_batch, &caps_native);
    let batch_elapsed = batch_start.elapsed();
    assert!(batch_targets.contains(&"JPG".to_string()));
    assert!(
        batch_elapsed < Duration::from_millis(5),
        "128-file policy::outputs exceeded 5ms: {batch_elapsed:?}"
    );

    let loop_start = Instant::now();
    for _ in 0..500 {
        let _ = outputs(&single_pdf, &caps_all);
        let _ = outputs(&mixed_batch, &caps_native);
        let _ = family_for_ext("flac");
    }
    let loop_elapsed = loop_start.elapsed();
    let avg_per_pair = loop_elapsed / 500;
    assert!(
        avg_per_pair < Duration::from_millis(5),
        "average policy::outputs exceeded 5ms: {avg_per_pair:?}"
    );

    // Also sanity-check validate_input_files stays crisp on real files
    let valid_start = Instant::now();
    let validated = validate_input_files(&[json_src, md_src, txt_src, png_src]).unwrap();
    let valid_elapsed = valid_start.elapsed();
    assert_eq!(validated.len(), 4);
    assert!(valid_elapsed < Duration::from_millis(50));

    // 4) Compiled CLI `--version` startup latency (< 300 ms)
    let cli_bin = PathBuf::from(env!("CARGO_BIN_EXE_flip"));
    assert!(cli_bin.is_file(), "CARGO_BIN_EXE_flip missing at {}", cli_bin.display());

    // First invocation of a freshly linked Mach-O binary on macOS incurs one-time AMFI/syspolicyd scan.
    let cold_start = Instant::now();
    let cold_out = Command::new(&cli_bin)
        .arg("--version")
        .output()
        .expect("spawn cold flip --version");
    let cold_elapsed = cold_start.elapsed();
    assert!(cold_out.status.success(), "cold flip --version failed: {cold_out:?}");
    let cold_stdout = String::from_utf8_lossy(&cold_out.stdout);
    assert!(
        cold_stdout.starts_with("flip 0.1.0"),
        "unexpected --version output: {cold_stdout:?}"
    );

    let mut warm_samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let start = Instant::now();
        let out = Command::new(&cli_bin)
            .arg("--version")
            .output()
            .expect("spawn flip --version");
        let elapsed = start.elapsed();
        assert!(out.status.success(), "flip --version failed: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.starts_with("flip 0.1.0"), "unexpected --version output: {stdout:?}");
        warm_samples.push(elapsed);
    }
    let min_cli = *warm_samples.iter().min().unwrap();
    let max_cli = *warm_samples.iter().max().unwrap();
    assert!(
        max_cli < Duration::from_millis(300),
        "CLI `--version` startup exceeded 300ms (cold={cold_elapsed:?}, warm={warm_samples:?})"
    );

    println!(
        "[stress_and_perf] 20+10 staging=0 leftovers | policy::outputs single={:?}, 128-batch={:?}, 500x avg={:?} | CLI --version cold={:?}, warm_min={:?}, warm_max={:?}",
        single_elapsed, batch_elapsed, avg_per_pair, cold_elapsed, min_cli, max_cli
    );
}
