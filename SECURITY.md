# Security Policy

## 100% Local Processing & Network Surface

- **Offline by design**: All file inspections, thumbnails, OCR (`Apple Vision`), image/PDF/document/data/media conversions, and `Create New File` actions run **100% locally** on your Mac. FLIP never uploads files, filenames, clipboard contents, or telemetry to any server, and requires no account.
- **Optional Update Check only**: The only network request FLIP can perform is an explicit, user-triggered **Check for Updates…** action (`About` tab or menu bar item). It invokes `/usr/bin/curl -fsSL --max-time 8 --proto =https` against `https://api.github.com/repos/thetinyfoundry/FLIP/releases/latest` (or a configured `https://` GitHub repository URL), rejects non-`https://` endpoints, sanitizes release links to `https://github.com/...`, and validates URLs again in `open_release_url` before launching `/usr/bin/open`.

---

## File & Path Isolation Architecture

FLIP treats untrusted files defensively across `src-tauri/src/core/policy.rs`, `src-tauri/src/core/staging.rs`, and `native-bridge/Sources/MediaAndDocs.swift`:

1. **Input Path & Volume Guards (`validate_input_files`)**:
   - Accepts only absolute local paths between **1 and 128** files per batch.
   - Rejects **POSIX symbolic links** (`lstat` / `symlink_metadata`) and **macOS Finder Aliases** (`com.apple.FinderInfo` `kIsAlias` flag `0x8000`).
   - Rejects undownloaded **iCloud placeholders** (`SF_DATALESS` / `.*.icloud` stubs).
   - Rejects **non-local / network volumes** (`statfs` `MNT_LOCAL` check with local APFS/HFS/tmpfs verification).
   - Rejects **directories** (except verified iWork package bundles `.pages`, `.numbers`, `.key`).
   - Enforces file size bounds: **100 MiB** for regular files, **2 GiB** for media files, **2 MiB** for text/Markdown reads, **20 MiB** for HTML imports, and **200 pages** maximum for PDF rendering/extraction.

2. **Atomic Staging & Non-Overwriting Commits (`JobStaging`)**:
   - Every conversion executes inside an isolated temporary staging folder `.flip-job-<32-hex-uuid>` beside the target output folder.
   - Outputs are validated (container/header signatures, image decode, `%PDF-` / `%%EOF` check, non-empty size) **before** committing.
   - Commits use atomic non-overwriting **`renamex_np(..., RENAME_EXCL)`** on macOS (falling back safely to `hard_link` / `O_CREAT | O_EXCL` on exotic filesystems) with Finder-style collision suffixing (`<stem> 2.<ext>`). Existing files and originals are **never** overwritten or modified.
   - Multi-page commits roll back partial outputs if any page fails.
   - `Drop` and explicit cleanup verify `.flip-job-<32-hex>` directory ownership, refuse to follow symlinks during recursive deletion, and strip read-only attributes (e.g., Office lock files) before removal.

3. **Decompression Bomb & Resource Guards**:
   - **Images**: **40 Megapixel** (`40,000,000` pixels) limit enforced both **pre-decode** (`CGImageSourceCopyPropertiesAtIndex` / header dimensions) and **post-decode** across `ImageIO`, Rust `image`, OCR, and `imagesToPDF`.
   - **ZIP / Office / Book Containers**: `.docx`, `.odt`, `.epub`, `.xlsx`, and `.pptx` containers are bounded at **4,096 entries** and **256 MiB** (`256 * 1024 * 1024` bytes) across both declared ZIP headers and actual decompressed byte counts (`validate_container`). Package-format iWork archives additionally check `/usr/bin/unzip -l` under bounded output and timeout guards before extraction.

4. **Offline HTML Local File Inclusion (LFI) & Script Guard**:
   - Before parsing HTML via `NSAttributedString`, `sanitizeHTMLForOfflineImport` strips `<script>`, `<iframe>`, `<object>`, `<embed>`, `<frame>`, `<frameset>`, `<link>`, `<base>`, and `<meta http-equiv="refresh">` elements, neutralizes external/local `src`, `srcset`, `background`, `poster`, `data`, and `file:` `href` attributes (preserving safe inline `data:image/`), and strips CSS `@import` and non-`data:image/` `url(...)` rules.
   - Optional `Pandoc` subprocesses fail closed if remote/missing resources are referenced offline.

---

## Logs & Redacted Diagnostics

- Runtime events are written to bounded rotating JSONL files in `~/Library/Logs/FLIP/events.jsonl` (rotated to `events.jsonl.previous` at **2 MiB**). User home directories and per-user temporary/system paths are reduced to `~` **when each line is written**, so usernames never reach the log file; file and folder names below the home directory are still recorded.
- Exporting via **Copy Diagnostics** (`Settings → About` or error views in the Wheel / Batch window) automatically redacts `/Users/<username>`, `/private/Users/<username>`, custom home directories, per-user macOS temporary/system paths (`/private/var/folders/...`, `/var/folders/...`, `/private/var/mobile`, `/private/var/root`, `/var/mobile`, `/var/root`), JSON-escaped newline/tab path prefixes (`\n/Users/...`), and percent-encoded URL user paths (`%2FUsers%2F...`, `%2Fprivate%2Fvar%2Ffolders%2F...`) to `~`.

---

## macOS Permissions Scope

- **Accessibility**: Used only for drag-trigger `CGEventTap` interception while a drag pasteboard is active and sending `Return` for Finder inline rename on `Create New File`.
- **Finder Automation**: Used only to read Finder's selected items and active folder.
- **Never requested**: FLIP never requests **Screen Recording**, Full Disk Access, or Contacts/Camera/Microphone permissions.

---

## Reporting a Vulnerability

Process isolation and input bounds reduce risk, **but parsers and external tools (`FFmpeg`, `Pandoc`, `NSAttributedString`, `PDFKit`) should still treat untrusted documents with caution**.

- **Do not** attach sensitive files or unredacted logs to public GitHub issues.
- Use **GitHub Private Vulnerability Reporting** on the repository if enabled, or contact the maintainer privately via the repository profile before posting exploit details or PoC files publicly.
