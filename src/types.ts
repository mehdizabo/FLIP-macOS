export type RouteName = "wheel" | "settings" | "batch" | "onboarding";

export type AppLanguage = "en" | "it";

export type WheelMode = "convert" | "new";

export type FormatFamily =
  | "image"
  | "pdf"
  | "document"
  | "data"
  | "audio"
  | "video"
  | "iwork"
  | "archive";

export type FeedbackKind =
  | "tick"
  | "sound"
  | "alignment"
  | "level"
  | "generic"
  | "success"
  | "error";

export type HapticIntensity = "light" | "medium" | "firm" | "strong";

export type PermissionRequestKind =
  | "accessibility_prompt"
  | "open_accessibility"
  | "open_automation"
  | "open_input_monitoring";

export interface RecentConversion {
  sourceName: string;
  targetFormat: string;
  outputPath: string;
  timestampUtc: string;
}

export interface AppPreferences {
  version: number;
  soundEnabled: boolean;
  hapticsEnabled: boolean;
  hapticIntensity: HapticIntensity | string;
  auraEnabled: boolean;
  notificationsEnabled: boolean;
  dragTrigger: string;
  idleShortcut: string;
  mouseTriggerButton: number | null;
  autoRenameOnCreate: boolean;
  openAfterCreating: boolean;
  launchAtLogin: boolean;
  outputSubfolder: string | null;
  fallbackFolder: string | null;
  imageQuality: number;
  preserveExif: boolean;
  stripGpsExif: boolean;
  pdfDpi: number;
  pdfPageSize: "a4" | "letter" | string;
  ffmpegPath: string | null;
  pandocPath: string | null;
  preferredFormats: Record<string, string[]>;
  newFileVisibleKinds: string[];
  recentConversions: RecentConversion[];
  onboardingCompleted?: boolean;
  language: AppLanguage | string;
}

export interface ConversionResult {
  source: string;
  format: string;
  outputs: string[];
  error: string | null;
}

export interface DetectedTools {
  ffmpeg: string | null;
  ffprobe: string | null;
  pandoc: string | null;
  whisper: string | null;
  whisperModel: string | null;
  /** Absolute path of the `gs` binary used for PDF/A, or null when it is not installed. */
  ghostscript: string | null;
  bridge: string | null;
  pages: boolean;
  numbers: boolean;
  keynote: boolean;
}

export interface EngineStatus extends DetectedTools {
  nativeFrameworks?: {
    imageIO: boolean;
    pdfKit: boolean;
    visionOcr: boolean;
    avFoundation: boolean;
  };
}

export interface PermissionsStatus {
  accessibility: boolean;
  automationFinder: string;
  inputMonitoring: boolean;
}

export interface QuickActionsStatus {
  servicesDir: string;
  convertInstalled: boolean;
  newFileInstalled: boolean;
  convertPath: string;
  newFilePath: string;
  cliExecutable: string | null;
  cliSymlinkPath: string | null;
}

export interface NewFileKindInfo {
  id: string;
  name: string;
  extension: string | null;
  category: string;
  page: number;
  available: boolean;
  disabledReason: string | null;
}

export interface UserTemplateInfo {
  id: string;
  name: string;
  group: string | null;
  extension: string | null;
  sourcePath: string;
}

export interface ClipboardStatusInfo {
  available: boolean;
  kind: string;
  extension: string;
  types?: string[];
}

export interface NewFileKindsResponse {
  kinds: NewFileKindInfo[];
  templates: UserTemplateInfo[];
  clipboard?: ClipboardStatusInfo;
}

export interface CreateNewResult {
  path: string;
  kindId: string;
  revealed: boolean;
  renameTriggered: boolean;
  fallbackUsed: boolean;
}

export interface InspectFilesResult {
  files: string[];
  compatibleFormats: string[];
  batchActions: string[];
  /** Image tool tokens (crop, rotate, scale, palette, ...) valid for every selected file. */
  toolActions?: string[];
  availabilityMessage: string;
  family?: FormatFamily | null;
  imageWidth?: number | null;
  imageHeight?: number | null;
}

export interface ThumbnailResult {
  ok: boolean;
  pngBase64: string | null;
  width: number | null;
  height: number | null;
  error: string | null;
}

export interface WheelStatePayload {
  mode: WheelMode;
  files: string[];
  isDrag: boolean;
  targetFolder: string;
  compatibleFormats: string[];
  batchActions: string[];
  toolActions?: string[];
  originX: number;
  originY: number;
  availabilityMessage?: string;
  family?: FormatFamily | null;
  imageWidth?: number | null;
  imageHeight?: number | null;
}

export interface WheelDragFilesPayload {
  files: string[];
}

export interface WheelPointerMovePayload {
  localX: number;
  localY: number;
}

export interface WheelPointerReleasePayload {
  button: number;
  localX: number;
  localY: number;
}

export interface ConversionProgressPayload {
  percent: number;
  label: string;
}

export interface ConversionCompletePayload {
  success: boolean;
  results: ConversionResult[];
  error: string | null;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestTag: string;
  releaseName: string;
  htmlUrl: string;
  publishedAt: string | null;
  body: string;
  updateAvailable: boolean;
  checkedAtUtc: string;
}

export interface SettingsNavigatePayload {
  tab?: "general" | "shortcuts" | "formats" | "newfile" | "engines" | "about";
  checkUpdates?: boolean;
}
