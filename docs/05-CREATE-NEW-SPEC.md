# 05 · Specifica "New File"

Al Mac manca un "Nuovo ▸" nel Finder come quello di Windows. FLIP lo aggiunge in **tre punti**: la ruota (modalità Idle · New), il menu della menu bar e il menu contestuale del Finder (Finder Sync Extension, Fase 5).

## 1. Dove viene creato il file

Risoluzione della cartella di destinazione, in ordine:
1. Il menu contestuale Finder Sync fornisce `targetedURL` (la cartella cliccata).
2. Finder in primo piano: `tell application "Finder" to get insertion location as alias` restituisce la cartella della finestra frontale (o il Desktop se non ci sono finestre).
3. Altrimenti: il Desktop dell'utente (impostabile in Settings: "Fallback folder").

Verifiche: la cartella esiste ed è scrivibile e non è un volume read-only. Se non lo è: errore chiaro al centro della ruota.

## 2. Tipi predefiniti (8 settori, configurabili)

| Settore | File | Come viene generato |
|---|---|---|
| 📁 Folder | `untitled folder` | `FileManager.createDirectory` |
| 📝 Plain Text | `Untitled.txt` | file vuoto UTF-8 |
| ✍️ Markdown | `Untitled.md` | `# Untitled\n` |
| 📄 Rich Text | `Untitled.rtf` | NSAttributedString vuota → RTF |
| 🟦 Word Document | `Untitled.docx` | NSAttributedString → `.officeOpenXML` |
| 📊 Spreadsheet | `Untitled.xlsx` / `.csv` | template minimo `blank.xlsx` incluso nel bundle |
| 🌐 HTML | `Untitled.html` | boilerplate HTML5 |
| 📋 **From Clipboard** | `Clipboard <date>.png/.txt/.rtf/.pdf` | dipende dal contenuto di `NSPasteboard.general`: immagine → PNG, testo ricco → RTF, testo → TXT, PDF → PDF |

Pagina 2 ("More"):
- JSON (`{}`)
- Shell Script (`#!/bin/zsh`, `chmod +x`)
- Python (`.py`)
- Swift
- PowerPoint (`blank.pptx`)
- Pages, Numbers e Keynote (solo se le app sono installate: si copia un template vuoto incluso, creato con le app Apple)
- **Templates…**

## 3. Template utente

- La cartella `~/Library/Application Support/FLIP/Templates/` si apre da Settings → New File → "Open Templates Folder".
- Ogni file messo lì diventa un tipo creabile (nome visualizzato = nome del file senza estensione, icona di sistema).
- Le sottocartelle diventano gruppi. Fino a 8 template preferiti finiscono direttamente sull'anello.
- Segnaposti opzionali nei file di testo: `{{date}}`, `{{time}}`, `{{user}}`, `{{folder}}`.

## 4. Dopo la creazione

1. Naming stile Finder: `Untitled.txt`, `Untitled 2.txt`, …, senza sovrascrivere mai.
2. Il file viene rivelato e selezionato nel Finder (`NSWorkspace.activateFileViewerSelecting`).
3. **Rinomina inline** (opzionale, attiva di default): con il Finder in primo piano e il file selezionato, invio del tasto `Return` via System Events (richiede Accessibility, che è già concesso). Se il permesso manca, si salta questo passo.
4. Opzionale: "Open after creating" apre il file nell'app predefinita.

## 5. Casi limite

- Cartella protetta (es. `/Applications`, volume di sistema): errore "Can't create files here".
- Ricerca o "Recenti" nel Finder (nessuna cartella reale): fallback sul Desktop, con un avviso.
- Appunti vuoti o con contenuto non supportato: settore "From Clipboard" disabilitato, con un tooltip.
- iCloud Drive: supportato (è una cartella locale). Le cartelle di rete restano consentite per *creare*, ma non per convertire (stessa policy di Windows).
