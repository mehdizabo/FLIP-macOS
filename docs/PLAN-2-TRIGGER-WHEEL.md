# PLAN 2 · Fase 2 (input globale) + Fase 3 (ruota), step 23–50

## Fase 2: trigger e input globale (`FlipInput`)

**S23. DragSessionMonitor.** Servizio `@MainActor` che espone `isDragging`, `draggedFileURLs` e `cursorLocation` (Combine/AsyncStream), basato sullo spike S03. Ignora i drag che non contengono file.
→ *Unit test con pasteboard finto; prova manuale dal Finder e dal Desktop.*

**S24. TriggerKeyTap.** Implementazione del meccanismo scelto in S04: consuma key-down, repeat e key-up del tasto trigger **solo** se `isDragging`. Fuori dal drag passa tutto. Si riattiva da solo se il sistema disattiva il tap (`tapDisabledByTimeout`).
→ *Nessun tasto "bloccato"; digitazione normale non influenzata (test manuale in TextEdit).*

**S25. Porting di ShortcutPressGate e ShortcutOrderGate.** Logica pura in `FlipCore` con gli stessi test di `InputTests.cs` (entrambi gli ordini, repeat, key-up orfano).
→ *Test portati e verdi.*

**S26. Modello delle scorciatoie.** `WheelShortcut`: tasto singolo per il drag (default `↓`, preset `↓ → Fn F1…F12 ⌥`) e hotkey globale Idle con `KeyboardShortcuts` (default `⌥⇧Space`). UI recorder in Settings → Shortcuts con rilevamento dei conflitti.
→ *Cambio della scorciatoia senza riavvio.*

**S27. File promise e sorgenti non-Finder.** Supporto a `NSFilePromiseReceiver` (Foto, Mail, Safari): ricezione in una cartella di staging, poi conversione con output in una cartella scelta (default Downloads), dato che non esiste un "accanto all'originale".
→ *Drag di una foto da Foto → ruota → JPG nei Downloads.*

**S28. Validazione input (porting di `Policy.ReadFiles`).** Al massimo 128 file, solo file regolari locali; esclusi cartelle, symlink/alias, volumi di rete (`URLResourceValues.volumeIsLocal`) e file iCloud non scaricati (`ubiquitousItemDownloadingStatus`).
→ *Test su ogni caso.*

**S29. FinderBridge.** API async `selectedItems()`, `insertionFolder()`, `reveal(_:)` e `beginRename(_:)` con timeout di 500 ms e cache per 1 s; errori tipizzati (permesso negato, Finder non in esecuzione).
→ *Test manuali + mock per i test unitari.*

**S30. Orchestratore dei trigger.** `TriggerCoordinator` decide la modalità (Drag, Idle·Convert o Idle·New) secondo `02-ARCHITECTURE`, con debounce per evitare doppie aperture.
→ *State machine testata con eventi simulati.*

**S31. Trigger da mouse (opzionale).** Tasto mouse 3/4/5 tramite event tap, configurabile, con modalità "hold and release" come Logi.
→ *Funziona con un mouse generico; disattivato di default.*

**S32. Modalità ridotta senza Accessibility.** Solo hotkey Idle (Carbon, nessun permesso) più la ruota come drop target fisso: richiamata con la hotkey, ci trascini sopra i file.
→ *L'app resta utile senza permessi; banner "Enable full mode".*

**S33. Gestione multi-monitor e Spaces.** Posizione del cursore → `NSScreen` corretto, clamp ai bordi del `visibleFrame`, comparsa sullo Space attivo e sopra le app a schermo intero (`.fullScreenAuxiliary`).
→ *Test su 2 monitor con DPI diversi e in full screen.*

**S34. Escape e cancellazione globali.** Esc chiude la ruota oppure annulla la conversione; mouse up fuori dalla ruota la chiude senza azione; perdita del drag (il drag finisce altrove) idem.
→ *Nessuna ruota "orfana" rimasta a schermo (test di stress con 50 aperture).*

## Fase 3: ruota radiale (`FlipWheel`)

**S35. RingGeometry.** Calcolo dei settori (n = 1…8, gap, raggi), `sector(at: point)` per angolo, zona morta centrale e raggio massimo; condivisa da disegno e hit test.
→ *Test parametrici su angoli e bordi (porting dei test di `SectorData`).*

**S36. RingSectorShape.** `Shape` SwiftUI con archi e angoli raccordati, `animatableData` per scala ed espansione.
→ *Snapshot test per n = 2…8.*

**S37. WheelPanel.** `NSPanel` borderless, trasparente, `.nonactivatingPanel`, livello `.popUpMenu`, `hidesOnDeactivate = false`, `NSHostingView` come contenuto; pre-creato all'avvio e nascosto per aprirsi in meno di 16 ms.
→ *Apertura misurata < 1 frame dopo il trigger.*

**S38. WheelViewModel.** Stato: modalità, file, settori (formato, icona, abilitato), selezione, pagina, progresso, esito. Input dal pannello (drag/mouse/tastiera), output verso il motore.
→ *Test del view model senza UI.*

**S39. WheelView: layout statico.** Anello, settori con SF Symbol ed etichetta, centro con titolo e sottotitolo, pill Convert/New.
→ *Rispetta la spec in 03 (dimensioni, font, Light/Dark).*

**S40. Stile Liquid Glass + fallback.** `glassEffect` e `GlassEffectContainer` su 26+; `NSVisualEffectView` mascherata su 15; supporto a Reduce Transparency e Increase Contrast.
→ *Screenshot comparativi in 4 combinazioni.*

**S41. Hover e selezione per angolo.** Tracking del mouse (`NSTrackingArea` + monitor durante il drag), selezione con isteresi di 3° per evitare lo sfarfallio, evidenziazione animata.
→ *Nessun flicker al confine tra settori.*

**S42. Drop sui settori.** `NSDraggingDestination` sul pannello: `draggingUpdated` → selezione, `performDragOperation` → formato scelto; restituisce `.copy`, oppure `[]` se il settore è disabilitato.
→ *L'originale non viene mai spostato (test con dischi diversi e con lo stesso disco).*

**S43. Conferma in modalità Idle.** Click, oppure release della hotkey sopra un settore (hold-and-release), con soglia: il release subito dopo l'apertura senza movimento lascia la ruota aperta.
→ *Entrambi i gesti funzionano in modo naturale.*

**S44. Navigazione da tastiera.** Frecce, 1–8, Return, Tab ed Esc; focus visibile; la ruota diventa key window solo in modalità Idle.
→ *Uso completo senza mouse.*

**S45. Paginazione "More".** Con più di 8 formati: settore More, dwell di 300 ms durante il drag o click, rotazione animata alla pagina 2 e ritorno.
→ *Funziona per PDF con Pandoc (12 uscite).*

**S46. Centro con anteprima QuickLook.** `QLThumbnailGenerator` asincrono (64 pt @2x), pila per più file, nome troncato al centro, testo dinamico "Convert to PNG" o messaggi di incompatibilità (porting di `AvailabilityMessage`, in inglese).
→ *Anteprima visibile entro 150 ms o placeholder elegante.*

**S47. Feedback sonoro e aptico.** Porting di `flip-tick.wav` (precaricato, latenza bassa, nessuna coda), haptic `.alignment` a ogni cambio settore; toggle in Settings.
→ *Il tick non si accumula con movimenti rapidi.*

**S48. Animazioni.** Apertura e chiusura a molla, cascata dei settori, pulsazione al drop, contrazione; rispetto di Reduce Motion (solo fade).
→ *Revisione visiva frame per frame in un video a 120 fps.*

**S49. Arco di progresso ed esiti.** Porting di `ProgressArcData`/`StartProgressVisual`: arco esterno, percentuale, sweep indeterminato se la percentuale non è nota, ✓ con trim, shake in caso di errore con il pulsante "Details".
→ *Simulatore di progresso nei preview SwiftUI.*

**S50. VoiceOver e accessibilità della ruota.** Elementi accessibili per i settori, annunci di apertura, selezione ed esito; test con VoiceOver attivo.
→ *Flusso Idle completo usabile con VoiceOver.*
