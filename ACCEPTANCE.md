# FLIP for macOS — Physical GUI Acceptance Checklist

This checklist covers manual live-desktop QA on **macOS 27.2.1 (`26B131`) Golden Gate** (and macOS 15+ arm64) after building a fresh `FLIP.app` / `.dmg` with `./scripts/build-macos.sh --dmg`.

> Automated CLI, Rust engine, IPC, Swift bridge unit/integration, signature, and `.dmg` mount checks are recorded separately in [`TEST_RESULTS.md`](TEST_RESULTS.md). Items below require an active WindowServer desktop session, physical input devices, and Finder.

---

## 1. Bundle Installation, Launch & Permissions Gate
- [ ] Mount `src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg`, verify `FLIP.app` and the `/Applications` symlink appear cleanly, and drag `FLIP.app` to `/Applications` (or run `./scripts/install-local.sh` to install `FLIP.app`, clear `com.apple.quarantine`, register Quick Actions, and link `~/.local/bin/flip`).
- [ ] Launch `/Applications/FLIP.app` (on first launch of ad-hoc signed build, use Control-click -> Open or **System Settings -> Privacy & Security -> Open Anyway**).
- [ ] Confirm FLIP stays accessory/menu-bar only (`LSUIElement`): menu bar icon appears, Dock icon does **not** appear, and duplicate launch (`open -a FLIP`) brings Settings / focuses cleanly instead of spawning a second instance (`flock`).
- [ ] Walk through First-Run **Onboarding** (`#onboarding`):
  - Grant **Accessibility** and verify status pill switches to green.
  - Grant **Input Monitoring** and verify status pill switches to green.
  - Trigger Finder **Automation** prompt and verify `Settings -> Engines & Permissions` detects Finder permission.
- [ ] **Limited Mode**: Revoke Accessibility/Input Monitoring in System Settings -> restart FLIP -> confirm no crash, `Option+Shift+Space` Idle shortcut still opens the wheel, and drag-and-drop onto the open Wheel / Batch window works using `~/Desktop` fallback.

## 2. Drag + Trigger (Both Orderings)
- [ ] **Order A — Drag First, Then Trigger**:
  1. In Finder, begin dragging a `.png` file.
  2. While holding the left mouse button, press `DownArrow` (default drag trigger).
  3. Wheel opens centered on the cursor in **Convert** mode with the dragged file pre-loaded and compatible targets shown (`JPG`, `WEBP`, `HEIC`, `PDF`, ...).
  4. Hover onto a sector, verify subtle acoustic tick + Force Touch trackpad tap + specular rim highlight; release mouse button -> file converts without overwriting original.
  5. Repeat with a quick tap/hold to verify the wheel does **not** duplicate-open within the same drag session.
- [ ] **Order B — Trigger First, Then Drag**:
  1. Press and hold `DownArrow` before starting a mouse drag.
  2. Start dragging a `.md` file in Finder -> wheel opens once on drag detection with `.md` targets (`EPUB`, `DOCX`, `PDF`, `HTML`, `TXT`, `RTF`, `ODT`, `PNG`, `JPG`, `WEBP`, `HEIC`).
  3. Drop over `EPUB` -> verify native EPUB 3 file is created cleanly without Pandoc.
- [ ] **Hub Dead-Zone & Cancel Semantics**:
  1. Releasing pointer inside the center `68 px` QuickLook hub does **not** trigger conversion and leaves the wheel open in Idle mode.
  2. Releasing pointer outside the `210 px` outer ring dismisses the wheel cleanly.
  3. Pressing `Esc` during a slow conversion (e.g., multi-page PDF / video) cancels cleanly and deletes partial staged files.

## 3. Idle Hotkey (`Option+Shift+Space`), Mode Switching & Finder Focus
- [ ] With a Finder window active and 1+ files selected, press `Option+Shift+Space`:
  - Wheel opens at cursor in **Convert** mode with selected files loaded.
  - Center hub shows QuickLook thumbnail of the primary selected file.
- [ ] With Desktop or empty Finder window active (no selection), press `Option+Shift+Space`:
  - Wheel opens in **New File** mode showing Folder, Text, Markdown, RTF, DOCX, XLSX, PPTX (and iWork kinds if Pages/Numbers/Keynote are installed) + user templates + Clipboard paste item if clipboard has content.
- [ ] Press `Tab` (or click the **Convert / New File** top mode pill) to switch cleanly between modes; verify keyboard `1`–`8`, Arrow keys, and `Enter`/`Space` activate sectors.
- [ ] Click **Copy Diagnostics** in an error hub (e.g., unsupported/corrupt input) and verify redacted Markdown report copies to Clipboard without leaking `/Users/<name>`.

## 4. Mouse Button Triggers (Buttons 2, 3, 4 / Right, Middle, Side)
- [ ] Open `Settings -> Shortcuts`.
- [ ] Record and save **Mouse Button 2** (`Right`), **Mouse Button 3** (`Middle`), and **Mouse Button 4** (`MouseButton4` / Logitech side button) as drag triggers one by one.
- [ ] For each configured button: start dragging a file in Finder, click/hold the configured mouse button, confirm wheel opens once at cursor, highlight updates smoothly, and release converts cleanly without spurious context menus stealing the drag.
- [ ] Restore `DownArrow` using preset button and verify instant live reload without restarting FLIP.

## 5. Full-Screen Spaces & Multi-Monitor / Mixed-DPI Scaling
- [ ] **Full-Screen Spaces**:
  - Open Finder or Preview in native macOS **Full-Screen Space**.
  - Press `Option+Shift+Space` -> verify Wheel renders **above** the full-screen window in the current Space without forcing macOS to switch back to Desktop Space 1.
- [ ] **Multi-Monitor & Mixed-DPI**:
  - Connect a secondary display with different scaling (`2x` Retina MacBook display + `1x` or scaled external monitor positioned left/above/below primary).
  - Invoke Wheel at the **top-left**, **top-right**, **bottom-left**, and **bottom-right** corners of **each** display.
  - Verify the `440x440` wheel + mode pill stays 100% visible inside the target display's `visibleFrame` (never clipped under menu bar, Dock, notch, or straddling two displays) and pointer hit-testing remains accurate on both displays.

## 6. Finder Inline Rename Reliability
- [ ] Invoke `New File` (via Wheel, Menu Bar `New File in Finder Folder…`, or `New File with FLIP.workflow` Quick Action) inside an active Finder folder with an existing `Untitled.txt`:
  - Verify `Untitled 2.txt` is created.
  - Verify Finder switches/reveals `Untitled 2.txt` and enters **inline filename editing mode** cleanly so typing immediately renames it.
- [ ] Run any **Conversion** (`photo.png -> JPG`):
  - Verify `photo.jpg` (or `photo 2.jpg` on collision) appears next to `photo.png`.
  - Verify inline rename is **NOT** triggered on conversions.

## 7. Interactive Format Ordering & Pagination Dwell
- [ ] Open `Settings -> Formats & Quality` -> **Preferred Format Order**.
- [ ] Select `Documents`, move `EPUB` earlier to position 1 using Move Left (`◀`) or drag-and-drop, open the Wheel on a `.md` file -> confirm `EPUB` appears immediately in Sector 1 on Ring Page 1.
- [ ] Click **Reset to Default** -> verify default family order restores and updates the Wheel immediately on next open.
- [ ] Open Wheel on a `.pdf` (>8 formats) and during drag hover over `More ▸` for **300 ms** -> confirm smooth ring flip to Page 2, and `Back ◂` dwell returns to Page 1.

## 8. Light/Dark Mode, Reduce Motion, Reduce Transparency & VoiceOver
- [ ] **Light & Dark Appearance**: Switch macOS Appearance (`Light` <-> `Dark`) and verify Settings, Batch, and Onboarding adapt natively while the Liquid Glass wheel remains legible over both white and dark wallpapers.
- [ ] **Reduce Motion** (`System Settings -> Accessibility -> Display -> Reduce motion`): Verify wheel spring pop-in, pulse, and sector transforms drop smoothly to minimal instantaneous transitions without jitter.
- [ ] **Reduce Transparency** (`System Settings -> Accessibility -> Display -> Reduce transparency`): Verify `.wheel-glass-ring`, `.wheel-hub`, and `.wheel-mode-pill` fall back cleanly to high-contrast opaque dark slate (`rgba(22, 24, 30, 0.96)`).
- [ ] **VoiceOver** (`Cmd+F5`):
  - Open Wheel -> hear open-mode announcement (`"Convert wheel opened for 1 file, ... options."`).
  - Step across sectors (`VO+Arrow` or Tab/Arrows) -> hear sector labels (`"Convert to PNG, sector 2 of 8"`).
  - Activate `More ▸` (`VO+Space` or `Enter`) -> hear `"Page 2 of 2"`.
  - Activate a target format -> hear `"Converting to ..."` and progress milestones (`25%`, `50%`, `75%`) plus final `"Converted 1 file to ..."` (or error details on failure).
  - Navigate `Settings -> Formats & Quality` -> Preferred Format Order -> confirm accessible labels announce `"Move ... earlier..."`, `"Move ... later..."`, and `"Reset format order for this family to default"` while preserving button focus across moves.

## 9. Quick Actions, `flip://` URL Scheme, Batch & Updates
- [ ] In `Settings -> Engines & Permissions`, click **Install Quick Actions** -> right-click a file in Finder -> **Quick Actions -> Convert with FLIP** opens Wheel/Batch directly with the file; **New File with FLIP** opens New File mode in that folder.
- [ ] Run `open "flip://wheel?mode=new"` and `open "flip://settings?tab=engines"` from Terminal -> confirm matching windows open cleanly.
- [ ] Shift/Option-click a sector (or tray `Batch / Multiple Formats…`) -> enqueue multiple files & multiple output formats (`PNG` + `PDF`), run batch conversion, cancel mid-queue, and inspect per-item progress/completion rows.
- [ ] Verify CLI install/symlink (`which flip`) works (`flip --help`, `flip convert note.md --to EPUB`) and `Check for Updates` reports current release comparison cleanly.

## 10. Single-Instance Socket IPC & CLI Control Commands (Step E)
- [ ] With FLIP already running in the menu bar, inspect socket permissions: `ls -la ~/Library/Application\ Support/FLIP/flip-instance.sock` confirms owner-only `srwx------` (`0600`).
- [ ] Run `flip wheel --mode new`, `flip wheel --mode convert <file>`, `flip settings`, and `flip batch <files...>` -> confirm each command drives the **existing** menu bar instance cleanly without spawning a duplicate process or Dock icon.
- [ ] Quit FLIP cleanly with `flip quit` (or terminate the host), verify stale `flip-instance.sock` cleanup, and run `flip wheel` when FLIP is closed -> confirm CLI launches `FLIP.app` and opens the wheel automatically.

## 11. Archives, Vision `REMOVE_BG`, `OPTIMIZE_PDF`, Native `CAF`/`FLAC`, Localization & Feedback Test (Steps F–G)
- [ ] **Archives (`ZIP` / `UNZIP`)**: Drop multiple files in Finder -> invoke `ZIP` -> confirm single deduplicated `<stem>.zip` archive; trigger `UNZIP` on a `.zip` -> confirm extraction into collision-safe subfolder (`<stem>/...`) and clean rejection of malformed/traversal archives.
- [ ] **Apple Vision Background Removal (`REMOVE_BG`)**: Trigger `REMOVE_BG` on a subject photo (e.g., portrait/object `.png`/`.jpg`) -> confirm transparent PNG cutout (`<stem>.png` or Finder counter if source is PNG); trigger `REMOVE_BG` on a blank solid-color image -> confirm clear error with redacted diagnostics option.
- [ ] **PDF Optimization (`OPTIMIZE_PDF`)**: Trigger `OPTIMIZE_PDF` on a raster-heavy `.pdf` -> verify noticeable size reduction while searchable text remains selectable in Preview; verify encrypted/locked PDFs return a clean error rather than corrupting output.
- [ ] **Native Zero-FFmpeg Audio (`CAF` & `FLAC`)**: Temporarily blank/hide FFmpeg path in `Settings -> Engines & Permissions` -> verify `CAF`, `FLAC`, `WAV`, `M4A`, `AIFF`, `ALAC` stay active in the wheel for `WAV`/`FLAC`/`CAF` files, while `MP3`/`OGG`/`OPUS` stay FFmpeg-gated (`FFmpeg` required).
- [ ] **EN / IT UI Localization (`Settings -> General`)**: Switch **Language** between **English** and **Italiano** -> verify Settings, Radial Wheel (pills/subtitles/VoiceOver strings), Batch Converter, and Onboarding update dynamically in both languages and persist across restart.
- [ ] **Sound & Haptics Preview Button**: Open `Settings -> General`:
  - With both `Wheel tick sound` and `Trackpad haptics` ON -> click **Test Wheel Sound & Haptics** (`Prova suono e feedback aptico ruota`) -> confirm audible tick and Force Touch haptic feedback plus `"Sound & haptics played"` (`"Suono e aptica riprodotti"`).
  - Toggle `Wheel tick sound` OFF and `Trackpad haptics` ON -> click preview -> confirm haptic-only pulse (no audio).
  - Toggle `Wheel tick sound` ON and `Trackpad haptics` OFF -> click preview -> confirm acoustic tick only (no Force Touch pulse).
  - Toggle both OFF -> click preview -> confirm muted feedback message (`"Sound & haptics are muted"`).

## 12. Native `FB2`/`RST`, `SVG` -> `ICO`, `ANIMATE_GIF`, `flip preview` & Local Installer (Steps I–L)
- [ ] **Zero-Pandoc `FB2` & `RST`**: With `FLIP_DISABLE_PANDOC=1` (or without Pandoc installed), drop `.md` or `.pdf` on the wheel and convert to `FB2` and `RST`; drop `.fb2` or `.rst` and convert back to `EPUB` / `HTML` / `PDF`.
- [ ] **`SVG` Input, `ICO` Output & `ANIMATE_GIF` (`GIF+`)**: Drop `.svg` -> convert to `ICO` and `PNG` (confirming offline sanitization strips external/file URLs cleanly); select 2+ `.png`/`.jpg` files -> trigger `GIF+` (`ANIMATE_GIF`) in Wheel/Batch and verify animated `.gif` playback in Preview/QuickLook.
- [ ] **CLI SVG Wheel Preview**: Run `flip preview --out /tmp/flip-wheel.svg --files photo.png --hover 0` and open in Safari/QuickLook to inspect the Liquid Glass ring layout.
- [ ] **Local Installer (`scripts/install-local.sh`)**: Run `./scripts/install-local.sh --dry-run`, then `./scripts/install-local.sh`, verify `FLIP.app` launches without Gatekeeper quarantine issues, `which flip` resolves `~/.local/bin/flip`, and Quick Actions appear in Finder.
