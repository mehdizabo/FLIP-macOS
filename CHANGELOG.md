# Changelog

All notable changes to **FLIP for macOS** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Smart vs Raw / Keep-Code conversion mode** toggle in the Wheel, Batch window and CLI; the `R` key toggles it globally, also during a Finder drag and in idle mode.
- **Image Tools** (the original FLIP 0.6 "Aura" tools, adapted to the port), available from the CLI (`--to CROP_9_16`, …) and from a new **Tools face** of the Wheel for any selection of images:
  - **Crop** to `9:16` (Reel), `4:5`, `1:1` or `16:9` (centred, exact ratio), **Rotate** 90° left/right and 180°, **Flip** horizontally/vertically, **Scale** to 50 %, 25 %, 1080p, 720p (never upscales) or **under 1 MB**, **Colour palette** (a strip of the dominant colours, as PNG) and **Remove EXIF & location** (lossless for JPEG and PNG; the camera orientation is baked into the pixels first).
  - Tools keep the source format, write `<name>-crop-9x16.jpg`-style results through the same atomic, never-overwrite staging as conversions, and never touch the original.
  - The wheel's top bar gets a **Tools / Formats** switch; `T` flips faces (also during a Finder drag, forwarded by the bridge's event tap like `R`). Crop, Rotate and Scale open a sub-page of presets with an "up" slot; during a drag, hovering a group for 300 ms opens it like `More ▸`.
  - EN/IT titles, VoiceOver labels and parity tests that keep the TypeScript token list identical to the Rust one.
- Parity guard: a Rust test embeds the original FLIP's extension/output tables and fails if this port stops supporting any input extension or output of the original (today: 0 missing; the port is a superset).

### Changed

- **Wheel redesigned as a Logi Options+-style actions ring** of circular bubbles on Apple Liquid Glass optics, with 120 fps magnetic spring physics (`wheelPhysics.ts`) and a circular icon-well hub. This supersedes the chamfered sectors and `68 px` QuickLook hub described under `0.1.0`.

### Fixed

- Drag: cursor jumping, hover flicker, hardened drag trigger, spring/WKWebView sync, and an `Esc` race that left the overlay open.
- Native `NSWindow` transparency is enforced on the wheel overlay.
- Dev/rebuild: arm64 bridge `SIGKILL` (137) after rebuilding `flip-macos-bridge`.
- Create New: Finder inline rename now receives `Return` via `cghidEventTap` after activation; the front Finder window is preferred and `open -R` is the reveal fallback.
- Documents: canvas-only HTML pages convert to text/Markdown instead of an empty result.
- Privacy audit handles NUL-delimited filenames and punctuation-delimited paths.

- Wheel physics: pointer events closer than 2 ms to the previous step no longer advance the spring by a fixed 6 ms (a 1 kHz pointer made the spring run ~60x faster than real time); the elapsed time is accumulated instead.

### Performance

- Release binary halved (21.0 MB to 10.8 MB): thin-LTO release profile (`codegen-units = 1`, `strip`) and `tokio` limited to `rt-multi-thread` + `time`. A clean release build takes about twice as long.

### Security & Privacy

- Release binaries no longer embed the build machine's checkout path or consult the source tree for the bridge and tick sound (debug builds keep the fallback; unbundled release runs can set `FLIP_BRIDGE_PATH`).
- `~/Library/Logs/FLIP/events.jsonl` redacts home directories and per-user temp paths to `~` when each line is written, not only when diagnostics are exported.

## [0.1.0] - 2026-11-18

Initial open-source release of **FLIP for macOS** (Apple Silicon `arm64`, macOS 15.0+ Sequoia / macOS 26 Tahoe / macOS 27 Golden Gate).

### Added

- **Liquid Glass Radial Wheel & UI Surfaces**:
  - Cursor-centered `440x440` Liquid Glass radial wheel (`#wheel`) with translucent chamfered SVG sectors, directional specular rim highlights, `68 px` QuickLook center hub dead-zone, `3°` angular hysteresis, and `<= 8` sectors per page with `300 ms` drag-dwell `More ▸` / `Back ◂` pagination.
  - Radial progress arc with percentage readout, `Convert` / `New File` mode pill (`Tab`), keyboard (`1`–`8`, Arrows, `Enter`/`Space`, `Esc`) and full **VoiceOver** `aria-live` announcements.
  - Native **English / Italiano (`EN` / `IT`)** live UI switcher (`Settings → General`).
  - Acoustic tick (`flip-tick.wav` via `NSSound`) + Force Touch trackpad haptics (`NSHapticFeedbackManager`) with live **Test Wheel Sound & Haptics** preview button.
  - `prefers-reduced-motion` and `prefers-reduced-transparency` accessibility fallbacks.
  - Multi-tab **Settings** (`#settings`) including interactive **Preferred Format Order** editor, **Batch / Multi-Format Converter** (`#batch`), and **First-Run Onboarding** (`#onboarding`).
- **Native macOS Host, Triggers & IPC**:
  - Accessory menu bar app (`LSUIElement`) with `TrayIconBuilder` menu and zero Dock clutter.
  - Single-instance `flock` lock + `0600` owner-only Unix domain socket IPC (`flip-instance.sock`) with stale socket recovery.
  - Persistent Swift/AppKit helper (`flip-macos-bridge`) using `CGEventTap` + `NSEvent` for Drag + Trigger (`DownArrow` default, Mouse Buttons `2`/`3`/`4`/`5`), both drag-first and trigger-first orderings, Idle global shortcut (`Option+Shift+Space`), full-screen Spaces overlay, and multi-monitor mixed-DPI clamping.
  - Finder Quick Actions (`~/Library/Services/Convert with FLIP.workflow` and `New File with FLIP.workflow`), `flip://` URL scheme routing, and `LaunchAgents` auto-start support.
- **100% Local Conversions & Apple Framework Integrations**:
  - **Images**: `PNG`, `JPG/JPEG`, `WEBP`, `HEIC/HEIF`, `AVIF`, `TIFF`, `GIF`, `BMP`, `ICNS`, `ICO`, and offline-sanitized `SVG` input rasterization -> `PNG`, `JPG`, `WEBP`, `HEIC`, `PDF`, `TIFF`, `GIF`, `BMP`, `ICO`, `ICNS`, `TXT` (Vision OCR), `REMOVE_BG` (`Vision` `VNGenerateForegroundInstanceMaskRequest`), `ZIP`, plus multi-image `COMBINE_PDF` and **`ANIMATE_GIF` (`GIF+`)**. Includes EXIF orientation baking and optional GPS metadata stripping.
  - **PDF**: `PDFKit` / Quartz rasterization (`144`/`200`/`300` DPI), Apple `Vision` OCR (`VNRecognizeTextRequest`), `OPTIMIZE_PDF` (text-preserving compression), `Merge PDFs`, and export to `TXT`, `DOCX`, `MD`, `HTML`, `RTF`, `ODT`, `EPUB`, `FB2`, `RST`, `ZIP`.
  - **Documents**: `TXT`, `MD`, `HTML`, `RTF`, `DOC`, `DOCX`, `ODT`, `EPUB`, **`FB2`**, **`RST`** -> paginated A4/Letter `PDF` (`NSAttributedString` / CoreText), `DOCX`, `MD`, `HTML`, `TXT`, `RTF`, `ODT`, native Pure-Rust **`EPUB 3`**, native Pure-Rust **`FB2`**, native Pure-Rust **`RST`**, image exports (`PNG`/`JPG`/`WEBP`/`HEIC`), and `ZIP`. Optional `Pandoc` support when installed, plus `FLIP_DISABLE_PANDOC` / `FLIP_DISABLE_FFMPEG` environment overrides.
  - **Structured Data**: Native Rust cross-converter across `JSON` (JSON5-tolerant), `YAML`, `TOML`, `XML`, `CSV` (dot-flattening), native bidirectional **`XLSX`** (shared strings, inline strings, cell formulas, numeric/boolean/date types, container bounds), `TXT`, `MD`, `HTML`, `PDF`, `ZIP`.
  - **Audio & Video**: Zero-FFmpeg native `AVFoundation` + CoreAudio `ExtAudioFile` + `/usr/bin/afconvert` conversions (`M4A`, `WAV`, `FLAC`, `AIFF`, `ALAC`, `CAF`, plus video `MP4`/`MOV`/`M4V`/animated `GIF`), with optional `FFmpeg` support for `MP3`, `OGG`, `OPUS`, `AAC`, `MKV`, `WEBM`, `AVI`.
  - **Apple iWork**: `.pages`, `.numbers`, `.key` export via QuickLook + AppleScript app automation.
  - **Archives**: Pure-Rust `ZIP` creation and collision-safe `UNZIP` subfolder extraction.
- **Create New File**:
  - Instant creation in the active Finder folder (`~/Desktop` fallback) with automatic Finder reveal and inline rename (`Return` — strictly suppressed on conversions).
  - Built-in kinds: Folder, `.txt`, `.md`, `.rtf`, `.docx`, `.xlsx`, `.pptx`, **Blank PDF (`.pdf`)**, `.html`, `.json`, `.csv`, **`.yaml`**, **`.toml`**, **`.xml`**, executable `.sh` (`chmod +x`), `.py`, `.swift`, **From Clipboard** (`.png`, `.pdf`, `.rtf`, `.webloc`, `.txt`), Apple iWork (`.pages`, `.numbers`, `.key`), and custom templates (`~/Library/Application Support/FLIP/Templates`) with `{{date}}`, `{{time}}`, `{{user}}`, `{{folder}}` expansion.
- **Atomic Safety & Defensive Guards**:
  - Isolated `.flip-job-<uuid>` staging, `renamex_np(RENAME_EXCL)` collision-free commits with Finder counters (`stem 2.ext`), refusal of symlinks/Finder aliases/undownloaded `.icloud` placeholders/network volumes, 128-file batch ceiling, `40 MP` / `200-page` / `256 MiB` / `4,096`-entry Zip-Slip limits, XXE/DOCTYPE stripping, and offline SVG/HTML LFI sanitization.
- **Integrated CLI & Testing**:
  - `flip` CLI (`convert`, `new`, `wheel`, `settings`, `batch`, `preview --out <path.svg>`, `quit`, `--help`, `--version`).
  - Automated DOM + SVG shim E2E test suite (`src/views.e2e.test.ts`), frontend unit tests (`bun test`), Rust unit/integration/IPC suites, and 50-cycle + 20+10 stress/latency benchmarks (`tests/stress_and_perf.rs`).
- **Packaging, Local Installer, Integrations & Governance**:
  - One-command build & ad-hoc/Developer ID signing (`scripts/build-macos.sh --dmg`, `scripts/sign-local.sh`), compressed `.dmg` packager with SHA-256 & Homebrew Cask sync (`scripts/package-dmg.sh --update-cask`), macOS local installer & uninstaller (`scripts/install-local.sh [--uninstall]`), Quick Actions installer & uninstaller (`scripts/install-quick-actions.sh [--uninstall]`, plus live Settings UI removal), Raycast Script Commands (`integrations/raycast/flip-{convert,new-file,wheel}.sh`), Homebrew Cask (`packaging/homebrew/flip.rb`), repository privacy audit & clean source exporter (`scripts/audit-repository.sh`, `scripts/export-source.sh`), root governance docs (`ARCHITECTURE.md`, `CONTRIBUTING.md`, `ROADMAP.md`, `PUBLIC_RELEASE_REVIEW.md`), and GitHub Actions `macos-15` CI (`.github/workflows/ci.yml`).
