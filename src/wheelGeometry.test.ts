import { describe, expect, test } from "bun:test";
import {
  consumeBatchHandoff,
  escapeHtml,
  getThumbnail,
  inspectFilesLocally,
  storeBatchHandoff,
} from "./ipc";
import { resolveRoute } from "./router";
import { mapBrowserKeyToToken } from "./views/SettingsView";
import {
  formatCompletionAnnouncement,
  formatPageFlipAnnouncement,
  formatProgressMilestoneAnnouncement,
  formatSectorAriaLabel,
  formatWheelOpenAnnouncement,
  getProgressMilestoneBucket,
  getSectorAngles,
  getSectorAtPoint,
  getSectorCenter,
  moveFormatToIndex,
  orderFormats,
  paginateWheelItems,
  polarToCartesian,
  progressArcSvgPath,
  progressRaySegment,
  progressTipPoint,
  reorderFormatList,
  sectorSvgPath,
  WHEEL_CENTER,
  WHEEL_DEAD_ZONE_RADIUS,
  WHEEL_OUTER_MAX_RADIUS,
  WHEEL_PROGRESS_RADIUS,
} from "./wheelGeometry";

describe("orderFormats", () => {
  test("orders formats using default Wheel.cs preference order and preserves unknown trailing items", () => {
    const input = ["PNG", "CUSTOM2", "PDF", "JPG", "DOCX", "CUSTOM1"];
    expect(orderFormats(input)).toEqual([
      "DOCX",
      "PDF",
      "JPG",
      "PNG",
      "CUSTOM2",
      "CUSTOM1",
    ]);
  });

  test("deduplicates and respects custom preference ordering", () => {
    const input = ["PNG", "WEBP", "PNG", "JPG", "AVIF"];
    expect(orderFormats(input, ["WEBP", "PNG"])).toEqual([
      "WEBP",
      "PNG",
      "JPG",
      "AVIF",
    ]);
  });

  test("handles empty input cleanly", () => {
    expect(orderFormats([])).toEqual([]);
  });
});

describe("polar & sector SVG geometry", () => {
  test("places 0 degrees at 12 o'clock and 90 degrees at 3 o'clock", () => {
    const top = polarToCartesian(100, 0);
    expect(top.x).toBeCloseTo(WHEEL_CENTER, 5);
    expect(top.y).toBeCloseTo(WHEEL_CENTER - 100, 5);

    const right = polarToCartesian(100, 90);
    expect(right.x).toBeCloseTo(WHEEL_CENTER + 100, 5);
    expect(right.y).toBeCloseTo(WHEEL_CENTER, 5);
  });

  test("builds chamfered SVG path for counts 1 through 8", () => {
    expect(sectorSvgPath(0, 0)).toBe("");

    for (let count = 1; count <= 8; count++) {
      for (let idx = 0; idx < count; idx++) {
        const d = sectorSvgPath(idx, count);
        expect(d.startsWith("M ")).toBe(true);
        expect(d).toContain(" Q ");
        expect(d).toContain(" A ");
        expect(d).toContain(" L ");
        expect(d.endsWith(" Z")).toBe(true);

        const largeArcTokens = d.match(/A [0-9.]+ [0-9.]+ 0 (\d)/g) ?? [];
        expect(largeArcTokens).toHaveLength(2);
        if (count === 1) {
          expect(largeArcTokens[0]?.endsWith("1")).toBe(true);
        } else {
          expect(largeArcTokens[0]?.endsWith("0")).toBe(true);
        }
      }
    }
  });

  test("positions sector 0 center directly above hub", () => {
    const center = getSectorCenter(0, 8, 124);
    expect(center.x).toBeCloseTo(WHEEL_CENTER, 4);
    expect(center.y).toBeCloseTo(WHEEL_CENTER - 124, 4);
    const bounds = getSectorAngles(0, 8);
    expect(bounds.spanDeg).toBe(45);
    expect(bounds.midDeg).toBe(0);
  });
});

describe("getSectorAtPoint hit-testing & 3° angular hysteresis", () => {
  test("enforces inner dead zone (r < 68px) and outer max radius (r > 210px)", () => {
    expect(getSectorAtPoint(WHEEL_CENTER, WHEEL_CENTER, 8)).toBe(-1);
    const insideDead = polarToCartesian(WHEEL_DEAD_ZONE_RADIUS - 0.5, 0);
    expect(getSectorAtPoint(insideDead.x, insideDead.y, 8)).toBe(-1);

    const onInnerEdge = polarToCartesian(WHEEL_DEAD_ZONE_RADIUS, 0);
    expect(getSectorAtPoint(onInnerEdge.x, onInnerEdge.y, 8)).toBe(0);

    const onOuterEdge = polarToCartesian(WHEEL_OUTER_MAX_RADIUS, 90);
    expect(getSectorAtPoint(onOuterEdge.x, onOuterEdge.y, 8)).toBe(2);

    const pastOuterEdge = polarToCartesian(WHEEL_OUTER_MAX_RADIUS + 0.5, 90);
    expect(getSectorAtPoint(pastOuterEdge.x, pastOuterEdge.y, 8)).toBe(-1);
  });

  test("maps all 8 sector centers accurately without previous selection", () => {
    for (let i = 0; i < 8; i++) {
      const pt = polarToCartesian(120, i * 45);
      expect(getSectorAtPoint(pt.x, pt.y, 8)).toBe(i);
    }
  });

  test("prevents boundary flicker with 3° angular hysteresis in both directions", () => {
    // Boundary between sector 0 (0°) and sector 1 (45°) is at 22.5°.
    // Moving clockwise past 22.5° into 24.5° (within 3° hysteresis) stays on sector 0.
    const cwInsideBand = polarToCartesian(120, 24.5);
    expect(getSectorAtPoint(cwInsideBand.x, cwInsideBand.y, 8)).toBe(1);
    expect(
      getSectorAtPoint(cwInsideBand.x, cwInsideBand.y, 8, { previousIndex: 0 }),
    ).toBe(0);

    // Moving past 22.5° + 3° = 25.5° switches cleanly to sector 1.
    const cwPastBand = polarToCartesian(120, 26.0);
    expect(
      getSectorAtPoint(cwPastBand.x, cwPastBand.y, 8, { previousIndex: 0 }),
    ).toBe(1);

    // Moving counter-clockwise back into 20.5° while on sector 1 stays on sector 1.
    const ccwInsideBand = polarToCartesian(120, 20.5);
    expect(
      getSectorAtPoint(ccwInsideBand.x, ccwInsideBand.y, 8, { previousIndex: 1 }),
    ).toBe(1);

    // Moving past 22.5° - 3° = 19.5° switches back to sector 0.
    const ccwPastBand = polarToCartesian(120, 19.0);
    expect(
      getSectorAtPoint(ccwPastBand.x, ccwPastBand.y, 8, { previousIndex: 1 }),
    ).toBe(0);
  });

  test("applies 3° hysteresis cleanly across the 0° / 360° wraparound boundary", () => {
    // Boundary between sector 7 (315°) and sector 0 (0°) is at 337.5°.
    const slightlyIntoZero = polarToCartesian(120, 339.2);
    expect(
      getSectorAtPoint(slightlyIntoZero.x, slightlyIntoZero.y, 8, {
        previousIndex: 7,
      }),
    ).toBe(7);

    const slightlyIntoSeven = polarToCartesian(120, 335.8);
    expect(
      getSectorAtPoint(slightlyIntoSeven.x, slightlyIntoSeven.y, 8, {
        previousIndex: 0,
      }),
    ).toBe(0);
  });
});

describe("ProgressArcData geometry", () => {
  test("clamps percent bounds and toggles largeArcFlag at 50%", () => {
    const low = progressArcSvgPath(0);
    expect(low).toContain(`A ${WHEEL_PROGRESS_RADIUS} ${WHEEL_PROGRESS_RADIUS} 0 0 1`);

    const mid = progressArcSvgPath(50);
    expect(mid).toContain(`A ${WHEEL_PROGRESS_RADIUS} ${WHEEL_PROGRESS_RADIUS} 0 0 1`);

    const overHalf = progressArcSvgPath(55);
    expect(overHalf).toContain(`A ${WHEEL_PROGRESS_RADIUS} ${WHEEL_PROGRESS_RADIUS} 0 1 1`);

    const capped = progressArcSvgPath(150);
    expect(capped).toBe(progressArcSvgPath(99.9));
  });

  test("aligns progress tip and ray outer endpoint", () => {
    for (const pct of [10, 25, 50, 75, 95]) {
      const tip = progressTipPoint(pct, WHEEL_PROGRESS_RADIUS);
      const ray = progressRaySegment(pct, 60, WHEEL_PROGRESS_RADIUS);
      expect(ray.x2).toBeCloseTo(tip.x, 5);
      expect(ray.y2).toBeCloseTo(tip.y, 5);
    }
  });
});

describe("paginateWheelItems", () => {
  test("renders up to 8 items on a single page without More or Back", () => {
    const zero = paginateWheelItems([], 0);
    expect(zero.totalPages).toBe(1);
    expect(zero.slots).toEqual([]);

    const eight = paginateWheelItems(["A", "B", "C", "D", "E", "F", "G", "H"], 0);
    expect(eight.totalPages).toBe(1);
    expect(eight.slots).toHaveLength(8);
    expect(eight.slots.every((s) => s.kind === "item")).toBe(true);
  });

  test("paginates >8 items into <=8 sector pages with More and Back and preserves all items", () => {
    for (const count of [9, 12, 14, 15, 22]) {
      const source = Array.from({ length: count }, (_, i) => `FMT_${i + 1}`);
      const first = paginateWheelItems(source, 0);
      expect(first.totalPages).toBeGreaterThan(1);

      const collected: string[] = [];
      for (let p = 0; p < first.totalPages; p++) {
        const pageData = paginateWheelItems(source, p);
        expect(pageData.page).toBe(p);
        expect(pageData.slots.length).toBeLessThanOrEqual(8);

        if (p === 0) {
          expect(pageData.slots.some((s) => s.kind === "more")).toBe(true);
          expect(pageData.slots.some((s) => s.kind === "back")).toBe(false);
        } else if (p === first.totalPages - 1) {
          expect(pageData.slots.some((s) => s.kind === "back")).toBe(true);
          expect(pageData.slots.some((s) => s.kind === "more")).toBe(false);
        } else {
          expect(pageData.slots.some((s) => s.kind === "back")).toBe(true);
          expect(pageData.slots.some((s) => s.kind === "more")).toBe(true);
        }

        for (const slot of pageData.slots) {
          if (slot.kind === "item") collected.push(slot.item);
        }
      }

      expect(collected).toEqual(source);
    }
  });

  test("clamps out-of-range page numbers safely", () => {
    const source = Array.from({ length: 12 }, (_, i) => `F${i}`);
    expect(paginateWheelItems(source, -3).page).toBe(0);
    expect(paginateWheelItems(source, 99).page).toBe(1);
  });
});

describe("router and IPC policy/escaping helpers", () => {
  test("resolves hash routes and defaults to settings", () => {
    expect(resolveRoute("")).toBe("settings");
    expect(resolveRoute("#")).toBe("settings");
    expect(resolveRoute("#settings")).toBe("settings");
    expect(resolveRoute("#wheel")).toBe("wheel");
    expect(resolveRoute("#/wheel?demo=many")).toBe("wheel");
    expect(resolveRoute("#batch")).toBe("batch");
    expect(resolveRoute("#onboarding")).toBe("onboarding");
    expect(resolveRoute("#unknown")).toBe("settings");
  });

  test("inspectFilesLocally enforces batch intersection, ZIP/UNZIP, REMOVE_BG, and OPTIMIZE_PDF", () => {
    const imgs = inspectFilesLocally(["/tmp/a.png", "/tmp/b.webp"]);
    expect(imgs.compatibleFormats).toContain("JPG");
    expect(imgs.compatibleFormats).toContain("REMOVE_BG");
    expect(imgs.compatibleFormats).not.toContain("PNG");
    expect(imgs.compatibleFormats).not.toContain("WEBP");
    expect(imgs.batchActions).toEqual(["COMBINE_PDF", "ZIP"]);

    const pdfs = inspectFilesLocally(["/tmp/a.pdf", "/tmp/b.pdf"]);
    expect(pdfs.compatibleFormats).toContain("OPTIMIZE_PDF");
    expect(pdfs.batchActions).toEqual(["MERGE_PDF", "ZIP"]);

    const mixed = inspectFilesLocally(["/tmp/a.png", "/tmp/b.mp3"]);
    expect(mixed.compatibleFormats).toEqual([]);
    expect(mixed.batchActions).toEqual(["ZIP"]);
    expect(mixed.availabilityMessage).toBe("");

    const singleZip = inspectFilesLocally(["/tmp/bundle.zip"]);
    expect(singleZip.family).toBe("archive");
    expect(singleZip.compatibleFormats).toEqual(["UNZIP"]);
    expect(singleZip.batchActions).toEqual([]);

    const singleOther = inspectFilesLocally(["/tmp/notes.bin"]);
    expect(singleOther.compatibleFormats).toEqual(["ZIP"]);
  });

  test("escapeHtml escapes special HTML characters safely", () => {
    expect(escapeHtml('<script>"test"&\'x\'</script>')).toBe(
      "&lt;script&gt;&quot;test&quot;&amp;&#39;x&#39;&lt;/script&gt;",
    );
  });

  test("getSectorAtPoint returns -1 safely on NaN/Infinity coordinates", () => {
    expect(getSectorAtPoint(Number.NaN, 120, 8)).toBe(-1);
    expect(getSectorAtPoint(220, Number.POSITIVE_INFINITY, 8)).toBe(-1);
  });

  test("mapBrowserKeyToToken resolves macOS Option-modified keys via KeyboardEvent.code", () => {
    expect(mapBrowserKeyToToken({ key: "å", code: "KeyA" })).toBe("A");
    expect(mapBrowserKeyToToken({ key: "Í", code: "KeyS" })).toBe("S");
    expect(mapBrowserKeyToToken({ key: "¡", code: "Digit1" })).toBe("1");
    expect(mapBrowserKeyToToken({ key: "\u00a0", code: "Space" })).toBe("Space");
    expect(mapBrowserKeyToToken({ key: "ArrowDown", code: "ArrowDown" })).toBe("Down");
    expect(mapBrowserKeyToToken({ key: "F8", code: "F8" })).toBe("F8");
    expect(mapBrowserKeyToToken({ key: "Enter", code: "Enter" })).toBeNull();
  });

  test("getThumbnail mock handles non-ASCII extensions without btoa failures", async () => {
    const res = await getThumbnail("/tmp/file.résumé", 128);
    expect(res.ok).toBe(true);
    expect(res.pngBase64).toBeTruthy();
  });

  test("storeBatchHandoff and consumeBatchHandoff consume stored state once", () => {
    const store = new Map<string, string>();
    (globalThis as Record<string, unknown>).localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => { store.set(k, v); },
      removeItem: (k: string) => { store.delete(k); },
    };

    storeBatchHandoff(["/tmp/a.png"], [
      { source: "/tmp/a.png", format: "JPG", outputs: [], error: "oops" },
    ]);
    const first = consumeBatchHandoff();
    expect(first.files).toEqual(["/tmp/a.png"]);
    expect(first.results).toHaveLength(1);

    const second = consumeBatchHandoff();
    expect(second.files).toEqual([]);
    expect(second.results).toBeNull();
  });
});

describe("preferred format reorder helpers", () => {
  test("reorderFormatList moves items earlier and later without mutating input", () => {
    const initial = ["DOCX", "PDF", "MD", "EPUB"];
    const movedLeft = reorderFormatList(initial, 2, -1);
    expect(movedLeft).toEqual(["DOCX", "MD", "PDF", "EPUB"]);
    expect(initial).toEqual(["DOCX", "PDF", "MD", "EPUB"]);

    const movedRight = reorderFormatList(initial, 1, 1);
    expect(movedRight).toEqual(["DOCX", "MD", "PDF", "EPUB"]);
  });

  test("reorderFormatList and moveFormatToIndex clamp boundary moves cleanly", () => {
    const initial = ["PNG", "JPG", "WEBP"];
    expect(reorderFormatList(initial, 0, -1)).toEqual(["PNG", "JPG", "WEBP"]);
    expect(reorderFormatList(initial, 2, 1)).toEqual(["PNG", "JPG", "WEBP"]);
    expect(reorderFormatList(initial, -1, 1)).toEqual(["PNG", "JPG", "WEBP"]);
    expect(moveFormatToIndex(initial, 2, 0)).toEqual(["WEBP", "PNG", "JPG"]);
    expect(moveFormatToIndex(initial, 0, 99)).toEqual(["JPG", "WEBP", "PNG"]);
    expect(moveFormatToIndex(["ONLY"], 0, 0)).toEqual(["ONLY"]);
  });
});

describe("VoiceOver live announcement formatters", () => {
  test("formatSectorAriaLabel formats action, unavailable, and pagination slots", () => {
    expect(
      formatSectorAriaLabel(
        {
          kind: "item",
          globalIndex: 1,
          item: { subtitle: "Convert to PNG", enabled: true },
        },
        1,
        8,
      ),
    ).toBe("Convert to PNG, sector 2 of 8");

    expect(
      formatSectorAriaLabel(
        {
          kind: "item",
          globalIndex: 0,
          item: {
            subtitle: "Convert to MKV",
            enabled: false,
            disabledReason: "Install FFmpeg",
          },
        },
        0,
        6,
      ),
    ).toBe("Convert to MKV, sector 1 of 6, unavailable (Install FFmpeg)");

    expect(
      formatSectorAriaLabel({ kind: "more", targetPage: 1 }, 7, 8),
    ).toBe("More options, page 2, sector 8 of 8");

    expect(
      formatSectorAriaLabel({ kind: "back", targetPage: 0 }, 7, 8),
    ).toBe("Previous options, page 1, sector 8 of 8");
  });

  test("open mode, page flip, progress milestone, and completion announcements", () => {
    expect(
      formatWheelOpenAnnouncement({
        mode: "convert",
        fileCount: 2,
        optionCount: 10,
        page: 0,
        totalPages: 2,
      }),
    ).toBe("Convert wheel opened for 2 files, 10 options, page 1 of 2.");

    expect(
      formatWheelOpenAnnouncement({
        mode: "convert",
        fileCount: 0,
        optionCount: 0,
      }),
    ).toBe("Convert wheel opened. No files selected yet.");

    expect(
      formatWheelOpenAnnouncement({
        mode: "new",
        fileCount: 0,
        optionCount: 7,
      }),
    ).toBe("Create New File wheel opened, 7 options.");

    expect(formatPageFlipAnnouncement(1, 3)).toBe("Page 2 of 3");

    expect(getProgressMilestoneBucket(0)).toBe(0);
    expect(getProgressMilestoneBucket(24)).toBe(0);
    expect(getProgressMilestoneBucket(25)).toBe(25);
    expect(getProgressMilestoneBucket(55)).toBe(50);
    expect(getProgressMilestoneBucket(99)).toBe(75);
    expect(formatProgressMilestoneAnnouncement("epub", 0)).toBe("Converting to EPUB");
    expect(formatProgressMilestoneAnnouncement("epub", 50)).toBe(
      "Converting to EPUB, 50% complete",
    );

    expect(
      formatCompletionAnnouncement({
        ok: true,
        succeeded: 1,
        outputFormat: "epub",
      }),
    ).toBe("Converted 1 file to EPUB.");
    expect(
      formatCompletionAnnouncement({
        ok: false,
        canceled: true,
      }),
    ).toBe("Conversion canceled.");
    expect(
      formatCompletionAnnouncement({
        ok: false,
        failed: 1,
        errorMessage: "Pandoc not found",
      }),
    ).toBe("Conversion failed: Pandoc not found");
    expect(
      formatCompletionAnnouncement({
        ok: true,
        createdLabel: "notes.md",
      }),
    ).toBe("Created notes.md.");
  });
});
