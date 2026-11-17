use crate::core::logging::log_event;
use crate::core::preferences::AppPreferences;
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const LAUNCH_AGENT_LABEL: &str = "com.thetinyfoundry.flip.mac";
pub const LAUNCH_AGENT_PLIST_NAME: &str = "com.thetinyfoundry.flip.mac.plist";
pub const APPLICATIONS_FLIP_EXE: &str = "/Applications/FLIP.app/Contents/MacOS/flip";
pub const LOGIN_ITEM_ARG: &str = "--login-item";

pub struct GuiInstanceLock {
    _file: File,
}

pub fn default_launch_agents_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Library")
        .join("LaunchAgents")
}

pub fn launch_agent_plist_path(dir: &Path) -> PathBuf {
    dir.join(LAUNCH_AGENT_PLIST_NAME)
}

pub fn resolve_launch_executable(current_exe: Option<&Path>, applications_exe: &Path) -> PathBuf {
    if let Some(exe) = current_exe {
        if exe.is_file() {
            let s = exe.to_string_lossy();
            if s.contains(".app/Contents/MacOS/") {
                return exe.to_path_buf();
            }
            if applications_exe.is_file() {
                return applications_exe.to_path_buf();
            }
            return exe.to_path_buf();
        }
    }
    applications_exe.to_path_buf()
}

pub fn current_launch_executable() -> PathBuf {
    let cur = std::env::current_exe().ok().map(|p| {
        if p.to_string_lossy().contains(".app/Contents/MacOS/") {
            p
        } else {
            fs::canonicalize(&p).unwrap_or(p)
        }
    });
    resolve_launch_executable(cur.as_deref(), Path::new(APPLICATIONS_FLIP_EXE))
}

pub fn xml_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn render_launch_agent_plist(exe_path: &Path) -> String {
    let escaped_exe = xml_escape(&exe_path.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LAUNCH_AGENT_LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{escaped_exe}</string>
		<string>{LOGIN_ITEM_ARG}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
"#
    )
}

pub fn is_launch_at_login_enabled_in_dir(dir: &Path) -> bool {
    let plist_path = launch_agent_plist_path(dir);
    if !plist_path.is_file() {
        return false;
    }
    fs::read_to_string(&plist_path)
        .map(|s| !s.trim().is_empty() && s.contains(LAUNCH_AGENT_LABEL))
        .unwrap_or(false)
}

pub fn is_launch_at_login_enabled() -> bool {
    is_launch_at_login_enabled_in_dir(&default_launch_agents_dir())
}

pub fn sync_preferences_with_launch_agent_in_dir(prefs: &mut AppPreferences, dir: &Path) -> bool {
    let enabled = is_launch_at_login_enabled_in_dir(dir);
    prefs.launch_at_login = enabled;
    enabled
}

pub fn sync_preferences_with_launch_agent(prefs: &mut AppPreferences) -> bool {
    sync_preferences_with_launch_agent_in_dir(prefs, &default_launch_agents_dir())
}

fn current_gui_domain() -> String {
    let uid = unsafe { libc::geteuid() };
    format!("gui/{uid}")
}

fn is_running_under_own_launch_agent() -> bool {
    std::env::var("XPC_SERVICE_NAME")
        .map(|val| val == LAUNCH_AGENT_LABEL)
        .unwrap_or(false)
}

fn run_launchctl_bounded(args: &[String]) -> Result<(), String> {
    let launchctl = Path::new("/bin/launchctl");
    if !launchctl.is_file() {
        return Err("/bin/launchctl not available".to_string());
    }
    let mut cmd = Command::new(launchctl);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn launchctl: {e}"))?;
    let start = Instant::now();
    let timeout = Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(format!("launchctl exited with {:?}", status.code()));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("launchctl timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("launchctl wait error: {e}"));
            }
        }
    }
}

fn launchctl_bootout_best_effort(plist_path: &Path) {
    if is_running_under_own_launch_agent() {
        // Avoid sending SIGTERM to ourselves if this process was started by our own LaunchAgent.
        return;
    }
    let domain = current_gui_domain();
    let target = format!("{domain}/{LAUNCH_AGENT_LABEL}");
    if run_launchctl_bounded(&["bootout".to_string(), target]).is_err() {
        let _ = run_launchctl_bounded(&[
            "bootout".to_string(),
            domain,
            plist_path.to_string_lossy().into_owned(),
        ]);
    }
}

fn launchctl_bootstrap_best_effort(plist_path: &Path) {
    let domain = current_gui_domain();
    let plist_str = plist_path.to_string_lossy().into_owned();
    if let Err(err) = run_launchctl_bounded(&["bootstrap".to_string(), domain, plist_str]) {
        log_event(
            "startup.launchctl.bootstrap_skipped",
            Some(json!({ "reason": err })),
        );
    }
}

pub fn set_launch_at_login_in_dir(
    dir: &Path,
    enabled: bool,
    exe_override: Option<&Path>,
    run_launchctl: bool,
) -> io::Result<bool> {
    let plist_path = launch_agent_plist_path(dir);
    if enabled {
        fs::create_dir_all(dir)?;
        let exe = exe_override
            .map(Path::to_path_buf)
            .unwrap_or_else(current_launch_executable);
        let content = render_launch_agent_plist(&exe);

        let temp = dir.join(format!(
            "{}.{}.tmp",
            LAUNCH_AGENT_PLIST_NAME,
            Uuid::new_v4().simple()
        ));
        if let Err(e) = fs::write(&temp, content.as_bytes()) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        if let Err(e) = fs::rename(&temp, &plist_path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }

        if run_launchctl {
            launchctl_bootout_best_effort(&plist_path);
            launchctl_bootstrap_best_effort(&plist_path);
        }
        log_event(
            "startup.launch_agent.enabled",
            Some(json!({ "plist": plist_path.to_string_lossy(), "exe": exe.to_string_lossy() })),
        );
        Ok(true)
    } else {
        if run_launchctl {
            launchctl_bootout_best_effort(&plist_path);
        }
        if fs::symlink_metadata(&plist_path).is_ok() {
            fs::remove_file(&plist_path)?;
        }
        log_event(
            "startup.launch_agent.disabled",
            Some(json!({ "plist": plist_path.to_string_lossy() })),
        );
        Ok(false)
    }
}

pub fn set_launch_at_login(enabled: bool) -> Result<bool, String> {
    set_launch_at_login_in_dir(&default_launch_agents_dir(), enabled, None, true)
        .map_err(|e| format!("Failed to update LaunchAgent: {e}"))
}

pub fn gui_lock_path() -> PathBuf {
    AppPreferences::app_support_dir().join("gui.lock")
}

pub fn try_acquire_gui_lock_at(lock_path: &Path) -> Option<GuiInstanceLock> {
    if let Some(parent) = lock_path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return None;
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(GuiInstanceLock { _file: file })
    } else {
        None
    }
}

pub fn acquire_gui_lock() -> Option<GuiInstanceLock> {
    try_acquire_gui_lock_at(&gui_lock_path())
}

pub fn is_gui_instance_running_at(lock_path: &Path) -> bool {
    if !lock_path.exists() {
        return false;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(lock_path) else {
        return false;
    };
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
        false
    } else {
        true
    }
}

pub fn is_gui_instance_running() -> bool {
    is_gui_instance_running_at(&gui_lock_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn launch_agent_write_sync_and_remove_in_temp_dir() {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("LaunchAgents");

        assert!(!is_launch_at_login_enabled_in_dir(&agents_dir));

        let fake_exe = dir.path().join("FLIP & Tools.app/Contents/MacOS/flip");
        fs::create_dir_all(fake_exe.parent().unwrap()).unwrap();
        fs::write(&fake_exe, b"binary").unwrap();

        let enabled =
            set_launch_at_login_in_dir(&agents_dir, true, Some(&fake_exe), false).unwrap();
        assert!(enabled);
        assert!(is_launch_at_login_enabled_in_dir(&agents_dir));

        let plist_path = launch_agent_plist_path(&agents_dir);
        assert!(plist_path.is_file());
        let raw = fs::read_to_string(&plist_path).unwrap();
        assert!(raw.contains(LAUNCH_AGENT_LABEL));
        assert!(raw.contains("FLIP &amp; Tools.app/Contents/MacOS/flip"));
        assert!(raw.contains("<key>RunAtLoad</key>"));
        assert!(raw.contains("<true/>"));

        if Path::new("/usr/bin/plutil").is_file() {
            let status = Command::new("/usr/bin/plutil")
                .arg("-lint")
                .arg(&plist_path)
                .status()
                .unwrap();
            assert!(status.success(), "LaunchAgent plist failed plutil -lint");
        }

        let mut prefs = AppPreferences::default();
        assert!(!prefs.launch_at_login);
        sync_preferences_with_launch_agent_in_dir(&mut prefs, &agents_dir);
        assert!(prefs.launch_at_login);

        let disabled = set_launch_at_login_in_dir(&agents_dir, false, None, false).unwrap();
        assert!(!disabled);
        assert!(!plist_path.exists());
        assert!(!is_launch_at_login_enabled_in_dir(&agents_dir));

        sync_preferences_with_launch_agent_in_dir(&mut prefs, &agents_dir);
        assert!(!prefs.launch_at_login);

        // Idempotent disable when plist already absent must succeed cleanly
        assert!(!set_launch_at_login_in_dir(&agents_dir, false, None, false).unwrap());

        // Empty/foreign plist is treated as disabled
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(&plist_path, b"").unwrap();
        assert!(!is_launch_at_login_enabled_in_dir(&agents_dir));
    }

    #[test]
    fn resolve_launch_executable_prefers_app_bundle_or_applications_install() {
        let dir = tempdir().unwrap();
        let app_exe = dir.path().join("Custom/FLIP.app/Contents/MacOS/flip");
        let system_app_exe = dir.path().join("Applications/FLIP.app/Contents/MacOS/flip");
        let dev_exe = dir.path().join("target/debug/flip");

        fs::create_dir_all(app_exe.parent().unwrap()).unwrap();
        fs::create_dir_all(system_app_exe.parent().unwrap()).unwrap();
        fs::create_dir_all(dev_exe.parent().unwrap()).unwrap();
        fs::write(&app_exe, b"1").unwrap();
        fs::write(&system_app_exe, b"2").unwrap();
        fs::write(&dev_exe, b"3").unwrap();

        assert_eq!(
            resolve_launch_executable(Some(&app_exe), &system_app_exe),
            app_exe
        );
        assert_eq!(
            resolve_launch_executable(Some(&dev_exe), &system_app_exe),
            system_app_exe
        );

        let missing_system = dir.path().join("Missing/FLIP.app/Contents/MacOS/flip");
        assert_eq!(
            resolve_launch_executable(Some(&dev_exe), &missing_system),
            dev_exe
        );
        assert_eq!(
            resolve_launch_executable(None, &missing_system),
            missing_system
        );
    }

    #[test]
    fn gui_instance_flock_prevents_duplicate_and_releases_on_drop() {
        let dir = tempdir().unwrap();
        let lock_file = dir.path().join("gui.lock");
        assert!(!is_gui_instance_running_at(&lock_file));

        let guard = try_acquire_gui_lock_at(&lock_file).expect("first lock must succeed");
        assert!(is_gui_instance_running_at(&lock_file));
        assert!(try_acquire_gui_lock_at(&lock_file).is_none());

        drop(guard);
        assert!(!is_gui_instance_running_at(&lock_file));
        assert!(try_acquire_gui_lock_at(&lock_file).is_some());
    }
}
