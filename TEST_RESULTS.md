# Test & Compatibility Results (`FLIP-macOS-port` v0.1.0)

Date: 2026-11-17  
Commit Scope: Steps A & B complete (Update Checker, Redacted Diagnostics, Compliance Docs, DMG Packager)

---

## 1. Host Environment & OS Compatibility Matrix

Verified host hardware & OS:
- **OS**: `macOS 27.2.1` (Build `26B131`, Golden Gate)
- **Architecture**: Apple Silicon (`arm64`)
- **Toolchain**:
  - Swift: `Apple Swift 6.3.2` (`swift-driver 1.148.6`, target `arm64-apple-macosx15.0` binary)
  - Rust / Cargo: `rustc 1.100.0` / `cargo 1.100.0`
  - Node.js: `v26.11.1`, Bun: `1.4.4`
- **Optional External Engines on Test Host**:
  - `FFmpeg`: Present (`/opt/homebrew/bin/ffmpeg` `8.1.1`)
  - `Pandoc`: Not installed on host
  - `Pages.app` / `Numbers.app` / `Keynote.app`: Not installed in `/Applications` on host

| Target Platform | Architecture | Status | Notes |
|---|---|---|---|
| **macOS 27.2.1 (`26B131`)** | Apple Silicon (`arm64`) | **Verified (Automated Tests + `.app`/`.dmg` Build + CLI Smoke)** | Primary target; native `flip-macos-bridge`, Tauri v2 `.app`, and `.dmg` built and verified here. Live WindowServer GUI drag-drop remains unexecuted in this headless/CLI run. |
| **macOS 26 (Tahoe / Liquid Glass)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Deployment target is macOS 15.0+; CSS/WebKit Liquid Glass properties align, not tested on physical macOS 26 hardware. |
| **macOS 15 (Sequoia fallback)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Minimum deployment target (`-target arm64-apple-macosx15.0`, `minimumSystemVersion: 15.0`); not booted on physical macOS 15 hardware. |
| **Intel (`x86_64`)** | `x86_64` | **Not Supported / Not Tested** | Current bridge and DMG build target `arm64` only. |

---

## 2. Automated Verification Summary

### 2.1 Native Swift/AppKit Bridge (`./native-bridge/build.sh`)
- **Result**: Passed (`exit 0`)
- **Artifact**: `native-bridge/bin/flip-macos-bridge` (`420,080` bytes, `arm64`, `-rwxr-xr-x`)
- Compiled via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` using Command Line Tools only.

### 2.2 TypeScript, Vite Production Build, and Bun Unit Tests
Command: `npx tsc --noEmit && npm run build && bun test`
- **TypeScript (`tsc --noEmit`)**: 0 errors
- **Vite Production Build (`npm run build`)**: 20 modules transformed (`dist/index.html` 0.39 kB, CSS 24.19 kB, JS 133.81 kB)
- **Bun Test Suite (`bun test`)**: **24 passed, 0 failed** across 2 files (`465` `expect()` assertions):
  - `src/ipc.test.ts`: **2/2 passed** (Rust `AppPreferences` schema alignment, mock IPC commands & events, `checkForUpdates`, `getDiagnosticsReport`, `onSettingsNavigate`, `copyDiagnosticsToClipboard` clipboard + manual fallback)
  - `src/wheelGeometry.test.ts`: **22/22 passed** (wheel format ordering, SVG chamfered sector geometry for 1–8 sectors, 3° angular hysteresis + 0°/360° wraparound, progress arc geometry, pagination, hash routing, HTML escaping, Option-modified key token mapping, batch handoff)

### 2.3 Rust Unit & Integration Test Suite (`cd src-tauri && cargo test`)
- **Total**: **62 passed, 0 failed**
  - **`flip_lib` unit tests**: **46/46 passed**
    - Includes `core::updater` (SemVer + prerelease comparison matching `FLIP-windows`, URL normalization/HTTPS validation, JSON release & tag parsing, curl error formatting)
    - Includes `core::logging` (2 MiB log rotation, `/Users/<username>` + `/private/var/folders/...` + custom home redaction to `~`, bounded 60-line `events.jsonl` tail & Markdown bundle formatting)
    - Includes `core::input_gate`, `core::naming`, `core::policy` (40 MP / 256 MiB / 4,096 zip entry / symlink / Finder alias / iCloud stub guards), `core::preferences`, `core::quick_actions`, `core::staging`, `core::startup`, `cli`, `engines::data`, `engines::document`, `host`, `host::monitor`, `host::tray` (including `"Check for Updates…"` menu item & mnemonic escaping)
  - **`tests/engines_integration.rs`**: **9/9 passed**
    - Image matrix (`PNG`, `JPG`, `WEBP`, `HEIC`, `BMP`, `TIFF`, `GIF`, `COMBINE_PDF`, alpha flattening, EXIF orientation, GPS strip, collision preservation)
    - Structured data matrix (`JSON`, `YAML`, `TOML`, `XML`, `CSV`, `HTML`, `MD`, `PDF`)
    - Documents & PDF (`A4`/`Letter` pagination, multi-page PDF rasterization, `Apple Vision` OCR + EXIF-oriented OCR + 0-byte empty OCR guard, PDF merge, > 40 MP rejection, > 256 MiB container & iWork zip bomb rejection, HTML LFI & `@import` sanitization)
    - Media (`FFmpeg 8.1.1` + native `AVFoundation` MP4→MOV/GIF + mid-flight cancellation)
    - Create New File built-in OOXML/scripts/templates & read-only/symlink guards
  - **`tests/host_ipc_integration.rs`**: **7/7 passed**
    - Verifies exact command & camelCase parameter alignment across all **25** Tauri commands and all **9** host-emitted events between Rust and `src/ipc.ts`
    - Verifies `UpdateCheckResult`, `AppPreferences`, `WheelStatePayload`, `EngineStatusPayload`, `QuickActionsStatus`, and `redact_user_paths`
    - Verifies `flip-macos-bridge` & `flip-tick.wav` resolution in both dev and relocated/symlinked `.app` layouts, plus `plutil -lint` on `src-tauri/Info.plist`

---

## 3. Release Bundle, Signing, DMG Packaging & Bundled CLI Verification

1. **Release `.app` Build (`./scripts/build-macos.sh`)**:
   - Built `src-tauri/target/release/bundle/macos/FLIP.app` (`21.70 MiB`), embedded `Contents/Resources/flip-macos-bridge` (`420,080` bytes, executable) and `flip-tick.wav`.
2. **Code Signing (`codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/FLIP.app`)**:
   - **Passed**: `valid on disk`, `satisfies its Designated Requirement` (ad-hoc local signature).
3. **DMG Packager (`./scripts/package-dmg.sh`)**:
   - Created `src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg` (`8.3 MiB`), ad-hoc signed and verified via `codesign --verify --verbose=2`.
   - Mounted with `hdiutil attach -readonly -nobrowse`: verified `FLIP.app` and `Applications -> /Applications` symlink inside mounted volume, verified `codesign --verify --deep --strict --verbose=2` on mounted `FLIP.app`, detached cleanly with `hdiutil detach`.
4. **Live Bundled CLI Smoke Test (`FLIP.app/Contents/MacOS/flip`)**:
   - `flip --version` -> `flip 0.1.0`
   - `flip new md --in <temp> --no-rename --no-reveal` -> created `Untitled.md` (`12` bytes), JSON `kindId: "md"`
   - `flip convert <temp>/Untitled.md --to PDF,DOCX` -> resolved bundled `Contents/Resources/flip-macos-bridge` cleanly and produced `Untitled.pdf` (`9,084` bytes) and `Untitled.docx` (`3,515` bytes)
   - `flip convert <temp>/Untitled.pdf --to PNG` -> rendered `Untitled-p001.png` (`44,721` bytes) via bundled `PDFKit` bridge

---

## 4. Unexecuted / Unverified Scenarios on This Host

The following paths were **not** exercised live on this machine and remain open for interactive/external QA:

- **Live WindowServer GUI & Finder Interaction**:
  - Interactive Finder file drag with `CGEventTap` drag-trigger key (`Down Arrow`) and `NSPasteboard(name: .drag)` inspection
  - Global idle shortcut (`Option+Shift+Space`), mouse-button hold-and-release trigger, live `NSAccessibility` / `Automation (Finder)` OS permission prompts, and Finder inline rename (`Return` key injection)
  - Visual/accessibility acceptance in a live desktop session: multi-monitor mixed-DPI clamping, Stage Manager, Light/Dark appearance, `Reduce Motion`, `Reduce Transparency`, `VoiceOver`, and menu bar tray interaction (including `"Check for Updates…"`)
  - Clipboard UI copy flows inside live `WKWebView` (`SettingsView`, `WheelView`, `BatchView` — verified via `bun test` mocks + TypeScript build, not clicked in a live `WKWebView` window)
- **Missing Optional External Apps / Tools on Host**:
  - **Pandoc** (`EPUB`, `FB2`, `RST` conversions): Not installed on host (`pandoc not found`); capability gating verified via unit tests, runtime Pandoc subprocess execution unverified here.
  - **Apple iWork** (`Pages.app`, `Numbers.app`, `Keynote.app` AppleScript automation): Not installed in `/Applications`; package-bundle `.pages` QuickLook PDF extraction and missing-app error paths were tested instead.
- **Live GitHub Network Request**:
  - `check_github_updates` invoking `/usr/bin/curl` against `https://api.github.com/repos/thetinyfoundry/FLIP/releases/latest` was not queried over live internet in the automated test suite; URL normalization, JSON release/tag parsing, SemVer comparison, and offline/timeout error formatting are unit-tested.
- **Developer ID Signing & Apple Notarization**:
  - Scripts (`scripts/sign-local.sh`, `scripts/package-dmg.sh`) support `APPLE_SIGNING_IDENTITY` and `xcrun notarytool` / `stapler`, but only ad-hoc local signing (`--sign -`) was executed because no paid Apple Developer certificate is configured.
