use crate::core::preferences::AppPreferences;
use crate::engines::bridge::PermissionsStatus;
use crate::engines::DetectedTools;
use chrono::Utc;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
pub const DIAGNOSTICS_TAIL_LINES: usize = 60;

pub struct DiagnosticLogger {
    dir: Mutex<PathBuf>,
}

impl DiagnosticLogger {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir: Mutex::new(dir),
        }
    }

    pub fn default_log_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Logs")
            .join("FLIP")
    }

    pub fn set_directory(&self, new_dir: PathBuf) {
        if let Ok(mut guard) = self.dir.lock() {
            *guard = new_dir;
        }
    }

    pub fn directory(&self) -> PathBuf {
        self.dir
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| Self::default_log_dir())
    }

    pub fn events_path_in(dir: &Path) -> PathBuf {
        dir.join("events.jsonl")
    }

    pub fn previous_path_in(dir: &Path) -> PathBuf {
        dir.join("events.jsonl.previous")
    }

    pub fn log(&self, name: &str, detail: Option<Value>) {
        let Ok(guard) = self.dir.lock() else {
            return;
        };
        let dir = guard.as_path();
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let path = Self::events_path_in(dir);
        let prev = Self::previous_path_in(dir);
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() >= MAX_LOG_BYTES {
                let _ = fs::rename(&path, &prev);
            }
        }
        let line_obj = json!({
            "utc": Utc::now().to_rfc3339(),
            "name": name,
            "detail": detail,
        });
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{line_obj}");
        }
    }

    pub fn tail_events(&self, max_lines: usize) -> Vec<String> {
        let dir = self.directory();
        Self::tail_events_in_dir(&dir, max_lines)
    }

    pub fn tail_events_in_dir(dir: &Path, max_lines: usize) -> Vec<String> {
        Self::tail_events_in_dir_with_home(dir, max_lines, dirs::home_dir().as_deref())
    }

    pub fn tail_events_in_dir_with_home(
        dir: &Path,
        max_lines: usize,
        home_dir: Option<&Path>,
    ) -> Vec<String> {
        if max_lines == 0 {
            return Vec::new();
        }
        let path = Self::events_path_in(dir);
        let Ok(bytes) = fs::read(&path) else {
            return Vec::new();
        };
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = lines.len().saturating_sub(max_lines);
        lines[start..]
            .iter()
            .map(|line| redact_paths_with_home(line, home_dir))
            .collect()
    }
}

static GLOBAL_LOGGER: OnceLock<DiagnosticLogger> = OnceLock::new();

pub fn global_logger() -> &'static DiagnosticLogger {
    GLOBAL_LOGGER.get_or_init(|| DiagnosticLogger::new(DiagnosticLogger::default_log_dir()))
}

pub fn log_event(name: &str, detail: Option<Value>) {
    global_logger().log(name, detail);
}

fn is_left_path_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '"' | '\'' | '/' | '=' | ':' | '(' | '[' | '{' | '`' | ',' | '<' | ';'
        )
}

fn is_segment_terminator(ch: char) -> bool {
    ch == '/'
        || ch.is_whitespace()
        || matches!(
            ch,
            '"' | '\'' | '\\' | ')' | ']' | '}' | ',' | ';' | '`' | '>' | '<'
        )
}

fn scan_segment_len(rest: &str) -> usize {
    let mut len = 0;
    for ch in rest.chars() {
        if is_segment_terminator(ch) {
            break;
        }
        len += ch.len_utf8();
    }
    len
}

fn match_var_folders_prefix(tail: &str) -> Option<usize> {
    let prefix_len = if tail.starts_with("/private/var/folders/") {
        "/private/var/folders/".len()
    } else if tail.starts_with("/var/folders/") {
        "/var/folders/".len()
    } else {
        return None;
    };

    let after_prefix = &tail[prefix_len..];
    let seg1_len = scan_segment_len(after_prefix);
    if seg1_len == 0 {
        return None;
    }

    let after_seg1 = &after_prefix[seg1_len..];
    if let Some(after_slash) = after_seg1.strip_prefix('/') {
        let seg2_len = scan_segment_len(after_slash);
        if seg2_len > 0 {
            return Some(prefix_len + seg1_len + 1 + seg2_len);
        }
    }
    Some(prefix_len + seg1_len)
}

fn match_users_prefix(tail: &str) -> Option<usize> {
    let prefix_len = if tail.starts_with("/private/Users/") {
        "/private/Users/".len()
    } else if tail.starts_with("/Users/") {
        "/Users/".len()
    } else {
        return None;
    };

    let user_len = scan_segment_len(&tail[prefix_len..]);
    if user_len == 0 {
        return None;
    }
    Some(prefix_len + user_len)
}

fn match_exact_home(tail: &str, home_candidates: &[String]) -> Option<usize> {
    for home in home_candidates {
        if tail.starts_with(home.as_str()) {
            let after = &tail[home.len()..];
            if after.is_empty() || after.chars().next().is_some_and(is_segment_terminator) {
                return Some(home.len());
            }
        }
    }
    None
}

fn match_system_user_var(tail: &str) -> Option<usize> {
    for prefix in ["/private/var/mobile", "/private/var/root"] {
        if let Some(after) = tail.strip_prefix(prefix) {
            if after.is_empty() || after.chars().next().is_some_and(is_segment_terminator) {
                return Some(prefix.len());
            }
        }
    }
    None
}

pub fn redact_user_paths(input: &str) -> String {
    redact_paths_with_home(input, dirs::home_dir().as_deref())
}

pub fn redact_paths_with_home(input: &str, home_dir: Option<&Path>) -> String {
    let mut home_candidates: Vec<String> = Vec::new();
    if let Some(home) = home_dir {
        let raw = home.to_string_lossy().trim_end_matches('/').to_string();
        if !raw.is_empty() {
            home_candidates.push(raw);
        }
        if let Ok(canon) = fs::canonicalize(home) {
            let canon_str = canon.to_string_lossy().trim_end_matches('/').to_string();
            if !canon_str.is_empty() && !home_candidates.contains(&canon_str) {
                home_candidates.push(canon_str);
            }
        }
    }
    home_candidates.sort_by_key(|s| std::cmp::Reverse(s.len()));

    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut prev_char: Option<char> = None;

    while i < input.len() {
        let tail = &input[i..];
        let at_boundary = prev_char.is_none_or(is_left_path_boundary);

        if at_boundary && tail.starts_with('/') {
            let matched_len = match_exact_home(tail, &home_candidates)
                .or_else(|| match_users_prefix(tail))
                .or_else(|| match_var_folders_prefix(tail))
                .or_else(|| match_system_user_var(tail));

            if let Some(len) = matched_len {
                out.push('~');
                i += len;
                prev_char = Some('~');
                continue;
            }
        }

        let ch = tail.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
        prev_char = Some(ch);
    }

    out
}

pub fn collect_macos_sw_vers() -> String {
    let sw_vers = Path::new("/usr/bin/sw_vers");
    if !sw_vers.is_file() {
        return format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH);
    }
    let Ok(output) = Command::new(sw_vers).stdin(Stdio::null()).output() else {
        return "macOS (sw_vers failed)".to_string();
    };
    if !output.status.success() {
        return "macOS (sw_vers unavailable)".to_string();
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut name = String::new();
    let mut version = String::new();
    let mut build = String::new();
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let val = v.trim();
            match k.trim() {
                "ProductName" => name = val.to_string(),
                "ProductVersion" => version = val.to_string(),
                "BuildVersion" => build = val.to_string(),
                _ => {}
            }
        }
    }
    if name.is_empty() && version.is_empty() {
        return "macOS (unknown version)".to_string();
    }
    if build.is_empty() {
        format!("{name} {version}").trim().to_string()
    } else {
        format!("{name} {version} ({build})").trim().to_string()
    }
}

pub struct DiagnosticsReportInput<'a> {
    pub app_version: &'a str,
    pub macos_version: &'a str,
    pub tools: &'a DetectedTools,
    pub permissions: Result<&'a PermissionsStatus, &'a str>,
    pub prefs: &'a AppPreferences,
    pub log_lines: &'a [String],
    pub home_dir: Option<&'a Path>,
}

fn fmt_opt_path(path: Option<&Path>) -> String {
    match path {
        Some(p) => format!("Detected (`{}`)", p.display()),
        None => "Not Found".to_string(),
    }
}

pub fn format_diagnostics_markdown(input: &DiagnosticsReportInput<'_>) -> String {
    let mouse_desc = match input.prefs.mouse_trigger_button {
        Some(b) => format!("Button {b}"),
        None => "Disabled".to_string(),
    };
    let perms_section = match input.permissions {
        Ok(p) => format!(
            "- **Accessibility**: {}\n- **Finder Automation**: {}\n- **Input Monitoring**: {}",
            if p.accessibility {
                "Granted"
            } else {
                "Not Granted"
            },
            p.automation_finder,
            if p.input_monitoring {
                "Granted"
            } else {
                "Not Granted"
            }
        ),
        Err(err) => format!("- **Status**: Unavailable ({err})"),
    };

    let events_section = if input.log_lines.is_empty() {
        "_No events recorded yet in `events.jsonl`._".to_string()
    } else {
        format!("```jsonl\n{}\n```", input.log_lines.join("\n"))
    };

    let raw = format!(
        "# FLIP Diagnostics Report\n\n\
        Generated (UTC): {}\n\n\
        ## App & System\n\
        - **FLIP Version**: {} (`com.thetinyfoundry.flip.mac`, {})\n\
        - **macOS**: {}\n\n\
        ## Engines & Frameworks\n\
        - **Swift Bridge**: {}\n\
        - **Native Apple Frameworks**: ImageIO, PDFKit, Vision OCR, AVFoundation (Ready)\n\
        - **FFmpeg**: {}\n\
        - **FFprobe**: {}\n\
        - **Pandoc**: {}\n\
        - **Apple iWork**: Pages ({}), Numbers ({}), Keynote ({})\n\n\
        ## macOS Permissions\n\
        {}\n\n\
        ## Preferences Summary\n\
        - **Drag Trigger**: {}\n\
        - **Idle Shortcut**: {}\n\
        - **Mouse Trigger**: {}\n\
        - **Sound / Haptics**: {} / {}\n\
        - **Launch at Login**: {}\n\
        - **Auto Rename on Create / Open After Creating**: {} / {}\n\
        - **Image Quality / Preserve EXIF / Strip GPS**: {}% / {} / {}\n\
        - **PDF DPI / Page Size**: {} DPI / {}\n\
        - **Output Subfolder**: {}\n\
        - **Fallback Folder**: {}\n\
        - **Custom FFmpeg / Pandoc Overrides**: {} / {}\n\
        - **Recent Conversions Recorded**: {}\n\n\
        ## Recent Events (last {} lines of `events.jsonl`, redacted)\n\
        {}\n",
        Utc::now().to_rfc3339(),
        input.app_version,
        std::env::consts::ARCH,
        input.macos_version,
        fmt_opt_path(input.tools.bridge.as_deref()),
        fmt_opt_path(input.tools.ffmpeg.as_deref()),
        fmt_opt_path(input.tools.ffprobe.as_deref()),
        fmt_opt_path(input.tools.pandoc.as_deref()),
        if input.tools.pages { "Installed" } else { "Missing" },
        if input.tools.numbers { "Installed" } else { "Missing" },
        if input.tools.keynote { "Installed" } else { "Missing" },
        perms_section,
        input.prefs.drag_trigger,
        input.prefs.idle_shortcut,
        mouse_desc,
        input.prefs.sound_enabled,
        input.prefs.haptics_enabled,
        input.prefs.launch_at_login,
        input.prefs.auto_rename_on_create,
        input.prefs.open_after_creating,
        input.prefs.image_quality,
        input.prefs.preserve_exif,
        input.prefs.strip_gps_exif,
        input.prefs.pdf_dpi,
        input.prefs.pdf_page_size,
        input
            .prefs
            .output_subfolder
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Same folder"),
        input.prefs.resolved_fallback_folder().display(),
        input
            .prefs
            .ffmpeg_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Auto-detect"),
        input
            .prefs
            .pandoc_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Auto-detect"),
        input.prefs.recent_conversions.len(),
        DIAGNOSTICS_TAIL_LINES,
        events_section
    );

    redact_paths_with_home(&raw, input.home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn diagnostic_logs_bounded_rotation() {
        let dir = tempdir().unwrap();
        let logger = DiagnosticLogger::new(dir.path().to_path_buf());
        let log_path = DiagnosticLogger::events_path_in(dir.path());
        let prev_path = DiagnosticLogger::previous_path_in(dir.path());

        fs::write(&log_path, vec![b'x'; (MAX_LOG_BYTES) as usize]).unwrap();
        logger.log("rotation-test", Some(json!({"ok": true})));

        assert!(prev_path.exists(), "previous rotated file must exist");
        let new_len = fs::metadata(&log_path).unwrap().len();
        assert!(
            new_len < 1024,
            "new log file should be small after rotation, got {new_len}"
        );
    }

    #[test]
    fn redacts_user_home_and_private_var_paths_without_corrupting_system_paths() {
        let custom_home = Path::new("/Volumes/WorkHome/custom_user");
        let sample = concat!(
            "source=/Users/alice/Desktop/tax return.pdf\n",
            "url=file:///Users/bob/Documents/secret.md\n",
            "priv=/private/Users/charlie/Downloads/pic.heic\n",
            "tmp=/private/var/folders/38/htd8qbz91714b54kc33lt3b80000gn/T/.tmpXYZ/out.png\n",
            "var=/var/folders/ab/cdef1234/C/com.thetinyfoundry.flip.mac/cache.bin\n",
            "custom=/Volumes/WorkHome/custom_user/Projects/file.txt\n",
            "exact_home=\"/Users/alice\"\n",
            "brew=/opt/homebrew/bin/ffmpeg\n",
            "web=https://example.com/Users/guide\n",
        );

        let redacted = redact_paths_with_home(sample, Some(custom_home));
        assert!(!redacted.contains("/Users/alice"));
        assert!(!redacted.contains("/Users/bob"));
        assert!(!redacted.contains("/private/Users/charlie"));
        assert!(!redacted.contains("htd8qbz91714b54kc33lt3b80000gn"));
        assert!(!redacted.contains("cdef1234"));
        assert!(!redacted.contains("custom_user"));

        assert!(redacted.contains("source=~/Desktop/tax return.pdf"));
        assert!(redacted.contains("url=file://~/Documents/secret.md"));
        assert!(redacted.contains("priv=~/Downloads/pic.heic"));
        assert!(redacted.contains("tmp=~/T/.tmpXYZ/out.png"));
        assert!(redacted.contains("var=~/C/com.thetinyfoundry.flip.mac/cache.bin"));
        assert!(redacted.contains("custom=~/Projects/file.txt"));
        assert!(redacted.contains("exact_home=\"~\""));
        assert!(redacted.contains("brew=/opt/homebrew/bin/ffmpeg"));
        assert!(redacted.contains("web=https://example.com/Users/guide"));
    }

    #[test]
    fn tail_events_and_markdown_diagnostics_bundle_are_redacted_and_bounded() {
        let dir = tempdir().unwrap();
        let logger = DiagnosticLogger::new(dir.path().to_path_buf());
        for i in 1..=75 {
            logger.log(
                "conversion.completed",
                Some(json!({
                    "index": i,
                    "source": format!("/Users/secretuser/Desktop/doc_{i}.png"),
                    "staging": "/private/var/folders/11/abcxyz/T/.flip-job-123/out.jpg",
                })),
            );
        }

        let tail = DiagnosticLogger::tail_events_in_dir_with_home(
            dir.path(),
            DIAGNOSTICS_TAIL_LINES,
            Some(Path::new("/Users/secretuser")),
        );
        assert_eq!(tail.len(), 60);
        assert!(tail[0].contains("\"index\":16"));
        assert!(tail[59].contains("\"index\":75"));
        for line in &tail {
            assert!(!line.contains("secretuser"));
            assert!(!line.contains("abcxyz"));
            assert!(line.contains("~/Desktop/doc_"));
            assert!(line.contains("~/T/.flip-job-123/out.jpg"));
        }

        let mut prefs = AppPreferences::default();
        prefs.fallback_folder = Some("/Users/secretuser/Inbox".to_string());
        prefs.ffmpeg_path = Some("/Users/secretuser/.local/bin/ffmpeg".to_string());
        let tools = DetectedTools {
            ffmpeg: Some(PathBuf::from("/opt/homebrew/bin/ffmpeg")),
            ffprobe: Some(PathBuf::from("/opt/homebrew/bin/ffprobe")),
            pandoc: None,
            bridge: Some(PathBuf::from(
                "/Users/secretuser/Applications/FLIP.app/Contents/Resources/flip-macos-bridge",
            )),
            pages: false,
            numbers: false,
            keynote: false,
        };
        let perms = PermissionsStatus {
            accessibility: true,
            automation_finder: "granted".to_string(),
            input_monitoring: true,
        };
        let md = format_diagnostics_markdown(&DiagnosticsReportInput {
            app_version: "0.1.0",
            macos_version: "macOS 27.2.1 (26B131)",
            tools: &tools,
            permissions: Ok(&perms),
            prefs: &prefs,
            log_lines: &tail,
            home_dir: Some(Path::new("/Users/secretuser")),
        });

        assert!(md.contains("# FLIP Diagnostics Report"));
        assert!(md.contains("macOS 27.2.1 (26B131)"));
        assert!(md.contains("/opt/homebrew/bin/ffmpeg"));
        assert!(md.contains("~/Applications/FLIP.app/Contents/Resources/flip-macos-bridge"));
        assert!(md.contains("~/Inbox"));
        assert!(!md.contains("secretuser"), "redacted markdown leaked username:\n{md}");
        assert!(!md.contains("abcxyz"), "redacted markdown leaked var folder hash:\n{md}");
    }
}
