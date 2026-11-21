pub mod archive;
pub mod bridge;
pub mod create_new;
pub mod data;
pub mod document;
pub mod image;
pub mod image_tools;
pub mod media;
pub mod pdf;
pub mod pdfa;
pub mod redact;
pub mod transcribe;

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

impl EngineError {
    pub fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Bridge(bridge::BridgeError::Cancelled)
        )
    }
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
    /// The whisper.cpp command line binary (`whisper-cli`), when installed.
    pub whisper: Option<PathBuf>,
    /// The multilingual `ggml-small.bin` model file, when present in the app support folder.
    pub whisper_model: Option<PathBuf>,
    /// The Ghostscript command line binary (`gs`), when installed; PDF/A needs it.
    pub ghostscript: Option<PathBuf>,
}

impl DetectedTools {
    pub fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            ffmpeg: self.ffmpeg.is_some(),
            pandoc: self.pandoc.is_some(),
            pages: self.pages,
            numbers: self.numbers,
            keynote: self.keynote,
            // Transcription needs all three parts: the binary, the model and ffmpeg.
            whisper: self.whisper.is_some() && self.whisper_model.is_some() && self.ffmpeg.is_some(),
            // PDF/A needs all three parts: Ghostscript, the native bridge (PDFKit pre-pass and
            // inspection) and the sRGB profile that ships with this Ghostscript install.
            ghostscript: self.bridge.is_some()
                && self
                    .ghostscript
                    .as_deref()
                    .is_some_and(|gs| pdfa::srgb_profile_for(gs).is_some()),
        }
    }
}

pub fn detect_tools(prefs: Option<&AppPreferences>) -> DetectedTools {
    detect_tools_with(prefs, &|key| std::env::var(key).ok())
}

fn env_flag_enabled(raw: Option<&str>) -> bool {
    raw.map(|v| {
        let trimmed = v.trim().to_ascii_lowercase();
        matches!(trimmed.as_str(), "1" | "true" | "yes" | "on")
    })
    .unwrap_or(false)
}

fn override_disables_tool(override_path: Option<&str>) -> bool {
    override_path
        .map(|raw| {
            let trimmed = raw.trim();
            trimmed.eq_ignore_ascii_case("none") || trimmed.eq_ignore_ascii_case("disabled")
        })
        .unwrap_or(false)
}

pub fn detect_tools_with(
    prefs: Option<&AppPreferences>,
    get_env: &dyn Fn(&str) -> Option<String>,
) -> DetectedTools {
    let ffmpeg_override = prefs.and_then(|p| p.ffmpeg_path.as_deref());
    let disable_ffmpeg = env_flag_enabled(get_env("FLIP_DISABLE_FFMPEG").as_deref())
        || override_disables_tool(ffmpeg_override);
    let ffmpeg = if disable_ffmpeg {
        None
    } else {
        find_tool(ffmpeg_override, "ffmpeg")
    };
    let ffprobe = if disable_ffmpeg {
        None
    } else {
        ffmpeg
            .as_ref()
            .and_then(|p| p.parent().map(|dir| dir.join("ffprobe")))
            .filter(|p| p.is_file())
            .or_else(|| find_tool(None, "ffprobe"))
    };

    let pandoc_override = prefs.and_then(|p| p.pandoc_path.as_deref());
    let disable_pandoc = env_flag_enabled(get_env("FLIP_DISABLE_PANDOC").as_deref())
        || override_disables_tool(pandoc_override);
    let pandoc = if disable_pandoc {
        None
    } else {
        find_tool(pandoc_override, "pandoc")
    };
    let bridge = bridge::resolve_bridge_binary().ok();

    // `FLIP_DISABLE_WHISPER=1` hides both the binary and the model, the same way the other flags do.
    let disable_whisper = env_flag_enabled(get_env("FLIP_DISABLE_WHISPER").as_deref());
    let whisper = if disable_whisper {
        None
    } else {
        // Homebrew renamed the binary between whisper.cpp versions, so accept both names.
        find_tool(None, "whisper-cli").or_else(|| find_tool(None, "whisper-cpp"))
    };
    let whisper_model = if disable_whisper {
        None
    } else {
        let model = transcribe::model_path();
        if model.is_file() {
            Some(model)
        } else {
            None
        }
    };

    // `FLIP_DISABLE_GHOSTSCRIPT=1` hides Ghostscript, so PDF/A is not offered (same pattern as Whisper).
    let ghostscript = if env_flag_enabled(get_env("FLIP_DISABLE_GHOSTSCRIPT").as_deref()) {
        None
    } else {
        find_tool(None, "gs")
    };

    DetectedTools {
        ffmpeg,
        ffprobe,
        pandoc,
        bridge,
        pages: Path::new("/Applications/Pages.app").exists(),
        numbers: Path::new("/Applications/Numbers.app").exists(),
        keynote: Path::new("/Applications/Keynote.app").exists(),
        whisper,
        whisper_model,
        ghostscript,
    }
}

fn find_tool(override_path: Option<&str>, binary_name: &str) -> Option<PathBuf> {
    if override_disables_tool(override_path) {
        return None;
    }
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
    validate_output_with_mode(path, format, false)
}

pub fn validate_output_with_mode(
    path: &Path,
    format: &str,
    raw_mode: bool,
) -> Result<(), EngineError> {
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

    if raw_mode
        && matches!(
            fmt.as_str(),
            "JSON" | "YAML" | "YML" | "TOML" | "FB2" | "XML" | "RST" | "TXT" | "MD" | "HTML" | "CSV"
        )
    {
        let raw = fs::read_to_string(path).map_err(|e| {
            EngineError::ValidationFailed(path.display().to_string(), e.to_string())
        })?;
        if raw.is_empty() {
            return Err(EngineError::ValidationFailed(
                path.display().to_string(),
                format!("{fmt} output is empty"),
            ));
        }
        return Ok(());
    }

    match fmt.as_str() {
        "PDF" | "OPTIMIZE_PDF" | "PDFA" => {
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
        "ICO" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || hdr != [0x00, 0x00, 0x01, 0x00] {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid ICO header".to_string(),
                ));
            }
            if let Err(e) = ::image::open(path) {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    format!("ICO decode failed: {e}"),
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
            if meta.len() <= 42 {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "FLAC output contains header only without audio frames".to_string(),
                ));
            }
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || &hdr != b"fLaC" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid FLAC header".to_string(),
                ));
            }
        }
        "CAF" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 4];
            if f.read(&mut hdr)? < 4 || &hdr != b"caff" {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid Core Audio Format (CAF) header".to_string(),
                ));
            }
        }
        "AAC" => {
            let mut f = File::open(path)?;
            let mut hdr = [0u8; 2];
            if f.read(&mut hdr)? < 2 || hdr[0] != 0xFF || (hdr[1] & 0xF0) != 0xF0 {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid AAC ADTS header".to_string(),
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
            let mut archive = ZipArchive::new(f).map_err(|e| {
                EngineError::ValidationFailed(path.display().to_string(), e.to_string())
            })?;
            if archive.is_empty() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Empty archive".to_string(),
                ));
            }
            if fmt == "XLSX" && archive.by_name("xl/workbook.xml").is_err() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Invalid XLSX archive missing xl/workbook.xml".to_string(),
                ));
            }
        }
        "TRANSCRIBE" => {
            let raw = fs::read_to_string(path).map_err(|e| {
                EngineError::ValidationFailed(
                    path.display().to_string(),
                    format!("Transcript is not valid UTF-8 text: {e}"),
                )
            })?;
            if raw.trim().is_empty() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Transcript is empty".to_string(),
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
        "FB2" => {
            let raw = fs::read_to_string(path)?;
            if raw.trim().is_empty() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "FB2 output is empty".to_string(),
                ));
            }
            let mut reader = quick_xml::Reader::from_str(&raw);
            let mut buf = Vec::new();
            let mut root_name: Option<String> = None;
            let mut depth: usize = 0;
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(quick_xml::events::Event::Start(ref e)) => {
                        if depth == 0 && root_name.is_some() {
                            return Err(EngineError::ValidationFailed(
                                path.display().to_string(),
                                "Multiple XML root elements in FB2".to_string(),
                            ));
                        }
                        if root_name.is_none() {
                            root_name = Some(
                                String::from_utf8_lossy(e.local_name().as_ref()).to_string(),
                            );
                        }
                        depth += 1;
                    }
                    Ok(quick_xml::events::Event::Empty(ref e)) => {
                        if depth == 0 && root_name.is_some() {
                            return Err(EngineError::ValidationFailed(
                                path.display().to_string(),
                                "Multiple XML root elements in FB2".to_string(),
                            ));
                        }
                        if root_name.is_none() {
                            root_name = Some(
                                String::from_utf8_lossy(e.local_name().as_ref()).to_string(),
                            );
                        }
                    }
                    Ok(quick_xml::events::Event::End(_)) => {
                        depth = depth.saturating_sub(1);
                    }
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
            if depth != 0 {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Truncated FB2 XML with unclosed element".to_string(),
                ));
            }
            if !root_name
                .as_deref()
                .map(|r| r.eq_ignore_ascii_case("FictionBook"))
                .unwrap_or(false)
            {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    format!("Expected <FictionBook> root element, found {root_name:?}"),
                ));
            }
        }
        "RST" => {
            let raw = fs::read_to_string(path)?;
            if raw.trim().is_empty() {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "RST output is empty".to_string(),
                ));
            }
        }
        "XML" => {
            let raw = fs::read_to_string(path)?;
            let mut reader = quick_xml::Reader::from_str(&raw);
            let mut buf = Vec::new();
            let mut saw_root = false;
            let mut depth: usize = 0;
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(quick_xml::events::Event::Start(_)) => {
                        if depth == 0 && saw_root {
                            return Err(EngineError::ValidationFailed(
                                path.display().to_string(),
                                "Multiple XML root elements".to_string(),
                            ));
                        }
                        saw_root = true;
                        depth += 1;
                    }
                    Ok(quick_xml::events::Event::Empty(_)) => {
                        if depth == 0 && saw_root {
                            return Err(EngineError::ValidationFailed(
                                path.display().to_string(),
                                "Multiple XML root elements".to_string(),
                            ));
                        }
                        saw_root = true;
                    }
                    Ok(quick_xml::events::Event::End(_)) => {
                        depth = depth.saturating_sub(1);
                    }
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
            if !saw_root || depth != 0 {
                return Err(EngineError::ValidationFailed(
                    path.display().to_string(),
                    "Truncated or empty XML document".to_string(),
                ));
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
    convert_files_with_options(paths, formats, prefs, cancel, progress, false)
}

pub fn convert_files_with_options(
    paths: &[PathBuf],
    formats: &[String],
    prefs: &AppPreferences,
    cancel: &Arc<AtomicBool>,
    progress: Option<ProgressCallback<'_>>,
    raw_mode: bool,
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

    // Handle batch-wide formats first (COMBINE_PDF / ANIMATE_GIF / MERGE_PDF / ZIP)
    let mut per_file_formats = Vec::new();
    for fmt in &distinct_formats {
        if fmt == "COMBINE_PDF" || fmt == "ANIMATE_GIF" || fmt == "MERGE_PDF" || fmt == "ZIP" {
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
                Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
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

            match convert_single_unit(
                source,
                format,
                prefs,
                &tools,
                &caps,
                cancel,
                raw_mode,
                &|pct| {
                    if let Some(cb) = progress {
                        let mapped = (((completed_units as f64) + (pct as f64 / 100.0)) * 100.0
                            / (total_units as f64))
                            .round()
                            .clamp(3.0, 98.0) as u8;
                        cb(mapped, &step_label);
                    }
                },
            ) {
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
                            "rawMode": raw_mode,
                        })),
                    );
                    results.push(ConversionResult {
                        source: source.to_string_lossy().into_owned(),
                        format: format.clone(),
                        outputs: out_strings,
                        error: None,
                    });
                }
                Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
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

    let (staged_out, target_format) = match action {
        "COMBINE_PDF" => {
            let staged_pdf = staging.staged_path("combined.pdf");
            image::combine_into_pdf(sources, &staged_pdf, staging.dir(), cancel)?;
            (staged_pdf, "PDF")
        }
        "ANIMATE_GIF" => {
            let staged_gif = staging.staged_path("animated.gif");
            image::animate_into_gif(sources, &staged_gif, staging.dir(), cancel)?;
            (staged_gif, "GIF")
        }
        "MERGE_PDF" => {
            let staged_pdf = staging.staged_path("merged.pdf");
            bridge::convert_pdf_merge(&staged_pdf, sources, Some(cancel))?;
            (staged_pdf, "PDF")
        }
        _ => {
            return Err(EngineError::Unsupported(format!(
                "Unsupported batch action `{action}`."
            )));
        }
    };

    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    validate_output(&staged_out, target_format)?;
    let committed = staging.commit_all(&[staged_out], anchor, target_format, false)?;
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

pub fn is_raw_text_source_ext(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "html"
            | "htm"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "csv"
            | "rst"
            | "fb2"
            | "svg"
    )
}

fn is_raw_text_target_format(format: &str) -> bool {
    matches!(
        format,
        "TXT" | "MD" | "HTML" | "RST" | "CSV" | "JSON" | "YAML" | "YML" | "TOML" | "XML" | "FB2"
    )
}

fn is_raw_rendered_target_format(format: &str) -> bool {
    matches!(
        format,
        "PDF" | "DOCX" | "RTF" | "ODT" | "EPUB" | "PNG" | "JPG" | "WEBP" | "HEIC"
    )
}

fn convert_raw_text_unit(
    source: &Path,
    format: &str,
    staging: &JobStaging,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Option<Vec<PathBuf>>, EngineError> {
    if !is_raw_text_target_format(format) && !is_raw_rendered_target_format(format) {
        return Ok(None);
    }

    let meta = fs::metadata(source)?;
    if meta.len() > document::MAX_HTML_READ_BYTES {
        return Err(EngineError::ConversionFailed(format!(
            "Source file exceeds {} MiB raw text read limit",
            document::MAX_HTML_READ_BYTES / (1024 * 1024)
        )));
    }

    let raw_text = fs::read_to_string(source).map_err(|e| {
        EngineError::ConversionFailed(format!(
            "Raw mode requires valid UTF-8 text source `{}`: {e}",
            source.display()
        ))
    })?;

    if is_raw_text_target_format(format) {
        unit_progress(60);
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        fs::write(&out, raw_text.as_bytes())?;
        return Ok(Some(vec![out]));
    }

    let raw_txt_path = staging.dir().join("raw-source.txt");
    fs::write(&raw_txt_path, raw_text.as_bytes())?;
    let outputs_vec = document::convert(
        &raw_txt_path,
        format,
        staging.dir(),
        prefs,
        tools,
        cancel,
        unit_progress,
    )?;
    Ok(Some(outputs_vec))
}

/// Transcribes one audio or video file; every tool arrives from `DetectedTools`, nothing is searched.
fn convert_transcribe_unit(
    source: &Path,
    staging: &JobStaging,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let whisper_cli = tools.whisper.as_deref().ok_or_else(|| {
        EngineError::Unsupported(
            "Transcribe needs the whisper.cpp command line tool (whisper-cli), which is not installed."
                .to_string(),
        )
    })?;
    let model = tools.whisper_model.as_deref().ok_or_else(|| {
        EngineError::Unsupported(format!(
            "Transcribe needs the model file `{}`, which is missing.",
            transcribe::model_path().display()
        ))
    })?;
    let ffmpeg = tools.ffmpeg.as_deref().ok_or_else(|| {
        EngineError::Unsupported(
            "Transcribe needs FFmpeg to decode the audio, and it is not installed.".to_string(),
        )
    })?;

    let out = staging.staged_path(&format!("output.{}", format_extension("TRANSCRIBE")));
    let engine_tools = transcribe::TranscribeTools {
        whisper_cli,
        model,
        ffmpeg,
    };
    transcribe::transcribe(source, &out, &engine_tools, staging.dir(), cancel, unit_progress)?;
    Ok(vec![out])
}

/// Converts one PDF to PDF/A-2b; both tools arrive from `DetectedTools`, nothing is searched.
fn convert_pdfa_unit(
    source: &Path,
    staging: &JobStaging,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let ghostscript = tools.ghostscript.as_deref().ok_or_else(|| {
        EngineError::Unsupported(
            "PDF/A needs Ghostscript (gs), which is not installed.".to_string(),
        )
    })?;
    let bridge = tools.bridge.as_deref().ok_or_else(|| {
        EngineError::Unsupported(
            "PDF/A needs FLIP's native bridge to read and prepare the PDF, and it was not found."
                .to_string(),
        )
    })?;

    let out = staging.staged_path(&format!("output.{}", format_extension("PDFA")));
    let engine_tools = pdfa::PdfaTools {
        ghostscript,
        bridge,
    };
    unit_progress(35);
    pdfa::convert(source, &out, &engine_tools, staging.dir(), cancel)?;
    Ok(vec![out])
}

#[allow(clippy::too_many_arguments)]
fn convert_standard_unit(
    source: &Path,
    ext: &str,
    format: &str,
    staging: &JobStaging,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    if IMAGE_EXTS.contains(&ext) {
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
        Ok(vec![out])
    } else if crate::core::policy::DATA_EXTS.contains(&ext) {
        unit_progress(40);
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        data::convert(source, format, &out, staging.dir(), &prefs.pdf_page_size, cancel)?;
        Ok(vec![out])
    } else if is_media_ext(ext) {
        if format == "TRANSCRIBE" {
            // The whole job holds the global conversion gate, so a long transcription blocks other
            // conversions until it finishes. That is intentional: one external tool at a time.
            return convert_transcribe_unit(source, staging, tools, cancel, unit_progress);
        }
        let out = staging.staged_path(&format!("output.{}", format_extension(format)));
        media::convert(source, format, &out, tools, cancel, unit_progress)?;
        Ok(vec![out])
    } else if ext == "pdf" {
        if format == "PDFA" {
            // The whole job holds the global conversion gate, so a long PDF/A conversion blocks other
            // conversions until it finishes. That is intentional: one external tool at a time.
            return convert_pdfa_unit(source, staging, tools, cancel, unit_progress);
        }
        pdf::convert(source, format, staging.dir(), prefs, tools, cancel, unit_progress)
    } else if ARCHIVE_EXTS.contains(&ext) {
        unit_progress(35);
        let extract_dir = staging.staged_path("extracted");
        archive::unzip_archive(source, &extract_dir, cancel)?;
        Ok(vec![extract_dir])
    } else {
        document::convert(source, format, staging.dir(), prefs, tools, cancel, unit_progress)
    }
}

/// Image tools keep the source's format, so the output format is resolved from the source and the
/// result is committed under a descriptive suffix (`photo-crop-9x16.jpg`), never over the original.
fn convert_image_tool_unit(
    source: &Path,
    tool: image_tools::ImageTool,
    prefs: &AppPreferences,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let ext = ext_lower(source);
    if !image_tools::tools_for_ext(&ext).contains(&tool.token()) {
        return Err(EngineError::Unsupported(format!(
            "Tool unavailable: .{ext} → {}",
            tool.token()
        )));
    }

    validate_container(source)?;
    let target_dir = resolve_target_dir(source, prefs)?;
    let staging = JobStaging::create(&target_dir)?;
    let out_format = tool.output_format(&ext);
    let staged = staging.staged_path(&format!("output.{}", format_extension(out_format)));

    unit_progress(35);
    image_tools::apply(tool, source, &staged, staging.dir(), prefs.image_quality, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    validate_output(&staged, out_format)?;
    let committed = staging.commit_one(&staged, source, Some(tool.suffix()), out_format)?;
    staging.cleanup()?;
    Ok(vec![committed])
}

#[allow(clippy::too_many_arguments)]
fn convert_single_unit(
    source: &Path,
    format: &str,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    caps: &EngineCapabilities,
    cancel: &Arc<AtomicBool>,
    raw_mode: bool,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    if let Some(tool) = image_tools::ImageTool::parse(format) {
        return convert_image_tool_unit(source, tool, prefs, cancel, unit_progress);
    }

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

    let staged_outputs: Vec<PathBuf> = if raw_mode && is_raw_text_source_ext(&ext) {
        if let Some(raw_outputs) = convert_raw_text_unit(
            source,
            format,
            &staging,
            prefs,
            tools,
            cancel,
            unit_progress,
        )? {
            raw_outputs
        } else {
            convert_standard_unit(
                source,
                &ext,
                format,
                &staging,
                prefs,
                tools,
                cancel,
                unit_progress,
            )?
        }
    } else {
        convert_standard_unit(
            source,
            &ext,
            format,
            &staging,
            prefs,
            tools,
            cancel,
            unit_progress,
        )?
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
        validate_output_with_mode(staged, format, raw_mode)?;
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

    #[test]
    fn detect_tools_with_honors_env_disable_flags() {
        let disabled = detect_tools_with(None, &|key| match key {
            "FLIP_DISABLE_FFMPEG" => Some("1".to_string()),
            "FLIP_DISABLE_PANDOC" => Some("true".to_string()),
            _ => None,
        });
        assert!(disabled.ffmpeg.is_none());
        assert!(disabled.ffprobe.is_none());
        assert!(disabled.pandoc.is_none());

        let only_ffmpeg_flag_zero = detect_tools_with(None, &|key| match key {
            "FLIP_DISABLE_FFMPEG" => Some("0".to_string()),
            _ => None,
        });
        // "0" does not forcibly disable an installed ffmpeg
        assert_eq!(
            only_ffmpeg_flag_zero.ffmpeg.is_some(),
            find_tool(None, "ffmpeg").is_some()
        );
    }

    #[test]
    fn detect_tools_with_honors_none_and_disabled_path_overrides() {
        let prefs = AppPreferences {
            ffmpeg_path: Some("none".to_string()),
            pandoc_path: Some(" DISABLED ".to_string()),
            ..Default::default()
        };
        let tools = detect_tools_with(Some(&prefs), &|_| None);
        assert!(tools.ffmpeg.is_none());
        assert!(tools.ffprobe.is_none());
        assert!(tools.pandoc.is_none());
        assert!(find_tool(Some("none"), "ffmpeg").is_none());
        assert!(find_tool(Some("disabled"), "pandoc").is_none());
    }

    #[test]
    fn detect_tools_with_honors_whisper_disable_flag() {
        let disabled = detect_tools_with(None, &|key| match key {
            "FLIP_DISABLE_WHISPER" => Some("1".to_string()),
            _ => None,
        });
        assert!(disabled.whisper.is_none());
        assert!(disabled.whisper_model.is_none());

        // Without the flag the result mirrors what this machine actually has.
        let enabled = detect_tools_with(None, &|_| None);
        let expected_binary =
            find_tool(None, "whisper-cli").or_else(|| find_tool(None, "whisper-cpp"));
        assert_eq!(enabled.whisper.is_some(), expected_binary.is_some());
        assert_eq!(enabled.whisper_model.is_some(), transcribe::model_path().is_file());
    }

    #[test]
    fn whisper_capability_needs_binary_model_and_ffmpeg() {
        let base = DetectedTools {
            ffmpeg: Some(PathBuf::from("/opt/homebrew/bin/ffmpeg")),
            ffprobe: None,
            pandoc: None,
            bridge: None,
            pages: false,
            numbers: false,
            keynote: false,
            whisper: Some(PathBuf::from("/opt/homebrew/bin/whisper-cli")),
            whisper_model: Some(PathBuf::from("/tmp/ggml-small.bin")),
            ghostscript: None,
        };
        assert!(base.capabilities().whisper);

        let mut no_binary = base.clone();
        no_binary.whisper = None;
        assert!(!no_binary.capabilities().whisper);

        let mut no_model = base.clone();
        no_model.whisper_model = None;
        assert!(!no_model.capabilities().whisper);

        let mut no_ffmpeg = base.clone();
        no_ffmpeg.ffmpeg = None;
        assert!(!no_ffmpeg.capabilities().whisper);
    }

    #[test]
    fn convert_single_unit_routes_transcribe_through_fake_tools() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fakes = dir.path().join("fakes");
        fs::create_dir(&fakes).unwrap();

        // Fake ffmpeg: decoding writes a stub file to its last argument; `volumedetect` reports
        // a loud enough peak and writes nothing.
        let ffmpeg = fakes.join("ffmpeg");
        fs::write(
            &ffmpeg,
            "#!/bin/sh\ncase \"$*\" in\n  *volumedetect*)\n    printf '[Parsed_volumedetect_0 @ 0x0] max_volume: -12.0 dB\\n' >&2\n    exit 0 ;;\nesac\nfor last do :; done\nprintf 'RIFFfake' > \"$last\"\n",
        )
        .unwrap();
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();

        // Fake whisper-cli: writes the transcript next to the `-of` prefix.
        let whisper = fakes.join("whisper-cli");
        fs::write(
            &whisper,
            "#!/bin/sh\nprefix=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -of) prefix=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nprintf 'Hello from the fake whisper.\\n' > \"$prefix.txt\"\n",
        )
        .unwrap();
        fs::set_permissions(&whisper, fs::Permissions::from_mode(0o755)).unwrap();

        let model = fakes.join("ggml-small.bin");
        fs::write(&model, b"fake-model-bytes").unwrap();

        let source = dir.path().join("meeting.m4a");
        fs::write(&source, b"fake audio bytes").unwrap();

        let tools = DetectedTools {
            ffmpeg: Some(ffmpeg),
            ffprobe: None,
            pandoc: None,
            bridge: None,
            pages: false,
            numbers: false,
            keynote: false,
            whisper: Some(whisper),
            whisper_model: Some(model),
            ghostscript: None,
        };
        let caps = tools.capabilities();
        assert!(caps.whisper);

        let prefs = AppPreferences::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let outputs_vec = convert_single_unit(
            &source,
            "TRANSCRIBE",
            &prefs,
            &tools,
            &caps,
            &cancel,
            false,
            &|_| {},
        )
        .unwrap();
        assert_eq!(outputs_vec.len(), 1);
        assert_eq!(
            outputs_vec[0].file_name().and_then(|n| n.to_str()),
            Some("meeting.txt")
        );
        assert_eq!(
            fs::read_to_string(&outputs_vec[0]).unwrap(),
            "Hello from the fake whisper.\n"
        );
        assert!(validate_output(&outputs_vec[0], "TRANSCRIBE").is_ok());

        // A missing model must be a clean Unsupported error, never a panic.
        let mut without_model = tools.clone();
        without_model.whisper_model = None;
        let staging = JobStaging::create(dir.path()).unwrap();
        let err = convert_standard_unit(
            &source,
            "m4a",
            "TRANSCRIBE",
            &staging,
            &prefs,
            &without_model,
            &cancel,
            &|_| {},
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::Unsupported(_)), "{err}");
        staging.cleanup().unwrap();
    }

    #[test]
    fn validate_output_checks_transcribe_utf8_and_non_blank() {
        let dir = tempfile::tempdir().unwrap();

        let good = dir.path().join("good.txt");
        fs::write(&good, "Ciao, questo è un transcript.\n").unwrap();
        assert!(validate_output(&good, "TRANSCRIBE").is_ok());

        let blank = dir.path().join("blank.txt");
        fs::write(&blank, "  \n\t\n").unwrap();
        let blank_err = validate_output(&blank, "TRANSCRIBE").unwrap_err();
        assert!(blank_err.to_string().contains("Transcript is empty"), "{blank_err}");

        let empty = dir.path().join("empty.txt");
        fs::write(&empty, b"").unwrap();
        assert!(validate_output(&empty, "TRANSCRIBE").is_err());

        let invalid = dir.path().join("invalid.txt");
        fs::write(&invalid, [0xFF, 0xFE, 0x00, 0x41]).unwrap();
        let invalid_err = validate_output(&invalid, "TRANSCRIBE").unwrap_err();
        assert!(
            invalid_err.to_string().contains("not valid UTF-8"),
            "{invalid_err}"
        );
    }

    #[test]
    fn detect_tools_with_honors_ghostscript_disable_flag() {
        let disabled = detect_tools_with(None, &|key| match key {
            "FLIP_DISABLE_GHOSTSCRIPT" => Some("1".to_string()),
            _ => None,
        });
        assert!(disabled.ghostscript.is_none());
        assert!(!disabled.capabilities().ghostscript);

        // `0` does not disable anything: the result mirrors what this machine actually has.
        let expected = find_tool(None, "gs");
        for value in [None, Some("0")] {
            let enabled = detect_tools_with(None, &|key| match key {
                "FLIP_DISABLE_GHOSTSCRIPT" => value.map(String::from),
                _ => None,
            });
            assert_eq!(enabled.ghostscript, expected, "flag value {value:?}");
        }
    }

    /// Builds `<root>/bin/gs` (a stub) and, when `with_profile` is true, the flat sRGB layout next to it.
    fn fake_ghostscript_install(root: &Path, with_profile: bool) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        fs::create_dir_all(root.join("bin")).unwrap();
        let gs = root.join("bin").join("gs");
        fs::write(&gs, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&gs, fs::Permissions::from_mode(0o755)).unwrap();
        if with_profile {
            let icc = root.join("share").join("ghostscript").join("iccprofiles");
            fs::create_dir_all(&icc).unwrap();
            fs::write(icc.join("srgb.icc"), b"fake icc").unwrap();
        }
        gs
    }

    fn pdfa_test_tools(gs: Option<PathBuf>, bridge: Option<PathBuf>) -> DetectedTools {
        DetectedTools {
            ffmpeg: None,
            ffprobe: None,
            pandoc: None,
            bridge,
            pages: false,
            numbers: false,
            keynote: false,
            whisper: None,
            whisper_model: None,
            ghostscript: gs,
        }
    }

    #[test]
    fn ghostscript_capability_needs_gs_bridge_and_srgb_profile() {
        let dir = tempfile::tempdir().unwrap();
        let complete = fake_ghostscript_install(&dir.path().join("complete"), true);
        let no_profile = fake_ghostscript_install(&dir.path().join("no-profile"), false);
        let bridge = dir.path().join("flip-macos-bridge");
        fs::write(&bridge, b"stub").unwrap();

        let all_three = pdfa_test_tools(Some(complete.clone()), Some(bridge.clone()));
        assert!(all_three.capabilities().ghostscript);
        // Nothing else turns on with it.
        assert!(!all_three.capabilities().whisper);
        assert!(!all_three.capabilities().ffmpeg);

        // gs present, bridge present, but the profile is missing: not offered.
        let without_profile = pdfa_test_tools(Some(no_profile), Some(bridge.clone()));
        assert!(!without_profile.capabilities().ghostscript);

        // gs and profile present, but no bridge: not offered.
        let without_bridge = pdfa_test_tools(Some(complete), None);
        assert!(!without_bridge.capabilities().ghostscript);

        // Bridge present, but no gs at all: not offered.
        let without_gs = pdfa_test_tools(None, Some(bridge));
        assert!(!without_gs.capabilities().ghostscript);

        // A gs path that does not exist cannot produce a profile either.
        let missing_gs = pdfa_test_tools(
            Some(dir.path().join("nowhere").join("bin").join("gs")),
            Some(dir.path().join("flip-macos-bridge")),
        );
        assert!(!missing_gs.capabilities().ghostscript);
    }

    #[test]
    fn ghostscript_capability_follows_a_symlinked_gs_to_its_real_install() {
        // Homebrew puts a symlink in /opt/homebrew/bin; the profile lives next to the real binary.
        let dir = tempfile::tempdir().unwrap();
        let real = fake_ghostscript_install(&dir.path().join("Cellar").join("ghostscript"), true);
        let links = dir.path().join("bin");
        fs::create_dir(&links).unwrap();
        let link = links.join("gs");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let bridge = dir.path().join("flip-macos-bridge");
        fs::write(&bridge, b"stub").unwrap();

        assert!(pdfa_test_tools(Some(link), Some(bridge)).capabilities().ghostscript);
    }

    #[test]
    fn validate_output_checks_pdfa_like_a_pdf() {
        let dir = tempfile::tempdir().unwrap();

        let good = dir.path().join("good.pdf");
        fs::write(&good, "%PDF-1.7\nbody\n%%EOF\n").unwrap();
        assert!(validate_output(&good, "PDFA").is_ok());
        assert!(validate_output(&good, "OPTIMIZE_PDF").is_ok());

        let no_header = dir.path().join("no-header.pdf");
        fs::write(&no_header, "hello world, not a pdf\n%%EOF\n").unwrap();
        let err = validate_output(&no_header, "PDFA").unwrap_err();
        assert!(err.to_string().contains("Missing %PDF- header"), "{err}");

        let truncated = dir.path().join("truncated.pdf");
        fs::write(&truncated, "%PDF-1.7\nbody without a trailer\n").unwrap();
        let err = validate_output(&truncated, "PDFA").unwrap_err();
        assert!(err.to_string().contains("missing %%EOF"), "{err}");

        let empty = dir.path().join("empty.pdf");
        fs::write(&empty, b"").unwrap();
        assert!(validate_output(&empty, "PDFA").is_err());
    }

    #[test]
    fn convert_single_unit_routes_pdfa_through_fake_tools() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let tool_dir = tempfile::tempdir().unwrap();

        // Fake Ghostscript: writes a tiny PDF to the file named by `-sOutputFile=` (`%%` is a literal `%`).
        let gs = fake_ghostscript_install(&tool_dir.path().join("gs-install"), true);
        fs::write(
            &gs,
            r##"#!/bin/sh
outfile=
for a in "$@"; do
  case "$a" in
    -sOutputFile=*) outfile=${a#-sOutputFile=} ;;
  esac
done
real=$(printf '%s' "$outfile" | sed 's/%%/%/g')
printf '%%PDF-1.7 fake pdfa output\n%%%%EOF\n' > "$real"
"##,
        )
        .unwrap();

        // Fake bridge: the staged output reads as PDF/A-2b, the source does not; nothing to mark printable.
        let bridge = tool_dir.path().join("flip-macos-bridge");
        fs::write(
            &bridge,
            r##"#!/bin/sh
case "$1" in
  pdf-printable) echo '{"ok":true,"annotations":0,"changed":0,"written":false}' ;;
  pdfa-inspect)
    case "$2" in
      */pdfa-out.pdf) echo '{"ok":true,"pages":2,"characters":900,"locked":false,"outputIntent":true,"part":2,"conformance":"B"}' ;;
      *) echo '{"ok":true,"pages":2,"characters":1000,"locked":false,"outputIntent":false,"part":null,"conformance":null}' ;;
    esac ;;
  *) echo '{"ok":false,"error":"unknown command"}'; exit 1 ;;
esac
"##,
        )
        .unwrap();
        fs::set_permissions(&bridge, fs::Permissions::from_mode(0o755)).unwrap();

        let source = home.path().join("report.pdf");
        let original = b"%PDF-1.7 original bytes\n%%EOF\n";
        fs::write(&source, original).unwrap();

        let tools = pdfa_test_tools(Some(gs), Some(bridge));
        let caps = tools.capabilities();
        assert!(caps.ghostscript);
        assert!(outputs(&[&source], &caps).contains(&"PDFA".to_string()));

        let prefs = AppPreferences::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let outputs_vec = convert_single_unit(
            &source,
            "PDFA",
            &prefs,
            &tools,
            &caps,
            &cancel,
            false,
            &|_| {},
        )
        .unwrap();
        assert_eq!(outputs_vec.len(), 1);
        assert_eq!(
            outputs_vec[0].file_name().and_then(|n| n.to_str()),
            Some("report 2.pdf")
        );
        assert_eq!(outputs_vec[0].parent(), Some(home.path()));
        assert_eq!(
            fs::read_to_string(&outputs_vec[0]).unwrap(),
            "%PDF-1.7 fake pdfa output\n%%EOF\n"
        );
        assert!(validate_output(&outputs_vec[0], "PDFA").is_ok());
        // The original stays as it was, and no staging folder is left behind.
        assert_eq!(fs::read(&source).unwrap(), original);
        let mut names: Vec<String> = fs::read_dir(home.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["report 2.pdf".to_string(), "report.pdf".to_string()]);

        // Without the capability the token is refused before any tool runs.
        let off = pdfa_test_tools(None, None).capabilities();
        let err = convert_single_unit(&source, "PDFA", &prefs, &tools, &off, &cancel, false, &|_| {})
            .unwrap_err();
        assert!(matches!(err, EngineError::Unsupported(_)), "{err}");

        // A missing tool must be a clean Unsupported error, never a panic.
        let staging = JobStaging::create(home.path()).unwrap();
        for broken in [
            pdfa_test_tools(None, tools.bridge.clone()),
            pdfa_test_tools(tools.ghostscript.clone(), None),
        ] {
            let err = convert_standard_unit(
                &source,
                "pdf",
                "PDFA",
                &staging,
                &prefs,
                &broken,
                &cancel,
                &|_| {},
            )
            .unwrap_err();
            assert!(matches!(err, EngineError::Unsupported(_)), "{err}");
        }
        staging.cleanup().unwrap();
    }

    #[test]
    fn validate_output_checks_fb2_xml_and_rst_non_empty() {
        let dir = tempfile::tempdir().unwrap();

        let valid_fb2 = dir.path().join("valid.fb2");
        fs::write(
            &valid_fb2,
            r#"<?xml version="1.0" encoding="UTF-8"?><FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><body><section><p>Hi</p></section></body></FictionBook>"#,
        )
        .unwrap();
        assert!(validate_output(&valid_fb2, "FB2").is_ok());

        let wrong_root_fb2 = dir.path().join("wrong.fb2");
        fs::write(&wrong_root_fb2, "<html/>").unwrap();
        assert!(validate_output(&wrong_root_fb2, "FB2").is_err());

        let malformed_fb2 = dir.path().join("bad.fb2");
        fs::write(&malformed_fb2, "<FictionBook><body></FictionBook>").unwrap();
        assert!(validate_output(&malformed_fb2, "FB2").is_err());

        let truncated_fb2 = dir.path().join("truncated.fb2");
        fs::write(&truncated_fb2, "<FictionBook><body><section><p>unclosed").unwrap();
        assert!(validate_output(&truncated_fb2, "FB2").is_err());

        let multiroot_fb2 = dir.path().join("multiroot.fb2");
        fs::write(&multiroot_fb2, "<FictionBook/> <extra/>").unwrap();
        assert!(validate_output(&multiroot_fb2, "FB2").is_err());

        let valid_rst = dir.path().join("valid.rst");
        fs::write(&valid_rst, "Title\n=====\n\nBody\n").unwrap();
        assert!(validate_output(&valid_rst, "RST").is_ok());

        let blank_rst = dir.path().join("blank.rst");
        fs::write(&blank_rst, "   \n\t\n").unwrap();
        assert!(validate_output(&blank_rst, "RST").is_err());

        // ICO validation checks 00 00 01 00 magic header and decodability
        let valid_ico = dir.path().join("valid.ico");
        let img = ::image::DynamicImage::ImageRgba8(::image::ImageBuffer::from_pixel(
            16,
            16,
            ::image::Rgba([255, 0, 0, 255]),
        ));
        img.save_with_format(&valid_ico, ::image::ImageFormat::Ico)
            .unwrap();
        assert!(validate_output(&valid_ico, "ICO").is_ok());

        let bad_ico = dir.path().join("bad.ico");
        fs::write(&bad_ico, b"PNG\x00not-an-ico").unwrap();
        assert!(validate_output(&bad_ico, "ICO").is_err());
    }
}
