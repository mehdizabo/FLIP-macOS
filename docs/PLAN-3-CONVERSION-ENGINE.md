# PLAN 3 · Fase 4: motore di conversione, step 51–70

> **Documento storico di pianificazione.** Descrive il piano iniziale del porting (Swift/SwiftUI nativo) e non l'implementazione attuale. L'architettura vigente (Tauri v2 + Rust + bridge Swift) è in [`../ARCHITECTURE.md`](../ARCHITECTURE.md) e [`DECISIONS.md`](DECISIONS.md).

Obiettivo: parità funzionale con FLIP Windows 0.5.1 (tabella del README) più le aggiunte native di `04-CONVERSION-MATRIX.md`.

## Nucleo (`FlipCore`)

**S51. FormatPolicy.** Porting di `Policy.Outputs` su `UTType`: famiglie, uscite per famiglia, esclusione del formato sorgente, intersezione multi-file e dipendenza dai motori disponibili (FFmpeg/Pandoc/libwebp).
→ *Tutti i casi di `ConversionTests.cs` portati come test parametrici.*

**S52. Ordine dei formati.** Porting di `OrderFormats` con gli ordini per famiglia configurabili (Settings → Formats, lista riordinabile con drag).
→ *Test di ordinamento e persistenza.*

**S53. OutputNaming.** `name.converted.ext`, collisioni in stile Finder (`name.converted 2.ext`), suffisso `-p01` per le pagine, gestione di nomi lunghi (255 byte UTF-8) e di Unicode NFD/NFC.
→ *Test con nomi esistenti, emoji e accenti.*

**S54. JobStaging e commit atomico.** Porting di `ConversionEngine`: cartella `.flip-job-UUID` nascosta accanto al sorgente, copia di staging del sorgente (APFS clonefile, istantaneo), scrittura nello staging, validazione, commit con `link`/`renamex_np(RENAME_EXCL)`, così l'overwrite è impossibile. Rollback multi-pagina e pulizia in `defer`.
→ *Test: crash simulato a metà = nessun file parziale visibile.*

**S55. Coda e cancellazione.** `actor ConversionQueue`: job serializzati, `Task` cancellabile, progresso via `AsyncStream`, timeout per famiglia.
→ *Esc durante un video da 2 GB annulla e pulisce entro 1 s.*

**S56. Validazione dell'output.** Riapertura con il motore nativo (`CGImageSource`, `PDFDocument`, `AVURLAsset.load(.isPlayable)`, parse JSON/XML), dimensione > 0, e limiti zip per DOCX/ODT/EPUB (porting di `ValidateContainer`).
→ *File corrotti rifiutati prima del commit.*

## Motori (`FlipEngines`)

**S57. ImageEngine.** ImageIO: lettura di tutti i formati supportati; scrittura PNG/JPG/TIFF/GIF/BMP/HEIC con qualità, orientamento EXIF applicato e profilo colore; GIF animata → prima immagine (o tutte le pagine verso TIFF).
→ *Golden test con immagini campione (incluse quelle di FLIP Windows).*

**S58. WebP con libwebp.** Pacchetto SPM con libwebp (BSD); encoder lossy/lossless; lettura comunque nativa.
→ *PNG → WebP → PNG con differenza percettiva entro la soglia.*

**S59. PDFEngine.** Pagina → immagine a 150/300 dpi (impostabile), testo estratto (`PDFPage.string`), immagini → PDF, unione di più PDF/immagini in un unico PDF, OCR con Vision (`VNRecognizeTextRequest`) sulle pagine senza testo.
→ *PDF multi-pagina di prova → PNG numerati; PDF scansionato → TXT.*

**S60. DocumentEngine nativo.** `NSAttributedString` per TXT, RTF, RTFD, HTML, DOC, DOCX e ODT in lettura e scrittura; PDF impaginato in A4/Letter con margini (Core Text `CTFramesetter` oppure `WKWebView.pdf`); TXT/MD → PNG/JPG della prima pagina.
→ *DOCX → PDF in A4 di default (come FLIP 0.5.1); fedeltà verificata sui file dello spike S08.*

**S61. Markdown.** Input: swift-markdown → HTML (con CSS sobrio) → NSAttributedString/PDF. Output: convertitore AttributedString/HTML → Markdown di base (titoli, enfasi, liste, link, codice, tabelle semplici).
→ *Round trip MD → DOCX → MD senza perdita delle strutture base.*

**S62. DataEngine.** JSON (`JSONSerialization`, ordine delle chiavi conservato con un parser custom), YAML (Yams), XML (`XMLDocument`), CSV (TabularData + RFC 4180), export verso TXT, HTML, MD e PDF. Porting delle regole e dei test di `MediaTools.cs`.
→ *Tutte le coppie della matrice dati verdi.*

**S63. ExternalTools (FFmpeg/Pandoc).** Rilevamento in `~/Library/Application Support/FLIP/tools`, `/opt/homebrew/bin`, `/usr/local/bin` e PATH di login; verifica della versione; esecuzione con `Process` ad argomenti strutturati, timeout, cancellazione (`terminate` → `kill`) e ambiente pulito. Scheda Engines con stato e pulsante "Locate…".
→ *Nessun motore incluso nel bundle; settori abilitati e disabilitati di conseguenza.*

**S64. MediaEngine: video nativo.** `AVAssetExportSession` (iOS/macOS 15: `export(to:as:)` async con progresso): MP4 H.264/HEVC, MOV, M4V; preset di qualità; estrazione audio in M4A/WAV.
→ *MOV 4K → MP4 con VideoToolbox, progresso fluido.*

**S65. MediaEngine: audio nativo.** `AVAssetReader/Writer` per WAV, AIFF, M4A/AAC e ALAC; FLAC con ExtAudioFile se lo spike S09 è positivo.
→ *Matrice audio nativa verde.*

**S66. MediaEngine: FFmpeg.** Porting degli argomenti di `MediaTools.Convert`: MP3, OGG, Opus, MKV, WebM e AVI; parsing del progresso da `-progress pipe:1` con la durata da `ffprobe` (o `AVURLAsset`).
→ *Parità con FLIP Windows per audio/video; percentuale reale.*

**S67. Video → GIF.** `AVAssetImageGenerator` a 10–15 fps, ridimensionamento a 480 px, `CGImageDestination` GIF con loop; limite di durata con avviso.
→ *Clip di 10 s → GIF < 8 MB.*

**S68. DocumentEngine con Pandoc.** EPUB, FB2 e RST in input e output; porting del filtro FB2 per i paragrafi (vedi `ARCHITECTURE.md` Windows) e dei limiti di sicurezza.
→ *Casi Pandoc dei test Windows verdi (con Pandoc installato).*

**S69. Batch / multi-output.** Porting di `SelectionWindow`: da "⌥-click" sul settore, oppure dalla voce "Convert to multiple formats…", una finestra con checkbox dei formati e progresso per file; blocco della modifica mentre il batch è attivo.
→ *10 file × 3 formati con progresso aggregato e annullabile.*

**S70. Notifiche e storico.** Notifica locale opzionale a fine conversione lunga (> 5 s) con l'azione "Show in Finder"; voce "Recent conversions" nella menu bar (ultimi 10).
→ *Click sulla notifica rivela il file.*
