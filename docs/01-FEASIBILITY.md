# 01 · Analisi di fattibilità

## 1. Mappa componenti Windows → macOS

| Componente FLIP (Windows) | File C# | Equivalente macOS | Fattibilità |
|---|---|---|---|
| Host nascosto + tray icon | `Program.cs` | `NSStatusItem` / SwiftUI `MenuBarExtra`, `LSUIElement = YES` | ✅ Nativo |
| Single instance + IPC | `Program.cs` | Unicità garantita da LaunchServices; IPC con `NSXPCConnection` o `DistributedNotificationCenter` | ✅ Nativo |
| Hook tastiera low-level | `ShortcutHook.cs` | `CGEventTap` (consuma il tasto) **oppure** Carbon `RegisterEventHotKey`, registrato solo durante il drag | ⚠️ Spike (S03–S04) |
| Raw Input mouse/Escape | `Native.cs` | `NSEvent.addGlobalMonitorForEvents` (`.leftMouseDragged`, `.leftMouseUp`) | ✅ Nativo |
| Rilevazione drag da Explorer | `ShortcutHook.cs` | `NSPasteboard(name: .drag).changeCount` + mouse premuto in movimento | ✅ Nativo, con un **vantaggio**: i file si leggono *prima* del drop |
| Finestra ruota (WPF trasparente) | `Wheel.cs` | `NSPanel` borderless, `.nonactivatingPanel`, livello `.popUpMenu`, contenuto SwiftUI | ✅ Nativo |
| Drop sui settori | `Wheel.cs` (OLE) | `NSDraggingDestination` sul pannello, hit test per settore in `draggingUpdated` | ✅ Nativo |
| Posizionamento multi-monitor/DPI | `Native.cs` | `NSScreen` + coordinate in punti (Retina gestita dal sistema) | ✅ Più semplice |
| Tick sonoro | `WheelSound.cs` | `AVAudioPlayer` precaricato o `AudioServicesPlaySystemSound` | ✅ |
| Haptic | — | `NSHapticFeedbackManager` (trackpad Force Touch) | ✅ **Nuovo** |
| Preferenze atomiche | `AppPreferences.cs` | `UserDefaults` / `@AppStorage` | ✅ |
| Avvio al login | `StartupIntegration.cs` | `SMAppService.mainApp.register()` | ✅ |
| Shortcut settings | `ShortcutSettingsWindow.cs` | SwiftUI `Settings` + recorder (pacchetto `KeyboardShortcuts`) | ✅ |
| Batch / multi-output | `SelectionWindow.cs` | Finestra SwiftUI o popover dalla ruota | ✅ |
| Policy formati | `Policy.cs` | `FormatPolicy.swift`, porting 1:1 su `UTType` | ✅ |
| Motore jobs (staging, no-overwrite) | `ConversionEngine.cs` | `actor ConversionEngine`, `FileManager`, `replaceItemAt`/link atomico | ✅ |
| Immagini (SkiaSharp) | `ConversionEngine.cs` | **ImageIO** (PNG, JPEG, TIFF, GIF, BMP, HEIC); WebP in lettura nativo, in scrittura con **libwebp** | ✅ / ⚠️ WebP |
| PDF (PDFium, PdfPig) | `ConversionEngine.cs` | **PDFKit** (render pagina → immagine, estrazione testo, immagini → PDF) | ✅ Nativo |
| Documenti (Pandoc/LibreOffice) | `DocumentTools.cs` | **NSAttributedString** legge e scrive TXT, RTF, RTFD, HTML, DOC, **DOCX**, **ODT**; PDF via Core Text o WebKit; Markdown con `swift-markdown`; EPUB, FB2 e RST con Pandoc opzionale | ✅ (gran parte nativa) |
| Audio/Video (FFmpeg) | `MediaTools.cs` | **AVFoundation**: MP4, MOV, M4V, M4A, AAC, WAV, AIFF, ALAC, CAF; FLAC da verificare; MP3, OGG, MKV, WebM e AVI con FFmpeg opzionale | ✅ / ⚠️ |
| Dati (JSON/YAML/XML/CSV) | `MediaTools.cs` | `JSONSerialization`, `XMLDocument`, **TabularData** (CSV), `Yams` (YAML) | ✅ |
| Update checker GitHub | `UpdateChecker.cs` | **Sparkle 2** (appcast firmato EdDSA) | ✅ Migliore |
| Log diagnostici | `Policy.cs` (Evidence) | `os.Logger` + file JSONL ruotato in `~/Library/Logs/FLIP` | ✅ |
| "Crea nuovo" | — (non esiste) | Generatori + template + Finder `insertion location` (AppleScript) | ✅ **Nuovo** |

## 2. Cosa diventa *migliore* su Mac

1. **Formati compatibili già all'apertura in modalità drag.** Su Windows la ruota non può sapere i formati prima del `DragEnter` OLE (vedi il commento in `Wheel.cs`). Su Mac basta leggere `NSPasteboard(name: .drag)`: la ruota mostra subito solo i formati validi.
2. **Molte meno dipendenze esterne.** DOCX, ODT, RTF, HTML e PDF diventano nativi, senza LibreOffice, e i video MP4/MOV usano l'hardware encoder (VideoToolbox).
3. **Anteprime.** `QLThumbnailGenerator` mostra la miniatura del file al centro della ruota.
4. **Nuove conversioni gratuite**: HEIC/AVIF, Video → GIF, unione di più immagini o PDF in un unico PDF, Live Text (OCR con il framework Vision) da immagine/PDF a TXT.
5. **Integrazione di sistema**: Shortcuts/App Intents, Quick Actions del Finder, menu contestuale tramite Finder Sync Extension.

## 3. Cosa *non* è possibile, o costa

| Limite | Impatto | Soluzione |
|---|---|---|
| Sandbox App Store vieta event tap attivi e scrittura arbitraria | Niente App Store | Developer ID + notarizzazione |
| Permesso **Accessibility** (o Input Monitoring) | Attrito all'onboarding | Onboarding guidato; senza permesso resta la modalità ridotta con hotkey Carbon |
| Permesso **Automation (Finder)** per selezione/cartella attiva | Prompt di sistema | Spiegazione nell'onboarding; fallback sul Desktop |
| WebP in scrittura | Una dipendenza C | `libwebp` tramite SPM (licenza BSD) |
| MP3, OGG, Opus, MKV, WebM, AVI, WMA | Senza FFmpeg i settori sono disabilitati | FFmpeg rilevato (`/opt/homebrew/bin`, `~/Library/Application Support/FLIP/tools`), come su Windows |
| EPUB, FB2, RST | Senza Pandoc i settori sono disabilitati | Pandoc opzionale (EPUB in uscita nativo in fase successiva) |
| Pages, Numbers, Keynote | Formato chiuso | Export tramite le app Apple via AppleScript, solo se installate (opzionale) |

## 4. Stima di massima

| Fase | Impegno (1 sviluppatore esperto Swift) |
|---|---|
| F0 Spike + F1 Fondamenta | 1,5–2 settimane |
| F2 Trigger + F3 Ruota | 3 settimane |
| F4 Motore conversioni | 3 settimane |
| F5 New File + integrazione | 1,5–2 settimane |
| F6 Qualità e rilascio | 1,5 settimane |
| **Totale MVP completo** | **~10–12 settimane** (con agenti AI può scendere molto, ma i test fisici restano manuali) |

## 5. Conclusione

Il porting è **fattibile**. L'unico rischio tecnico che va verificato prima di tutto il resto è **ricevere il tasto mentre il Finder gestisce un drag** (S03–S05): tutto il resto usa API Apple documentate e stabili.
