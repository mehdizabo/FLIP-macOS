import { describe, expect, test } from "bun:test";
import {
  EN_STRINGS,
  IT_STRINGS,
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
});
