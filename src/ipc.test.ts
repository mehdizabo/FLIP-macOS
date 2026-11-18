import { describe, expect, test } from "bun:test";
import {
  cancelConversion,
  checkForUpdates,
  consumeBatchHandoff,
  consumeSettingsNavigation,
  copyDiagnosticsToClipboard,
  defaultPreferences,
  dismissWheel,
  DOC_NATIVE_EXTS,
  DOC_PANDOC_ONLY_EXTS,
  events,
  executeConversion,
  executeCreateNew,
  formatError,
  getDiagnosticsReport,
  getEngineStatus,
  getNewFileKinds,
  getPermissionsStatus,
  getPreferences,
  getQuickActionsStatus,
  getRecentConversions,
  getWheelState,
  inspectFiles,
  inspectFilesLocally,
  installFinderQuickActions,
  openBatchWithFiles,
  openReleaseUrl,
  openWheelMode,
  requestPermission,
  savePreferences,
  switchWheelMode,
} from "./ipc";

describe("Frontend IPC surface & contract alignment", () => {
  test("defaultPreferences matches Rust AppPreferences schema fields", () => {
    const prefs = defaultPreferences();
    expect(prefs.version).toBe(1);
    expect(prefs.soundEnabled).toBe(true);
    expect(prefs.hapticsEnabled).toBe(true);
    expect(prefs.notificationsEnabled).toBe(true);
    expect(prefs.dragTrigger).toBe("Down");
    expect(prefs.idleShortcut).toBe("Option+Shift+Space");
    expect(prefs.mouseTriggerButton).toBeNull();
    expect(prefs.autoRenameOnCreate).toBe(true);
    expect(prefs.openAfterCreating).toBe(false);
    expect(prefs.launchAtLogin).toBe(false);
    expect(prefs.outputSubfolder).toBeNull();
    expect(prefs.fallbackFolder).toBeNull();
    expect(prefs.imageQuality).toBe(92);
    expect(prefs.preserveExif).toBe(true);
    expect(prefs.stripGpsExif).toBe(false);
    expect(prefs.pdfDpi).toBe(144);
    expect(prefs.pdfPageSize).toBe("a4");
    expect(prefs.ffmpegPath).toBeNull();
    expect(prefs.pandocPath).toBeNull();
    expect(prefs.onboardingCompleted).toBe(false);
    expect(Array.isArray(prefs.newFileVisibleKinds)).toBe(true);
    expect(Array.isArray(prefs.recentConversions)).toBe(true);
    expect(Object.keys(prefs.preferredFormats)).toEqual([
      "image",
      "pdf",
      "document",
      "data",
      "audio",
      "video",
    ]);
  });

  test("mock IPC commands emit expected payloads and events", async () => {
    const g = globalThis as Record<string, unknown>;
    const prevStorage = g.localStorage;
    const prevWindow = g.window;
    const store = new Map<string, string>();
    g.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => {
        store.set(k, v);
      },
      removeItem: (k: string) => {
        store.delete(k);
      },
    };
    g.window = {
      location: { hash: "#wheel" },
    };

    try {
      const wheelStates: string[] = [];
      const progressPct: number[] = [];
      const escapes: unknown[] = [];

      const u1 = await events.onWheelState((p) => wheelStates.push(p.mode));
      const u2 = await events.onConversionProgress((p) => progressPct.push(p.percent));
      const u3 = await events.onWheelEscape((p) => escapes.push(p));

      const ws = await openWheelMode("convert", ["/tmp/a.png", "/tmp/b.png"]);
      expect(ws.mode).toBe("convert");
      expect(ws.batchActions).toEqual(["COMBINE_PDF", "ZIP"]);
      expect(wheelStates).toContain("convert");

      const switched = await switchWheelMode("new");
      expect(switched.mode).toBe("new");

      const cur = await getWheelState();
      expect(cur.mode).toBe("new");

      const inspected = await inspectFiles(["/tmp/a.pdf", "/tmp/b.pdf"]);
      expect(inspected.batchActions).toEqual(["MERGE_PDF", "ZIP"]);
      expect(inspected.compatibleFormats).toContain("OPTIMIZE_PDF");
      expect(inspected.family).toBe("pdf");

      const zipInspect = await inspectFiles(["/tmp/archive.zip"]);
      expect(zipInspect.family).toBe("archive");
      expect(zipInspect.compatibleFormats).toEqual(["UNZIP"]);
      expect(zipInspect.batchActions).toEqual([]);

      const bgConv = await executeConversion(["/tmp/photo.jpg"], ["REMOVE_BG"]);
      expect(bgConv.success).toBe(true);
      expect(bgConv.results[0]?.outputs[0]).toBe("/tmp/photo.png");

      const optConv = await executeConversion(["/tmp/report.pdf"], ["OPTIMIZE_PDF"]);
      expect(optConv.success).toBe(true);
      expect(optConv.results[0]?.outputs[0]).toBe("/tmp/report 2.pdf");

      const unzipConv = await executeConversion(["/tmp/archive.zip"], ["UNZIP"]);
      expect(unzipConv.success).toBe(true);
      expect(unzipConv.results[0]?.outputs[0]).toBe("/tmp/archive");

      const zipConv = await executeConversion(["/tmp/a.png", "/tmp/b.png"], ["ZIP"]);
      expect(zipConv.success).toBe(true);
      expect(zipConv.results[0]?.outputs[0]).toBe("/tmp/a.zip");

      const conv = await executeConversion(["/tmp/photo.heic"], ["PNG"]);
      expect(conv.success).toBe(true);
      expect(conv.results[0]?.outputs[0]).toBe("/tmp/photo.png");
      expect(progressPct.length).toBeGreaterThan(0);

      await cancelConversion();

      const created = await executeCreateNew("md", "/tmp");
      expect(created.path).toBe("/tmp/Untitled.md");
      expect(created.renameTriggered).toBe(true);

      const kinds = await getNewFileKinds();
      expect(kinds.kinds.length).toBeGreaterThanOrEqual(17);
      expect(kinds.clipboard?.available).toBe(true);

      const eng = await getEngineStatus();
      expect(eng.nativeFrameworks?.imageIO).toBe(true);

      const p0 = await getPermissionsStatus();
      expect(typeof p0.accessibility).toBe("boolean");
      const p1 = await requestPermission("open_input_monitoring");
      expect(p1.inputMonitoring).toBe(true);

      const qa0 = await getQuickActionsStatus();
      expect(qa0.convertInstalled).toBe(false);
      expect(qa0.newFileInstalled).toBe(false);
      const qa1 = await installFinderQuickActions(true);
      expect(qa1.convertInstalled).toBe(true);
      expect(qa1.newFileInstalled).toBe(true);
      expect(qa1.cliSymlinkPath).toBe("/Users/demo/.local/bin/flip");

      const prefs = await getPreferences();
      prefs.dragTrigger = "Space";
      prefs.preferredFormats.document = ["EPUB", "MD"];
      const saved = await savePreferences(prefs);
      expect(saved.dragTrigger).toBe("Space");
      const reloaded = await getPreferences();
      expect(reloaded.preferredFormats.document.slice(0, 2)).toEqual(["EPUB", "MD"]);
      expect(reloaded.preferredFormats.document).toContain("HEIC");
      expect((await getRecentConversions()).length).toBeGreaterThan(0);

      await dismissWheel("test-dismiss");
      expect(escapes.length).toBe(1);

      await openBatchWithFiles(["/tmp/x.png"]);
      const handoff = consumeBatchHandoff();
      expect(handoff.files).toEqual(["/tmp/x.png"]);
      expect(handoff.results).toBeNull();

      u1();
      u2();
      u3();
      await openBatchWithFiles(["/tmp/x.png"], [
        { source: "/tmp/old.png", format: "JPG", outputs: [], error: "prev" },
      ]);
      const handoffWithRes = consumeBatchHandoff();
      expect(handoffWithRes.files).toEqual(["/tmp/x.png"]);
      expect(handoffWithRes.results).toHaveLength(1);

      await openBatchWithFiles(["/tmp/y.png"]);
      const handoffCleared = consumeBatchHandoff();
      expect(handoffCleared.files).toEqual(["/tmp/y.png"]);
      expect(handoffCleared.results).toBeNull();

      const navEvents: Array<{ tab?: string; checkUpdates?: boolean }> = [];
      const uNav = await events.onSettingsNavigate((p) => navEvents.push(p));
      expect(typeof uNav).toBe("function");
      uNav();
      const uTmpl = await events.onTemplatesChanged(() => {});
      expect(typeof uTmpl).toBe("function");
      uTmpl();

      const upd = await checkForUpdates();
      expect(upd.currentVersion).toBe("0.1.0");
      expect(upd.latestTag.startsWith("v")).toBe(true);
      expect(upd.htmlUrl.startsWith("https://github.com/")).toBe(true);
      expect(typeof upd.updateAvailable).toBe("boolean");

      expect(await consumeSettingsNavigation()).toBeNull();
      let rejectedUrl = false;
      try {
        await openReleaseUrl("https://evil.example/payload");
      } catch (err) {
        rejectedUrl = formatError(err).includes("https://github.com/");
      }
      expect(rejectedUrl).toBe(true);
      let openedReleaseUrl = "";
      (g.window as Record<string, unknown>).open = (u: string) => {
        openedReleaseUrl = u;
        return null;
      };
      await openReleaseUrl("https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1");
      expect(openedReleaseUrl).toBe("https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1");

      const diag = await getDiagnosticsReport();
      expect(diag).toContain("# FLIP Diagnostics Report");
      expect(diag).toContain("~/Desktop/sample.heic");

      const prevNav = g.navigator;
      let copiedPayload = "";
      g.navigator = {
        clipboard: {
          writeText: async (text: string) => {
            copiedPayload = text;
          },
        },
      };
      const copyOk = await copyDiagnosticsToClipboard();
      expect(copyOk.ok).toBe(true);
      expect(copyOk.method).toBe("clipboard");
      expect(copiedPayload).toContain("# FLIP Diagnostics Report");

      g.navigator = {
        clipboard: {
          writeText: async () => {
            throw new Error("denied");
          },
        },
      };
      const copyFallback = await copyDiagnosticsToClipboard();
      expect(copyFallback.ok).toBe(false);
      expect(copyFallback.method).toBe("manual");
      expect(copyFallback.report).toContain("# FLIP Diagnostics Report");
      g.navigator = prevNav;
    } finally {
      g.localStorage = prevStorage;
      g.window = prevWindow;
    }
  });

  test("native EPUB 3 policy exposes EPUB without Pandoc and keeps FB2/RST Pandoc-gated", () => {
    expect(DOC_NATIVE_EXTS.has("epub")).toBe(true);
    expect(DOC_PANDOC_ONLY_EXTS.has("epub")).toBe(false);
    expect(DOC_PANDOC_ONLY_EXTS.has("fb2")).toBe(true);
    expect(DOC_PANDOC_ONLY_EXTS.has("rst")).toBe(true);

    const epubInputNoPandoc = inspectFilesLocally(["/tmp/book.epub"], { pandoc: false });
    expect(epubInputNoPandoc.compatibleFormats).toContain("TXT");
    expect(epubInputNoPandoc.compatibleFormats).toContain("MD");
    expect(epubInputNoPandoc.compatibleFormats).toContain("PDF");
    expect(epubInputNoPandoc.compatibleFormats).toContain("HEIC");
    expect(epubInputNoPandoc.compatibleFormats).not.toContain("FB2");
    expect(epubInputNoPandoc.compatibleFormats).not.toContain("RST");

    const mdInputNoPandoc = inspectFilesLocally(["/tmp/readme.md"], { pandoc: false });
    expect(mdInputNoPandoc.compatibleFormats).toContain("EPUB");
    expect(mdInputNoPandoc.compatibleFormats).toContain("HEIC");
    expect(mdInputNoPandoc.compatibleFormats).not.toContain("FB2");

    const txtInputNoPandoc = inspectFilesLocally(["/tmp/note.txt"], { pandoc: false });
    expect(txtInputNoPandoc.compatibleFormats).toContain("EPUB");

    const pdfInputNoPandoc = inspectFilesLocally(["/tmp/doc.pdf"], { pandoc: false });
    expect(pdfInputNoPandoc.compatibleFormats).toContain("EPUB");
    expect(pdfInputNoPandoc.compatibleFormats).not.toContain("FB2");
    expect(pdfInputNoPandoc.compatibleFormats).not.toContain("RST");

    const fb2NoPandoc = inspectFilesLocally(["/tmp/book.fb2"], { pandoc: false });
    expect(fb2NoPandoc.compatibleFormats).toEqual(["ZIP"]);

    const fb2WithPandoc = inspectFilesLocally(["/tmp/book.fb2"], { pandoc: true });
    expect(fb2WithPandoc.compatibleFormats).toContain("EPUB");
    expect(fb2WithPandoc.compatibleFormats).toContain("PDF");
    expect(fb2WithPandoc.compatibleFormats).toContain("RST");

    expect(defaultPreferences().preferredFormats.document).toContain("HEIC");
    expect(defaultPreferences().preferredFormats.document).toContain("EPUB");
    expect(defaultPreferences().preferredFormats.pdf).toContain("TIFF");
  });
});
