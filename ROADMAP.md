# Roadmap

## 1. Post-v0.1.0 Physical WindowServer Acceptance (Pre-Public Binary Release)

Complete the physical GUI walkthrough in [ACCEPTANCE.md](ACCEPTANCE.md) on real macOS desktop hardware:

- Physical Finder/Desktop drag with configurable keyboard keys and Mouse Buttons `2`/`3`/`4` in both drag-first and trigger-first orderings, plus `Esc` cancellation and center hub dead-zone behavior.
- Mixed-DPI multi-monitor edge clamping and native Full-Screen Spaces overlay positioning (`visibleOnAllWorkspaces`).
- Live Finder inline rename (`Return`) reliability on `Create New File`, plus acoustic `flip-tick.wav` playback and Force Touch trackpad haptics across output devices.
- Light/Dark appearance, `Reduce Motion`, `Reduce Transparency`, and live **VoiceOver** (`Cmd+F5`) walkthrough.
- Live `LaunchAgents` persistence across an actual macOS user sign-in / reboot, plus verification on **macOS 15 Sequoia** and **macOS 26 Tahoe** hardware.
- Remote execution of `.github/workflows/ci.yml` on GitHub Actions `macos-15` runners.

## 2. Signed Distribution & Notarization

- Configure an Apple **Developer ID Application** certificate (`APPLE_SIGNING_IDENTITY`) and **Apple Notarization** profile (`APPLE_NOTARY_PROFILE`) so `scripts/sign-local.sh` and `scripts/package-dmg.sh` produce Gatekeeper-notarized `.app` and `.dmg` releases without ad-hoc Control-click bypasses.
- Publish versioned GitHub Releases and verify `brew install --cask` against the hosted `.dmg` URL in `packaging/homebrew/flip.rb`.

## 3. Future Enhancements (v0.2.0+)

- **Finder Sync Extension**: Evaluate an optional Finder Sync Extension (`FIFinderSync`) to augment context menus and badge status alongside existing Quick Actions (`~/Library/Services/*.workflow`).
- **Spotlight & Shortcuts App Intents**: Expose native macOS `AppIntents` (`Convert with FLIP`, `New File with FLIP`) for Apple Shortcuts and Spotlight actions.
- **Universal Binary (`arm64` + `x86_64`)**: Evaluate Intel `x86_64` cross-compilation alongside Apple Silicon if demand warrants.
- **iWork & Optional External Engine Matrix**: Expand live QA coverage with installed Apple Pages / Numbers / Keynote and optional Pandoc integration.
