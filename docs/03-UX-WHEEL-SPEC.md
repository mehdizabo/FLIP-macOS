# 03 · Specifica UX della ruota

Riferimento visivo: **Logi Options+ "Actions Ring"** e gli overlay di sistema di macOS 26 (Liquid Glass). La ruota deve sembrare **parte di macOS**, non un'app di terze parti.

## 1. Modalità e attivazione

| Modalità | Trigger | Contenuto | Conferma |
|---|---|---|---|
| **Drag** | Durante il drag di file dal Finder, premi il tasto trigger (default `↓`) | Formati compatibili con *tutti* i file trascinati | **Drop** sul settore |
| **Idle · Convert** | Hotkey globale (default `⌥⇧Space`) con file selezionati nel Finder | Formati compatibili | Click, oppure *hold-and-release* (rilasci la hotkey sopra il settore, come Logi) |
| **Idle · New** | Hotkey globale senza selezione, oppure `Tab` nella ruota Convert | Tipi di file da creare | Click / release |
| **Mouse button** (opz.) | Tasto laterale o centrale del mouse | Come la modalità Idle | Release |

Regole:
- Il tasto trigger viene **consumato solo durante un drag di file**. Fuori dal drag passa all'app in primo piano, come oggi su Windows.
- Ordine dei gesti libero: drag poi tasto, **oppure** tasto tenuto premuto poi drag (porting di `ShortcutOrderGate`).
- La ruota appare **centrata sul cursore**, spostata all'interno dello schermo se il cursore è vicino al bordo, mai a cavallo tra due monitor.

## 2. Geometria

- Diametro **320 pt** (Logi ≈ 300). Corona esterna 320, foro centrale 120 pt.
- Al massimo **8 settori** sul primo anello. Con più di 8 formati, l'ottavo settore diventa **"More"**: un dwell di 300 ms (o un click) ruota l'anello sulla pagina 2 con animazione. Funziona anche durante il drag, come lo spring-loading del Finder.
- Gap di 4 pt tra i settori, angoli arrotondati (`RingSectorShape` con raggi interno ed esterno raccordati).
- La geometria è **una sola** (`RingGeometry`) e serve sia al disegno sia all'hit test, come `SectorData` su Windows. Zona morta: il foro centrale annulla la selezione.
- La selezione dipende **dall'angolo**, non dalla distanza: basta un piccolo movimento verso un settore (come Logi), entro un raggio massimo di 2× la ruota.

## 3. Stile visivo

- **macOS 26+**: `.glassEffect(.regular.interactive(), in: RingSectorShape)` per ogni settore e `GlassEffectContainer` per il morphing; il settore evidenziato usa `.tint(accentColor)`.
- **macOS 15**: `NSVisualEffectView` `.hudWindow` con maschera ad anello, bordo a 0,5 pt `white.opacity(0.2)`, ombra morbida.
- Contenuto di ogni settore: **SF Symbol** (es. `photo`, `doc.richtext`, `waveform`, `film`, `curlybraces`) più l'etichetta del formato (`PNG`) in SF Pro Rounded semibold 13 pt.
- Settore disabilitato: opacità 0,35, nessun hover, nessun tick. Su Mac i formati incompatibili si possono semplicemente nascondere, perché li conosciamo in anticipo.
- **Centro**: miniatura QuickLook del file (o pila di miniature con badge "×3"), nome troncato al centro e, sotto, il nome del formato evidenziato ("Convert to PNG"). Con nessun settore attivo: "Esc to cancel".
- Rispetta **Light/Dark mode**, il colore d'accento di sistema, **Reduce Motion** e **Reduce Transparency**.

## 4. Movimento e feedback

| Evento | Animazione | Suono | Haptic |
|---|---|---|---|
| Apertura | scale 0,85 → 1 + fade, spring (response 0,28, damping 0,8); settori in cascata di 15 ms | — | — |
| Cambio settore | settore +6% di scala, glass tinto, morphing dell'evidenziazione | tick breve (porting di `flip-tick.wav`) | `.alignment` |
| More → pagina 2 | rotazione di 45° + crossfade | tick doppio | `.levelChange` |
| Drop / conferma | il settore "pulsa", poi la ruota si contrae verso il centro | — | `.generic` |
| Conversione | arco di progresso sul bordo esterno + percentuale al centro (porting di `ProgressArcData`) | — | — |
| Successo | ✓ verde disegnato con un trim, chiusura dopo 600 ms | suono "success" soft (opz.) | — |
| Errore | shake orizzontale + messaggio breve al centro, poi chiusura o "Show details" | `NSSound.beep` | — |

## 5. Tastiera e accessibilità

- `←/→` oppure `↑/↓` ruotano la selezione, `1–8` scelgono direttamente, `Return` conferma, `Tab` passa tra Convert e New, `Esc` chiude o annulla.
- VoiceOver: ogni settore è un `accessibilityElement` ("Convert to PNG, button, 1 of 6").
- Quando la ruota è aperta in modalità Idle, focus visibile e annuncio VoiceOver.
- Per il drag non serve focus: il pannello è `nonactivating` e non ruba il focus al Finder.

## 6. Ruota "New" (vedi 05-CREATE-NEW-SPEC)

Stessa geometria, con icone colorate dei tipi di file (icone di sistema da `NSWorkspace.icon(for: UTType)`). In alto, una pill "Convert | New" mostra la modalità attiva quando entrambe sono disponibili.

## 7. Menu bar e impostazioni

- Icona monocromatica nella menu bar (template image). Menu:
  - Open Wheel
  - New File in Finder Folder…
  - Recent conversions ▸
  - Settings…
  - Check for Updates…
  - About
  - Quit
- La voce Settings apre una finestra SwiftUI a schede:
  - **General**: login item, suoni, haptic, sottocartella di output
  - **Shortcuts**: trigger nel drag, hotkey Idle, tasto del mouse
  - **Formats**: ordine e preferiti per famiglia, qualità JPEG/WebP, pagina A4/Letter
  - **New File**: tipi visibili, cartella dei template
  - **Engines**: stato di FFmpeg e Pandoc, con il pulsante "Locate…" e le istruzioni Homebrew
  - **Permissions**: stato e pulsanti per aprire le Impostazioni di Sistema
  - **About**
