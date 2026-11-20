use crate::core::preferences::{default_preferred_formats, AppPreferences};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::CStr;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

pub const MAX_BATCH_FILES: usize = 128;
pub const MAX_REGULAR_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_MEDIA_FILE_BYTES: u64 = 2048 * 1024 * 1024;
pub const MAX_CONTAINER_ENTRIES: usize = 4096;
pub const MAX_CONTAINER_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("Select between 1 and 128 files.")]
    InvalidBatchCount,
    #[error("Path is not absolute or local: {0}")]
    InvalidPath(String),
    #[error("File does not exist: {0}")]
    MissingFile(String),
    #[error("Directories cannot be converted directly: {0}")]
    DirectoryNotAllowed(String),
    #[error("Symbolic links and aliases are refused for safety: {0}")]
    SymlinkRefused(String),
    #[error("iCloud file is not downloaded locally: {0}")]
    CloudFileNotDownloaded(String),
    #[error("Network volumes are refused for conversions: {0}")]
    NetworkVolumeRefused(String),
    #[error("File exceeds size limit: {0}")]
    FileTooLarge(String),
    #[error("Container exceeds limit of 4096 internal entries.")]
    ContainerTooManyEntries,
    #[error("Expanded document exceeds 256 MiB safety limit.")]
    ContainerBombDetected,
    #[error("Invalid zip container: {0}")]
    InvalidContainer(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatFamily {
    Image,
    Data,
    Audio,
    Video,
    Pdf,
    Document,
    IWork,
    Archive,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub ffmpeg: bool,
    pub pandoc: bool,
    pub pages: bool,
    pub numbers: bool,
    pub keynote: bool,
    /// Transcription is offered only when whisper binary, model and ffmpeg are all present.
    pub whisper: bool,
}

impl EngineCapabilities {
    pub fn native_only() -> Self {
        Self::default()
    }

    pub fn all() -> Self {
        Self {
            ffmpeg: true,
            pandoc: true,
            pages: true,
            numbers: true,
            keynote: true,
            whisper: true,
        }
    }
}

pub const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "heic", "heif", "avif", "icns",
    "ico", "svg",
];
pub const DATA_EXTS: &[&str] = &["json", "yaml", "yml", "toml", "xml", "csv", "xlsx"];
pub const AUDIO_NATIVE_EXTS: &[&str] = &[
    "wav", "aiff", "aif", "m4a", "aac", "alac", "caf", "flac", "mp3",
];
pub const AUDIO_ALL_EXTS: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus", "aiff", "aif", "alac", "caf",
];
pub const VIDEO_NATIVE_EXTS: &[&str] = &["mp4", "mov", "m4v"];
pub const VIDEO_ALL_EXTS: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "wmv", "m4v"];
pub const DOC_NATIVE_EXTS: &[&str] = &[
    "txt", "md", "markdown", "html", "htm", "docx", "odt", "rtf", "doc", "epub", "fb2", "rst",
];
pub const DOC_PANDOC_ONLY_EXTS: &[&str] = &[];
pub const IWORK_EXTS: &[&str] = &["pages", "numbers", "key"];
pub const ARCHIVE_EXTS: &[&str] = &["zip"];

pub fn ext_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn canonical_format_for_ext(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "jpeg" => "JPG".to_string(),
        "tif" => "TIFF".to_string(),
        "markdown" => "MD".to_string(),
        "htm" => "HTML".to_string(),
        "yml" => "YAML".to_string(),
        "heif" => "HEIC".to_string(),
        "aif" => "AIFF".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

pub fn family_for_ext(ext: &str) -> Option<FormatFamily> {
    let e = ext.to_ascii_lowercase();
    let es = e.as_str();
    if IMAGE_EXTS.contains(&es) {
        Some(FormatFamily::Image)
    } else if DATA_EXTS.contains(&es) {
        Some(FormatFamily::Data)
    } else if AUDIO_ALL_EXTS.contains(&es) {
        Some(FormatFamily::Audio)
    } else if VIDEO_ALL_EXTS.contains(&es) {
        Some(FormatFamily::Video)
    } else if es == "pdf" {
        Some(FormatFamily::Pdf)
    } else if DOC_NATIVE_EXTS.contains(&es) || DOC_PANDOC_ONLY_EXTS.contains(&es) {
        Some(FormatFamily::Document)
    } else if IWORK_EXTS.contains(&es) {
        Some(FormatFamily::IWork)
    } else if ARCHIVE_EXTS.contains(&es) {
        Some(FormatFamily::Archive)
    } else {
        None
    }
}

pub fn is_media_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    AUDIO_ALL_EXTS.contains(&e.as_str()) || VIDEO_ALL_EXTS.contains(&e.as_str())
}

fn targets_for_single_ext(ext: &str, caps: &EngineCapabilities) -> Vec<&'static str> {
    match ext {
        e if IMAGE_EXTS.contains(&e) => {
            vec![
                "PNG", "JPG", "WEBP", "HEIC", "PDF", "TIFF", "GIF", "BMP", "ICO", "ICNS", "TXT",
                "REMOVE_BG",
            ]
        }
        e if DATA_EXTS.contains(&e) => {
            vec!["JSON", "YAML", "TOML", "XML", "CSV", "XLSX", "TXT", "HTML", "MD", "PDF"]
        }
        e if AUDIO_ALL_EXTS.contains(&e) => {
            if caps.ffmpeg {
                let mut out = vec![
                    "MP3", "M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF", "OGG", "OPUS", "AAC",
                ];
                if caps.whisper {
                    out.push("TRANSCRIBE");
                }
                out
            } else if AUDIO_NATIVE_EXTS.contains(&e) {
                let mut out = vec!["M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF"];
                if caps.whisper {
                    out.push("TRANSCRIBE");
                }
                out
            } else {
                vec![]
            }
        }
        e if VIDEO_ALL_EXTS.contains(&e) => {
            if caps.ffmpeg {
                let mut out = vec!["MP4", "MOV", "GIF", "M4A", "MP3", "WAV", "MKV", "WEBM", "AVI", "M4V"];
                if caps.whisper {
                    out.push("TRANSCRIBE");
                }
                out
            } else if VIDEO_NATIVE_EXTS.contains(&e) {
                let mut out = vec!["MP4", "MOV", "M4V", "M4A", "WAV", "GIF"];
                if caps.whisper {
                    out.push("TRANSCRIBE");
                }
                out
            } else {
                vec![]
            }
        }
        "pdf" => vec![
            "PNG",
            "JPG",
            "WEBP",
            "HEIC",
            "TIFF",
            "TXT",
            "DOCX",
            "MD",
            "HTML",
            "RTF",
            "ODT",
            "EPUB",
            "FB2",
            "RST",
            "OPTIMIZE_PDF",
        ],
        e if DOC_NATIVE_EXTS.contains(&e) => vec![
            "PDF", "DOCX", "MD", "HTML", "TXT", "RTF", "ODT", "PNG", "JPG", "WEBP", "HEIC",
            "EPUB", "FB2", "RST",
        ],
        e if DOC_PANDOC_ONLY_EXTS.contains(&e) => {
            if caps.pandoc {
                vec![
                    "PDF", "DOCX", "MD", "HTML", "TXT", "RTF", "ODT", "PNG", "JPG", "WEBP",
                    "HEIC", "EPUB", "FB2", "RST",
                ]
            } else {
                vec![]
            }
        }
        "pages" => {
            let mut out = vec!["PDF"];
            if caps.pages {
                out.extend_from_slice(&["DOCX", "RTF", "TXT", "EPUB"]);
            }
            out
        }
        "numbers" => {
            let mut out = vec!["PDF"];
            if caps.numbers {
                out.extend_from_slice(&["XLSX", "CSV"]);
            }
            out
        }
        "key" => {
            let mut out = vec!["PDF"];
            if caps.keynote {
                out.push("PPTX");
            }
            out
        }
        e if ARCHIVE_EXTS.contains(&e) => vec!["UNZIP"],
        _ => vec![],
    }
}

pub fn outputs<P: AsRef<Path>>(paths: &[P], caps: &EngineCapabilities) -> Vec<String> {
    if paths.is_empty() || paths.len() > MAX_BATCH_FILES {
        return Vec::new();
    }

    if paths.len() == 1 {
        let single = paths[0].as_ref();
        if single.is_dir() {
            let ext = ext_lower(single);
            if !IWORK_EXTS.contains(&ext.as_str()) {
                return Vec::new();
            }
            let same = canonical_format_for_ext(&ext);
            return targets_for_single_ext(&ext, caps)
                .into_iter()
                .filter(|t| *t != same)
                .map(String::from)
                .collect();
        }
        let ext = ext_lower(single);
        let same = canonical_format_for_ext(&ext);
        let mut out: Vec<String> = targets_for_single_ext(&ext, caps)
            .into_iter()
            .filter(|t| *t != same)
            .map(String::from)
            .collect();
        if !ARCHIVE_EXTS.contains(&ext.as_str()) && !out.iter().any(|f| f == "ZIP") {
            out.push("ZIP".to_string());
        }
        return out;
    }

    let mut shared: Option<Vec<String>> = None;
    for p in paths {
        if p.as_ref().is_dir() {
            return Vec::new();
        }
        let ext = ext_lower(p.as_ref());
        let same = canonical_format_for_ext(&ext);
        let filtered: Vec<String> = targets_for_single_ext(&ext, caps)
            .into_iter()
            .filter(|t| *t != same && *t != "UNZIP")
            .map(String::from)
            .collect();

        shared = match shared {
            None => Some(filtered),
            Some(prev) => {
                let set: HashSet<&str> = filtered.iter().map(String::as_str).collect();
                Some(
                    prev.into_iter()
                        .filter(|fmt| set.contains(fmt.as_str()))
                        .collect(),
                )
            }
        };
    }

    shared.unwrap_or_default()
}

pub fn batch_actions<P: AsRef<Path>>(paths: &[P]) -> Vec<String> {
    if paths.len() < 2 || paths.len() > MAX_BATCH_FILES {
        return Vec::new();
    }
    let has_dir = paths.iter().any(|p| p.as_ref().is_dir());
    let exts: Vec<String> = paths.iter().map(|p| ext_lower(p.as_ref())).collect();
    let mut actions = Vec::new();
    if exts.iter().all(|e| IMAGE_EXTS.contains(&e.as_str())) {
        actions.push("COMBINE_PDF".to_string());
        actions.push("ANIMATE_GIF".to_string());
    } else if exts.iter().all(|e| e == "pdf") {
        actions.push("MERGE_PDF".to_string());
    }
    if !has_dir {
        actions.push("ZIP".to_string());
    }
    actions
}

pub fn order_formats(
    outputs: &[String],
    family: Option<FormatFamily>,
    prefs: Option<&AppPreferences>,
) -> Vec<String> {
    let default_map = default_preferred_formats();
    let pref_map = prefs
        .map(|p| &p.preferred_formats)
        .unwrap_or(&default_map);

    let preferred_list: Vec<&str> = match family {
        Some(FormatFamily::Archive) => vec!["UNZIP", "ZIP"],
        Some(fam) => {
            let key = match fam {
                FormatFamily::Image => "image",
                FormatFamily::Pdf => "pdf",
                FormatFamily::Document | FormatFamily::IWork => "document",
                FormatFamily::Data => "data",
                FormatFamily::Audio => "audio",
                FormatFamily::Video => "video",
                FormatFamily::Archive => unreachable!(),
            };
            pref_map
                .get(key)
                .or_else(|| default_map.get(key))
                .map(|v| v.iter().map(String::as_str).collect())
                .unwrap_or_default()
        }
        None => vec![
            "DOCX",
            "PDF",
            "OPTIMIZE_PDF",
            "HTML",
            "MD",
            "TXT",
            "RTF",
            "ODT",
            "EPUB",
            "FB2",
            "RST",
            "JSON",
            "YAML",
            "TOML",
            "XML",
            "CSV",
            "XLSX",
            "PNG",
            "JPG",
            "WEBP",
            "HEIC",
            "REMOVE_BG",
            "TIFF",
            "GIF",
            "BMP",
            "ICO",
            "ICNS",
            "MP4",
            "MOV",
            "M4A",
            "MP3",
            "WAV",
            "FLAC",
            "AIFF",
            "ALAC",
            "CAF",
            "OGG",
            "OPUS",
            "AAC",
            "TRANSCRIBE",
            "MKV",
            "WEBM",
            "AVI",
            "M4V",
            "UNZIP",
            "ZIP",
        ],
    };

    let available: HashSet<&str> = outputs.iter().map(String::as_str).collect();
    let mut ordered: Vec<String> = preferred_list
        .into_iter()
        .filter(|fmt| available.contains(*fmt))
        .map(String::from)
        .collect();
    for fmt in outputs {
        if !ordered.contains(fmt) {
            ordered.push(fmt.clone());
        }
    }
    ordered
}

pub fn availability_message<P: AsRef<Path>>(paths: &[P], caps: &EngineCapabilities) -> String {
    if paths.is_empty() {
        return "Drop local files here".to_string();
    }
    if !outputs(paths, caps).is_empty() || !batch_actions(paths).is_empty() {
        return String::new();
    }

    let format_ext_label = |p: &Path| -> String {
        let ext = ext_lower(p).to_ascii_uppercase();
        if ext.is_empty() {
            "no extension".to_string()
        } else if ext.len() > 18 {
            ext.chars().take(18).collect()
        } else {
            ext
        }
    };

    let mut unsupported: Vec<String> = Vec::new();
    let mut listed: Vec<String> = Vec::new();

    for p in paths {
        let single = [p.as_ref()];
        let label = format_ext_label(p.as_ref());
        if outputs(&single, caps).is_empty() && !unsupported.contains(&label) && unsupported.len() < 4 {
            unsupported.push(label.clone());
        }
        if !listed.contains(&label) && listed.len() < 4 {
            listed.push(label);
        }
    }

    let count_line = if paths.len() == 1 {
        "1 file selected".to_string()
    } else {
        format!("{} files selected", paths.len())
    };

    let detail_line = if !unsupported.is_empty() {
        format!("\nUnsupported: {}", unsupported.join(", "))
    } else if !listed.is_empty() {
        format!("\nIncompatible formats: {}", listed.join(", "))
    } else {
        String::new()
    };

    format!("{count_line}{detail_line}\nNo common output format")
}

#[cfg(target_os = "macos")]
fn is_local_filesystem(path: &Path) -> bool {
    let Ok(cstr) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(cstr.as_ptr(), &mut stat) };
    if rc != 0 {
        return false;
    }
    // MNT_LOCAL (0x00001000 on macOS) indicates a local mount
    if (stat.f_flags & libc::MNT_LOCAL as u32) != 0 {
        return true;
    }
    // Check filesystem type string fallback for APFS/HFS in virtualized mounts
    let fstype = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }
        .to_string_lossy()
        .to_ascii_lowercase();
    matches!(fstype.as_str(), "apfs" | "hfs" | "tmpfs" | "devfs")
}

#[cfg(not(target_os = "macos"))]
fn is_local_filesystem(_path: &Path) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn is_finder_alias(path: &Path) -> bool {
    let Ok(cstr) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut buf = [0u8; 32];
    let rc = unsafe {
        libc::getxattr(
            cstr.as_ptr(),
            c"com.apple.FinderInfo".as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    // FinderInfo bytes 8..10 are big-endian Finder flags; bit 0x8000 (buf[8] & 0x80) is kIsAlias.
    rc >= 9 && (buf[8] & 0x80) != 0
}

#[cfg(not(target_os = "macos"))]
fn is_finder_alias(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn is_dataless_icloud(path: &Path, meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    if (meta.st_flags() & SF_DATALESS) != 0 {
        return true;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') && name.ends_with(".icloud") {
            return true;
        }
    }
    false
}

#[cfg(not(target_os = "macos"))]
fn is_dataless_icloud(_path: &Path, _meta: &fs::Metadata) -> bool {
    false
}

pub fn validate_input_files<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<PathBuf>, PolicyError> {
    if paths.is_empty() || paths.len() > MAX_BATCH_FILES {
        return Err(PolicyError::InvalidBatchCount);
    }

    let mut seen = HashSet::new();
    let mut validated = Vec::with_capacity(paths.len());

    for raw in paths {
        let p = raw.as_ref();
        if !p.is_absolute() {
            return Err(PolicyError::InvalidPath(p.display().to_string()));
        }
        let meta = fs::symlink_metadata(p).map_err(|_| PolicyError::MissingFile(p.display().to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(PolicyError::SymlinkRefused(p.display().to_string()));
        }
        let ext = ext_lower(p);
        let is_iwork_package = matches!(ext.as_str(), "pages" | "numbers" | "key");
        if meta.is_dir() {
            if !is_iwork_package {
                return Err(PolicyError::DirectoryNotAllowed(p.display().to_string()));
            }
        } else if !meta.is_file() {
            return Err(PolicyError::InvalidPath(p.display().to_string()));
        }
        if is_finder_alias(p) {
            return Err(PolicyError::SymlinkRefused(p.display().to_string()));
        }
        if is_dataless_icloud(p, &meta) {
            return Err(PolicyError::CloudFileNotDownloaded(p.display().to_string()));
        }
        if !is_local_filesystem(p) {
            return Err(PolicyError::NetworkVolumeRefused(p.display().to_string()));
        }

        let max_bytes = if is_media_ext(&ext) {
            MAX_MEDIA_FILE_BYTES
        } else {
            MAX_REGULAR_FILE_BYTES
        };
        if !meta.is_dir() && meta.len() > max_bytes {
            return Err(PolicyError::FileTooLarge(p.display().to_string()));
        }

        let dedup_key = match (p.parent().and_then(|d| fs::canonicalize(d).ok()), p.file_name()) {
            (Some(canon_dir), Some(fname)) => canon_dir.join(fname),
            _ => p.to_path_buf(),
        };
        if seen.insert(dedup_key) {
            validated.push(p.to_path_buf());
        }
    }

    Ok(validated)
}

pub fn validate_container(path: &Path) -> Result<(), PolicyError> {
    let ext = ext_lower(path);
    if !matches!(ext.as_str(), "docx" | "epub" | "odt" | "xlsx" | "pptx") {
        return Ok(());
    }
    let file = File::open(path).map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;

    if archive.len() > MAX_CONTAINER_ENTRIES {
        return Err(PolicyError::ContainerTooManyEntries);
    }

    let mut declared_total: u64 = 0;
    let mut actual_total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;
        let declared = entry.size();
        declared_total = declared_total
            .checked_add(declared)
            .ok_or(PolicyError::ContainerBombDetected)?;
        if declared_total > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(PolicyError::ContainerBombDetected);
        }
        // Also enforce actual decompressed stream total across ALL entries (guards forged zip headers)
        let mut buf = [0u8; 65536];
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;
            if n == 0 {
                break;
            }
            actual_total = actual_total
                .checked_add(n as u64)
                .ok_or(PolicyError::ContainerBombDetected)?;
            if actual_total > MAX_CONTAINER_EXPANDED_BYTES {
                return Err(PolicyError::ContainerBombDetected);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    #[test]
    fn image_and_batch_intersection_rules() {
        let caps = EngineCapabilities::native_only();
        let png_out = outputs(&["image.png"], &caps);
        assert!(png_out.contains(&"JPG".to_string()));
        assert!(png_out.contains(&"HEIC".to_string()));
        assert!(png_out.contains(&"REMOVE_BG".to_string()));
        assert!(png_out.contains(&"ZIP".to_string()));
        assert!(!png_out.contains(&"PNG".to_string()));

        // Mixed image + audio shares zero per-file formats, while batch ZIP remains available
        assert!(outputs(&["image.png", "audio.mp3"], &EngineCapabilities::all()).is_empty());
        assert_eq!(
            batch_actions(&["image.png", "audio.mp3"]),
            vec!["ZIP".to_string()]
        );
        assert_eq!(
            availability_message(&["image.png", "audio.mp3"], &EngineCapabilities::all()),
            ""
        );

        // Without FFmpeg, unsupported MKV batch still exposes ZIP while per-file intersection is empty
        assert!(outputs(&["image.png", "video.mkv"], &caps).is_empty());
        assert_eq!(
            batch_actions(&["image.png", "video.mkv"]),
            vec!["ZIP".to_string()]
        );

        // Non-iWork directory triggers explicit availability warning message
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("myfolder");
        std::fs::create_dir(&folder).unwrap();
        let dir_msg = availability_message(&[&folder], &caps);
        assert!(dir_msg.contains("Unsupported"), "dir_msg was: {dir_msg}");

        // Pure intersection: image + mp4 shares GIF on macOS
        assert_eq!(outputs(&["image.png", "video.mp4"], &caps), vec!["GIF".to_string()]);

        // Supported image batch keeps intersection & exposes COMBINE_PDF + ANIMATE_GIF + ZIP
        let batch = outputs(&["image.png", "image.webp"], &caps);
        assert!(batch.contains(&"JPG".to_string()));
        assert!(batch.contains(&"ICO".to_string()));
        assert!(batch.contains(&"ICNS".to_string()));
        assert!(batch.contains(&"TXT".to_string()));
        assert!(batch.contains(&"REMOVE_BG".to_string()));
        assert!(!batch.contains(&"PNG".to_string()));
        assert!(!batch.contains(&"WEBP".to_string()));
        assert!(!batch.contains(&"ZIP".to_string()));
        assert_eq!(
            batch_actions(&["a.png", "b.jpg"]),
            vec![
                "COMBINE_PDF".to_string(),
                "ANIMATE_GIF".to_string(),
                "ZIP".to_string()
            ]
        );

        // SVG input & ICO self-exclusion rules
        assert_eq!(family_for_ext("svg"), Some(FormatFamily::Image));
        let svg_out = outputs(&["logo.svg"], &caps);
        assert!(!svg_out.contains(&"SVG".to_string()));
        for target in [
            "PNG", "JPG", "WEBP", "HEIC", "PDF", "BMP", "TIFF", "GIF", "ICO", "ICNS", "TXT",
            "REMOVE_BG", "ZIP",
        ] {
            assert!(svg_out.contains(&target.to_string()), "SVG missing target {target}");
        }
        let ico_out = outputs(&["favicon.ico"], &caps);
        assert!(!ico_out.contains(&"ICO".to_string()));
        assert!(ico_out.contains(&"PNG".to_string()));
        assert_eq!(
            batch_actions(&["a.svg", "b.png"]),
            vec![
                "COMBINE_PDF".to_string(),
                "ANIMATE_GIF".to_string(),
                "ZIP".to_string()
            ]
        );

        assert_eq!(availability_message(&["image.png"], &caps), "");
    }

    #[test]
    fn txt_and_document_outputs_with_and_without_pandoc() {
        let native = EngineCapabilities::native_only();
        let with_pd = EngineCapabilities {
            pandoc: true,
            ..Default::default()
        };

        assert!(DOC_NATIVE_EXTS.contains(&"epub"));
        assert!(DOC_NATIVE_EXTS.contains(&"fb2"));
        assert!(DOC_NATIVE_EXTS.contains(&"rst"));
        assert!(!DOC_PANDOC_ONLY_EXTS.contains(&"epub"));
        assert!(!DOC_PANDOC_ONLY_EXTS.contains(&"fb2"));
        assert!(!DOC_PANDOC_ONLY_EXTS.contains(&"rst"));

        let txt_native = outputs(&["note.txt"], &native);
        assert!(txt_native.contains(&"PDF".to_string()));
        assert!(txt_native.contains(&"DOCX".to_string()));
        assert!(txt_native.contains(&"PNG".to_string()));
        assert!(txt_native.contains(&"HEIC".to_string()));
        assert!(txt_native.contains(&"EPUB".to_string()));
        assert!(txt_native.contains(&"FB2".to_string()));
        assert!(txt_native.contains(&"RST".to_string()));
        assert!(!txt_native.contains(&"PAGES".to_string()));

        assert!(outputs(&["note.html"], &native).contains(&"EPUB".to_string()));
        assert!(outputs(&["note.html"], &native).contains(&"DOCX".to_string()));
        assert!(outputs(&["note.html"], &native).contains(&"FB2".to_string()));
        assert!(outputs(&["note.html"], &native).contains(&"RST".to_string()));
        assert!(outputs(&["deck.pdf"], &native).contains(&"EPUB".to_string()));
        assert!(outputs(&["deck.pdf"], &native).contains(&"FB2".to_string()));
        assert!(outputs(&["deck.pdf"], &native).contains(&"RST".to_string()));
        assert!(outputs(&["deck.pdf"], &with_pd).contains(&"FB2".to_string()));
        assert!(outputs(&["deck.pdf"], &with_pd).contains(&"RST".to_string()));

        let epub_native = outputs(&["book.epub"], &native);
        assert_eq!(family_for_ext("epub"), Some(FormatFamily::Document));
        for needed in [
            "TXT", "MD", "HTML", "RTF", "DOCX", "ODT", "PDF", "PNG", "JPG", "WEBP", "HEIC",
            "FB2", "RST",
        ] {
            assert!(epub_native.contains(&needed.to_string()), "EPUB missing native output {needed}");
        }
        assert!(!epub_native.contains(&"EPUB".to_string()));

        let fb2_native = outputs(&["book.fb2"], &native);
        assert_eq!(family_for_ext("fb2"), Some(FormatFamily::Document));
        for needed in [
            "TXT", "MD", "HTML", "RTF", "DOCX", "ODT", "PDF", "PNG", "JPG", "WEBP", "HEIC",
            "EPUB", "RST", "ZIP",
        ] {
            assert!(fb2_native.contains(&needed.to_string()), "FB2 missing native output {needed}");
        }
        assert!(!fb2_native.contains(&"FB2".to_string()));

        let rst_native = outputs(&["guide.rst"], &native);
        assert_eq!(family_for_ext("rst"), Some(FormatFamily::Document));
        for needed in [
            "TXT", "MD", "HTML", "RTF", "DOCX", "ODT", "PDF", "PNG", "JPG", "WEBP", "HEIC",
            "EPUB", "FB2", "ZIP",
        ] {
            assert!(rst_native.contains(&needed.to_string()), "RST missing native output {needed}");
        }
        assert!(!rst_native.contains(&"RST".to_string()));
        assert!(!outputs(&["note.txt"], &with_pd).contains(&"PAGES".to_string()));

        let txt_pd = outputs(&["note.txt"], &with_pd);
        assert!(txt_pd.contains(&"EPUB".to_string()));
        assert!(txt_pd.contains(&"FB2".to_string()));
        assert!(txt_pd.contains(&"RST".to_string()));
        assert!(txt_pd.contains(&"ZIP".to_string()));
        assert_eq!(txt_pd.len(), 14);

        // Order formats preserves set membership
        let ordered = order_formats(&txt_pd, Some(FormatFamily::Document), None);
        let s1: HashSet<_> = ordered.iter().collect();
        let s2: HashSet<_> = txt_pd.iter().collect();
        assert_eq!(s1, s2);
    }

    #[test]
    fn data_and_media_policy_rules() {
        let native = EngineCapabilities::native_only();
        let with_ff = EngineCapabilities {
            ffmpeg: true,
            ..Default::default()
        };

        let json_out: HashSet<String> = outputs(&["data.json"], &native).into_iter().collect();
        let expected: HashSet<String> = [
            "YAML", "TOML", "XML", "CSV", "XLSX", "TXT", "HTML", "MD", "PDF", "ZIP",
        ]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(json_out, expected);

        let xlsx_out: HashSet<String> = outputs(&["sheet.xlsx"], &native).into_iter().collect();
        let xlsx_expected: HashSet<String> = [
            "JSON", "YAML", "TOML", "XML", "CSV", "TXT", "HTML", "MD", "PDF", "ZIP",
        ]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(xlsx_out, xlsx_expected);
        assert_eq!(family_for_ext("xlsx"), Some(FormatFamily::Data));

        assert!(outputs(&["table.csv"], &native).contains(&"JSON".to_string()));
        assert!(outputs(&["table.csv"], &native).contains(&"XLSX".to_string()));
        assert!(outputs(&["audio.mp3"], &with_ff).contains(&"WAV".to_string()));
        assert!(outputs(&["audio.mp3"], &with_ff).contains(&"CAF".to_string()));
        assert_eq!(outputs(&["audio.ogg"], &native), vec!["ZIP".to_string()]);
        assert_eq!(outputs(&["audio.opus"], &native), vec!["ZIP".to_string()]);

        // Zero-FFmpeg native audio matrix: WAV, AIFF, M4A, AAC, FLAC, CAF -> WAV, AIFF, M4A, ALAC, CAF, FLAC
        for ext in ["wav", "aiff", "m4a", "aac", "flac", "caf"] {
            let file = format!("track.{ext}");
            let out = outputs(&[file.as_str()], &native);
            let same = canonical_format_for_ext(ext);
            for target in ["WAV", "AIFF", "M4A", "ALAC", "CAF", "FLAC"] {
                if target == same {
                    assert!(!out.contains(&target.to_string()), "{file} should not target self {target}");
                } else {
                    assert!(out.contains(&target.to_string()), "{file} missing native target {target}");
                }
            }
            for gated in ["MP3", "OGG", "OPUS"] {
                assert!(!out.contains(&gated.to_string()), "{file} unexpectedly exposed {gated} without FFmpeg");
            }
        }

        // PDF and archive policy rules
        let pdf_out = outputs(&["doc.pdf"], &native);
        assert!(pdf_out.contains(&"OPTIMIZE_PDF".to_string()));
        assert!(pdf_out.contains(&"ZIP".to_string()));
        assert_eq!(outputs(&["bundle.zip"], &native), vec!["UNZIP".to_string()]);
        assert_eq!(
            batch_actions(&["a.zip", "b.zip"]),
            vec!["ZIP".to_string()]
        );
        assert_eq!(
            batch_actions(&["a.pdf", "b.pdf"]),
            vec!["MERGE_PDF".to_string(), "ZIP".to_string()]
        );
        assert!(outputs(&["audio.ogg"], &with_ff).contains(&"WAV".to_string()));

        let mp4_ff: HashSet<String> = outputs(&["clip.mp4"], &with_ff).into_iter().collect();
        for needed in ["MOV", "GIF", "M4A", "MP3", "WAV", "MKV", "WEBM", "AVI", "M4V"] {
            assert!(mp4_ff.contains(needed), "missing {needed}");
        }
        assert!(!mp4_ff.contains("MP4"));
    }

    #[test]
    fn transcribe_is_offered_for_media_only_with_whisper() {
        let without = EngineCapabilities {
            ffmpeg: true,
            ..Default::default()
        };
        let with = EngineCapabilities {
            ffmpeg: true,
            whisper: true,
            ..Default::default()
        };

        for ext in ["mp3", "m4a", "mp4", "mkv"] {
            let file = format!("clip.{ext}");
            let off = outputs(&[file.as_str()], &without);
            let on = outputs(&[file.as_str()], &with);
            assert!(
                !off.contains(&"TRANSCRIBE".to_string()),
                "{file} offers TRANSCRIBE without whisper: {off:?}"
            );
            assert!(
                on.contains(&"TRANSCRIBE".to_string()),
                "{file} misses TRANSCRIBE with whisper: {on:?}"
            );
            let mut base = on.clone();
            base.retain(|f| f != "TRANSCRIBE");
            assert_eq!(base, off, "{file} changed its other targets");
        }

        for ext in ["png", "jpg", "txt", "docx", "pdf", "json", "zip"] {
            let file = format!("file.{ext}");
            assert!(
                !outputs(&[file.as_str()], &with).contains(&"TRANSCRIBE".to_string()),
                "{file} must never offer TRANSCRIBE"
            );
        }

        // A mixed selection is an intersection: the image has no TRANSCRIBE, so neither has the job.
        let mixed = outputs(&["clip.mp3", "photo.jpg"], &with);
        assert!(
            !mixed.contains(&"TRANSCRIBE".to_string()),
            "mixed selection offered TRANSCRIBE: {mixed:?}"
        );

        assert!(outputs(&["clip.m4a"], &EngineCapabilities::all())
            .contains(&"TRANSCRIBE".to_string()));
    }

    #[test]
    fn transcribe_sits_after_aac_in_the_global_fallback_order() {
        let available = vec![
            "AAC".to_string(),
            "MKV".to_string(),
            "TRANSCRIBE".to_string(),
            "ZIP".to_string(),
        ];
        let ordered = order_formats(&available, None, None);
        let aac = ordered.iter().position(|f| f == "AAC").unwrap();
        let transcribe = ordered.iter().position(|f| f == "TRANSCRIBE").unwrap();
        let mkv = ordered.iter().position(|f| f == "MKV").unwrap();
        assert_eq!(transcribe, aac + 1, "ordered={ordered:?}");
        assert!(transcribe < mkv, "ordered={ordered:?}");
    }

    #[test]
    fn batch_bounds_and_symlinks_rejected() {
        let caps = EngineCapabilities::all();
        let empty: Vec<&str> = Vec::new();
        assert!(outputs(&empty, &caps).is_empty());
        assert!(matches!(
            validate_input_files(&empty),
            Err(PolicyError::InvalidBatchCount)
        ));

        let too_many: Vec<PathBuf> = (0..129)
            .map(|i| PathBuf::from(format!("/tmp/file_{i}.txt")))
            .collect();
        assert!(outputs(&too_many, &caps).is_empty());
        assert!(matches!(
            validate_input_files(&too_many),
            Err(PolicyError::InvalidBatchCount)
        ));

        let dir = tempdir().unwrap();
        let real = dir.path().join("real.txt");
        fs::write(&real, b"hello").unwrap();
        let link = dir.path().join("alias.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(matches!(
            validate_input_files(&[&link]),
            Err(PolicyError::SymlinkRefused(_))
        ));
        assert!(matches!(
            validate_input_files(&[dir.path()]),
            Err(PolicyError::DirectoryNotAllowed(_))
        ));
        assert_eq!(validate_input_files(&[&real]).unwrap(), vec![real]);
    }

    #[test]
    fn container_bomb_over_256mib_rejected() {
        let dir = tempdir().unwrap();
        let bomb_path = dir.path().join("over-limit.docx");
        {
            let file = File::create(&bomb_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file("word/document.xml", opts).unwrap();
            let zeros = vec![0u8; 1024 * 1024];
            for _ in 0..257 {
                zip.write_all(&zeros).unwrap();
            }
            zip.finish().unwrap();
        }
        let res = validate_container(&bomb_path);
        assert_eq!(res, Err(PolicyError::ContainerBombDetected));
    }

    #[test]
    fn finder_alias_icloud_and_multi_entry_bomb_rejected() {
        let dir = tempdir().unwrap();
        let regular = dir.path().join("normal.txt");
        fs::write(&regular, b"hello").unwrap();

        // Deduplicate equivalent relative dot segments
        let dot_path = dir.path().join(".").join("normal.txt");
        let deduped = validate_input_files(&[regular.clone(), dot_path]).unwrap();
        assert_eq!(deduped.len(), 1);

        // .icloud placeholder rejected
        let icloud_stub = dir.path().join(".report.pdf.icloud");
        fs::write(&icloud_stub, b"stub").unwrap();
        assert!(matches!(
            validate_input_files(&[&icloud_stub]),
            Err(PolicyError::CloudFileNotDownloaded(_))
        ));

        // Finder Alias (com.apple.FinderInfo kIsAlias bit 0x80 at byte 8) rejected
        #[cfg(target_os = "macos")]
        {
            let alias_path = dir.path().join("finder_alias.txt");
            fs::write(&alias_path, b"book\0\0\0\0mark\0\0\0\0").unwrap();
            let cstr = std::ffi::CString::new(alias_path.as_os_str().as_bytes()).unwrap();
            let mut finfo = [0u8; 32];
            finfo[0..8].copy_from_slice(b"alisMACS");
            finfo[8] = 0x80;
            let rc = unsafe {
                libc::setxattr(
                    cstr.as_ptr(),
                    c"com.apple.FinderInfo".as_ptr(),
                    finfo.as_ptr() as *const libc::c_void,
                    finfo.len(),
                    0,
                    0,
                )
            };
            assert_eq!(rc, 0);
            assert!(matches!(
                validate_input_files(&[&alias_path]),
                Err(PolicyError::SymlinkRefused(_))
            ));
        }

        // Multi-entry zip totaling > 256 MiB across entries rejected
        let multi_bomb = dir.path().join("multi.docx");
        {
            let file = File::create(&multi_bomb).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let zeros = vec![0u8; 1024 * 1024];
            for name in ["word/a.xml", "word/b.xml"] {
                zip.start_file(name, opts).unwrap();
                for _ in 0..130 {
                    zip.write_all(&zeros).unwrap();
                }
            }
            zip.finish().unwrap();
        }
        assert_eq!(
            validate_container(&multi_bomb),
            Err(PolicyError::ContainerBombDetected)
        );
    }
}
