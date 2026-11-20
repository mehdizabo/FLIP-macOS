# FLIP for Mac: piano di porting

> **Documento storico di pianificazione.** Descrive il piano iniziale del porting (Swift/SwiftUI nativo) e non l'implementazione attuale. L'architettura vigente (Tauri v2 + Rust + bridge Swift) è in [`../ARCHITECTURE.md`](../ARCHITECTURE.md) e [`DECISIONS.md`](DECISIONS.md).

> Sorgente: `FLIP-windows` (FLIP 0.5.1, .NET 10 WPF, circa 3.000 righe C#)
> Obiettivo: un'app **nativa macOS**, interfaccia **in inglese**, con una ruota radiale di qualità paragonabile all'Actions Ring di Logi Options+. Mantiene tutte le funzioni di FLIP e aggiunge **"New File"**, l'equivalente di "Nuovo ▸" di Windows.

## Verdetto in una riga

**Il porting è fattibile e il Mac lo rende più naturale che su Windows.** Molte conversioni diventano native (ImageIO, PDFKit, AVFoundation, NSAttributedString per DOCX/ODT/RTF, TabularData) e il drag aggiunge un vantaggio: macOS espone i file trascinati *prima* del drop (`NSPasteboard(name: .drag)`). Il codice C# **non si riusa**: si riscrive in Swift, ma la logica (matrice formati, regole di sicurezza, naming, test) si porta quasi 1:1.
Ci sono tre vincoli reali:
1. **Niente Mac App Store**: intercettare un tasto durante il drag richiede Accessibility/Input Monitoring, che sono incompatibili con la sandbox. Distribuzione **Developer ID + notarizzazione** (come Logi Options+).
2. **WebP in scrittura, MP3, OGG, MKV, WebM, AVI ed EPUB** non sono nativi. Servono librerie (libwebp) o motori opzionali (FFmpeg, Pandoc), come oggi su Windows.
3. Il comportamento del tasto *durante* un drag del Finder va verificato con uno spike (Fase 0) prima di investire nella UI.

## Indice dei documenti

| # | File | Contenuto |
|---|------|-----------|
| 01 | [01-FEASIBILITY.md](01-FEASIBILITY.md) | Analisi di fattibilità: mappa Windows → macOS, cosa è nativo e cosa no |
| 02 | [02-ARCHITECTURE.md](02-ARCHITECTURE.md) | Stack, moduli, processi, permessi, distribuzione |
| 03 | [03-UX-WHEEL-SPEC.md](03-UX-WHEEL-SPEC.md) | Specifica della ruota: modalità, gesti, stile Liquid Glass, stati |
| 04 | [04-CONVERSION-MATRIX.md](04-CONVERSION-MATRIX.md) | Matrice conversioni e motore per ogni coppia di formati |
| 05 | [05-CREATE-NEW-SPEC.md](05-CREATE-NEW-SPEC.md) | Funzione "New File" (da zero, template, dagli appunti) |
| 06 | [06-RISKS.md](06-RISKS.md) | Registro dei rischi e mitigazioni |
| P1 | [PLAN-1-SPIKES-FOUNDATION.md](PLAN-1-SPIKES-FOUNDATION.md) | Step 01–22: decisioni, spike tecnici, scheletro dell'app |
| P2 | [PLAN-2-TRIGGER-WHEEL.md](PLAN-2-TRIGGER-WHEEL.md) | Step 23–50: input globale e ruota radiale |
| P3 | [PLAN-3-CONVERSION-ENGINE.md](PLAN-3-CONVERSION-ENGINE.md) | Step 51–70: motore di conversione |
| P4 | [PLAN-4-CREATE-NEW-INTEGRATION.md](PLAN-4-CREATE-NEW-INTEGRATION.md) | Step 71–87: New File e integrazione con Finder, Shortcuts e sistema |
| P5 | [PLAN-5-QUALITY-RELEASE.md](PLAN-5-QUALITY-RELEASE.md) | Step 88–98: test, accessibilità, firma, notarizzazione, rilascio |

**Totale: 98 step** in 7 fasi (F0–F6).

## Visione del prodotto (cosa ho capito)

- **FLIP for Mac** vive nella **menu bar**, senza icona nel Dock.
- **Modalità Drag**: trascini uno o più file dal Finder e premi un tasto (default `↓`, configurabile). Sotto il cursore appare la ruota con **solo i formati compatibili**: rilasci il file su un settore e FLIP salva una copia convertita accanto all'originale. L'originale non viene mai toccato né sovrascritto.
- **Modalità Idle**: premi una scorciatoia globale (es. `⌥⇧Space`, configurabile, opzionalmente un tasto del mouse).
  - Se nel Finder ci sono file selezionati, la ruota offre la **conversione**.
  - Se non c'è selezione, offre **New File** nella cartella attiva del Finder (o sul Desktop): file vuoti, template, contenuto degli appunti.
- **Feedback di qualità**: Liquid Glass (macOS 26), animazioni a molla, tick sonoro, haptic sul trackpad, arco di avanzamento con percentuale, segno di spunta finale, Esc per annullare.
- **Tutto in locale**: nessun cloud, nessun account.

## Decisioni chiave (proposte, da confermare)

| Tema | Proposta | Alternativa |
|------|----------|-------------|
| Linguaggio/UI | Swift 6 + SwiftUI, con AppKit per pannelli, eventi e drag | — |
| macOS minimo | **macOS 15 Sequoia**, con Liquid Glass su 26+ e fallback `NSVisualEffectView` | Solo macOS 26+ (più semplice, meno utenti) |
| Distribuzione | Developer ID + notarizzazione + Sparkle | App Store: impossibile per la funzione core |
| Motori opzionali | FFmpeg/Pandoc **rilevati** (Homebrew/percorso utente), non inclusi nel pacchetto | Inclusione di FFmpeg LGPL: pesa sulla licenza e sulla dimensione |
| Nome/bundle | `FLIP` · `com.thetinyfoundry.flip.mac` | — |

## Domande aperte per te

1. macOS minimo: 15+ con fallback, oppure solo 26+ (Tahoe) per avere Liquid Glass ovunque?
2. Vuoi anche un trigger **da tasto del mouse** (pulsante laterale o centrale, come Logi)? Richiede un event tap, quindi lo stesso permesso Accessibility.
3. "New File": va bene creare nella cartella frontale del Finder e avviare subito la **rinomina inline**?
4. Formati Apple (Pages, Numbers, Keynote): vuoi l'export tramite le app Apple, se installate (AppleScript)?
5. Hai un Apple Developer account a pagamento (99 $/anno)? Serve per la notarizzazione.
