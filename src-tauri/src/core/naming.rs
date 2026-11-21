use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_FILENAME_BYTES: usize = 255;
pub const MAX_COLLISION_ATTEMPTS: usize = 10_000;

#[derive(Debug, Error)]
pub enum NamingError {
    #[error("Too many files with the same name; refusing to overwrite any existing file")]
    ExhaustedCollisions,
}

pub fn format_extension(format: &str) -> String {
    match format.trim().trim_start_matches('.').to_ascii_uppercase().as_str() {
        "JPG" | "JPEG" => "jpg".to_string(),
        "TIFF" | "TIF" => "tiff".to_string(),
        "MARKDOWN" => "md".to_string(),
        "HTM" => "html".to_string(),
        "YML" => "yaml".to_string(),
        "HEIF" => "heic".to_string(),
        "AIF" => "aiff".to_string(),
        "REMOVE_BG" => "png".to_string(),
        "OPTIMIZE_PDF" | "PDFA" => "pdf".to_string(),
        "TRANSCRIBE" => "txt".to_string(),
        "ZIP" => "zip".to_string(),
        "UNZIP" => String::new(),
        other => other.to_ascii_lowercase(),
    }
}

pub fn conversion_stem(source: &Path) -> String {
    let raw = source
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("output");
    raw.to_string()
}

pub fn clamp_filename_bytes(
    stem: &str,
    fixed_suffix: &str,
    ext_with_dot: &str,
    max_bytes: usize,
) -> String {
    let fixed_len = fixed_suffix.len() + ext_with_dot.len();
    if fixed_len >= max_bytes {
        // Extreme fallback if suffix+ext alone is huge
        let mut s = format!("{stem}{fixed_suffix}{ext_with_dot}");
        while s.len() > max_bytes {
            s.pop();
        }
        return s;
    }
    let budget = max_bytes - fixed_len;
    let mut end = stem.len().min(budget);
    while end > 0 && !stem.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = if end == 0 { "file" } else { &stem[..end] };
    format!("{truncated}{fixed_suffix}{ext_with_dot}")
}

/// Direct-stem conversion output path (NO `.converted` suffix, NEVER overwrites).
/// Collision naming follows Finder style: `<stem> 2.<ext>`, `<stem> 3.<ext>`, etc.
/// Multi-page outputs use `<stem>-p001.<ext>` (and `<stem>-p001 2.<ext>` on collision).
pub fn candidate_conversion_path(
    dir: &Path,
    stem: &str,
    page_suffix: Option<&str>,
    format: &str,
    counter: usize,
) -> PathBuf {
    let ext = format_extension(format);
    let ext_with_dot = if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    };
    let page = page_suffix.unwrap_or("");
    let counter_part = if counter <= 1 {
        String::new()
    } else {
        format!(" {counter}")
    };
    let fixed_suffix = format!("{page}{counter_part}");
    let filename = clamp_filename_bytes(stem, &fixed_suffix, &ext_with_dot, MAX_FILENAME_BYTES);
    dir.join(filename)
}

fn path_occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub fn next_available_conversion_path(
    dir: &Path,
    source: &Path,
    page_suffix: Option<&str>,
    format: &str,
) -> Result<PathBuf, NamingError> {
    let stem = conversion_stem(source);
    for counter in 1..=MAX_COLLISION_ATTEMPTS {
        let candidate = candidate_conversion_path(dir, &stem, page_suffix, format, counter);
        if !path_occupied(&candidate) {
            return Ok(candidate);
        }
    }
    Err(NamingError::ExhaustedCollisions)
}

/// Finder-style "Create New File" path:
/// - Regular file: `Untitled.<ext>`, `Untitled 2.<ext>`, `Untitled 3.<ext>`, ...
/// - Folder (`ext == None`): `untitled folder`, `untitled folder 2`, `untitled folder 3`, ...
pub fn candidate_new_item_path(
    dir: &Path,
    base_stem: &str,
    ext: Option<&str>,
    counter: usize,
) -> PathBuf {
    let ext_with_dot = match ext {
        Some(e) if !e.trim().is_empty() => format!(".{}", e.trim().trim_start_matches('.')),
        _ => String::new(),
    };
    let counter_part = if counter <= 1 {
        String::new()
    } else {
        format!(" {counter}")
    };
    let filename = clamp_filename_bytes(base_stem, &counter_part, &ext_with_dot, MAX_FILENAME_BYTES);
    dir.join(filename)
}

pub fn next_available_new_item_path(
    dir: &Path,
    base_stem: &str,
    ext: Option<&str>,
) -> Result<PathBuf, NamingError> {
    for counter in 1..=MAX_COLLISION_ATTEMPTS {
        let candidate = candidate_new_item_path(dir, base_stem, ext, counter);
        if !path_occupied(&candidate) {
            return Ok(candidate);
        }
    }
    Err(NamingError::ExhaustedCollisions)
}

pub fn clipboard_base_stem() -> String {
    format!("Clipboard {}", Local::now().format("%Y-%m-%d at %H.%M.%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn direct_stem_naming_and_finder_collisions() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.heic");
        fs::write(&src, b"raw").unwrap();

        let p1 = next_available_conversion_path(dir.path(), &src, None, "PNG").unwrap();
        assert_eq!(p1.file_name().unwrap().to_str().unwrap(), "photo.png");

        fs::write(&p1, b"existing").unwrap();
        let p2 = next_available_conversion_path(dir.path(), &src, None, "PNG").unwrap();
        assert_eq!(p2.file_name().unwrap().to_str().unwrap(), "photo 2.png");

        fs::write(&p2, b"existing2").unwrap();
        let p3 = next_available_conversion_path(dir.path(), &src, None, "PNG").unwrap();
        assert_eq!(p3.file_name().unwrap().to_str().unwrap(), "photo 3.png");
    }

    #[test]
    fn multipage_naming_uses_p001_and_collision_suffix() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("report.pdf");
        let page1 = next_available_conversion_path(dir.path(), &src, Some("-p001"), "PNG").unwrap();
        assert_eq!(page1.file_name().unwrap().to_str().unwrap(), "report-p001.png");

        fs::write(&page1, b"x").unwrap();
        let page1_dup =
            next_available_conversion_path(dir.path(), &src, Some("-p001"), "PNG").unwrap();
        assert_eq!(
            page1_dup.file_name().unwrap().to_str().unwrap(),
            "report-p001 2.png"
        );
    }

    #[test]
    fn unicode_and_long_255_byte_clamping_stays_valid_utf8() {
        let dir = tempdir().unwrap();
        let long_stem = "è🎉".repeat(120); // well over 255 bytes
        let src = dir.path().join(format!("{long_stem}.txt"));
        let p = candidate_conversion_path(dir.path(), &conversion_stem(&src), Some("-p001"), "PNG", 2);
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.len() <= 255, "filename exceeded 255 bytes: {}", name.len());
        assert!(name.ends_with("-p001 2.png"));
    }

    #[test]
    fn transcribe_outputs_use_txt_extension() {
        assert_eq!(format_extension("TRANSCRIBE"), "txt");
        assert_eq!(format_extension(" transcribe "), "txt");

        let dir = tempdir().unwrap();
        let src = dir.path().join("interview.m4a");
        fs::write(&src, b"audio").unwrap();
        let p = next_available_conversion_path(dir.path(), &src, None, "TRANSCRIBE").unwrap();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "interview.txt");
        fs::write(&p, b"first").unwrap();
        let p2 = next_available_conversion_path(dir.path(), &src, None, "TRANSCRIBE").unwrap();
        assert_eq!(p2.file_name().unwrap().to_str().unwrap(), "interview 2.txt");
    }

    #[test]
    fn pdfa_outputs_are_named_like_optimize_pdf_results() {
        assert_eq!(format_extension("PDFA"), "pdf");
        assert_eq!(format_extension(" pdfa "), "pdf");
        assert_eq!(format_extension("PDFA"), format_extension("OPTIMIZE_PDF"));

        let dir = tempdir().unwrap();
        let src = dir.path().join("report.pdf");
        fs::write(&src, b"%PDF-1.7").unwrap();
        // The source already owns `report.pdf`, so the result is the next free numbered name.
        let p = next_available_conversion_path(dir.path(), &src, None, "PDFA").unwrap();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "report 2.pdf");
        let optimize = next_available_conversion_path(dir.path(), &src, None, "OPTIMIZE_PDF").unwrap();
        assert_eq!(optimize, p, "PDFA and OPTIMIZE_PDF must name results the same way");
        fs::write(&p, b"first").unwrap();
        let p2 = next_available_conversion_path(dir.path(), &src, None, "PDFA").unwrap();
        assert_eq!(p2.file_name().unwrap().to_str().unwrap(), "report 3.pdf");
    }

    #[test]
    fn create_new_item_naming_file_and_folder() {
        let dir = tempdir().unwrap();
        let u1 = next_available_new_item_path(dir.path(), "Untitled", Some("txt")).unwrap();
        assert_eq!(u1.file_name().unwrap().to_str().unwrap(), "Untitled.txt");
        fs::write(&u1, b"").unwrap();
        let u2 = next_available_new_item_path(dir.path(), "Untitled", Some("txt")).unwrap();
        assert_eq!(u2.file_name().unwrap().to_str().unwrap(), "Untitled 2.txt");

        let f1 = next_available_new_item_path(dir.path(), "untitled folder", None).unwrap();
        assert_eq!(f1.file_name().unwrap().to_str().unwrap(), "untitled folder");
        fs::create_dir(&f1).unwrap();
        let f2 = next_available_new_item_path(dir.path(), "untitled folder", None).unwrap();
        assert_eq!(f2.file_name().unwrap().to_str().unwrap(), "untitled folder 2");
    }
}
