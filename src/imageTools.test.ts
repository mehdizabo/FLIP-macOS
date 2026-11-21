import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { EN_STRINGS, IT_STRINGS } from "./i18n";
import { getFormatMeta, getToolGroupMeta, toolGroupKey, toolTitleKey } from "./icons";
import {
  buildToolEntries,
  computeScaledDimensions,
  IMAGE_TOOL_TOKENS,
  isImageToolToken,
  TOOL_GROUPS,
  toolActionsForExtensions,
  toolGroupOf,
  toolOutputExtension,
  toolOutputSuffix,
} from "./imageTools";

const RUST_SOURCE = readFileSync(
  join(import.meta.dir, "..", "src-tauri", "src", "engines", "image_tools.rs"),
  "utf8",
);

/** Pulls a `&[ "A", "B", ... ]` string list out of the Rust source, by constant name. */
function rustTokenList(name: string): string[] {
  const match = RUST_SOURCE.match(new RegExp(`${name}[^=]*=\\s*&\\[([^\\]]*)\\]`));
  if (!match) throw new Error(`${name} not found in image_tools.rs`);
  return Array.from(match[1].matchAll(/"([A-Z0-9_]+)"/g), (m) => m[1]);
}

const IMAGE_EXTS: ReadonlySet<string> = new Set([
  "jpg",
  "jpeg",
  "png",
  "webp",
  "gif",
  "bmp",
  "tif",
  "tiff",
  "heic",
  "heif",
  "avif",
]);

describe("image tool tokens: Rust and TypeScript stay in lock-step", () => {
  test("IMAGE_TOOL_TOKENS is identical (set and order) to Rust ALL_TOOL_TOKENS", () => {
    expect([...IMAGE_TOOL_TOKENS]).toEqual(rustTokenList("ALL_TOOL_TOKENS"));
  });

  test("suffixes mirror the Rust ImageTool::suffix table", () => {
    for (const token of IMAGE_TOOL_TOKENS) {
      // Every suffix the TS side advertises must be literally present in the Rust engine.
      expect(RUST_SOURCE).toContain(`"${toolOutputSuffix(token)}"`);
    }
    expect(toolOutputSuffix("REDACT")).toBe("-redacted");
  });

  test("every token is recognised and groups only contain known tokens", () => {
    for (const token of IMAGE_TOOL_TOKENS) {
      expect(isImageToolToken(token)).toBe(true);
      expect(isImageToolToken(token.toLowerCase())).toBe(true);
    }
    expect(isImageToolToken("PNG")).toBe(false);
    expect(isImageToolToken("REMOVE_BG")).toBe(false);
    for (const group of TOOL_GROUPS) {
      for (const token of group.tokens) {
        expect(toolGroupOf(token)).toBe(group.id);
      }
    }
    expect(toolGroupOf("PALETTE")).toBeNull();
  });
});

describe("image tool presentation", () => {
  test("each token has its own icon, label and EN + IT title", () => {
    const seenTitles = new Set<string>();
    for (const token of IMAGE_TOOL_TOKENS) {
      const key = toolTitleKey(token);
      expect(EN_STRINGS[key]).toBeDefined();
      expect(IT_STRINGS[key]).toBeDefined();

      const en = getFormatMeta(token, "en");
      const it = getFormatMeta(token, "it");
      expect(en.iconSvg).toContain("<svg");
      expect(en.shortLabel.length).toBeGreaterThan(0);
      expect(en.shortLabel.length).toBeLessThanOrEqual(7);
      // Must not fall into the generic "Convert to X" branch.
      expect(en.title).not.toContain("Convert to");
      expect(en.title).toBe(EN_STRINGS[key]);
      expect(it.title).toBe(IT_STRINGS[key]);
      seenTitles.add(en.title);
    }
    expect(seenTitles.size).toBe(IMAGE_TOOL_TOKENS.length);
  });

  test("tool groups have names in both languages", () => {
    for (const group of TOOL_GROUPS) {
      const key = toolGroupKey(group.id);
      expect(EN_STRINGS[key]).toBeDefined();
      expect(IT_STRINGS[key]).toBeDefined();
      expect(getToolGroupMeta(group.id, "it").title).toBe(IT_STRINGS[key]);
      expect(getToolGroupMeta(group.id, "en").iconSvg).toContain("<svg");
    }
  });
});

describe("tool availability and first-page entries", () => {
  test("output extension follows the source format, palette is always PNG", () => {
    expect(toolOutputExtension("CROP_1_1", "jpg")).toBe("jpg");
    expect(toolOutputExtension("CROP_1_1", "HEIC")).toBe("jpg");
    expect(toolOutputExtension("ROTATE_LEFT", "webp")).toBe("webp");
    expect(toolOutputExtension("FLIP_H", "tiff")).toBe("tiff");
    expect(toolOutputExtension("SCALE_50", "gif")).toBe("png");
    expect(toolOutputExtension("REDACT", "jpg")).toBe("jpg");
    expect(toolOutputExtension("REDACT", "HEIC")).toBe("jpg");
    expect(toolOutputExtension("REDACT", "webp")).toBe("webp");
    expect(toolOutputExtension("PALETTE", "jpg")).toBe("png");
  });

  test("selection of images offers tools; any non-image offers none", () => {
    expect(toolActionsForExtensions(["jpg", "png"], IMAGE_EXTS)).toHaveLength(
      IMAGE_TOOL_TOKENS.length,
    );
    expect(toolActionsForExtensions(["jpg", "pdf"], IMAGE_EXTS)).toEqual([]);
    expect(toolActionsForExtensions(["jpg", "pdf"], IMAGE_EXTS)).not.toContain("REDACT");
    expect(toolActionsForExtensions([], IMAGE_EXTS)).toEqual([]);
  });

  test("EXIF removal is withheld when a file cannot carry metadata", () => {
    const withGif = toolActionsForExtensions(["jpg", "gif"], IMAGE_EXTS);
    expect(withGif).not.toContain("STRIP_EXIF");
    expect(withGif).toContain("CROP_1_1");
  });

  test("first page: Crop, Rotate, Scale, Remove BG, Redact, Palette, EXIF, ZIP in original-FLIP order", () => {
    const entries = buildToolEntries([...IMAGE_TOOL_TOKENS], ["PNG", "REMOVE_BG"], ["ZIP"]);
    expect(
      entries.map((e) => (e.kind === "group" ? `group:${e.id}` : `leaf:${e.token}`)),
    ).toEqual([
      "group:crop",
      "group:rotate",
      "group:scale",
      "leaf:REMOVE_BG",
      "leaf:REDACT",
      "leaf:PALETTE",
      "leaf:STRIP_EXIF",
      "leaf:ZIP",
    ]);
  });

  test("first page omits Redact when the selection cannot use it", () => {
    const withoutRedact = IMAGE_TOOL_TOKENS.filter((t) => t !== "REDACT");
    const entries = buildToolEntries([...withoutRedact], ["PNG"], []);
    expect(entries.some((e) => e.kind === "leaf" && e.token === "REDACT")).toBe(false);
  });

  test("first page omits what the selection cannot use", () => {
    expect(buildToolEntries([], ["PDF"], [])).toEqual([]);
    const gifOnly = buildToolEntries(
      toolActionsForExtensions(["gif"], IMAGE_EXTS),
      ["PNG"],
      [],
    );
    expect(gifOnly.some((e) => e.kind === "leaf" && e.token === "STRIP_EXIF")).toBe(false);
    expect(gifOnly.some((e) => e.kind === "group" && e.id === "crop")).toBe(true);
  });
});

describe("computeScaledDimensions", () => {
  test("computes expected dimensions for 4032x3024 across SCALE_* presets", () => {
    expect(computeScaledDimensions(4032, 3024, "SCALE_50")).toEqual({ width: 2016, height: 1512 });
    expect(computeScaledDimensions(4032, 3024, "SCALE_25")).toEqual({ width: 1008, height: 756 });
    expect(computeScaledDimensions(4032, 3024, "SCALE_1080P")).toEqual({ width: 1440, height: 1080 });
    expect(computeScaledDimensions(4032, 3024, "SCALE_720P")).toEqual({ width: 960, height: 720 });
    expect(computeScaledDimensions(4032, 3024, "scale_1080p")).toEqual({ width: 1440, height: 1080 });
  });

  test("handles portrait orientation and never upscales smaller images", () => {
    expect(computeScaledDimensions(3024, 4032, "SCALE_1080P")).toEqual({ width: 1080, height: 1440 });
    expect(computeScaledDimensions(1280, 720, "SCALE_1080P")).toEqual({ width: 1280, height: 720 });
    expect(computeScaledDimensions(640, 480, "SCALE_720P")).toEqual({ width: 640, height: 480 });
  });

  test("returns null for SCALE_1MB, non-scale tokens, and non-positive dimensions", () => {
    expect(computeScaledDimensions(4032, 3024, "SCALE_1MB")).toBeNull();
    expect(computeScaledDimensions(4032, 3024, "CROP_1_1")).toBeNull();
    expect(computeScaledDimensions(4032, 3024, "PNG")).toBeNull();
    expect(computeScaledDimensions(0, 3024, "SCALE_50")).toBeNull();
    expect(computeScaledDimensions(4032, -10, "SCALE_50")).toBeNull();
    expect(computeScaledDimensions(Number.NaN, 3024, "SCALE_50")).toBeNull();
  });
});
