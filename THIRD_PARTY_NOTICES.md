# Third-Party Notices & Attributions

FLIP for macOS (`FLIP-macOS-port`) — source code, radial Liquid Glass UI, native Swift bridge, icons, and procedural acoustic wheel tick (`flip-tick.wav`) — is licensed under the **MIT License** (see [LICENSE](LICENSE)). This license does not replace the licenses of third-party components listed below.

## Upstream Attribution

| Component | Relationship | License / Upstream |
|---|---|---|
| **FLIP (Windows)** | Original upstream product, format policies, wheel UX geometry, and safety architecture | MIT License · [github.com/thetinyfoundry/FLIP](https://github.com/thetinyfoundry/FLIP) |

---

## Application Framework & Direct Rust Crates (`src-tauri/Cargo.toml` / `Cargo.lock`)

| Component | Resolved Version | License | Upstream |
|---|---|---|---|
| `tauri` | `2.12.6` | `Apache-2.0 OR MIT` | [github.com/tauri-apps/tauri](https://github.com/tauri-apps/tauri) |
| `tauri-build` (build) | `2.7.5` | `Apache-2.0 OR MIT` | [github.com/tauri-apps/tauri](https://github.com/tauri-apps/tauri) |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | [github.com/serde-rs/serde](https://github.com/serde-rs/serde) |
| `serde_json` | `1.0.154` | `MIT OR Apache-2.0` | [github.com/serde-rs/json](https://github.com/serde-rs/json) |
| `serde_yaml` | `0.9.34+deprecated` | `MIT OR Apache-2.0` | [github.com/dtolnay/serde-yaml](https://github.com/dtolnay/serde-yaml) |
| `toml` | `0.8.23` | `MIT OR Apache-2.0` | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |
| `quick-xml` | `0.37.5` | `MIT` | [github.com/tafia/quick-xml](https://github.com/tafia/quick-xml) |
| `csv` | `1.4.0` | `Unlicense/MIT` | [github.com/BurntSushi/rust-csv](https://github.com/BurntSushi/rust-csv) |
| `pulldown-cmark` | `0.12.2` | `MIT` | [github.com/raphlinus/pulldown-cmark](https://github.com/raphlinus/pulldown-cmark) |
| `image` | `0.25.10` | `MIT OR Apache-2.0` | [github.com/image-rs/image](https://github.com/image-rs/image) |
| `zip` | `2.6.1` | `MIT` | [github.com/zip-rs/zip2](https://github.com/zip-rs/zip2.git) |
| `uuid` | `1.29.3` | `Apache-2.0 OR MIT` | [github.com/uuid-rs/uuid](https://github.com/uuid-rs/uuid) |
| `chrono` | `0.4.45` | `MIT OR Apache-2.0` | [github.com/chronotope/chrono](https://github.com/chronotope/chrono) |
| `thiserror` | `2.0.21` | `MIT OR Apache-2.0` | [github.com/dtolnay/thiserror](https://github.com/dtolnay/thiserror) |
| `dirs` | `5.0.1` | `MIT OR Apache-2.0` | [github.com/soc/dirs-rs](https://github.com/soc/dirs-rs) |
| `tokio` | `1.54.1` | `MIT` | [github.com/tokio-rs/tokio](https://github.com/tokio-rs/tokio) |
| `libc` | `0.2.191` | `MIT OR Apache-2.0` | [github.com/rust-lang/libc](https://github.com/rust-lang/libc) |
| `tempfile` (dev-only) | `3.27.0` | `MIT OR Apache-2.0` | [github.com/Stebalien/tempfile](https://github.com/Stebalien/tempfile) |
| `sha2` (dev-only) | `0.10.9` | `MIT OR Apache-2.0` | [github.com/RustCrypto/hashes](https://github.com/RustCrypto/hashes) |

> **Transitive Rust Dependency Audit (`aarch64-apple-darwin`)**: All transitive crates in the resolved `Cargo.lock` dependency graph use permissive licenses (`MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `Unlicense`, `Zlib`, `Unicode-3.0`, `0BSD`, `CC0-1.0`, `BSL-1.0`) plus five `MPL-2.0` transitive crates (`cssparser` 0.37.0, `cssparser-macros` 0.7.1, `dtoa-short` 0.3.5, `option-ext` 0.2.0, `selectors` 0.38.0). **Zero** `GPL` or `LGPL` crates are compiled into FLIP. Note also that WebP encoding/decoding uses pure-Rust crates via `image` (`image-webp`), so no external `libwebp` C shared library is bundled.

---

## Frontend Dependencies (`package.json` / `pnpm-lock.yaml`)

| Package | Resolved Version | License | Scope |
|---|---|---|---|
| `@tauri-apps/api` | `2.12.5` | `Apache-2.0 OR MIT` | Runtime JS IPC bindings |
| `@tauri-apps/cli` | `2.12.5` | `Apache-2.0 OR MIT` | Build & packaging (devDependency) |
| `typescript` | `5.9.3` | `Apache-2.0` | Typechecking & compilation (devDependency) |
| `vite` | `6.4.6` | `MIT` | Frontend bundler (devDependency) |

---

## macOS System Frameworks (`native-bridge/build.sh`)

The native Swift helper (`flip-macos-bridge`) dynamically links Apple's system frameworks provided by macOS at runtime (not redistributed by FLIP):

- `Foundation`, `AppKit`, `CoreGraphics`, `CoreText`, `CoreML`, `ApplicationServices`
- `PDFKit`, `Vision`, `AVFoundation`, `QuickLookThumbnailing`, `UniformTypeIdentifiers`, `ImageIO`, `AudioToolbox`

---

## Optional External Tools — User-Installed Only (NOT Bundled)

**FFmpeg** (`ffmpeg` / `ffprobe`), **Pandoc** (`pandoc`), and **Apple iWork** (`Pages.app`, `Numbers.app`, `Keynote.app`) are **NOT bundled or redistributed** in the FLIP source repository, `FLIP.app`, or the `.dmg` installer:

| Tool | Typical Upstream License | Status in FLIP |
|---|---|---|
| **FFmpeg / ffprobe** | `LGPL-2.1-or-later` or `GPL-2.0-or-later` (depending on user build flags) | **Not bundled.** Optional user installation (e.g., `brew install ffmpeg` or custom path in Settings). |
| **Pandoc** | `GPL-2.0-or-later` | **Not bundled.** Optional user installation (e.g., `brew install pandoc` or custom path in Settings). |
| **whisper.cpp** (`whisper-cli`, older name `whisper-cpp`) | `MIT` according to its upstream repository; check the upstream license text | **Not bundled.** Optional user installation (e.g., `brew install whisper-cpp` or `scripts/install-whisper.sh`). |
| **OpenAI Whisper model weights, ggml format** (`ggml-small.bin`) | `MIT` according to their upstream repositories; check the upstream license text | **Not bundled.** Downloaded by the user into `~/Library/Application Support/FLIP/models/` (e.g., with `scripts/install-whisper.sh`). |
| **Apple Pages / Numbers / Keynote** | Proprietary (Apple) | **Not bundled.** Optional user installation invoked only via local Apple Events when present. |

**whisper.cpp** (the `whisper-cli` command line tool, plus FFmpeg used to decode the audio) and the **OpenAI Whisper model weights in ggml format** (`ggml-small.bin`) are only downloaded by the user, never bundled or redistributed by FLIP. Both are stated as MIT according to their upstream repositories; review the upstream license text before redistributing them.

If you distribute compiled binaries (`FLIP.app` or `.dmg`), retain this notice file and ensure compliance with the license terms (including `Apache-2.0` and `MPL-2.0` notice obligations) for all compiled dependencies in the binary bundle.
