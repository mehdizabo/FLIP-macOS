# Miglioramenti possibili — Panoramica e priorità

> Stato: **proposta, nessuna modifica applicata al codice** · Revisione del 2026-11-19 sul commit `298874e` (`main`)
> Documenti collegati: [01 Affidabilità input](01-RELIABILITY-INPUT.md) · [02 Performance e architettura](02-PERFORMANCE-ARCHITECTURE.md) · [03 Sicurezza, privacy, rilascio](03-SECURITY-PRIVACY-RELEASE.md) · [04 Test, CI, documentazione, DX](04-TESTING-CI-DOCS-DX.md)

## 1. Come è stata fatta l'analisi (e i suoi limiti)

Ispezione **statica** del repository (sorgenti Rust, TypeScript, Swift, CSS, script, CI, documenti), con alcune misure rapide
(conteggi di righe, `unwrap`/`unsafe`, dimensioni dei bundle, dipendenze duplicate) e con l'esperienza dei bug corretti nelle
ultime sessioni (drag intermittente, `Esc` doppio, tasto `R` senza focus, HTML senza testo).

Cosa **non** è stato fatto: nessuna prova su desktop reale (Finder, multi-monitor, VoiceOver), nessun profiling con Instruments,
nessuna build notarizzata. Ogni punto è marcato con un livello di fiducia:

- **[Verificato]** letto nel codice o misurato durante la revisione;
- **[Inferenza]** conseguenza logica del codice, da confermare dal vivo;
- **[Da misurare]** ipotesi di beneficio che richiede una misura prima/dopo.

## 2. Fotografia del progetto

| Area | Dato | Note |
|---|---|---|
| Stack | Tauri v2 + Rust (`src-tauri`) + frontend TypeScript/Vite (`src`) + bridge Swift (`native-bridge`) | build con soli Command Line Tools (`swiftc`), niente Xcode |
| Dimensione | ~57k righe tracciate (inclusi ~4,6k di schemi generati e lockfile) | |
| File più grandi | `engines/document.rs` 6.391 · `views.e2e.test.ts` 2.745 · `views/WheelView.ts` 2.363 · `cli.rs` 2.068 · `MediaAndDocs.swift` 1.881 · `views/SettingsView.ts` 1.676 | |
| Test frontend | `bun test`: 59 test, ~2.880 asserzioni | i documenti riportano ancora 42 |
| Test Rust | lib 75 + `host_ipc_integration` 9 + `engines_integration` + `stress_and_perf` | `clippy -D warnings` pulito |
| Test Swift | **0** | il bridge è testato solo indirettamente da Rust/CLI |
| Bundle frontend | JS 216 KB (gzip 56 KB) · CSS 34 KB | un solo bundle per tutte e 4 le finestre |
| Binario release | `flip` ≈ 21 MB | nessun `[profile.release]` in `Cargo.toml` |
| Vulnerabilità note | `npm audit`: 0 | `cargo audit`: vedi doc 03 |

**Cosa va già bene** (da non toccare): conversione atomica con staging `.flip-job-*`, validazioni anti zip-bomb/symlink,
socket IPC `0600` con recupero stale, redazione dei path nei diagnostics, test di contratto Rust↔TS su comandi ed eventi,
`clippy -D warnings` in CI, suite di stress sulle 50 aperture della wheel.

## 3. Backlog prioritario (top 16)

Impatto: **A**lto / **M**edio / **B**asso · Sforzo: **S** (ore) / **M** (1–3 giorni) / **L** (settimana+).

| # | ID | Miglioramento | Impatto | Sforzo | Doc |
|---|---|---|---|---|---|
| 1 | REL-01 | Estrarre la state machine del drag da `MonitorController` in un tipo puro e testabile | A | M | [01](01-RELIABILITY-INPUT.md) |
| 2 | REL-04 | Disaccoppiare gli eventi puntatore dallo step della fisica (oggi la molla può girare più veloce del tempo reale) | A | S | [01](01-RELIABILITY-INPUT.md) |
| 3 | REL-02 | Una sola fonte di verità per lo stato "wheel aperta" (epoch + ack) fra Swift, Rust e frontend | A | M | [01](01-RELIABILITY-INPUT.md) |
| 4 | REL-07 | Aprire la wheel subito e popolare il contesto Finder in modo asincrono (oggi attende un processo esterno) | A | M | [01](01-RELIABILITY-INPUT.md) |
| 5 | SEC-01 | Impostare una CSP (oggi `csp: null`) e rendere obbligatorio l'escape nelle ~30 `innerHTML` | A | S | [03](03-SECURITY-PRIVACY-RELEASE.md) |
| 6 | SEC-07 | `events.jsonl` salva path completi dei file utente in chiaro: redigere/hashare alla scrittura | A | S | [03](03-SECURITY-PRIVACY-RELEASE.md) |
| 7 | QA-01 | Smoke test su `WKWebView` reale (CSS/SVG supportati davvero), perché lo shim DOM non vede i bug WebKit | A | M | [04](04-TESTING-CI-DOCS-DX.md) |
| 8 | QA-03 | CI: separare `verify` (ogni PR) da `package` (main/tag), aggiungere audit/fmt, usare `@tauri-apps/cli` già presente | M | S | [04](04-TESTING-CI-DOCS-DX.md) |
| 9 | PERF-03 | Bridge in modalità persistente (RPC) per `finder-context`, `clipboard-inspect`, `thumbnail` invece di un processo per richiesta | A | M | [02](02-PERFORMANCE-ARCHITECTURE.md) |
| 10 | REL-05 | Sostituire i flag sparsi di `WheelView.ts` con un `InputArbiter`/reducer esplicito | A | L | [01](01-RELIABILITY-INPUT.md) |
| 11 | SEC-02 | Limitare i 28 comandi per finestra (app manifest di `tauri-build`) e validare i path di `get_thumbnail`/`reveal_in_finder` | M | M | [03](03-SECURITY-PRIVACY-RELEASE.md) |
| 12 | PERF-05 | Spezzare `document.rs` e unificare i tokenizer HTML scritti a mano (≥6 duplicati) | M | L | [02](02-PERFORMANCE-ARCHITECTURE.md) |
| 13 | PERF-01 | Profilo `release` (`lto`, `codegen-units`, `strip`) + `tokio` ridotto | M | S | [02](02-PERFORMANCE-ARCHITECTURE.md) |
| 14 | QA-07 | Marcare come storici i documenti `docs/0x` (descrivono SwiftUI nativo, non Tauri) e aggiornare spec/CHANGELOG | M | S | [04](04-TESTING-CI-DOCS-DX.md) |
| 15 | SEC-06 | Notarizzazione + entitlements versionati + workflow di release | A | L | [03](03-SECURITY-PRIVACY-RELEASE.md) |
| 16 | QA-04 | Un solo package manager (oggi `package-lock.json` tracciato ma si lavora con `pnpm`) | B | S | [04](04-TESTING-CI-DOCS-DX.md) |

## 4. Percorso suggerito

1. **Sprint 1 — "Stabilità e igiene" (≈1 settimana):** REL-04, SEC-01, SEC-07, QA-03, QA-04, PERF-01, QA-07, SEC-08 (path di build nel binario).
2. **Sprint 2 — "Testabilità" (≈1–2 settimane):** REL-01 + test Swift, REL-02, QA-01 (harness `WKWebView`), PERF-03.
3. **Sprint 3 — "Struttura e rilascio" (≈2+ settimane):** REL-05, PERF-05, SEC-02, SEC-06 (notarizzazione e release workflow).

## 5. Punti da verificare dal vivo prima di agire

- Se in modalità drag funzionano `Tab`, `1`–`8`, frecce, `Enter` (la finestra non è key: **[Inferenza]**, vedi REL-06).
- Latenza reale di apertura della wheel in modalità idle (processo `finder-context` + AppleScript): **[Da misurare]**.
- Costo GPU di ~11 layer `backdrop-filter` contemporanei su display ProMotion/5K: **[Da misurare]** (PERF-08).
- ~~Riduzione dimensione binario con `lto`/`strip`~~: **misurata** (vedi §6).

## 6. Avanzamento Sprint 1 (ramo `sprint-1`, 2026-11-20)

Ogni voce ha un commit e una prova; "prova rossa" = il test nuovo fallisce senza la modifica.

| ID | Stato | Commit | Prova |
|---|---|---|---|
| SEC-08 | Fatto | `fc80b8a` | Build release con `--features tauri/custom-protocol`: 0 occorrenze del path di build nel binario (il vecchio binario ne aveva 2). `*_from` ora prendono `Option<&Path>`; test release-like. Effetto collaterale: un binario release **fuori dal bundle** non trova più il bridge dal repo, serve `FLIP_BRIDGE_PATH`. |
| SEC-07 | Fatto (solo redazione) | `b1bbb4e` | Prova rossa: senza la modifica il log contiene `/Users/alice/...` e la home. Restano registrati i nomi di file e cartelle sotto la home; hash dei nomi, `FLIP_LOG_VERBOSE` e retention a 14 giorni **non** fatti. |
| QA-07 | Fatto in parte | `c27b14c` | Banner "storico" su `docs/00–06` e `PLAN-*`, `[Unreleased]` nel CHANGELOG (dai commit reali), sezione 14 in `ACCEPTANCE.md`. **Non** fatto: riscrittura di `03-UX-WHEEL-SPEC.md`, scelta della lingua dei documenti. |
| PERF-01 | Fatto | `a6f87fd` | `cargo build --release --features tauri/custom-protocol`, stesso commit: **21.022.992 → 10.774.976 byte (−48,8 %)**; build pulita ~80 s → ~176 s (`codegen-units = 1`). Smoke CLI md → EPUB → TXT sul binario ridotto: OK. `tokio` ridotto a `rt-multi-thread` + `time`. Avviamento a freddo **non** misurato. |
| REL-04 | Fatto (variante prudente) | `31ec68b` | La molla non supera più il tempo reale: eventi a < 2 ms dallo step precedente vengono accumulati, non arrotondati a 6 ms (con mouse a 1 kHz il vecchio codice integrava **6000 ms per 100 ms reali, 60×**). Resta lo step per evento (non spostato in rAF) per non riaprire il fix WKWebView di `607b36f`. Lo stub rAF dei test usava `Date.now()` invece di `performance.now()`: corretto. **Controllo dal vivo necessario** con un mouse ad alta frequenza (il feel è stato tarato a mano). |
| SEC-01 | Solo metà: escaping | `ccab327` | Test d'iniezione su Batch (lista e risultati) ed Error Hub della Wheel; prova rossa togliendo `escapeHtml`. **La CSP non è attiva** (vedi 03). Tagged template e regola di lint non fatti. |
| QA-03 | Minimo | `5734e9e` | `concurrency` con `cancel-in-progress` e `audit-repository.sh` in CI (passa in locale; `actionlint` pulito salvo un SC2012 già presente). Non fatti: split `verify`/`package`, `cargo audit`, pin degli action, rimozione dello step `tauri-cli` (serve ancora: `build-macos.sh` usa `cargo tauri build`). Non eseguibile senza un push. |
| QA-04 | Rimandato | — | È una scelta di Capo (npm vs pnpm): il lavoro locale è in pnpm, la CI e il lockfile tracciato in npm. `pnpm-workspace.yaml` contiene ancora il segnaposto `allowBuilds: esbuild: set this to true or false` e non va committato così. |
| REL-01, REL-02, REL-05, SEC-02, PERF-03, PERF-05 | Non iniziati | — | Sprint 2–3. |
