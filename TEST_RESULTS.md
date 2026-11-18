# Test & Compatibility Results (`FLIP-macOS-port` v0.1.0)

Date: 2026-11-18  
Commit Scope: Steps A–H complete (`ZIP`/`UNZIP` archive engine, Apple Vision `REMOVE_BG`, Quartz `OPTIMIZE_PDF`, zero-FFmpeg CoreAudio/`afconvert` `CAF`/`FLAC`, single-instance Unix socket IPC + CLI control commands, `EN`/`IT` localization + `"Test Wheel Sound & Haptics"` button, 50-cycle & 20+10 stress/performance test suite, `.app` & `.dmg` release verification)

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
  - `FFmpeg`: Present (`/opt/homebrew/bin/ffmpeg` `8.1.1`) — note: because `detect_tools` checks `/opt/homebrew/bin/ffmpeg` before `PATH`, zero-FFmpeg audio paths (`CAF`/`FLAC`/`M4A`/`WAV`/`AIFF`/`ALAC`, including `ALAC -> M4A`) were explicitly verified with `EngineCapabilities`/`DetectedTools { ffmpeg: None, .. }` in `engines_integration.rs` and via direct `flip-macos-bridge convert-av` + `/usr/bin/afconvert` invocations
  - `Pandoc`: Not installed on host (built-in EPUB 3 runs completely Pandoc-free; Pandoc is only required for `FB2`/`RST`)
  - `Pages.app` / `Numbers.app` / `Keynote.app`: Not installed in `/Applications` on host

| Target Platform | Architecture | Status | Notes |
|---|---|---|---|
| **macOS 27.2.1 (`26B131`)** | Apple Silicon (`arm64`) | **Verified (Automated Tests + Stress/Perf + `.app`/`.dmg` Build + CLI Smoke)** | Primary target; native `flip-macos-bridge`, Tauri v2 `.app`, and `.dmg` built and verified here. Live WindowServer GUI drag-drop & VoiceOver walkthrough checklist documented in [`ACCEPTANCE.md`](ACCEPTANCE.md). |
| **macOS 26 (Tahoe / Liquid Glass)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Deployment target is macOS 15.0+; CSS/WebKit Liquid Glass properties align, not tested on physical macOS 26 hardware. |
| **macOS 15 (Sequoia fallback)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Minimum deployment target (`-target arm64-apple-macosx15.0`, `minimumSystemVersion: 15.0`); `.github/workflows/ci.yml` targets `macos-15` runner, not yet executed remotely on macOS 15 hardware. |
| **Intel (`x86_64`)** | `x86_64` | **Not Supported / Not Tested** | Current bridge and DMG build target `arm64` only. |

---

## 2. Automated Verification Summary

### 2.1 Native Swift/AppKit Bridge (`./native-bridge/build.sh`)
- **Result**: Passed (`exit 0`)
- **Artifact**: `native-bridge/bin/flip-macos-bridge` (`484,128` bytes before bundle re-sign; `481,472` bytes inside signed `FLIP.app/Contents/Resources/flip-macos-bridge`, `arm64`, `-rwxr-xr-x@`)
- Compiled via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` using Command Line Tools only (zero `Xcode.app` required).

### 2.2 TypeScript, Vite Production Build, and Bun Unit Tests
Command: `npx tsc --noEmit && npm run build && bun test`
- **TypeScript (`tsc --noEmit`)**: 0 errors
- **Vite Production Build (`npm run build`)**: 21 modules transformed (`dist/index.html` 0.39 kB, `dist/assets/index-vTEYyANX.css` 25.06 kB, `dist/assets/index-CE--PlCN.js` 199.18 kB in 230–243 ms)
- **Bun Test Suite (`bun test`)**: **35 passed, 0 failed** across 3 files (`1,711` `expect()` assertions):
  - `src/ipc.test.ts`: **4/4 passed** (Rust `AppPreferences` schema alignment including `language` & format lists, mock IPC commands & events including `"sound"`/`"tick"`/`"none"` feedback test & language updates, native Pandoc-free `EPUB 3`, and zero-FFmpeg `CAF`/`FLAC` audio capability policy)
  - `src/wheelGeometry.test.ts`: **26/26 passed** (wheel format ordering, `reorderFormatList` / `moveFormatToIndex` clamping, SVG chamfered sector geometry for 1–8 sectors, 3° angular hysteresis + 0°/360° wraparound, progress arc geometry, pagination, hash routing, `inspectFilesLocally` coverage including `ZIP`/`UNZIP`/`REMOVE_BG`/`OPTIMIZE_PDF`, HTML escaping, Option-modified key token mapping, non-ASCII thumbnail btoa fallback, batch handoff, and VoiceOver live announcement formatters)
  - `src/i18n.test.ts`: **5/5 passed** (`EN`/`IT` full key parity, non-empty dictionary validation, consistent placeholder tokens, clean interpolation without double substitution, safe fallback to English on invalid/empty inputs, and localization of built-in New File kind names, disabled reasons, clipboard kinds, categories, and availability messages)

### 2.3 Rust Clippy, Unit, Engine/IPC Integration & Stress/Perf Suite (`cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Clippy (`cargo clippy --all-targets -- -D warnings`)**: **Passed with 0 warnings**
- **Cargo Test Suite (`cargo test` offline default suite)**: **80 passed, 0 failed, 1 ignored** (`live_github_update_check_and_tags_fallback`, hits live `github.com`):
  - **`flip_lib` unit tests**: **55/55 passed** (+ 1 `#[ignore]`)
    - Covers `core::policy`, `core::preferences` (incl. `language` `"en"`/`"it"` sanitization & default backfill), `core::input_gate`, `core::naming`, `core::staging`, `core::startup`, `core::quick_actions`, `core::logging`, `core::updater`, `cli` (incl. `flip wheel`/`settings`/`batch`/`quit` parser & execution), `engines::data`, `engines::document`, `host::commands` (incl. independent `sound_enabled`/`haptics_enabled` `resolve_feedback_kind`), `host::monitor`, `host::tray`.
  - **`tests/engines_integration.rs`**: **15/15 passed**
    - Covers `zip_and_unzip_roundtrip_dedup_collisions_and_safety_rejections` (Zip-Slip, >256 MiB / >4,096 entries, collisions, stem deduplication)
    - Covers `vision_remove_bg_cuts_subject_and_rejects_blank_image` (transparent PNG cutout + blank-canvas rejection)
    - Covers `optimize_pdf_shrinks_heavy_pdf_preserves_text_and_rejects_over_200_pages`
    - Covers `native_zero_ffmpeg_audio_caf_and_flac_roundtrips` (`WAV <-> CAF`, `WAV <-> FLAC`, `CAF -> FLAC`, `FLAC -> CAF`, `FLAC -> M4A`, `CAF -> M4A`, `FLAC -> ALAC`, `ALAC -> FLAC`, `ALAC -> M4A -> WAV`, zero FFmpeg binary)
    - Plus native `EPUB 3`, PDF multi-page rasterization + Vision OCR + merge, image matrix + EXIF/GPS, >40 MP / HTML LFI safety, FFmpeg/AVFoundation video + mid-flight cancellation, read-only fallback, OOXML + templates.
  - **`tests/host_ipc_integration.rs`**: **8/8 passed**
    - Verifies exact command & camelCase alignment across all **27** Tauri commands and all **11** host-emitted events between Rust and `src/ipc.ts`
    - Verifies `unix_socket_ipc_roundtrips_permissions_and_stale_recovery` (`0600` owner-only socket, live command roundtrips, stale socket cleanup/recovery)
    - Verifies bridge & WAV resource resolution in dev and relocated `.app` layouts, `Info.plist` `plutil -lint`, screen clamping, and Tauri capabilities consistency.
  - **`tests/stress_and_perf.rs`**: **2/2 passed** (`2.28 s`)
    - **`stress_50_cycle_trigger_wheel_escape_and_multimonitor_clamping`**: Drives 50 rapid trigger -> wheel open -> pointer drag -> natural release / global Escape teardown cycles across 4 simulated multi-monitor layouts (Retina + 1x left + top-notch + ultrawide side-Dock), verifying `!press.is_down()` / `!order.held()` BEFORE any `reset()` on normal release cycles, mid-hold Escape teardown `reset()` on `cycle % 5 == 4`, `acted_epoch` drag-end retention, 0 orphaned wheel `is_open` / `is_drag` states, and 100% bounded screen coordinates.
    - **`stress_20_plus_10_staging_cleanup_policy_latency_and_cli_startup_perf`**:
      - Executes **20 sequential successful conversions** (`Data`, `Document`/`EPUB`, `Image`, `ZIP`) + **10 intentionally failing/mid-flight cancelled `convert_files` jobs that each enter `JobStaging::create`** (2 mid-flight cancellations triggered in progress callback at `pct >= 20`, 2 corrupt JSON, 2 zip-slip/truncated PK-zip `UNZIP`, 2 corrupt PNG, 1 corrupt PDF `OPTIMIZE_PDF`, 1 blank transparent PNG `REMOVE_BG`) + 2 read-only `JobStaging::create` `Drop` aborts in an isolated `TempDir`: **0 leftover `.flip-job-*` or `.flip-probe-*` directories**.
      - `policy::outputs` latency: **16.83 µs** single file, **1.21 ms** max 128-file batch, **21.81 µs** avg over 500 mixed lookups (target `< 5 ms`).
      - Compiled CLI `flip --version` startup (`target/debug/flip`): **7.24 ms – 9.74 ms** across 5 warm runs after macOS one-time `1.046 s` first-exec AMFI/syspolicyd cache scan on newly linked binary (target `< 300 ms`).

---

## 3. Release Bundle, Signing, DMG Packaging & Bundled CLI Verification

1. **Release `.app` Build (`./scripts/build-macos.sh --dmg`)**:
   - Built `src-tauri/target/release/bundle/macos/FLIP.app` (`22.23 MiB` / `23,309,300` bytes total files; main `flip` Mach-O executable `20,530,624` bytes), embedded signed `Contents/Resources/flip-macos-bridge` (`481,472` bytes, `-rwxr-xr-x@`) and `Contents/Resources/resources/flip-tick.wav`.
2. **Code Signing (`codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/FLIP.app`)**:
   - **Passed**: `valid on disk`, `satisfies its Designated Requirement` (ad-hoc local signature).
3. **DMG Packager & Mount Verification (`src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg`, `8,931,632` bytes / `8.5 MiB`)**:
   - `codesign --verify --deep --strict --verbose=2` on `.dmg`: **Passed** (`valid on disk`, `satisfies its Designated Requirement`).
   - Mounted with `hdiutil attach -nobrowse -readonly -mountpoint ...`: verified `FLIP.app` and `Applications -> /Applications` symlink inside mounted volume, verified `codesign --verify --deep --strict --verbose=2` on mounted `FLIP.app`, and ejected cleanly with `hdiutil detach`.
4. **Live Bundled Release CLI & Bridge Smoke Tests (`FLIP.app/Contents/MacOS/flip` & `Contents/Resources/flip-macos-bridge`)**:
   - **`--version` startup latency**: `flip 0.1.0`, **6.36 ms** warm.
   - **`ZIP` -> `UNZIP` roundtrip**: `convert hello.txt notes.md --to ZIP` -> `hello.zip` (`244` B) -> `convert hello.zip --to UNZIP` -> extracted `hello/` with identical file contents verified.
   - **Audio (`WAV -> FLAC -> WAV`, `WAV -> CAF`, `WAV -> ALAC -> M4A`) & Zero-FFmpeg Direct Bridge**: Bundled CLI converted 1 s 44.1 kHz `tone.wav` to `tone.flac` (`21,230` B) -> `tone 2.wav` (`176,478` B), `tone.caf` (`176,530` B), and `tone.alac` (`25,912` B) -> `tone 2.m4a` (`18,451` B); direct `flip-macos-bridge convert-av` (no FFmpeg) also converted `tone.alac -> direct_alac.m4a` (`8,728` B) and short 1,764-frame `src-tauri/resources/flip-tick.wav -> direct_tick.flac` (`1,836` B).
   - **Native `EPUB 3` (`MD -> EPUB -> TXT`)**: `notes.md` -> `notes.epub` (`2,132` B) -> `notes 2.txt` (`13` B) cleanly with no Pandoc installed.
   - **`OPTIMIZE_PDF`**: 800×600 PNG-backed `big.pdf` (`479,328` B) -> `big 2.pdf` (`383,743` B, `-20%` reduction cleanly).
   - **Apple Vision `REMOVE_BG`**: `subject.png` (rendered from `src-tauri/icons/icon.icns`) -> `subject 2.png` (`rc=0`, transparent cutout PNG cleanly).

---

## 4. Unexecuted / Unverified Scenarios on This Host

The following paths were **not** executed live inside a WindowServer GUI session on this machine and remain tracked for physical desktop verification in [`ACCEPTANCE.md`](ACCEPTANCE.md):

- **Live WindowServer GUI, Finder & Accessibility Walkthrough**:
  - Interactive Finder drag + trigger (`Drag -> Trigger` and `Trigger -> Drag`), `NSPasteboard(name: .drag)` live capture, Mouse Button `2`/`3`/`4` triggers, global Idle shortcut (`Option+Shift+Space`), Full-Screen Spaces overlay, multi-monitor mixed-DPI positioning, live Finder inline rename (`Return`), Light/Dark appearance, `Reduce Motion`, `Reduce Transparency`, and **VoiceOver** live announcements
  - Live `WKWebView` clicks/controls across `#settings` (`EN`/`IT` language switcher, `Test Wheel Sound & Haptics` physical audio + Force Touch feedback, `Preferred Format Order` chips + `Reset to Default`), `#wheel`, `#batch`, `#onboarding`, and live `flip://` / `flip wheel` / `flip settings` / `flip batch` / `flip quit` GUI socket launches
- **Missing Optional External Apps / Tools on Host**:
  - **Pandoc** (`FB2` and `RST` conversions): Not installed on host; capability gating verified via Rust & Bun unit tests, runtime Pandoc subprocess execution unverified here.
  - **Apple iWork** (`Pages.app`, `Numbers.app`, `Keynote.app` AppleScript automation): Not installed in `/Applications`; missing-app errors and `.pages` QuickLook extraction tested instead.
- **GitHub Actions CI Execution**:
  - `.github/workflows/ci.yml` verified locally against all stages (incl. `cargo clippy --all-targets -- -D warnings` and `cargo test` running `stress_and_perf.rs`), not yet triggered on a live GitHub `macos-15` runner.
- **Developer ID Signing & Apple Notarization**:
  - Scripts (`scripts/sign-local.sh`, `scripts/package-dmg.sh`) support `APPLE_SIGNING_IDENTITY` and `xcrun notarytool` / `stapler`, but only ad-hoc local signing (`--sign -`) was executed because no paid Apple Developer certificate is configured.
