use crate::core::preferences::AppPreferences;
use serde::{Deserialize, Serialize};
use std::fs::{self, Permissions};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

pub const SOCKET_FILE_NAME: &str = "flip-instance.sock";
const MAX_IPC_LINE_BYTES: u64 = 1024 * 1024;
const MAX_CONCURRENT_CLIENTS: usize = 16;
const MACOS_SUN_PATH_MAX: usize = 103;

#[derive(Debug, Error)]
pub enum IpcSocketError {
    #[error("FLIP instance is not running ({0})")]
    NotRunning(String),
    #[error("Another FLIP instance is already listening at {0}")]
    AlreadyRunning(String),
    #[error("Invalid Unix socket path: {0}")]
    InvalidSocketPath(String),
    #[error("IPC protocol error: {0}")]
    Protocol(String),
    #[error("IPC IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    Ping,
    Activate,
    OpenUrl {
        url: String,
    },
    OpenWheel {
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        folder: Option<String>,
    },
    OpenSettings,
    OpenBatch {
        #[serde(default)]
        files: Vec<String>,
    },
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

impl IpcResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            error: None,
            pid: Some(std::process::id()),
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: None,
            error: Some(error.into()),
            pid: Some(std::process::id()),
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    AppPreferences::app_support_dir().join(SOCKET_FILE_NAME)
}

fn is_shared_system_dir(dir: &Path) -> bool {
    let s = dir.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    let tmp_trimmed = std::env::temp_dir()
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    matches!(
        trimmed,
        "" | "/" | "/tmp" | "/private/tmp" | "/var/tmp" | "/var/folders" | "/Users"
    ) || trimmed == tmp_trimmed
        || dirs::home_dir().is_some_and(|h| h == dir)
}

fn ensure_secure_parent_dir(socket_path: &Path) -> Result<(), IpcSocketError> {
    let parent = socket_path.parent().ok_or_else(|| {
        IpcSocketError::InvalidSocketPath(format!(
            "Socket path has no parent directory: {}",
            socket_path.display()
        ))
    })?;

    let created = if let Ok(meta) = fs::symlink_metadata(parent) {
        if meta.file_type().is_symlink() {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Socket parent directory must not be a symlink: {}",
                parent.display()
            )));
        }
        if !meta.is_dir() {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Socket parent is not a directory: {}",
                parent.display()
            )));
        }
        let my_uid = unsafe { libc::geteuid() };
        if meta.uid() != my_uid {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Socket parent directory is not owned by current user: {}",
                parent.display()
            )));
        }
        false
    } else {
        fs::create_dir_all(parent)?;
        true
    };

    if (created || !is_shared_system_dir(parent)) && parent.is_dir() {
        fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn validate_socket_path_length(socket_path: &Path) -> Result<(), IpcSocketError> {
    if socket_path.as_os_str().len() > MACOS_SUN_PATH_MAX {
        return Err(IpcSocketError::InvalidSocketPath(format!(
            "Socket path exceeds {MACOS_SUN_PATH_MAX}-byte macOS Unix socket limit: {}",
            socket_path.display()
        )));
    }
    Ok(())
}

fn verify_peer_same_user(stream: &UnixStream) -> bool {
    let mut euid: libc::uid_t = 0;
    let mut egid: libc::gid_t = 0;
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut euid, &mut egid) };
    if rc != 0 {
        return false;
    }
    let my_uid = unsafe { libc::geteuid() };
    euid == my_uid
}

pub struct IpcServerHandle {
    socket_path: PathBuf,
    bound_identity: (u64, u64),
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    cleaned: bool,
}

impl IpcServerHandle {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn stop(&mut self) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        if let Ok(meta) = fs::symlink_metadata(&self.socket_path) {
            if meta.file_type().is_socket() && (meta.dev(), meta.ino()) == self.bound_identity {
                let _ = fs::remove_file(&self.socket_path);
            }
        }
    }
}

impl Drop for IpcServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_server<F>(socket_path: &Path, handler: F) -> Result<IpcServerHandle, IpcSocketError>
where
    F: Fn(IpcCommand) -> IpcResponse + Send + Sync + 'static,
{
    validate_socket_path_length(socket_path)?;
    ensure_secure_parent_dir(socket_path)?;

    if let Ok(meta) = fs::symlink_metadata(socket_path) {
        if meta.file_type().is_symlink() {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Refusing to bind over symlink at {}",
                socket_path.display()
            )));
        }
        if !meta.file_type().is_socket() {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Refusing to overwrite non-socket file at {}",
                socket_path.display()
            )));
        }
        let my_uid = unsafe { libc::geteuid() };
        if meta.uid() != my_uid {
            return Err(IpcSocketError::InvalidSocketPath(format!(
                "Refusing socket owned by another user at {}",
                socket_path.display()
            )));
        }

        match UnixStream::connect(socket_path) {
            Ok(_active_stream) => {
                return Err(IpcSocketError::AlreadyRunning(
                    socket_path.display().to_string(),
                ));
            }
            Err(e) if matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound) => {
                // Stale socket left by a crashed process: safe to remove before binding.
                let _ = fs::remove_file(socket_path);
            }
            Err(e) => {
                return Err(IpcSocketError::AlreadyRunning(format!(
                    "{} ({e})",
                    socket_path.display()
                )));
            }
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    if let Err(err) = fs::set_permissions(socket_path, Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(socket_path);
        return Err(IpcSocketError::Io(err));
    }

    let meta = fs::symlink_metadata(socket_path)?;
    let bound_identity = (meta.dev(), meta.ino());
    listener.set_nonblocking(true)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_loop = Arc::clone(&shutdown);
    let handler_arc = Arc::new(handler);
    let active_clients = Arc::new(AtomicUsize::new(0));

    let thread = thread::Builder::new()
        .name("flip-ipc-socket".to_string())
        .spawn(move || {
            let fd = listener.as_raw_fd();
            while !shutdown_loop.load(Ordering::Acquire) {
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let poll_rc = unsafe { libc::poll(&mut pfd, 1, 50) };
                if poll_rc < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                if poll_rc == 0 || (pfd.revents & libc::POLLIN) == 0 {
                    continue;
                }

                loop {
                    if shutdown_loop.load(Ordering::Acquire) {
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _addr)) => {
                            if active_clients.load(Ordering::Relaxed) >= MAX_CONCURRENT_CLIENTS {
                                let mut s = stream;
                                let resp = IpcResponse::err("Too many concurrent IPC clients");
                                if let Ok(line) = serde_json::to_string(&resp) {
                                    let _ = writeln!(s, "{line}");
                                    let _ = s.flush();
                                }
                                continue;
                            }
                            let h = Arc::clone(&handler_arc);
                            let counter = Arc::clone(&active_clients);
                            counter.fetch_add(1, Ordering::Relaxed);
                            thread::spawn(move || {
                                handle_client_connection(stream, &*h);
                                counter.fetch_sub(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
        })?;

    Ok(IpcServerHandle {
        socket_path: socket_path.to_path_buf(),
        bound_identity,
        shutdown,
        thread: Some(thread),
        cleaned: false,
    })
}

fn handle_client_connection<F>(mut stream: UnixStream, handler: &F)
where
    F: Fn(IpcCommand) -> IpcResponse,
{
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    if !verify_peer_same_user(&stream) {
        let resp = IpcResponse::err("Unauthorized IPC peer UID");
        if let Ok(line) = serde_json::to_string(&resp) {
            let _ = writeln!(stream, "{line}");
            let _ = stream.flush();
        }
        return;
    }

    let mut line = String::new();
    let read_res = {
        let mut reader = BufReader::new((&stream).take(MAX_IPC_LINE_BYTES));
        reader.read_line(&mut line)
    };

    let response = match read_res {
        Ok(0) => return,
        Ok(_) => {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return;
            }
            match serde_json::from_str::<IpcCommand>(trimmed) {
                Ok(cmd) => handler(cmd),
                Err(err) => IpcResponse::err(format!("Invalid IPC command: {err}")),
            }
        }
        Err(err) => IpcResponse::err(format!("Failed to read IPC request: {err}")),
    };

    if let Ok(encoded) = serde_json::to_string(&response) {
        let _ = writeln!(stream, "{encoded}");
        let _ = stream.flush();
    }
}

pub fn send_command(socket_path: &Path, cmd: &IpcCommand) -> Result<IpcResponse, IpcSocketError> {
    send_command_with_timeout(socket_path, cmd, Duration::from_secs(5))
}

pub fn send_command_with_timeout(
    socket_path: &Path,
    cmd: &IpcCommand,
    timeout: Duration,
) -> Result<IpcResponse, IpcSocketError> {
    validate_socket_path_length(socket_path)?;

    let meta = fs::symlink_metadata(socket_path).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            IpcSocketError::NotRunning(socket_path.display().to_string())
        } else {
            IpcSocketError::Io(e)
        }
    })?;

    if meta.file_type().is_symlink() || !meta.file_type().is_socket() {
        return Err(IpcSocketError::InvalidSocketPath(format!(
            "Target is not a user Unix socket: {}",
            socket_path.display()
        )));
    }
    let my_uid = unsafe { libc::geteuid() };
    if meta.uid() != my_uid {
        return Err(IpcSocketError::InvalidSocketPath(format!(
            "Socket is not owned by current user: {}",
            socket_path.display()
        )));
    }

    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        if matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound) {
            IpcSocketError::NotRunning(socket_path.display().to_string())
        } else {
            IpcSocketError::Io(e)
        }
    })?;

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let payload = serde_json::to_string(cmd)
        .map_err(|e| IpcSocketError::Protocol(format!("Failed to serialize command: {e}")))?;
    writeln!(stream, "{payload}")?;
    stream.flush()?;

    let mut resp_line = String::new();
    let mut reader = BufReader::new(stream.take(MAX_IPC_LINE_BYTES));
    let n = reader.read_line(&mut resp_line)?;
    if n == 0 || resp_line.trim().is_empty() {
        return Err(IpcSocketError::Protocol(
            "Empty response from FLIP IPC host".to_string(),
        ));
    }
    serde_json::from_str::<IpcResponse>(resp_line.trim())
        .map_err(|e| IpcSocketError::Protocol(format!("Invalid IPC response JSON: {e}")))
}

pub fn is_server_listening(socket_path: &Path) -> bool {
    send_command_with_timeout(socket_path, &IpcCommand::Ping, Duration::from_millis(500))
        .is_ok_and(|r| r.ok)
}
