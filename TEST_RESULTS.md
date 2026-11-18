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
  - `FFmpeg`: Present (`/opt/homebrew/bin/ffmpeg` `8.1.1`) — note: zero-FFmpeg audio paths (`CAF`/`FLAC`/`M4A`/`WAV`/`AIFF`/`ALAC`) and zero-FFmpeg release CLI tests were explicitly verified with FFmpeg capability off and under `PATH=/usr/bin:/bin`
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
- **Artifact**: `native-bridge/bin/flip-macos-bridge` (`467,376` bytes before bundle re-sign; `464,816` bytes inside signed `FLIP.app/Contents/Resources/flip-macos-bridge`, `arm64`, `-rwxr-xr-x@`)
- Compiled via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` using Command Line Tools only (zero `Xcode.app` required).

### 2.2 TypeScript, Vite Production Build, and Bun Unit Tests
Command: `npx tsc --noEmit && npm run build && bun test`
- **TypeScript (`tsc --noEmit`)**: 0 errors
- **Vite Production Build (`npm run build`)**: 21 modules transformed (`dist/index.html` 0.39 kB, `dist/assets/index-vTEYyANX.css` 25.06 kB, `dist/assets/index-ByTp273z.js` 194.95 kB in 226–236 ms)
- **Bun Test Suite (`bun test`)**: **34 passed, 0 failed** across 3 files (`1,597` `expect()` assertions):
  - `src/ipc.test.ts`: **4/4 passed** (Rust `AppPreferences` schema alignment including `language` & format lists, mock IPC commands & events including `"sound"`/`"tick"`/`"none"` feedback test & language updates, native Pandoc-free `EPUB 3`, and zero-FFmpeg `CAF`/`FLAC` audio capability policy)
  - `src/wheelGeometry.test.ts`: **26/26 passed** (wheel format ordering, `reorderFormatList` / `moveFormatToIndex` clamping, SVG chamfered sector geometry for 1–8 sectors, 3° angular hysteresis + 0°/360° wraparound, progress arc geometry, pagination, hash routing, `inspectFilesLocally` coverage including `ZIP`/`UNZIP`/`REMOVE_BG`/`OPTIMIZE_PDF`, HTML escaping, Option-modified key token mapping, non-ASCII thumbnail btoa fallback, batch handoff, and VoiceOver live announcement formatters)
  - `src/i18n.test.ts`: **4/4 passed** (`EN`/`IT` full key parity, non-empty dictionary validation, consistent placeholder tokens, clean interpolation without double substitution, safe fallback to English on invalid/empty inputs)

### 2.3 Rust Clippy, Unit, Engine/IPC Integration & Stress/Perf Suite (`cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Clippy (`cargo clippy --all-targets -- -D warnings`)**: **Passed with 0 warnings**
- **Cargo Test Suite (`cargo test` offline default suite)**: **80 passed, 0 failed, 1 ignored** (`live_github_update_check_and_tags_fallback`, hits live `github.com`):
  - **`flip_lib` unit tests**: **55/55 passed** (+ 1 `#[ignore]`)
    - Covers `core::policy`, `core::preferences` (incl. `language` `"en"`/`"it"` sanitization & default backfill), `core::input_gate`, `core::naming`, `core::staging`, `core::startup`, `core::quick_actions`, `core::logging`, `core::updater`, `cli` (incl. `flip wheel`/`settings`/`batch`/`quit` parser & execution), `engines::data`, `engines::document`, `host::commands` (incl. independent `sound_enabled`/`haptics_enabled` `resolve_feedback_kind`), `host::monitor`, `host::tray`.
  - **`tests/engines_integration.rs`**: **15/15 passed**
    - Covers `zip_and_unzip_roundtrip_dedup_collisions_and_safety_rejections` (Zip-Slip, >256 MiB / >4,096 entries, collisions, stem deduplication)
    - Covers `vision_remove_bg_cuts_subject_and_rejects_blank_image` (transparent PNG cutout + blank-canvas rejection)
    - Covers `optimize_pdf_shrinks_heavy_pdf_preserves_text_and_rejects_over_200_pages`
    - Covers `native_zero_ffmpeg_audio_caf_and_flac_roundtrips` (`WAV <-> CAF`, `WAV <-> FLAC`, `CAF -> FLAC`, `FLAC -> CAF`, `FLAC -> M4A`, `CAF -> M4A`, `FLAC -> ALAC`, `ALAC -> FLAC`, zero FFmpeg binary)
    - Plus native `EPUB 3`, PDF multi-page rasterization + Vision OCR + merge, image matrix + EXIF/GPS, >40 MP / HTML LFI safety, FFmpeg/AVFoundation video + mid-flight cancellation, read-only fallback, OOXML + templates.
  - **`tests/host_ipc_integration.rs`**: **8/8 passed**
    - Verifies exact command & camelCase alignment across all **27** Tauri commands and all **11** host-emitted events between Rust and `src/ipc.ts`
    - Verifies `unix_socket_ipc_roundtrips_permissions_and_stale_recovery` (`0600` owner-only socket, live command roundtrips, stale socket cleanup/recovery)
    - Verifies bridge & WAV resource resolution in dev and relocated `.app` layouts, `Info.plist` `plutil -lint`, screen clamping, and Tauri capabilities consistency.
  - **`tests/stress_and_perf.rs`**: **2/2 passed** (`2.05 s`)
    - **`stress_50_cycle_trigger_wheel_escape_and_multimonitor_clamping`**: Drives 50 rapid trigger -> wheel open -> pointer drag -> global Escape / drag-end -> gate reset cycles across 4 simulated multi-monitor layouts (Retina + 1x left + top-notch + ultrawide side-Dock) with 0 stuck `press_gate` / `order_gate` flags, 0 orphaned wheel `is_open` / `is_drag` states, and 100% bounded screen coordinates.
    - **`stress_20_plus_10_staging_cleanup_policy_latency_and_cli_startup_perf`**:
      - Executes **20 sequential successful conversions** (`Data`, `Document`/`EPUB`, `Image`, `ZIP`) + **10 intentionally failing/cancelled/mid-job `Drop` conversions** (pre-cancelled, corrupt JSON, Zip-Slip, missing source, unsupported target, read-only aborted `JobStaging`) in an isolated `TempDir`: **0 leftover `.flip-job-*` or `.flip-probe-*` directories**.
      - `policy::outputs` latency: **9.71 µs** single file, **1.31 ms** max 128-file batch, **21.15 µs** avg over 500 mixed lookups (target `< 5 ms`).
      - Compiled CLI `flip --version` startup (`target/debug/flip`): **6.50 ms – 9.23 ms** across 5 warm runs after macOS one-time `1.086 s` first-exec AMFI/syspolicyd cache scan on newly linked binary (target `< 300 ms`).

---

## 3. Release Bundle, Signing, DMG Packaging & Bundled CLI Verification

1. **Release `.app` Build (`./scripts/build-macos.sh --dmg`)**:
   - Built `src-tauri/target/release/bundle/macos/FLIP.app` (`22.29 MiB`, main `flip` Mach-O executable `20,493,328` bytes), embedded signed `Contents/Resources/flip-macos-bridge` (`464,816` bytes, `-rwxr-xr-x@`) and `Contents/Resources/resources/flip-tick.wav`.
2. **Code Signing (`codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/FLIP.app`)**:
   - **Passed**: `valid on disk`, `satisfies its Designated Requirement` (ad-hoc local signature).
3. **DMG Packager & Mount Verification (`src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg`, `8,917,516` bytes / `8.5 MiB`)**:
   - `codesign --verify --deep --strict --verbose=2` on `.dmg`: **Passed** (`valid on disk`, `satisfies its Designated Requirement`).
   - Mounted with `hdiutil attach -nobrowse -readonly -mountpoint ...`: verified `FLIP.app` and `Applications -> /Applications` symlink inside mounted volume, verified `codesign --verify --deep --strict --verbose=2` on mounted `FLIP.app`, and ejected cleanly with `hdiutil detach`.
4. **Live Bundled Release CLI Smoke Tests (`FLIP.app/Contents/MacOS/flip`)**:
   - **`--version` startup latency**: `flip 0.1.0`, **6.18 ms – 8.07 ms** across 5 runs.
   - **`ZIP` -> `UNZIP` roundtrip**: `convert hello.txt note.md --to ZIP` -> `hello.zip` (`255` B) -> `convert hello.zip --to UNZIP` -> extracted `hello/` matching byte-for-byte via `diff -u`.
   - **Zero-FFmpeg Audio (`WAV -> FLAC -> WAV` & `WAV -> CAF`)**: Both with default `PATH` and stripped `PATH=/usr/bin:/bin` (no Homebrew FFmpeg visible), `tick.wav` (`3.5K`) converted cleanly to `tick.flac` (`9.8K`), `tick.caf` (`7.0K`), and back from `tick.flac` to `tick 2.wav` (`7.0K`).
   - **Native `EPUB 3` (`MD -> EPUB -> TXT`)**: `book.md` -> `book.epub` -> `book.txt` cleanly with no Pandoc installed.
   - **`OPTIMIZE_PDF`**: `cover.png` (`1.8M`) -> `cover.pdf` (`2.9M`) -> `cover 2.pdf` (`174K`, `-94%` reduction cleanly).
   - **Apple Vision `REMOVE_BG`**: `cover.png` (`1.8M`) -> `cover 2.png` (`1.3M`, transparent cutout PNG cleanly).

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
