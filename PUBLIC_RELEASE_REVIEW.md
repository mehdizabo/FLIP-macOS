# Pre-Release Readiness Review (`FLIP-macOS-port` v0.1.0)

This review evaluates the current source repository, local Apple Silicon (`arm64`) `.app` and `.dmg` release artifacts, and verification evidence. Automated checks do **not** replace physical WindowServer desktop acceptance or Apple Developer ID notarization.

## Overall Status

- **Source Repository & Local Ad-Hoc Developer Builds**: **READY (with documented local-build caveats)** — all automated Rust, TypeScript, DOM E2E, stress/latency, privacy audit, `.app` bundle, `.dmg` mount, isolated installer, Homebrew Cask syntax/style, and live bundled CLI smoke tests pass on `macOS 27.2.1` (`arm64`).
- **Public Unattended End-User Binary Distribution**: **HOLD PENDING NOTARIZATION & PHYSICAL GUI QA** — public distribution requires an Apple Developer ID signature + Apple notarization (currently ad-hoc signed locally) and completion of the physical WindowServer checklist in [ACCEPTANCE.md](ACCEPTANCE.md).

## Verification Matrix

| Area | Evidence & Scope | Status |
|---|---|---|
| **Native Swift Bridge (`flip-macos-bridge`)** | Compiled cleanly via `/usr/bin/swiftc -O -target arm64-apple-macosx15.0` with Command Line Tools only | **PASS** |
| **Frontend TypeScript, Vite & Bun Suite** | `npx tsc --noEmit` (0 errors), `npm run build`, `bun test` (unit + contract + i18n + in-memory DOM E2E across all 4 views) | **PASS** |
| **Rust Linting & Test Suite** | `cargo clippy --all-targets -- -D warnings` (0 warnings), `cargo test` offline suite (`flip_lib`, `engines_integration`, `host_ipc_integration`, `stress_and_perf`; 1 `#[ignore]` live GitHub network test) | **PASS** |
| **Bundle, Codesign & DMG Packaging** | `./scripts/build-macos.sh --dmg` + `./scripts/package-dmg.sh --update-cask`, `codesign --verify --deep --strict` on `.app` & `.dmg`, `hdiutil attach` mount & `/Applications` symlink check | **PASS (Ad-Hoc Signed)** |
| **Local Installer & Uninstaller** | Isolated `HOME` / `--dest` dry-run, install, Quick Actions `plutil -lint`, CLI symlink, fail-fast refusal on regular file collision, and `--uninstall` cleanup | **PASS (Isolated Sandbox)** |
| **Homebrew Cask** | `ruby -c packaging/homebrew/flip.rb` and `brew style packaging/homebrew/flip.rb` (with `brew developer off` afterward) | **PASS** |
| **Bundled Release CLI Smokes** | Live `FLIP.app/Contents/MacOS/flip` CLI checks across `CSV <-> XLSX <-> JSON`, `new yaml/pdf`, `FB2`/`RST`/`EPUB`, `SVG -> ICO`, `ANIMATE_GIF`, `ZIP`/`UNZIP`, `WAV -> FLAC/CAF`, `OPTIMIZE_PDF`, `REMOVE_BG`, and `flip preview` | **PASS** |
| **Repository Privacy & Source Export** | `./scripts/audit-repository.sh` and `./scripts/export-source.sh` clean ZIP verification | **PASS** |
| **Physical Finder Drag, Hotkeys & Spaces** | Real Finder drag + `CGEventTap`, Mouse Buttons `2`/`3`/`4`, `Option+Shift+Space`, Full-Screen Spaces, mixed-DPI multi-monitor edges | **OPEN ([ACCEPTANCE.md](ACCEPTANCE.md))** |
| **Live WindowServer UI, Audio & VoiceOver** | Physical `WKWebView` interaction, acoustic `flip-tick.wav`, Force Touch haptics, `Reduce Motion`/`Reduce Transparency`, VoiceOver (`Cmd+F5`), LaunchAgent reboot persistence | **OPEN ([ACCEPTANCE.md](ACCEPTANCE.md))** |
| **Apple iWork & Optional Pandoc Live Host** | `Pages.app` / `Numbers.app` / `Keynote.app` and `pandoc` are not installed on this host; missing-tool handling and zero-Pandoc pure-Rust engines verified instead | **UNVERIFIED ON HOST** |
| **macOS 15 / macOS 26 & CI Runner** | Built with `-target arm64-apple-macosx15.0`, tested on `macOS 27.2.1` (`26B131`); physical macOS 15/26 hardware and live GitHub `macos-15` CI not yet executed | **OPEN** |
| **Developer ID Signing & Notarization** | Hooks present in `scripts/sign-local.sh` and `scripts/package-dmg.sh`; requires paid Apple Developer certificate and `notarytool` credentials | **BLOCKED FOR PUBLIC BINARY** |

## Key Safety & Hardening Controls Verified

- Zero-overwrite atomic `.flip-job-<uuid>` staging with `renamex_np(RENAME_EXCL)` commits and Finder collision suffixes (`stem 2.ext`).
- Symlink, Finder alias, undownloaded `.icloud` stub, and non-local volume rejection (`validate_input_files`).
- `40 MP` image decode ceilings, `200`-page PDF caps, `256 MiB` / `4,096`-entry Zip-Slip limits across archive/OOXML/EPUB/XLSX containers, XXE/DOCTYPE stripping, and offline SVG/HTML LFI sanitization.
- Owner-only (`0600`) Unix socket IPC with stale-socket recovery and redacted Markdown diagnostic exports (`Copy Diagnostics`).

## Recommended Release Procedure

1. Complete physical desktop QA in [ACCEPTANCE.md](ACCEPTANCE.md) and trigger `.github/workflows/ci.yml` on `macos-15`.
2. Sign and notarize `FLIP.app` and `FLIP-0.1.0-macOS-arm64.dmg` with a valid Developer ID certificate (`APPLE_SIGNING_IDENTITY`, `APPLE_NOTARY_PROFILE`).
3. Re-run `./scripts/package-dmg.sh --update-cask`, `./scripts/audit-repository.sh`, and `./scripts/export-source.sh` before publishing GitHub Releases or updating Homebrew tap URLs.
