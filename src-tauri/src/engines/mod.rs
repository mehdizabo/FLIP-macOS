pub mod archive;
pub mod bridge;
pub mod create_new;
pub mod data;
pub mod document;
pub mod image;
pub mod media;
pub mod pdf;

use crate::core::logging::log_event;
use crate::core::naming::format_extension;
use crate::core::policy::{
    batch_actions, ext_lower, is_media_ext, outputs, validate_container, validate_input_files,
    EngineCapabilities, ARCHIVE_EXTS, IMAGE_EXTS,
};
use crate::core::preferences::{expand_tilde_path, AppPreferences};
use crate::core::staging::JobStaging;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;
use zip::ZipArchive;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Policy(#[from] crate::core::policy::PolicyError),
    #[error("{0}")]
    Staging(#[from] crate::core::staging::StagingError),
    #[error("{0}")]
    Bridge(#[from] bridge::BridgeError),
    #[error("Conversion cancelled")]
    Cancelled,
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    ConversionFailed(String),
    #[error("Validation failed for output `{0}`: {1}")]
    ValidationFailed(String, String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionResult {
    pub source: String,
    pub format: String,
    pub outputs: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub pandoc: Option<PathBuf>,
    pub bridge: Option<PathBuf>,
    pub pages: bool,
    pub numbers: bool,
    pub keynote: bool,
}

impl DetectedTools {
    pub fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            ffmpeg: self.ffmpeg.is_some(),
            pandoc: self.pandoc.is_some(),
            pages: self.pages,
            numbers: self.numbers,
            keynote: self.keynote,
        }
    }
}

pub fn detect_tools(prefs: Option<&AppPreferences>) -> DetectedTools {
    let ffmpeg = find_tool(
        prefs.and_then(|p| p.ffmpeg_path.as_deref()),
        "ffmpeg",
    );
    let ffprobe = ffmpeg
        .as_ref()
        .and_then(|p| p.parent().map(|dir| dir.join("ffprobe")))
        .filter(|p| p.is_file())
        .or_else(|| find_tool(None, "ffprobe"));
    let pandoc = find_tool(
        prefs.and_then(|p| p.pandoc_path.as_deref()),
        "pandoc",
    );
    let bridge = bridge::resolve_bridge_binary().ok();

    DetectedTools {
        ffmpeg,
        ffprobe,
        pandoc,
        bridge,
        pages: Path::new("/Applications/Pages.app").exists(),
        numbers: Path::new("/Applications/Numbers.app").exists(),
        keynote: Path::new("/Applications/Keynote.app").exists(),
    }
}

fn find_tool(override_path: Option<&str>, binary_name: &str) -> Option<PathBuf> {
    if let Some(custom) = override_path {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            let p = expand_tilde_path(trimmed);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    let app_support_tool = AppPreferences::app_support_dir()
        .join("tools")
        .join(binary_name);
    if app_support_tool.is_file() {
        return Some(app_support_tool);
    }

    for candidate in [
        format!("/opt/homebrew/bin/{binary_name}"),
        format!("/usr/local/bin/{binary_name}"),
        format!("/usr/bin/{binary_name}"),
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub type ProgressCallback<'a> = &'a dyn Fn(u8, &str);

static CONVERSION_GATE: OnceLock<Mutex<()>> = OnceLock::new();

fn gate() -> &'static Mutex<()> {
    CONVERSION_GATE.get_or_init(|| Mutex::new(()))
}

pub fn validate_output(path: &Path, format: &str) -> Result<(), EngineError> {
    let meta = fs::metadata(path).map_err(|e| {
        EngineError::ValidationFailed(path.display().to_string(), e.to_string())
    })?;
    let fmt = format.trim().to_ascii_uppercase();
    if fmt == "UNZIP" {
        if !meta.is_dir() {
            return Err(EngineError::ValidationFailed(
                path.display().to_string(),
                "Unzipped output is not a directory".to_string(),
            ));
        }
        return Ok(());
    }
    if meta.len() == 0 {
        return Err(EngineError::ValidationFailed(
            path.display().to_string(),
            "Output file is empty".to_string(),
        ));
    }

    match fmt.as_str() {
        "PDF" | "OPTIMIZE_PDF" => {
            use std::io::{Seek, SeekFrom};
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 5];
            let n = f.read(&mut hdr)?;
            if n < 5 || &hdr != b"%PDF-" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Missing %PDF- header".to_string(),
                ));
            }
            let tail_len = meta.len().min(2048);
            f.seek(SeekFrom::End(-(tail_len as i64)))?;
            let mut tail = vec![0u8; tail_len as usize];
            f.read_exact(&mut tail)?;
            if !tail.windows(5).any(|w| w == b"%%EOF") {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Truncated PDF missing %%EOF trailer".to_string(),
                ));
            }
        }
        "PNG" | "REMOVE_BG" | "JPG" | "JPEG" | "WEBP" | "BMP" | "TIFF" | "TIF" | "GIF" => {
            if let Err(e) = ::image::open(path) {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    format!("Image decode failed: {e}"),
                ));
            }
        }
        "ICNS" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || &hdr != b"icns" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid ICNS header".to_string(),
                ));
            }
        }
        "HEIC" | "MP4" | "MOV" | "M4V" | "M4A" | "ALAC" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 12];
            let n = f.read(&mut hdr)?;
            let valid_box = n >= 8
                && matches!(
                    &hdr[4..8],
                    b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip"
                );
            if !valid_box {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    format!("Invalid {fmt} ISO-BMFF container header"),
                ));
            }
        }
        "WAV" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 12];
            let n = f.read(&mut hdr)?;
            if n < 12 || &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid RIFF WAVE header".to_string(),
                ));
            }
        }
        "AIFF" | "AIF" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 12];
            let n = f.read(&mut hdr)?;
            if n < 12 || &hdr[0..4] != b"FORM" || (&hdr[8..12] != b"AIFF" && &hdr[8..12] != b"AIFC") {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid AIFF header".to_string(),
                ));
            }
        }
        "FLAC" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || &hdr != b"fLaC" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid FLAC header".to_string(),
                ));
            }
        }
        "OGG" | "OPUS" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || &hdr != b"OggS" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid Ogg container header".to_string(),
                ));
            }
        }
        "AVI" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 12];
            let n = f.read(&mut hdr)?;
            if n < 12 || &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"AVI " {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid RIFF AVI header".to_string(),
                ));
            }
        }
        "MKV" | "WEBM" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || hdr != [0x1A, 0x45, 0xDF, 0xA3] {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid Matroska/WebM EBML header".to_string(),
                ));
            }
        }
        "DOCX" | "ODT" | "EPUB" | "XLSX" | "PPTX" | "ZIP" => {
            let f = File::open(path)?;
            let archive = ZipArchive::new(f).map_err(|e| {
                EngineError::ValidationFailed(path.display().to_string(), e.to_string())
            })?;
            if archive.is_empty() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Empty archive".to_string(),
                ));
            }
        }
        "JSON" => {
            let raw = fs::read_to_string(path)?;
            serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| {
                EngineError::ValidationFailed(path.display().to_string(), e.to_string())
            })?;
        }
        "YAML" | "YML" => {
            let raw = fs::read_to_string(path)?;
            serde_yaml::from_str::<serde_yaml::Value>(&raw).map_err(|e| {
                EngineError::ValidationFailed(path.display().to_string(), e.to_string())
            })?;
        }
        "TOML" => {
            let raw = fs::read_to_string(path)?;
            toml::from_str::<toml::Value>(&raw).map_err(|e| {
                EngineError::ValidationFailed(path.display().to_string(), e.to_string())
            })?;
        }
        "XML" => {
            let raw = fs::read_to_string(path)?;
            let mut reader = quick_xml::Reader::from_str(&raw);
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(quick_xml::events::Event::Eof) => break,
                    Ok(_) => {}
                    Err(e) => {
                        return Err(EngineError::ValidationFailed(
                            path.display().to_string(),
                            e.to_string(),
                        ))
                    }
                }
                buf.clear();
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_unwritable_dir_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) || matches!(
        err.raw_os_error(),
        Some(libc::EACCES | libc::EPERM | libc::EROFS)
    )
}

fn probe_dir_writable(dir: &Path) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Not a directory: {}", dir.display()),
        ));
    }
    let meta = fs::metadata(dir)?;
    if meta.permissions().readonly() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Directory is read-only: {}", dir.display()),
        ));
    }
    let probe = dir.join(format!(".flip-probe-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&probe)?;
    let _ = fs::remove_dir(&probe);
    Ok(())
}

fn ensure_writable_dir(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    probe_dir_writable(dir)
}

fn default_downloads_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
    })
}

pub fn resolve_target_dir(source: &Path, prefs: &AppPreferences) -> Result<PathBuf, EngineError> {
    let parent = source.parent().ok_or_else(|| {
        EngineError::ConversionFailed(format!("Source has no parent folder: {}", source.display()))
    })?;
    let subfolder = prefs
        .output_subfolder
        .as_deref()
        .map(|s| s.trim().trim_matches('/'))
        .filter(|s| !s.is_empty() && !s.contains(".."));

    let primary = match subfolder {
        Some(sub) => parent.join(sub),
        None => parent.to_path_buf(),
    };

    let primary_res = if subfolder.is_some() {
        ensure_writable_dir(&primary)
    } else {
        probe_dir_writable(&primary)
    };

    match primary_res {
        Ok(()) => Ok(primary),
        Err(primary_err) if is_unwritable_dir_error(&primary_err) => {
            let fallback = prefs.resolved_fallback_folder();
            if ensure_writable_dir(&fallback).is_ok() {
                log_event(
                    "conversion.target_fallback",
                    Some(json!({
                        "sourceParent": parent.to_string_lossy(),
                        "targetDir": fallback.to_string_lossy(),
                        "reason": primary_err.to_string(),
                    })),
                );
                return Ok(fallback);
            }
            let downloads = default_downloads_dir();
            if downloads != fallback && ensure_writable_dir(&downloads).is_ok() {
                log_event(
                    "conversion.target_fallback",
                    Some(json!({
                        "sourceParent": parent.to_string_lossy(),
                        "targetDir": downloads.to_string_lossy(),
                        "reason": primary_err.to_string(),
                    })),
                );
                return Ok(downloads);
            }
            Err(EngineError::Io(primary_err))
        }
        Err(err) => Err(EngineError::Io(err)),
    }
}

pub fn convert_files(
    paths: &[PathBuf],
    formats: &[String],
    prefs: &AppPreferences,
    cancel: &Arc<AtomicBool>,
    progress: Option<ProgressCallback<'_>>,
) -> Result<Vec<ConversionResult>, EngineError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    let validated = validate_input_files(paths)?;
    let distinct_formats: Vec<String> = {
        let mut seen = Vec::new();
        for f in formats {
            let up = f.trim().trim_start_matches('.').to_ascii_uppercase();
            if !up.is_empty() && !seen.contains(&up) {
                seen.push(up);
            }
        }
        seen
    };
    if distinct_formats.is_empty() {
        return Err(EngineError::Unsupported(
            "Select at least one output format.".to_string(),
        ));
    }

    let _guard = gate()
        .lock()
        .map_err(|_| EngineError::ConversionFailed("Conversion gate poisoned".to_string()))?;

    let tools = detect_tools(Some(prefs));
    let caps = tools.capabilities();
    let mut results = Vec::new();

    // Handle batch-wide formats first (COMBINE_PDF / MERGE_PDF / ZIP)
    let mut per_file_formats = Vec::new();
    for fmt in &distinct_formats {
        if fmt == "COMBINE_PDF" || fmt == "MERGE_PDF" || fmt == "ZIP" {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            let first = &validated[0];
            let label = format!("{} files → {}", validated.len(), fmt);
            if let Some(cb) = progress {
                cb(15, &label);
            }
            let res = if fmt == "ZIP" {
                run_zip_archive(&validated, first, prefs, cancel)
            } else {
                run_batch_merge(&validated, fmt, first, prefs, cancel)
            };
            match res {
                Ok(committed) => {
                    results.push(ConversionResult {
                        source: first.to_string_lossy().into_owned(),
                        format: fmt.clone(),
                        outputs: committed
                            .into_iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        error: None,
                    });
                }
                Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                Err(e) => {
                    results.push(ConversionResult {
                        source: first.to_string_lossy().into_owned(),
                        format: fmt.clone(),
                        outputs: Vec::new(),
                        error: Some(e.to_string()),
                    });
                }
            }
        } else {
            per_file_formats.push(fmt.clone());
        }
    }

    let total_units = (validated.len() * per_file_formats.len()).max(1);
    let mut completed_units = 0usize;

    for source in &validated {
        for format in &per_file_formats {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }

            let file_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");
            let step_label = format!("{file_name} → {format}");
            let base_pct = ((completed_units * 100) / total_units).clamp(2, 95) as u8;
            if let Some(cb) = progress {
                cb(base_pct, &step_label);
            }

            match convert_single_unit(source, format, prefs, &tools, &caps, cancel, &|pct| {
                if let Some(cb) = progress {
                    let mapped = (((completed_units as f64) + (pct as f64 / 100.0)) * 100.0
                        / (total_units as f64))
                        .round()
                        .clamp(3.0, 98.0) as u8;
                    cb(mapped, &step_label);
                }
            }) {
                Ok(outputs_vec) => {
                    let out_strings: Vec<String> = outputs_vec
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    log_event(
                        "conversion.completed",
                        Some(json!({
                            "source": source.to_string_lossy(),
                            "format": format,
                            "outputs": &out_strings,
                        })),
                    );
                    results.push(ConversionResult {
                        source: source.to_string_lossy().into_owned(),
                        format: format.clone(),
                        outputs: out_strings,
                        error: None,
                    });
                }
                Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                Err(err) => {
                    log_event(
                        "conversion.failed",
                        Some(json!({
                            "source": source.to_string_lossy(),
                            "format": format,
                            "error": err.to_string(),
                        })),
                    );
                    results.push(ConversionResult {
                        source: source.to_string_lossy().into_owned(),
                        format: format.clone(),
                        outputs: Vec::new(),
                        error: Some(err.to_string()),
                    });
                }
            }

            completed_units += 1;
            if let Some(cb) = progress {
                let done_pct = ((completed_units * 100) / total_units).min(100) as u8;
                cb(done_pct, &step_label);
            }
        }
    }

    Ok(results)
}

fn run_batch_merge(
    sources: &[PathBuf],
    action: &str,
    anchor: &Path,
    prefs: &AppPreferences,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<PathBuf>, EngineError> {
    if !batch_actions(sources).contains(&action.to_string()) {
        return Err(EngineError::Unsupported(format!(
            "Batch action `{action}` is not valid for this file selection."
        )));
    }
    let target_dir = resolve_target_dir(anchor, prefs)?;
    let staging = JobStaging::create(&target_dir)?;
    let staged_pdf = staging.staged_path("combined.pdf");

    if action == "COMBINE_PDF" {
        image::combine_into_pdf(sources, &staged_pdf, staging.dir(), cancel)?;
    } else {
        bridge::convert_pdf_merge(&staged_pdf, sources, Some(cancel))?;
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    validate_output(&staged_pdf, "PDF")?;
    let committed = staging.commit_all(&[staged_pdf], anchor, "PDF", false)?;
    staging.cleanup()?;
    Ok(committed)
}

fn run_zip_archive(
    sources: &[PathBuf],
    anchor: &Path,
    prefs: &AppPreferences,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<PathBuf>, EngineError> {
    if sources.len() == 1 && ext_lower(&sources[0]) == "zip" {
        return Err(EngineError::Unsupported(
            "Use UNZIP on a single .zip archive.".to_string(),
        ));
    }
    if sources.len() >= 2 && !batch_actions(sources).contains(&"ZIP".to_string()) {
        return Err(EngineError::Unsupported(
            "Batch action `ZIP` is not valid for this file selection.".to_string(),
        ));
    }
    let target_dir = resolve_target_dir(anchor, prefs)?;
    let staging = JobStaging::create(&target_dir)?;
    let staged_zip = staging.staged_path("archive.zip");

    archive::zip_files(sources, &staged_zip, cancel)?;

    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    validate_output(&staged_zip, "ZIP")?;
    let committed = staging.commit_all(&[staged_zip], anchor, "ZIP", false)?;
    staging.cleanup()?;
    Ok(committed)
}

fn convert_single_unit(
    source: &Path,
    format: &str,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    caps: &EngineCapabilities,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let available = outputs(&[source], caps);
    if !available.contains(&format.to_string()) {
        return Err(EngineError::Unsupported(format!(
            "Conversion unavailable: .{} → {}",
            ext_lower(source),
            format
        )));
    }

    validate_container(source)?;

    let ext = ext_lower(source);
    let target_dir = resolve_target_dir(source, prefs)?;
    let staging = JobStaging::create(&target_dir)?;

    let staged_outputs: Vec<PathBuf> = if IMAGE_EXTS.contains(&ext.as_str()) {
        unit_progress(35);
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        image::convert(
            source,
            format,
            &out,
            staging.dir(),
            image::ImageEncodeOptions {
                quality_pct: prefs.image_quality,
                preserve_exif: prefs.preserve_exif,
                strip_gps_exif: prefs.strip_gps_exif,
            },
            cancel,
        )?;
        vec![out]
    } else if crate::core::policy::DATA_EXTS.contains(&ext.as_str()) {
        unit_progress(40);
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        data::convert(source, format, &out, staging.dir(), &prefs.pdf_page_size, cancel)?;
        vec![out]
    } else if is_media_ext(&ext) {
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        media::convert(source, format, &out, tools, cancel, unit_progress)?;
        vec![out]
    } else if ext == "pdf" {
        pdf::convert(source, format, staging.dir(), prefs, tools, cancel, unit_progress)?
    } else if ARCHIVE_EXTS.contains(&ext.as_str()) {
        unit_progress(35);
        let extract_dir = staging.staged_path("extracted");
        archive::unzip_archive(source, &extract_dir, cancel)?;
        vec![extract_dir]
    } else {
        document::convert(source, format, staging.dir(), prefs, tools, cancel, unit_progress)?
    };

    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    if staged_outputs.is_empty() {
        return Err(EngineError::ConversionFailed(
            "Engine produced zero output files".to_string(),
        ));
    }

    for staged in &staged_outputs {
        validate_output(staged, format)?;
    }

    let is_multipage = staged_outputs.len() > 1
        || ((ext == "pdf"
            || crate::core::policy::DOC_NATIVE_EXTS.contains(&ext.as_str())
            || crate::core::policy::DOC_PANDOC_ONLY_EXTS.contains(&ext.as_str()))
            && matches!(format, "PNG" | "JPG" | "WEBP" | "HEIC" | "TIFF"));

    let committed = staging.commit_all(&staged_outputs, source, format, is_multipage)?;
    staging.cleanup()?;
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwritable_dir_error_classifies_readonly_and_permission_codes() {
        assert!(is_unwritable_dir_error(&std::io::Error::from_raw_os_error(
            libc::EROFS
        )));
        assert!(is_unwritable_dir_error(&std::io::Error::from_raw_os_error(
            libc::EACCES
        )));
        assert!(is_unwritable_dir_error(&std::io::Error::from_raw_os_error(
            libc::EPERM
        )));
        assert!(is_unwritable_dir_error(&std::io::Error::new(
            std::io::ErrorKind::ReadOnlyFilesystem,
            "rofs"
        )));
        assert!(!is_unwritable_dir_error(&std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing"
        )));
    }
}
