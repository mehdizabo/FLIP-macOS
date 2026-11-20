use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("Native bridge binary flip-macos-bridge not found")]
    BridgeNotFound,
    #[error("Operation cancelled")]
    Cancelled,
    #[error("Command `{0}` timed out after {1}s")]
    TimedOut(String, u64),
    #[error("Bridge command `{0}` failed: {1}")]
    CommandFailed(String, String),
    #[error("Failed to parse bridge output: {0}")]
    InvalidOutput(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsStatus {
    pub accessibility: bool,
    pub automation_finder: String,
    pub input_monitoring: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinderContext {
    pub frontmost_bundle_id: String,
    pub selected_files: Vec<String>,
    pub target_folder: String,
    pub automation_granted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealResult {
    pub ok: bool,
    pub rename_triggered: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardInspect {
    pub available: bool,
    pub kind: String,
    pub extension: String,
    #[serde(default)]
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailResult {
    pub ok: bool,
    pub png_base64: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}

/// Repository checkout (`src-tauri/`) used as a development-only fallback for the bridge and the
/// tick sound. It is `None` in release builds, so a shipped binary neither embeds the build
/// machine's absolute path nor consults a working copy; unbundled release runs can point at the
/// bridge with `FLIP_BRIDGE_PATH`.
#[cfg(debug_assertions)]
pub fn dev_checkout_dir() -> Option<&'static Path> {
    Some(Path::new(env!("CARGO_MANIFEST_DIR")))
}

#[cfg(not(debug_assertions))]
pub fn dev_checkout_dir() -> Option<&'static Path> {
    None
}

pub fn resolve_bridge_binary() -> Result<PathBuf, BridgeError> {
    let explicit = std::env::var("FLIP_BRIDGE_PATH").ok();
    let current_exe = std::env::current_exe().ok();
    resolve_bridge_binary_from(
        current_exe.as_deref(),
        dev_checkout_dir(),
        explicit.as_deref(),
    )
}

fn probe_bridge_near_exe(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let sibling = dir.join("flip-macos-bridge");
    if sibling.is_file() {
        return Some(sibling);
    }
    let contents = dir.parent()?;
    let res = contents.join("Resources").join("flip-macos-bridge");
    if res.is_file() {
        return Some(res);
    }
    let up_res = contents
        .join("Resources")
        .join("_up_")
        .join("native-bridge")
        .join("bin")
        .join("flip-macos-bridge");
    if up_res.is_file() {
        return Some(up_res);
    }
    None
}

fn exe_is_in_app_bundle(exe: &Path) -> bool {
    let Some(macos_dir) = exe.parent() else {
        return false;
    };
    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    macos_dir.file_name().and_then(|s| s.to_str()) == Some("MacOS")
        && contents_dir.file_name().and_then(|s| s.to_str()) == Some("Contents")
}

pub fn resolve_bridge_binary_from(
    exe_path: Option<&Path>,
    manifest_dir: Option<&Path>,
    explicit_env: Option<&str>,
) -> Result<PathBuf, BridgeError> {
    if let Some(explicit) = explicit_env {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Ok(p);
        }
    }

    let dev_bin = manifest_dir.and_then(Path::parent).and_then(|repo_root| {
        let candidate = repo_root
            .join("native-bridge")
            .join("bin")
            .join("flip-macos-bridge");
        candidate.is_file().then_some(candidate)
    });

    let canonical_exe = exe_path.and_then(|exe| {
        std::fs::canonicalize(exe)
            .ok()
            .filter(|canon| canon != exe)
    });
    let in_bundle = exe_path.is_some_and(exe_is_in_app_bundle)
        || canonical_exe.as_deref().is_some_and(exe_is_in_app_bundle);

    // Outside an assembled `.app` bundle (`cargo tauri dev`, `cargo test`), prefer the canonical
    // freshly-signed repo binary over `target/<profile>/flip-macos-bridge`, which Tauri builds
    // overwrite in-place and macOS ARM64 vnode signature caching can SIGKILL (137).
    if !in_bundle {
        if let Some(ref dev) = dev_bin {
            return Ok(dev.clone());
        }
    }

    if let Some(exe) = exe_path {
        if let Some(found) = probe_bridge_near_exe(exe) {
            return Ok(found);
        }
    }
    if let Some(ref canon) = canonical_exe {
        if let Some(found) = probe_bridge_near_exe(canon) {
            return Ok(found);
        }
    }

    if let Some(dev) = dev_bin {
        return Ok(dev);
    }

    Err(BridgeError::BridgeNotFound)
}

/// Longest pause between checks on a running child (also the cancel/timeout reaction time).
const MAX_POLL_DELAY_MS: u64 = 20;

/// Pause before checking a running child again. It starts at 1 ms and doubles, so short commands
/// (most bridge calls finish in a few ms) are not rounded up to a fixed 20 ms tick, then it settles at
/// `MAX_POLL_DELAY_MS` so long conversions poll exactly as often as before.
fn poll_delay(attempt: u32) -> Duration {
    Duration::from_millis((1u64 << attempt.min(5)).min(MAX_POLL_DELAY_MS))
}

pub fn run_external_command(
    exe: &Path,
    args: &[String],
    cwd: Option<&Path>,
    timeout_secs: u64,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<(String, String), BridgeError> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let out_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(ref mut r) = stdout_pipe {
            let _ = r.read_to_string(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(ref mut r) = stderr_pipe {
            let _ = r.read_to_string(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let label = exe
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tool")
        .to_string();

    let mut poll_attempt = 0u32;
    let status = loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(BridgeError::Cancelled);
        }
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(BridgeError::TimedOut(label, timeout_secs));
        }
        std::thread::sleep(poll_delay(poll_attempt));
        poll_attempt = poll_attempt.saturating_add(1);
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    if !status.success() {
        let mut detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        if detail.is_empty() {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(sig) = status.signal() {
                    detail = format!("process terminated by signal {sig}");
                } else if let Some(code) = status.code() {
                    detail = format!("exited with status {code}");
                } else {
                    detail = "process exited unsuccessfully".to_string();
                }
            }
            #[cfg(not(unix))]
            {
                detail = match status.code() {
                    Some(code) => format!("exited with status {code}"),
                    None => "process exited unsuccessfully".to_string(),
                };
            }
        }
        let snippet: String = detail.chars().take(800).collect();
        return Err(BridgeError::CommandFailed(label, snippet));
    }

    Ok((stdout, stderr))
}

pub fn run_bridge_json(
    args: &[String],
    timeout_secs: u64,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let bin = resolve_bridge_binary()?;
    let (stdout, _stderr) = run_external_command(&bin, args, None, timeout_secs, cancel)?;
    let last_line = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| BridgeError::InvalidOutput("Empty bridge stdout".into()))?;
    serde_json::from_str(last_line).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn permissions(prompt_ax: bool, open_ax: bool, open_automation: bool) -> Result<PermissionsStatus, BridgeError> {
    let mut args = vec!["permissions".to_string()];
    if prompt_ax {
        args.push("--prompt-ax".to_string());
    }
    if open_ax {
        args.push("--open-ax".to_string());
    }
    if open_automation {
        args.push("--open-automation".to_string());
    }
    let val = run_bridge_json(&args, 15, None)?;
    serde_json::from_value(val).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn finder_context() -> Result<FinderContext, BridgeError> {
    let val = run_bridge_json(&["finder-context".to_string()], 10, None)?;
    serde_json::from_value(val).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn finder_reveal(paths: &[&Path], rename: bool) -> Result<RevealResult, BridgeError> {
    let mut args = vec!["finder-reveal".to_string()];
    if rename {
        args.push("--rename".to_string());
    }
    for p in paths {
        args.push(p.to_string_lossy().into_owned());
    }
    let val = run_bridge_json(&args, 10, None)?;
    serde_json::from_value(val).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn clipboard_inspect() -> Result<ClipboardInspect, BridgeError> {
    let val = run_bridge_json(&["clipboard-inspect".to_string()], 10, None)?;
    serde_json::from_value(val).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn clipboard_save(output_path: &Path) -> Result<Value, BridgeError> {
    let args = vec![
        "clipboard-save".to_string(),
        output_path.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 15, None)
}

pub fn thumbnail(path: &Path, size: u32) -> Result<ThumbnailResult, BridgeError> {
    let args = vec![
        "thumbnail".to_string(),
        path.to_string_lossy().into_owned(),
        size.to_string(),
    ];
    let val = run_bridge_json(&args, 15, None)?;
    serde_json::from_value(val).map_err(|e| BridgeError::InvalidOutput(e.to_string()))
}

pub fn ocr(
    image_path: &Path,
    out_txt_path: Option<&Path>,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let mut args = vec!["ocr".to_string(), image_path.to_string_lossy().into_owned()];
    if let Some(out) = out_txt_path {
        args.push(out.to_string_lossy().into_owned());
    }
    // Cold first-run E5/Vision initialization on macOS can take ~28s; subsequent calls take <0.4s.
    run_bridge_json(&args, 90, cancel)
}

pub fn ocr_boxes(image: &Path, cancel: Option<&Arc<AtomicBool>>) -> Result<Value, BridgeError> {
    let args = vec!["ocr-boxes".to_string(), image.to_string_lossy().into_owned()];
    // Same cold-start allowance as `ocr`: the first Vision initialisation can take ~30s.
    run_bridge_json(&args, 90, cancel)
}

pub fn convert_image(
    input: &Path,
    format: &str,
    output: &Path,
    quality_01: f64,
    preserve_exif: bool,
    strip_gps: bool,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "convert-image".to_string(),
        input.to_string_lossy().into_owned(),
        format.to_string(),
        output.to_string_lossy().into_owned(),
        format!("{quality_01:.3}"),
        if preserve_exif { "true" } else { "false" }.to_string(),
        if strip_gps { "true" } else { "false" }.to_string(),
    ];
    run_bridge_json(&args, 60, cancel)
}

pub fn remove_bg(
    input: &Path,
    output_png: &Path,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "remove-bg".to_string(),
        input.to_string_lossy().into_owned(),
        output_png.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 60, cancel)
}

pub fn optimize_pdf(
    input: &Path,
    output_pdf: &Path,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "optimize-pdf".to_string(),
        input.to_string_lossy().into_owned(),
        output_pdf.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 120, cancel)
}

pub fn create_iwork(kind: &str, output: &Path) -> Result<Value, BridgeError> {
    let args = vec![
        "create-iwork".to_string(),
        kind.to_string(),
        output.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 30, None)
}

pub fn convert_pdf_render(
    pdf: &Path,
    out_dir: &Path,
    format: &str,
    dpi: u16,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Vec<PathBuf>, BridgeError> {
    let args = vec![
        "convert-pdf".to_string(),
        "render".to_string(),
        pdf.to_string_lossy().into_owned(),
        out_dir.to_string_lossy().into_owned(),
        format.to_string(),
        dpi.to_string(),
    ];
    let val = run_bridge_json(&args, 120, cancel)?;
    let outputs = val
        .get("outputs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| BridgeError::InvalidOutput("Missing outputs array".into()))?;
    Ok(outputs
        .iter()
        .filter_map(|v| v.as_str().map(PathBuf::from))
        .collect())
}

pub fn convert_pdf_extract_text(
    pdf: &Path,
    out_txt: &Path,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "convert-pdf".to_string(),
        "extract-text".to_string(),
        pdf.to_string_lossy().into_owned(),
        out_txt.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 120, cancel)
}

pub fn convert_pdf_from_images(
    out_pdf: &Path,
    images: &[PathBuf],
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let mut args = vec![
        "convert-pdf".to_string(),
        "from-images".to_string(),
        out_pdf.to_string_lossy().into_owned(),
    ];
    for img in images {
        args.push(img.to_string_lossy().into_owned());
    }
    run_bridge_json(&args, 90, cancel)
}

pub fn convert_pdf_merge(
    out_pdf: &Path,
    pdfs: &[PathBuf],
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let mut args = vec![
        "convert-pdf".to_string(),
        "merge".to_string(),
        out_pdf.to_string_lossy().into_owned(),
    ];
    for pdf in pdfs {
        args.push(pdf.to_string_lossy().into_owned());
    }
    run_bridge_json(&args, 90, cancel)
}

pub fn convert_doc(
    input: &Path,
    format: &str,
    output: &Path,
    page_size: &str,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "convert-doc".to_string(),
        input.to_string_lossy().into_owned(),
        format.to_string(),
        output.to_string_lossy().into_owned(),
        "--page-size".to_string(),
        page_size.to_string(),
    ];
    run_bridge_json(&args, 120, cancel)
}

pub fn convert_av(
    input: &Path,
    format: &str,
    output: &Path,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Value, BridgeError> {
    let args = vec![
        "convert-av".to_string(),
        input.to_string_lossy().into_owned(),
        format.to_string(),
        output.to_string_lossy().into_owned(),
    ];
    run_bridge_json(&args, 300, cancel)
}

pub fn feedback(kind: &str, wav_path: Option<&Path>) -> Result<(), BridgeError> {
    let mut args = vec!["feedback".to_string(), kind.to_string()];
    if let Some(w) = wav_path {
        args.push(w.to_string_lossy().into_owned());
    }
    let _ = run_bridge_json(&args, 5, None)?;
    Ok(())
}

pub fn notify(title: &str, body: &str, reveal_path: Option<&Path>) -> Result<Value, BridgeError> {
    let mut args = vec![
        "notify".to_string(),
        title.to_string(),
        body.to_string(),
    ];
    if let Some(path) = reveal_path {
        args.push(path.to_string_lossy().into_owned());
    }
    run_bridge_json(&args, 8, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_delay_starts_short_grows_and_caps_at_the_old_fixed_tick() {
        let delays: Vec<u64> = (0..12).map(|i| poll_delay(i).as_millis() as u64).collect();
        assert_eq!(&delays[..6], &[1, 2, 4, 8, 16, 20]);
        assert!(delays.windows(2).all(|w| w[0] <= w[1]), "never shrinks: {delays:?}");
        assert!(delays.iter().all(|&d| (1..=MAX_POLL_DELAY_MS).contains(&d)));
        assert_eq!(poll_delay(u32::MAX), Duration::from_millis(MAX_POLL_DELAY_MS));
        // A command finishing within ~30 ms is noticed on the first polls, not after a 20 ms tick.
        assert!(delays[..5].iter().sum::<u64>() <= 31);
    }

    #[test]
    fn slow_commands_keep_their_output_cancel_and_timeout_behaviour() {
        let sh = Path::new("/bin/sh");
        let (out, _) = run_external_command(
            sh,
            &["-c".into(), "sleep 0.15; printf done".into()],
            None,
            10,
            None,
        )
        .expect("slow command completes");
        assert_eq!(out, "done");

        let started = Instant::now();
        let err = run_external_command(sh, &["-c".into(), "sleep 5".into()], None, 1, None)
            .expect_err("times out");
        assert!(matches!(err, BridgeError::TimedOut(_, 1)), "{err:?}");
        assert!(started.elapsed() < Duration::from_millis(1500), "timeout overshoot: {:?}", started.elapsed());

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let err = run_external_command(sh, &["-c".into(), "sleep 5".into()], None, 30, Some(&cancel))
            .expect_err("cancelled");
        assert!(matches!(err, BridgeError::Cancelled), "{err:?}");
        assert!(started.elapsed() < Duration::from_millis(1000), "cancel latency: {:?}", started.elapsed());
    }

    /// Manual timing of the process-spawn path: `cargo test --lib external_command_latency -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe, run manually"]
    fn external_command_latency() {
        let exe = Path::new("/usr/bin/true");
        let mut samples: Vec<f64> = (0..40)
            .map(|_| {
                let start = Instant::now();
                run_external_command(exe, &[], None, 10, None).expect("true runs");
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "run_external_command(/usr/bin/true): median {:.1} ms, min {:.1} ms, max {:.1} ms",
            samples[samples.len() / 2],
            samples[0],
            samples[samples.len() - 1]
        );
    }

    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
    }

    fn union_box(boxes: &[[f64; 4]]) -> [f64; 4] {
        let min_x = boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
        let min_y = boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min);
        let max_x = boxes
            .iter()
            .map(|b| b[0] + b[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = boxes
            .iter()
            .map(|b| b[1] + b[3])
            .fold(f64::NEG_INFINITY, f64::max);
        [min_x, min_y, max_x - min_x, max_y - min_y]
    }

    fn intersection_over_expected(expected: [f64; 4], got: [f64; 4]) -> f64 {
        let left = expected[0].max(got[0]);
        let top = expected[1].max(got[1]);
        let right = (expected[0] + expected[2]).min(got[0] + got[2]);
        let bottom = (expected[1] + expected[3]).min(got[1] + got[3]);
        let inter = (right - left).max(0.0) * (bottom - top).max(0.0);
        let area = expected[2] * expected[3];
        if area <= 0.0 {
            0.0
        } else {
            inter / area
        }
    }

    fn box_from_json(value: &Value) -> [f64; 4] {
        let b = value.as_array().expect("box array");
        [
            b[0].as_f64().expect("x"),
            b[1].as_f64().expect("y"),
            b[2].as_f64().expect("w"),
            b[3].as_f64().expect("h"),
        ]
    }

    #[test]
    fn ocr_boxes_finds_the_fixture_values_in_upright_and_exif6_space() {
        let dir = fixture_dir();
        let expected_raw = std::fs::read_to_string(dir.join("redact-sample.expected.json"))
            .expect("redact-sample.expected.json is committed");
        let expected: Value = serde_json::from_str(&expected_raw).expect("valid expected json");
        let expected = expected.as_array().expect("expected values array");

        for name in ["redact-sample.png", "redact-sample-rot6.jpg"] {
            let value = ocr_boxes(&dir.join(name), None).unwrap_or_else(|err| {
                panic!("{name}: ocr-boxes failed ({err}); build the bridge with ./native-bridge/build.sh")
            });
            assert_eq!(value["ok"].as_bool(), Some(true), "{name}: {value}");
            assert_eq!(value["width"].as_u64(), Some(1200), "{name}: upright width");
            assert_eq!(value["height"].as_u64(), Some(800), "{name}: upright height");

            let empty = Vec::new();
            let lines = value["lines"].as_array().unwrap_or(&empty);
            for entry in expected {
                let text = entry["text"].as_str().expect("expected text");
                let words: Vec<&str> = text.split_whitespace().collect();
                let expected_box = box_from_json(&entry["box"]);

                let mut recognised: Option<[f64; 4]> = None;
                'search: for line in lines {
                    let line_words = line["words"].as_array().expect("line words");
                    for window in line_words.windows(words.len()) {
                        let joined = window
                            .iter()
                            .map(|w| w["text"].as_str().expect("word text"))
                            .collect::<Vec<_>>()
                            .join("");
                        if joined == words.join("") {
                            let boxes: Vec<[f64; 4]> =
                                window.iter().map(|w| box_from_json(&w["box"])).collect();
                            recognised = Some(union_box(&boxes));
                            break 'search;
                        }
                    }
                }

                let got = recognised
                    .unwrap_or_else(|| panic!("{name}: words of {text:?} were not recognised"));
                let ratio = intersection_over_expected(expected_box, got);
                assert!(
                    ratio >= 0.6,
                    "{name}: {text:?} overlap {ratio:.3} (expected {expected_box:?}, got {got:?})"
                );
            }
        }
    }
}
