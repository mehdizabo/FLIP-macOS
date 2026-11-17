#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICES_DIR="${HOME}/Library/Services"
CLI_DIR="${HOME}/.local/bin"
INSTALL_CLI=0
REFRESH_PBS=1

usage() {
  cat <<'EOF'
Install FLIP Finder Quick Actions (~/Library/Services) and optional CLI symlink.

Usage:
  ./scripts/install-quick-actions.sh [options]

Options:
  --install-cli         Also symlink `flip` into ~/.local/bin/flip (or --cli-dir)
  --cli-dir <dir>       Target directory for the `flip` CLI symlink (implies --install-cli)
  --services-dir <dir>  Override target Services directory (default: ~/Library/Services)
  --no-pbs              Skip refreshing macOS Services cache via pbs
  -h, --help            Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-cli)
      INSTALL_CLI=1
      shift
      ;;
    --cli-dir)
      [[ $# -ge 2 ]] || { echo "Error: --cli-dir requires a path" >&2; exit 1; }
      CLI_DIR="$2"
      INSTALL_CLI=1
      shift 2
      ;;
    --services-dir)
      [[ $# -ge 2 ]] || { echo "Error: --services-dir requires a path" >&2; exit 1; }
      SERVICES_DIR="$2"
      shift 2
      ;;
    --no-pbs)
      REFRESH_PBS=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

APP_PATH=""
for candidate in \
  "/Applications/FLIP.app" \
  "${HOME}/Applications/FLIP.app" \
  "$ROOT_DIR/src-tauri/target/release/bundle/macos/FLIP.app"
do
  if [[ -d "$candidate" ]]; then
    APP_PATH="$candidate"
    break
  fi
done

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="/Applications/FLIP.app"
fi

CLI_BIN=""
for candidate in \
  "$APP_PATH/Contents/MacOS/flip" \
  "/Applications/FLIP.app/Contents/MacOS/flip" \
  "${HOME}/Applications/FLIP.app/Contents/MacOS/flip" \
  "$ROOT_DIR/src-tauri/target/release/flip" \
  "$ROOT_DIR/src-tauri/target/debug/flip"
do
  if [[ -x "$candidate" ]]; then
    CLI_BIN="$candidate"
    break
  fi
done

if [[ -z "$CLI_BIN" ]]; then
  CLI_BIN="/Applications/FLIP.app/Contents/MacOS/flip"
fi

xml_escape() {
  printf '%s' "$1" | /usr/bin/sed \
    -e 's/&/\&amp;/g' \
    -e 's/</\&lt;/g' \
    -e 's/>/\&gt;/g' \
    -e 's/"/\&quot;/g' \
    -e "s/'/\\&apos;/g"
}

zsh_single_quote() {
  local s="$1"
  local sq="'\\''"
  printf "'%s'" "${s//\'/$sq}"
}

APP_HINT_Q="$(zsh_single_quote "$APP_PATH")"
BIN_HINT_Q="$(zsh_single_quote "$CLI_BIN")"

CONVERT_SCRIPT="$(cat <<EOF
#!/bin/zsh
set -euo pipefail

APP_HINT=$APP_HINT_Q
BIN_HINT=$BIN_HINT_Q

pct_encode() {
  setopt localoptions nomultibyte
  local raw="\$1" out="" i ch hex
  local LC_ALL=C
  for (( i = 1; i <= \$#raw; i++ )); do
    ch="\${raw[i]}"
    case "\$ch" in
      [A-Za-z0-9._~/-]) out+="\$ch" ;;
      *)
        printf -v hex '%%%02X' "'\$ch"
        out+="\$hex"
        ;;
    esac
  done
  printf '%s' "\$out"
}

URL="flip://wheel?mode=convert"
for f in "\$@"; do
  [ -n "\$f" ] || continue
  URL+="&file=\$(pct_encode "\$f")"
done

APP_PATH=""
if [ -d "\$APP_HINT" ]; then
  APP_PATH="\$APP_HINT"
elif [ -d "/Applications/FLIP.app" ]; then
  APP_PATH="/Applications/FLIP.app"
elif [ -d "\$HOME/Applications/FLIP.app" ]; then
  APP_PATH="\$HOME/Applications/FLIP.app"
else
  FOUND="\$(/usr/bin/mdfind "kMDItemCFBundleIdentifier == 'com.thetinyfoundry.flip.mac'" 2>/dev/null | /usr/bin/head -n 1 || true)"
  if [ -n "\$FOUND" ] && [ -d "\$FOUND" ]; then
    APP_PATH="\$FOUND"
  fi
fi

if [ -n "\$APP_PATH" ]; then
  /usr/bin/open -a "\$APP_PATH" "\$URL"
elif [ -x "\$BIN_HINT" ]; then
  "\$BIN_HINT" "\$URL" >/dev/null 2>&1 &
else
  /usr/bin/open -b "com.thetinyfoundry.flip.mac" "\$URL"
fi
EOF
)"

NEW_FILE_SCRIPT="$(cat <<EOF
#!/bin/zsh
set -euo pipefail

APP_HINT=$APP_HINT_Q
BIN_HINT=$BIN_HINT_Q

pct_encode() {
  setopt localoptions nomultibyte
  local raw="\$1" out="" i ch hex
  local LC_ALL=C
  for (( i = 1; i <= \$#raw; i++ )); do
    ch="\${raw[i]}"
    case "\$ch" in
      [A-Za-z0-9._~/-]) out+="\$ch" ;;
      *)
        printf -v hex '%%%02X' "'\$ch"
        out+="\$hex"
        ;;
    esac
  done
  printf '%s' "\$out"
}

TARGET_FOLDER=""
if [ "\$#" -gt 0 ] && [ -n "\$1" ]; then
  if [ -d "\$1" ]; then
    TARGET_FOLDER="\$1"
  else
    TARGET_FOLDER="\${1:h}"
  fi
fi

URL="flip://wheel?mode=new"
if [ -n "\$TARGET_FOLDER" ]; then
  URL+="&folder=\$(pct_encode "\$TARGET_FOLDER")"
fi

APP_PATH=""
if [ -d "\$APP_HINT" ]; then
  APP_PATH="\$APP_HINT"
elif [ -d "/Applications/FLIP.app" ]; then
  APP_PATH="/Applications/FLIP.app"
elif [ -d "\$HOME/Applications/FLIP.app" ]; then
  APP_PATH="\$HOME/Applications/FLIP.app"
else
  FOUND="\$(/usr/bin/mdfind "kMDItemCFBundleIdentifier == 'com.thetinyfoundry.flip.mac'" 2>/dev/null | /usr/bin/head -n 1 || true)"
  if [ -n "\$FOUND" ] && [ -d "\$FOUND" ]; then
    APP_PATH="\$FOUND"
  fi
fi

if [ -n "\$APP_PATH" ]; then
  /usr/bin/open -a "\$APP_PATH" "\$URL"
elif [ -x "\$BIN_HINT" ]; then
  "\$BIN_HINT" "\$URL" >/dev/null 2>&1 &
else
  /usr/bin/open -b "com.thetinyfoundry.flip.mac" "\$URL"
fi
EOF
)"

write_workflow() {
  local bundle_name="$1"
  local menu_title="$2"
  local send_type="$3"
  local script_body="$4"

  local contents_dir="$SERVICES_DIR/$bundle_name/Contents"
  mkdir -p "$contents_dir"

  local title_xml
  title_xml="$(xml_escape "$menu_title")"
  local type_xml
  type_xml="$(xml_escape "$send_type")"
  local script_xml
  script_xml="$(xml_escape "$script_body")"

  local uuid_action uuid_in uuid_out
  uuid_action="$(uuidgen | tr '[:lower:]' '[:upper:]')"
  uuid_in="$(uuidgen | tr '[:lower:]' '[:upper:]')"
  uuid_out="$(uuidgen | tr '[:lower:]' '[:upper:]')"

  cat > "$contents_dir/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
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
				<string>$title_xml</string>
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
				<string>$type_xml</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
EOF

  cat > "$contents_dir/document.wflow" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
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
					<string>$script_xml</string>
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
				<string>$uuid_in</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
					<string>Unix</string>
				</array>
				<key>OutputUUID</key>
				<string>$uuid_out</string>
				<key>UUID</key>
				<string>$uuid_action</string>
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
EOF

  if [[ -x /usr/bin/plutil ]]; then
    /usr/bin/plutil -lint "$contents_dir/Info.plist" >/dev/null
    /usr/bin/plutil -lint "$contents_dir/document.wflow" >/dev/null
  fi
}

echo "==> Installing Finder Quick Actions to $SERVICES_DIR"
write_workflow "Convert with FLIP.workflow" "Convert with FLIP" "public.item" "$CONVERT_SCRIPT"
write_workflow "New File with FLIP.workflow" "New File with FLIP" "public.folder" "$NEW_FILE_SCRIPT"

if [[ "$REFRESH_PBS" -eq 1 ]] && [[ -x /System/Library/CoreServices/pbs ]]; then
  /System/Library/CoreServices/pbs -update >/dev/null 2>&1 || true
fi

if [[ "$INSTALL_CLI" -eq 1 ]]; then
  if [[ ! -x "$CLI_BIN" ]]; then
    echo "Error: FLIP CLI binary not found/executable at $CLI_BIN" >&2
    exit 1
  fi
  mkdir -p "$CLI_DIR"
  DEST_CLI="$CLI_DIR/flip"
  if [[ -e "$DEST_CLI" && ! -L "$DEST_CLI" ]]; then
    echo "Error: Refusing to overwrite non-symlink file at $DEST_CLI" >&2
    exit 1
  fi
  ln -sfn "$CLI_BIN" "$DEST_CLI"
  echo "==> Linked CLI: $DEST_CLI -> $CLI_BIN"
  case ":$PATH:" in
    *":$CLI_DIR:"*) ;;
    *)
      echo "Note: Add $CLI_DIR to your PATH:"
      echo "  export PATH=\"$CLI_DIR:\$PATH\""
      ;;
  esac
fi

echo "Installed:"
echo "  - $SERVICES_DIR/Convert with FLIP.workflow"
echo "  - $SERVICES_DIR/New File with FLIP.workflow"
