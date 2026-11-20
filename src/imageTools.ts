/**
 * Image tools ("Strumenti"): the second face of the wheel.
 *
 * Tool tokens are plain action ids that go through `executeConversion` like a format. They are
 * defined by `src-tauri/src/engines/image_tools.rs`; a Rust test keeps `IMAGE_TOOL_TOKENS` identical.
 */

/** Every tool token, in the order the wheel presents them (same as Rust `ALL_TOOL_TOKENS`). */
export const IMAGE_TOOL_TOKENS = [
  "CROP_9_16",
  "CROP_4_5",
  "CROP_1_1",
  "CROP_16_9",
  "ROTATE_RIGHT",
  "ROTATE_LEFT",
  "ROTATE_180",
  "FLIP_H",
  "FLIP_V",
  "SCALE_50",
  "SCALE_25",
  "SCALE_1080P",
  "SCALE_720P",
  "SCALE_1MB",
  "REDACT",
  "PALETTE",
  "STRIP_EXIF",
] as const;

export type ImageToolToken = (typeof IMAGE_TOOL_TOKENS)[number];
export type ToolGroupId = "crop" | "rotate" | "scale";

export interface ToolGroup {
  id: ToolGroupId;
  tokens: readonly ImageToolToken[];
}

/** Tools that open a sub-page of presets instead of acting directly. */
export const TOOL_GROUPS: readonly ToolGroup[] = [
  { id: "crop", tokens: ["CROP_9_16", "CROP_4_5", "CROP_1_1", "CROP_16_9"] },
  { id: "rotate", tokens: ["ROTATE_RIGHT", "ROTATE_LEFT", "ROTATE_180", "FLIP_H", "FLIP_V"] },
  { id: "scale", tokens: ["SCALE_50", "SCALE_25", "SCALE_1080P", "SCALE_720P", "SCALE_1MB"] },
];

const TOKEN_SET: ReadonlySet<string> = new Set(IMAGE_TOOL_TOKENS);

export function isImageToolToken(token: string): token is ImageToolToken {
  return TOKEN_SET.has(token.trim().toUpperCase());
}

export function toolGroupOf(token: string): ToolGroupId | null {
  const up = token.trim().toUpperCase();
  return TOOL_GROUPS.find((g) => (g.tokens as readonly string[]).includes(up))?.id ?? null;
}

const SUFFIXES: Record<ImageToolToken, string> = {
  CROP_9_16: "-crop-9x16",
  CROP_4_5: "-crop-4x5",
  CROP_1_1: "-crop-1x1",
  CROP_16_9: "-crop-16x9",
  ROTATE_RIGHT: "-rotate-right",
  ROTATE_LEFT: "-rotate-left",
  ROTATE_180: "-rotate-180",
  FLIP_H: "-flip-h",
  FLIP_V: "-flip-v",
  SCALE_50: "-50pct",
  SCALE_25: "-25pct",
  SCALE_1080P: "-1080p",
  SCALE_720P: "-720p",
  SCALE_1MB: "-under-1mb",
  REDACT: "-redacted",
  PALETTE: "-palette",
  STRIP_EXIF: "-clean",
};

/** File-name suffix the engine appends (mirrors Rust `ImageTool::suffix`). */
export function toolOutputSuffix(token: ImageToolToken): string {
  return SUFFIXES[token];
}

/** Extension of the produced file (mirrors Rust `ImageTool::output_format`). */
export function toolOutputExtension(token: ImageToolToken, sourceExt: string): string {
  if (token === "PALETTE") return "png";
  switch (sourceExt.toLowerCase()) {
    case "jpg":
    case "jpeg":
    case "heic":
    case "heif":
    case "avif":
      return "jpg";
    case "webp":
      return "webp";
    case "bmp":
      return "bmp";
    case "tif":
    case "tiff":
      return "tiff";
    default:
      return "png";
  }
}

/** One slot on the Tools face's first page. */
export type ToolEntry =
  | { kind: "group"; id: ToolGroupId; tokens: ImageToolToken[] }
  | { kind: "leaf"; token: string };

/**
 * First page of the Tools face, in the order of the original FLIP:
 * Crop, Rotate, Scale, Background, Redact, Palette, EXIF, ZIP. Entries the selection cannot use are omitted.
 */
export function buildToolEntries(
  toolActions: readonly string[] | undefined,
  compatibleFormats: readonly string[],
  batchActions: readonly string[],
): ToolEntry[] {
  const available = new Set((toolActions ?? []).map((t) => t.toUpperCase()));
  const entries: ToolEntry[] = [];

  for (const group of TOOL_GROUPS) {
    const tokens = group.tokens.filter((t) => available.has(t));
    if (tokens.length > 0) entries.push({ kind: "group", id: group.id, tokens: [...tokens] });
  }
  if (compatibleFormats.includes("REMOVE_BG")) entries.push({ kind: "leaf", token: "REMOVE_BG" });
  if (available.has("REDACT")) entries.push({ kind: "leaf", token: "REDACT" });
  if (available.has("PALETTE")) entries.push({ kind: "leaf", token: "PALETTE" });
  if (available.has("STRIP_EXIF")) entries.push({ kind: "leaf", token: "STRIP_EXIF" });
  if (compatibleFormats.includes("ZIP") || batchActions.includes("ZIP")) {
    entries.push({ kind: "leaf", token: "ZIP" });
  }
  return entries;
}

/** Tool tokens that apply to every file of a selection (mirrors Rust `tools_for_paths`). */
export function toolActionsForExtensions(
  exts: readonly string[],
  imageExts: ReadonlySet<string>,
): string[] {
  if (exts.length === 0 || !exts.every((e) => imageExts.has(e.toLowerCase()))) return [];
  const carriesMetadata = (e: string) =>
    ["jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "heif", "avif"].includes(e.toLowerCase());
  return IMAGE_TOOL_TOKENS.filter((t) => t !== "STRIP_EXIF" || exts.every(carriesMetadata));
}
