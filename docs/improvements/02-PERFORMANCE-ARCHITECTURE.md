# 02 · Performance, struttura del codice e architettura

> Parte del piano in [00-OVERVIEW.md](00-OVERVIEW.md). Stato: **proposta**.
> Livelli di fiducia: **[Verificato]** · **[Inferenza]** · **[Da misurare]**.
> Regola generale: **misurare prima e dopo** — nessuno di questi interventi va fatto "a occhio".

---

## PERF-01 · Profilo `release` e feature di `tokio` (Impatto M · Sforzo S)

**Problema [Verificato].** `src-tauri/Cargo.toml` non ha alcuna sezione `[profile.*]`; il binario release
`src-tauri/target/release/flip` pesa ≈ 21 MB. `tokio` è dichiarato con `features = ["full"]` ma il codice usa solo
`tokio::task::spawn_blocking` e `tokio::time::sleep` (`host/commands.rs`, `lib.rs`).

**Proposta.**
```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
# panic = "abort"  # NON consigliato qui: i thread di monitor/IPC devono poter fallire senza abbattere l'app

[dependencies]
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "time"] }
```
**[Da misurare]**: dimensione `.app`/`.dmg`, tempo di build CI, tempo di avvio a freddo. Se `lto` allunga troppo la CI,
usare un profilo dedicato (`[profile.dist]`) solo per i rilasci.

## PERF-02 · Igiene delle dipendenze (Impatto M · Sforzo S)

- **`serde_yaml 0.9`** [Verificato in `Cargo.toml`]: il crate è dichiarato non più mantenuto dall'autore; valutare un
  sostituto mantenuto (`serde_norway`/`serde_yml`) dietro un piccolo adattatore in `engines/data.rs` e coprire con i test di
  roundtrip YAML esistenti.
- **`dirs 5`**: la 6.x è disponibile; verificare il changelog e aggiornare (modifica minima: `dirs::home_dir`).
- `cargo tree -d` mostra duplicati (`base64`, `digest`, `cpufeatures`, `crypto-common`, `block-buffer`) **[Verificato]**: in gran parte
  trascinati da `tauri` e da `sha2` (dev-dependency) — non agire a mano, ma ricontrollare dopo ogni update di Tauri.
- Aggiungere `cargo deny` (licenze + advisories + bans) alla CI (vedi QA-03) e allineare `THIRD_PARTY_NOTICES.md`
  con `cargo about`/`cargo deny list` invece di tenerlo a mano.

## PERF-03 · Bridge Swift persistente (RPC) invece di un processo per richiesta (Impatto A · Sforzo M)

**Problema [Verificato].** `engines/bridge.rs` avvia un **nuovo processo** `flip-macos-bridge <comando>` per ogni richiesta:
`finder_context()` (a ogni apertura idle), `clipboard_inspect()` (a ogni apertura in modalità New, in `get_new_file_kinds`),
`thumbnail()` (a ogni cambio di file nell'hub), `permissions`, `finder_reveal`, ecc., con polling `sleep(20 ms)` in
`run_external_command`. Esiste già un processo **persistente** (`monitor`) che legge comandi JSON da stdin
(`set_config`, `wheel_state`, `feedback`, `recheck_permissions`).

**Proposta.**
1. Estendere il protocollo stdin/stdout del monitor a RPC con `id`: `{"cmd":"rpc","id":7,"op":"finder_context"}` →
   `{"event":"rpc_result","id":7,"ok":true,"data":{…}}`.
2. Spostare nel monitor, nell'ordine: `finder_context`, `clipboard_inspect`, `thumbnail` (le tre sul percorso caldo).
3. In Rust: `bridge::rpc(op, args, timeout)` con mappa `id → oneshot`; **fallback** allo spawn attuale se il monitor non è
   attivo (permessi mancanti/riavvio) — comportamento invariato nei casi limite.
4. Vantaggio atteso **[Da misurare]**: eliminare il costo di avvio di un processo Swift + caricamento framework a ogni apertura.

## PERF-04 · Frontend: lavoro per hover (Impatto B · Sforzo S)

**Osservazioni [Verificato].**
- `hubEl.innerHTML` viene ricostruito in 6 punti di `renderHub`; è già mitigato da `renderedHubKey` per gli stati idle.
  Dopo la modifica che evita il doppio `renderSectors()` (`slotSignature`), il percorso caldo è già ragionevole.
- `playWheelFeedback("tick")` invia un IPC a ogni cambio settore: accettabile, ma con REL-03/REL-04 conviene
  limitarlo a 1 ogni ~30 ms per evitare raffiche durante spostamenti rapidi sul bordo fra settori.

## PERF-05 · Struttura dei file e duplicazione dei tokenizer HTML (Impatto M · Sforzo L)

### 5a · `src-tauri/src/engines/document.rs` (6.391 righe)

**Problema [Verificato].** Un unico file contiene: lettura/scrittura EPUB 3, FB2, RST, HTML→Markdown, HTML→testo, HTML→RST,
normalizzazione XHTML5, integrazione Pandoc, dispatcher `convert` e i test. Funzioni molto lunghe: `write_fb2_from_html`
(≈1.000 righe fino alla definizione successiva), `html_to_rst` (≈900), `parse_fb2_to_html` e
`normalize_html_to_xhtml5_body` (≈360 ciascuna), `read_epub_to_html` (≈320).

**Duplicazione chiave.** Esistono **almeno sei** scanner HTML scritti a mano, ciascuno con la propria gestione di
commenti/`<script>`/`<style>`/attributi con virgolette: `html_to_markdown`, `html_to_plain_text`, `html_to_rst`,
`normalize_html_to_xhtml5_body`, `find_tag_range_case_insensitive`, `extract_html_title` (quest'ultimo aggiunto per il fix
dell'HTML senza testo). Il bug "Output file is empty" su pagine con solo `<canvas>/<script>` nasceva proprio da questa
frammentazione: ogni funzione decideva da sola cosa fosse "contenuto".

**Proposta.**
1. Modulo `engines/document/` con `mod.rs` (dispatcher `convert`), `html_tokens.rs` (un unico tokenizer con eventi
   `Open{name,attrs,self_closing}`, `Close`, `Text`, `Comment`), `markdown.rs`, `plain.rs`, `rst.rs`, `epub.rs`, `fb2.rs`,
   `pandoc.rs`, `tests/`.
2. Riscrivere i convertitori come consumatori di `html_tokens` **uno alla volta**, mantenendo i test golden esistenti
   (`html_to_markdown_handles_*`, roundtrip) come rete di sicurezza.
3. Valutare un crate (`html5ever`/`lol_html`) solo se il costo in dimensione del binario è accettabile; il requisito di
   sanitizzazione offline (niente inclusioni esterne) va ritestato in ogni caso.

### 5b · `src/views/WheelView.ts` (2.363 righe)

**Problema [Verificato].** Tutto vive in una sola closure `mountWheelView` (≈42 funzioni, decine di `let` mutabili condivise).
Ogni modifica richiede di tenere a mente tutto lo stato (vedi REL-05).

**Proposta.** `src/wheel/` con `state.ts` (reducer/tipi), `input.ts` (arbitro puntatore), `physicsLoop.ts` (rAF),
`render.ts` (sector/hub/progress, funzioni pure che producono markup), `ipcBindings.ts` (sottoscrizioni).
`WheelView.ts` resta il collante (< 300 righe). Procedere per estrazione, non riscrittura: i test E2E fanno da rete.

### 5c · Altri file grandi

`src-tauri/src/cli.rs` (2.068), `native-bridge/Sources/MediaAndDocs.swift` (1.881), `src/views/SettingsView.ts` (1.676),
`engines/data.rs` (1.756), `core/quick_actions.rs` (1.275): stessa strategia — un sottomodulo per responsabilità; per Swift
dividere `MediaAndDocs.swift` in `Audio.swift`, `Video.swift`, `Pdf.swift`, `Documents.swift` (nessun impatto sul comando
`swiftc` se `build.sh` compila `Sources/*.swift`).

## PERF-06 · Funzioni con troppi argomenti (Impatto B · Sforzo S)

**[Verificato].** 8 occorrenze di `#[allow(clippy::too_many_arguments)]` (`host/mod.rs:present_wheel_at` con 9 parametri,
`engines/mod.rs`, `engines/document.rs`). Introdurre `struct PresentWheelRequest { mode, files, is_drag, target_folder,
cursor, screen }` (con `Default`) e analoghi per le conversioni. Rende i punti di chiamata (`lib.rs`, `monitor.rs`,
`commands.rs`) più leggibili e riduce gli errori di ordine degli argomenti.

## PERF-07 · Avvio della finestra wheel e code splitting (Impatto M · Sforzo S)

**[Verificato].** Un solo bundle (216 KB, gzip 56 KB) serve `wheel`, `settings`, `batch`, `onboarding`; la finestra wheel — il
percorso critico — carica e analizza anche il codice delle altre viste.
**Proposta.** Import dinamici per rotta in `src/main.ts`/`router.ts` (`await import("./views/WheelView")`), così la wheel
carica solo `WheelView` + `wheelPhysics` + `wheelGeometry`. **[Da misurare]** con `performance.now()` fra `DOMContentLoaded`
e primo `renderSectors()`. Considerare di tenere la finestra wheel **pre-caricata e nascosta** (già lo è: `visible: false`),
verificando che non venga distrutta/ricreata.

## PERF-08 · Costo GPU del vetro (Impatto M · Sforzo M)

**[Inferenza / Da misurare].** Con la wheel aperta ci sono ~11 superfici con `backdrop-filter: blur(34px) saturate(200%)`
(8 pucks, hub, pill modalità, toggle Raw) in una finestra trasparente 440×440, più `will-change: transform` sui pucks.
Su display ad alta densità/ProMotion può costare frame.
**Proposta.** Misurare con Instruments (Core Animation FPS / GPU) durante lo spostamento del cursore; se necessario:
ridurre il raggio del blur per i pucks non attivi, spegnere il blur dei pucks `is-dimmed`, oppure usare un unico layer di
backdrop con maschera SVG. Rispettare già `prefers-reduced-transparency` (c'è).

## PERF-09 · Cache e lavoro per apertura (Impatto B · Sforzo S)

**[Verificato].** `detect_tools()` (probe di file in `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, PATH, `app_support/tools`) e
`resolve_bridge_binary()` (con `canonicalize`) vengono rieseguiti a ogni `inspect_paths_with_prefs`, quindi a ogni drag e a ogni
`inspect_files`. Sono operazioni economiche (solo `stat`), ma banali da cachare con invalidazione su cambio preferenze
(`prefs:changed`) — beneficio piccolo, rischio quasi nullo.
