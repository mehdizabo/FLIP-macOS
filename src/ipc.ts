import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { normalizeLanguage } from "./i18n";
import type {
  AppPreferences,
  ConversionCompletePayload,
  ConversionProgressPayload,
  ConversionResult,
  CreateNewResult,
  DetectedTools,
  EngineStatus,
  FeedbackKind,
  FormatFamily,
  InspectFilesResult,
  NewFileKindInfo,
  NewFileKindsResponse,
  PermissionRequestKind,
  PermissionsStatus,
  QuickActionsStatus,
  RecentConversion,
  SettingsNavigatePayload,
  ThumbnailResult,
  UpdateCheckResult,
  UserTemplateInfo,
  WheelDragFilesPayload,
  WheelMode,
  WheelPointerMovePayload,
  WheelPointerReleasePayload,
  WheelStatePayload,
} from "./types";

export const MOCK_PREFS_STORAGE_KEY = "flip:mockPreferences";
export const BATCH_HANDOFF_STORAGE_KEY = "flip:batchPendingFiles";
export const BATCH_RESULTS_STORAGE_KEY = "flip:batchLastResults";

export function isTauriRuntime(): boolean {
  try {
    return isTauri();
  } catch {
    return false;
  }
}

export function escapeHtml(raw: string): string {
  return raw
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function basename(filePath: string): string {
  const cleaned = filePath.replace(/\/+$/, "");
  const idx = cleaned.lastIndexOf("/");
  if (idx >= 0) return cleaned.slice(idx + 1) || cleaned;
  const winIdx = cleaned.lastIndexOf("\\");
  return winIdx >= 0 ? cleaned.slice(winIdx + 1) || cleaned : cleaned;
}

export function dirname(filePath: string): string {
  const cleaned = filePath.replace(/\/+$/, "");
  const idx = cleaned.lastIndexOf("/");
  if (idx > 0) return cleaned.slice(0, idx);
  return "/";
}

export function extLower(filePath: string): string {
  const name = basename(filePath);
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "";
  return name.slice(dot + 1).toLowerCase();
}

export function formatError(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return "Unexpected error";
  }
}

export function defaultPreferences(): AppPreferences {
  return {
    version: 1,
    soundEnabled: true,
    hapticsEnabled: true,
    notificationsEnabled: true,
    dragTrigger: "Down",
    idleShortcut: "Option+Shift+Space",
    mouseTriggerButton: null,
    autoRenameOnCreate: true,
    openAfterCreating: false,
    launchAtLogin: false,
    outputSubfolder: null,
    fallbackFolder: null,
    imageQuality: 92,
    preserveExif: true,
    stripGpsExif: false,
    pdfDpi: 144,
    pdfPageSize: "a4",
    ffmpegPath: null,
    pandocPath: null,
    preferredFormats: {
      image: ["PNG", "JPG", "WEBP", "HEIC", "PDF", "REMOVE_BG", "TIFF", "GIF", "BMP"],
      pdf: [
        "PNG",
        "JPG",
        "TXT",
        "DOCX",
        "OPTIMIZE_PDF",
        "MD",
        "HTML",
        "RTF",
        "WEBP",
        "HEIC",
        "TIFF",
        "ODT",
        "EPUB",
        "FB2",
        "RST",
      ],
      document: [
        "PDF",
        "DOCX",
        "MD",
        "HTML",
        "TXT",
        "RTF",
        "ODT",
        "PNG",
        "JPG",
        "WEBP",
        "HEIC",
        "EPUB",
        "FB2",
        "RST",
      ],
      data: ["JSON", "YAML", "TOML", "CSV", "XML", "MD", "HTML", "PDF", "TXT"],
      audio: ["MP3", "M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF", "OGG", "OPUS", "AAC"],
      video: ["MP4", "MOV", "GIF", "M4A", "MP3", "WAV", "MKV", "WEBM", "AVI", "M4V"],
    },
    newFileVisibleKinds: ["folder", "txt", "md", "rtf", "docx", "xlsx", "html", "clipboard"],
    recentConversions: [],
    onboardingCompleted: false,
    language: "en",
  };
}

// ---------- Browser Preview Mock State ----------

const MOCK_BUILT_IN_KINDS: NewFileKindInfo[] = [
  { id: "folder", name: "Folder", extension: null, category: "system", page: 1, available: true, disabledReason: null },
  { id: "txt", name: "Plain Text", extension: "txt", category: "document", page: 1, available: true, disabledReason: null },
  { id: "md", name: "Markdown", extension: "md", category: "document", page: 1, available: true, disabledReason: null },
  { id: "rtf", name: "Rich Text", extension: "rtf", category: "document", page: 1, available: true, disabledReason: null },
  { id: "docx", name: "Word Document", extension: "docx", category: "office", page: 1, available: true, disabledReason: null },
  { id: "xlsx", name: "Spreadsheet", extension: "xlsx", category: "office", page: 1, available: true, disabledReason: null },
  { id: "html", name: "HTML", extension: "html", category: "code", page: 1, available: true, disabledReason: null },
  { id: "clipboard", name: "From Clipboard", extension: "png", category: "clipboard", page: 1, available: true, disabledReason: null },
  { id: "json", name: "JSON", extension: "json", category: "data", page: 2, available: true, disabledReason: null },
  { id: "csv", name: "CSV", extension: "csv", category: "data", page: 2, available: true, disabledReason: null },
  { id: "sh", name: "Shell Script", extension: "sh", category: "code", page: 2, available: true, disabledReason: null },
  { id: "py", name: "Python", extension: "py", category: "code", page: 2, available: true, disabledReason: null },
  { id: "swift", name: "Swift", extension: "swift", category: "code", page: 2, available: true, disabledReason: null },
  { id: "pptx", name: "Presentation", extension: "pptx", category: "office", page: 2, available: true, disabledReason: null },
  { id: "pages", name: "Pages Document", extension: "pages", category: "iwork", page: 2, available: true, disabledReason: null },
  { id: "numbers", name: "Numbers Sheet", extension: "numbers", category: "iwork", page: 2, available: true, disabledReason: null },
  { id: "keynote", name: "Keynote Deck", extension: "key", category: "iwork", page: 2, available: false, disabledReason: "Keynote.app not installed" },
];

const MOCK_TEMPLATES: UserTemplateInfo[] = [
  {
    id: "template:/Users/demo/Library/Application Support/FLIP/Templates/Meeting Notes.md",
    name: "Meeting Notes",
    group: null,
    extension: "md",
    sourcePath: "/Users/demo/Library/Application Support/FLIP/Templates/Meeting Notes.md",
  },
];

const mockEventListeners = new Map<string, Set<(payload: unknown) => void>>();

function emitMockEvent<T>(eventName: string, payload: T): void {
  const listeners = mockEventListeners.get(eventName);
  if (!listeners) return;
  for (const fn of listeners) {
    fn(payload);
  }
}

function loadMockPrefs(): AppPreferences {
  try {
    const raw = localStorage.getItem(MOCK_PREFS_STORAGE_KEY);
    if (raw) {
      return { ...defaultPreferences(), ...JSON.parse(raw) };
    }
  } catch {
    // ignore storage errors
  }
  const seeded = defaultPreferences();
  seeded.recentConversions = [
    {
      sourceName: "IMG_4820.heic",
      targetFormat: "PNG",
      outputPath: "/Users/demo/Desktop/IMG_4820.png",
      timestampUtc: new Date(Date.now() - 12 * 60_000).toISOString(),
    },
    {
      sourceName: "Quarterly-Report.pdf",
      targetFormat: "MD",
      outputPath: "/Users/demo/Documents/Quarterly-Report.md",
      timestampUtc: new Date(Date.now() - 48 * 60_000).toISOString(),
    },
  ];
  return seeded;
}

function saveMockPrefs(prefs: AppPreferences): void {
  try {
    localStorage.setItem(MOCK_PREFS_STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // ignore
  }
}

let mockWheelState: WheelStatePayload = {
  mode: "convert",
  files: ["/Users/demo/Desktop/vacation-photo.heic"],
  isDrag: false,
  targetFolder: "/Users/demo/Desktop",
  compatibleFormats: ["PNG", "JPG", "WEBP", "PDF", "REMOVE_BG", "TIFF", "GIF", "BMP", "ZIP"],
  batchActions: [],
  originX: 220,
  originY: 220,
  family: "image",
};

let mockPermissions: PermissionsStatus = {
  accessibility: true,
  automationFinder: "granted",
  inputMonitoring: true,
};

let mockQuickActions: QuickActionsStatus = {
  servicesDir: "/Users/demo/Library/Services",
  convertInstalled: false,
  newFileInstalled: false,
  convertPath: "/Users/demo/Library/Services/Convert with FLIP.workflow",
  newFilePath: "/Users/demo/Library/Services/New File with FLIP.workflow",
  cliExecutable: "/Applications/FLIP.app/Contents/MacOS/flip",
  cliSymlinkPath: null,
};

let mockCancelRequested = false;

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "heic", "heif", "avif", "icns", "ico"]);
const DATA_EXTS = new Set(["json", "yaml", "yml", "toml", "xml", "csv"]);
export const AUDIO_NATIVE_EXTS = new Set(["wav", "aiff", "aif", "m4a", "aac", "alac", "caf", "flac", "mp3"]);
const AUDIO_EXTS = new Set(["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus", "aiff", "aif", "alac", "caf"]);
const VIDEO_EXTS = new Set(["mp4", "mkv", "webm", "avi", "mov", "wmv", "m4v"]);
const ARCHIVE_EXTS = new Set(["zip"]);
export const DOC_NATIVE_EXTS = new Set(["txt", "md", "markdown", "html", "htm", "docx", "odt", "rtf", "doc", "epub"]);
export const DOC_PANDOC_ONLY_EXTS = new Set(["fb2", "rst"]);

function canonicalForExt(ext: string): string {
  switch (ext) {
    case "jpeg":
      return "JPG";
    case "tif":
      return "TIFF";
    case "markdown":
      return "MD";
    case "htm":
      return "HTML";
    case "yml":
      return "YAML";
    case "heif":
      return "HEIC";
    case "aif":
      return "AIFF";
    default:
      return ext.toUpperCase();
  }
}

function mockTargetsForExt(
  ext: string,
  caps: { pandoc?: boolean; ffmpeg?: boolean } = {},
): { targets: string[]; family: FormatFamily | null } {
  const hasPandoc = Boolean(caps.pandoc);
  const hasFfmpeg = caps.ffmpeg ?? true;
  if (IMAGE_EXTS.has(ext)) {
    return {
      targets: ["PNG", "JPG", "WEBP", "HEIC", "PDF", "REMOVE_BG", "TIFF", "GIF", "BMP"],
      family: "image",
    };
  }
  if (ARCHIVE_EXTS.has(ext)) {
    return { targets: ["UNZIP"], family: "archive" };
  }
  if (DATA_EXTS.has(ext)) {
    return { targets: ["JSON", "YAML", "TOML", "XML", "CSV", "TXT", "HTML", "MD", "PDF"], family: "data" };
  }
  if (AUDIO_EXTS.has(ext)) {
    if (hasFfmpeg) {
      return {
        targets: ["MP3", "M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF", "OGG", "OPUS", "AAC"],
        family: "audio",
      };
    }
    if (AUDIO_NATIVE_EXTS.has(ext)) {
      return {
        targets: ["M4A", "WAV", "FLAC", "AIFF", "ALAC", "CAF"],
        family: "audio",
      };
    }
    return { targets: [], family: "audio" };
  }
  if (VIDEO_EXTS.has(ext)) {
    if (hasFfmpeg) {
      return { targets: ["MP4", "MOV", "GIF", "M4A", "MP3", "WAV", "MKV", "WEBM", "AVI", "M4V"], family: "video" };
    }
    if (ext === "mp4" || ext === "mov" || ext === "m4v") {
      return { targets: ["MP4", "MOV", "M4V", "M4A", "WAV", "GIF"], family: "video" };
    }
    return { targets: [], family: "video" };
  }
  if (ext === "pdf") {
    const targets = [
      "PNG",
      "JPG",
      "WEBP",
      "HEIC",
      "TIFF",
      "OPTIMIZE_PDF",
      "TXT",
      "DOCX",
      "MD",
      "HTML",
      "RTF",
      "ODT",
      "EPUB",
    ];
    if (hasPandoc) targets.push("FB2", "RST");
    return { targets, family: "pdf" };
  }
  if (DOC_NATIVE_EXTS.has(ext)) {
    const targets = [
      "PDF",
      "DOCX",
      "MD",
      "HTML",
      "TXT",
      "RTF",
      "ODT",
      "PNG",
      "JPG",
      "WEBP",
      "HEIC",
      "EPUB",
    ];
    if (hasPandoc) targets.push("FB2", "RST");
    return { targets, family: "document" };
  }
  if (DOC_PANDOC_ONLY_EXTS.has(ext)) {
    return {
      targets: hasPandoc
        ? ["PDF", "DOCX", "MD", "HTML", "TXT", "RTF", "ODT", "PNG", "JPG", "WEBP", "HEIC", "EPUB", "FB2", "RST"]
        : [],
      family: "document",
    };
  }
  if (ext === "pages") return { targets: ["PDF", "DOCX", "RTF", "TXT", "EPUB"], family: "iwork" };
  if (ext === "numbers") return { targets: ["PDF", "XLSX", "CSV"], family: "iwork" };
  if (ext === "key") return { targets: ["PDF", "PPTX"], family: "iwork" };
  return { targets: [], family: null };
}

export function inspectFilesLocally(
  paths: string[],
  caps: { pandoc?: boolean; ffmpeg?: boolean } = {},
): InspectFilesResult {
  if (paths.length === 0) {
    return {
      files: [],
      compatibleFormats: [],
      batchActions: [],
      availabilityMessage: "Drop local files here",
      family: null,
    };
  }

  let shared: string[] | null = null;
  let primaryFamily: FormatFamily | null = null;

  for (const p of paths) {
    const ext = extLower(p);
    const same = canonicalForExt(ext);
    const { targets, family } = mockTargetsForExt(ext, caps);
    if (primaryFamily === null) primaryFamily = family;
    const filtered = targets.filter((t) => t !== same && (paths.length === 1 || t !== "UNZIP"));
    if (shared === null) {
      shared = filtered;
    } else {
      const nextSet = new Set(filtered);
      shared = shared.filter((f) => nextSet.has(f));
    }
  }

  const exts = paths.map(extLower);
  const compatibleFormats = shared ?? [];
  if (paths.length === 1 && !ARCHIVE_EXTS.has(exts[0] ?? "") && !compatibleFormats.includes("ZIP")) {
    compatibleFormats.push("ZIP");
  }

  const batchActions: string[] = [];
  if (paths.length >= 2) {
    if (exts.every((e) => IMAGE_EXTS.has(e))) batchActions.push("COMBINE_PDF");
    else if (exts.every((e) => e === "pdf")) batchActions.push("MERGE_PDF");
    batchActions.push("ZIP");
  }

  const availabilityMessage =
    compatibleFormats.length === 0 && batchActions.length === 0
      ? `${paths.length === 1 ? "1 file selected" : `${paths.length} files selected`}\nNo common output format`
      : "";

  return {
    files: paths,
    compatibleFormats,
    batchActions,
    availabilityMessage,
    family: primaryFamily,
  };
}

function makeDemoThumbnailSvg(ext: string): string {
  const upper = (ext || "FILE")
    .replace(/[^\x20-\x7E]/g, "")
    .toUpperCase()
    .slice(0, 5) || "FILE";
  const svgStr = `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
    <defs>
      <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" stop-color="#38bdf8"/>
        <stop offset="100%" stop-color="#6366f1"/>
      </linearGradient>
    </defs>
    <rect x="16" y="10" width="96" height="108" rx="16" fill="url(#g)"/>
    <rect x="24" y="20" width="80" height="58" rx="10" fill="rgba(255,255,255,0.22)"/>
    <text x="64" y="102" font-family="-apple-system,sans-serif" font-size="18" font-weight="700" fill="#ffffff" text-anchor="middle">${upper}</text>
  </svg>`;
  return btoa(svgStr);
}

// ---------- Public Typed IPC Surface ----------

export async function getWheelState(): Promise<WheelStatePayload> {
  if (!isTauriRuntime()) {
    const hashQuery = window.location.hash.split("?")[1] ?? "";
    const params = new URLSearchParams(hashQuery);
    const demo = params.get("demo");
    if (demo === "image") {
      const files = ["/Users/demo/Desktop/vacation-photo.heic"];
      const inspected = inspectFilesLocally(files);
      mockWheelState = {
        ...mockWheelState,
        mode: "convert",
        files,
        compatibleFormats: inspected.compatibleFormats,
        batchActions: inspected.batchActions,
        availabilityMessage: inspected.availabilityMessage,
        family: inspected.family,
      };
    } else if (demo === "new") {
      mockWheelState = {
        ...mockWheelState,
        mode: "new",
        files: [],
        compatibleFormats: [],
        batchActions: [],
      };
    } else if (demo === "many") {
      const files = ["/Users/demo/Documents/Quarterly-Report.pdf"];
      const inspected = inspectFilesLocally(files);
      mockWheelState = {
        ...mockWheelState,
        mode: "convert",
        files,
        compatibleFormats: inspected.compatibleFormats,
        batchActions: inspected.batchActions,
        availabilityMessage: inspected.availabilityMessage,
        family: inspected.family,
      };
    } else if (demo === "empty") {
      mockWheelState = {
        ...mockWheelState,
        mode: "convert",
        files: [],
        compatibleFormats: [],
        batchActions: [],
        availabilityMessage: "Drop local files here",
      };
    }
    return { ...mockWheelState };
  }
  return invoke<WheelStatePayload>("get_wheel_state");
}

export async function openWheelMode(mode: WheelMode, files: string[]): Promise<WheelStatePayload> {
  if (!isTauriRuntime()) {
    const inspected = inspectFilesLocally(files);
    mockWheelState = {
      ...mockWheelState,
      mode,
      files: [...files],
      compatibleFormats: inspected.compatibleFormats,
      batchActions: inspected.batchActions,
      availabilityMessage: inspected.availabilityMessage,
      family: inspected.family,
    };
    emitMockEvent("wheel:state", mockWheelState);
    return { ...mockWheelState };
  }
  return invoke<WheelStatePayload>("open_wheel_mode", { mode, files });
}

export async function switchWheelMode(mode: WheelMode): Promise<WheelStatePayload> {
  if (!isTauriRuntime()) {
    mockWheelState = { ...mockWheelState, mode };
    return { ...mockWheelState };
  }
  return invoke<WheelStatePayload>("switch_wheel_mode", { mode });
}

export async function inspectFiles(paths: string[]): Promise<InspectFilesResult> {
  if (!isTauriRuntime()) {
    return inspectFilesLocally(paths);
  }
  const raw = await invoke<Partial<InspectFilesResult> & { formats?: string[] }>("inspect_files", {
    paths,
  });
  return {
    files: raw.files ?? paths,
    compatibleFormats: raw.compatibleFormats ?? raw.formats ?? [],
    batchActions: raw.batchActions ?? [],
    availabilityMessage: raw.availabilityMessage ?? "",
    family: raw.family ?? null,
  };
}

export async function getThumbnail(path: string, size = 128): Promise<ThumbnailResult> {
  if (!isTauriRuntime()) {
    return {
      ok: true,
      pngBase64: makeDemoThumbnailSvg(extLower(path)),
      width: 128,
      height: 128,
      error: null,
    };
  }
  try {
    return await invoke<ThumbnailResult>("get_thumbnail", { path, size });
  } catch (err) {
    return {
      ok: false,
      pngBase64: null,
      width: null,
      height: null,
      error: formatError(err),
    };
  }
}

export async function executeConversion(
  paths: string[],
  formats: string[],
): Promise<ConversionCompletePayload> {
  if (!isTauriRuntime()) {
    mockCancelRequested = false;
    const steps = [16, 44, 72, 94];
    for (const pct of steps) {
      if (mockCancelRequested) {
        const cancelled: ConversionCompletePayload = {
          success: false,
          results: [],
          error: "Conversion cancelled",
        };
        emitMockEvent("conversion:complete", cancelled);
        return cancelled;
      }
      const label =
        formats[0] === "COMBINE_PDF" || formats[0] === "MERGE_PDF" || formats[0] === "ZIP"
          ? `${paths.length} files → ${formats[0]}`
          : `${basename(paths[0] ?? "file")} → ${formats[0] ?? ""}`;
      emitMockEvent<ConversionProgressPayload>("conversion:progress", {
        percent: pct,
        label,
      });
      await new Promise((r) => setTimeout(r, 150));
    }

    const results: ConversionResult[] = [];
    const prefs = loadMockPrefs();
    for (const fmt of formats) {
      if (fmt === "COMBINE_PDF" || fmt === "MERGE_PDF" || fmt === "ZIP") {
        const first = paths[0] ?? "/Users/demo/Desktop/file";
        const stem = basename(first).replace(/\.[^.]+$/, "") || "archive";
        const ext = fmt === "ZIP" ? "zip" : "pdf";
        const targetFormat = fmt === "ZIP" ? "ZIP" : "PDF";
        const out = `${dirname(first)}/${stem}.${ext}`;
        results.push({ source: first, format: fmt, outputs: [out], error: null });
        prefs.recentConversions.unshift({
          sourceName: paths.length === 1 ? basename(first) : `${paths.length} files`,
          targetFormat,
          outputPath: out,
          timestampUtc: new Date().toISOString(),
        });
      } else {
        for (const src of paths) {
          const stem = basename(src).replace(/\.[^.]+$/, "") || "output";
          const srcExt = extLower(src);
          let out: string;
          if (fmt === "UNZIP") {
            out = `${dirname(src)}/${stem}`;
          } else if (fmt === "REMOVE_BG") {
            out = srcExt === "png" ? `${dirname(src)}/${stem} 2.png` : `${dirname(src)}/${stem}.png`;
          } else if (fmt === "OPTIMIZE_PDF") {
            out = `${dirname(src)}/${stem} 2.pdf`;
          } else {
            out = `${dirname(src)}/${stem}.${fmt.toLowerCase()}`;
          }
          results.push({ source: src, format: fmt, outputs: [out], error: null });
          prefs.recentConversions.unshift({
            sourceName: basename(src),
            targetFormat: fmt,
            outputPath: out,
            timestampUtc: new Date().toISOString(),
          });
        }
      }
    }
    prefs.recentConversions = prefs.recentConversions.slice(0, 10);
    saveMockPrefs(prefs);
    const payload: ConversionCompletePayload = { success: true, results, error: null };
    emitMockEvent("conversion:complete", payload);
    return payload;
  }

  const res = await invoke<ConversionCompletePayload | ConversionResult[]>("execute_conversion", {
    paths,
    formats,
  });
  if (Array.isArray(res)) {
    const failed = res.find((r) => Boolean(r.error));
    return {
      success: !failed,
      results: res,
      error: failed?.error ?? null,
    };
  }
  return res;
}

export async function cancelConversion(): Promise<void> {
  if (!isTauriRuntime()) {
    mockCancelRequested = true;
    return;
  }
  await invoke<void>("cancel_conversion");
}

export async function executeCreateNew(
  kindId: string,
  targetFolder?: string | null,
): Promise<CreateNewResult> {
  if (!isTauriRuntime()) {
    const folder = targetFolder || "/Users/demo/Desktop";
    const match = MOCK_BUILT_IN_KINDS.find((k) => k.id === kindId);
    const filename =
      kindId === "folder"
        ? "untitled folder"
        : kindId === "clipboard"
          ? "Clipboard 2026-11-17 at 16.10.00.png"
          : `Untitled.${match?.extension ?? "txt"}`;
    await new Promise((r) => setTimeout(r, 120));
    return {
      path: `${folder}/${filename}`,
      kindId,
      revealed: true,
      renameTriggered: loadMockPrefs().autoRenameOnCreate,
      fallbackUsed: false,
    };
  }
  return invoke<CreateNewResult>("execute_create_new", {
    kindId,
    targetFolder: targetFolder ?? null,
  });
}

export async function getNewFileKinds(): Promise<NewFileKindsResponse> {
  if (!isTauriRuntime()) {
    return {
      kinds: MOCK_BUILT_IN_KINDS,
      templates: MOCK_TEMPLATES,
      clipboard: {
        available: true,
        kind: "image",
        extension: "png",
        types: ["public.png"],
      },
    };
  }
  const raw = await invoke<NewFileKindsResponse | NewFileKindInfo[]>("get_new_file_kinds");
  if (Array.isArray(raw)) {
    return { kinds: raw, templates: [] };
  }
  return {
    kinds: raw.kinds ?? [],
    templates: raw.templates ?? [],
    clipboard: raw.clipboard,
  };
}

function mergePreferredFormats(
  defaults: Record<string, string[]>,
  incoming?: Record<string, string[]>,
): Record<string, string[]> {
  const merged: Record<string, string[]> = {};
  for (const [fam, defList] of Object.entries(defaults)) {
    const custom = incoming?.[fam];
    if (Array.isArray(custom) && custom.length > 0) {
      const next = [...custom];
      const upper = new Set(next.map((f) => f.toUpperCase()));
      for (const d of defList) {
        if (!upper.has(d.toUpperCase())) next.push(d);
      }
      merged[fam] = next;
    } else {
      merged[fam] = [...defList];
    }
  }
  if (incoming) {
    for (const [fam, list] of Object.entries(incoming)) {
      if (!(fam in merged) && Array.isArray(list)) {
        merged[fam] = [...list];
      }
    }
  }
  return merged;
}

export async function getPreferences(): Promise<AppPreferences> {
  const defaults = defaultPreferences();
  if (!isTauriRuntime()) {
    const mock = loadMockPrefs();
    return {
      ...defaults,
      ...mock,
      language: normalizeLanguage(mock.language),
      preferredFormats: mergePreferredFormats(defaults.preferredFormats, mock.preferredFormats),
    };
  }
  const prefs = await invoke<Partial<AppPreferences>>("get_preferences");
  return {
    ...defaults,
    ...prefs,
    language: normalizeLanguage(prefs.language),
    preferredFormats: mergePreferredFormats(defaults.preferredFormats, prefs.preferredFormats),
  };
}

export async function savePreferences(prefs: AppPreferences): Promise<AppPreferences> {
  const sanitized: AppPreferences = {
    ...prefs,
    language: normalizeLanguage(prefs.language),
  };
  if (!isTauriRuntime()) {
    saveMockPrefs(sanitized);
    emitMockEvent("prefs:changed", sanitized);
    return sanitized;
  }
  const res = await invoke<AppPreferences | void>("save_preferences", { prefs: sanitized });
  return res
    ? { ...res, language: normalizeLanguage(res.language) }
    : sanitized;
}

export async function getEngineStatus(): Promise<EngineStatus> {
  if (!isTauriRuntime()) {
    return {
      ffmpeg: "/opt/homebrew/bin/ffmpeg",
      ffprobe: "/opt/homebrew/bin/ffprobe",
      pandoc: null,
      bridge: "/Applications/FLIP.app/Contents/Resources/flip-macos-bridge",
      pages: true,
      numbers: true,
      keynote: false,
      nativeFrameworks: {
        imageIO: true,
        pdfKit: true,
        visionOcr: true,
        avFoundation: true,
      },
    };
  }
  const raw = await invoke<Partial<EngineStatus> & { tools?: DetectedTools }>("get_engine_status");
  const base: DetectedTools = raw.tools ?? {
    ffmpeg: raw.ffmpeg ?? null,
    ffprobe: raw.ffprobe ?? null,
    pandoc: raw.pandoc ?? null,
    bridge: raw.bridge ?? null,
    pages: Boolean(raw.pages),
    numbers: Boolean(raw.numbers),
    keynote: Boolean(raw.keynote),
  };
  return {
    ...base,
    nativeFrameworks: raw.nativeFrameworks ?? {
      imageIO: true,
      pdfKit: true,
      visionOcr: true,
      avFoundation: true,
    },
  };
}

export async function getPermissionsStatus(): Promise<PermissionsStatus> {
  if (!isTauriRuntime()) {
    return { ...mockPermissions };
  }
  return invoke<PermissionsStatus>("get_permissions_status");
}

export async function requestPermission(kind: PermissionRequestKind): Promise<PermissionsStatus> {
  if (!isTauriRuntime()) {
    if (
      kind === "accessibility_prompt" ||
      kind === "open_accessibility" ||
      kind === "open_input_monitoring"
    ) {
      mockPermissions = { ...mockPermissions, accessibility: true, inputMonitoring: true };
    } else if (kind === "open_automation") {
      mockPermissions = { ...mockPermissions, automationFinder: "granted" };
    }
    return { ...mockPermissions };
  }
  return invoke<PermissionsStatus>("request_permission", { kind });
}

export async function playWheelFeedback(kind: FeedbackKind): Promise<void> {
  if (!isTauriRuntime()) return;
  try {
    await invoke<void>("play_wheel_feedback", { kind });
  } catch {
    // non-fatal feedback failure
  }
}

export async function dismissWheel(reason: string): Promise<void> {
  if (!isTauriRuntime()) {
    emitMockEvent("wheel:escape", { reason });
    return;
  }
  await invoke<void>("dismiss_wheel", { reason });
}

export async function openWindow(label: "settings" | "batch" | "onboarding" | "wheel"): Promise<void> {
  if (!isTauriRuntime()) {
    window.location.hash = `#${label}`;
    return;
  }
  await invoke<void>("open_window", { label });
}

export async function openTemplatesFolder(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke<void>("open_templates_folder");
}

export async function revealLogs(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke<void>("reveal_logs");
}

export async function revealInFinder(paths: string[]): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke<void>("reveal_in_finder", { paths });
}

export async function getRecentConversions(): Promise<RecentConversion[]> {
  if (!isTauriRuntime()) {
    return loadMockPrefs().recentConversions;
  }
  return invoke<RecentConversion[]>("get_recent_conversions");
}

export async function getQuickActionsStatus(): Promise<QuickActionsStatus> {
  if (!isTauriRuntime()) {
    return { ...mockQuickActions };
  }
  return invoke<QuickActionsStatus>("get_quick_actions_status");
}

export async function installFinderQuickActions(
  installCli = false,
): Promise<QuickActionsStatus> {
  if (!isTauriRuntime()) {
    mockQuickActions = {
      ...mockQuickActions,
      convertInstalled: true,
      newFileInstalled: true,
      cliSymlinkPath: installCli
        ? "/Users/demo/.local/bin/flip"
        : mockQuickActions.cliSymlinkPath,
    };
    return { ...mockQuickActions };
  }
  return invoke<QuickActionsStatus>("install_finder_quick_actions", { installCli });
}

export async function checkForUpdates(repoUrl?: string): Promise<UpdateCheckResult> {
  if (!isTauriRuntime()) {
    return {
      currentVersion: "0.1.0",
      latestTag: "v0.1.0",
      releaseName: "FLIP 0.1.0",
      htmlUrl: "https://github.com/thetinyfoundry/FLIP/releases",
      publishedAt: new Date().toISOString(),
      body: "Browser preview mock release check (you are up to date).",
      updateAvailable: false,
      checkedAtUtc: new Date().toISOString(),
    };
  }
  return invoke<UpdateCheckResult>("check_for_updates", { repoUrl: repoUrl ?? null });
}

export async function openReleaseUrl(url: string): Promise<void> {
  const trimmed = url.trim();
  if (!trimmed.toLowerCase().startsWith("https://github.com/")) {
    throw new Error("Release URL must start with https://github.com/.");
  }
  if (!isTauriRuntime()) {
    if (typeof window !== "undefined" && typeof window.open === "function") {
      window.open(trimmed, "_blank", "noopener,noreferrer");
    }
    return;
  }
  await invoke<void>("open_release_url", { url: trimmed });
}

export async function consumeSettingsNavigation(): Promise<SettingsNavigatePayload | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  return invoke<SettingsNavigatePayload | null>("consume_settings_navigation");
}

export async function getDiagnosticsReport(): Promise<string> {
  if (!isTauriRuntime()) {
    return [
      "# FLIP Diagnostics Report",
      "",
      `Generated (UTC): ${new Date().toISOString()}`,
      "",
      "## App & System",
      "- **FLIP Version**: 0.1.0 (`com.thetinyfoundry.flip.mac`, arm64)",
      "- **macOS**: macOS 27.2.1 (26B131) [Browser Preview]",
      "",
      "## Engines & Frameworks",
      "- **Swift Bridge**: Detected (`/Applications/FLIP.app/Contents/Resources/flip-macos-bridge`)",
      "- **Native Apple Frameworks**: ImageIO, PDFKit, Vision OCR, AVFoundation (Ready)",
      "- **FFmpeg**: Detected (`/opt/homebrew/bin/ffmpeg`)",
      "- **Pandoc**: Not Found",
      "",
      "## Recent Events (last 60 lines of `events.jsonl`, redacted)",
      "```jsonl",
      '{"utc":"2026-11-17T20:00:00Z","name":"wheel.open","detail":{"mode":"convert","path":"~/Desktop/sample.heic"}}',
      "```",
      "",
    ].join("\n");
  }
  return invoke<string>("get_diagnostics_report");
}

export interface CopyTextResult {
  ok: boolean;
  method: "clipboard" | "execCommand" | "manual";
  error?: string;
}

export interface CopyDiagnosticsResult extends CopyTextResult {
  report: string;
}

export async function copyTextWithFallback(text: string): Promise<CopyTextResult> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return { ok: true, method: "clipboard" };
    }
  } catch {
    // Fall through to execCommand fallback
  }

  try {
    if (
      typeof document !== "undefined" &&
      document.body &&
      typeof document.execCommand === "function"
    ) {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "true");
      ta.style.position = "fixed";
      ta.style.left = "-9999px";
      ta.style.top = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      if (ok) {
        return { ok: true, method: "execCommand" };
      }
    }
  } catch {
    // Fall through to manual fallback
  }

  return {
    ok: false,
    method: "manual",
    error: "Clipboard access unavailable; copy diagnostics manually.",
  };
}

export async function copyDiagnosticsToClipboard(): Promise<CopyDiagnosticsResult> {
  const report = await getDiagnosticsReport();
  const res = await copyTextWithFallback(report);
  return { ...res, report };
}

// ---------- Batch Handoff Helpers ----------

export function storeBatchHandoff(
  files: string[],
  results?: ConversionResult[],
): void {
  try {
    localStorage.setItem(BATCH_HANDOFF_STORAGE_KEY, JSON.stringify(files));
    if (results) {
      localStorage.setItem(BATCH_RESULTS_STORAGE_KEY, JSON.stringify(results));
    } else {
      localStorage.removeItem(BATCH_RESULTS_STORAGE_KEY);
    }
  } catch {
    // ignore storage errors
  }
}

export function consumeBatchHandoff(): {
  files: string[];
  results: ConversionResult[] | null;
} {
  let files: string[] = [];
  let results: ConversionResult[] | null = null;
  try {
    const rawFiles = localStorage.getItem(BATCH_HANDOFF_STORAGE_KEY);
    if (rawFiles) {
      files = JSON.parse(rawFiles);
      localStorage.removeItem(BATCH_HANDOFF_STORAGE_KEY);
    }
    const rawResults = localStorage.getItem(BATCH_RESULTS_STORAGE_KEY);
    if (rawResults) {
      results = JSON.parse(rawResults);
      localStorage.removeItem(BATCH_RESULTS_STORAGE_KEY);
    }
  } catch {
    // ignore
  }
  return { files, results };
}

export async function openBatchWithFiles(
  files: string[],
  results?: ConversionResult[],
): Promise<void> {
  storeBatchHandoff(files, results);
  if (isTauriRuntime()) {
    try {
      await emit("batch:files", { files, results: results ?? null });
    } catch {
      // ignore emit failure
    }
  }
  await openWindow("batch");
}

// ---------- Event Listeners ----------

async function subscribeEvent<T>(
  eventName: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    let set = mockEventListeners.get(eventName);
    if (!set) {
      set = new Set();
      mockEventListeners.set(eventName, set);
    }
    const wrapped = (p: unknown) => handler(p as T);
    set.add(wrapped);
    return () => {
      set?.delete(wrapped);
    };
  }
  return listen<T>(eventName, (event) => handler(event.payload));
}

export const events = {
  onWheelState: (cb: (p: WheelStatePayload) => void) =>
    subscribeEvent<WheelStatePayload>("wheel:state", cb),
  onWheelDragFiles: (cb: (p: WheelDragFilesPayload) => void) =>
    subscribeEvent<WheelDragFilesPayload>("wheel:drag-files", cb),
  onWheelPointerMove: (cb: (p: WheelPointerMovePayload) => void) =>
    subscribeEvent<WheelPointerMovePayload>("wheel:pointer-move", cb),
  onWheelPointerRelease: (cb: (p: WheelPointerReleasePayload) => void) =>
    subscribeEvent<WheelPointerReleasePayload>("wheel:pointer-release", cb),
  onWheelEscape: (cb: (p: Record<string, unknown>) => void) =>
    subscribeEvent<Record<string, unknown>>("wheel:escape", cb),
  onConversionProgress: (cb: (p: ConversionProgressPayload) => void) =>
    subscribeEvent<ConversionProgressPayload>("conversion:progress", cb),
  onConversionComplete: (cb: (p: ConversionCompletePayload) => void) =>
    subscribeEvent<ConversionCompletePayload>("conversion:complete", cb),
  onBatchFiles: (cb: (p: { files: string[]; results?: ConversionResult[] | null }) => void) =>
    subscribeEvent<{ files: string[]; results?: ConversionResult[] | null }>("batch:files", cb),
  onSettingsNavigate: (cb: (p: SettingsNavigatePayload) => void) =>
    subscribeEvent<SettingsNavigatePayload>("settings:navigate", cb),
  onTemplatesChanged: (cb: (p: { count: number }) => void) =>
    subscribeEvent<{ count: number }>("templates:changed", cb),
  onPrefsChanged: (cb: (p: AppPreferences) => void) =>
    subscribeEvent<AppPreferences>("prefs:changed", cb),
};

export interface NativeDragDropCallbacks {
  onEnter?: (paths: string[], logicalX: number, logicalY: number) => void;
  onOver?: (logicalX: number, logicalY: number) => void;
  onDrop?: (paths: string[], logicalX: number, logicalY: number) => void;
  onLeave?: () => void;
}

export async function subscribeNativeDragDrop(
  callbacks: NativeDragDropCallbacks,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return () => {};
  }
  try {
    const webview = getCurrentWebview();
    return await webview.onDragDropEvent((event) => {
      const dpr = window.devicePixelRatio || 1;
      const p = event.payload;
      if (p.type === "enter") {
        callbacks.onEnter?.(p.paths, p.position.x / dpr, p.position.y / dpr);
      } else if (p.type === "over") {
        callbacks.onOver?.(p.position.x / dpr, p.position.y / dpr);
      } else if (p.type === "drop") {
        callbacks.onDrop?.(p.paths, p.position.x / dpr, p.position.y / dpr);
      } else if (p.type === "leave") {
        callbacks.onLeave?.();
      }
    });
  } catch {
    return () => {};
  }
}
