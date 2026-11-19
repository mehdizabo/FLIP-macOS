# FLIP for macOS

**100% local Liquid Glass radial file converter & Create New File utility for macOS Finder.**

[Italiano (README.it.md)](README.it.md)

FLIP brings a fast, cursor-centered radial actions wheel (inspired by Logi Options+ Actions Ring) directly to macOS Finder and Desktop. Convert images, PDFs, documents, structured data, audio, video, and Apple iWork files **without ever overwriting originals or uploading anything to the cloud** — and create new files or custom templates directly inside your active Finder folder with instant inline rename.

---

## Features

- **Enhanced Liquid Glass Radial Wheel**: Translucent layered glass sectors, directional specular rim highlights, spring transitions, QuickLook center hub thumbnail, `Convert` / `New File` mode pill (`Tab`), radial progress arc with percentage readout, **VoiceOver live announcer**, **English / Italiano (`EN` / `IT`) UI switcher** (`Settings → General`), acoustic tick (`flip-tick.wav`), Force Touch trackpad haptics, and a live **Test Wheel Sound & Haptics** preview button.
- **Accessory Menu Bar App & Single-Instance IPC**: Lives cleanly in the macOS menu bar (`LSUIElement`) with zero Dock clutter (`Open FLIP Wheel`, `New File in Finder Folder…`, `Batch / Multiple Formats…`, `Settings & Shortcuts…`, `Open Templates Folder`, `Reveal Diagnostic Logs`, `Quit FLIP`). Enforces single-instance exclusivity via `flock` + local Unix domain socket IPC (`flip-instance.sock`, mode `0600`) so Finder Quick Actions, `flip://` URLs, and CLI control commands drive the active instance seamlessly.
- **Batch & Multi-Format Converter**: Convert multiple files into one or more output formats simultaneously (`Shift+Click` any sector, click `Multi` in the hub, or open from the menu bar).
- **Zero-Overwrite Naming**:
  - **Conversions** directly inherit the original filename stem (`photo.heic` → `photo.png`). On collision, FLIP appends a Finder-style counter (`photo 2.png`, `photo 3.png`). Multi-page exports use `<stem>-p001.<ext>`. Original files are never modified or overwritten, and inline rename is **not** triggered on conversions.
  - **Atomic Staging & Safety Guards**: Every job executes inside an isolated `.flip-job-<uuid>` folder, validates outputs before committing with `renamex_np(RENAME_EXCL)`, rejects symlinks/Finder aliases/undownloaded iCloud stubs/network volumes, caps batches at 128 files, blocks > 40 MP image bombs, > 200-page PDF optimization bombs, and > 256 MiB / 4,096-entry ZIP / Zip-Slip containers, and strips external/local-file inclusions from offline HTML and SVG inputs.

---

## Triggers

| Mode | Default Trigger | How It Works |
|---|---|---|
| **Drag + Keyboard** | `Down Arrow` (configurable) | Start dragging file(s) from Finder/Desktop and press `Down` (or hold `Down` first, then drag). Drop directly onto a compatible format sector. Reads `NSPasteboard(name: .drag)` in real time before drop. |
| **Idle + Keyboard** | `Option+Shift+Space` (configurable) | Opens at your cursor anywhere on macOS. If Finder has selected files, opens in **Convert** mode; otherwise opens in **New File** mode targeting the active Finder folder (`~/Desktop` fallback). Press `Tab` to switch modes. |
| **Mouse Button (Optional)** | Disabled (optional `Btn 2` / `3` / `4`) | Hold-and-release support for middle or side mouse buttons (Logi / multi-button mice). |

Wheel navigation also supports arrow keys, number keys `1`–`8`, `Enter`/`Space`, `Tab` (switch Convert / New File), `Esc` (cancel active conversion or dismiss), and 300 ms drag-dwell over `More ▸` / `Back ◂` when more than 7 formats exist.

---

## Create New File

macOS Finder has no built-in "New File" menu. FLIP adds instant creation in the frontmost Finder folder (falling back to `~/Desktop` or a custom folder in Settings), selects the item in Finder, and **immediately triggers inline rename (`Return`)** so you can type the final name right away:

- **Built-in Kinds**: Folder (`untitled folder`), Plain Text (`.txt`), Markdown (`.md`), Rich Text (`.rtf`), Word Document (`.docx`), Spreadsheet (`.xlsx`), Presentation (`.pptx`), Blank PDF A4 (`.pdf`), HTML (`.html`), JSON (`.json`), CSV (`.csv`), YAML Config (`.yaml`), TOML Config (`.toml`), XML Document (`.xml`), executable Shell Script (`.sh`, `chmod +x`), Python (`.py`), Swift (`.swift`).
- **From Clipboard**: Inspects `NSPasteboard.general` and creates `.png`, `.pdf`, `.rtf`, `.webloc` URL bookmark, or `.txt` as `Clipboard <timestamp>.<ext>`.
- **Apple iWork**: Blank `.pages`, `.numbers`, and `.key` files when Pages, Numbers, or Keynote are installed.
- **Custom User Templates**: Drop any file into `~/Library/Application Support/FLIP/Templates` (subfolders become groups). Plain-text templates automatically expand `{{date}}`, `{{time}}`, `{{user}}`, and `{{folder}}`.

---

## Supported Conversions & Apple Integration

| Family | Inputs | Outputs | Engines |
|---|---|---|---|
| **Images** | `PNG`, `JPG/JPEG`, `WEBP`, `HEIC/HEIF`, `AVIF`, `TIFF`, `GIF`, `BMP`, `ICNS`, `ICO`, `SVG` (offline-sanitized) | `PNG`, `JPG`, `WEBP`, `HEIC`, `PDF`, `TIFF`, `GIF`, `BMP`, `ICO`, `ICNS`, `TXT` (OCR), `REMOVE_BG`, `ZIP`, plus `COMBINE_PDF` & `ANIMATE_GIF` (`GIF+`) | Native `ImageIO` (EXIF orientation & optional GPS strip) + `Vision` (`VNGenerateForegroundInstanceMaskRequest` `REMOVE_BG`) + Rust `image` + `PDFKit` |
| **PDF** | `PDF` | `PNG`, `JPG`, `WEBP`, `HEIC`, `TIFF` (144/200/300 DPI), `TXT` (with **Apple Vision** OCR for scanned pages), `DOCX`, `MD`, `HTML`, `RTF`, `ODT`, `EPUB` (native EPUB 3), `FB2`, `RST` (native zero-Pandoc or Pandoc), `OPTIMIZE_PDF` (text-preserving PDF compression), `ZIP`, `Merge PDFs` | Native `PDFKit` / Quartz + `Vision` (`VNRecognizeTextRequest`) + pure-Rust EPUB 3 / FB2 / RST writers |
| **Documents** | `TXT`, `MD`, `HTML`, `RTF`, `DOC`, `DOCX`, `ODT`, `EPUB`, `FB2`, `RST` | `PDF` (A4/Letter paginated), `DOCX`, `MD`, `HTML`, `TXT`, `RTF`, `ODT`, `EPUB` (native EPUB 3), `FB2`, `RST` (native zero-Pandoc or Pandoc), `PNG`, `JPG`, `WEBP`, `HEIC`, `ZIP` | Native `NSAttributedString` / CoreText + `pulldown-cmark` + pure-Rust EPUB 3 / FB2 / RST + optional `Pandoc` |
| **Apple iWork** | `.pages`, `.numbers`, `.key` | `PDF`, `ZIP` (+ `DOCX`/`RTF`/`TXT`/`EPUB` for Pages, `XLSX`/`CSV` for Numbers, `PPTX` for Keynote) | QuickLook preview + AppleScript app automation |
| **Structured Data** | `JSON`, `YAML/YML`, `TOML`, `XML`, `CSV`, `XLSX` | `JSON`, `YAML`, `TOML`, `XML`, `CSV` (nested dot-flattening), `XLSX` (native OOXML), `TXT`, `MD`, `HTML`, `PDF`, `ZIP` | Native Rust parsers + pure-Rust `XLSX` OOXML reader/writer + CoreText PDF |
| **Audio** | `MP3`, `WAV`, `M4A`, `AAC`, `AIFF`, `ALAC`, `CAF`, `FLAC`, `OGG`, `OPUS`, `WMA` | **Native (zero FFmpeg)**: `M4A`, `WAV`, `FLAC`, `AIFF`, `ALAC`, `CAF` · With FFmpeg: + `MP3`, `OGG`, `OPUS`, `AAC`, plus `ZIP` | Native `AVFoundation` + CoreAudio `ExtAudioFile` + `/usr/bin/afconvert` + optional `FFmpeg` |
| **Video** | `MP4`, `MOV`, `M4V`, `MKV`, `WEBM`, `AVI`, `WMV` | Native: `MP4`, `MOV`, `M4V`, `M4A`, `WAV`, animated `GIF`, `ZIP` · With FFmpeg: + `MKV`, `WEBM`, `AVI`, `MP3` | Native `AVFoundation` + optional `FFmpeg` (real-time progress & `Esc` cancel) |
| **Archives** | `ZIP` (and any single/multi-file selection) | `UNZIP` (safe subfolder extraction) · `ZIP` (collision-safe archive creation) | Pure-Rust `zip` engine (`archive.rs` with Zip-Slip & >256 MiB / 4,096-entry guards) |

> **Note on Optional Tools**: FFmpeg and Pandoc are **not** bundled inside FLIP. **EPUB 3, `FB2`, `RST` books, and lossless `CAF`/`FLAC` audio conversions work out of the box with zero FFmpeg and zero Pandoc installed.** Install `brew install ffmpeg pandoc` (or set custom paths in **Settings → Engines & Permissions**) to unlock extra media formats (`MKV`/`WebM`/`AVI`/`MP3`/`OGG`/`OPUS`/`AAC` with FFmpeg) or prefer Pandoc when available. Set `FLIP_DISABLE_FFMPEG=1` or `FLIP_DISABLE_PANDOC=1` in the environment to force built-in native engines only.

---

## CLI & Control Commands

FLIP ships an integrated CLI inside `FLIP.app/Contents/MacOS/flip` (linkable to `~/.local/bin/flip` via **Settings → Engines & Permissions**):

```bash
flip                                          # Launch menu bar host (single-instance)
flip convert <file...> --to <FMT[,FMT2]>      # Local conversion with JSON report
flip new <kind> [--in <folder>] [--no-rename] # Create new file/folder in target folder
flip wheel [--mode convert|new] [files...]    # Open radial wheel via Unix socket IPC
flip settings                                 # Open FLIP Settings window
flip batch [files...]                         # Open Batch & Multi-Format Converter
flip preview --out <path.svg> [--files f1,f2] [--mode convert|new] [--hover N]
                                              # Render standalone radial wheel SVG preview
flip quit                                     # Quit active menu bar instance cleanly
flip --help                                   # Show CLI usage
flip --version                                # Show version
```

---

## macOS Permissions

- **Accessibility** (`Privacy & Security → Accessibility`): Needed so FLIP's `CGEventTap` can intercept your drag trigger key *only* while dragging files (without interfering with normal typing) and send `Return` to trigger Finder inline rename after creating a new file.
- **Finder Automation** (`Privacy & Security → Automation → FLIP → Finder`): Needed to inspect your selected Finder files and active Finder insertion folder.
- **Limited Mode**: If permissions are not granted, FLIP still works cleanly using your Idle hotkey (`Option+Shift+Space`), drag-and-drop directly onto the wheel or Batch window, and `~/Desktop` fallback for New File creation.

---

## Local Build & Run

### Prerequisites

- **macOS 15.0+** (Apple Silicon; designed and tested on **macOS 27.2.1 Golden Gate**)
- **Apple Command Line Tools** (`xcode-select --install`) — full `Xcode.app` is **not** required (`/usr/bin/swiftc` and `/usr/bin/clang` are used directly)
- **Rust toolchain** (`cargo`) and `tauri-cli` (`cargo install tauri-cli --version ^2`)
- **Node.js / npm** (and optional **Bun** for fast frontend unit tests)
- Optional: `brew install ffmpeg pandoc`

### Development Mode

```bash
npm install
./native-bridge/build.sh
cargo tauri dev
```

### Production `.app` & `.dmg` Bundle + Ad-Hoc Signing

Build the native Swift bridge, Vite frontend, release `.app` bundle, verify embedded resources (`flip-macos-bridge` with `+x` and `flip-tick.wav`), and ad-hoc sign + strictly verify in one command (append `--dmg` to also package and sign the compressed Apple Silicon `.dmg` installer):

```bash
./scripts/build-macos.sh --dmg
```

Signed outputs are written to:

```text
src-tauri/target/release/bundle/macos/FLIP.app
src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg
```

You can also install or uninstall `FLIP.app` locally (copies to `/Applications` or `~/Applications`, clears `com.apple.quarantine`, installs/removes Finder Quick Actions, and manages `~/.local/bin/flip`), package/update the Homebrew Cask SHA-256 (`packaging/homebrew/flip.rb`), audit repository privacy, export a clean source archive, or re-sign with `./scripts/sign-local.sh`:

```bash
# Install built FLIP.app + Quick Actions + ~/.local/bin/flip symlink
./scripts/install-local.sh

# Cleanly uninstall FLIP.app + Quick Actions + ~/.local/bin/flip symlink
./scripts/install-local.sh --uninstall

# Repackage DMG and sync SHA-256 into packaging/homebrew/flip.rb
./scripts/package-dmg.sh --update-cask

# Audit tracked/untracked files for secrets/personal paths and export source-only ZIP
./scripts/audit-repository.sh
./scripts/export-source.sh

# Default ad-hoc signature (no Apple Developer account required)
./scripts/sign-local.sh

# Optional Developer ID signing + notarization
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
APPLE_NOTARY_PROFILE="notary-profile" \
./scripts/sign-local.sh
```

### Running Tests

```bash
./native-bridge/build.sh
npx tsc --noEmit
npm run build
bun test
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

See [`TEST_RESULTS.md`](TEST_RESULTS.md) for verified unit, integration, 50-cycle stress, latency benchmark, and release bundle test outputs.

---

## Architecture Overview

- `native-bridge/` — Swift CLI & daemon helper (`flip-macos-bridge`, compiled via `/usr/bin/swiftc`) handling real-time `NSPasteboard(name: .drag)` monitoring, `CGEventTap`/`NSEvent` triggers, Finder Apple Events, `QuickLookThumbnailing`, `PDFKit` & Quartz `OPTIMIZE_PDF`, `Vision` (`VNRecognizeTextRequest` OCR & `VNGenerateForegroundInstanceMaskRequest` `REMOVE_BG`), `NSAttributedString` A4/Letter PDF pagination, `AVFoundation` / CoreAudio `ExtAudioFile` + `/usr/bin/afconvert` zero-FFmpeg `CAF`/`FLAC`, `NSSound`, and `NSHapticFeedbackManager`.
- `src-tauri/` — Rust core policies (`policy.rs`), preferences (`en`/`it`), atomic `.flip-job-<uuid>` staging, conversion engines (`archive.rs` `ZIP`/`UNZIP`, `document.rs` native EPUB 3 / FB2 / RST, `image.rs` incl. `ICO` & `ANIMATE_GIF`, `data.rs`, `media.rs`), single-instance `flock` + Unix domain socket IPC (`ipc_socket.rs`), `tauri::tray::TrayIconBuilder` accessory host, multi-monitor clamping, Tauri v2 IPC commands, `flip preview` SVG generator (`cli.rs`), and `tests/stress_and_perf.rs`.
- `src/` — Vite + TypeScript Liquid Glass radial wheel (`#wheel` with VoiceOver live announcer), bilingual `src/i18n.ts` (`EN` / `IT`), Settings & Shortcut/Format Order/Feedback Test editor (`#settings`), Batch & Multi-Format converter (`#batch`), Onboarding wizard (`#onboarding`), and automated DOM E2E suite (`src/views.e2e.test.ts`).
- `integrations/raycast/` — Ready-to-use Raycast Script Commands (`flip-convert.sh`, `flip-new-file.sh`, `flip-wheel.sh`) driving `flip` CLI & Unix socket IPC.
- `packaging/homebrew/flip.rb` — Homebrew Cask manifest for `FLIP-<version>-macOS-arm64.dmg` synced via `scripts/package-dmg.sh --update-cask`.
- `scripts/audit-repository.sh` & `scripts/export-source.sh` — Pre-release privacy/secret/path scanner and clean `git archive` source ZIP exporter.
- `ARCHITECTURE.md`, `CONTRIBUTING.md`, `ROADMAP.md`, `PUBLIC_RELEASE_REVIEW.md`, `CHANGELOG.md` — Root governance, architecture, roadmap, and pre-release readiness docs.
- `docs/PORTING-NOTES.md` — Module-by-module mapping from `FLIP-windows/src/Flip/*.cs` and macOS deltas.
- `ACCEPTANCE.md` — Physical desktop GUI QA checklist for macOS releases.
- `TEST_RESULTS.md` — Recorded toolchain versions, stress/latency metrics, and `.app`/`.dmg`/CLI verification results.

---

## License

MIT License — see [LICENSE](LICENSE).
