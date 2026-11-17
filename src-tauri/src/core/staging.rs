use crate::core::naming::{candidate_conversion_path, conversion_stem};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StagingError {
    #[error("Refusing to clean an unowned temporary directory: {0}")]
    UnownedDirectory(String),
    #[error("Refusing to follow a symlink inside staging directory: {0}")]
    SymlinkRefused(String),
    #[error("IO failure during staging or commit: {0}")]
    Io(#[from] io::Error),
    #[error("Collision limit reached without overwriting existing files")]
    CollisionExhausted,
}

pub struct JobStaging {
    parent: PathBuf,
    job_dir: PathBuf,
    cleaned: bool,
}

impl JobStaging {
    pub fn create(parent_dir: &Path) -> Result<Self, StagingError> {
        let job_name = format!(".flip-job-{}", Uuid::new_v4().simple());
        let job_dir = parent_dir.join(job_name);
        fs::create_dir(&job_dir)?;
        Ok(Self {
            parent: parent_dir.to_path_buf(),
            job_dir,
            cleaned: false,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.job_dir
    }

    pub fn parent(&self) -> &Path {
        &self.parent
    }

    pub fn staged_path(&self, filename: &str) -> PathBuf {
        self.job_dir.join(filename)
    }

    /// Atomically commits `staged` to `<parent>/<stem><page_suffix>.<ext>`
    /// (or `<stem><page_suffix> 2.<ext>` on collision) WITHOUT ever overwriting.
    pub fn commit_one(
        &self,
        staged: &Path,
        source: &Path,
        page_suffix: Option<&str>,
        format: &str,
    ) -> Result<PathBuf, StagingError> {
        let stem = conversion_stem(source);
        let staged_meta = fs::symlink_metadata(staged)?;
        if staged_meta.file_type().is_symlink() {
            return Err(StagingError::SymlinkRefused(staged.display().to_string()));
        }

        for counter in 1..=crate::core::naming::MAX_COLLISION_ATTEMPTS {
            let dest = candidate_conversion_path(&self.parent, &stem, page_suffix, format, counter);
            if staged_meta.is_dir() {
                if fs::symlink_metadata(&dest).is_err() {
                    match fs::rename(staged, &dest) {
                        Ok(()) => return Ok(dest),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(StagingError::Io(e)),
                    }
                }
            } else {
                match fs::hard_link(staged, &dest) {
                    Ok(()) => {
                        let _ = fs::remove_file(staged);
                        return Ok(dest);
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(StagingError::Io(e)),
                }
            }
        }
        Err(StagingError::CollisionExhausted)
    }

    pub fn commit_all(
        &self,
        staged_files: &[PathBuf],
        source: &Path,
        format: &str,
        multi_page: bool,
    ) -> Result<Vec<PathBuf>, StagingError> {
        let mut committed: Vec<PathBuf> = Vec::with_capacity(staged_files.len());
        let use_page_suffix = multi_page || staged_files.len() > 1;
        for (idx, staged) in staged_files.iter().enumerate() {
            let suffix = if use_page_suffix {
                Some(format!("-p{:03}", idx + 1))
            } else {
                None
            };
            match self.commit_one(staged, source, suffix.as_deref(), format) {
                Ok(dest) => committed.push(dest),
                Err(err) => {
                    for prev in committed {
                        let _ = fs::remove_file(prev);
                    }
                    return Err(err);
                }
            }
        }
        Ok(committed)
    }

    pub fn cleanup(mut self) -> Result<(), StagingError> {
        self.cleaned = true;
        delete_job_dir(&self.job_dir, &self.parent)
    }
}

impl Drop for JobStaging {
    fn drop(&mut self) {
        if !self.cleaned && self.job_dir.exists() {
            let _ = delete_job_dir(&self.job_dir, &self.parent);
            self.cleaned = true;
        }
    }
}

pub fn delete_job_dir(job_dir: &Path, expected_parent: &Path) -> Result<(), StagingError> {
    let job_name = job_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| StagingError::UnownedDirectory(job_dir.display().to_string()))?;

    let uuid_part = job_name
        .strip_prefix(".flip-job-")
        .ok_or_else(|| StagingError::UnownedDirectory(job_dir.display().to_string()))?;

    if uuid_part.len() != 32 || !uuid_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StagingError::UnownedDirectory(job_dir.display().to_string()));
    }

    let actual_parent = job_dir
        .parent()
        .ok_or_else(|| StagingError::UnownedDirectory(job_dir.display().to_string()))?;
    let canon_actual = fs::canonicalize(actual_parent).unwrap_or_else(|_| actual_parent.to_path_buf());
    let canon_expected =
        fs::canonicalize(expected_parent).unwrap_or_else(|_| expected_parent.to_path_buf());
    if canon_actual != canon_expected {
        return Err(StagingError::UnownedDirectory(job_dir.display().to_string()));
    }

    clean_recursive(job_dir)
}

fn clean_recursive(dir: &Path) -> Result<(), StagingError> {
    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(StagingError::SymlinkRefused(dir.display().to_string()));
    }
    let mut perms = meta.permissions();
    if perms.readonly() {
        perms.set_mode(0o755);
        fs::set_permissions(dir, perms)?;
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let emeta = fs::symlink_metadata(&path)?;
        if emeta.file_type().is_symlink() {
            return Err(StagingError::SymlinkRefused(path.display().to_string()));
        }
        if emeta.is_dir() {
            clean_recursive(&path)?;
        } else {
            let mut fperms = emeta.permissions();
            if fperms.readonly() {
                fperms.set_mode(0o644);
                fs::set_permissions(&path, fperms)?;
            }
            fs::remove_file(&path)?;
        }
    }
    fs::remove_dir(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn readonly_lock_cleaned_and_unowned_cleanup_refused() {
        let dir = tempdir().unwrap();
        let job = JobStaging::create(dir.path()).unwrap();
        let job_path = job.dir().to_path_buf();

        let lock_file = job_path.join(".~lock.prepared.pdf#");
        fs::write(&lock_file, b"synthetic office lock").unwrap();
        let mut perms = fs::metadata(&lock_file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&lock_file, perms).unwrap();

        job.cleanup().unwrap();
        assert!(!job_path.exists(), "staging job directory must be removed");

        // Unowned directory must be refused
        let unowned = dir.path().join("user-folder");
        fs::create_dir(&unowned).unwrap();
        let res = delete_job_dir(&unowned, dir.path());
        assert!(matches!(res, Err(StagingError::UnownedDirectory(_))));
        assert!(unowned.exists());
    }

    #[test]
    fn concurrent_commits_never_overwrite() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("photo.png");
        fs::write(&source, b"orig").unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(4));
        let mut handles = Vec::new();
        for i in 0..4 {
            let dir_path = dir.path().to_path_buf();
            let src_path = source.clone();
            let b = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let job = JobStaging::create(&dir_path).unwrap();
                let staged = job.staged_path("out.jpg");
                fs::write(&staged, format!("payload-{i}")).unwrap();
                b.wait();
                let out = job.commit_one(&staged, &src_path, None, "JPG").unwrap();
                job.cleanup().unwrap();
                out
            }));
        }

        let outputs: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let unique: std::collections::HashSet<_> = outputs.iter().collect();
        assert_eq!(unique.len(), 4, "all 4 concurrent commits must get distinct filenames");
        assert!(dir.path().join("photo.jpg").exists());
        assert!(dir.path().join("photo 2.jpg").exists());
        assert!(dir.path().join("photo 3.jpg").exists());
        assert!(dir.path().join("photo 4.jpg").exists());
    }

    #[test]
    fn collision_preserves_existing_file_content() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("immagine è.png");
        fs::write(&src, b"png-bytes").unwrap();
        let existing = dir.path().join("immagine è.jpg");
        fs::write(&existing, b"DO_NOT_OVERWRITE").unwrap();

        let job = JobStaging::create(dir.path()).unwrap();
        let staged = job.staged_path("out.jpg");
        fs::write(&staged, b"new-jpg-bytes").unwrap();
        let committed = job.commit_one(&staged, &src, None, "JPG").unwrap();
        job.cleanup().unwrap();

        assert_eq!(committed, dir.path().join("immagine è 2.jpg"));
        assert_eq!(fs::read(&existing).unwrap(), b"DO_NOT_OVERWRITE");
        assert_eq!(fs::read(&committed).unwrap(), b"new-jpg-bytes");
    }

    #[test]
    fn drop_cleans_up_on_mid_job_abort() {
        let dir = tempdir().unwrap();
        let job_path;
        {
            let job = JobStaging::create(dir.path()).unwrap();
            job_path = job.dir().to_path_buf();
            fs::write(job.staged_path("partial.png"), b"incomplete").unwrap();
            assert!(job_path.exists());
        } // dropped without commit or cleanup
        assert!(!job_path.exists(), "Drop must remove leftover .flip-job-* dir");
    }
}
