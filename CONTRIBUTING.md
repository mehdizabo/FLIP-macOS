# Contributing

Build and test on Apple Silicon macOS using [README.md](README.md). Keep changes focused, preserve existing safety invariants, and include verification relevant to the behavior you modify.

## Ground Rules

- **Never commit secrets or private artifacts**: Do not commit test fixtures with personal documents, raw logs, local paths, credentials, `.app`/`.dmg` binaries, or packaged third-party executables. Run `./scripts/audit-repository.sh` before preparing a PR or source archive.
- **Preserve zero-overwrite safety**: Original files must never be modified or overwritten. Keep staged `.flip-job-<uuid>` execution, `renamex_np(RENAME_EXCL)` commits with Finder-style collision suffixes, symlink/alias/iCloud/network-volume refusal, container decompression limits, and offline SVG/HTML sanitization intact.
- **Keep inline rename scoped**: Trigger Finder inline rename (`Return`) only for `Create New File`, never on conversions.
- **Separate automated checks from live desktop QA**: Automated `bun test`, `cargo test`, DOM E2E shims, and CLI smoke tests are **not** proof of physical Finder drag-and-drop, mixed-DPI WindowServer placement, or VoiceOver audio behavior. Record automated results in `TEST_RESULTS.md` and physical GUI results in `ACCEPTANCE.md`.

## Verification Commands

```bash
./native-bridge/build.sh
npx tsc --noEmit && npm run build && bun test
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
./scripts/audit-repository.sh
```

Before changing codecs, document/spreadsheet engines, or input gating, test file signatures, readback roundtrips, filename collisions, cancellation cleanup, and both trigger-first / drag-first orderings.
