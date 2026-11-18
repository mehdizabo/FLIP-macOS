# FLIP for macOS

**100% local Liquid Glass radial file converter & Create New File utility for macOS Finder.**

[Italiano (README.it.md)](README.it.md)

FLIP brings a fast, cursor-centered radial actions wheel (inspired by Logi Options+ Actions Ring) directly to macOS Finder and Desktop. Convert images, PDFs, documents, structured data, audio, video, and Apple iWork files **without ever overwriting originals or uploading anything to the cloud** — and create new files or custom templates directly inside your active Finder folder with instant inline rename.

---

## Features

- **Enhanced Liquid Glass Radial Wheel**: Translucent layered glass sectors, directional specular rim highlights, spring transitions, QuickLook center hub thumbnail, `Convert` / `New File` mode pill (`Tab`), radial progress arc with percentage readout, **VoiceOver live announcer**, acoustic tick (`flip-tick.wav`), and Force Touch trackpad haptics.
- **Accessory Menu Bar App**: Lives cleanly in the macOS menu bar (`LSUIElement`) with zero Dock clutter (`Open FLIP Wheel`, `New File in Finder Folder…`, `Batch / Multiple Formats…`, `Settings & Shortcuts…`, `Open Templates Folder`, `Reveal Diagnostic Logs`, `Quit FLIP`).
- **Batch & Multi-Format Converter**: Convert multiple files into one or more output formats simultaneously (`Shift+Click` any sector, click `Multi` in the hub, or open from the menu bar).
- **Zero-Overwrite Naming**:
  - **Conversions** directly inherit the original filename stem (`photo.heic` → `photo.png`). On collision, FLIP appends a Finder-style counter (`photo 2.png`, `photo 3.png`). Multi-page exports use `<stem>-p001.<ext>`. Original files are never modified or overwritten, and inline rename is **not** triggered on conversions.
  - **Atomic Staging & Safety Guards**: Every job executes inside an isolated `.flip-job-<uuid>` folder, validates outputs before committing with `renamex_np(RENAME_EXCL)`, rejects symlinks/Finder aliases/undownloaded iCloud stubs/network volumes, caps batches at 128 files, blocks > 40 MP image bombs and > 256 MiB / 4,096-entry ZIP containers, and strips external/local-file inclusions from offline HTML imports.

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

- **Built-in Kinds**: Folder (`untitled folder`), Plain Text (`.txt`), Markdown (`.md`), Rich Text (`.rtf`), Word Document (`.docx`), Spreadsheet (`.xlsx`), Presentation (`.pptx`), HTML (`.html`), JSON (`.json`), CSV (`.csv`), executable Shell Script (`.sh`, `chmod +x`), Python (`.py`), Swift (`.swift`).
- **From Clipboard**: Inspects `NSPasteboard.general` and creates `.png`, `.pdf`, `.rtf`, `.webloc` URL bookmark, or `.txt` as `Clipboard <timestamp>.<ext>`.
- **Apple iWork**: Blank `.pages`, `.numbers`, and `.key` files when Pages, Numbers, or Keynote are installed.
- **Custom User Templates**: Drop any file into `~/Library/Application Support/FLIP/Templates` (subfolders become groups). Plain-text templates automatically expand `{{date}}`, `{{time}}`, `{{user}}`, and `{{folder}}`.

---

## Supported Conversions & Apple Integration

| Family | Inputs | Outputs | Engines |
|---|---|---|---|
| **Images** | `PNG`, `JPG/JPEG`, `WEBP`, `HEIC/HEIF`, `AVIF`, `TIFF`, `GIF`, `BMP`, `ICNS`, `ICO` | `PNG`, `JPG`, `WEBP`, `HEIC`, `PDF`, `TIFF`, `GIF`, `BMP`, plus `Combine into PDF` | Native `ImageIO` (EXIF orientation & optional GPS strip) + Rust `image` + `PDFKit` |
| **PDF** | `PDF` | `PNG`, `JPG`, `WEBP`, `HEIC`, `TIFF` (144/200/300 DPI), `TXT` (with **Apple Vision** OCR for scanned pages), `DOCX`, `MD`, `HTML`, `RTF`, `ODT`, `EPUB` (native EPUB 3) (+ `FB2`/`RST` via Pandoc), `Merge PDFs` | Native `PDFKit` + `Vision` (`VNRecognizeTextRequest`) + pure-Rust EPUB 3 writer |
| **Documents** | `TXT`, `MD`, `HTML`, `RTF`, `DOC`, `DOCX`, `ODT`, `EPUB` (+ `FB2`, `RST` via Pandoc) | `PDF` (A4/Letter paginated), `DOCX`, `MD`, `HTML`, `TXT`, `RTF`, `ODT`, `EPUB` (native EPUB 3), `PNG`, `JPG`, `WEBP`, `HEIC` (+ `FB2`/`RST` via Pandoc) | Native `NSAttributedString` / CoreText + `pulldown-cmark` + pure-Rust EPUB 3 + optional `Pandoc` |
| **Apple iWork** | `.pages`, `.numbers`, `.key` | `PDF` (+ `DOCX`/`RTF`/`TXT`/`EPUB` for Pages, `XLSX`/`CSV` for Numbers, `PPTX` for Keynote) | QuickLook preview + AppleScript app automation |
| **Structured Data** | `JSON`, `YAML/YML`, `TOML`, `XML`, `CSV` | `JSON`, `YAML`, `TOML`, `XML`, `CSV` (nested dot-flattening), `TXT`, `MD`, `HTML`, `PDF` | Native Rust parsers + CoreText PDF |
| **Audio** | `MP3`, `WAV`, `M4A`, `AAC`, `AIFF`, `ALAC`, `CAF`, `FLAC`, `OGG`, `OPUS`, `WMA` | Native: `M4A`, `WAV`, `AIFF`, `ALAC` · With FFmpeg: `MP3`, `M4A`, `WAV`, `FLAC`, `AIFF`, `ALAC`, `OGG`, `OPUS`, `AAC` | Native `AVFoundation` + `afconvert` + optional `FFmpeg` |
| **Video** | `MP4`, `MOV`, `M4V`, `MKV`, `WEBM`, `AVI`, `WMV` | Native: `MP4`, `MOV`, `M4V`, `M4A`, `WAV`, animated `GIF` · With FFmpeg: + `MKV`, `WEBM`, `AVI`, `MP3` | Native `AVFoundation` + optional `FFmpeg` (real-time progress & `Esc` cancel) |

> **Note on Optional Tools**: FFmpeg and Pandoc are **not** bundled inside FLIP. Install them via Homebrew (`brew install ffmpeg pandoc`) or configure custom paths in **Settings → Engines & Permissions** to unlock extra formats (`MKV`/`WebM`/`MP3`/`FLAC`/`OGG` with FFmpeg, and `FB2`/`RST` books with Pandoc — **EPUB 3 is built-in natively with zero Pandoc required**).

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

### Production `.app` Bundle & Ad-Hoc Signing

Build the native Swift bridge, Vite frontend, release `.app` bundle, verify embedded resources (`flip-macos-bridge` with `+x` and `flip-tick.wav`), and ad-hoc sign + strictly verify in one command:

```bash
./scripts/build-macos.sh
```

The signed bundle is output to:

```text
src-tauri/target/release/bundle/macos/FLIP.app
```

You can also sign or re-sign an existing bundle with `./scripts/sign-local.sh`:

```bash
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
cd src-tauri && cargo test
```

---

## Architecture Overview

- `native-bridge/` — Swift CLI & daemon helper (`flip-macos-bridge`, compiled via `/usr/bin/swiftc`) handling real-time `NSPasteboard(name: .drag)` monitoring, `CGEventTap`/`NSEvent` triggers, Finder Apple Events, `QuickLookThumbnailing`, `PDFKit`, `Vision` OCR, `NSAttributedString` A4/Letter PDF pagination, `AVFoundation`, `NSSound`, and `NSHapticFeedbackManager`.
- `src-tauri/` — Rust core policies, atomic `.flip-job-<uuid>` staging, conversion engines, `tauri::tray::TrayIconBuilder` accessory host, multi-monitor clamping, and Tauri v2 IPC commands.
- `src/` — Vite + TypeScript Liquid Glass radial wheel (`#wheel` with VoiceOver live announcer), Settings & Shortcut/Format Order editor (`#settings`), Batch & Multi-Format converter (`#batch`), and Onboarding wizard (`#onboarding`).
- `docs/PORTING-NOTES.md` — Module-by-module mapping from `FLIP-windows/src/Flip/*.cs` and macOS deltas.
- `ACCEPTANCE.md` — Physical desktop GUI QA checklist for macOS releases.

---

## License

MIT License — see [LICENSE](LICENSE).
