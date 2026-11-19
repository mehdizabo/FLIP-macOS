# Architecture

FLIP for macOS is an Apple Silicon (`arm64`) desktop utility built with **Tauri v2**, a **Rust** core/conversion runtime (`src-tauri/`), a **Vite + TypeScript** frontend (`src/`), and a **Swift/AppKit** native system bridge (`native-bridge/`, compiled via Command Line Tools `/usr/bin/swiftc` into `flip-macos-bridge`). `src-tauri/target/` and `dist/` hold local build artifacts; `_local/` is ignored for private development notes/evidence.

## 1. Host, Single-Instance IPC & Window Surfaces

- **Accessory Menu Bar Host (`src-tauri/src/host/`)**: Runs as an `LSUIElement` menu bar application (`TrayIconBuilder`) with no Dock icon. Single-instance exclusivity uses `flock` on a lock file plus an owner-only (`0600`) Unix domain socket (`flip-instance.sock`, with stale-socket recovery) so duplicate GUI launches, `flip://` URL scheme events, Finder Quick Actions, Raycast scripts, and CLI control commands (`flip wheel`, `flip settings`, `flip batch`, `flip quit`) route cleanly to the active host.
- **Tauri v2 Windows (`src-tauri/tauri.conf.json`, `src/`)**:
  - `#wheel`: `440x440` transparent, always-on-top, non-focus-stealing (`visibleOnAllWorkspaces: true`) Liquid Glass radial wheel overlay (`src/views/WheelView.ts`, `src/wheelGeometry.ts`, `src/styles/wheel.css`). Shared sector geometry drives both SVG rendering and hit-testing with a `68 px` center QuickLook hub dead-zone, `210 px` outer cancel radius, `3°` angular hysteresis, and `<= 8` sectors per page (`More ▸` / `Back ◂` with `300 ms` drag-dwell).
  - `#settings`: Multi-tab Settings & Shortcuts recorder (`General` with live `EN`/`IT` switcher and `Test Wheel Sound & Haptics`, `Shortcuts`, `Formats & Quality` with interactive Preferred Format Order, `New File & Templates`, `Engines & Permissions`, `About & Logs`).
  - `#batch`: Multi-file & multi-format queue (`src/views/BatchView.ts`) supporting `Shift`/`Option`-click handoff and cancellation.
  - `#onboarding`: First-run permissions and workflow wizard (`src/views/OnboardingView.ts`).
- **Multi-Monitor & Mixed-DPI Clamping**: `src-tauri/src/host/screen.rs` and Cocoa `NSScreen` queries clamp the wheel inside the active display's `visibleFrame` so it never clips under the menu bar, notch, or Dock.

## 2. Drag, Idle & Mouse Trigger Pipeline

- **Native Monitor (`native-bridge/Sources/Monitor.swift`)**: Streams NDJSON pointer/key/drag state over `CGEventTap` + `NSEvent` monitors and inspects `NSPasteboard(name: .drag)` in real time before drop. Supports configurable single-key drag triggers (`DownArrow` default), mouse button triggers (`Button 2`/`3`/`4`/`5`), and the global Idle shortcut (`Option+Shift+Space`).
- **Input Gate (`src-tauri/src/core/input_gate.rs`)**: `ShortcutPressGate` and `ShortcutOrderGate` support both drag-first and held-key-first orderings, suppress duplicate opens within a single drag session (`acted_epoch`), consume the trigger key only while a Finder drag is active, and cleanly reset on natural release or `Esc`.
- **Feedback (`native-bridge/Sources/Common.swift`, `src-tauri/resources/flip-tick.wav`)**: Plays the acoustic tick via `NSSound` and Force Touch trackpad pulses via `NSHapticFeedbackManager`.

## 3. Core Policy, Naming, Staging & Security Guards

- **Policy (`src-tauri/src/core/policy.rs`)**: Computes per-engine capabilities, intersects compatible destinations for mixed multi-file selections, enforces batch limits (1–128 files), and gates FFmpeg-only / Pandoc-only options cleanly (`FLIP_DISABLE_FFMPEG` / `FLIP_DISABLE_PANDOC` overrides supported).
- **Input & Container Guards**: Refuses POSIX symlinks (`lstat`), Finder aliases (`com.apple.FinderInfo` `kIsAlias`), undownloaded iCloud stubs (`SF_DATALESS` / `.icloud`), and non-local/network volumes (`MNT_LOCAL`). Enforces `40 MP` pre- and post-decode image limits, `200`-page PDF caps, `256 MiB` / `4,096`-entry Zip-Slip limits across ZIP/DOCX/ODT/EPUB/XLSX/PPTX containers, XXE/DOCTYPE stripping, and offline SVG/HTML LFI sanitization (`sanitizeHTMLForOfflineImport`).
- **Zero-Overwrite Atomic Staging (`src-tauri/src/core/staging.rs`, `src-tauri/src/core/naming.rs`)**: Jobs run inside `.flip-job-<32-hex-uuid>` beside the target output folder (falling back to a configured folder if the source directory is read-only), validate output headers/decodability before commit, and commit atomically with `renamex_np(..., RENAME_EXCL)` using Finder collision counters (`stem 2.ext`, `<stem>-p001.ext`). Failed multi-page commits roll back that format's new outputs without touching originals.
- **Inline Rename Isolation**: Finder inline rename (`Return` via `FinderBridge.swift`) is triggered **only** for `Create New File`, never on conversions.

## 4. Conversion Engines & System Integrations

- **Images (`engines/image.rs`, `MediaAndDocs.swift`)**: `ImageIO` (EXIF orientation baking, optional GPS strip, `HEIC`/`AVIF`/`WEBP`/`ICNS`/`SVG`), Rust `image` crate (`ICO` encode/decode, multi-image `ANIMATE_GIF`), `PDFKit` (`COMBINE_PDF`), and Apple `Vision` (`VNGenerateForegroundInstanceMaskRequest` `REMOVE_BG`, `VNRecognizeTextRequest` OCR).
- **PDF & Documents (`engines/pdf.rs`, `engines/document.rs`, `MediaAndDocs.swift`)**: `PDFKit` / Quartz rasterization and `OPTIMIZE_PDF`, `NSAttributedString` / CoreText A4/Letter paginated PDFs, `pulldown-cmark`, pure-Rust native **EPUB 3**, **FB2**, and **RST** readers/writers (zero Pandoc required), optional `Pandoc`, and AppleScript iWork exports (`.pages`/`.numbers`/`.key`).
- **Structured Data (`engines/data.rs`)**: Pure-Rust cross-converter across `JSON` (JSON5-tolerant), `YAML`, `TOML`, `XML`, `CSV` (nested dot-flattening), native bidirectional **`XLSX`** (sheet/formula/shared-string/styles XML + zip writer), `TXT`, `MD`, `HTML`, and CoreText `PDF`.
- **Audio & Video (`engines/media.rs`, `MediaAndDocs.swift`)**: Zero-FFmpeg `AVFoundation` + CoreAudio `ExtAudioFile` + `/usr/bin/afconvert` (`M4A`, `WAV`, `FLAC`, `AIFF`, `ALAC`, `CAF`, `MP4`, `MOV`, `M4V`, animated `GIF`), plus optional `FFmpeg` (`MP3`, `OGG`, `OPUS`, `AAC`, `MKV`, `WEBM`, `AVI`) with progress parsing and cancellation.
- **Archives (`engines/archive.rs`)**: Pure-Rust `ZIP` creation (deduplicated entries) and collision-safe `UNZIP` subfolder extraction.
- **Create New File (`engines/create_new.rs`)**: Folder, `.txt`, `.md`, `.rtf`, `.docx`, `.xlsx`, `.pptx`, `.html`, `.json`, `.yaml`, `.toml`, `.xml`, `.csv`, blank `.pdf`, `.sh` (`chmod +x`), `.py`, `.swift`, `From Clipboard` (`.png`/`.pdf`/`.rtf`/`.webloc`/`.txt`), iWork, and custom templates (`{{date}}`, `{{time}}`, `{{user}}`, `{{folder}}`).
- **System Integration & Logs**: `LaunchAgents` (`core/startup.rs`), Finder Quick Actions (`core/quick_actions.rs`, `scripts/install-quick-actions.sh`), Raycast script commands (`integrations/raycast/`), rotating 2 MiB JSONL logs (`~/Library/Logs/FLIP/events.jsonl`), and redacted Markdown diagnostics (`Copy Diagnostics`).
