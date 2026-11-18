# Test & Compatibility Results (`FLIP-macOS-port` v0.1.0)

Date: 2026-11-18  
Commit Scope: Steps A–D complete (Native Pure-Rust EPUB 3 Engine, Interactive Preferred Format Ordering, VoiceOver Live Announcer, CI Workflow, Porting & Acceptance Docs, DMG Packager)

---

## 1. Host Environment & OS Compatibility Matrix

Verified host hardware & OS:
- **OS**: `macOS 27.2.1` (Build `26B131`, Golden Gate)
- **Architecture**: Apple Silicon (`arm64`)
- **Toolchain**:
  - Swift: `Apple Swift 6.3.2` (`swift-driver 1.148.6`, target `arm64-apple-macosx15.0` binary)
  - Rust / Cargo: `rustc 1.100.0` / `cargo 1.100.0`, `tauri-cli 2.12.4`
  - Node.js: `v26.11.1`, Bun: `1.4.4`
- **Optional External Engines on Test Host**:
  - `FFmpeg`: Present (`/opt/homebrew/bin/ffmpeg` `8.1.1`)
  - `Pandoc`: Not installed on host (built-in EPUB 3 runs completely Pandoc-free; Pandoc is only required for `FB2`/`RST`)
  - `Pages.app` / `Numbers.app` / `Keynote.app`: Not installed in `/Applications` on host

| Target Platform | Architecture | Status | Notes |
|---|---|---|---|
| **macOS 27.2.1 (`26B131`)** | Apple Silicon (`arm64`) | **Verified (Automated Tests + `.app`/`.dmg` Build + CLI Smoke)** | Primary target; native `flip-macos-bridge`, Tauri v2 `.app`, and `.dmg` built and verified here. Live WindowServer GUI drag-drop & VoiceOver walkthrough checklist documented in [`ACCEPTANCE.md`](ACCEPTANCE.md). |
| **macOS 26 (Tahoe / Liquid Glass)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Deployment target is macOS 15.0+; CSS/WebKit Liquid Glass properties align, not tested on physical macOS 26 hardware. |
| **macOS 15 (Sequoia fallback)** | Apple Silicon (`arm64`) | **Not Executed on Host** | Minimum deployment target (`-target arm64-apple-macosx15.0`, `minimumSystemVersion: 15.0`); not booted on physical macOS 15 hardware. |
| **Intel (`x86_64`)** | `x86_64` | **Not Supported / Not Tested** | Current bridge and DMG build target `arm64` only. |

---

## 2. Automated Verification Summary

### 2.1 Native Swift/AppKit Bridge (`./native-bridge/build.sh`)
- **Result**: Passed (`exit 0`)
- **Artifact**: `native-bridge/bin/flip-macos-bridge` (`437,312` bytes, `arm64`, `-rwxr-xr-x@`)
- Compiled via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` using Command Line Tools only.

### 2.2 TypeScript, Vite Production Build, and Bun Unit Tests
Command: `npx tsc --noEmit && npm run build && bun test`
- **TypeScript (`tsc --noEmit`)**: 0 errors
- **Vite Production Build (`npm run build`)**: 20 modules transformed (`dist/index.html` 0.39 kB, CSS 25.06 kB, JS 142.54 kB)
- **Bun Test Suite (`bun test`)**: **29 passed, 0 failed** across 2 files (`524` `expect()` assertions):
  - `src/ipc.test.ts`: **3/3 passed** (Rust `AppPreferences` schema alignment including `HEIC` in document and `TIFF` in PDF preferences plus custom-order preservation on default backfill, mock IPC commands & events, and `DOC_NATIVE_EXTS` / `DOC_PANDOC_ONLY_EXTS` / `inspectFilesLocally` verifying `EPUB` works without Pandoc while `FB2`/`RST` remain Pandoc-gated)
  - `src/wheelGeometry.test.ts`: **26/26 passed** (wheel format ordering, `reorderFormatList` / `moveFormatToIndex` clamping, SVG chamfered sector geometry for 1–8 sectors, 3° angular hysteresis + 0°/360° wraparound, progress arc geometry, pagination, hash routing, HTML escaping, Option-modified key token mapping, batch handoff, and VoiceOver live announcement formatters for `formatSectorAriaLabel`, open mode, page flips, 25/50/75% progress milestones, and completion/error/cancel states)

### 2.3 Rust Clippy, Unit & Integration Test Suite (`cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Clippy (`cargo clippy --all-targets -- -D warnings`)**: **Passed with 0 warnings**
- **Cargo Test Suite (`cargo test` offline default suite)**: **70 passed, 0 failed, 1 ignored**
  - **`flip_lib` unit tests**: **52/52 passed** (+ 1 `#[ignore]` live network test `live_github_update_check_and_tags_fallback`)
    - Includes `engines::document::tests::epub3_writer_and_reader_produce_valid_container_and_wellformed_xhtml5`, `epub_reader_respects_spine_order_and_lexical_fallback_when_spine_incomplete`, and `html_comment_apostrophes_scripts_cdata_and_duplicate_heading_ids_handled_safely`
    - Includes `core::policy::tests::txt_and_document_outputs_with_and_without_pandoc` (asserting `DOC_NATIVE_EXTS` contains `epub` and `DOC_PANDOC_ONLY_EXTS` keeps only `fb2`, `rst`) and `core::preferences::tests::sanitize_preserves_custom_order_while_backfilling_new_defaults`
    - Includes `core::updater`, `core::logging` (redacted diagnostic bundle), `core::input_gate`, `core::naming`, `core::policy`, `core::preferences`, `core::quick_actions`, `core::staging`, `core::startup`, `cli`, `engines::data`, `host`, `host::monitor`, `host::tray`
  - **`tests/engines_integration.rs`**: **11/11 passed**
    - Includes `native_epub3_md_epub_txt_and_pdf_epub_roundtrips_without_pandoc` verifying Pandoc-free `MD -> EPUB -> TXT` and `PDF -> EPUB` roundtrips plus ZIP entry `0` = `mimetype` (`CompressionMethod::Stored`, exact `application/epub+zip` payload)
    - Includes image matrix (`PNG`, `JPG`, `WEBP`, `HEIC`, `BMP`, `TIFF`, `GIF`, `COMBINE_PDF`, alpha flattening, EXIF orientation, GPS strip), structured data matrix (`JSON`, `YAML`, `TOML`, `XML`, `CSV`, `HTML`, `MD`, `PDF`), PDF multi-page rasterization + Vision OCR + merge, > 40 MP / > 256 MiB / zip-bomb / HTML LFI guards, media/FFmpeg + cancellation, read-only fallback, and Create New File OOXML + templates
  - **`tests/host_ipc_integration.rs`**: **7/7 passed**
    - Verifies exact command & camelCase parameter alignment across all **27** Tauri commands and all **10** host-emitted events between Rust and `src/ipc.ts`
    - Verifies bridge & WAV resource resolution in dev and relocated `.app` layouts, `Info.plist` `plutil -lint`, screen clamping, and config/capabilities consistency

---

## 3. Release Bundle, Signing, DMG Packaging & Bundled CLI Verification

1. **Release `.app` Build (`./scripts/build-macos.sh --dmg`)**:
   - Built `src-tauri/target/release/bundle/macos/FLIP.app` (`21.99 MiB`), embedded `Contents/Resources/flip-macos-bridge` (`437,312` bytes, `-rwxr-xr-x@`) and `flip-tick.wav`.
2. **Code Signing (`codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/FLIP.app`)**:
   - **Passed**: `valid on disk`, `satisfies its Designated Requirement` (ad-hoc local signature).
3. **DMG Packager & Mount Verification (`src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg`, `8.9M`)**:
   - `codesign --verify --strict --verbose=2` on `.dmg`: **Passed** (`valid on disk`, `satisfies its Designated Requirement`).
   - Mounted with `hdiutil attach -nobrowse -readonly -mountpoint ...`: verified `FLIP.app` and `Applications -> /Applications` symlink inside mounted volume, verified `codesign --verify --deep --strict --verbose=2` on mounted `FLIP.app`, and detached cleanly with `hdiutil detach`.
4. **Live Bundled CLI Native EPUB 3 Smoke Test (`FLIP.app/Contents/MacOS/flip`)**:
   - `FLIP.app/Contents/MacOS/flip convert <tmp>/smoke.md --to EPUB`: converted Markdown to EPUB 3 cleanly with **no Pandoc installed**; Python `zipfile` inspection confirmed entry `0` is `mimetype` (`compress_type: 0` = `ZIP_STORED`, `extra_len: 0`, `20` bytes `application/epub+zip`) alongside `META-INF/container.xml`, `OEBPS/content.opf`, `OEBPS/nav.xhtml`, `OEBPS/style.css`, and `OEBPS/chapter.xhtml`.
   - `FLIP.app/Contents/MacOS/flip convert <tmp>/smoke.epub --to TXT`: extracted `smoke.txt` preserving heading, paragraph, and bullet list text cleanly.
   - `FLIP.app/Contents/MacOS/flip convert <tmp>/nasty.html --to EPUB -> TXT`: verified HTML comments with apostrophes, `<script>`/`<style>` blocks containing `<`/quotes, `<![CDATA[...]]>`, unquoted URL attributes ending in `/`, and duplicate heading `id`s parse cleanly into valid XML (`chapter.xhtml`, `nav.xhtml`) and expected `TXT`.

---

## 4. Unexecuted / Unverified Scenarios on This Host

The following paths were **not** executed live on this machine and are tracked for physical desktop verification in [`ACCEPTANCE.md`](ACCEPTANCE.md):

- **Live WindowServer GUI, Finder & Accessibility Walkthrough**:
  - Interactive Finder drag + trigger in both orders (`Drag -> Trigger` and `Trigger -> Drag`), `NSPasteboard(name: .drag)` live drag inspection, Mouse Button `2`/`3`/`4` triggers, global Idle shortcut (`Option+Shift+Space`), full-screen Spaces overlay, multi-monitor mixed-DPI clamping, live Finder inline rename (`Return`), Light/Dark appearance, `Reduce Motion`, `Reduce Transparency`, and live **VoiceOver** navigation + live announcements (`aria-live="polite"`)
  - Live `WKWebView` clicks/drag-drop inside Settings (`Preferred Format Order` chips + `Reset to Default`), Wheel, Onboarding, and Batch windows (logic covered by TypeScript build + `bun test` mocks; unexecuted in a live desktop WebView)
- **Missing Optional External Apps / Tools on Host**:
  - **Pandoc** (`FB2` and `RST` conversions): Not installed on host (`pandoc not found`); capability gating verified via Rust & Bun unit tests, runtime Pandoc subprocess execution unverified here.
  - **Apple iWork** (`Pages.app`, `Numbers.app`, `Keynote.app` AppleScript automation): Not installed in `/Applications`; missing-app errors and `.pages` QuickLook extraction tested instead.
- **GitHub Actions CI Execution**:
  - `.github/workflows/ci.yml` created and aligned with all verified local steps, not yet triggered on a live GitHub `macos-15` runner.
- **Developer ID Signing & Apple Notarization**:
  - Scripts (`scripts/sign-local.sh`, `scripts/package-dmg.sh`) support `APPLE_SIGNING_IDENTITY` and `xcrun notarytool` / `stapler`, but only ad-hoc local signing (`--sign -`) was executed because no paid Apple Developer certificate is configured.
