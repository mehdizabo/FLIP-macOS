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
  uninstallFinderQuickActions,
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
    expect(prefs.language).toBe("en");
    expect(prefs.preferredFormats.audio).toContain("CAF");
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
      expect(ws.batchActions).toEqual(["COMBINE_PDF", "ANIMATE_GIF", "ZIP"]);
      expect(wheelStates).toContain("convert");

      const animConv = await executeConversion(["/tmp/a.png", "/tmp/b.png"], ["ANIMATE_GIF"]);
      expect(animConv.success).toBe(true);
      expect(animConv.results[0]?.outputs[0]).toBe("/tmp/a.gif");

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
      expect(kinds.kinds.length).toBeGreaterThanOrEqual(21);
      for (const id of ["yaml", "toml", "xml", "pdf"]) {
        expect(kinds.kinds.some((k) => k.id === id)).toBe(true);
      }
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
      const qa2 = await uninstallFinderQuickActions(true);
      expect(qa2.convertInstalled).toBe(false);
      expect(qa2.newFileInstalled).toBe(false);
      expect(qa2.cliSymlinkPath).toBeNull();

      const prefs = await getPreferences();
      const prefsUpdates: string[] = [];
      const uPrefs = await events.onPrefsChanged((p) => prefsUpdates.push(p.language));
      prefs.dragTrigger = "Space";
      prefs.language = "it";
      prefs.preferredFormats.document = ["EPUB", "MD"];
      const saved = await savePreferences(prefs);
      expect(saved.dragTrigger).toBe("Space");
      expect(saved.language).toBe("it");
      expect(prefsUpdates).toEqual(["it"]);
      uPrefs();
      const reloaded = await getPreferences();
      expect(reloaded.language).toBe("it");
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

  test("native EPUB 3, FB2, and RST document policy works with zero Pandoc", () => {
    expect(DOC_NATIVE_EXTS.has("epub")).toBe(true);
    expect(DOC_NATIVE_EXTS.has("fb2")).toBe(true);
    expect(DOC_NATIVE_EXTS.has("rst")).toBe(true);
    expect(DOC_PANDOC_ONLY_EXTS.has("epub")).toBe(false);
    expect(DOC_PANDOC_ONLY_EXTS.has("fb2")).toBe(false);
    expect(DOC_PANDOC_ONLY_EXTS.has("rst")).toBe(false);

    const epubInputNoPandoc = inspectFilesLocally(["/tmp/book.epub"], { pandoc: false });
    expect(epubInputNoPandoc.compatibleFormats).toContain("TXT");
    expect(epubInputNoPandoc.compatibleFormats).toContain("MD");
    expect(epubInputNoPandoc.compatibleFormats).toContain("PDF");
    expect(epubInputNoPandoc.compatibleFormats).toContain("HEIC");
    expect(epubInputNoPandoc.compatibleFormats).toContain("FB2");
    expect(epubInputNoPandoc.compatibleFormats).toContain("RST");

    const mdInputNoPandoc = inspectFilesLocally(["/tmp/readme.md"], { pandoc: false });
    expect(mdInputNoPandoc.compatibleFormats).toContain("EPUB");
    expect(mdInputNoPandoc.compatibleFormats).toContain("HEIC");
    expect(mdInputNoPandoc.compatibleFormats).toContain("FB2");
    expect(mdInputNoPandoc.compatibleFormats).toContain("RST");

    const txtInputNoPandoc = inspectFilesLocally(["/tmp/note.txt"], { pandoc: false });
    expect(txtInputNoPandoc.compatibleFormats).toContain("EPUB");
    expect(txtInputNoPandoc.compatibleFormats).toContain("FB2");
    expect(txtInputNoPandoc.compatibleFormats).toContain("RST");

    const pdfInputNoPandoc = inspectFilesLocally(["/tmp/doc.pdf"], { pandoc: false });
    expect(pdfInputNoPandoc.compatibleFormats).toContain("EPUB");
    expect(pdfInputNoPandoc.compatibleFormats).toContain("FB2");
    expect(pdfInputNoPandoc.compatibleFormats).toContain("RST");

    const fb2NoPandoc = inspectFilesLocally(["/tmp/book.fb2"], { pandoc: false });
    expect(fb2NoPandoc.compatibleFormats).toContain("EPUB");
    expect(fb2NoPandoc.compatibleFormats).toContain("PDF");
    expect(fb2NoPandoc.compatibleFormats).toContain("RST");
    expect(fb2NoPandoc.compatibleFormats).toContain("TXT");

    const rstNoPandoc = inspectFilesLocally(["/tmp/notes.rst"], { pandoc: false });
    expect(rstNoPandoc.compatibleFormats).toContain("FB2");
    expect(rstNoPandoc.compatibleFormats).toContain("HTML");
    expect(rstNoPandoc.compatibleFormats).toContain("PDF");
    expect(rstNoPandoc.compatibleFormats).toContain("EPUB");

    expect(defaultPreferences().preferredFormats.document).toContain("HEIC");
    expect(defaultPreferences().preferredFormats.document).toContain("EPUB");
    expect(defaultPreferences().preferredFormats.document).toContain("FB2");
    expect(defaultPreferences().preferredFormats.document).toContain("RST");
    expect(defaultPreferences().preferredFormats.pdf).toContain("TIFF");
    expect(defaultPreferences().preferredFormats.pdf.slice(0, 7)).toContain("OPTIMIZE_PDF");
    expect(defaultPreferences().preferredFormats.image.slice(0, 7)).toContain("REMOVE_BG");
  });

  test("native zero-FFmpeg audio policy supports CAF and FLAC while keeping MP3/OGG/OPUS FFmpeg-gated", () => {
    const wavNoFf = inspectFilesLocally(["/tmp/tone.wav"], { ffmpeg: false });
    expect(wavNoFf.family).toBe("audio");
    expect(wavNoFf.compatibleFormats).toContain("CAF");
    expect(wavNoFf.compatibleFormats).toContain("FLAC");
    expect(wavNoFf.compatibleFormats).toContain("AIFF");
    expect(wavNoFf.compatibleFormats).toContain("M4A");
    expect(wavNoFf.compatibleFormats).toContain("ALAC");
    expect(wavNoFf.compatibleFormats).not.toContain("WAV");
    expect(wavNoFf.compatibleFormats).not.toContain("MP3");
    expect(wavNoFf.compatibleFormats).not.toContain("OGG");
    expect(wavNoFf.compatibleFormats).not.toContain("OPUS");

    const cafNoFf = inspectFilesLocally(["/tmp/sound.caf"], { ffmpeg: false });
    expect(cafNoFf.compatibleFormats).toContain("WAV");
    expect(cafNoFf.compatibleFormats).toContain("FLAC");
    expect(cafNoFf.compatibleFormats).not.toContain("CAF");

    const flacNoFf = inspectFilesLocally(["/tmp/lossless.flac"], { ffmpeg: false });
    expect(flacNoFf.compatibleFormats).toContain("CAF");
    expect(flacNoFf.compatibleFormats).toContain("WAV");
    expect(flacNoFf.compatibleFormats).not.toContain("FLAC");

    const aacNoFf = inspectFilesLocally(["/tmp/voice.aac"], { ffmpeg: false });
    expect(aacNoFf.compatibleFormats).toContain("CAF");
    expect(aacNoFf.compatibleFormats).toContain("FLAC");

    const oggNoFf = inspectFilesLocally(["/tmp/podcast.ogg"], { ffmpeg: false });
    expect(oggNoFf.compatibleFormats).toEqual(["ZIP"]);

    const opusWithFf = inspectFilesLocally(["/tmp/podcast.opus"], { ffmpeg: true });
    expect(opusWithFf.compatibleFormats).toContain("CAF");
    expect(opusWithFf.compatibleFormats).toContain("MP3");
  });

  test("SVG input, ICO output, and ANIMATE_GIF image policy alignment", () => {
    const svgRes = inspectFilesLocally(["/tmp/logo.svg"]);
    expect(svgRes.family).toBe("image");
    expect(svgRes.compatibleFormats).not.toContain("SVG");
    for (const target of [
      "PNG",
      "JPG",
      "WEBP",
      "HEIC",
      "PDF",
      "BMP",
      "TIFF",
      "GIF",
      "ICO",
      "ICNS",
      "TXT",
      "REMOVE_BG",
      "ZIP",
    ]) {
      expect(svgRes.compatibleFormats).toContain(target);
    }

    const icoRes = inspectFilesLocally(["/tmp/favicon.ico"]);
    expect(icoRes.family).toBe("image");
    expect(icoRes.compatibleFormats).not.toContain("ICO");
    expect(icoRes.compatibleFormats).toContain("PNG");

    const multiSvgPng = inspectFilesLocally(["/tmp/a.svg", "/tmp/b.png"]);
    expect(multiSvgPng.batchActions).toEqual(["COMBINE_PDF", "ANIMATE_GIF", "ZIP"]);

    expect(defaultPreferences().preferredFormats.image).toContain("ICO");
  });

  test("bidirectional XLSX structured data policy alignment", () => {
    const csvRes = inspectFilesLocally(["/tmp/table.csv"]);
    expect(csvRes.family).toBe("data");
    expect(csvRes.compatibleFormats).toContain("XLSX");

    const jsonRes = inspectFilesLocally(["/tmp/config.json"]);
    expect(jsonRes.family).toBe("data");
    expect(jsonRes.compatibleFormats).toContain("XLSX");

    const xlsxRes = inspectFilesLocally(["/tmp/sheet.xlsx"]);
    expect(xlsxRes.family).toBe("data");
    expect(xlsxRes.compatibleFormats).not.toContain("XLSX");
    for (const target of ["JSON", "YAML", "TOML", "XML", "CSV", "TXT", "HTML", "MD", "PDF", "ZIP"]) {
      expect(xlsxRes.compatibleFormats).toContain(target);
    }

    expect(defaultPreferences().preferredFormats.data).toContain("XLSX");
    expect(defaultPreferences().preferredFormats.data.slice(0, 7)).toContain("XLSX");
  });

  test("getEngineStatus keeps whisper and whisperModel from the mock and from raw payloads", async () => {
    const g = globalThis as Record<string, unknown>;
    const prevWindow = g.window;
    const prevIsTauri = g.isTauri;
    try {
      delete g.isTauri;
      g.window = { location: { hash: "" } };
      const mock = await getEngineStatus();
      expect(mock.whisper).toBe("/opt/homebrew/bin/whisper-cli");
      expect(mock.whisperModel).toContain("ggml-small.bin");

      g.isTauri = true;
      g.window = {
        __TAURI_INTERNALS__: {
          invoke: async (cmd: string) => {
            expect(cmd).toBe("get_engine_status");
            return {
              tools: {
                ffmpeg: "/usr/local/bin/ffmpeg",
                ffprobe: null,
                pandoc: null,
                whisper: "/usr/local/bin/whisper-cli",
                whisperModel: "/Users/demo/Library/Application Support/FLIP/models/ggml-small.bin",
                bridge: "/opt/flip-macos-bridge",
                pages: false,
                numbers: false,
                keynote: false,
              },
            };
          },
        },
      };
      const nested = await getEngineStatus();
      expect(nested.whisper).toBe("/usr/local/bin/whisper-cli");
      expect(nested.whisperModel).toBe(
        "/Users/demo/Library/Application Support/FLIP/models/ggml-small.bin",
      );

      g.window = {
        __TAURI_INTERNALS__: {
          invoke: async () => ({
            ffmpeg: null,
            ffprobe: null,
            pandoc: null,
            whisper: null,
            whisperModel: "/models/ggml-small.bin",
            bridge: null,
            pages: true,
            numbers: false,
            keynote: false,
          }),
        },
      };
      const flat = await getEngineStatus();
      expect(flat.whisper).toBeNull();
      expect(flat.whisperModel).toBe("/models/ggml-small.bin");
      expect(flat.pages).toBe(true);
    } finally {
      g.window = prevWindow;
      if (prevIsTauri === undefined) delete g.isTauri;
      else g.isTauri = prevIsTauri;
    }
  });
});
