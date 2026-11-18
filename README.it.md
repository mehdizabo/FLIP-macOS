# FLIP per macOS

**Convertitore radiale di file e utility Create New File in stile Liquid Glass, 100% locale per macOS Finder.**

[English (README.md)](README.md)

FLIP porta una ruota radiale veloce centrata sul cursore (ispirata all'Actions Ring di Logi Options+) direttamente su Finder e Scrivania di macOS. Converte immagini, PDF, documenti, dati strutturati, audio, video e file Apple iWork **senza mai sovrascrivere gli originali né caricare nulla nel cloud** — e crea nuovi file o template personalizzati direttamente nella cartella attiva del Finder con rinomina inline immediata.

---

## Funzionalità principali

- **Ruota Radiale Liquid Glass**: settori in vetro traslucido stratificato, riflessi speculari direzionali, animazioni a molla, anteprima QuickLook nell'hub centrale, selettore modalità `Convert` / `New File` (`Tab`), arco radiale di avanzamento con percentuale, **annunci VoiceOver live**, **selettore lingua English / Italiano (`EN` / `IT`)**, feedback acustico (`flip-tick.wav`), aptico sul trackpad e pulsante **Prova suono e feedback aptico ruota**.
- **App Menu Bar (`LSUIElement`) & IPC Single-Instance**: vive nella barra dei menu senza ingombrare il Dock (`Open FLIP Wheel`, `New File in Finder Folder…`, `Batch / Multiple Formats…`, `Settings & Shortcuts…`, `Open Templates Folder`, `Reveal Diagnostic Logs`, `Quit FLIP`). Garantisce istanza singola tramite `flock` e Unix domain socket locale (`flip-instance.sock`, permessi `0600`) per gestire Azioni Rapide, URL `flip://` e comandi di controllo CLI.
- **Convertitore Batch & Multi-Formato**: converte più file verso uno o più formati contemporaneamente (`Shift+Click` su un settore, pulsante `Multi` al centro o dal menu nella menu bar).
- **Naming senza sovrascrittura**:
  - **Conversioni**: ereditano direttamente il nome base del file originale (`photo.heic` → `photo.png`). In caso di collisione aggiunge il contatore stile Finder (`photo 2.png`, `photo 3.png`). Le esportazioni multipagina usano `<nome>-p001.<ext>`. Non attiva **mai** la rinomina inline sulle conversioni.
  - **Staging atomico e protezioni**: ogni job lavora in `.flip-job-<uuid>`, valida gli output prima del commit atomico (`renamex_np(RENAME_EXCL)`), rifiuta symlink, alias del Finder, file iCloud non scaricati localmente e volumi di rete, limita i batch a 128 file, blocca immagini > 40 MP, ottimizzazioni PDF oltre 200 pagine e archivi ZIP > 256 MiB / 4.096 entry / Zip-Slip, e sanitizza HTML offline.

---

## Trigger disponibili

| Modalità | Trigger predefinito | Funzionamento |
|---|---|---|
| **Drag + Tastiera** | `Freccia Giù` (configurabile) | Trascina uno o più file dal Finder/Scrivania e premi `Down` (oppure tieni premuto `Down` prima di trascinare). Rilascia su un settore compatibile. Legge `NSPasteboard(name: .drag)` in tempo reale prima del drop. |
| **Idle + Tastiera** | `Option+Shift+Space` (configurabile) | Apre la ruota sotto il cursore ovunque. Se ci sono file selezionati nel Finder si apre in modalità **Convert**; altrimenti in **New File** sulla cartella attiva del Finder (fallback `~/Desktop`). Premi `Tab` per cambiare modalità. |
| **Tasto Mouse (Opzionale)** | Disabilitato (opzionale `Btn 2` / `3` / `4`) | Supporto tieni-premuto-e-rilascia per tasto centrale o laterali (mouse Logi / multi-tasto). |

La ruota supporta anche frecce direzionali, tasti numerici `1`–`8`, `Invio`/`Spazio`, `Tab`, `Esc` (annulla o chiude) e dwell di 300 ms su `More ▸` / `Back ◂` durante il trascinamento quando ci sono più di 7 formati.

---

## Create New File

FLIP aggiunge la creazione immediata di nuovi file nella cartella del Finder in primo piano (con fallback su `~/Desktop` o cartella personalizzata nelle Impostazioni), seleziona il file nel Finder e **avvia subito la rinomina inline (`Invio`)**:

- **Tipi predefiniti**: Cartella (`untitled folder`), Plain Text (`.txt`), Markdown (`.md`), Rich Text (`.rtf`), Word (`.docx`), Foglio di calcolo (`.xlsx`), Presentazione (`.pptx`), HTML (`.html`), JSON (`.json`), CSV (`.csv`), Shell Script eseguibile (`.sh`, `chmod +x`), Python (`.py`), Swift (`.swift`).
- **From Clipboard**: ispeziona `NSPasteboard.general` e salva immagini (`.png`), PDF, RTF, segnalibri web (`.webloc`) o testo (`.txt`) come `Clipboard <timestamp>.<ext>`.
- **Apple iWork**: crea nuovi `.pages`, `.numbers` e `.key` se Pages, Numbers o Keynote sono installati.
- **Template utente**: inserisci qualsiasi file in `~/Library/Application Support/FLIP/Templates` (le sottocartelle diventano gruppi). I template testuali espandono automaticamente `{{date}}`, `{{time}}`, `{{user}}` e `{{folder}}`.

---

## Conversioni supportate e integrazione Apple

| Famiglia | Input | Output | Motori |
|---|---|---|---|
| **Immagini** | `PNG`, `JPG/JPEG`, `WEBP`, `HEIC/HEIF`, `AVIF`, `TIFF`, `GIF`, `BMP`, `ICNS`, `ICO` | `PNG`, `JPG`, `WEBP`, `HEIC`, `PDF`, `TIFF`, `GIF`, `BMP`, `REMOVE_BG`, `ZIP`, più `Combine into PDF` | `ImageIO` nativo (EXIF & rimozione GPS opzionale) + `Vision` (`VNGenerateForegroundInstanceMaskRequest` `REMOVE_BG`) + `image` Rust + `PDFKit` |
| **PDF** | `PDF` | `PNG`, `JPG`, `WEBP`, `HEIC`, `TIFF` (144/200/300 DPI), `TXT` (con **Apple Vision** OCR per PDF scansionati), `DOCX`, `MD`, `HTML`, `RTF`, `ODT`, `EPUB` (EPUB 3 nativo), `OPTIMIZE_PDF` (compressione PDF preservando testo selezionabile), `ZIP` (+ `FB2`/`RST` con Pandoc), `Merge PDFs` | `PDFKit` / Quartz nativo + `Vision` (`VNRecognizeTextRequest`) + writer EPUB 3 in Rust |
| **Documenti** | `TXT`, `MD`, `HTML`, `RTF`, `DOC`, `DOCX`, `ODT`, `EPUB` (+ `FB2`, `RST` con Pandoc) | `PDF` (A4/Letter impaginato), `DOCX`, `MD`, `HTML`, `TXT`, `RTF`, `ODT`, `EPUB` (EPUB 3 nativo), `PNG`, `JPG`, `WEBP`, `HEIC`, `ZIP` (+ `FB2`/`RST` con Pandoc) | `NSAttributedString` / CoreText nativo + `pulldown-cmark` + EPUB 3 nativo + `Pandoc` opzionale |
| **Apple iWork** | `.pages`, `.numbers`, `.key` | `PDF`, `ZIP` (+ `DOCX`/`RTF`/`TXT`/`EPUB` per Pages, `XLSX`/`CSV` per Numbers, `PPTX` per Keynote) | Anteprima QuickLook + AppleScript verso app Apple |
| **Dati strutturati** | `JSON`, `YAML/YML`, `TOML`, `XML`, `CSV` | `JSON`, `YAML`, `TOML`, `XML`, `CSV` (appiattimento chiavi con punto), `TXT`, `MD`, `HTML`, `PDF`, `ZIP` | Parser Rust nativi + CoreText PDF |
| **Audio** | `MP3`, `WAV`, `M4A`, `AAC`, `AIFF`, `ALAC`, `CAF`, `FLAC`, `OGG`, `OPUS`, `WMA` | **Nativi (zero FFmpeg)**: `M4A`, `WAV`, `FLAC`, `AIFF`, `ALAC`, `CAF` · Con FFmpeg: + `MP3`, `OGG`, `OPUS`, `AAC`, più `ZIP` | `AVFoundation` nativo + CoreAudio `ExtAudioFile` + `/usr/bin/afconvert` + `FFmpeg` opzionale |
| **Video** | `MP4`, `MOV`, `M4V`, `MKV`, `WEBM`, `AVI`, `WMV` | Nativi: `MP4`, `MOV`, `M4V`, `M4A`, `WAV`, `GIF` animata, `ZIP` · Con FFmpeg: + `MKV`, `WEBM`, `AVI`, `MP3` | `AVFoundation` nativo + `FFmpeg` opzionale (progresso in tempo reale & `Esc`) |
| **Archivi** | `ZIP` (e qualsiasi selezione di file) | `UNZIP` (estrazione sicura in sottocartella) · `ZIP` (creazione archivio senza sovrascritture) | Motore Rust `zip` (`archive.rs` con blocchi Zip-Slip e >256 MiB / 4.096 entry) |

> **Nota sui tool opzionali**: FFmpeg e Pandoc **non** sono inclusi nel bundle. **EPUB 3 e le conversioni audio lossless `CAF` e `FLAC` funzionano nativamente senza FFmpeg e senza Pandoc.** Installa `brew install ffmpeg pandoc` (o indica i percorsi in **Settings → Engines & Permissions**) per sbloccare formati extra (`MKV`/`WebM`/`AVI`/`MP3`/`OGG`/`OPUS`/`AAC` con FFmpeg e libri `FB2`/`RST` con Pandoc).

---

## CLI e comandi di controllo

FLIP include una CLI integrata in `FLIP.app/Contents/MacOS/flip` (collegabile in `~/.local/bin/flip` da **Settings → Engines & Permissions**):

```bash
flip                                          # Avvia app menu bar (istanza singola)
flip convert <file...> --to <FMT[,FMT2]>      # Conversione locale con output JSON
flip new <kind> [--in <folder>] [--no-rename] # Crea nuovo file/cartella
flip wheel [--mode convert|new] [files...]    # Apre la ruota via Unix socket IPC
flip settings                                 # Apre la finestra Impostazioni
flip batch [files...]                         # Apre il convertitore Batch & Multi-Formato
flip quit                                     # Chiude in modo pulito l'istanza attiva
flip --help                                   # Guida rapida CLI
flip --version                                # Mostra versione
```

---

## Permessi macOS

- **Accessibilità** (`Impostazioni di Sistema → Privacy e sicurezza → Accessibilità`): permette al `CGEventTap` di intercettare il tasto di drag *solo* durante un trascinamento file (senza disturbare la digitazione normale) e di inviare `Invio` per la rinomina inline nel Finder.
- **Automazione Finder** (`Privacy e sicurezza → Automazione → FLIP → Finder`): consente di leggere i file selezionati e la cartella attiva nel Finder.
- **Modalità limitata**: senza permessi FLIP resta utilizzabile tramite la scorciatoia Idle (`Option+Shift+Space`), il drag-and-drop diretto sulla ruota o sulla finestra Batch e il fallback su `~/Desktop`.

---

## Compilazione ed esecuzione locale

### Prerequisiti

- **macOS 15.0+** (Apple Silicon; sviluppato per **macOS 27.2.1 Golden Gate**)
- **Apple Command Line Tools** (`xcode-select --install`) — `Xcode.app` completo **non** è necessario (usa `/usr/bin/swiftc` e `/usr/bin/clang`)
- **Rust** (`cargo`) e `tauri-cli` (`cargo install tauri-cli --version ^2`)
- **Node.js / npm** (e **Bun** opzionale per i test frontend)
- Opzionali: `brew install ffmpeg pandoc`

### Sviluppo

```bash
npm install
./native-bridge/build.sh
cargo tauri dev
```

### Packaging `.app`, `.dmg` e firma ad-hoc locale

Compila il bridge nativo Swift, il frontend Vite, il bundle `.app` release, verifica l'inclusione di `flip-macos-bridge` (`+x`) e `flip-tick.wav`, e applica la firma ad-hoc con verifica rigorosa in un solo comando (usa `--dmg` per generare anche l'immagine disco compressa e firmata):

```bash
./scripts/build-macos.sh --dmg
```

Output firmati:

```text
src-tauri/target/release/bundle/macos/FLIP.app
src-tauri/target/release/bundle/dmg/FLIP-0.1.0-macOS-arm64.dmg
```

Per firmare nuovamente o usare un certificato Developer ID / notarizzazione:

```bash
# Firma ad-hoc locale (nessun certificato a pagamento richiesto)
./scripts/sign-local.sh

# Opzionale: Developer ID + notarizzazione Apple
APPLE_SIGNING_IDENTITY="Developer ID Application: Il Tuo Nome (TEAMID)" \
APPLE_NOTARY_PROFILE="notary-profile" \
./scripts/sign-local.sh
```

### Suite di test

```bash
./native-bridge/build.sh
npx tsc --noEmit
npm run build
bun test
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

I risultati completi di test unitari, integrazione, stress a 50 cicli, benchmark di latenza e verifica `.app`/`.dmg` sono documentati in [`TEST_RESULTS.md`](TEST_RESULTS.md).

---

## Licenza

Licenza MIT — vedi [LICENSE](LICENSE).
