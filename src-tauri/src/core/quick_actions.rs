use crate::core::logging::log_event;
use crate::core::startup::{xml_escape, APPLICATIONS_FLIP_EXE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const CONVERT_WORKFLOW_NAME: &str = "Convert with FLIP.workflow";
pub const NEW_FILE_WORKFLOW_NAME: &str = "New File with FLIP.workflow";
pub const FLIP_BUNDLE_ID: &str = "com.thetinyfoundry.flip.mac";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlipUrlAction {
    Convert {
        files: Vec<String>,
        formats: Vec<String>,
    },
    OpenWheel {
        mode: String,
        files: Vec<String>,
        folder: Option<String>,
    },
    CreateNew {
        kind: Option<String>,
        folder: Option<String>,
        no_rename: bool,
    },
    OpenBatch {
        files: Vec<String>,
    },
    OpenSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickActionsStatus {
    pub services_dir: String,
    pub convert_installed: bool,
    pub new_file_installed: bool,
    pub convert_path: String,
    pub new_file_path: String,
    pub cli_executable: Option<String>,
    pub cli_symlink_path: Option<String>,
}

pub fn percent_decode(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(format!("Incomplete percent escape in `{input}`"));
            }
            let hi = (bytes[i + 1] as char)
                .to_digit(16)
                .ok_or_else(|| format!("Invalid hex digit in `{input}`"))?;
            let lo = (bytes[i + 2] as char)
                .to_digit(16)
                .ok_or_else(|| format!("Invalid hex digit in `{input}`"))?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|e| format!("Invalid UTF-8 in URL parameter: {e}"))
}

pub fn parse_flip_url(raw: &str) -> Result<FlipUrlAction, String> {
    let trimmed = raw.trim();
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);

    let colon_pos = without_fragment
        .find("://")
        .ok_or_else(|| format!("Invalid URL (missing scheme): `{raw}`"))?;
    let scheme = &without_fragment[..colon_pos];
    if !scheme.eq_ignore_ascii_case("flip") {
        return Err(format!("Unsupported URL scheme `{scheme}` (expected `flip://`)"));
    }

    let rest = &without_fragment[colon_pos + 3..];
    let (authority_and_path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };

    let action = authority_and_path
        .trim_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut files: Vec<String> = Vec::new();
    let mut formats: Vec<String> = Vec::new();
    let mut mode: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut folder: Option<String> = None;
    let mut no_rename = false;

    if let Some(q) = query {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (raw_k, raw_v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            let key = percent_decode(raw_k)?.to_ascii_lowercase();
            let val = percent_decode(raw_v)?;
            match key.as_str() {
                "file" | "path" => {
                    if !val.trim().is_empty() {
                        files.push(val);
                    }
                }
                "files" => {
                    for part in val.split(',') {
                        let p = part.trim();
                        if !p.is_empty() {
                            files.push(p.to_string());
                        }
                    }
                }
                "to" | "format" | "formats" => {
                    for part in val.split(',') {
                        let up = part.trim().trim_start_matches('.').to_ascii_uppercase();
                        if !up.is_empty() && !formats.contains(&up) {
                            formats.push(up);
                        }
                    }
                }
                "mode" => {
                    let m = val.trim().to_ascii_lowercase();
                    if matches!(m.as_str(), "convert" | "new" | "auto") {
                        mode = Some(m);
                    }
                }
                "kind" => {
                    let trimmed_k = val.trim();
                    if !trimmed_k.is_empty() {
                        kind = Some(if trimmed_k.starts_with("template:") {
                            trimmed_k.to_string()
                        } else {
                            trimmed_k.to_ascii_lowercase()
                        });
                    }
                }
                "folder" | "in" | "dir" => {
                    let f = val.trim();
                    if !f.is_empty() {
                        folder = Some(f.to_string());
                    }
                }
                "no_rename" | "norename" => {
                    let v = val.trim().to_ascii_lowercase();
                    no_rename = matches!(v.as_str(), "" | "1" | "true" | "yes");
                }
                "rename" => {
                    let v = val.trim().to_ascii_lowercase();
                    if matches!(v.as_str(), "0" | "false" | "no") {
                        no_rename = true;
                    }
                }
                _ => {}
            }
        }
    }

    match action.as_str() {
        "convert" => {
            if !formats.is_empty() {
                Ok(FlipUrlAction::Convert { files, formats })
            } else {
                Ok(FlipUrlAction::OpenWheel {
                    mode: "convert".to_string(),
                    files,
                    folder,
                })
            }
        }
        "new" => Ok(FlipUrlAction::CreateNew {
            kind,
            folder,
            no_rename,
        }),
        "wheel" => {
            let resolved_mode = mode.unwrap_or_else(|| {
                if files.is_empty() {
                    "new".to_string()
                } else {
                    "convert".to_string()
                }
            });
            Ok(FlipUrlAction::OpenWheel {
                mode: resolved_mode,
                files,
                folder,
            })
        }
        "batch" => Ok(FlipUrlAction::OpenBatch { files }),
        "settings" => Ok(FlipUrlAction::OpenSettings),
        "" | "open" => Ok(FlipUrlAction::OpenWheel {
            mode: mode.unwrap_or_else(|| "auto".to_string()),
            files,
            folder,
        }),
        other => Err(format!("Unsupported flip:// URL action `{other}`")),
    }
}

pub fn default_services_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Library")
        .join("Services")
}

pub fn default_cli_symlink_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".local")
        .join("bin")
}

pub fn resolve_app_bundle_path(current_exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(exe) = current_exe {
        let s = exe.to_string_lossy();
        if let Some(idx) = s.find(".app/Contents/MacOS/") {
            let app_bundle = PathBuf::from(&s[..idx + 4]);
            if app_bundle.is_dir() {
                return Some(app_bundle);
            }
        }
    }
    let system_app = PathBuf::from("/Applications/FLIP.app");
    if system_app.is_dir() {
        return Some(system_app);
    }
    if let Some(home) = dirs::home_dir() {
        let user_app = home.join("Applications").join("FLIP.app");
        if user_app.is_dir() {
            return Some(user_app);
        }
    }
    None
}

pub fn resolve_cli_executable(current_exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(app_bundle) = resolve_app_bundle_path(current_exe) {
        let exe = app_bundle.join("Contents").join("MacOS").join("flip");
        if exe.is_file() {
            return Some(exe);
        }
    }
    let sys_exe = PathBuf::from(APPLICATIONS_FLIP_EXE);
    if sys_exe.is_file() {
        return Some(sys_exe);
    }
    if let Some(exe) = current_exe {
        if exe.is_file() {
            return Some(exe.to_path_buf());
        }
    }
    None
}

fn zsh_single_quote(raw: &str) -> String {
    let escaped = raw.replace('\'', r"'\''");
    format!("'{escaped}'")
}

pub fn build_convert_workflow_script(app_path: Option<&Path>, cli_exe: Option<&Path>) -> String {
    let app_hint = app_path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Applications/FLIP.app".to_string());
    let bin_hint = cli_exe
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| APPLICATIONS_FLIP_EXE.to_string());
    let app_hint_q = zsh_single_quote(&app_hint);
    let bin_hint_q = zsh_single_quote(&bin_hint);

    format!(
        r#"#!/bin/zsh
set -euo pipefail

APP_HINT={app_hint_q}
BIN_HINT={bin_hint_q}

pct_encode() {{
  setopt localoptions nomultibyte
  local raw="$1" out="" i ch hex
  local LC_ALL=C
  for (( i = 1; i <= $#raw; i++ )); do
    ch="${{raw[i]}}"
    case "$ch" in
      [A-Za-z0-9._~/-]) out+="$ch" ;;
      *)
        printf -v hex '%%%02X' "'$ch"
        out+="$hex"
        ;;
    esac
  done
  printf '%s' "$out"
}}

URL="flip://wheel?mode=convert"
for f in "$@"; do
  [ -n "$f" ] || continue
  URL+="&file=$(pct_encode "$f")"
done

APP_PATH=""
if [ -d "$APP_HINT" ]; then
  APP_PATH="$APP_HINT"
elif [ -d "/Applications/FLIP.app" ]; then
  APP_PATH="/Applications/FLIP.app"
elif [ -d "$HOME/Applications/FLIP.app" ]; then
  APP_PATH="$HOME/Applications/FLIP.app"
else
  FOUND="$(/usr/bin/mdfind "kMDItemCFBundleIdentifier == '{FLIP_BUNDLE_ID}'" 2>/dev/null | /usr/bin/head -n 1 || true)"
  if [ -n "$FOUND" ] && [ -d "$FOUND" ]; then
    APP_PATH="$FOUND"
  fi
fi

if [ -n "$APP_PATH" ]; then
  /usr/bin/open -a "$APP_PATH" "$URL"
elif [ -x "$BIN_HINT" ]; then
  "$BIN_HINT" "$URL" >/dev/null 2>&1 &
else
  /usr/bin/open -b "{FLIP_BUNDLE_ID}" "$URL"
fi
"#
    )
}

pub fn build_new_file_workflow_script(app_path: Option<&Path>, cli_exe: Option<&Path>) -> String {
    let app_hint = app_path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Applications/FLIP.app".to_string());
    let bin_hint = cli_exe
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| APPLICATIONS_FLIP_EXE.to_string());
    let app_hint_q = zsh_single_quote(&app_hint);
    let bin_hint_q = zsh_single_quote(&bin_hint);

    format!(
        r#"#!/bin/zsh
set -euo pipefail

APP_HINT={app_hint_q}
BIN_HINT={bin_hint_q}

pct_encode() {{
  setopt localoptions nomultibyte
  local raw="$1" out="" i ch hex
  local LC_ALL=C
  for (( i = 1; i <= $#raw; i++ )); do
    ch="${{raw[i]}}"
    case "$ch" in
      [A-Za-z0-9._~/-]) out+="$ch" ;;
      *)
        printf -v hex '%%%02X' "'$ch"
        out+="$hex"
        ;;
    esac
  done
  printf '%s' "$out"
}}

TARGET_FOLDER=""
if [ "$#" -gt 0 ] && [ -n "$1" ]; then
  if [ -d "$1" ]; then
    TARGET_FOLDER="$1"
  else
    TARGET_FOLDER="${{1:h}}"
  fi
fi

URL="flip://wheel?mode=new"
if [ -n "$TARGET_FOLDER" ]; then
  URL+="&folder=$(pct_encode "$TARGET_FOLDER")"
fi

APP_PATH=""
if [ -d "$APP_HINT" ]; then
  APP_PATH="$APP_HINT"
elif [ -d "/Applications/FLIP.app" ]; then
  APP_PATH="/Applications/FLIP.app"
elif [ -d "$HOME/Applications/FLIP.app" ]; then
  APP_PATH="$HOME/Applications/FLIP.app"
else
  FOUND="$(/usr/bin/mdfind "kMDItemCFBundleIdentifier == '{FLIP_BUNDLE_ID}'" 2>/dev/null | /usr/bin/head -n 1 || true)"
  if [ -n "$FOUND" ] && [ -d "$FOUND" ]; then
    APP_PATH="$FOUND"
  fi
fi

if [ -n "$APP_PATH" ]; then
  /usr/bin/open -a "$APP_PATH" "$URL"
elif [ -x "$BIN_HINT" ]; then
  "$BIN_HINT" "$URL" >/dev/null 2>&1 &
else
  /usr/bin/open -b "{FLIP_BUNDLE_ID}" "$URL"
fi
"#
    )
}

pub fn render_service_info_plist(menu_title: &str, send_file_types: &[&str]) -> String {
    let escaped_title = xml_escape(menu_title);
    let types_xml = send_file_types
        .iter()
        .map(|t| format!("\t\t\t\t<string>{}</string>", xml_escape(t)))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSBackgroundColorName</key>
			<string>background</string>
			<key>NSIconName</key>
			<string>NSActionTemplate</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{escaped_title}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
{types_xml}
			</array>
		</dict>
	</array>
</dict>
</plist>
"#
    )
}

pub fn render_automator_document_wflow(shell_script: &str) -> String {
    let escaped_script = xml_escape(shell_script);
    let action_uuid = Uuid::new_v4().to_string().to_uppercase();
    let input_uuid = Uuid::new_v4().to_string().to_uppercase();
    let output_uuid = Uuid::new_v4().to_string().to_uppercase();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{escaped_script}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/zsh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>{input_uuid}</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
					<string>Unix</string>
				</array>
				<key>OutputUUID</key>
				<string>{output_uuid}</string>
				<key>UUID</key>
				<string>{action_uuid}</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict>
					<key>0</key>
					<dict>
						<key>default value</key>
						<integer>0</integer>
						<key>name</key>
						<string>inputMethod</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>0</string>
					</dict>
					<key>1</key>
					<dict>
						<key>default value</key>
						<false/>
						<key>name</key>
						<string>CheckedForUserDefaultShell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>1</string>
					</dict>
					<key>2</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>source</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>2</string>
					</dict>
					<key>3</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>COMMAND_STRING</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>3</string>
					</dict>
					<key>4</key>
					<dict>
						<key>default value</key>
						<string>/bin/sh</string>
						<key>name</key>
						<string>shell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>4</string>
					</dict>
				</dict>
				<key>isViewVisible</key>
				<integer>1</integer>
				<key>location</key>
				<string>309.000000:253.000000</string>
				<key>nibPath</key>
				<string>/System/Library/Automator/Run Shell Script.action/Contents/Resources/Base.lproj/main.nib</string>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleID</key>
		<string>com.apple.finder</string>
		<key>applicationBundleIDsByPath</key>
		<dict>
			<key>/System/Library/CoreServices/Finder.app</key>
			<string>com.apple.finder</string>
		</dict>
		<key>applicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>applicationPaths</key>
		<array>
			<string>/System/Library/CoreServices/Finder.app</string>
		</array>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>15</integer>
		<key>processesInput</key>
		<integer>0</integer>
		<key>serviceApplicationBundleID</key>
		<string>com.apple.finder</string>
		<key>serviceApplicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#
    )
}

fn is_workflow_bundle_valid(bundle_dir: &Path) -> bool {
    let contents = bundle_dir.join("Contents");
    contents.join("Info.plist").is_file() && contents.join("document.wflow").is_file()
}

fn write_workflow_bundle(
    services_dir: &Path,
    bundle_name: &str,
    info_plist: &str,
    document_wflow: &str,
) -> Result<PathBuf, String> {
    let bundle_dir = services_dir.join(bundle_name);
    let contents_dir = bundle_dir.join("Contents");
    fs::create_dir_all(&contents_dir)
        .map_err(|e| format!("Failed to create workflow directory `{}`: {e}", contents_dir.display()))?;

    let info_path = contents_dir.join("Info.plist");
    let wflow_path = contents_dir.join("document.wflow");

    write_atomic(&info_path, info_plist.as_bytes())?;
    write_atomic(&wflow_path, document_wflow.as_bytes())?;
    Ok(bundle_dir)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp = parent.join(format!("{name}.{}.tmp", Uuid::new_v4().simple()));
    if let Err(e) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write `{}`: {e}", path.display()));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to commit `{}`: {e}", path.display()));
    }
    Ok(())
}

pub fn install_cli_symlink_in(dir: &Path, exe: &Path) -> Result<PathBuf, String> {
    if !exe.is_file() {
        return Err(format!(
            "FLIP executable not found at `{}`",
            exe.display()
        ));
    }
    fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create `{}`: {e}", dir.display()))?;
    let dest = dir.join("flip");
    if let Ok(meta) = fs::symlink_metadata(&dest) {
        if !meta.file_type().is_symlink() {
            return Err(format!(
                "Refusing to overwrite existing non-symlink file at `{}`",
                dest.display()
            ));
        }
        fs::remove_file(&dest)
            .map_err(|e| format!("Failed to replace existing symlink `{}`: {e}", dest.display()))?;
    }
    std::os::unix::fs::symlink(exe, &dest)
        .map_err(|e| format!("Failed to create CLI symlink `{}`: {e}", dest.display()))?;
    Ok(dest)
}

fn detect_cli_symlink(dirs_to_check: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs_to_check {
        let candidate = dir.join("flip");
        if fs::symlink_metadata(&candidate).is_ok() && candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn refresh_macos_services_best_effort() {
    let pbs = Path::new("/System/Library/CoreServices/pbs");
    if !pbs.is_file() {
        return;
    }
    let Ok(mut child) = Command::new(pbs)
        .arg("-update")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let start = Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if start.elapsed() >= Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn get_quick_actions_status_in(
    services_dir: &Path,
    cli_dirs: &[PathBuf],
    app_path: Option<&Path>,
    cli_exe: Option<&Path>,
) -> QuickActionsStatus {
    let _ = app_path;
    let convert_path = services_dir.join(CONVERT_WORKFLOW_NAME);
    let new_file_path = services_dir.join(NEW_FILE_WORKFLOW_NAME);
    QuickActionsStatus {
        services_dir: services_dir.to_string_lossy().into_owned(),
        convert_installed: is_workflow_bundle_valid(&convert_path),
        new_file_installed: is_workflow_bundle_valid(&new_file_path),
        convert_path: convert_path.to_string_lossy().into_owned(),
        new_file_path: new_file_path.to_string_lossy().into_owned(),
        cli_executable: cli_exe.map(|p| p.to_string_lossy().into_owned()),
        cli_symlink_path: detect_cli_symlink(cli_dirs).map(|p| p.to_string_lossy().into_owned()),
    }
}

pub fn get_quick_actions_status() -> QuickActionsStatus {
    let current_exe = std::env::current_exe().ok();
    let app_path = resolve_app_bundle_path(current_exe.as_deref());
    let cli_exe = resolve_cli_executable(current_exe.as_deref());
    let cli_dirs = vec![
        default_cli_symlink_dir(),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    get_quick_actions_status_in(
        &default_services_dir(),
        &cli_dirs,
        app_path.as_deref(),
        cli_exe.as_deref(),
    )
}

pub fn install_quick_actions_in(
    services_dir: &Path,
    app_path: Option<&Path>,
    cli_exe: Option<&Path>,
    install_cli_dir: Option<&Path>,
    refresh_pbs: bool,
) -> Result<QuickActionsStatus, String> {
    let convert_info = render_service_info_plist("Convert with FLIP", &["public.item"]);
    let convert_script = build_convert_workflow_script(app_path, cli_exe);
    let convert_wflow = render_automator_document_wflow(&convert_script);

    let new_info = render_service_info_plist("New File with FLIP", &["public.folder"]);
    let new_script = build_new_file_workflow_script(app_path, cli_exe);
    let new_wflow = render_automator_document_wflow(&new_script);

    write_workflow_bundle(
        services_dir,
        CONVERT_WORKFLOW_NAME,
        &convert_info,
        &convert_wflow,
    )?;
    write_workflow_bundle(
        services_dir,
        NEW_FILE_WORKFLOW_NAME,
        &new_info,
        &new_wflow,
    )?;

    if let Some(cli_dir) = install_cli_dir {
        let target_exe = cli_exe.ok_or_else(|| {
            "Could not locate FLIP executable for CLI symlink creation".to_string()
        })?;
        install_cli_symlink_in(cli_dir, target_exe)?;
    }

    if refresh_pbs {
        refresh_macos_services_best_effort();
    }

    log_event(
        "quick_actions.installed",
        Some(json!({
            "servicesDir": services_dir.to_string_lossy(),
            "cliLinked": install_cli_dir.is_some(),
        })),
    );

    let mut cli_dirs = Vec::new();
    if let Some(d) = install_cli_dir {
        cli_dirs.push(d.to_path_buf());
    }
    cli_dirs.push(default_cli_symlink_dir());
    cli_dirs.push(PathBuf::from("/opt/homebrew/bin"));
    cli_dirs.push(PathBuf::from("/usr/local/bin"));

    Ok(get_quick_actions_status_in(
        services_dir,
        &cli_dirs,
        app_path,
        cli_exe,
    ))
}

pub fn install_quick_actions(install_cli: bool) -> Result<QuickActionsStatus, String> {
    let current_exe = std::env::current_exe().ok();
    let app_path = resolve_app_bundle_path(current_exe.as_deref());
    let cli_exe = resolve_cli_executable(current_exe.as_deref());
    let cli_dir = if install_cli {
        Some(default_cli_symlink_dir())
    } else {
        None
    };
    install_quick_actions_in(
        &default_services_dir(),
        app_path.as_deref(),
        cli_exe.as_deref(),
        cli_dir.as_deref(),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_flip_url_covers_convert_new_wheel_batch_settings_and_utf8() {
        assert_eq!(
            parse_flip_url(
                "flip://convert?file=/Users/test/My%20File%2B1%20%C3%A8.png&file=/tmp/b.jpg&to=webp,pdf"
            )
            .unwrap(),
            FlipUrlAction::Convert {
                files: vec![
                    "/Users/test/My File+1 è.png".to_string(),
                    "/tmp/b.jpg".to_string()
                ],
                formats: vec!["WEBP".to_string(), "PDF".to_string()],
            }
        );

        // Literal `+` in filename must remain `+`
        assert_eq!(
            parse_flip_url("flip://convert?file=/tmp/C++%20notes.txt&to=MD").unwrap(),
            FlipUrlAction::Convert {
                files: vec!["/tmp/C++ notes.txt".to_string()],
                formats: vec!["MD".to_string()],
            }
        );

        // `convert` without `to=` opens Convert wheel
        assert_eq!(
            parse_flip_url("flip://convert?file=/tmp/photo.heic").unwrap(),
            FlipUrlAction::OpenWheel {
                mode: "convert".to_string(),
                files: vec!["/tmp/photo.heic".to_string()],
                folder: None,
            }
        );

        // `new` with kind, folder, rename=false
        assert_eq!(
            parse_flip_url("flip://new?kind=MD&in=/Users/demo/Desktop&rename=false").unwrap(),
            FlipUrlAction::CreateNew {
                kind: Some("md".to_string()),
                folder: Some("/Users/demo/Desktop".to_string()),
                no_rename: true,
            }
        );

        assert_eq!(
            parse_flip_url("flip://wheel?mode=new&folder=/tmp").unwrap(),
            FlipUrlAction::OpenWheel {
                mode: "new".to_string(),
                files: vec![],
                folder: Some("/tmp".to_string()),
            }
        );

        assert_eq!(
            parse_flip_url("flip://batch?files=/tmp/a.png,/tmp/b.png").unwrap(),
            FlipUrlAction::OpenBatch {
                files: vec!["/tmp/a.png".to_string(), "/tmp/b.png".to_string()],
            }
        );

        assert_eq!(
            parse_flip_url("flip://settings").unwrap(),
            FlipUrlAction::OpenSettings
        );

        assert_eq!(
            parse_flip_url("flip://new?kind=template:/Users/Demo/My%20Template.md&no_rename=1")
                .unwrap(),
            FlipUrlAction::CreateNew {
                kind: Some("template:/Users/Demo/My Template.md".to_string()),
                folder: None,
                no_rename: true,
            }
        );

        assert_eq!(
            parse_flip_url("flip://open").unwrap(),
            FlipUrlAction::OpenWheel {
                mode: "auto".to_string(),
                files: vec![],
                folder: None,
            }
        );

        assert!(parse_flip_url("https://example.com").is_err());
        assert!(parse_flip_url("flip://convert?file=%ZZ").is_err());
        assert!(parse_flip_url("flip://unknown_action").is_err());
    }

    #[test]
    fn quick_actions_workflows_and_cli_symlink_install_cleanly_in_temp_dir() {
        let dir = tempdir().unwrap();
        let services_dir = dir.path().join("Services");
        let bin_dir = dir.path().join("bin");
        let fake_app = dir.path().join("FLIP & Co.app");
        let fake_exe = fake_app.join("Contents/MacOS/flip");
        fs::create_dir_all(fake_exe.parent().unwrap()).unwrap();
        fs::write(&fake_exe, b"#!/bin/sh\n").unwrap();

        let status_before = get_quick_actions_status_in(
            &services_dir,
            std::slice::from_ref(&bin_dir),
            Some(&fake_app),
            Some(&fake_exe),
        );
        assert!(!status_before.convert_installed);
        assert!(!status_before.new_file_installed);
        assert!(status_before.cli_symlink_path.is_none());

        let status_after = install_quick_actions_in(
            &services_dir,
            Some(&fake_app),
            Some(&fake_exe),
            Some(&bin_dir),
            false,
        )
        .unwrap();
        assert!(status_after.convert_installed);
        assert!(status_after.new_file_installed);
        assert_eq!(
            status_after.cli_symlink_path.as_deref(),
            Some(bin_dir.join("flip").to_string_lossy().as_ref())
        );

        for name in [CONVERT_WORKFLOW_NAME, NEW_FILE_WORKFLOW_NAME] {
            let info_plist = services_dir.join(name).join("Contents/Info.plist");
            let doc_wflow = services_dir.join(name).join("Contents/document.wflow");
            assert!(info_plist.is_file());
            assert!(doc_wflow.is_file());

            let info_raw = fs::read_to_string(&info_plist).unwrap();
            assert!(info_raw.contains("com.apple.finder"));
            assert!(info_raw.contains("runWorkflowAsService"));

            let doc_raw = fs::read_to_string(&doc_wflow).unwrap();
            assert!(doc_raw.contains("com.apple.RunShellScript"));
            assert!(doc_raw.contains("FLIP &amp; Co.app"));
            assert!(doc_raw.contains("COMMAND_STRING"));

            if Path::new("/usr/bin/plutil").is_file() {
                for plist in [&info_plist, &doc_wflow] {
                    let st = Command::new("/usr/bin/plutil")
                        .arg("-lint")
                        .arg(plist)
                        .status()
                        .unwrap();
                    assert!(st.success(), "plutil -lint failed for {}", plist.display());
                }
            }
        }

        // Refuse to overwrite non-symlink file in cli_dir
        let non_symlink_dir = dir.path().join("occupied_bin");
        fs::create_dir_all(&non_symlink_dir).unwrap();
        fs::write(non_symlink_dir.join("flip"), b"existing tool").unwrap();
        let err = install_cli_symlink_in(&non_symlink_dir, &fake_exe).unwrap_err();
        assert!(err.contains("Refusing to overwrite"));

        // Reinstalling workflows & symlink over existing installation is idempotent
        let status_reinstall = install_quick_actions_in(
            &services_dir,
            Some(&fake_app),
            Some(&fake_exe),
            Some(&bin_dir),
            false,
        )
        .unwrap();
        assert!(status_reinstall.convert_installed);
        assert!(status_reinstall.new_file_installed);
    }
}
