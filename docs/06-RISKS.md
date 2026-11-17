# 06 · Registro dei rischi

| ID | Rischio | Prob. | Impatto | Mitigazione | Step |
|---|---|---|---|---|---|
| R1 | Il tasto premuto durante un drag del Finder non arriva all'hotkey Carbon (il Finder è in un tracking loop) | Media | Alto | Spike con Carbon **e** CGEventTap; il tap a livello `kCGSessionEventTap` vede comunque gli eventi | S03–S04 |
| R2 | Il pannello che appare *durante* il drag non riceve `draggingEntered` | Bassa | Alto | Spike S05; finestra di livello `.popUpMenu` registrata prima (nascosta, `alphaValue = 0`) | S05 |
| R3 | Prompt di permessi che spaventano l'utente | Alta | Medio | Onboarding a 3 schermate con animazione, modalità ridotta senza Accessibility | S19–S20 |
| R4 | Le API per la selezione del Finder via Apple Events sono lente (100–300 ms) | Media | Medio | Chiamata asincrona; apertura immediata della ruota con un placeholder; cache della selezione | S29 |
| R5 | Liquid Glass su forme custom con prestazioni scarse | Bassa | Medio | `GlassEffectContainer`, un solo container, misura con Instruments; fallback con materiale piatto | S06, S48 |
| R6 | FLAC/WebP/MP3 non nativi | Certa | Basso | libwebp inclusa; FFmpeg opzionale rilevato; settori disabilitati con spiegazione | S09, S63 |
| R7 | Licenze (FFmpeg GPL/LGPL, Pandoc GPL) | Media | Alto | **Non includerli**: solo rilevamento ed esecuzione di binari installati dall'utente | S63 |
| R8 | Il drag di file da app diverse dal Finder (Mail, Safari, Foto) usa promise file e non URL | Alta | Medio | Gestire `NSFilePromiseReceiver` (scrivere in una cartella temporanea, poi convertire nel Downloads o in una cartella scelta) | S27 |
| R9 | macOS cambia le API dei permessi (Sequoia: prompt mensili per screen capture ecc.) | Media | Medio | Non usare mai Screen Recording; testare su ogni beta | S94 |
| R10 | Conflitto del tasto `↓` con app che lo usano durante il drag | Bassa | Basso | Tasto configurabile; consumo solo mentre il pasteboard di drag contiene file | S24 |
| R11 | La notarizzazione fallisce per i binari esterni | Bassa | Medio | Nessun binario esterno nel bundle; libwebp compilata da sorgente | S96 |
| R12 | Rinomina inline fragile (timing del Finder) | Media | Basso | Ritardo adattivo + verifica che la selezione sia quella giusta; opzione disattivabile | S76 |
