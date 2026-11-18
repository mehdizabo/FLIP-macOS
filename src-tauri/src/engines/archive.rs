use crate::core::naming::{clamp_filename_bytes, MAX_FILENAME_BYTES};
use crate::core::policy::{
    PolicyError, MAX_BATCH_FILES, MAX_CONTAINER_ENTRIES, MAX_CONTAINER_EXPANDED_BYTES,
};
use crate::engines::EngineError;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const ZIP_COPY_CHUNK_BYTES: usize = 64 * 1024;
const ZIP64_THRESHOLD_BYTES: u64 = 3_500_000_000;

pub fn unique_zip_entry_name(source: &Path, used_lower: &mut HashSet<String>) -> String {
    let raw_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains('\\'))
        .unwrap_or("file");

    if used_lower.insert(raw_name.to_ascii_lowercase()) {
        return raw_name.to_string();
    }

    let (stem, dot_ext) = match raw_name.rfind('.') {
        Some(idx) if idx > 0 => (&raw_name[..idx], &raw_name[idx..]),
        _ => (raw_name, ""),
    };

    for counter in 2..=crate::core::naming::MAX_COLLISION_ATTEMPTS {
        let candidate = clamp_filename_bytes(
            stem,
            &format!(" ({counter})"),
            dot_ext,
            MAX_FILENAME_BYTES,
        );
        if used_lower.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }

    format!("{raw_name}_{}", used_lower.len() + 1)
}

pub fn zip_files(
    sources: &[PathBuf],
    output_zip: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    if sources.is_empty() || sources.len() > MAX_BATCH_FILES {
        return Err(PolicyError::InvalidBatchCount.into());
    }

    let mut total_input_bytes: u64 = 0;
    for src in sources {
        let meta = fs::symlink_metadata(src)
            .map_err(|_| PolicyError::MissingFile(src.display().to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(PolicyError::SymlinkRefused(src.display().to_string()).into());
        }
        if meta.is_dir() {
            return Err(PolicyError::DirectoryNotAllowed(src.display().to_string()).into());
        }
        if !meta.is_file() {
            return Err(PolicyError::InvalidPath(src.display().to_string()).into());
        }
        total_input_bytes = total_input_bytes.saturating_add(meta.len());
    }

    let use_zip64 = total_input_bytes >= ZIP64_THRESHOLD_BYTES;
    let out_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_zip)?;
    let mut writer = ZipWriter::new(out_file);
    let mut used_lower: HashSet<String> = HashSet::with_capacity(sources.len());
    let mut buf = vec![0u8; ZIP_COPY_CHUNK_BYTES];

    for src in sources {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let meta = fs::symlink_metadata(src)?;
        let mode = meta.permissions().mode();
        let safe_mode = if (mode & 0o111) != 0 { 0o755 } else { 0o644 };
        let entry_name = unique_zip_entry_name(src, &mut used_lower);

        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(safe_mode)
            .large_file(use_zip64);

        writer
            .start_file(&entry_name, opts)
            .map_err(|e| EngineError::ConversionFailed(format!("Failed to add `{entry_name}` to zip: {e}")))?;

        let mut in_f = File::open(src)?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            let n = in_f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
        }
    }

    let finished = writer
        .finish()
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to finalize zip archive: {e}")))?;
    finished.sync_all()?;
    Ok(())
}

fn validate_safe_zip_relative_path(raw: &str) -> Result<PathBuf, PolicyError> {
    if raw.is_empty() || raw.contains('\0') || raw.contains('\\') || raw.starts_with('/') {
        return Err(PolicyError::InvalidContainer(format!(
            "Unsafe path inside zip archive: `{raw}`"
        )));
    }
    let mut out = PathBuf::new();
    for comp in Path::new(raw).components() {
        match comp {
            Component::Normal(seg) => {
                if seg.is_empty() {
                    return Err(PolicyError::InvalidContainer(format!(
                        "Empty path segment inside zip archive: `{raw}`"
                    )));
                }
                out.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PolicyError::InvalidContainer(format!(
                    "Zip-slip path traversal rejected: `{raw}`"
                )));
            }
        }
    }
    Ok(out)
}

pub fn unzip_archive(
    zip_path: &Path,
    extract_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, EngineError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    fs::create_dir(extract_dir)?;

    let file = File::open(zip_path).map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;

    if archive.is_empty() {
        return Err(PolicyError::InvalidContainer("Zip archive contains no entries.".into()).into());
    }
    if archive.len() > MAX_CONTAINER_ENTRIES {
        return Err(PolicyError::ContainerTooManyEntries.into());
    }

    let mut declared_total: u64 = 0;
    let mut actual_total: u64 = 0;

    for i in 0..archive.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }

        let mut entry = archive
            .by_index(i)
            .map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;

        const S_IFMT: u32 = 0o170000;
        const S_IFLNK: u32 = 0o120000;
        const S_IFREG: u32 = 0o100000;
        const S_IFDIR: u32 = 0o040000;

        if entry.is_symlink() {
            return Err(PolicyError::InvalidContainer(format!(
                "Refusing symlink entry inside archive: `{}`",
                entry.name()
            ))
            .into());
        }
        if let Some(mode) = entry.unix_mode() {
            let ftype = mode & S_IFMT;
            if ftype == S_IFLNK || (ftype != 0 && ftype != S_IFREG && ftype != S_IFDIR) {
                return Err(PolicyError::InvalidContainer(format!(
                    "Refusing unsafe entry mode inside archive: `{}`",
                    entry.name()
                ))
                .into());
            }
        }

        let raw_name = entry.name().to_string();
        if entry.enclosed_name().is_none() {
            return Err(PolicyError::InvalidContainer(format!(
                "Zip-slip traversal rejected: `{raw_name}`"
            ))
            .into());
        }
        let rel = validate_safe_zip_relative_path(&raw_name)?;

        declared_total = declared_total
            .checked_add(entry.size())
            .ok_or(PolicyError::ContainerBombDetected)?;
        if declared_total > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(PolicyError::ContainerBombDetected.into());
        }

        if rel.as_os_str().is_empty() {
            if entry.is_dir() {
                continue;
            }
            return Err(PolicyError::InvalidContainer(format!(
                "Empty file entry path inside zip archive: `{raw_name}`"
            ))
            .into());
        }

        let dest = extract_dir.join(&rel);
        if entry.is_dir() || raw_name.ends_with('/') {
            fs::create_dir_all(&dest)?;
            continue;
        }

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out_f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&dest)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    EngineError::Policy(PolicyError::InvalidContainer(format!(
                        "Duplicate entry path in zip archive: `{raw_name}`"
                    )))
                } else {
                    EngineError::Io(e)
                }
            })?;

        let remaining = MAX_CONTAINER_EXPANDED_BYTES.saturating_sub(actual_total);
        let mut limited = (&mut entry).take(remaining + 1);
        let copied = io::copy(&mut limited, &mut out_f)
            .map_err(|e| PolicyError::InvalidContainer(e.to_string()))?;

        actual_total = actual_total
            .checked_add(copied)
            .ok_or(PolicyError::ContainerBombDetected)?;
        if actual_total > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(PolicyError::ContainerBombDetected.into());
        }

        if let Some(mode) = entry.unix_mode() {
            if (mode & 0o111) != 0 {
                let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o755));
            }
        }
    }

    Ok(extract_dir.to_path_buf())
}
