# Test & Compatibility Results (`FLIP-macOS-port` v0.1.0)

Date: 2026-11-18  
Commit Scope: Steps A–L complete (`ZIP`/`UNZIP` archive engine, Apple Vision `REMOVE_BG`, Quartz `OPTIMIZE_PDF`, zero-FFmpeg CoreAudio/`afconvert` `CAF`/`FLAC`, single-instance Unix socket IPC + CLI control commands, `EN`/`IT` localization + `"Test Wheel Sound & Haptics"` button, native zero-Pandoc `FB2`/`RST` engines + `FLIP_DISABLE_FFMPEG`/`FLIP_DISABLE_PANDOC` overrides, `flip preview --out` SVG generator, automated DOM E2E suite `views.e2e.test.ts`, offline-sanitized `SVG` input + `ICO` output + multi-image `ANIMATE_GIF`, `install-local.sh`, Homebrew Cask `packaging/homebrew/flip.rb`, `CHANGELOG.md`, 50-cycle & 20+10 stress/performance test suite, `.app` & `.dmg` release verification)

---

## 1. Host Environment & OS Compatibility Matrix

Verified host hardware & OS:
- **OS**: `macOS 27.2.1` (Build `26B131`, Golden Gate)
- **Architecture**: Apple Silicon (`arm64`)
- **Toolchain**:
  - Swift: `Apple Swift 6.3.2` (`swift-driver 1.148.6`, target `arm64-apple-macosx15.0` binary)
  - Rust / Cargo: `rustc 1.100.0` / `cargo 1.100.0`, `tauri-cli 2.12.4`
  - Node.js: `v26.11.1`, `npm 11.16.0`, Bun: `1.4.4`
- **Optional External Engines on Test Host**:
  - `FFmpeg`: Present (`/opt/homebrew/bin/ffmpeg` `8.1.1`) — zero-FFmpeg audio paths (`CAF`/`FLAC`/`M4A`/`WAV`/`AIFF`/`ALAC`, including `ALAC -> M4A`) verified with `EngineCapabilities`/`DetectedTools { ffmpeg: None, .. }`, `FLIP_DISABLE_FFMPEG=1`, and direct `flip-macos-bridge convert-av` + `/usr/bin/afconvert` invocations
  - `Pandoc`: Not installed on host (`EPUB 3`, `FB2`, and `RST` all run natively in pure Rust with zero Pandoc required; `FLIP_DISABLE_PANDOC=1` also explicitly verified)
  - `Pages.app` / `Numbers.app` / `Keynote.app`: Not installed in `/Applications` on host

| Target Platform | Architecture | Status | Notes |
|---|---|---|---|
| **macOS 27.2.1 (`26B131`)** | Apple Silicon (`arm64`) | **Verified (Automated Tests + DOM E2E + Stress/Perf + `.app`/`.dmg`/Installer + CLI Smoke)** | Primary target; native `flip-macos-bridge`, Tauri v2 `.app`, `.dmg`, `install-local.sh`, Homebrew Cask SHA-256, and CLI smokes verified here. Live WindowServer GUI drag-drop & VoiceOver walkthrough checklist documented in [`ACCEPTANCE.md`](ACCEPTANCE.md). |
| **macOS 26 (Tahoe / Liquid Glass)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Deployment target is macOS 15.0+; CSS/WebKit Liquid Glass properties align, not tested on physical macOS 26 hardware. |
| **macOS 15 (Sequoia fallback)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Minimum deployment target (`-target arm64-apple-macosx15.0`, `minimumSystemVersion: 15.0`, Cask `depends_on macos: ">= :sequoia"`); `.github/workflows/ci.yml` targets `macos-15` runner, not yet executed remotely on macOS 15 hardware. |
| **Intel (`x86_64`)** | `x86_64` | **Not Supported / Not Tested** | Current bridge, DMG, and Cask (`depends_on arch: :arm64`) target `arm64` only. |

---

## 2. Automated Verification Summary

### 2.1 Native Swift/AppKit Bridge (`./native-bridge/build.sh`)
- **Result**: Passed (`exit 0`)
- **Artifact**: `native-bridge/bin/flip-macos-bridge` (`489,104` bytes before bundle re-sign; `486,416` bytes inside signed `FLIP.app/Contents/Resources/flip-macos-bridge`, `arm64`, `-rwxr-xr-x@`)
- Compiled via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` using Command Line Tools only (zero `Xcode.app` required).

### 2.2 TypeScript, Vite Production Build, and Bun Unit + DOM E2E Tests
Command: `npx tsc --noEmit && npm run build && bun test`
- **TypeScript (`tsc --noEmit`)**: 0 errors
- **Vite Production Build (`npm run build`)**: 21 modules transformed (`dist/index.html` 0.39 kB, `dist/assets/index-vTEYyANX.css` 25.06 kB, `dist/assets/index-CGhqwAa3.js` 199.61 kB in 234–244 ms)
- **Bun Test Suite (`bun test`)**: **41 passed, 0 failed** across 4 files (`1,864` `expect()` assertions in `4.58 s`):
  - `src/views.e2e.test.ts`: **5/5 passed** (lightweight in-memory DOM + SVG shim E2E suite covering all 4 views: `WheelView` Convert & New File sectors + click/keyboard nav + `More`/`Back` focus restoration + VoiceOver live announcer + Error Hub `Copy Diagnostics` / Batch Details; `SettingsView` all 6 tabs + live `EN`/`IT` switch + shortcut recording & `Escape` cancel + format reordering with focus retention + serialized feedback test after `saveQueue`; `BatchView` file loading, `Select All`/`Clear`, `ICO`/`REMOVE_BG`/`COMBINE_PDF`/`ANIMATE_GIF`/`ZIP` selection & result rows; `OnboardingView` steps 1 -> 2 -> 3 + permission probes + completion key)
  - `src/ipc.test.ts`: **5/5 passed** (Rust `AppPreferences` schema alignment including `language` & format lists, mock IPC commands & events including `"sound"`/`"tick"`/`"none"` feedback test & language updates, native Pandoc-free `EPUB 3`/`FB2`/`RST`, zero-FFmpeg `CAF`/`FLAC` audio capability policy, and `SVG` input / `ICO` output / `ANIMATE_GIF` policy alignment)
  - `src/wheelGeometry.test.ts`: **26/26 passed** (wheel format ordering, `reorderFormatList` / `moveFormatToIndex` clamping, SVG chamfered sector geometry for 1–8 sectors, 3° angular hysteresis + 0°/360° wraparound, progress arc geometry, pagination, hash routing, `inspectFilesLocally` coverage including `ZIP`/`UNZIP`/`REMOVE_BG`/`OPTIMIZE_PDF`/`ICO`/`SVG`/`ANIMATE_GIF`, HTML escaping, Option-modified key token mapping, non-ASCII thumbnail `btoa` fallback, batch handoff, and VoiceOver live announcement formatters)
  - `src/i18n.test.ts`: **5/5 passed** (`EN`/`IT` full key parity, non-empty dictionary validation, consistent placeholder tokens, clean interpolation without double substitution, safe fallback to English on invalid/empty inputs, and localization of built-in New File kind names, disabled reasons, clipboard kinds, categories, and availability messages)

### 2.3 Rust Clippy, Unit, Engine/IPC Integration & Stress/Perf Suite (`cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Clippy (`cargo clippy --all-targets -- -D warnings`)**: **Passed with 0 warnings**
- **Cargo Test Suite (`cargo test` offline default suite)**: **97 passed, 0 failed, 1 ignored** (`live_github_update_check_and_tags_fallback`, hits live `github.com`):
  - **`flip_lib` unit tests**: **67/67 passed** (+ 1 `#[ignore]`)
    - Covers `core::policy` (incl. `FB2`/`RST`, `SVG` input, `ICO` output, `ANIMATE_GIF`), `core::preferences` (incl. `language` `"en"`/`"it"` & `ICO` default backfill), `core::input_gate`, `core::naming`, `core::staging`, `core::startup`, `core::quick_actions`, `core::logging`, `core::updater`, `cli` (incl. `flip wheel`/`settings`/`batch`/`preview`/`quit` parser & execution), `engines::data`, `engines::document` (native `EPUB 3`, `FB2`, `RST` readers & writers + security guards), `engines` (`FLIP_DISABLE_FFMPEG`/`FLIP_DISABLE_PANDOC` overrides), `host::commands`, `host::monitor`, `host::tray`.
  - **`tests/engines_integration.rs`**: **19/19 passed** (`49.48 s`)
    - Covers `zip_and_unzip_roundtrip_dedup_collisions_and_safety_rejections` (Zip-Slip, >256 MiB / >4,096 entries, collisions, stem deduplication)
    - Covers `vision_remove_bg_cuts_subject_and_rejects_blank_image` (transparent PNG cutout + blank-canvas rejection)
    - Covers `optimize_pdf_shrinks_heavy_pdf_preserves_text_and_rejects_over_200_pages`
    - Covers `native_zero_ffmpeg_audio_caf_and_flac_roundtrips` (`WAV <-> CAF`, `WAV <-> FLAC`, `CAF -> FLAC`, `FLAC -> CAF`, `FLAC -> M4A`, `CAF -> M4A`, `FLAC -> ALAC`, `ALAC -> FLAC`, `ALAC -> M4A -> WAV`, zero FFmpeg binary)
    - Covers `native_zero_pandoc_fb2_and_rst_matrix_and_tool_disable_overrides` and `zero_pandoc_full_fb2_rst_matrix_across_docs_and_images`
    - Covers `svg_input_rasterization_ico_output_and_animate_gif_batch` (incl. offline SVG LFI sanitization, hardened ICO/WEBP decoding, and multi-image `ANIMATE_GIF`)
    - Plus native `EPUB 3`, PDF multi-page rasterization + Vision OCR + merge, image matrix + EXIF/GPS, >40 MP / HTML LFI safety, FFmpeg/AVFoundation video + mid-flight cancellation, read-only fallback, OOXML + templates.
  - **`tests/host_ipc_integration.rs`**: **9/9 passed** (`0.47 s`)
    - Verifies exact command & camelCase alignment across all **27** Tauri commands and all **11** host-emitted events between Rust and `src/ipc.ts`
    - Verifies `unix_socket_ipc_roundtrips_permissions_and_stale_recovery` (`0600` owner-only socket, live command roundtrips, stale socket cleanup/recovery)
    - Verifies `cli_preview_binary_generates_valid_svg_and_sectors`
    - Verifies bridge & WAV resource resolution in dev and relocated `.app` layouts, `Info.plist` `plutil -lint`, screen clamping, and Tauri capabilities consistency.
  - **`tests/stress_and_perf.rs`**: **2/2 passed** (`3.31 s` with `--nocapture`, `1.50 s` in full suite)
    - **`stress_50_cycle_trigger_wheel_escape_and_multimonitor_clamping`**: Drives 50 rapid trigger -> wheel open -> pointer drag -> natural release / global Escape teardown cycles across 4 simulated multi-monitor layouts (Retina + 1x left + top-notch + ultrawide side-Dock), verifying `!press.is_down()` / `!order.held()` BEFORE any `reset()` on normal release cycles, mid-hold Escape teardown `reset()` on `cycle % 5 == 4`, `acted_epoch` drag-end retention, 0 orphaned wheel `is_open` / `is_drag` states, and 100% bounded screen coordinates.
    - **`stress_20_plus_10_staging_cleanup_policy_latency_and_cli_startup_perf`**:
      - Executes **20 sequential successful conversions** (`Data`, `Document`/`EPUB`, `Image`, `ZIP`) + **10 intentionally failing/mid-flight cancelled `convert_files` jobs that each enter `JobStaging::create`** + 2 read-only `JobStaging::create` `Drop` aborts in an isolated `TempDir`: **0 leftover `.flip-job-*` or `.flip-probe-*` directories**.
      - `policy::outputs` latency: **21.791 µs** single file, **1.6415 ms** max 128-file batch, **54.525 µs** avg over 500 mixed lookups (target `< 5 ms`).
      - Compiled CLI `flip --version` startup (`target/debug/flip`): **8.26 ms – 19.97 ms** warm across 5 runs after macOS one-time `1.222 s` cold first-exec AMFI/syspolicyd cache scan on newly linked binary (target `< 300 ms`).

---

## 3. Release Bundle, Signing, DMG Packaging, Installer & Bundled CLI Verification

1. **Release `.app` Build (`./scripts/build-macos.sh --dmg`)**:
   - Built `src-tauri/target/release/bundle/macos/FLIP.app` (`22.65 MiB` / `23,632,612` bytes total files; main `flip` Mach-O executable `20,848,992` bytes), embedded signed `Contents/Resources/flip-macos-bridge` (`486,416` bytes, `-rwxr-xr-x@`) and `Contents/Resources/resources/flip-tick.wav`.
2. **Code Signing (`codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/FLIP.app`)**:
   - **Passed**: `valid on disk`, `satisfies its Designated Requirement` (ad-hoc local signature).
3. **DMG Packager, SHA-256 & Homebrew Cask Sync (`./scripts/package-dmg.sh --update-cask`)**:
   - Built `src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg` (`9,065,745` bytes / `8.6 MiB`).
   - **SHA-256**: `7aa65cea17524e73db9edd3e1771eb43531baba0005c51d1199ea96380674668` — verified synced into `packaging/homebrew/flip.rb` (`ruby -c packaging/homebrew/flip.rb` -> `Syntax OK`).
   - `codesign --verify --deep --strict --verbose=2` on `.dmg`: **Passed** (`valid on disk`, `satisfies its Designated Requirement`).
   - Mounted with `hdiutil attach -nobrowse -readonly -mountpoint ...`: verified `FLIP.app` and `Applications -> /Applications` symlink inside mounted volume, verified `codesign --verify --deep --strict --verbose=2` on mounted `FLIP.app`, and ejected cleanly with `hdiutil detach`.
4. **Local Installer (`scripts/install-local.sh`)**:
   - `./scripts/install-local.sh --dest <tempdir> --no-quick-actions --no-cli --dry-run`: validated source bundle & printed planned actions with zero filesystem mutations (`test ! -e <tempdir>/FLIP.app`).
   - `./scripts/install-local.sh --dest <tempdir> --no-quick-actions --no-cli`: copied `FLIP.app`, cleared `com.apple.quarantine`, and passed `codesign --verify --deep --strict --verbose=2` on `<tempdir>/FLIP.app`.
5. **Live Bundled Release CLI Smoke Tests (`<tempdir>/FLIP.app/Contents/MacOS/flip`)**:
   - **`--version`**: `flip 0.1.0` (`exit 0`).
   - **Zero-Pandoc `FB2` & `RST`**: `FLIP_DISABLE_PANDOC=1 flip convert doc.md --to FB2,RST` -> `doc.fb2` (`529` B) and `doc.rst` (`40` B), `success: true`.
   - **`SVG -> ICO`**: `flip convert badge.svg --to ICO` -> `badge.ico` (`6.3` KB), `success: true`.
   - **Multi-Image `ANIMATE_GIF`**: `flip convert img1.png img2.png --to ANIMATE_GIF` -> `img1.gif` (`153` B), `success: true`.
   - **CLI SVG Wheel Preview**: `flip preview --out wheel.svg --files img1.png --hover 0` -> `wheel.svg` (`9.4` KB, valid `<svg>` with 8 sectors: `JPG`, `WEBP`, `HEIC`, `PDF`, `No BG`, `TIFF`, `GIF`, `More ▸`), `success: true`.

---

## 4. Unexecuted / Unverified Scenarios on This Host

The following paths were **not** executed live inside a WindowServer GUI session on this machine and remain tracked for physical desktop verification in [`ACCEPTANCE.md`](ACCEPTANCE.md):

- **Live WindowServer GUI, Finder & Accessibility Walkthrough**:
  - Interactive Finder drag + trigger (`Drag -> Trigger` and `Trigger -> Drag`), `NSPasteboard(name: .drag)` live capture, Mouse Button `2`/`3`/`4` triggers, global Idle shortcut (`Option+Shift+Space`), Full-Screen Spaces overlay, multi-monitor mixed-DPI positioning, live Finder inline rename (`Return`), Light/Dark appearance, `Reduce Motion`, `Reduce Transparency`, and **VoiceOver** live announcements
  - Live `WKWebView` clicks/controls across `#settings` (`EN`/`IT` language switcher, `Test Wheel Sound & Haptics` physical audio + Force Touch feedback, `Preferred Format Order` chips + `Reset to Default`), `#wheel`, `#batch`, `#onboarding`, and live `flip://` / `flip wheel` / `flip settings` / `flip batch` / `flip quit` GUI socket launches
  - Default interactive `./scripts/install-local.sh` execution into `/Applications` (or `~/Applications`) with live Quick Actions registration in `~/Library/Services` and `~/.local/bin/flip` symlink
- **Missing Optional External Apps / Tools on Host**:
  - **Pandoc** (`FB2` and `RST` optional Pandoc-preferred path): Not installed on host; built-in native zero-Pandoc `FB2`/`RST` engines verified end-to-end, while live Pandoc subprocess invocation remains unverified here.
  - **Apple iWork** (`Pages.app`, `Numbers.app`, `Keynote.app` AppleScript automation): Not installed in `/Applications`; missing-app errors and `.pages` QuickLook extraction tested instead.
- **GitHub Actions CI Execution**:
  - `.github/workflows/ci.yml` verified locally against all stages, not yet triggered on a live GitHub `macos-15` runner.
- **Developer ID Signing & Apple Notarization**:
  - Scripts (`scripts/sign-local.sh`, `scripts/package-dmg.sh`) support `APPLE_SIGNING_IDENTITY` and `xcrun notarytool` / `stapler`, but only ad-hoc local signing (`--sign -`) was executed because no paid Apple Developer certificate is configured.
