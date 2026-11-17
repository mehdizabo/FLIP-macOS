import { describe, expect, test } from "bun:test";
import {
  cancelConversion,
  consumeBatchHandoff,
  defaultPreferences,
  dismissWheel,
  events,
  executeConversion,
  executeCreateNew,
  getEngineStatus,
  getNewFileKinds,
  getPermissionsStatus,
  getPreferences,
  getRecentConversions,
  getWheelState,
  inspectFiles,
  openBatchWithFiles,
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
      expect(ws.batchActions).toEqual(["COMBINE_PDF"]);
      expect(wheelStates).toContain("convert");

      const switched = await switchWheelMode("new");
      expect(switched.mode).toBe("new");

      const cur = await getWheelState();
      expect(cur.mode).toBe("new");

      const inspected = await inspectFiles(["/tmp/a.pdf", "/tmp/b.pdf"]);
      expect(inspected.batchActions).toEqual(["MERGE_PDF"]);
      expect(inspected.family).toBe("pdf");

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

      const prefs = await getPreferences();
      prefs.dragTrigger = "Space";
      const saved = await savePreferences(prefs);
      expect(saved.dragTrigger).toBe("Space");
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
    } finally {
      g.localStorage = prevStorage;
      g.window = prevWindow;
    }
  });
});
