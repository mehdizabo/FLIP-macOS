# 04 · Matrice delle conversioni

> **Documento storico di pianificazione.** Descrive il piano iniziale del porting (Swift/SwiftUI nativo) e non l'implementazione attuale. L'architettura vigente (Tauri v2 + Rust + bridge Swift) è in [`../ARCHITECTURE.md`](../ARCHITECTURE.md) e [`DECISIONS.md`](DECISIONS.md).

Legenda motore: **N** = nativo Apple · **L** = libreria inclusa (SPM) · **FF** = FFmpeg opzionale · **PD** = Pandoc opzionale · **APP** = app Apple via AppleScript (opzionale).
Regola invariata da Windows: il formato del sorgente non compare mai tra le uscite. Con più file, le uscite sono l'**intersezione** dei formati compatibili.

## Immagini

| Input | Uscite | Motore |
|---|---|---|
| PNG, JPG/JPEG, WebP, BMP, TIFF, GIF, **HEIC/HEIF, AVIF** (lettura) | PNG, JPG, TIFF, GIF, BMP, **HEIC**, PDF | N (ImageIO, PDFKit) |
| idem | **WebP** | L (libwebp) |
| più immagini selezionate | **"Combine into PDF"** (un PDF unico con più pagine) | N, *nuovo* |

Opzioni: qualità JPEG/HEIC/WebP (default 0,9), conservazione dei metadati EXIF (default sì, tranne GPS se l'utente lo disattiva), profilo colore sRGB per il web.

## PDF

| Input | Uscite | Motore |
|---|---|---|
| PDF | PNG, JPG, WebP, HEIC (una per pagina: `name.converted-p01.png`) | N (PDFKit) + L per WebP |
| PDF | TXT (testo estratto; OCR con Vision se la pagina non ha testo) | N |
| PDF | RTF, DOCX, HTML, MD (testo con struttura minima) | N (lossy) / PD per risultati migliori |
| più PDF | **"Merge PDFs"** | N, *nuovo* |

## Documenti di testo

| Input | Uscite | Motore |
|---|---|---|
| TXT, RTF, RTFD, HTML, DOC, DOCX, ODT | TXT, RTF, HTML, DOCX, ODT, **PDF** (A4 di default) | N (`NSAttributedString` + Core Text/WebKit per il PDF) |
| MD | HTML, PDF, DOCX, RTF, ODT, TXT | N (swift-markdown → HTML → NSAttributedString) |
| qualsiasi documento di testo | **MD** | N (convertitore AttributedString → Markdown, base) / PD (fedele) |
| qualsiasi documento di testo | EPUB, FB2, RST | PD |
| EPUB, FB2, RST | altri formati di documento | PD |
| TXT, MD | PNG, JPG (render della prima pagina, come oggi) | N |
| Pages, Numbers, Keynote | PDF, DOCX, XLSX, PPTX | APP (opzionale, Fase 4 o successiva) |

## Dati strutturati

| Input | Uscite | Motore |
|---|---|---|
| JSON, YAML/YML, XML, CSV | JSON, YAML, XML, CSV, TXT, HTML (tabella), MD (tabella), PDF | N + L (Yams) |

Note: CSV ↔ JSON si gestisce con `TabularData.DataFrame`. Per JSON annidato → CSV vale la stessa regola già adottata su Windows (appiattimento con chiavi puntate): **portare i test esistenti**.

## Audio

| Input | Uscite | Motore |
|---|---|---|
| WAV, AIFF, M4A, AAC, MP3, FLAC, CAF | WAV, AIFF, M4A (AAC), **ALAC** | N (AVAssetReader/Writer) |
| idem | FLAC | N (`ExtAudioFile`, da verificare con spike S09) → altrimenti FF |
| idem | MP3, OGG, OPUS | FF |
| OGG, OPUS, WMA (input) | qualsiasi | FF (Opus in CAF è nativo) |

## Video

| Input | Uscite | Motore |
|---|---|---|
| MP4, MOV, M4V | MP4 (H.264/HEVC), MOV, M4V | N (AVAssetExportSession, VideoToolbox) |
| MP4, MOV, M4V | M4A, WAV (estrazione audio) | N |
| MP4, MOV, M4V | **GIF animata** (≤ 15 s, 480 px) | N (AVAssetImageGenerator + ImageIO), *nuovo* |
| MKV, WebM, AVI, WMV | qualsiasi; e qualsiasi → MKV/WebM/AVI, MP3 | FF |

## Ordine preferito dei settori (per famiglia)

| Famiglia | Ordine predefinito (configurabile) |
|---|---|
| Immagini | PNG, JPG, WebP, HEIC, PDF, TIFF, GIF, BMP |
| PDF | PNG, JPG, TXT, DOCX, MD, HTML, RTF, WebP |
| Documenti | PDF, DOCX, MD, HTML, TXT, RTF, ODT, EPUB |
| Dati | JSON, YAML, CSV, XML, MD, HTML, PDF, TXT |
| Audio | MP3, M4A, WAV, FLAC, AIFF, ALAC, OGG |
| Video | MP4, MOV, GIF, M4A, MP3, WAV, MKV, WebM |

## Naming dell'output (porting 1:1)

`photo.heic` → `photo.converted.png`; se esiste già → `photo.converted 2.png` (uso lo spazio, stile Finder, invece di `-2`). Più pagine → `doc.converted-p01.png`. **Mai** sovrascrivere.
