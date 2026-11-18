use crate::core::ipc_socket::{self, IpcCommand, IpcResponse, IpcSocketError};
use crate::core::preferences::{expand_tilde_path, AppPreferences};
use crate::core::quick_actions::{
    parse_flip_url, resolve_app_bundle_path, resolve_cli_executable, FlipUrlAction,
};
use crate::core::startup::LOGIN_ITEM_ARG;
use crate::engines::create_new::create_new_item;
use crate::engines::{convert_files, ConversionResult};
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
  flip quit                                    Quit running FLIP menu bar instance\n  \
  flip --help                                  Show this help message\n  \
  flip --version                               Show version information\n\n\
EXAMPLES:\n  \
  flip convert photo.heic --to PNG,JPG\n  \
  flip new md --in ~/Desktop\n  \
  flip new folder --in ~/Documents --no-rename\n  \
  flip wheel --mode convert photo.heic\n  \
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
        let status = Command::new("/usr/bin/open")
            .arg("-g")
            .arg("-a")
            .arg(&bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("Failed to launch `{}`: {e}", bundle.display()))?;
        if status.success() {
            return Ok(());
        }
        return Err(format!(
            "`open -a {}` exited with status {:?}",
            bundle.display(),
            status.code()
        ));
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

    match ipc_socket::send_command(socket_path, &ipc_cmd) {
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
        Duration::from_secs(5),
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
            let folder = if mode.as_deref() == Some("new")
                && resolved_files.len() == 1
                && Path::new(&resolved_files[0]).is_dir()
            {
                Some(resolved_files.remove(0))
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
            ]
        );

        if let Some(mut srv) = server_slot.lock().unwrap().take() {
            srv.stop();
        }
        assert!(!sock.exists(), "server stop must remove socket file");
    }
}
