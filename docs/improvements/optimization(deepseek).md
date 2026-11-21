# Piano di ottimizzazione e riduzione del codice — `optimization(deepseek)`

> Stato: **proposta del 2026-11-18** · Nessuna riga di codice modificata da questo piano.
> Documenti collegati: [00 Panoramica](00-OVERVIEW.md) · [02 Performance e architettura](02-PERFORMANCE-ARCHITECTURE.md) · [04 Test, CI, DX](04-TESTING-CI-DOCS-DX.md)
> Livelli di fiducia usati sotto: **[Verificato]** = misurato o letto nel codice in questa sessione · **[Inferenza]** = conseguenza logica, da confermare · **[Da misurare]** = serve una misura prima/dopo.

## 0. Metodo, limiti e cosa NON è stato fatto

Analisi **statica**: conteggi (`wc -l`, `du`), `grep` mirati, `cmp` byte-per-byte sui file sospetti duplicati, lettura delle porzioni di codice citate.

**Non** fatto, e quindi **non** affermato qui:

- nessuna build né `cargo check` / `tsc --noEmit` eseguita in questa sessione;
- nessun profiling (Instruments, tempi di avvio a freddo, costo GPU dei `backdrop-filter`);
- nessun test eseguito.

⚠️ Il worktree **ha modifiche non committate** (`src-tauri/src/engines/image.rs`, `engines/mod.rs`, nuovo `engines/image_tools.rs`, `pnpm-lock.yaml` e `pnpm-workspace.yaml` non tracciati). Qualunque misura "prima" va fatta su un albero pulito o dichiarando il commit di riferimento.

**Nota di onestà sulla premessa.** Il progetto **non è grasso**: la sua documentazione (2.500 righe) è coerente e datata, la suite di test (Rust ≈ 4.925 righe, TS ≈ 4.248) è la rete che rende sicuri i refactor qui sotto. Il guadagno realistico è **5–10 % delle righe di sorgente** e **~0,6 MB di repo**, non una riscrittura.

---

## 1. Fotografia misurata

| Voce | Valore | Fiducia |
|---|---|---|
| `src-tauri/src/cli.rs` | 2.068 righe | **[Verificato]** |
| `src-tauri/src/engines/document.rs` | 6.391 righe | **[Verificato]** |
| `src-tauri/src/core/preferences.rs` | 397 righe | **[Verificato]** |
| `src/views/WheelView.ts` | 2.369 righe | **[Verificato]** |
| `src/views/BatchView.ts` / `SettingsView.ts` | 748 / 1.676 righe | **[Verificato]** |
| TypeScript totale in `src` | 13.183 righe (di cui `views.e2e.test.ts` 2.857) | **[Verificato]** |
| CSS in `src/styles` | 1.902 righe (`wheel.css` 1.030, `app.css` 872) | **[Verificato]** |
| `.unwrap()` nei sorgenti Rust | 361 occorrenze | **[Verificato]** |
| `innerHTML` in `src/views` | 30 occorrenze (Batch 14, Wheel 13, Settings 2, Onboarding 1) + 3 nei test e2e | **[Verificato]** |
| `src-tauri/target/` | 12 GB (ignorato da git) | **[Verificato]** |
| `node_modules` / `dist` | 115 MB / 260 KB | **[Verificato]** |
| File tracciati da git | 7,2 MB | **[Verificato]** |

---

## 2. Interventi proposti

### FASE 1 — Deduplicazione a rischio quasi zero (nessun cambio di comportamento)

#### OPT-01 · Ciclo di parsing argomenti in `cli.rs` (5 copie) — Impatto M · Sforzo S

**[Verificato]** Lo stesso preambolo compare 5 volte, in `parse_convert_args` (riga 173), `parse_new_args` (232), e nei parser di `wheel` (318), `settings`/`batch` (367) e `preview` (408):

```rust
let mut positional_only = false;
while i < args.len() {
    let arg = &args[i];
    if !positional_only && arg == "--" { positional_only = true; i += 1; continue; }
    if !positional_only && (arg == "--help" || arg == "-h") { return Ok(CliCommand::Help); }
    ...
}
```

**[Verificato]** Inoltre la costruzione del JSON d'errore `{"success": false, ..., "error": msg}` con `unwrap_or_else(|_| "{\"success\":false}".to_string())` è ripetuta alle righe **865, 909, 943, 1396, 1413, 1476** (6 copie), più il gemello `out_json` alla 1426.

**Azione:** introdurre un piccolo `ArgCursor` (`next()`, `eat("--")`, `help_requested()`, `value_after(flag)`) + un `fn fail_json(command: Option<&str>, msg: String) -> CliOutcome`. I 5 parser diventano descrittivi; il comportamento (incluso il vincolo *positional-only* dopo `--`) resta identico.

**Stima di riduzione:** **[Inferenza]** ~150–250 righe. Va misurata con `wc -l` prima/dopo, non promessa.

**Rete di sicurezza:** i test di `cli.rs` coprono solo 4 funzioni (`cli_parser_handles_gui_launch_flags_help_version_convert_and_new`, `cli_execute_convert_and_new_end_to_end_json_and_exit_codes`, `cli_parser_and_ipc_execution_for_wheel_settings_batch_and_quit`, `cli_preview_parses_flags_and_generates_well_formed_wheel_svg`). **Prima** del refactor va aggiunto un test di tabella sul parsing: ogni forma (`--to=x`, `-t x`, `--` , argomento con `-` iniziale) × ogni comando. Senza questo, OPT-01 è un rischio, non un guadagno.

#### OPT-02 · Normalizzazione copiata in `core/preferences.rs` — Impatto S · Sforzo S

**[Verificato]** In `sanitize()` (righe 223–255) e in `push_recent()` (256–275) il blocco `recent_conversions.truncate(10)` + backfill di `default_preferred_formats()` è **identico** (righe 243–254 e 262–273).

**Azione:** un metodo privato `fn backfill_preferred_formats(&mut self)` chiamato da entrambi.

**Riduzione:** ~12 righe. Piccola, ma toglie il caso classico "ho corretto un ramo e non l'altro" (il test `sanitize_preserves_custom_order_while_backfilling_new_defaults` protegge già).

#### OPT-03 · Helper di escaping/zip duplicati fra moduli — Impatto S · Sforzo S

**[Verificato]** Stessa logica, definita due volte:

| Helper | Copia A | Copia B |
|---|---|---|
| `html_escape` | `engines/document.rs:25` (`pub`) | `engines/data.rs:1615` (privato) |
| `xml_escape` | `engines/document.rs:143` (`pub`) | `core/startup.rs:59` (`pub`) |
| `read_zip_entry_string` | `engines/document.rs:1731` | `engines/data.rs:1065` |

**Azione:** una sola implementazione in un modulo comune (es. `core/text.rs` per l'escaping, `engines/zip_util.rs` per lo zip), con `pub(crate) use` nei moduli che li usano.

**Attenzione [Verificato]:** `document.rs` ha anche `xml_escape_text` (riga 161), che **non** è un duplicato ma una variante con semantica diversa. Va confrontata riga per riga prima di unificare: unificare per nome e non per comportamento è il modo più facile di introdurre un bug invisibile in XML/EPUB.

---

### FASE 2 — Peso del repository (nessuna funzione persa)

#### OPT-04 · Tre file tracciati duplicati byte-per-byte — Impatto S · Sforzo S

**[Verificato con `cmp`]**

| File | Gemello | Dimensione |
|---|---|---|
| `docs/assets/Flip.ico` | `src-tauri/icons/icon.ico` | identici |
| `docs/assets/flip-tick.wav` | `src-tauri/resources/flip-tick.wav` | identici |
| `src-tauri/gen/schemas/desktop-schema.json` | `src-tauri/gen/schemas/macOS-schema.json` | identici, 117.263 byte **ciascuno** |

**Azione:** le due copie in `docs/assets` diventano reference Markdown verso l'asset canonico in `src-tauri/`; per gli schemi si tiene **uno** dei due (o, meglio, si smette di tracciare `src-tauri/gen/schemas/` e lo si rigenera con `cargo tauri build`).

**Da verificare prima:** che `gen/schemas` non sia letto a runtime da nessun percorso e che la CI lo rigeneri. **[Verificato]** che oggi è interamente tracciato (4 file, ~300 KB) e che `.gitignore` non ne parla.

**Guadagno:** ~237 KB tracciati + 2 file. **[Da misurare]**: nessun impatto su runtime.

#### OPT-05 · Spazio locale — Impatto B (zero sul repo) · Sforzo S

**[Verificato]** `src-tauri/target/` = 12 GB, `node_modules` = 115 MB. Sono già ignorati: è solo spazio su disco. `cargo clean` + reinstallazione li liberano, costo = una build completa in più.

#### OPT-06 · `dist/` stantio — Impatto B · Sforzo S

`dist/` è ignorato e datato. Non è un problema di repo, ma è un modo per misurare per sbaglio un bundle vecchio: va rigenerato prima di qualsiasi misura di performance frontend.

---

### FASE 3 — Struttura (prepara le ottimizzazioni vere, non le promette)

#### OPT-07 · `engines/document.rs` (6.391 righe) — Impatto M · Sforzo L

Il file contiene almeno: escaping (`html_escape`, `xml_escape`, `xml_escape_text`), conversione testo/HTML (`plain_text_to_html`, `html_to_plain_text`, `html_to_plain_text_with_fallback`), estrazione (`extract_html_title`, `extract_html_body_slice`), IO zip. **Azione:** split per formato/servizio in un submodulo `engines/document/`, tenendo le API `pub` invariate.

**Onestà:** non è una riduzione di righe, è una riduzione di *rischio*: rende possibili i lavori successivi (es. PERF-05, tokenizer HTML) e accorcia le revisioni. **[Inferenza]**

#### OPT-08 · `WheelView.ts` (2.369 righe) e `innerHTML` — Impatto M · Sforzo L

**[Verificato]** 30 `innerHTML` in 4 view. **[Verificato]** la metà "escaping" di SEC-01 è già fatta (`ccab327`: test d'iniezione su Batch ed Error Hub della Wheel); **la CSP non è attiva** (`csp: null`), e i tagged template con escape automatico non ci sono.

**Azione:** far passare tutti i 30 punti da un unico `html` tagged template (impossibile dimenticare l'escape) e poi attivare la CSP. Questo è **anche** l'intervento di sicurezza a più alto rapporto impatto/costo del progetto — già schedato come SEC-01, qui non lo duplico, lo collego.

---

### FASE 4 — Cose da NON fare (per non rompere nulla)

| Tentazione | Perché no |
|---|---|
| Cancellare `docs/0x` o `PLAN-1..5` | Sono marcati "storici" e datati: costo zero, valore di tracciabilità |
| Ridurre i test per "pulizia" | Sono l'unica rete per FASE 1 e FASE 3; le 4 funzioni di test CLI sono già **troppo poche**, non troppe |
| Sostituire i 361 `.unwrap()` in massa | Molti sono in test o su invarianti locali; un `?` cieco cambia messaggi d'errore e contratti CLI |
| Ottimizzare "a occhio" `backdrop-filter` / apertura wheel | Richiede Instruments: senza misura è [Da misurare], e il progetto lo sa già (PERF-08, REL-07) |
| Unificare `xml_escape`/`xml_escape_text` per nome | Semantica diversa [Verificato]: bug silenzioso in EPUB |
| Scegliere npm vs pnpm senza Capo | Scelta di processo (QA-04), non tecnica |

---

## 3. Ordine di esecuzione suggerito

| # | Passo | Prerequisito | Rischio | Guadagno atteso |
|---|---|---|---|---|
| 1 | Test di tabella sul parser CLI | — | nullo | rende sicuro il passo 2 |
| 2 | OPT-01 `ArgCursor` + `fail_json` | passo 1 | basso | **[Inferenza]** ~150–250 righe |
| 3 | OPT-02, OPT-03 (con confronto semantico degli escape) | — | basso | ~80–120 righe, meno duplicazione |
| 4 | OPT-04 duplicati byte-per-byte | verifica uso di `gen/schemas` | nullo | ~237 KB di repo |
| 5 | OPT-05/06 pulizia locale + `dist` | — | nullo | 12 GB su disco |
| 6 | Misura "prima" dichiarata (commit + comandi) | worktree pulito | — | rende misurabile tutto il resto |
| 7 | SEC-01 completo (tagged template + CSP) | — | medio | sicurezza, non righe |
| 8 | OPT-07 split `document.rs` | 1–3 | medio | manutenibilità |
| 9 | OPT-08 `WheelView.ts` | 7 | medio-alto | manutenibilità |

## 4. Criteri di accettazione (perché "senza perdere funzioni")

Per **ogni** intervento:

1. `cargo test` + `cargo clippy -- -D warnings` verdi (il progetto li impone già in CI).
2. `bun test` (o il runner documentato) verde: 59 test attuali invariati.
3. `cli_execute_convert_and_new_end_to_end_json_and_exit_codes` verde: **il contratto JSON della CLI non cambia** (chiavi `success`/`command`/`error`, codici d'uscita).
4. `wc -l` prima/dopo registrato nel commit: nessuna affermazione di riduzione senza numero.
5. Per gli schemi: `cargo tauri build` li rigenera identici.

## 5. Stima complessiva, dichiarata come stima

- **Righe di sorgente:** **[Inferenza]** −5 % / −10 % (≈ 700–2.200 righe su Rust+TS), di cui solo OPT-01..03 sono "sicure" e piccole.
- **Repo:** **[Verificato]** ~237 KB di duplicati esatti, più eventuali ~300 KB se si smette di tracciare `gen/schemas`.
- **Prestazioni:** **nessun guadagno promesso**. Gli unici interventi con impatto reale sono già a backlog (PERF-03 bridge persistente, REL-07 apertura wheel asincrona) e vanno **misurati prima/dopo**.
- **Sicurezza:** l'unico guadagno concreto e verificabile è completare SEC-01 (CSP + escaping obbligatorio), che questo piano non sostituisce ma richiama.
