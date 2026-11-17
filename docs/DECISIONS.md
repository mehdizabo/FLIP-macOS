# Architectural & Product Decisions (`FLIP-macOS-port`)

Date: 2026-11-17  
Source Windows repo: `/Users/mimmo98/_MyWorkspace/_Projects/FLIP-windows` (v0.5.1)  
Target macOS repo: `/Users/mimmo98/_MyWorkspace/_Projects/FLIP-macOS-port`

## 1. User Product Decisions

1. **Target macOS & Visual Direction**
   - Built specifically for **macOS 27.2.1 (`26B131`, Golden Gate / Apple Silicon)** with an **English UI**.
   - Visual style: **Enhanced Liquid Glass** radial menu inspired by Logi Options+ Actions Ring — layered translucent glass sectors, directional specular rim highlights, subtle chromatic refraction, spring physics, dynamic QuickLook center hub, `Convert` / `New File` mode pill, radial progress arc with percentage counter, and tactile/acoustic feedback (`flip-tick.wav` + trackpad haptics).

2. **Triggers (Keyboard-First + Optional Mouse Button)**
   - **During Finder / Desktop File Drag**: Pressing a single configurable key (default `Down Arrow`, presets: `Down`, `Up`, `Left`, `Right`, `Space`, `Tab`, `Fn`, `F1`–`F12`) opens the wheel centered at the cursor. Supports both drag-then-key and key-then-drag ordering (`ShortcutOrderGate` & `ShortcutPressGate`). Reads `NSPasteboard(name: .drag)` in real time so compatible formats appear immediately before drop.
   - **In Idle (Keyboard)**: Pressing a global shortcut (default `Option+Shift+Space`, configurable) opens the wheel at the cursor:
     - If Finder has selected files -> opens in **Convert** mode.
     - If Finder has no selection -> opens in **New File** mode targeting the frontmost Finder folder (falling back to `~/Desktop`).
     - Pressing `Tab` or clicking the mode pill switches between **Convert** and **New File**.
   - **Optional Mouse Trigger**: Configurable middle/side mouse button (`Button 2` / `Button 3` / `Button 4`) supporting hold-and-release selection for Logi/multi-button mouse users.

3. **Output Naming & Inline Rename**
   - **Conversions**: Inherit the original filename stem directly (`<stem>.<ext>`, e.g., `photo.heic` -> `photo.png`). On collision, append a Finder-style non-overwriting counter (`<stem> 2.<ext>`, `<stem> 3.<ext>`). Multi-page exports use `<stem>-p001.<ext>`. **Never overwrite** an existing file and **do NOT trigger inline rename** on conversion.
   - **Create New File**: Create `Untitled.<ext>` (or `Untitled 2.<ext>` on collision) in the active Finder folder, reveal/select it in Finder, and **immediately trigger inline rename** (`Return` key event to Finder) when `autoRenameOnCreate` is enabled (default `true`).

4. **Format Coverage & Native Apple Integration**
   - Maximum local format coverage combining Rust native engines, macOS system frameworks (`PDFKit`, `Vision` OCR, `AVFoundation`, `AppKit`/`NSAttributedString`, `QuickLookThumbnailing`, `sips`, `textutil`, AppleScript for Finder & Apple iWork `Pages`/`Numbers`/`Keynote`), and optional local engines (`FFmpeg`, `Pandoc`).
   - All conversions execute **100% locally** with atomic staging (`.flip-job-<uuid>`), container bomb guards, symlink refusal, and non-overwriting commit/rollback.

5. **License & Distribution**
   - **MIT License** (matching the upstream Windows fork).
   - No paid Apple Developer certificate required for local builds: build scripts support **ad-hoc code signing (`codesign --force --deep --sign -`)** by default, plus optional `APPLE_SIGNING_IDENTITY` / notarization hooks for future signed/commercial releases.

## 2. Hybrid Architecture: Tauri v2 + Rust + Native Swift/AppKit Bridge

To deliver both a rapid cross-layer Tauri v2 desktop app (per user request) and authentic macOS system integration without requiring full `Xcode.app` (using CommandLineTools `/usr/bin/swiftc` and `/usr/bin/clang`):

- **Frontend (`src/`, Vite + TypeScript)**: Enhanced Liquid Glass radial wheel overlay (`wheel` transparent always-on-top window), Settings & Shortcuts recorder window, and Batch/Multi-Format converter window (`SelectionWindow` port).
- **Rust Backend (`src-tauri/`)**:
  - `core`: Format compatibility policy & batch intersection, `ShortcutPressGate`/`ShortcutOrderGate`, direct stem naming, atomic `.flip-job-<uuid>` staging & container validation, JSON preferences, rotating JSONL diagnostic logs.
  - `engines`: Image (`image` crate + `sips` + `PDFKit`/`Vision` bridge), Structured Data (`JSON`/`YAML`/`TOML`/`XML`/`CSV` -> `TXT`/`HTML`/`MD`/`PDF`), PDF (`PDFKit` + `Vision OCR` + multi-PDF merge), Documents (`textutil` / `NSAttributedString` / `pulldown-cmark` / A4 PDF / Pandoc / iWork), Media (`FFmpeg` with real-time progress + `AVFoundation`), and Create New File (`Folder`, office templates, scripts, `From Clipboard`, user templates with `{{date}}`/`{{time}}`/`{{user}}`/`{{folder}}`).
  - **Tauri v2 Host**: Accessory menu bar app (`TrayIconBuilder`), global window clamping across multi-monitor screens, background IPC listener for the native monitor stream.
- **Native Swift Bridge (`native-bridge/`, compiled via `/usr/bin/swiftc`)**:
  - Single native helper binary (`flip-macos-bridge`) providing:
    - Real-time `monitor` JSONL stream (`NSPasteboard(name: .drag)`, mouse button/cursor state, `CGEventTap` + `NSEvent` global key/mouse monitors)
    - Finder context (`selectedItems`, `insertionFolder`), `finder-reveal` + inline rename
    - `QLThumbnailGenerator` previews
    - `PDFKit` page rendering, PDF merging, image-to-PDF combining
    - Apple `Vision` (`VNRecognizeTextRequest`) OCR for images & scanned PDFs
    - `NSAttributedString` / Apple iWork (`Pages`/`Numbers`/`Keynote`) export
    - `AVFoundation` native media export & `NSSound` + `NSHapticFeedbackManager` wheel tick feedback
    - `AXIsProcessTrusted()` Accessibility & Automation permission checks/prompts.
