# 04 · Test, CI, documentazione ed esperienza di sviluppo

> Parte del piano in [00-OVERVIEW.md](00-OVERVIEW.md). Stato: **proposta**.
> Livelli di fiducia: **[Verificato]** · **[Inferenza]** · **[Da misurare]**.

---

## QA-01 · Test su `WKWebView` reale (Impatto A · Sforzo M)

**Problema [Verificato].** I test E2E (`src/views.e2e.test.ts`, ~2.750 righe) girano su uno **shim DOM/SVG fatto a mano**
(`ShimElement`, selettori e `style` simulati). È ottimo per la logica, ma non può rilevare problemi di rendering WebKit:
i difetti trovati di recente erano proprio di quel tipo (`translate3d` su `<g>` SVG con `foreignObject`, `color-mix()` con
`calc(var(...))`, ripitturazione dei pucks) e sono stati scoperti solo dall'uso reale.

**Proposta.** Piccolo eseguibile di test in Swift (compilabile con `swiftc`, senza Xcode) che:
1. crea un `WKWebView` fuori schermo, carica `dist/index.html#wheel` con un payload `wheel:state` finto
   (iniettato via `evaluateJavaScript`);
2. imposta `--bubble-*`/puntatore, aspetta due frame e legge `getComputedStyle` (es. `--prox-rim-pct`, `transform` di
   `.wheel-bubble-body`, colore effettivo di `.wheel-sector-icon`) per verificare che le proprietà siano **accettate**
   da WebKit (`CSS.supports(...)` per `color-mix`, `backdrop-filter`);
3. (opzionale) `takeSnapshot` e confronto con un'immagine di riferimento con tolleranza, solo su CI `macos-15` fissa.
Esecuzione: `native-bridge/build.sh --webkit-test` e job dedicato in CI. Nota: `tauri-driver`/WebDriver non è utilizzabile
su macOS per `WKWebView`, quindi questa è l'alternativa pratica.

## QA-02 · Test Swift (Impatto A · Sforzo M)

**[Verificato].** `native-bridge/` contiene solo `Sources/` e `build.sh`: nessun test. Vedi REL-01 per l'estrazione di
`DragSession`; stesso approccio per il parsing delle scorciatoie (`ParsedShortcut.parse`, `keyCodeForName` in `Common.swift`:
tabella di test per ogni alias) e per `readDragFileURLs` (con un `NSPasteboard(name:)` privato e URL temporanei).

## QA-03 · Pipeline CI (Impatto M · Sforzo S)

**Stato [Verificato] (`.github/workflows/ci.yml`).** Un solo job su `macos-15` (timeout 35 min) che, a ogni push/PR, esegue
build bridge, `tsc`, `vite build`, `bun test`, `clippy`, `cargo test`, **build `.app` + `.dmg`** e smoke CLI.

**Lacune e proposte.**
- **Separare** `verify` (ogni PR: lint/test, ~10 min) da `package` (solo `main` e tag: `.dmg`, smoke, artifact).
- Aggiungere `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`.
- **`./scripts/audit-repository.sh` non è in CI**, anche se `CONTRIBUTING.md` lo include nei comandi di verifica: aggiungerlo.
- Mancano `cargo fmt --check` (vedi QA-05), `cargo audit`/`cargo deny`, `npm audit --omit=dev`.
- La CI esegue `cargo install tauri-cli --locked` (lento) se assente, mentre `@tauri-apps/cli` è **già** in `devDependencies`:
  usare `npx tauri build`/`npm run tauri -- build` ed eliminare lo step.
- Cache: `Swatinem/rust-cache` è presente; aggiungere cache per `native-bridge/.build` se si introduce uno step di test Swift.
- Pin degli action per SHA e Dependabot per `github-actions`.
- Pubblicare come artifact i risultati dei test (JUnit/`--reporter`) e un riepilogo nel job summary per aggiornare
  `TEST_RESULTS.md` automaticamente (QA-06).

## QA-04 · Due package manager (Impatto B · Sforzo S)

**[Verificato].** `package-lock.json` è tracciato e la CI usa `npm ci`; in locale si lavora con `pnpm` (processi `pnpm tauri dev` in
esecuzione) e nel working tree compaiono `pnpm-lock.yaml` e `pnpm-workspace.yaml` **non tracciati**: rischio di dipendenze
divergenti fra locale e CI e di commit accidentali dei file pnpm.
**Proposta.** Scegliere uno. Se `npm`: aggiungere `pnpm-lock.yaml`, `pnpm-workspace.yaml` a `.gitignore` e il campo
`"packageManager": "npm@…"`. Se `pnpm`: tracciare `pnpm-lock.yaml`, usare `pnpm/action-setup` in CI e rimuovere `package-lock.json`.
In ogni caso documentarlo in `CONTRIBUTING.md`.

## QA-05 · Formattazione e lint (Impatto M · Sforzo S)

**[Verificato].**
- Rust: `rustfmt` è disponibile ma `cargo fmt --check` segnala **466 differenze** (le più numerose in
  `tests/engines_integration.rs` 154, `engines/document.rs` 92, `cli.rs` 23, `core/policy.rs` 21): il codice non è formattato
  con rustfmt. **Proposta:** un singolo commit "style: cargo fmt" (nessun cambiamento di comportamento) e poi
  `cargo fmt --check` in CI.
- TypeScript: nessuna configurazione ESLint/Biome/Prettier e nessuno script `lint` in `package.json`; `tsconfig` è già `strict`
  con `noUnusedLocals`. **Proposta:** Biome (un solo binario, formatter + lint) con regola contro `innerHTML` non taggato
  (vedi SEC-01) e `npm run lint` in CI.
- Swift: nessun formatter; opzionale `swift-format` se disponibile nei Command Line Tools.
- Copertura: non misurata. `bun test --coverage` e `cargo llvm-cov` (non bloccanti, solo report) per individuare le aree non coperte
  (`cli.rs`, `quick_actions.rs`, viste Settings/Batch).

## QA-06 · Numeri dei test nei documenti (Impatto B · Sforzo S)

**[Verificato].** `TEST_RESULTS.md` e `PUBLIC_RELEASE_REVIEW.md` riportano **42** test `bun` (1.914 asserzioni) e **99** test
`cargo`; oggi `bun test` esegue **59** test (~2.880 asserzioni) e la suite Rust è cambiata. I numeri scritti a mano invecchiano.
**Proposta.** Generare `TEST_RESULTS.md` con `scripts/record-test-results.sh` (esegue i comandi e scrive data, commit, conteggi),
oppure togliere i conteggi dai documenti e rimandare all'ultimo run di CI.

## QA-07 · Documentazione obsoleta o contraddittoria (Impatto M · Sforzo S)

**[Verificato].**
- `docs/00-INDEX.md` (riga ~46) e `docs/02-ARCHITECTURE.md` (riga 5) descrivono **Swift 6 + SwiftUI nativo** con `NSPanel`,
  mentre l'architettura reale è **Tauri v2 + Rust + bridge Swift** (`docs/DECISIONS.md §2`, `docs/PORTING-NOTES.md`).
- `docs/03-UX-WHEEL-SPEC.md` descrive `.glassEffect(...)`/`GlassEffectContainer` e settori ad anello; l'implementazione attuale
  è **Liquid Glass in WebKit** con **bolle circolari** in stile Logi Options+, fisica a molle magnetiche e hub da 104 px.
- `CHANGELOG.md` si ferma a `0.1.0` e descrive ancora "chamfered SVG sectors" e "68 px QuickLook center hub": manca una
  sezione `[Unreleased]` per redesign Liquid Glass, fisica magnetica, `R` globale in drag, fallback HTML→MD/TXT, fix del drag.
- `ACCEPTANCE.md` non include i nuovi controlli (reazione magnetica, `R`/`Esc` in drag, riapertura ripetuta).
- Lingue miste: `docs/0x-*` in italiano, documenti in radice in inglese (con `README.it.md`).

**Proposta.**
1. Banner in testa a `docs/00…06` e `PLAN-*`: *"Documento storico di pianificazione — l'architettura attuale è in
   `ARCHITECTURE.md` e `docs/DECISIONS.md`"*.
2. Aggiornare `docs/03-UX-WHEEL-SPEC.md` (o crearne `03b`) con il design attuale e i parametri di `wheelPhysics.ts`
   (`MAGNETIC_RADIUS`, `SPRING_STIFFNESS`, …) come fonte unica.
3. Aggiungere `[Unreleased]` al `CHANGELOG.md` e aggiornare `ACCEPTANCE.md`.
4. Scegliere la lingua per tipo di documento (suggerito: radice e `docs/` tecnici in inglese; traduzioni `*.it.md` opzionali).

## QA-08 · Esperienza di sviluppo (Impatto M · Sforzo S)

- **Un comando unico di verifica.** I passi sono ripetuti in `CONTRIBUTING.md`, `README.md`, CI e nelle istruzioni agli agenti.
  Creare `scripts/verify.sh` (bridge → tsc/build → bun test → clippy → cargo test → audit) con `--fast` (senza stress) e usarlo
  in CI e nei documenti.
- **`.cargo/config.toml`** forza `CC=/usr/bin/clang`, `CXX`, `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk`
  **[Verificato]**: funziona solo con i soli Command Line Tools; su macchine con Xcode completo o SDK in altro percorso il build
  dipende da override manuali (in sessione si è usato `SDKROOT=$(xcrun --show-sdk-path)`). **Proposta:** rimuovere i valori
  hard-coded dal file versionato e impostare `SDKROOT` da `scripts/verify.sh` con `xcrun --show-sdk-path`.
- **`justfile`/`Makefile`** con `dev`, `test`, `verify`, `package`, `clean-bridge`.
- **Modalità debug del wheel:** overlay opzionale (`?debug=1` o `FLIP_DEBUG_WHEEL=1`) con coordinate puntatore, sorgente
  attiva (DOM/monitor/native drag), `activeIndex`, `epoch`, eventi/s: ridurrebbe drasticamente il tempo di diagnosi dei
  problemi "non funziona sempre".
- **Log strutturati con livelli** (`FLIP_LOG=debug`) e `scripts/tail-events.sh` che formatta `events.jsonl`.
- **Pre-commit hook** (`.githooks/pre-commit`) per `cargo fmt --check` + `npm run lint` + `audit-repository.sh --staged`.

## QA-09 · Accessibilità e internazionalizzazione (Impatto M · Sforzo M)

- Verifica automatica nel harness `WKWebView` (QA-01) con `axe-core` per ruoli/nome/contrasto della wheel e delle viste.
- Controllare la navigazione **da tastiera in drag mode** (REL-06) e l'ordine di focus quando la wheel è key (idle).
- i18n: EN/IT con test di parità chiavi; aggiungere test su pluralizzazione (`Intl.PluralRules`) per stringhe con numero e
  un controllo di lunghezza massima nelle etichette dei settori (es. etichette lunghe in IT che oggi fanno `text-overflow: ellipsis`).

## QA-10 · Roadmap già dichiarata (promemoria)

Da `ROADMAP.md`: accettazione fisica su WindowServer, notarizzazione (vedi SEC-06), Finder Sync Extension, App Intents,
binario universale `arm64`+`x86_64`, matrice iWork/Pandoc. Suggerimento: per il binario universale, `lipo` fra due build
`aarch64-apple-darwin` e `x86_64-apple-darwin` e `swiftc -target x86_64-apple-macosx15.0` nel bridge — **non** prima di aver
automatizzato il rilascio (SEC-06), perché raddoppia i tempi di build e di test.
