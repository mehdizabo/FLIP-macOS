use crate::core::ipc_socket::{self, IpcCommand, IpcResponse, IpcSocketError};
use crate::core::policy::{batch_actions, ext_lower, family_for_ext, order_formats, outputs};
use crate::core::preferences::{expand_tilde_path, AppPreferences};
use crate::core::quick_actions::{
    parse_flip_url, resolve_app_bundle_path, resolve_cli_executable, FlipUrlAction,
};
use crate::core::startup::LOGIN_ITEM_ARG;
use crate::engines::create_new::{built_in_kinds, create_new_item};
use crate::engines::document::xml_escape;
use crate::engines::{convert_files, detect_tools, ConversionResult};
use crate::host::commands::ConversionCompletePayload;
use serde_json::json;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Gui {
        initial_url: Option<String>,
    },
    Help,
    Version,
    Convert {
        files: Vec<PathBuf>,
        formats: Vec<String>,
    },
    New {
        kind: String,
        folder: Option<PathBuf>,
        no_rename: bool,
        no_reveal: bool,
    },
    Wheel {
        mode: Option<String>,
        files: Vec<PathBuf>,
    },
    Settings,
    Batch {
        files: Vec<PathBuf>,
    },
    Preview {
        out: PathBuf,
        files: Vec<PathBuf>,
        mode: Option<String>,
        hover: Option<usize>,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn has_login_item_flag<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter()
        .skip(1)
        .any(|a| a.into().to_string_lossy() == LOGIN_ITEM_ARG)
}

pub fn help_text() -> String {
    format!(
        "FLIP {} — Local macOS file converter & Create New File utility\n\n\
USAGE:\n  \
  flip                                         Launch menu bar app\n  \
  flip convert <file...> --to <FMT[,FMT2]>     Convert file(s) locally and print JSON\n  \
  flip new <kind> [--in <folder>] [--no-rename]\n                                               Create a new file/folder and print JSON\n  \
  flip wheel [--mode convert|new] [files...]   Open the radial wheel (launches FLIP if needed)\n  \
  flip settings                                Open FLIP Settings window\n  \
  flip batch [files...]                        Open Batch & Multi-Format Converter window\n  \
  flip preview --out <path.svg> [--files f1,f2] [--mode convert|new] [--hover N]\n                                               Render standalone radial wheel SVG preview\n  \
  flip quit                                    Quit running FLIP menu bar instance\n  \
  flip --help                                  Show this help message\n  \
  flip --version                               Show version information\n\n\
EXAMPLES:\n  \
  flip convert photo.heic --to PNG,JPG\n  \
  flip new md --in ~/Desktop\n  \
  flip new folder --in ~/Documents --no-rename\n  \
  flip wheel --mode convert photo.heic\n  \
  flip preview --out wheel.svg --files photo.heic --hover 0\n  \
  flip batch *.png\n  \
  flip settings\n  \
  flip quit\n",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn version_text() -> String {
    format!("flip {}", env!("CARGO_PKG_VERSION"))
}

fn push_formats_from_str(raw: &str, out: &mut Vec<String>) {
    for part in raw.split(',') {
        let up = part.trim().trim_start_matches('.').to_ascii_uppercase();
        if !up.is_empty() && !out.contains(&up) {
            out.push(up);
        }
    }
}

pub fn parse_cli_args<I, S>(args: I) -> Result<CliCommand, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let filtered: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|s| s.into().to_string_lossy().into_owned())
        .filter(|s| !s.starts_with("-psn_") && s != LOGIN_ITEM_ARG)
        .collect();

    if filtered.is_empty() {
        return Ok(CliCommand::Gui { initial_url: None });
    }

    let first = filtered[0].as_str();
    match first {
        "--help" | "-h" | "help" => Ok(CliCommand::Help),
        "--version" | "-V" | "-v" | "version" => Ok(CliCommand::Version),
        "convert" => parse_convert_args(&filtered[1..]),
        "new" => parse_new_args(&filtered[1..]),
        "wheel" => parse_wheel_args(&filtered[1..]),
        "settings" => parse_settings_args(&filtered[1..]),
        "batch" => parse_batch_args(&filtered[1..]),
        "preview" => parse_preview_args(&filtered[1..]),
        "quit" => parse_quit_args(&filtered[1..]),
        arg if arg.starts_with("flip://") || arg.starts_with("FLIP://") => {
            match parse_flip_url(arg)? {
                FlipUrlAction::Convert { files, formats } => Ok(CliCommand::Convert {
                    files: files.into_iter().map(PathBuf::from).collect(),
                    formats,
                }),
                FlipUrlAction::CreateNew {
                    kind: Some(kind),
                    folder,
                    no_rename,
                } => Ok(CliCommand::New {
                    kind,
                    folder: folder.map(PathBuf::from),
                    no_rename,
                    no_reveal: false,
                }),
                _ => Ok(CliCommand::Gui {
                    initial_url: Some(arg.to_string()),
                }),
            }
        }
        other => Err(format!(
            "Unknown command `{other}`. Run `flip --help` for usage."
        )),
    }
}

fn parse_convert_args(args: &[String]) -> Result<CliCommand, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut formats: Vec<String> = Vec::new();
    let mut i = 0;
    let mut positional_only = false;

    while i < args.len() {
        let arg = &args[i];
        if !positional_only && arg == "--" {
            positional_only = true;
            i += 1;
            continue;
        }
        if !positional_only && (arg == "--help" || arg == "-h") {
            return Ok(CliCommand::Help);
        }
        if !positional_only && (arg == "--to" || arg == "-t") {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("Missing value after `{arg}`"))?;
            push_formats_from_str(val, &mut formats);
            i += 2;
            continue;
        }
        if !positional_only {
            if let Some(val) = arg.strip_prefix("--to=") {
                push_formats_from_str(val, &mut formats);
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                return Err(format!("Unknown option for `convert`: `{arg}`"));
            }
        }
        files.push(PathBuf::from(arg));
        i += 1;
    }

    if files.is_empty() {
        return Err("`flip convert` requires at least one input file.".to_string());
    }
    if formats.is_empty() {
        return Err("`flip convert` requires `--to <FORMAT[,FORMAT2]>`.".to_string());
    }

    Ok(CliCommand::Convert { files, formats })
}

fn parse_new_args(args: &[String]) -> Result<CliCommand, String> {
    let mut kind: Option<String> = None;
    let mut folder: Option<PathBuf> = None;
    let mut no_rename = false;
    let mut no_reveal = false;
    let mut i = 0;
    let mut positional_only = false;

    while i < args.len() {
        let arg = &args[i];
        if !positional_only && arg == "--" {
            positional_only = true;
            i += 1;
            continue;
        }
        if !positional_only && (arg == "--help" || arg == "-h") {
            return Ok(CliCommand::Help);
        }
        if !positional_only && arg == "--in" {
            let val = args
                .get(i + 1)
                .ok_or_else(|| "Missing folder path after `--in`".to_string())?;
            if val.trim().is_empty() {
                return Err("Missing folder path after `--in`".to_string());
            }
            folder = Some(PathBuf::from(val));
            i += 2;
            continue;
        }
        if !positional_only {
            if let Some(val) = arg.strip_prefix("--in=") {
                if val.trim().is_empty() {
                    return Err("Missing folder path after `--in=`".to_string());
                }
                folder = Some(PathBuf::from(val));
                i += 1;
                continue;
            }
            if arg == "--no-rename" {
                no_rename = true;
                i += 1;
                continue;
            }
            if arg == "--no-reveal" {
                no_reveal = true;
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                return Err(format!("Unknown option for `new`: `{arg}`"));
            }
        }

        if kind.is_none() {
            kind = Some(arg.clone());
        } else {
            return Err(format!("Unexpected positional argument for `new`: `{arg}`"));
        }
        i += 1;
    }

    let raw_kind = kind.ok_or_else(|| {
        "`flip new` requires a <kind> argument (e.g. folder, txt, md, rtf, docx, xlsx, pptx, html, json, sh, clipboard).".to_string()
    })?;
    let normalized_kind = if raw_kind.starts_with("template:") {
        raw_kind
    } else {
        raw_kind.trim().to_ascii_lowercase()
    };

    Ok(CliCommand::New {
        kind: normalized_kind,
        folder,
        no_rename,
        no_reveal,
    })
}

fn normalize_wheel_mode(raw: &str) -> Result<String, String> {
    let m = raw.trim().to_ascii_lowercase();
    match m.as_str() {
        "convert" | "new" => Ok(m),
        _ => Err(format!(
            "Invalid `--mode` value `{raw}` for `wheel` (expected `convert` or `new`)."
        )),
    }
}

fn parse_wheel_args(args: &[String]) -> Result<CliCommand, String> {
    let mut mode: Option<String> = None;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    let mut positional_only = false;

    while i < args.len() {
        let arg = &args[i];
        if !positional_only && arg == "--" {
            positional_only = true;
            i += 1;
            continue;
        }
        if !positional_only && (arg == "--help" || arg == "-h") {
            return Ok(CliCommand::Help);
        }
        if !positional_only && (arg == "--mode" || arg == "-m") {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("Missing value after `{arg}` (expected `convert` or `new`)."))?;
            mode = Some(normalize_wheel_mode(val)?);
            i += 2;
            continue;
        }
        if !positional_only {
            if let Some(val) = arg.strip_prefix("--mode=") {
                mode = Some(normalize_wheel_mode(val)?);
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                return Err(format!("Unknown option for `wheel`: `{arg}`"));
            }
        }
        files.push(PathBuf::from(arg));
        i += 1;
    }

    Ok(CliCommand::Wheel { mode, files })
}

fn parse_settings_args(args: &[String]) -> Result<CliCommand, String> {
    if let Some(arg) = args.first() {
        if arg == "--help" || arg == "-h" {
            return Ok(CliCommand::Help);
        }
        return Err(format!("Unexpected argument for `settings`: `{arg}`"));
    }
    Ok(CliCommand::Settings)
}

fn parse_batch_args(args: &[String]) -> Result<CliCommand, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut positional_only = false;
    for arg in args {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && (arg == "--help" || arg == "-h") {
            return Ok(CliCommand::Help);
        }
        if !positional_only && arg.starts_with('-') && arg != "-" {
            return Err(format!("Unknown option for `batch`: `{arg}`"));
        }
        files.push(PathBuf::from(arg));
    }
    Ok(CliCommand::Batch { files })
}

fn parse_quit_args(args: &[String]) -> Result<CliCommand, String> {
    if let Some(arg) = args.first() {
        if arg == "--help" || arg == "-h" {
            return Ok(CliCommand::Help);
        }
        return Err(format!("Unexpected argument for `quit`: `{arg}`"));
    }
    Ok(CliCommand::Quit)
}

fn push_preview_files_arg(raw: &str, out: &mut Vec<PathBuf>) {
    for part in raw.split(',') {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            out.push(PathBuf::from(trimmed));
        }
    }
}

fn parse_preview_args(args: &[String]) -> Result<CliCommand, String> {
    let mut out: Option<PathBuf> = None;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut mode: Option<String> = None;
    let mut hover: Option<usize> = None;
    let mut positional_only = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if !positional_only && arg == "--" {
            positional_only = true;
            i += 1;
            continue;
        }
        if !positional_only && (arg == "--help" || arg == "-h") {
            return Ok(CliCommand::Help);
        }
        if !positional_only && (arg == "--out" || arg == "-o") {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("Missing SVG output path after `{arg}`."))?;
            if val.trim().is_empty() {
                return Err("Missing SVG output path after `--out`.".to_string());
            }
            out = Some(PathBuf::from(val));
            i += 2;
            continue;
        }
        if !positional_only && arg == "--files" {
            let val = args
                .get(i + 1)
                .ok_or_else(|| "Missing file list after `--files`.".to_string())?;
            push_preview_files_arg(val, &mut files);
            i += 2;
            continue;
        }
        if !positional_only && (arg == "--mode" || arg == "-m") {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("Missing value after `{arg}` (expected `convert` or `new`)."))?;
            mode = Some(normalize_wheel_mode(val)?);
            i += 2;
            continue;
        }
        if !positional_only && arg == "--hover" {
            let val = args
                .get(i + 1)
                .ok_or_else(|| "Missing index after `--hover`.".to_string())?;
            let idx = val
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("Invalid `--hover` index `{val}` (expected non-negative integer)."))?;
            hover = Some(idx);
            i += 2;
            continue;
        }
        if !positional_only {
            if let Some(val) = arg.strip_prefix("--out=") {
                if val.trim().is_empty() {
                    return Err("Missing SVG output path after `--out=`.".to_string());
                }
                out = Some(PathBuf::from(val));
                i += 1;
                continue;
            }
            if let Some(val) = arg.strip_prefix("--files=") {
                push_preview_files_arg(val, &mut files);
                i += 1;
                continue;
            }
            if let Some(val) = arg.strip_prefix("--mode=") {
                mode = Some(normalize_wheel_mode(val)?);
                i += 1;
                continue;
            }
            if let Some(val) = arg.strip_prefix("--hover=") {
                let idx = val
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid `--hover=` index `{val}` (expected non-negative integer)."))?;
                hover = Some(idx);
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                return Err(format!("Unknown option for `preview`: `{arg}`"));
            }
        }
        files.push(PathBuf::from(arg));
        i += 1;
    }

    let out_path = out.ok_or_else(|| "`flip preview` requires `--out <path.svg>`.".to_string())?;
    Ok(CliCommand::Preview {
        out: out_path,
        files,
        mode,
        hover,
    })
}

pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {
                if parts.is_empty() {
                    parts.push(comp);
                }
            }
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = parts.last() {
                    parts.pop();
                } else if !matches!(parts.last(), Some(Component::RootDir | Component::Prefix(_))) {
                    parts.push(comp);
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        PathBuf::from(".")
    } else {
        parts.iter().collect()
    }
}

pub fn resolve_cli_path(raw: &Path, cwd: Option<&Path>) -> PathBuf {
    let raw_str = raw.to_string_lossy();
    if raw_str.trim().is_empty() {
        return raw.to_path_buf();
    }
    let expanded = if raw_str == "~" || raw_str.starts_with("~/") {
        expand_tilde_path(&raw_str)
    } else {
        raw.to_path_buf()
    };
    let abs = if expanded.is_absolute() {
        expanded
    } else if let Some(base) = cwd {
        base.join(expanded)
    } else if let Ok(cur) = std::env::current_dir() {
        cur.join(expanded)
    } else {
        expanded
    };
    lexical_normalize(&abs)
}

pub fn launch_flip_host_app() -> Result<(), String> {
    let current_exe = std::env::current_exe().ok();
    if let Some(bundle) = resolve_app_bundle_path(current_exe.as_deref()) {
        let mut last_err = String::new();
        for attempt in 0..15 {
            let output = Command::new("/usr/bin/open")
                .arg("-g")
                .arg("-a")
                .arg(&bundle)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .output()
                .map_err(|e| format!("Failed to launch `{}`: {e}", bundle.display()))?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            last_err = if stderr.is_empty() {
                format!(
                    "`open -a {}` exited with status {:?}",
                    bundle.display(),
                    output.status.code()
                )
            } else {
                format!(
                    "`open -a {}` exited with status {:?}: {stderr}",
                    bundle.display(),
                    output.status.code()
                )
            };
            // macOS LaunchServices returns -609 / -600 while an exiting instance finishes AppKit teardown.
            if attempt < 14
                && (stderr.contains("-609")
                    || stderr.contains("-600")
                    || crate::core::startup::is_gui_instance_running())
            {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            break;
        }
        return Err(last_err);
    }

    if let Some(exe) = resolve_cli_executable(current_exe.as_deref()).or(current_exe) {
        Command::new(&exe)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to launch `{}`: {e}", exe.display()))?;
        return Ok(());
    }

    Err("Could not locate FLIP.app or executable to launch.".to_string())
}

fn ipc_command_name(cmd: &IpcCommand) -> &'static str {
    match cmd {
        IpcCommand::Ping => "ping",
        IpcCommand::Activate => "activate",
        IpcCommand::OpenUrl { .. } => "open_url",
        IpcCommand::OpenWheel { .. } => "open_wheel",
        IpcCommand::OpenSettings => "open_settings",
        IpcCommand::OpenBatch { .. } => "open_batch",
        IpcCommand::Quit => "quit",
    }
}

fn format_ipc_outcome(cmd_name: &str, resp: IpcResponse, launched: bool) -> CliOutcome {
    if resp.ok {
        let stdout = serde_json::to_string_pretty(&json!({
            "success": true,
            "command": cmd_name,
            "launched": launched,
            "message": resp.message,
            "pid": resp.pid,
        }))
        .unwrap_or_else(|_| "{\"success\":true}".to_string());
        CliOutcome {
            exit_code: 0,
            stdout: format!("{stdout}\n"),
            stderr: String::new(),
        }
    } else {
        let msg = resp
            .error
            .unwrap_or_else(|| format!("Host rejected `{cmd_name}` command."));
        let stdout = serde_json::to_string_pretty(&json!({
            "success": false,
            "command": cmd_name,
            "launched": launched,
            "error": msg,
        }))
        .unwrap_or_else(|_| "{\"success\":false}".to_string());
        CliOutcome {
            exit_code: 1,
            stdout: format!("{stdout}\n"),
            stderr: format!("{msg}\n"),
        }
    }
}

pub fn execute_control_command_with<L>(
    ipc_cmd: IpcCommand,
    socket_path: &Path,
    launch_if_offline: bool,
    wait_timeout: Duration,
    mut launcher: L,
) -> CliOutcome
where
    L: FnMut() -> Result<(), String>,
{
    let cmd_name = ipc_command_name(&ipc_cmd);

    let initial_send = match ipc_socket::send_command(socket_path, &ipc_cmd) {
        Ok(resp) if launch_if_offline && resp.error.as_deref() == Some("FLIP is shutting down") => {
            let mut settled = Err(IpcSocketError::NotRunning(socket_path.display().to_string()));
            for _ in 0..40 {
                std::thread::sleep(Duration::from_millis(50));
                match ipc_socket::send_command(socket_path, &ipc_cmd) {
                    Ok(r) if r.error.as_deref() == Some("FLIP is shutting down") => continue,
                    other => {
                        settled = other;
                        break;
                    }
                }
            }
            settled
        }
        other => other,
    };

    match initial_send {
        Ok(resp) => format_ipc_outcome(cmd_name, resp, false),
        Err(IpcSocketError::NotRunning(_)) if !launch_if_offline => {
            let msg = "FLIP is not running.".to_string();
            let stdout = serde_json::to_string_pretty(&json!({
                "success": false,
                "command": cmd_name,
                "running": false,
                "error": msg,
            }))
            .unwrap_or_else(|_| "{\"success\":false}".to_string());
            CliOutcome {
                exit_code: 1,
                stdout: format!("{stdout}\n"),
                stderr: format!("{msg}\n"),
            }
        }
        Err(IpcSocketError::NotRunning(_)) => {
            if let Err(launch_err) = launcher() {
                let stdout = serde_json::to_string_pretty(&json!({
                    "success": false,
                    "command": cmd_name,
                    "launched": false,
                    "error": launch_err,
                }))
                .unwrap_or_else(|_| "{\"success\":false}".to_string());
                return CliOutcome {
                    exit_code: 1,
                    stdout: format!("{stdout}\n"),
                    stderr: format!("{launch_err}\n"),
                };
            }

            let deadline = Instant::now() + wait_timeout;
            loop {
                match ipc_socket::send_command(socket_path, &ipc_cmd) {
                    Ok(resp) if resp.error.as_deref() == Some("FLIP is shutting down")
                        && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Ok(resp) => return format_ipc_outcome(cmd_name, resp, true),
                    Err(IpcSocketError::NotRunning(_)) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => {
                        let msg = format!(
                            "Launched FLIP, but IPC socket did not respond: {err}"
                        );
                        let stdout = serde_json::to_string_pretty(&json!({
                            "success": false,
                            "command": cmd_name,
                            "launched": true,
                            "error": msg,
                        }))
                        .unwrap_or_else(|_| "{\"success\":false}".to_string());
                        return CliOutcome {
                            exit_code: 1,
                            stdout: format!("{stdout}\n"),
                            stderr: format!("{msg}\n"),
                        };
                    }
                }
            }
        }
        Err(err) => {
            let msg = err.to_string();
            let stdout = serde_json::to_string_pretty(&json!({
                "success": false,
                "command": cmd_name,
                "error": msg,
            }))
            .unwrap_or_else(|_| "{\"success\":false}".to_string());
            CliOutcome {
                exit_code: 1,
                stdout: format!("{stdout}\n"),
                stderr: format!("{msg}\n"),
            }
        }
    }
}

pub fn execute_cli_command(cmd: CliCommand, prefs: &AppPreferences) -> CliOutcome {
    execute_cli_command_in(cmd, prefs, None)
}

pub fn execute_cli_command_in(
    cmd: CliCommand,
    prefs: &AppPreferences,
    cwd: Option<&Path>,
) -> CliOutcome {
    let default_sock = ipc_socket::default_socket_path();
    execute_cli_command_with_ipc(
        cmd,
        prefs,
        cwd,
        &default_sock,
        Duration::from_secs(8),
        launch_flip_host_app,
    )
}

pub fn execute_cli_command_with_ipc<L>(
    cmd: CliCommand,
    prefs: &AppPreferences,
    cwd: Option<&Path>,
    socket_path: &Path,
    wait_timeout: Duration,
    launcher: L,
) -> CliOutcome
where
    L: FnMut() -> Result<(), String>,
{
    match cmd {
        CliCommand::Help => CliOutcome {
            exit_code: 0,
            stdout: help_text(),
            stderr: String::new(),
        },
        CliCommand::Version => CliOutcome {
            exit_code: 0,
            stdout: format!("{}\n", version_text()),
            stderr: String::new(),
        },
        CliCommand::Convert { files, formats } => {
            let resolved_files: Vec<PathBuf> =
                files.iter().map(|p| resolve_cli_path(p, cwd)).collect();
            let cancel = Arc::new(AtomicBool::new(false));
            match convert_files(&resolved_files, &formats, prefs, &cancel, None) {
                Ok(results) => {
                    let first_err = results.iter().find_map(|r| r.error.clone());
                    let success = !results.is_empty() && first_err.is_none();
                    let payload = ConversionCompletePayload {
                        success,
                        results,
                        error: first_err.clone(),
                    };
                    let stdout = serde_json::to_string_pretty(&payload)
                        .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    CliOutcome {
                        exit_code: if success { 0 } else { 1 },
                        stdout: format!("{stdout}\n"),
                        stderr: first_err.map(|e| format!("{e}\n")).unwrap_or_default(),
                    }
                }
                Err(err) => {
                    let msg = err.to_string();
                    let payload = ConversionCompletePayload {
                        success: false,
                        results: Vec::<ConversionResult>::new(),
                        error: Some(msg.clone()),
                    };
                    let stdout = serde_json::to_string_pretty(&payload)
                        .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    CliOutcome {
                        exit_code: 1,
                        stdout: format!("{stdout}\n"),
                        stderr: format!("{msg}\n"),
                    }
                }
            }
        }
        CliCommand::New {
            kind,
            folder,
            no_rename,
            no_reveal,
        } => {
            let resolved_folder = folder.as_ref().map(|p| resolve_cli_path(p, cwd));
            if let Some(ref explicit) = resolved_folder {
                if !explicit.is_dir() {
                    let msg = format!(
                        "Target folder does not exist or is not a directory: `{}`",
                        explicit.display()
                    );
                    let err_json = serde_json::to_string_pretty(&json!({
                        "success": false,
                        "error": msg,
                    }))
                    .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    return CliOutcome {
                        exit_code: 1,
                        stdout: format!("{err_json}\n"),
                        stderr: format!("{msg}\n"),
                    };
                }
            }

            let resolved_kind = if let Some(tmpl) = kind.strip_prefix("template:") {
                format!(
                    "template:{}",
                    resolve_cli_path(Path::new(tmpl), cwd).to_string_lossy()
                )
            } else {
                kind
            };

            let mut eff_prefs = prefs.clone();
            if no_rename {
                eff_prefs.auto_rename_on_create = false;
            }

            match create_new_item(
                &resolved_kind,
                resolved_folder.as_deref(),
                &eff_prefs,
                !no_reveal,
            ) {
                Ok(res) => {
                    let stdout = serde_json::to_string_pretty(&res)
                        .unwrap_or_else(|_| "{}".to_string());
                    CliOutcome {
                        exit_code: 0,
                        stdout: format!("{stdout}\n"),
                        stderr: String::new(),
                    }
                }
                Err(err) => {
                    let msg = err.to_string();
                    let err_json = serde_json::to_string_pretty(&json!({
                        "success": false,
                        "error": msg,
                    }))
                    .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    CliOutcome {
                        exit_code: 1,
                        stdout: format!("{err_json}\n"),
                        stderr: format!("{msg}\n"),
                    }
                }
            }
        }
        CliCommand::Wheel { mode, files } => {
            let mut resolved_files: Vec<String> = files
                .iter()
                .map(|p| resolve_cli_path(p, cwd).to_string_lossy().into_owned())
                .collect();
            let folder = if mode.as_deref() == Some("new") && !resolved_files.is_empty() {
                let first = PathBuf::from(&resolved_files[0]);
                if first.is_dir() {
                    resolved_files.clear();
                    Some(first.to_string_lossy().into_owned())
                } else if first.is_file() {
                    resolved_files.clear();
                    first
                        .parent()
                        .filter(|p| p.is_dir())
                        .map(|p| p.to_string_lossy().into_owned())
                } else {
                    let msg = format!(
                        "Target path does not exist: `{}`",
                        first.display()
                    );
                    let err_json = serde_json::to_string_pretty(&json!({
                        "success": false,
                        "command": "open_wheel",
                        "error": msg,
                    }))
                    .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    return CliOutcome {
                        exit_code: 1,
                        stdout: format!("{err_json}\n"),
                        stderr: format!("{msg}\n"),
                    };
                }
            } else {
                None
            };
            execute_control_command_with(
                IpcCommand::OpenWheel {
                    mode,
                    files: resolved_files,
                    folder,
                },
                socket_path,
                true,
                wait_timeout,
                launcher,
            )
        }
        CliCommand::Settings => execute_control_command_with(
            IpcCommand::OpenSettings,
            socket_path,
            true,
            wait_timeout,
            launcher,
        ),
        CliCommand::Batch { files } => {
            let resolved_files: Vec<String> = files
                .iter()
                .map(|p| resolve_cli_path(p, cwd).to_string_lossy().into_owned())
                .collect();
            execute_control_command_with(
                IpcCommand::OpenBatch {
                    files: resolved_files,
                },
                socket_path,
                true,
                wait_timeout,
                launcher,
            )
        }
        CliCommand::Preview {
            out,
            files,
            mode,
            hover,
        } => execute_preview_command(out, files, mode, hover, prefs, cwd),
        CliCommand::Quit => execute_control_command_with(
            IpcCommand::Quit,
            socket_path,
            false,
            wait_timeout,
            launcher,
        ),
        CliCommand::Gui { .. } => CliOutcome {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        },
    }
}

fn preview_format_short_label(fmt: &str) -> &str {
    match fmt {
        "REMOVE_BG" => "No BG",
        "OPTIMIZE_PDF" => "Optimize",
        "COMBINE_PDF" => "Combine",
        "MERGE_PDF" => "Merge PDF",
        "UNZIP" => "Extract",
        other => other,
    }
}

fn preview_new_kind_short_label(id: &str, name: &str) -> String {
    match id {
        "folder" => "Folder".to_string(),
        "txt" => "TXT".to_string(),
        "md" => "MD".to_string(),
        "rtf" => "RTF".to_string(),
        "docx" => "DOCX".to_string(),
        "xlsx" => "XLSX".to_string(),
        "html" => "HTML".to_string(),
        "clipboard" => "Clipboard".to_string(),
        "json" => "JSON".to_string(),
        "csv" => "CSV".to_string(),
        "sh" => "SH".to_string(),
        "py" => "PY".to_string(),
        "swift" => "Swift".to_string(),
        "pptx" => "PPTX".to_string(),
        "pages" => "Pages".to_string(),
        "numbers" => "Numbers".to_string(),
        "keynote" => "Keynote".to_string(),
        _ => name.to_string(),
    }
}

fn polar_to_cartesian(cx: f64, cy: f64, r: f64, angle_deg: f64) -> (f64, f64) {
    let rad = (angle_deg - 90.0).to_radians();
    (cx + r * rad.cos(), cy + r * rad.sin())
}

fn fmt_svg_num(n: f64) -> String {
    let rounded = (n * 1000.0).round() / 1000.0;
    let normalized = if rounded == -0.0 { 0.0 } else { rounded };
    let mut s = format!("{normalized:.3}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

fn build_preview_sector_path(index: usize, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let cx = 220.0_f64;
    let cy = 220.0_f64;
    let r_in = 78.0_f64;
    let r_out = 170.0_f64;
    let corner_inset = 7.0_f64;

    let safe_count = count.max(1);
    let span_deg = 360.0 / (safe_count as f64);
    let gap_deg = if safe_count == 1 {
        1.8
    } else {
        2.2_f64.min(span_deg * 0.06)
    };
    let mid_deg = (index as f64) * span_deg;
    let start_deg = mid_deg - span_deg / 2.0 + gap_deg;
    let end_deg = mid_deg + span_deg / 2.0 - gap_deg;
    let arc_span = (end_deg - start_deg).max(1.0);
    let large_arc = if arc_span > 180.0 { 1 } else { 0 };

    let outer_corner_deg = 2.4_f64.min(arc_span * 0.08);
    let inner_corner_deg = 4.2_f64.min(arc_span * 0.14);

    let p0 = polar_to_cartesian(cx, cy, r_out - corner_inset, start_deg);
    let p1_ctrl = polar_to_cartesian(cx, cy, r_out, start_deg);
    let p1_end = polar_to_cartesian(cx, cy, r_out, start_deg + outer_corner_deg);

    let p2_start = polar_to_cartesian(cx, cy, r_out, end_deg - outer_corner_deg);
    let p3_ctrl = polar_to_cartesian(cx, cy, r_out, end_deg);
    let p3_end = polar_to_cartesian(cx, cy, r_out - corner_inset, end_deg);

    let p4 = polar_to_cartesian(cx, cy, r_in + corner_inset, end_deg);
    let p5_ctrl = polar_to_cartesian(cx, cy, r_in, end_deg);
    let p5_end = polar_to_cartesian(cx, cy, r_in, end_deg - inner_corner_deg);

    let p6_start = polar_to_cartesian(cx, cy, r_in, start_deg + inner_corner_deg);
    let p7_ctrl = polar_to_cartesian(cx, cy, r_in, start_deg);
    let p7_end = polar_to_cartesian(cx, cy, r_in + corner_inset, start_deg);

    format!(
        "M {} {} Q {} {} {} {} A {} {} 0 {large_arc} 1 {} {} Q {} {} {} {} L {} {} Q {} {} {} {} A {} {} 0 {large_arc} 0 {} {} Q {} {} {} {} Z",
        fmt_svg_num(p0.0),
        fmt_svg_num(p0.1),
        fmt_svg_num(p1_ctrl.0),
        fmt_svg_num(p1_ctrl.1),
        fmt_svg_num(p1_end.0),
        fmt_svg_num(p1_end.1),
        fmt_svg_num(r_out),
        fmt_svg_num(r_out),
        fmt_svg_num(p2_start.0),
        fmt_svg_num(p2_start.1),
        fmt_svg_num(p3_ctrl.0),
        fmt_svg_num(p3_ctrl.1),
        fmt_svg_num(p3_end.0),
        fmt_svg_num(p3_end.1),
        fmt_svg_num(p4.0),
        fmt_svg_num(p4.1),
        fmt_svg_num(p5_ctrl.0),
        fmt_svg_num(p5_ctrl.1),
        fmt_svg_num(p5_end.0),
        fmt_svg_num(p5_end.1),
        fmt_svg_num(r_in),
        fmt_svg_num(r_in),
        fmt_svg_num(p6_start.0),
        fmt_svg_num(p6_start.1),
        fmt_svg_num(p7_ctrl.0),
        fmt_svg_num(p7_ctrl.1),
        fmt_svg_num(p7_end.0),
        fmt_svg_num(p7_end.1),
    )
}

#[derive(Debug, Clone)]
struct PreviewSlot {
    label: String,
    subtitle: String,
    is_nav: bool,
    enabled: bool,
}

fn compute_preview_wheel(
    files: &[PathBuf],
    mode: &str,
    prefs: &AppPreferences,
    cwd: Option<&Path>,
) -> (Vec<PreviewSlot>, String, String) {
    if mode == "new" {
        let mut kinds = built_in_kinds(false);
        let order_rank: std::collections::HashMap<&str, usize> = prefs
            .new_file_visible_kinds
            .iter()
            .enumerate()
            .map(|(idx, id)| (id.as_str(), idx))
            .collect();
        kinds.sort_by_key(|k| {
            order_rank
                .get(k.id.as_str())
                .copied()
                .unwrap_or(100 + (k.page as usize))
        });

        let mut slots: Vec<PreviewSlot> = kinds
            .into_iter()
            .map(|k| PreviewSlot {
                label: preview_new_kind_short_label(&k.id, &k.name),
                subtitle: format!("New {}", k.name),
                is_nav: false,
                enabled: k.available,
            })
            .collect();
        if slots.len() > 8 {
            slots.truncate(7);
            slots.push(PreviewSlot {
                label: "More ▸".to_string(),
                subtitle: "Next page".to_string(),
                is_nav: true,
                enabled: true,
            });
        }
        return (
            slots,
            "New File".to_string(),
            "Choose item to create".to_string(),
        );
    }

    if files.is_empty() {
        let placeholders = ["DOCX", "PDF", "MD", "HTML", "PNG", "JPG", "WEBP", "MP3"];
        let slots = placeholders
            .into_iter()
            .map(|lbl| PreviewSlot {
                label: lbl.to_string(),
                subtitle: format!("Convert to {lbl}"),
                is_nav: false,
                enabled: true,
            })
            .collect();
        return (
            slots,
            "FLIP Preview".to_string(),
            "Drop local files".to_string(),
        );
    }

    let resolved: Vec<PathBuf> = files.iter().map(|p| resolve_cli_path(p, cwd)).collect();
    let tools = detect_tools(Some(prefs));
    let caps = tools.capabilities();

    let compat = outputs(&resolved, &caps);
    let actions = batch_actions(&resolved);
    let primary_ext = ext_lower(&resolved[0]);
    let family = family_for_ext(&primary_ext);
    let ordered_compat = order_formats(&compat, family, Some(prefs));

    let mut combined = actions;
    for fmt in ordered_compat {
        if !combined.contains(&fmt) {
            combined.push(fmt);
        }
    }

    let hub_title = if resolved.len() == 1 {
        resolved[0]
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("File")
            .to_string()
    } else {
        format!("{} files", resolved.len())
    };

    if combined.is_empty() {
        let slots = (0..8)
            .map(|_| PreviewSlot {
                label: "—".to_string(),
                subtitle: "No common output format".to_string(),
                is_nav: false,
                enabled: false,
            })
            .collect();
        return (slots, hub_title, "No common output format".to_string());
    }

    let mut slots: Vec<PreviewSlot> = combined
        .into_iter()
        .map(|fmt| PreviewSlot {
            label: preview_format_short_label(&fmt).to_string(),
            subtitle: format!("→ {fmt}"),
            is_nav: false,
            enabled: true,
        })
        .collect();
    if slots.len() > 8 {
        slots.truncate(7);
        slots.push(PreviewSlot {
            label: "More ▸".to_string(),
            subtitle: "Next formats".to_string(),
            is_nav: true,
            enabled: true,
        });
    }

    let hub_subtitle = format!("{} options", slots.len());
    (slots, hub_title, hub_subtitle)
}

pub fn render_wheel_preview_svg(
    files: &[PathBuf],
    mode: &str,
    hover: Option<usize>,
    prefs: &AppPreferences,
    cwd: Option<&Path>,
) -> (String, Vec<String>) {
    let (slots, hub_title, default_subtitle) = compute_preview_wheel(files, mode, prefs, cwd);
    let count = slots.len();
    let active_idx = hover.filter(|&idx| idx < count && slots[idx].enabled);
    let hub_subtitle = active_idx
        .map(|idx| slots[idx].subtitle.clone())
        .unwrap_or(default_subtitle);

    let mut sectors_svg = String::new();
    let sweep = if count > 0 { 360.0 / (count as f64) } else { 360.0 };

    for (idx, slot) in slots.iter().enumerate() {
        let d = build_preview_sector_path(idx, count);
        let center_angle = (idx as f64) * sweep;
        let label_pos = polar_to_cartesian(220.0, 220.0, 124.0, center_angle);
        let is_active = active_idx == Some(idx);
        let fill = if is_active {
            "url(#flip-sector-active-grad)"
        } else if slot.is_nav {
            "url(#flip-sector-nav-grad)"
        } else {
            "url(#flip-sector-rest-grad)"
        };
        let stroke = if is_active {
            "rgba(125,211,252,0.95)"
        } else {
            "rgba(255,255,255,0.22)"
        };
        let stroke_w = if is_active { "2" } else { "1" };
        let opacity = if slot.enabled { "1" } else { "0.42" };
        let short = xml_escape(&slot.label);
        let slot_num = idx + 1;

        sectors_svg.push_str(&format!(
            "    <g class=\"wheel-sector-group{active_cls}\" data-slot-index=\"{idx}\" data-active=\"{is_active}\" opacity=\"{opacity}\">\n\
             \x20     <path class=\"wheel-sector-path\" d=\"{d}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{stroke_w}\" />\n\
             \x20     <text x=\"{lx:.2}\" y=\"{ly:.2}\" text-anchor=\"middle\" dominant-baseline=\"central\" font-family=\"-apple-system, SF Pro Display, Helvetica, Arial, sans-serif\" font-size=\"13\" font-weight=\"600\" fill=\"#f8fafc\">{short}</text>\n\
             \x20     <text x=\"{lx:.2}\" y=\"{ky:.2}\" text-anchor=\"middle\" dominant-baseline=\"central\" font-family=\"-apple-system, SF Pro Display, Helvetica, Arial, sans-serif\" font-size=\"9.5\" font-weight=\"500\" fill=\"rgba(226,232,240,0.72)\">{slot_num}</text>\n\
             \x20   </g>\n",
            active_cls = if is_active { " is-active" } else { "" },
            lx = label_pos.0,
            ly = label_pos.1 - 6.0,
            ky = label_pos.1 + 11.0,
        ));
    }

    let mode_badge = if mode == "new" { "NEW FILE" } else { "CONVERT" };
    let truncated_title: String = hub_title.chars().take(20).collect();
    let truncated_sub: String = hub_subtitle.chars().take(28).collect();

    let svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 440 440\" width=\"440\" height=\"440\">\n\
        \x20 <defs>\n\
        \x20   <radialGradient id=\"flip-bg-grad\" cx=\"50%\" cy=\"45%\" r=\"60%\">\n\
        \x20     <stop offset=\"0%\" stop-color=\"#1e293b\" />\n\
        \x20     <stop offset=\"100%\" stop-color=\"#0b1120\" />\n\
        \x20   </radialGradient>\n\
        \x20   <linearGradient id=\"flip-sector-rest-grad\" x1=\"0%\" y1=\"0%\" x2=\"100%\" y2=\"100%\">\n\
        \x20     <stop offset=\"0%\" stop-color=\"rgba(255,255,255,0.18)\" />\n\
        \x20     <stop offset=\"50%\" stop-color=\"rgba(148,163,184,0.10)\" />\n\
        \x20     <stop offset=\"100%\" stop-color=\"rgba(15,23,42,0.34)\" />\n\
        \x20   </linearGradient>\n\
        \x20   <linearGradient id=\"flip-sector-nav-grad\" x1=\"0%\" y1=\"0%\" x2=\"100%\" y2=\"100%\">\n\
        \x20     <stop offset=\"0%\" stop-color=\"rgba(56,189,248,0.24)\" />\n\
        \x20     <stop offset=\"100%\" stop-color=\"rgba(99,102,241,0.22)\" />\n\
        \x20   </linearGradient>\n\
        \x20   <linearGradient id=\"flip-sector-active-grad\" x1=\"0%\" y1=\"0%\" x2=\"100%\" y2=\"100%\">\n\
        \x20     <stop offset=\"0%\" stop-color=\"rgba(56,189,248,0.72)\" />\n\
        \x20     <stop offset=\"55%\" stop-color=\"rgba(14,165,233,0.56)\" />\n\
        \x20     <stop offset=\"100%\" stop-color=\"rgba(99,102,241,0.68)\" />\n\
        \x20   </linearGradient>\n\
        \x20   <radialGradient id=\"flip-hub-grad\" cx=\"38%\" cy=\"30%\" r=\"70%\">\n\
        \x20     <stop offset=\"0%\" stop-color=\"rgba(30,41,59,0.92)\" />\n\
        \x20     <stop offset=\"100%\" stop-color=\"rgba(15,23,42,0.98)\" />\n\
        \x20   </radialGradient>\n\
        \x20 </defs>\n\
        \x20 <rect width=\"440\" height=\"440\" rx=\"28\" fill=\"url(#flip-bg-grad)\" />\n\
        \x20 <circle cx=\"220\" cy=\"220\" r=\"178\" fill=\"none\" stroke=\"rgba(255,255,255,0.14)\" stroke-width=\"1.2\" />\n\
        \x20 <circle cx=\"220\" cy=\"220\" r=\"174\" fill=\"none\" stroke=\"rgba(56,189,248,0.22)\" stroke-width=\"1\" />\n\
        \x20 <g id=\"wheel-sectors-layer\">\n\
        {sectors_svg}\
        \x20 </g>\n\
        \x20 <circle cx=\"220\" cy=\"220\" r=\"68\" fill=\"url(#flip-hub-grad)\" stroke=\"rgba(255,255,255,0.24)\" stroke-width=\"1.2\" />\n\
        \x20 <text x=\"220\" y=\"196\" text-anchor=\"middle\" font-family=\"-apple-system, SF Pro Display, Helvetica, Arial, sans-serif\" font-size=\"9.5\" font-weight=\"700\" letter-spacing=\"0.8\" fill=\"#38bdf8\">{}</text>\n\
        \x20 <text x=\"220\" y=\"219\" text-anchor=\"middle\" font-family=\"-apple-system, SF Pro Display, Helvetica, Arial, sans-serif\" font-size=\"13.5\" font-weight=\"700\" fill=\"#f8fafc\">{}</text>\n\
        \x20 <text x=\"220\" y=\"239\" text-anchor=\"middle\" font-family=\"-apple-system, SF Pro Display, Helvetica, Arial, sans-serif\" font-size=\"11\" font-weight=\"500\" fill=\"rgba(203,213,225,0.88)\">{}</text>\n\
        </svg>\n",
        xml_escape(mode_badge),
        xml_escape(&truncated_title),
        xml_escape(&truncated_sub),
    );
    let sector_labels = slots.into_iter().map(|s| s.label).collect();
    (svg, sector_labels)
}

fn execute_preview_command(
    out: PathBuf,
    files: Vec<PathBuf>,
    mode: Option<String>,
    hover: Option<usize>,
    prefs: &AppPreferences,
    cwd: Option<&Path>,
) -> CliOutcome {
    let eff_mode = mode.unwrap_or_else(|| "convert".to_string());
    let out_path = resolve_cli_path(&out, cwd);
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(parent) {
            let msg = format!("Failed to create parent directory for preview SVG: {e}");
            let err_json = serde_json::to_string_pretty(&json!({
                "success": false,
                "command": "preview",
                "error": msg,
            }))
            .unwrap_or_else(|_| "{\"success\":false}".to_string());
            return CliOutcome {
                exit_code: 1,
                stdout: format!("{err_json}\n"),
                stderr: format!("{msg}\n"),
            };
        }
    }

    let (svg, sectors) = render_wheel_preview_svg(&files, &eff_mode, hover, prefs, cwd);
    if let Err(e) = std::fs::write(&out_path, &svg) {
        let msg = format!("Failed to write preview SVG `{}`: {e}", out_path.display());
        let err_json = serde_json::to_string_pretty(&json!({
            "success": false,
            "command": "preview",
            "error": msg,
        }))
        .unwrap_or_else(|_| "{\"success\":false}".to_string());
        return CliOutcome {
            exit_code: 1,
            stdout: format!("{err_json}\n"),
            stderr: format!("{msg}\n"),
        };
    }

    let out_json = serde_json::to_string_pretty(&json!({
        "success": true,
        "command": "preview",
        "out": out_path.to_string_lossy(),
        "mode": eff_mode,
        "sectorCount": sectors.len(),
        "sectors": sectors,
        "hover": hover,
    }))
    .unwrap_or_else(|_| "{\"success\":true}".to_string());
    CliOutcome {
        exit_code: 0,
        stdout: format!("{out_json}\n"),
        stderr: String::new(),
    }
}

pub enum DispatchAction {
    Exit(i32),
    LaunchGui {
        initial_url: Option<String>,
        is_login_item: bool,
    },
}

pub fn dispatch_from_args<I, S>(args: I) -> DispatchAction
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let collected: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let is_login_item = has_login_item_flag(&collected);
    match parse_cli_args(&collected) {
        Ok(CliCommand::Gui { initial_url }) => DispatchAction::LaunchGui {
            initial_url,
            is_login_item,
        },
        Ok(cmd) => {
            let mut prefs = AppPreferences::load();
            prefs.sanitize();
            let outcome = execute_cli_command(cmd, &prefs);
            if !outcome.stdout.is_empty() {
                print!("{}", outcome.stdout);
            }
            if !outcome.stderr.is_empty() {
                eprint!("{}", outcome.stderr);
            }
            DispatchAction::Exit(outcome.exit_code)
        }
        Err(err) => {
            let err_json = serde_json::to_string_pretty(&json!({
                "success": false,
                "error": err,
            }))
            .unwrap_or_else(|_| "{\"success\":false}".to_string());
            println!("{err_json}");
            eprintln!("{err}");
            DispatchAction::Exit(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn cli_parser_handles_gui_launch_flags_help_version_convert_and_new() {
        assert_eq!(
            parse_cli_args(["flip"]).unwrap(),
            CliCommand::Gui { initial_url: None }
        );
        assert_eq!(
            parse_cli_args(["flip", "-psn_0_12345"]).unwrap(),
            CliCommand::Gui { initial_url: None }
        );
        assert_eq!(
            parse_cli_args(["flip", "--login-item"]).unwrap(),
            CliCommand::Gui { initial_url: None }
        );
        assert!(has_login_item_flag(["flip", "--login-item"]));
        assert!(!has_login_item_flag(["flip"]));

        assert_eq!(parse_cli_args(["flip", "--help"]).unwrap(), CliCommand::Help);
        assert_eq!(
            parse_cli_args(["flip", "--version"]).unwrap(),
            CliCommand::Version
        );

        assert_eq!(
            parse_cli_args(["flip", "convert", "a.png", "b.jpg", "--to", "webp,pdf"]).unwrap(),
            CliCommand::Convert {
                files: vec![PathBuf::from("a.png"), PathBuf::from("b.jpg")],
                formats: vec!["WEBP".to_string(), "PDF".to_string()],
            }
        );
        assert_eq!(
            parse_cli_args(["flip", "convert", "--to=PNG", "--", "-dash.heic"]).unwrap(),
            CliCommand::Convert {
                files: vec![PathBuf::from("-dash.heic")],
                formats: vec!["PNG".to_string()],
            }
        );
        assert_eq!(
            parse_cli_args(["flip", "convert", "a.png", "-t", ".jpg", "--to", "webp,jpg"])
                .unwrap(),
            CliCommand::Convert {
                files: vec![PathBuf::from("a.png")],
                formats: vec!["JPG".to_string(), "WEBP".to_string()],
            }
        );

        assert_eq!(
            parse_cli_args(["flip", "new", "MD", "--in", "/tmp/work", "--no-rename"]).unwrap(),
            CliCommand::New {
                kind: "md".to_string(),
                folder: Some(PathBuf::from("/tmp/work")),
                no_rename: true,
                no_reveal: false,
            }
        );

        assert!(parse_cli_args(["flip", "convert", "a.png"]).is_err());
        assert!(parse_cli_args(["flip", "convert", "--to", "PNG"]).is_err());
        assert!(parse_cli_args(["flip", "convert", "a.png", "--unknown"]).is_err());
        assert!(parse_cli_args(["flip", "new"]).is_err());
        assert!(parse_cli_args(["flip", "new", "txt", "--in"]).is_err());
        assert!(parse_cli_args(["flip", "new", "txt", "--in", ""]).is_err());
        assert!(parse_cli_args(["flip", "new", "txt", "--in="]).is_err());
        assert!(parse_cli_args(["flip", "bogus"]).is_err());
    }

    #[test]
    fn cli_execute_convert_and_new_end_to_end_json_and_exit_codes() {
        let dir = tempdir().unwrap();
        let prefs = AppPreferences::default();

        // Create a real JSON file and convert to YAML + TOML via CLI
        let src = dir.path().join("sample.json");
        fs::write(&src, r#"{"hello":"world","count":3}"#).unwrap();

        let conv_cmd = parse_cli_args([
            "flip",
            "convert",
            src.to_str().unwrap(),
            "--to",
            "YAML,TOML",
        ])
        .unwrap();
        let conv_out = execute_cli_command(conv_cmd, &prefs);
        assert_eq!(conv_out.exit_code, 0, "stderr: {}", conv_out.stderr);
        let parsed: serde_json::Value = serde_json::from_str(&conv_out.stdout).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["results"].as_array().unwrap().len(), 2);
        assert!(dir.path().join("sample.yaml").is_file());
        assert!(dir.path().join("sample.toml").is_file());

        // Relative paths (`sample.json` / `./sub/../sample.json`) must resolve against cwd
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let rel_cmd = parse_cli_args([
            "flip",
            "convert",
            "./sub/../sample.json",
            "--to",
            "XML",
        ])
        .unwrap();
        let rel_out = execute_cli_command_in(rel_cmd, &prefs, Some(dir.path()));
        assert_eq!(rel_out.exit_code, 0, "stderr: {}", rel_out.stderr);
        assert!(dir.path().join("sample.xml").is_file());

        // Relative symlink must STILL be refused by policy (leaf symlink is not dereferenced)
        let sym = dir.path().join("link.json");
        std::os::unix::fs::symlink(&src, &sym).unwrap();
        let sym_cmd = parse_cli_args(["flip", "convert", "link.json", "--to", "YAML"]).unwrap();
        let sym_out = execute_cli_command_in(sym_cmd, &prefs, Some(dir.path()));
        assert_eq!(sym_out.exit_code, 1);
        assert!(sym_out.stderr.contains("Symbolic links"));

        // Missing file -> exit_code 1 with JSON error
        let bad_conv = parse_cli_args([
            "flip",
            "convert",
            dir.path().join("missing.png").to_str().unwrap(),
            "--to",
            "JPG",
        ])
        .unwrap();
        let bad_out = execute_cli_command(bad_conv, &prefs);
        assert_eq!(bad_out.exit_code, 1);
        let bad_json: serde_json::Value = serde_json::from_str(&bad_out.stdout).unwrap();
        assert_eq!(bad_json["success"], false);

        // `flip new md --in <dir> --no-rename --no-reveal`
        let new_cmd = parse_cli_args([
            "flip",
            "new",
            "md",
            "--in",
            dir.path().to_str().unwrap(),
            "--no-rename",
            "--no-reveal",
        ])
        .unwrap();
        let new_out = execute_cli_command(new_cmd, &prefs);
        assert_eq!(new_out.exit_code, 0, "stderr: {}", new_out.stderr);
        let new_json: serde_json::Value = serde_json::from_str(&new_out.stdout).unwrap();
        assert_eq!(new_json["kindId"], "md");
        assert_eq!(new_json["renameTriggered"], false);
        assert!(PathBuf::from(new_json["path"].as_str().unwrap()).is_file());

        // `flip new ... --in .` resolves cleanly to an absolute path and expands {{folder}}
        let work_dir = dir.path().join("MyFolder");
        fs::create_dir_all(&work_dir).unwrap();
        let tmpl = dir.path().join("Note.md");
        fs::write(&tmpl, b"folder={{folder}}\n").unwrap();
        let dot_cmd = parse_cli_args([
            "flip",
            "new",
            "template:../Note.md",
            "--in",
            ".",
            "--no-rename",
            "--no-reveal",
        ])
        .unwrap();
        let dot_out = execute_cli_command_in(dot_cmd, &prefs, Some(&work_dir));
        assert_eq!(dot_out.exit_code, 0, "stderr: {}", dot_out.stderr);
        let dot_json: serde_json::Value = serde_json::from_str(&dot_out.stdout).unwrap();
        let created_path = PathBuf::from(dot_json["path"].as_str().unwrap());
        assert!(created_path.is_absolute());
        assert_eq!(created_path, work_dir.join("Note.md"));
        assert_eq!(fs::read_to_string(&created_path).unwrap(), "folder=MyFolder\n");

        // `flip new md --in /nonexistent` must fail rather than falling back to Desktop
        let bad_dir_cmd = parse_cli_args([
            "flip",
            "new",
            "md",
            "--in",
            dir.path().join("no-such-dir").to_str().unwrap(),
            "--no-rename",
            "--no-reveal",
        ])
        .unwrap();
        let bad_dir_out = execute_cli_command(bad_dir_cmd, &prefs);
        assert_eq!(bad_dir_out.exit_code, 1);
    }

    #[test]
    fn cli_parser_and_ipc_execution_for_wheel_settings_batch_and_quit() {
        use std::sync::Mutex;

        assert_eq!(
            parse_cli_args(["flip", "wheel"]).unwrap(),
            CliCommand::Wheel {
                mode: None,
                files: vec![],
            }
        );
        assert_eq!(
            parse_cli_args(["flip", "wheel", "--mode", "CONVERT", "a.png", "b.jpg"]).unwrap(),
            CliCommand::Wheel {
                mode: Some("convert".to_string()),
                files: vec![PathBuf::from("a.png"), PathBuf::from("b.jpg")],
            }
        );
        assert_eq!(
            parse_cli_args(["flip", "wheel", "--mode=new", "--", "-file.txt"]).unwrap(),
            CliCommand::Wheel {
                mode: Some("new".to_string()),
                files: vec![PathBuf::from("-file.txt")],
            }
        );
        assert_eq!(
            parse_cli_args(["flip", "settings"]).unwrap(),
            CliCommand::Settings
        );
        assert_eq!(
            parse_cli_args(["flip", "batch"]).unwrap(),
            CliCommand::Batch { files: vec![] }
        );
        assert_eq!(
            parse_cli_args(["flip", "batch", "one.png", "two.jpg"]).unwrap(),
            CliCommand::Batch {
                files: vec![PathBuf::from("one.png"), PathBuf::from("two.jpg")],
            }
        );
        assert_eq!(parse_cli_args(["flip", "quit"]).unwrap(), CliCommand::Quit);

        assert!(parse_cli_args(["flip", "wheel", "--mode", "bogus"]).is_err());
        assert!(parse_cli_args(["flip", "wheel", "--mode"]).is_err());
        assert!(parse_cli_args(["flip", "wheel", "--unknown"]).is_err());
        assert!(parse_cli_args(["flip", "settings", "extra"]).is_err());
        assert!(parse_cli_args(["flip", "batch", "--unknown"]).is_err());
        assert!(parse_cli_args(["flip", "quit", "extra"]).is_err());

        let dir = tempdir().unwrap();
        let sock = dir.path().join("flip-cli-test.sock");
        let work_dir = dir.path().join("work");
        fs::create_dir_all(&work_dir).unwrap();
        let prefs = AppPreferences::default();

        // 1. Offline `flip quit` must NOT launch app and must return explicit offline JSON
        let mut launch_calls = 0usize;
        let offline_quit = execute_cli_command_with_ipc(
            CliCommand::Quit,
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_millis(400),
            || {
                launch_calls += 1;
                Ok(())
            },
        );
        assert_eq!(launch_calls, 0, "quit must never cold-launch host app");
        assert_eq!(offline_quit.exit_code, 1);
        let oq_json: serde_json::Value = serde_json::from_str(&offline_quit.stdout).unwrap();
        assert_eq!(oq_json["success"], false);
        assert_eq!(oq_json["running"], false);

        // 2. Offline `flip settings` cold-launches host & retries IPC cleanly
        let received: Arc<Mutex<Vec<IpcCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let rec_for_server = Arc::clone(&received);
        let sock_for_launch = sock.clone();
        let server_slot: Arc<Mutex<Option<ipc_socket::IpcServerHandle>>> =
            Arc::new(Mutex::new(None));
        let slot_for_launch = Arc::clone(&server_slot);

        let cold_settings = execute_cli_command_with_ipc(
            CliCommand::Settings,
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(2),
            || {
                let rec = Arc::clone(&rec_for_server);
                let s_path = sock_for_launch.clone();
                let slot = Arc::clone(&slot_for_launch);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(60));
                    let handle = ipc_socket::start_server(&s_path, move |cmd| {
                        rec.lock().unwrap().push(cmd);
                        IpcResponse::ok("handled")
                    })
                    .unwrap();
                    *slot.lock().unwrap() = Some(handle);
                });
                Ok(())
            },
        );
        assert_eq!(cold_settings.exit_code, 0, "stderr: {}", cold_settings.stderr);
        let cs_json: serde_json::Value = serde_json::from_str(&cold_settings.stdout).unwrap();
        assert_eq!(cs_json["success"], true);
        assert_eq!(cs_json["launched"], true);
        assert_eq!(cs_json["command"], "open_settings");

        // 3. Running host: `flip wheel`, `flip batch`, `flip quit` resolve relative paths & send JSON
        let wheel_out = execute_cli_command_with_ipc(
            parse_cli_args(["flip", "wheel", "--mode", "convert", "./sub/../photo.heic"]).unwrap(),
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("must not launch when server is already running"),
        );
        assert_eq!(wheel_out.exit_code, 0);
        let wheel_json: serde_json::Value = serde_json::from_str(&wheel_out.stdout).unwrap();
        assert_eq!(wheel_json["launched"], false);

        let batch_out = execute_cli_command_with_ipc(
            parse_cli_args(["flip", "batch", "a.png", "b.jpg"]).unwrap(),
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("must not launch when server is already running"),
        );
        assert_eq!(batch_out.exit_code, 0);

        let quit_out = execute_cli_command_with_ipc(
            CliCommand::Quit,
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("must not launch on quit"),
        );
        assert_eq!(quit_out.exit_code, 0);

        // 4. `flip wheel --mode new` with existing dir, existing file, and nonexistent path
        let sub_folder = work_dir.join("notes_dir");
        fs::create_dir_all(&sub_folder).unwrap();
        let w_dir_out = execute_cli_command_with_ipc(
            parse_cli_args(["flip", "wheel", "--mode", "new", "./notes_dir"]).unwrap(),
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("server already running"),
        );
        assert_eq!(w_dir_out.exit_code, 0);

        let sub_file = sub_folder.join("doc.txt");
        fs::write(&sub_file, b"hi").unwrap();
        let w_file_out = execute_cli_command_with_ipc(
            parse_cli_args(["flip", "wheel", "--mode", "new", "notes_dir/doc.txt"]).unwrap(),
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("server already running"),
        );
        assert_eq!(w_file_out.exit_code, 0);

        let w_missing_out = execute_cli_command_with_ipc(
            parse_cli_args(["flip", "wheel", "--mode", "new", "./missing_dir"]).unwrap(),
            &prefs,
            Some(&work_dir),
            &sock,
            Duration::from_secs(1),
            || panic!("must fail before launching on nonexistent path"),
        );
        assert_eq!(w_missing_out.exit_code, 1);

        let recorded = received.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![
                IpcCommand::OpenSettings,
                IpcCommand::OpenWheel {
                    mode: Some("convert".to_string()),
                    files: vec![lexical_normalize(&work_dir).join("photo.heic").to_string_lossy().into_owned()],
                    folder: None,
                },
                IpcCommand::OpenBatch {
                    files: vec![
                        lexical_normalize(&work_dir).join("a.png").to_string_lossy().into_owned(),
                        lexical_normalize(&work_dir).join("b.jpg").to_string_lossy().into_owned(),
                    ],
                },
                IpcCommand::Quit,
                IpcCommand::OpenWheel {
                    mode: Some("new".to_string()),
                    files: vec![],
                    folder: Some(lexical_normalize(&sub_folder).to_string_lossy().into_owned()),
                },
                IpcCommand::OpenWheel {
                    mode: Some("new".to_string()),
                    files: vec![],
                    folder: Some(lexical_normalize(&sub_folder).to_string_lossy().into_owned()),
                },
            ]
        );

        if let Some(mut srv) = server_slot.lock().unwrap().take() {
            srv.stop();
        }
        assert!(!sock.exists(), "server stop must remove socket file");
    }

    #[test]
    fn cli_preview_parses_flags_and_generates_well_formed_wheel_svg() {
        let parsed = parse_cli_args([
            "flip",
            "preview",
            "--out",
            "out/wheel.svg",
            "--files",
            "photo.heic,extra.png",
            "--mode",
            "convert",
            "--hover",
            "1",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            CliCommand::Preview {
                out: PathBuf::from("out/wheel.svg"),
                files: vec![PathBuf::from("photo.heic"), PathBuf::from("extra.png")],
                mode: Some("convert".to_string()),
                hover: Some(1),
            }
        );
        assert!(parse_cli_args(["flip", "preview"]).is_err());
        assert!(parse_cli_args(["flip", "preview", "--out", "a.svg", "--hover", "bad"]).is_err());

        let dir = tempfile::tempdir().unwrap();
        let svg_out = dir.path().join("wheel-convert.svg");
        let prefs = AppPreferences::default();
        let outcome = execute_cli_command_with_ipc(
            parse_cli_args([
                "flip",
                "preview",
                "--out",
                svg_out.to_str().unwrap(),
                "--files",
                "photo.heic",
                "--hover",
                "0",
            ])
            .unwrap(),
            &prefs,
            Some(dir.path()),
            &dir.path().join("unused.sock"),
            Duration::from_millis(50),
            || panic!("preview must not spawn GUI"),
        );
        assert_eq!(outcome.exit_code, 0, "stderr={}", outcome.stderr);
        let svg = std::fs::read_to_string(&svg_out).unwrap();
        assert!(svg.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("data-slot-index=\"0\" data-active=\"true\""));
        assert!(svg.contains("PNG"));
        assert!(svg.contains("photo.heic"));

        // Verify XML well-formedness using quick-xml
        let mut reader = quick_xml::Reader::from_str(&svg);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("Invalid preview SVG XML: {e}\n{svg}"),
            }
            buf.clear();
        }

        // Also test --mode new preview
        let new_svg_out = dir.path().join("wheel-new.svg");
        let new_outcome = execute_cli_command_with_ipc(
            parse_cli_args([
                "flip",
                "preview",
                "--out",
                new_svg_out.to_str().unwrap(),
                "--mode=new",
                "--hover=2",
            ])
            .unwrap(),
            &prefs,
            Some(dir.path()),
            &dir.path().join("unused.sock"),
            Duration::from_millis(50),
            || panic!("preview must not spawn GUI"),
        );
        assert_eq!(new_outcome.exit_code, 0);
        let new_svg = std::fs::read_to_string(&new_svg_out).unwrap();
        assert!(new_svg.contains("NEW FILE"));
        assert!(new_svg.contains("Folder"));
        assert!(new_svg.contains("data-slot-index=\"2\" data-active=\"true\""));

        // Verify sector geometry parity with wheelGeometry.ts for count=1 and count=2
        let path_one = build_preview_sector_path(0, 1);
        assert!(path_one.contains("A 170 170 0 1 1"), "path_one={path_one}");
        assert!(path_one.contains("A 78 78 0 1 0"), "path_one={path_one}");
        assert!(path_one.contains(" Q "), "path_one={path_one}");

        let path_two = build_preview_sector_path(0, 2);
        assert!(path_two.contains("A 170 170 0 0 1"), "path_two={path_two}");
        assert!(path_two.contains("A 78 78 0 0 0"), "path_two={path_two}");
        assert!(path_two.contains(" Q "), "path_two={path_two}");

        // Multi-file preview surfaces batch actions on page 1 before conversion outputs
        let batch_svg_out = dir.path().join("wheel-batch.svg");
        let batch_outcome = execute_cli_command_with_ipc(
            parse_cli_args([
                "flip",
                "preview",
                "--out",
                batch_svg_out.to_str().unwrap(),
                "--files",
                "a.png,b.png",
            ])
            .unwrap(),
            &prefs,
            Some(dir.path()),
            &dir.path().join("unused.sock"),
            Duration::from_millis(50),
            || panic!("preview must not spawn GUI"),
        );
        assert_eq!(batch_outcome.exit_code, 0);
        let batch_svg = std::fs::read_to_string(&batch_svg_out).unwrap();
        assert!(batch_svg.contains(">ZIP<"), "batch_svg={batch_svg}");

        // Custom new_file_visible_kinds puts reordered item on page 1
        let mut custom_prefs = prefs.clone();
        custom_prefs.new_file_visible_kinds = vec!["json".to_string(), "folder".to_string()];
        let (_slots, _title, _sub) = compute_preview_wheel(&[], "new", &custom_prefs, None);
        let (reordered_svg, reordered_labels) =
            render_wheel_preview_svg(&[], "new", None, &custom_prefs, None);
        assert_eq!(reordered_labels.first().map(String::as_str), Some("JSON"));
        assert!(reordered_svg.contains(">JSON<"), "reordered_svg={reordered_svg}");

        // Single unknown file still offers ZIP; non-iWork directory renders 8 disabled placeholder sectors + "No common output format"
        let (_zip_svg, zip_labels) =
            render_wheel_preview_svg(&[PathBuf::from("unknown.xyz123")], "convert", None, &prefs, None);
        assert_eq!(zip_labels, vec!["ZIP".to_string()]);

        let plain_dir = dir.path().join("plain-folder");
        std::fs::create_dir(&plain_dir).unwrap();
        let (empty_svg, empty_labels) =
            render_wheel_preview_svg(&[plain_dir], "convert", None, &prefs, None);
        assert_eq!(empty_labels.len(), 8);
        assert!(empty_svg.contains("No common output format"), "empty_svg={empty_svg}");
        assert!(empty_svg.contains("opacity=\"0.42\""), "empty_svg={empty_svg}");
    }
}
