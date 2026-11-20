# 03 · Sicurezza, privacy e rilascio

> Parte del piano in [00-OVERVIEW.md](00-OVERVIEW.md). Stato: **proposta**.
> Livelli di fiducia: **[Verificato]** · **[Inferenza]** · **[Da misurare]**.
> Nota: nessun problema critico è emerso; sono interventi di *difesa in profondità* e di preparazione alla distribuzione pubblica.

---

## SEC-01 · Content Security Policy assente (Impatto A · Sforzo S)

**Problema [Verificato].** `src-tauri/tauri.conf.json` ha `app.security.csp: null` insieme a `macOSPrivateApi: true`.
Il frontend costruisce HTML con template string e `innerHTML` in ~30 punti (`WheelView.ts` 13, `BatchView.ts` 14,
`SettingsView.ts` 2, `OnboardingView.ts` 1); gli input non fidati sono **nomi di file arbitrari** e messaggi di errore.
Oggi si affida tutto a `escapeHtml()` — corretto, ma una dimenticanza futura diventerebbe esecuzione di script nella finestra
con accesso ai comandi Tauri.

**Proposta.**
1. CSP restrittiva, da validare in dev e in build:
   `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: asset:;
   connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`
   (`style-src 'unsafe-inline'` serve finché si usano `style="--bubble-*"` inline; le proprietà impostate via
   `element.style.setProperty` non violano la CSP).
2. Tagged template `html\`…\`` che esegue l'escape di default (e `raw()` esplicito) al posto dell'interpolazione manuale;
   regola di lint che vieta `innerHTML = ` con template non taggati.
3. Test E2E che inietta un nome file `"><img src=x onerror=…>` e verifica che non finisca nel DOM come markup (in parte già
   coperto da `escapeHtml`, ma va fissato a livello di vista).

## SEC-02 · Superficie dei comandi per finestra (Impatto M · Sforzo M)

**Problema [Verificato].** Esiste un'unica capability `default` con `core:default` per tutte e quattro le finestre
(`src-tauri/capabilities/default.json`). I 28 comandi applicativi (`save_preferences`, `install_finder_quick_actions`,
`execute_create_new`, `open_window`, …) sono invocabili da qualunque finestra, inclusa la wheel (trasparente, sempre in
primo piano, che mostra nomi di file).

**Proposta.** In `src-tauri/build.rs` dichiarare i comandi nell'app manifest
(`tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[...]))`) e creare una capability per
finestra: `wheel` solo con i comandi necessari (`get_wheel_state`, `inspect_files`, `execute_conversion`, `execute_create_new`,
`dismiss_wheel`, `play_wheel_feedback`, `get_thumbnail`, `get_preferences`, `get_new_file_kinds`, `switch_wheel_mode`,
`open_batch…`), `settings` con i comandi di configurazione, ecc. Il test `frontend_and_rust_command_and_event_names_match_exactly`
si estende a verificare che ogni comando sia assegnato ad almeno una capability.

## SEC-03 · Validazione dei path nei comandi di sola lettura (Impatto M · Sforzo S)

**Problema [Verificato].** `execute_conversion` passa da `validate_input_files` (symlink, alias, volumi di rete, limiti), ma
`get_thumbnail(path)` e `reveal_in_finder(paths)` in `host/commands.rs` inoltrano il path al bridge **senza validazione**
(`bridge::thumbnail(Path::new(&path), …)`, `bridge::finder_reveal(&refs, false)`). Per una webview locale il rischio è basso,
ma la miniatura di un file arbitrario è una lettura di dati utente.

**Proposta.** `validate_user_path()` unica in `core/policy.rs` (canonicalizzazione, rifiuto symlink/alias/non-locali, esistenza)
riusata dai tre comandi, con test accanto a quelli di `validate_input_files`. Per `get_thumbnail`, accettare solo path
presenti nello stato corrente della wheel/batch (`wheel.payload.files` o lista batch registrata).

## SEC-04 · Event tap attivo sempre (Impatto M · Sforzo M)

**Osservazione [Verificato].** `Monitor.swift` crea un `CGEventTap` **`.defaultTap`** (che può modificare/consumare eventi)
su `keyDown/keyUp/flagsChanged` per tutta la sessione. Il codice emette solo `Esc`, `R`, il tasto trigger e lo shortcut idle e
non registra altri tasti — buona base. Rischi residui: (a) un tap attivo che si blocca può ritardare l'input di sistema (macOS
lo disabilita con `tapDisabledByTimeout`, già gestito); (b) la percezione di "keylogger" in fase di onboarding/review.

**Proposta.**
1. Tenere il tap in modalità **`.listenOnly`** a riposo e passare a `.defaultTap` solo quando serve consumare un tasto
   (`dragActive` o `wheelOpen`), oppure ricreare il tap on demand.
2. Documentare in `SECURITY.md` e nell'onboarding **cosa viene letto e cosa mai salvato** ("nessun tasto viene registrato o
   inviato"), e aggiungere un test che garantisca che nessun keycode fuori dalla tabella di scorciatoie venga scritto in
   `events.jsonl` o su stdout.

## SEC-05 · Updater (Impatto B · Sforzo S)

**[Verificato].** `core/updater.rs` usa `/usr/bin/curl --proto =https` verso l'API GitHub e apre la pagina della release nel
browser (`open_release_url` valida il dominio): nessuna installazione automatica, quindi rischio contenuto.
Se in futuro si vorrà l'auto-update: `tauri-plugin-updater` con firma minisign, non uno script `curl`.
Nel frattempo: limitare la dimensione della risposta (`--max-filesize`) e il numero di redirect (`--max-redirs 3`).

## SEC-06 · Notarizzazione, entitlements e workflow di release (Impatto A · Sforzo L)

**Stato [Verificato].** Build ad-hoc; `ROADMAP.md §2` dichiara la notarizzazione ancora da fare. `scripts/sign-local.sh` usa
`--options runtime` con identità Developer ID e accetta `APPLE_ENTITLEMENTS`, ma **nessun file entitlements è versionato**.
`Info.plist` contiene `LSUIElement` e `NSAppleEventsUsageDescription`.

**Proposta.**
1. Versionare `src-tauri/entitlements.plist` minimo (con `com.apple.security.automation.apple-events`) e verificare
   con una build notarizzata di prova se servono altre eccezioni (la webview WKWebView gira nei processi Apple e di norma non
   richiede `allow-jit` per l'host: **[Da verificare]** in pratica).
2. Workflow `release.yml` su tag `v*`: build → firma Developer ID → `notarytool submit --wait` → `stapler staple` →
   GitHub Release → aggiornamento di `packaging/homebrew/flip.rb` (versione + SHA-256) con un job separato/PR automatica.
3. Controllo post-build in CI: `spctl --assess --type execute`, `codesign --verify --deep --strict` (già presente), `stapler validate`.
4. Secrets (`APPLE_*`) solo su ambiente protetto del repository, mai nei log.

## SEC-07 · Path completi dei file utente in `events.jsonl` (Impatto A · Sforzo S)

**Problema [Verificato].** Il file di log (`~/Library/Logs/FLIP/events.jsonl`) contiene i path **completi** dei file
convertiti, ad es. `{"name":"conversion.failed","detail":{"source":"<path assoluto con nome utente>", …}}`. La funzione
`redact_user_paths` (`core/logging.rs`) viene applicata solo quando si genera il report Diagnostics ("redacted"), **non alla
scrittura**. Il log resta quindi sul disco (rotazione a 2 MiB) con nomi di file e cartelle personali.

**Proposta.**
- Redigere alla scrittura: sostituire la home con `~` e, per i nomi file, registrare solo estensione + hash troncato
  (`sha256(path)[..8]`) salvo modalità `FLIP_LOG_VERBOSE=1`.
- Aggiornare `SECURITY.md` ("cosa viene registrato") e il test `redacts_user_home_…` perché copra anche il percorso di scrittura.
- Retention: oltre la rotazione a 2 MiB, cancellare i log più vecchi di 14 giorni.

## SEC-08 · Path di build incorporato nel binario (Impatto B · Sforzo S)

**[Verificato].** `env!("CARGO_MANIFEST_DIR")` è usato in codice non di test in `engines/bridge.rs:78` e `host/mod.rs:312`:
il percorso assoluto della macchina di build finisce nel binario release e abilita in produzione un fallback pensato per lo
sviluppo. **Proposta:** racchiuderlo in `#[cfg(debug_assertions)]` (o `option_env!` condizionato) e testare che
`resolve_bridge_binary_from` in release non consulti la working copy.

## SEC-09 · `unsafe` e `unwrap` (Impatto B · Sforzo S)

**[Verificato].** ~20 occorrenze di `unsafe` (`ipc_socket.rs` 6, `policy.rs` 5, `startup.rs` 4, `document.rs` 2, `host/mod.rs` 1,
`archive.rs` 1, `staging.rs` 1), in larga parte chiamate `libc` (`geteuid`, `getpeereid`, `renamex_np`, `flock`).
Fuori dai moduli di test restano ≈14 `unwrap/expect/panic` (stima: `document.rs` 6, `logging.rs` 2, `input_gate.rs` 2, altri 1 ciascuno).
**Proposta.** `#![warn(clippy::undocumented_unsafe_blocks)]` (poi `deny`), un commento `// SAFETY:` per ogni blocco e
`#![warn(clippy::unwrap_used, clippy::expect_used)]` con `cfg_attr(test, allow(...))`; correggere i 14 casi.

## SEC-10 · Controlli sulla supply chain (Impatto M · Sforzo S)

- **`npm audit`**: 0 vulnerabilità al momento della revisione **[Verificato]**.
- **`cargo audit`**: **non eseguito con esito** durante la revisione (recupero dell'advisory DB non completato) —
  **[Da verificare]**; aggiungerlo alla CI (`cargo audit` o `cargo deny check advisories`) con job schedulato settimanale.
- Dependabot/Renovate per `cargo`, `npm` e `github-actions`; pin degli action per SHA nei workflow.
- `cargo deny check licenses` per sostituire la manutenzione manuale di `THIRD_PARTY_NOTICES.md`.

## Note positive (nessuna azione richiesta)

**[Verificato]** Gli script di packaging usano `set -euo pipefail`, directory temporanee con `mktemp` e `trap cleanup EXIT`
(`package-dmg.sh`); `sign-local.sh` firma prima il bridge e poi l'app. Il socket IPC è `0600` con directory `0700`, controllo
`getpeereid` sullo stesso utente e rifiuto di symlink: da mantenere come riferimento per le nuove superfici (es. RPC del bridge, PERF-03).
