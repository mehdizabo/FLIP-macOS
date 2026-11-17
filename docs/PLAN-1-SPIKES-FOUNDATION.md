# PLAN 1 · Fase 0 (spike e decisioni) + Fase 1 (fondamenta), step 01–22

Formato di ogni step: **cosa** → *criterio di completamento*.

## Fase 0: decisioni e spike tecnici (prima di scrivere la UI vera)

**S01. Congelare il perimetro MVP.** Rileggere 00–05, rispondere alle domande aperte di `00-INDEX.md` e fissare il macOS minimo, i trigger e i formati del MVP.
→ *Documento `DECISIONS.md` con le scelte firmate.*

**S02. Inventario della logica da portare.** Estrarre da `Policy.cs`, `ConversionEngine.cs`, `MediaTools.cs`, `DocumentTools.cs`, `ConversionTests.cs` e `InputTests.cs` le regole (estensioni, intersezioni, naming, limiti, validazioni) e i casi di test.
→ *`PORTING-NOTES.md` con una tabella regola → file C# → modulo Swift.*

**S03. Spike A: rilevamento del drag.** Mini app CLI/AppKit che osserva `NSPasteboard(name: .drag).changeCount` più `NSEvent` global monitor (`.leftMouseDragged`, `.leftMouseUp`) e stampa gli URL trascinati dal Finder, dal Desktop e da Foto/Mail (promise).
→ *Log affidabile di inizio e fine drag e dei file, latenza < 50 ms.*

**S04. Spike B: tasto durante il drag.** Nella stessa app, provare (a) Carbon `RegisterEventHotKey` registrato solo durante il drag e (b) `CGEventTap` attivo che consuma `↓` solo se il drag è attivo. Testare entrambi gli ordini: drag→tasto e tasto→drag.
→ *Tabella risultati; scelta del meccanismo primario e del fallback.*

**S05. Spike C: pannello come drop target a drag già iniziato.** Mostrare un `NSPanel` non attivante sotto il cursore a metà drag; verificare `draggingEntered/Updated/performDragOperation`, l'operazione `.copy` (il Finder non deve spostare l'originale) e il rilascio fuori dalla ruota.
→ *Video della prova + note sul livello della finestra e su `collectionBehavior`.*

**S06. Spike D: ruota Liquid Glass.** Prototipo SwiftUI di 8 settori con `glassEffect` su una forma custom, hover per angolo e animazioni a molla; misurare la frequenza dei frame con Instruments; fallback per macOS 15.
→ *60/120 fps stabili, screenshot in Light e Dark.*

**S07. Spike E: bridge Finder.** Leggere la selezione e l'`insertion location` via `NSAppleScript`/ScriptingBridge, misurare la latenza e verificare il prompt Automation.
→ *Latenza misurata, testo di `NSAppleEventsUsageDescription` definito.*

**S08. Spike F: conversioni documento native.** DOCX → PDF, DOCX → ODT, HTML → DOCX e MD → PDF con NSAttributedString/WebKit su 5 file reali di FLIP Windows.
→ *Report sulla fedeltà (stili, tabelle, immagini) con la decisione su dove Pandoc resta necessario.*

**S09. Spike G: audio nativo.** Verificare la scrittura FLAC con `ExtAudioFile`, ALAC/AAC con `AVAssetWriter` e i tempi di export MP4→MOV/M4A.
→ *Matrice audio confermata (aggiornare `04-CONVERSION-MATRIX.md`).*

**S10. Go/No-go.** Riesame degli spike. Se R1 o R2 falliscono, valutare il piano B: trigger con modificatore (es. tenere premuto `⌃` durante il drag) o una "zona calda" sul bordo dello schermo.
→ *Decisione scritta; il piano prosegue.*

## Fase 1: fondamenta dell'app

**S11. Repository e progetto.** Nuovo repo `FLIP-mac`, progetto Xcode, target `FLIP` (macOS, SwiftUI lifecycle, `LSUIElement`), pacchetti locali `FlipCore`, `FlipEngines`, `FlipInput`, `FlipWheel` (vedi 02).
→ *Build pulita, `swift test` verde sul pacchetto vuoto.*

**S12. Tooling.** SwiftLint, swift-format, `.editorconfig`, Swift 6 strict concurrency, warning trattati come errori in CI.
→ *`make lint` e `make test` funzionanti.*

**S13. CI GitHub Actions.** Runner `macos-26`: build, test e lint a ogni PR; artifact dell'app (non firmata) per ogni branch.
→ *Badge verde.*

**S14. Menu bar app.** `MenuBarExtra` con icona template e le voci di `03-UX-WHEEL-SPEC §7`, con placeholder per le azioni.
→ *L'app parte, non compare nel Dock, il menu funziona e Quit chiude.*

**S15. Preferences model.** `Preferences` (`@Observable` + `UserDefaults`, chiavi versionate, migrazioni), porting di `AppPreferences.cs`.
→ *Test di persistenza e migrazione.*

**S16. Logging.** `os.Logger` per sottosistema + `EventLog` JSONL in `~/Library/Logs/FLIP/events.jsonl` con rotazione a 2 MiB (porting di `Evidence`). Voce di menu "Reveal Logs".
→ *Test di rotazione; nessun percorso loggato a livello `.public` senza bisogno.*

**S17. Settings window.** Scena `Settings` con le schede General, Shortcuts, Formats, New File, Engines, Permissions e About, per ora vuote.
→ *Navigazione completa; `⌘,` apre la finestra.*

**S18. Login item.** Toggle "Open at Login" con `SMAppService.mainApp`, che riflette lo stato reale, anche se cambiato dalle Impostazioni di Sistema.
→ *Prova manuale con logout/login.*

**S19. PermissionCenter.** Stato di Accessibility (`AXIsProcessTrustedWithOptions`), Automation Finder (probe innocuo con `AEDeterminePermissionToAutomateTarget`) e Input Monitoring; notifica dei cambi di stato con polling leggero solo mentre Settings è aperto.
→ *La scheda Permissions mostra lo stato vero e ha i pulsanti "Open System Settings".*

**S20. Onboarding.** Prima esecuzione con 3 schermate in inglese (cos'è FLIP, come si usa con una GIF del gesto, permessi uno alla volta con il motivo). Possibilità di saltare con la modalità ridotta.
→ *Copy rivisto, schermate con stile Liquid Glass coerente.*

**S21. Localizzazione predisposta.** Tutte le stringhe in String Catalog (`Localizable.xcstrings`); per ora solo inglese, con l'italiano possibile in seguito.
→ *Nessuna stringa hardcoded (verifica con lint).*

**S22. App icon e identità.** Icona macOS 26 (Icon Composer, layer per Liquid Glass) derivata da `Flip.ico`/`flip-cover.png`, simbolo della menu bar, About.
→ *Asset catalog completo in Light, Dark e Tinted.*
