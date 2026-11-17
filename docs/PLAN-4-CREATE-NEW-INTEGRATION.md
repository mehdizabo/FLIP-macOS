# PLAN 4 · Fase 5: New File e integrazione di sistema, step 71–87

## New File (vedi 05-CREATE-NEW-SPEC)

**S71. CreateNewEngine: modello.** `NewFileKind` (id, nome, UTType, icona, generatore, disponibilità) con un registro dei tipi predefiniti e dei template utente.
→ *Test del registro e della disponibilità (es. Pages solo se installato: `NSWorkspace.urlForApplication(withBundleIdentifier:)`).*

**S72. Generatori nativi.** Folder, TXT, MD, RTF, DOCX (NSAttributedString), HTML, JSON, script con `chmod +x`, Python e Swift.
→ *Ogni file generato si apre senza errori nell'app predefinita.*

**S73. Template inclusi nel bundle.** `blank.xlsx`, `blank.pptx`, `blank.pages`, `blank.numbers`, `blank.key` (creati una volta con le app e verificati) in `Resources/Templates`.
→ *Le copie si aprono correttamente; nel bundle nessun metadato personale.*

**S74. From Clipboard.** Lettura di `NSPasteboard.general` per tipo (immagine → PNG, PDF, RTF, testo, URL → `.webloc`), nome con la data, settore disabilitato se gli appunti sono vuoti.
→ *Screenshot copiato → `Clipboard 2026-11-17 at 15.02.png` nella cartella attiva.*

**S75. Template utente.** Cartella in Application Support creata al primo avvio con un README; scansione con `DispatchSource`/FSEvents; sottocartelle come gruppi; segnaposti `{{date}}` ecc. nei file di testo.
→ *Un file aggiunto alla cartella compare nella ruota senza riavvio.*

**S76. Destinazione, naming e rinomina inline.** Risoluzione della cartella (Finder Sync › Finder frontale › Desktop), verifica di scrivibilità, naming "Untitled N", reveal, poi rinomina inline con timing adattivo e verifica che la selezione sia quella del file creato.
→ *In 20 prove consecutive la rinomina parte sul file giusto, o viene saltata senza danni.*

**S77. Ruota New.** Collegamento di `CreateNewEngine` alla ruota (Idle·New), pagina 2 "More", pill Convert/New, icone di sistema; Settings → New File per scegliere e ordinare i tipi visibili.
→ *Hotkey senza selezione → ruota New → file creato e in rinomina in meno di 1 s.*

**S78. New File dalla menu bar.** Sottomenu "New File in Finder Folder ▸" con gli stessi tipi.
→ *Funziona anche senza aprire la ruota.*

## Integrazione con il sistema

**S79. Finder Sync Extension: scaffold.** Target `FlipFinderSync` sandboxed, App Group condiviso, cartelle monitorate (`/` tramite `directoryURLs`), attivazione guidata (Settings → Permissions → "Enable Finder Extension").
→ *L'estensione compare in Impostazioni di Sistema → Estensioni → Finder.*

**S80. Menu contestuale "New File ▸".** Sul fondo della finestra (`.contextualMenuForContainer`): i tipi di New File; la creazione avviene nell'app principale via XPC, perché l'estensione è sandboxed.
→ *Click destro su uno spazio vuoto → New File → Markdown.*

**S81. Menu contestuale "Convert with FLIP ▸".** Sugli elementi selezionati (`.contextualMenuForItems`): formati compatibili (intersezione) e "Open Wheel…".
→ *Click destro su 3 PNG → Convert with FLIP → WebP.*

**S82. App Intents.** `ConvertFilesIntent(files, format)`, `CreateNewFileIntent(kind, folder)` e `OpenWheelIntent`; `AppShortcutsProvider` con frasi in inglese.
→ *Azioni disponibili in Shortcuts e Spotlight; uso da Siri ("Convert with FLIP").*

**S83. Quick Actions del Finder.** Esposizione di Convert tramite Shortcuts/App Intents con `IntentFile` in input, così compare nel pannello Quick Actions e nella Touch Bar/menu Services.
→ *Quick Action "Convert to PDF with FLIP" visibile nel Finder.*

**S84. URL scheme e CLI.** `flip://convert?…` e un piccolo eseguibile `flip` (opzionale, installabile da Settings) per l'uso da Terminale o da Raycast/Alfred.
→ *`flip convert *.heic --to jpg` funziona.*

**S85. Sparkle 2.** Feed appcast su GitHub Releases, chiavi EdDSA, voce "Check for Updates…" (sostituisce `UpdateChecker.cs`), controllo automatico settimanale disattivabile.
→ *Aggiornamento da 0.1.0 a 0.1.1 in test.*

**S86. Privacy e sicurezza.** Nessuna rete tranne Sparkle; `PrivacyInfo.xcprivacy`; audit di `Process` e Apple Events; revisione con `security-review`.
→ *Report di sicurezza senza problemi critici.*

**S87. Robustezza del sistema.** Comportamento dopo sleep/wake, cambio utente rapido, cambio monitor, revoca dei permessi a runtime (degrado a modalità ridotta senza crash).
→ *Checklist manuale completata.*
