# Cross-platform collaboration brief

Status: working document for the `v0.6.1-macos` port.

This document separates the behavior that should remain consistent across
Windows, macOS, and Linux from the implementation details that must stay
platform-specific. It is intended to support a future collaboration request;
it is not an upstream pull request and does not claim that the upstream
project has accepted the port.

## Current repository state

- The macOS implementation is the Tauri/Rust/Swift/TypeScript tree published
  at tag `v0.6.1-macos`.
- The repository `main` branch currently contains an older Windows-oriented
  WPF tree. It must not be presented as the current macOS source until the
  branch layout is intentionally repaired.
- The upstream Windows project is currently `v0.6.1`. Its public history was
  MIT before the `v0.6.0` release and GPL-3.0-only from that release onward.
- The macOS release declares MIT. New GPL-only upstream code must not be
  copied into the MIT port without a deliberate licensing decision and
  provenance record.

The two repositories have no common Git ancestor on their current default
branches. A cross-platform collaboration should therefore share behavioral
contracts and selected changes, not merge the repositories wholesale.

## Compatibility audit

The statuses below describe the current macOS source, not a promise that every
path has been physically exercised on a Mac.

| Upstream Windows capability | macOS status | Evidence or next action |
| --- | --- | --- |
| Radial wheel, pagination, hit testing and progress | Equivalent | `src/views/WheelView.ts`, `src/wheelGeometry.ts`, and geometry tests |
| Drag-first and trigger-first input ordering | Equivalent | `native-bridge/Sources/Monitor.swift`, `src-tauri/src/core/input_gate.rs` |
| Single instance, tray/menu bar host and IPC | Equivalent | Unix `flock` plus owner-only Unix socket |
| No-overwrite output naming and atomic staging | Equivalent | `src-tauri/src/core/naming.rs`, `staging.rs` |
| Image conversion and quick image tools | Equivalent with native engines | ImageIO, Rust image engine and Vision replace Windows-specific APIs |
| PDF rendering, OCR and PDF/A | Equivalent with native engines | PDFKit, Vision and optional Ghostscript |
| Structured data and XLSX | Equivalent with native engines | `src-tauri/src/engines/data.rs` |
| Archives and container safety | Equivalent with native implementation | ZIP/UNZIP engine and decompression limits |
| Documents, Pandoc and Office workflows | Equivalent with deliberate differences | Native EPUB/FB2/RST engines; Pandoc and iWork remain optional |
| Audio/video conversion | Equivalent with deliberate differences | AVFoundation/CoreAudio first; FFmpeg is optional |
| Settings, update checks, diagnostics and localization | Equivalent | Tauri settings, updater, redacted logs and EN/IT dictionaries |
| First-run onboarding and settings hub | Equivalent | `OnboardingView.ts` and `SettingsView.ts` |
| 3D and vector conversion family | Gap | No macOS counterpart is currently documented for the upstream 3D/vector matrix |
| Watch folders | Gap | No macOS watch-folder service is currently documented or mapped |
| Browser drop service | Gap or platform-specific omission | Decide whether Finder/clipboard workflows are the intended macOS equivalent |
| Clipboard conversion service | Partial | Clipboard-based new-file creation exists; full Windows conversion parity needs an explicit contract |
| Tobii/gaze orb workflow | Gap or platform-specific omission | Requires a macOS accessibility/device design before implementation |
| Windows shell integration and registry startup | Not applicable | Replaced by Finder Quick Actions, LaunchAgents, Raycast and menu-bar integration |
| Windows WPF/WinForms/GDI+/User32 implementation | Not applicable | Must remain outside the shared cross-platform contract |

The existing `src-tauri/tests/original_flip_parity.rs` checks the older 0.5.1
format matrix. It is useful regression coverage, but it is not sufficient to
claim parity with upstream 0.6.1. The next parity milestone is to add focused
tests for each row marked `Equivalent` and to record explicit decisions for
each `Gap` or `Partial` row.

## Shared contracts for Windows, macOS and Linux

The implementations can be different while preserving these observable rules:

1. Never overwrite or delete the source file.
2. Stage outputs privately, validate them, and commit them atomically.
3. Use deterministic collision-safe names.
4. Reject unsafe links, aliases, incomplete cloud files, archive traversal and
   decompression bombs.
5. Make cancellation clean up temporary work.
6. Expose the same format-policy concepts, including mixed-selection
   intersection and optional-engine availability.
7. Keep diagnostics useful while redacting credentials, private paths and
   file contents.
8. Keep conversion behavior separate from platform shell integration.

Windows, macOS and Linux should each have an adapter layer for input events,
file-manager integration, startup, notifications, accessibility and native
codecs. The shared layer should contain policy, naming, staging, engine
contracts and test fixtures where the license permits reuse.

## Recommended implementation order

### 1. Make the macOS repository unambiguous

Do not rewrite the existing release tag. Decide whether the macOS source should
become the default branch through a reviewed branch migration, or whether the
repository should be split into a clean macOS repository. Until that decision,
describe `v0.6.1-macos` as the source of the macOS release.

### 2. Close or document the parity gaps

Prioritize the gaps that affect the product promise:

1. 3D/vector format policy and native-engine decision.
2. Watch-folder behavior and permissions model.
3. Clipboard conversion versus clipboard-based new-file creation.
4. A documented decision on gaze/accessibility integration.

Do not implement all four as one change. Each needs its own behavior contract,
tests, and platform-specific design.

### 3. Prepare Linux around the same contracts

The Linux port should implement the shared policy and staging contracts first,
then add a desktop adapter for the chosen file manager and desktop protocol.
Avoid copying macOS AppKit or Windows shell assumptions into the shared core.

The Linux repository was not identified among the public repositories currently
visible for this account, so this document intentionally does not invent a
repository URL or claim Linux feature parity.

## Draft collaboration request

> Hello, I maintain an independent macOS port of FLIP at
> `https://github.com/mehdizabo/FLIP-macOS`.
>
> The port uses a separate Tauri/Rust/Swift/TypeScript implementation because
> the Windows WPF/WinForms system APIs cannot be reused directly on macOS. It
> preserves the main user-facing contracts: radial conversion workflow,
> no-overwrite output naming, atomic staging, local processing, optional
> external engines, cancellation, and diagnostic safeguards.
>
> I have documented the platform mapping, verified parity areas, and listed
> the remaining gaps in `docs/CROSS-PLATFORM-COLLABORATION.md`. I am also
> developing a Linux implementation around the same shared contracts.
>
> Would you be open to discussing a documented community-port relationship,
> shared behavior tests, or a small documentation change linking the ports?
> I am not proposing to merge the macOS and Windows implementations wholesale.
> I would first follow the licensing and contribution rules you prefer.

This draft should be sent only after the repository state, license provenance,
and parity table have been reviewed. It is intentionally a collaboration
question rather than a code pull request.

## Evidence required before requesting integration

- A clean source branch whose README, tag, build system and license agree.
- A current parity test matrix, not only the old 0.5.1 test.
- Focused tests for each shared safety contract.
- A list of intentionally unsupported Windows features on macOS.
- A Linux status document using the same contract vocabulary.
- A provenance record for code copied from the pre-GPL upstream snapshot.
- Separate automated-test results and physical desktop acceptance results.
