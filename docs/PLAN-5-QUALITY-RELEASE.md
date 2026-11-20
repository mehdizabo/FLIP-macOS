# PLAN 5 · Fase 6: qualità e rilascio, step 88–98

> **Documento storico di pianificazione.** Descrive il piano iniziale del porting (Swift/SwiftUI nativo) e non l'implementazione attuale. L'architettura vigente (Tauri v2 + Rust + bridge Swift) è in [`../ARCHITECTURE.md`](../ARCHITECTURE.md) e [`DECISIONS.md`](DECISIONS.md).

**S88. Suite di test automatici.** Unit (FlipCore), golden file per ogni coppia della matrice (fixture in `Tests/Fixtures`, comprese quelle di FLIP Windows), test di integrazione per i motori opzionali (saltati se il motore manca), snapshot della ruota.
→ *Copertura ≥ 80% su FlipCore e FlipEngines; CI verde.*

**S89. Test UI.** XCUITest per Settings, onboarding e ruota in modalità Idle (driver da tastiera); il drag dal Finder resta un test manuale documentato.
→ *Smoke UI in CI.*

**S90. Checklist di accettazione fisica.** Porting e adattamento del `ROADMAP.md` Windows: drag con tasto in entrambi gli ordini, Esc e cancellazione, multi-monitor/DPI, trackpad vs mouse, full screen, Stage Manager, Light/Dark, Reduce Motion/Transparency, VoiceOver.
→ *`ACCEPTANCE.md` compilata con data ed evidenze (video).*

**S91. Prestazioni.** Instruments: tempo di avvio < 300 ms, apertura ruota < 16 ms dal trigger, CPU a riposo ≈ 0% (nessun polling continuo), memoria a riposo < 40 MB, nessun leak dopo 100 conversioni.
→ *Report Instruments salvato.*

**S92. Gestione errori e messaggi.** Catalogo errori in inglese, chiaro e azionabile ("FFmpeg isn't installed. Install it with Homebrew: `brew install ffmpeg`"), con il pulsante "Copy diagnostics" (log redatti).
→ *Revisione del copy completata.*

**S93. Documentazione utente.** README in inglese (e italiano), GIF/video demo del gesto (porting di `flip-demo.mp4`), guida ai permessi, FAQ e tabella delle conversioni.
→ *Documenti pubblicabili.*

**S94. Compatibilità OS.** Test su macOS 15 (fallback) e 26 (Liquid Glass), Apple Silicon e Intel (se supportato: universal binary); verifica sulla beta di macOS 27.
→ *Matrice di compatibilità in `TEST_RESULTS.md`.*

**S95. Licenze e notice.** `THIRD_PARTY_NOTICES.md` (Sparkle, KeyboardShortcuts, Yams, swift-markdown, libwebp); chiarimento che FFmpeg e Pandoc sono installati dall'utente e non distribuiti.
→ *Revisione delle licenze completata.*

**S96. Firma e notarizzazione.** Certificato Developer ID, hardened runtime, entitlement minimi (Apple Events), firma di app ed estensione, `notarytool submit --wait`, `stapler staple`; tutto in uno script `scripts/release.sh` e in un workflow CI con i segreti.
→ *`spctl -a -vv` = "accepted, source=Notarized Developer ID".*

**S97. Packaging.** DMG con sfondo e link ad Applicazioni (`create-dmg`), firmato e notarizzato; appcast Sparkle aggiornato; Homebrew Cask opzionale.
→ *Installazione pulita su un Mac mai usato (o un nuovo utente).*

**S98. Beta e rilascio 1.0.** TestFlight non è disponibile fuori dall'App Store: beta privata via DMG a 5–10 tester con un modulo di feedback; correzioni; tag `v1.0.0`, release notes e annuncio.
→ *Release pubblica; backlog post-1.0 aperto (Pages/Numbers/Keynote via AppleScript, EPUB nativo, localizzazione italiana, ruota personalizzabile con azioni oltre la conversione, in stile Logi).*

---

### Dopo la 1.0: idee coerenti con "Logi Actions Ring"

- La ruota diventa un **launcher di azioni** contestuale: ridimensiona immagine, comprimi PDF, rimuovi lo sfondo (Vision `VNGenerateForegroundInstanceMaskRequest`), estrai il testo (OCR), copia il percorso, crea uno ZIP.
- **Profili per app**: azioni diverse se è in primo piano il Finder, Foto o Anteprima.
- **Azioni utente**: script e Shortcut personali assegnabili a un settore.
