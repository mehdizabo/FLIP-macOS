# Miglioramenti possibili — Panoramica e priorità

> Stato: **proposta del 2026-11-19 (commit `298874e`); lo Sprint 1 è stato applicato in parte il 2026-11-20, vedi §6** · Le tabelle §2–§5 descrivono il codice *prima* dello Sprint 1
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
| 16 | QA-04 | Un solo package manager (pnpm; `package-lock.json` rimosso il 2026-11-20) | B | S | [04](04-TESTING-CI-DOCS-DX.md) |

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
| REL-04 | Fatto (variante prudente) | `31ec68b` | La molla non supera più il tempo reale: eventi a < 2 ms dallo step precedente vengono accumulati, non arrotondati a 6 ms (con mouse a 1 kHz il vecchio codice integrava **6000 ms per 100 ms reali, 60×**). Resta lo step per evento (non spostato in rAF) per non riaprire il fix WKWebView di `607b36f`. Lo stub rAF dei test usava `Date.now()` invece di `performance.now()`: corretto. **Controllo dal vivo necessario** con un mouse ad alta frequenza (il feel è stato tarato a mano): `Monitor.swift` emette la posizione quando il cursore si sposta di ≥ 0,4 pt, **senza limite di frequenza**, quindi in drag con un mouse oltre ~500 Hz il vecchio codice poteva davvero accelerare la molla e questa modifica cambia il feel. Con mouse/trackpad più lenti non cambia nulla. Annullabile con `git revert 31ec68b`. |
| SEC-01 | Solo metà: escaping | `ccab327` | Test d'iniezione su Batch (lista e risultati) ed Error Hub della Wheel; prova rossa togliendo `escapeHtml`. **La CSP non è attiva** (vedi 03). Tagged template e regola di lint non fatti. |
| QA-03 | Minimo | `5734e9e` | `concurrency` con `cancel-in-progress` e `audit-repository.sh` in CI (passa in locale; `actionlint` pulito salvo un SC2012 già presente). Non fatti: split `verify`/`package`, `cargo audit`, pin degli action, rimozione dello step `tauri-cli` (serve ancora: `build-macos.sh` usa `cargo tauri build`). Non eseguibile senza un push. |
| QA-04 | Fatto (2026-11-20) | vedi `git log` | Scelta di Capo: pnpm. `package-lock.json` rimosso, `pnpm-lock.yaml` e `pnpm-workspace.yaml` (`allowBuilds: esbuild: true`) tracciati, `packageManager: pnpm@12.16.1`, `pnpm/action-setup@v6` in CI, `tauri.conf.json`, `build-macos.sh` e README aggiornati. La CI resta non eseguibile senza un push. |
| REL-01, REL-02, REL-05, SEC-02, PERF-03, PERF-05 | Non iniziati | — | Sprint 2–3. |

## 7. Novità 0.6 "Aura" portate nel port (ramo `feat/0.6-tools`, 2026-11-20)

Dal video del FLIP originale 0.6: gli **strumenti immagine** e la **seconda faccia della ruota**. La rimozione dello sfondo
c'era già; la parità delle estensioni con l'originale è verificata (0 mancanti, il port è un sovrainsieme) e ora protetta
da un test Rust che incorpora le tabelle di `Policy.cs`.

| Voce | Stato | Prova |
|---|---|---|
| Motore strumenti (`engines/image_tools.rs`): ritaglio 9:16 · 4:5 · 1:1 · 16:9, ruota/specchia, scala 50 % · 25 % · 1080p · 720p · sotto 1 MB, palette, rimozione EXIF/GPS | Fatto | 14 test unitari + 9 di integrazione (dimensioni esatte, pixel in posizione dopo rotazione/specchio, `<1 MB`, EXIF assente e immagine ancora dritta, PNG `tEXt` pulito con CRC valido, nessuna sovrascrittura, originale intatto). Prova a mano con la CLI reale su una foto 6016×6016: crop `3384×6016` (9:16 esatto), 1080p `1080×1080`, sotto-1MB `883 KB`, palette PNG con 6 colori e codici hex corretti. |
| Faccia "Strumenti" della ruota (`T`, interruttore nella barra, Ritaglia/Ruota/Scala come sotto-pagine con slot "su", dwell in drag) | Fatto, **non visto dal vivo** | e2e `WheelView: Tools face…` (con prova rossa: rompendo l'apertura dei gruppi il test fallisce), `tsc`, 77 test bun, `vite build`. Il layout reale della barra a tre pulsanti e dei sei/otto bubble **non è stato guardato**: il pannello browser integrato è stato rifiutato. |
| Tasto `T` anche durante il drag (`Monitor.swift` → `monitor.rs` → `wheel:toggle-face`) | Compilato, **non provato dal vivo** | `swiftc -typecheck`, `clippy -D warnings`, test IPC host (elenco eventi TS↔Rust). Vedi `ACCEPTANCE.md` §14 "Tools during a Finder drag". |
| Contratto TS↔Rust | Fatto | Test `imageTools.test.ts`: la lista dei token e i suffissi TS devono coincidere con `ALL_TOOL_TOKENS` / `ImageTool::suffix` in Rust. |
| Oscura (`REDACT`): il bridge Swift risponde `ocr-boxes` (un riquadro per parola, in spazio raddrizzato), `engines/redact.rs` riconosce IBAN, carte, email, telefoni, codice fiscale e P.IVA, e `engines/image_tools.rs` li copre con riquadri neri pieni; nessun dato trovato = errore e nessun file | Fatto, **voce nella ruota non vista dal vivo** | 43 test dei riconoscitori (IBAN mod-97, Luhn, cifre di controllo di codice fiscale e P.IVA, nessun falso positivo su date e importi), 6 unitari del motore e 3 d'integrazione sul bridge vero (PNG, JPEG con EXIF 6 senza EXIF in uscita, nessun file su un'immagine senza testo), e un test che confronta i riquadri OCR con quelli attesi per la fixture con dati finti. Prova a mano con la CLI reale sulla fixture: i sei valori sono coperti, la riga «Invoice …» resta leggibile, l'originale è invariato; su un'immagine senza testo esce con 1 e non scrive nulla. L'OCR può non leggere un dato (foto storta, font strano): per questo l'etichetta chiede sempre di controllare il risultato. Da vedere dal vivo: `ACCEPTANCE.md` §14 «Tools: Redact». |
| Trascrizione (`TRANSCRIBE`, Whisper locale): `engines/transcribe.rs` (ffmpeg → wav 16 kHz → `whisper-cli`, lingua automatica, solo `.txt`), `scripts/install-whisper.sh`, etichette e riga in Impostazioni → Motori | Motore e UI fatti, **non ancora collegato all'app** (manca P3) | 12 test del motore con ffmpeg e whisper finti (avanzamento, annullamento, errori, percorsi con spazi e accenti, file muto). **Provato col vero `whisper-cli` 1.9.5** (Homebrew, formula `whisper.cpp`) e il modello `ggml-small.bin` (SHA-1 verificato sul README del progetto): un vocale italiano e uno inglese trascritti correttamente con lingua rilevata da sola, file con spazio e accento nel nome, nessun temporaneo rimasto. **Trovato in prova:** su un file muto Whisper scrive «you» e nessun suo flag lo evita, quindi il motore misura il picco con `ffmpeg volumedetect` e rifiuta i file sotto −50 dB con l'errore «nessun parlato». **Resta scoperto il rumore di fondo sopra −50 dB** (misurato con rumore rosa a −46,6, −36,5 e −27,1 dB): Whisper emette caratteri a caso (`ᶠᶠᶠ…`, khmer) e il motore li scrive in un `.txt`; sotto −56 dB non emette nulla e l'errore scatta. L'avanzamento è live: su 3 min 24 s di audio `whisper-cli -pp` stampa 12 %, 26 %, 39 %, 51 %, 64 %, 76 %, 89 %, 100 %, e la trascrizione richiede circa 12 s. Limiti: il cancello globale `gate()` fa aspettare le altre conversioni per tutta la durata; il filtro scarta anche ogni riga interamente tra parentesi (`[…]`, `(…)`). |
| PDF/A (`PDFA`): comando bridge `pdfa` con `kCGPDFContextCreatePDFA` | Spike concluso: **NO-GO** (ramo `side/pdfa-spike` tenuto, non fuso) | Misurato: l'XMP (compresso) dichiara PDF/A **parte 2, conformità B** e non U come dice l'header dell'SDK; ha `/OutputIntents` (`GTS_PDFA1`); pagine, dimensioni e testo si conservano (la rotazione diventa orientamento visibile, senza `/Rotate`); i **link si perdono**; sopra 200 pagine rifiuta. **Validato poi con veraPDF 1.30.0** (installato dal lead il 2026-11-20 con il sì di Capo): su 5 file vari (pagina raster, PDF con Helvetica non incorporata, tre PDF Apple) l'output passa il PDF/A-2b e gli input no; su un campione reale di 16 manuali PDF installati sul Mac **passano 8 e falliscono 8**: 7 violano la regola `6.2.11.3.2-1` (font TrueType a codifica CID senza `CIDToGIDMap`) e 1 la `6.2.4.2-2` (overprint con CMYK ICC). A 2u fallisce anche la dichiarazione nei metadati (`6.6.4-3`). Quindi il file *dichiara* 2b ma è conforme solo in metà dei casi reali, e il wrapper non se ne accorge (controlla solo la dichiarazione): il NO-GO è confermato dai fatti. Strada non provata: Ghostscript con `-dPDFA=2` (AGPL, installazione pesante: 42,9 MB più 12 dipendenze nuove e 10 aggiornamenti), solo con il sì di Capo. |
| Aura overlay, anteprima Space | **Non fatti** | Ondata 3, ancora da preparare. Tobii saltato (su macOS esiste il Dwell Control di sistema). |

Le due ondate che restano (Whisper e PDF/A; Aura e anteprima Spazio; la prima, Oscura, è fusa), con divisione dei compiti, contratti,
arresti obbligati e valori predefiniti delle domande aperte, sono in [`05-0.6-NEXT-WAVES.md`](05-0.6-NEXT-WAVES.md).
