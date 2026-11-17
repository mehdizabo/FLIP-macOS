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

pub fn resolve_bridge_binary() -> Result<PathBuf, BridgeError> {
    if let Ok(explicit) = std::env::var("FLIP_BRIDGE_PATH") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Ok(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("flip-macos-bridge");
            if sibling.is_file() {
                return Ok(sibling);
            }
            if let Some(contents) = dir.parent() {
                let res = contents.join("Resources").join("flip-macos-bridge");
                if res.is_file() {
                    return Ok(res);
                }
            }
        }
    }

    // Development / cargo test path inside repo
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent() {
        let dev_bin = repo_root
            .join("native-bridge")
            .join("bin")
            .join("flip-macos-bridge");
        if dev_bin.is_file() {
            return Ok(dev_bin);
        }
    }

    Err(BridgeError::BridgeNotFound)
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
        std::thread::sleep(Duration::from_millis(20));
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    if !status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
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
