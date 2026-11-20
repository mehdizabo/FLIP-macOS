# 01 · Affidabilità di input, focus e wheel

> Parte del piano in [00-OVERVIEW.md](00-OVERVIEW.md). Stato: **proposta**.
> Livelli di fiducia: **[Verificato]** · **[Inferenza]** · **[Da misurare]**.

Contesto: le ultime segnalazioni "non funziona sempre" avevano tutte la stessa radice — **stato distribuito su tre processi**
(monitor Swift, host Rust, frontend TypeScript) aggiornato da eventi asincroni, senza un modello unico e senza test sulla
parte più delicata (Swift). Le correzioni fatte sono puntuali; questo documento propone di eliminare la classe di errori.

---

## REL-01 · State machine del drag testabile (Impatto A · Sforzo M)

**Problema [Verificato].** `native-bridge/Sources/Monitor.swift` concentra in `MonitorController.shared` (singleton) la logica
di rilevamento del drag: `checkDragState` (≈85 righe) usa 9 variabili di stato (`lastChangeCount`, `pendingChangeCount`,
`pendingEmptyRetries`, `pendingEmptyStart`, `dragActive`, `dragFiles`, `dragWheelTriggeredForDrag`, `dragKeyHeld`,
`dragKeyConsumed`) e legge direttamente `NSPasteboard`, `NSEvent.pressedMouseButtons` e l'orologio di sistema.
Non è testabile senza un desktop reale. I bug recenti (`changeCount` "incollato", `wheelOpen` bloccato, drag ripetuto che non
si riapre) erano esattamente in questa macchina a stati.

**Proposta.**
1. Estrarre `struct DragSession` (valore, senza AppKit) con un solo metodo
   `mutating func step(_ input: DragInput) -> [DragEffect]`, dove `DragInput` contiene `changeCount`, `leftDown`,
   `forceEndIfUp`, `forceRead`, `now`, `filesProvider: () -> [String]`, `keyHeld`; e `DragEffect` è `.dragStart(files)`,
   `.dragEnd`, `.triggerWheel(files)`, `.consumeKey`.
2. `MonitorController` diventa un adattatore sottile (legge pasteboard/clock, scrive JSON).
3. Test con un eseguibile di test compilato da `swiftc` (nessun Xcode necessario): `native-bridge/Tests/DragSessionTests.swift`
   + opzione `native-bridge/build.sh --test`, eseguita in CI.
4. Casi da coprire (tutti già incontrati dal vivo): pasteboard vuoto per 300 ms poi popolato; drag ripetuto con stesso
   `changeCount`; tasto premuto prima/dopo il drag; rilascio con `forceEndIfUp`; nuovo drag mentre `wheel_state:false` è in volo.

## REL-02 · Una sola fonte di verità per "wheel aperta" (Impatto A · Sforzo M)

**Problema [Verificato].** Lo stato "aperta" esiste in tre copie: `wheelOpen` (Swift), `wheel.is_open` (Rust, `host/mod.rs`),
`closing`/`phase` (frontend). Oggi si riallineano con toppe ad hoc (`resync_monitor_wheel_state` in `host/monitor.rs`,
`dragWheelTriggeredForDrag` in Swift, `alreadyHidden` su `wheel:escape`). Ogni nuova scorciatoia (come `R`) aggiunge un
caso in più.

**Proposta.**
- Rust è autorevole. Ogni apertura incrementa `open_epoch`; Swift riceve `wheel_state {open, epoch}` e il frontend
  `wheel:state {…, epoch}`.
- Swift non imposta più `wheelOpen = true` in modo ottimistico: invia `trigger_drag_wheel {requestId}` e attende
  `wheel_state` di conferma (con timeout 300 ms → reset locale).
- I comandi dal frontend che chiudono (`dismiss_wheel`) portano l'`epoch`; Rust ignora quelli con epoch vecchio
  (sostituisce la guardia dei 100 ms contro la doppia chiusura).

## REL-03 · Pipeline del puntatore senza limite di frequenza (Impatto M · Sforzo S)

**Problema [Verificato].** In `Monitor.swift` ogni `leftMouseDragged`/`mouseMoved` con wheel aperta chiama
`emitMouseMoveIfChanged(force: true)`, in più c'è il timer da 16 ms. Ogni evento attraversa: stdout JSON → thread Rust →
`win.emit("wheel:pointer-move")` → IPC → handler in `WheelView.ts`. Con mouse a 500–1000 Hz il frontend riceve più
eventi dei frame disponibili.

**Proposta.** Coalescere in Swift a ≥ 4 ms fra due emissioni (salvo il primo e l'ultimo dopo la pausa) e nel frontend
"ultimo valore vince": il listener salva `latestPointer` e un solo `requestAnimationFrame` lo consuma.
Beneficio **[Da misurare]** con un contatore di eventi/s in modalità debug.

## REL-04 · La fisica avanza a ogni evento, non a ogni frame (Impatto A · Sforzo S)

**Problema [Verificato].** In `WheelView.ts`, `updateCursorProximity` → `stepPhysicsImmediate()` esegue
`springField.step(dt)` per **ogni** evento puntatore, con
`dt = rawDt < 0.002 ? 0.006 : Math.min(0.032, rawDt)`.
Se arrivano eventi più fitti di 2 ms, la simulazione avanza di 6 ms per evento anche se il tempo reale trascorso è minore:
con mouse ad alta frequenza la molla gira **più veloce del tempo reale** (più "nervosa" del previsto) e il costo CPU cresce
con la frequenza degli eventi. La soglia è stata introdotta nell'ultimo intervento per mantenere una risposta sincrona nei
test, quindi è un compromesso noto.

**Proposta.** Gli eventi puntatore impostano **solo** i target (`springField.setPointer`) e fanno `schedulePhysicsLoop()`;
unico punto di step: `physicsTick(now)` in `requestAnimationFrame`, con `dt` reale clampato a 32 ms. I test usano uno
stub di rAF con `flushFrames(n)` (già c'è un wrapper rAF in `views.e2e.test.ts`). Aggiungere un test: 1.000 eventi in 100 ms
simulati non devono far avanzare il tempo della molla più di 100 ms.

## REL-05 · Flag sparsi in `WheelView.ts` → macchina a stati esplicita (Impatto A · Sforzo L)

**Problema [Verificato].** L'arbitraggio fra DOM `pointermove`, stream del monitor e drag nativo Tauri usa ≥ 8 flag e finestre
temporali: `isHoveringControl`, `isDragHovering`, `lastDomPointerMoveAt` (350 ms), `lastMonitorPointerMoveAt` (800 ms),
`lastReleaseHandledAt` (250 ms), `latchedDwellSlot`, `lastDragCoords`, `currentPointerCanvas`. Le soglie sono euristiche e
interagiscono (è stato necessario sbloccare `isHoveringControl` in tre punti).

**Proposta.**
- Normalizzare le sorgenti in un'unica `PointerSample {source: "dom"|"monitor"|"nativeDrag", x, y, t}`.
- Un `InputArbiter` decide la sorgente attiva (priorità: monitor durante drag; DOM altrimenti) e produce un solo flusso.
- Stato esplicito `idle | hovering(slot) | dragging(slot) | committing | closing` con transizioni testate a tabella.
- Diventa naturale dividere il file (vedi PERF-05).

## REL-06 · Tasti in modalità drag: tabella unica (Impatto M · Sforzo M)

**Problema [Inferenza].** In drag mode la finestra non è key (`focusable: false`, voluto per non interrompere il drag di
Finder), quindi il listener DOM `keydown` non riceve nulla. `Esc`, `R` e (dal ramo `feat/0.6-tools`) `T` (cambio faccia Formati/Strumenti, `keyCode 17` → `toggle_face`) funzionano perché
intercettati dall'event tap in `Monitor.swift`; `Tab` (cambio modalità), `1`–`8`, frecce ed `Enter` **probabilmente no** in drag. Oggi ogni tasto richiede
codice in Swift, in `monitor.rs`, in `ipc.ts` e in `WheelView.ts`.

**Proposta.** Tabella unica in Swift `wheelKeybindings: [UInt16: String]` (`53→escape`, `15→toggle_raw`, `48→toggle_mode`,
`18…28→slot:N`…) e un solo evento `wheel:key {name}`; il frontend ha un solo `handleWheelKey(name)` usato sia dal DOM
sia dal monitor. **Verificare dal vivo** quali tasti oggi non funzionano in drag prima di implementare.
Attenzione: l'intercettazione globale consuma il tasto per tutte le app finché la wheel è aperta — mantenere la condizione
`wheelOpen` e nessun modificatore Cmd/Opt/Ctrl.

## REL-07 · Apertura idle: la wheel attende un processo esterno (Impatto A · Sforzo M)

**Problema [Verificato].** `trigger_idle_wheel` (`host/monitor.rs`) chiama `query_finder_context_or_fallback` →
`bridge::finder_context()` → **avvia un nuovo processo** `flip-macos-bridge finder-context` (AppleScript, timeout 10 s) e solo
dopo `present_wheel_at`. Lo stesso `docs/06-RISKS.md` (R4) prescriveva "apertura immediata con placeholder". In modalità
New, `get_new_file_kinds` avvia anche `clipboard-inspect` (altro processo) e `sync_tray_and_windows`.

**Proposta.**
1. Aprire la wheel subito con `mode: "auto"` (anello vuoto/skeleton) e inviare `wheel:state` quando il contesto arriva
   (l'`epoch` di REL-02 evita aggiornamenti su aperture superate).
2. Cache della selezione Finder per 1–2 s e della clipboard keyed su `NSPasteboard.general.changeCount`.
3. Vedi PERF-03 per eliminare lo spawn del processo.
Misurare prima: tempo da hotkey a primo frame **[Da misurare]**.

## REL-08 · Salute del monitor visibile all'utente (Impatto M · Sforzo S)

**Problema [Verificato].** Il supervisore (`start_monitor_supervisor`) riavvia il monitor con backoff e il tap viene riattivato
su `tapDisabledByTimeout`, ma l'utente non vede se il monitor è attivo, senza permessi o in restart loop: l'unico indizio è
`monitor.exited` in `events.jsonl`.

**Proposta.** Stato `monitor: ok | limited(noAccessibility) | restarting(n)` esposto in tray e in Settings → Permessi;
contatore di restart nell'ultima ora nel Diagnostics.

## REL-09 · Drag da app diverse da Finder (Impatto M · Sforzo M)

**Problema [Verificato].** Nessuna gestione di `NSFilePromiseReceiver` (grep: nessuna occorrenza). `readDragFileURLs()` legge
solo URL di file/`NSFilenamesPboardType`/item `fileURL`. Trascinando da Mail, Safari, Foto la wheel non si apre. Era già il
rischio **R8** di `docs/06-RISKS.md`.

**Proposta.** Rilevare `com.apple.pasteboard.promised-file-url`, mostrare la wheel in stato "preparazione", ricevere la
promise in una cartella temporanea di staging e solo dopo convertire (con pulizia in `JobStaging`).

## REL-10 · Timer sempre attivo (Impatto B · Sforzo S)

**Osservazione [Verificato].** Il `Timer` da 16 ms in `setupDragPolling` gira sempre (anche a riposo) per `checkDragState`
O(1) e polling AX ogni 90 tick; in sviluppo `ps` mostrava ≈0,4 % CPU. Basso rischio; opzionale: timer adattivo
(1 Hz a riposo, 60 Hz quando `NSEvent.pressedMouseButtons & 1`).

---

### Checklist di regressione manuale suggerita (da aggiungere a `ACCEPTANCE.md`)

- Drag → `↓` → rilascio su settore, ripetuto 10 volte di fila senza riavviare l'app.
- Drag → `Esc` → nuovo drag immediato → `↓`.
- Drag lento (>400 ms prima di premere `↓`) e drag con `↓` già premuto.
- `R` e `Tab` in drag mode e in idle mode.
- Wheel aperta vicino al Dock/menu bar: il settore evidenziato coincide con il cursore.
