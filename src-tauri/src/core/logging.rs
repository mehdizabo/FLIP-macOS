use chrono::Utc;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

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
}

static GLOBAL_LOGGER: OnceLock<DiagnosticLogger> = OnceLock::new();

pub fn global_logger() -> &'static DiagnosticLogger {
    GLOBAL_LOGGER.get_or_init(|| DiagnosticLogger::new(DiagnosticLogger::default_log_dir()))
}

pub fn log_event(name: &str, detail: Option<Value>) {
    global_logger().log(name, detail);
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
        assert!(new_len < 1024, "new log file should be small after rotation, got {new_len}");
    }
}
