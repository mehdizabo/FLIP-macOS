import { describe, expect, test } from "bun:test";
import {
  EN_STRINGS,
  IT_STRINGS,
  localizeAvailabilityMessage,
  localizeClipboardKind,
  localizeDisabledReason,
  localizeKindCategory,
  localizeNewKindName,
  normalizeLanguage,
  t,
  type TranslationKey,
} from "./i18n";

describe("EN/IT i18n dictionary and formatter", () => {
  test("every key exists and is non-empty in both en and it", () => {
    const enKeys = Object.keys(EN_STRINGS).sort();
    const itKeys = Object.keys(IT_STRINGS).sort();
    expect(itKeys).toEqual(enKeys);
    expect(enKeys.length).toBeGreaterThan(80);

    for (const key of enKeys as TranslationKey[]) {
      expect(EN_STRINGS[key].trim().length).toBeGreaterThan(0);
      expect(IT_STRINGS[key].trim().length).toBeGreaterThan(0);
    }
  });

  test("preserves placeholder tokens consistently between en and it", () => {
    const extractVars = (s: string) =>
      Array.from(s.matchAll(/\{([a-zA-Z0-9_]+)\}/g), (m) => m[1]).sort();

    for (const key of Object.keys(EN_STRINGS) as TranslationKey[]) {
      expect(extractVars(IT_STRINGS[key])).toEqual(extractVars(EN_STRINGS[key]));
    }
  });

  test("interpolates variables cleanly without double-substituting braces", () => {
    expect(t("en", "wheel.filesCount", { count: 4 })).toBe("4 files");
    expect(t("it", "wheel.filesCount", { count: 4 })).toBe("4 file");

    expect(
      t("it", "settings.formats.moveEarlierAria", {
        fmt: "EPUB",
        pos: 2,
        total: 10,
      }),
    ).toBe("Sposta EPUB prima (sinistra o su), posizione 2 di 10");

    // Value containing `{total}` must not be re-expanded into another placeholder
    expect(
      t("en", "vo.sectorItem", {
        subtitle: "{total}",
        sector: 1,
        total: 8,
      }),
    ).toBe("{total}, sector 1 of 8");

    // Unprovided placeholders stay intact
    expect(t("en", "settings.shortcuts.current")).toBe("Current: {value}");
  });

  test("falls back safely to English on invalid, empty, or undefined languages", () => {
    expect(normalizeLanguage("en")).toBe("en");
    expect(normalizeLanguage("it")).toBe("it");
    expect(normalizeLanguage("IT")).toBe("it");
    expect(normalizeLanguage("it-IT")).toBe("it");
    expect(normalizeLanguage("it_CH")).toBe("it");
    expect(normalizeLanguage("fr")).toBe("en");
    expect(normalizeLanguage("")).toBe("en");
    expect(normalizeLanguage(null)).toBe("en");
    expect(normalizeLanguage(undefined)).toBe("en");

    expect(t("fr", "settings.general.testFeedbackBtn")).toBe(
      "Test Wheel Sound & Haptics",
    );
    expect(t(undefined, "settings.title.general")).toBe("General");
    expect(t("it", "settings.general.testFeedbackBtn")).toBe(
      "Testa Suono e Aptica Ruota",
    );
  });

  test("localizes built-in new-file kinds, disabled reasons, clipboard types, and availability messages", () => {
    expect(localizeNewKindName("it", "folder", "Folder")).toBe("Cartella");
    expect(localizeNewKindName("it", "txt", "Plain Text")).toBe("Testo Semplice");
    expect(localizeNewKindName("en", "txt", "Plain Text")).toBe("Plain Text");
    expect(localizeNewKindName("it", "yaml", "YAML Config")).toBe("Configurazione YAML");
    expect(localizeNewKindName("it", "toml", "TOML Config")).toBe("Configurazione TOML");
    expect(localizeNewKindName("it", "xml", "XML Document")).toBe("Documento XML");
    expect(localizeNewKindName("it", "pdf", "Blank PDF (A4)")).toBe("PDF Vuoto (A4)");
    expect(localizeKindCategory("it", "system")).toBe("sistema");
    expect(localizeKindCategory("it", "clipboard")).toBe("appunti");

    expect(
      localizeDisabledReason("it", "Clipboard does not contain image, PDF, RTF, or text"),
    ).toBe("Gli appunti non contengono immagine, PDF, RTF o testo");
    expect(localizeDisabledReason("it", "Keynote.app not installed")).toBe(
      "Keynote.app non installata",
    );
    expect(localizeClipboardKind("it", "image")).toBe("immagine");
    expect(localizeClipboardKind("it", "text")).toBe("testo");

    expect(localizeAvailabilityMessage("it", "Drop local files here")).toBe(
      "Trascina qui file locali",
    );
    expect(
      localizeAvailabilityMessage(
        "it",
        "2 files selected\nUnsupported: DMG, no extension\nNo common output format",
      ),
    ).toBe(
      "2 file selezionati\nNon supportati: DMG, senza estensione\nNessun formato di output comune",
    );
    expect(
      localizeAvailabilityMessage(
        "en",
        "1 file selected\nIncompatible formats: TXT, PNG\nNo common output format",
      ),
    ).toBe("1 file selected\nIncompatible formats: TXT, PNG\nNo common output format");

    expect(t("en", "wheel.actionAnimateGif")).toBe("Animate into GIF");
    expect(t("it", "wheel.actionAnimateGif")).toBe("Anima in GIF");

    expect(t("en", "settings.general.hapticsLabel")).toBe("Trackpad & mouse haptics");
    expect(t("it", "settings.general.hapticsLabel")).toBe("Feedback aptico trackpad e mouse");
    expect(t("en", "settings.general.hapticIntensityLabel")).toBe("Haptic intensity");
    expect(t("it", "settings.general.hapticIntensityLabel")).toBe("Intensità feedback aptico");
    expect(t("en", "settings.general.hapticLight")).toBe("Light");
    expect(t("en", "settings.general.hapticMedium")).toBe("Medium");
    expect(t("en", "settings.general.hapticFirm")).toBe("Firm");
    expect(t("en", "settings.general.hapticStrong")).toBe("Strong");
    expect(t("it", "settings.general.hapticLight")).toBe("Leggero");
    expect(t("it", "settings.general.hapticMedium")).toBe("Medio");
    expect(t("it", "settings.general.hapticFirm")).toBe("Deciso");
    expect(t("it", "settings.general.hapticStrong")).toBe("Forte");
    expect(t("en", "settings.general.auraLabel")).toBe("Screen-edge Aura glow");
    expect(t("it", "settings.general.auraLabel")).toBe("Bagliore Aura ai bordi dello schermo");
    expect(t("en", "settings.general.auraDesc")).toContain("Breathing orange");
    expect(t("it", "settings.general.auraDesc")).toContain("Respiro luminoso arancione");
  });
});
