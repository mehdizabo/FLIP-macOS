export const WHEEL_CANVAS_SIZE = 440;
export const WHEEL_CENTER = WHEEL_CANVAS_SIZE / 2; // 220
export const WHEEL_OUTER_RADIUS = 170; // 340px ring inside 440px canvas
export const WHEEL_INNER_RADIUS = 78;
export const WHEEL_HUB_DIAMETER = 132;
export const WHEEL_HUB_RADIUS = WHEEL_HUB_DIAMETER / 2; // 66
export const WHEEL_DEAD_ZONE_RADIUS = 68;
export const WHEEL_OUTER_MAX_RADIUS = 210;
export const WHEEL_PROGRESS_RADIUS = 73;
export const WHEEL_HYSTERESIS_DEG = 3;
export const MAX_SECTORS_PER_PAGE = 8;
export const DRAG_DWELL_MS = 300;

export const DEFAULT_PREFERRED_FORMATS: readonly string[] = [
  "DOCX",
  "PDF",
  "HTML",
  "MD",
  "TXT",
  "RTF",
  "ODT",
  "EPUB",
  "FB2",
  "RST",
  "JSON",
  "YAML",
  "TOML",
  "XML",
  "CSV",
  "JPG",
  "PNG",
  "WEBP",
  "HEIC",
  "BMP",
  "TIFF",
  "GIF",
  "MP4",
  "MP3",
  "WAV",
  "MKV",
  "WEBM",
  "AVI",
  "MOV",
  "M4V",
  "FLAC",
  "OGG",
  "OPUS",
  "M4A",
  "AAC",
  "AIFF",
  "ALAC",
  "XLSX",
  "PPTX",
  "COMBINE_PDF",
  "MERGE_PDF",
];

export interface Point2D {
  x: number;
  y: number;
}

export interface SectorAngleBounds {
  index: number;
  count: number;
  spanDeg: number;
  gapDeg: number;
  midDeg: number;
  startDeg: number;
  endDeg: number;
}

export interface SectorGeometryOptions {
  cx?: number;
  cy?: number;
  innerRadius?: number;
  outerRadius?: number;
  cornerInset?: number;
}

export interface HitTestOptions {
  cx?: number;
  cy?: number;
  innerDeadZoneRadius?: number;
  outerMaxRadius?: number;
  previousIndex?: number;
  hysteresisDeg?: number;
}

export type PaginatedSlot<T> =
  | { kind: "item"; item: T; globalIndex: number }
  | { kind: "more"; targetPage: number }
  | { kind: "back"; targetPage: number };

export interface PaginatedRingPage<T> {
  page: number;
  totalPages: number;
  slots: PaginatedSlot<T>[];
}

/**
 * Port of `Wheel.OrderFormats` (FLIP-windows/src/Flip/Wheel.cs).
 * Orders known preferred formats first, then appends any remaining targets in input order.
 */
export function orderFormats(
  outputs: readonly string[],
  preferredOrder: readonly string[] = DEFAULT_PREFERRED_FORMATS,
): string[] {
  const deduped: string[] = [];
  const seen = new Set<string>();
  for (const raw of outputs) {
    const clean = raw.trim();
    if (!clean || seen.has(clean)) continue;
    seen.add(clean);
    deduped.push(clean);
  }

  const ordered: string[] = [];
  const orderedSet = new Set<string>();

  for (const pref of preferredOrder) {
    if (seen.has(pref) && !orderedSet.has(pref)) {
      ordered.push(pref);
      orderedSet.add(pref);
    }
  }

  for (const fmt of deduped) {
    if (!orderedSet.has(fmt)) {
      ordered.push(fmt);
      orderedSet.add(fmt);
    }
  }

  return ordered;
}

/**
 * Converts polar coordinates where 0° points straight up (12 o'clock)
 * and positive angles rotate clockwise.
 */
export function polarToCartesian(
  radius: number,
  angleDeg: number,
  cx: number = WHEEL_CENTER,
  cy: number = WHEEL_CENTER,
): Point2D {
  const rad = ((angleDeg - 90) * Math.PI) / 180;
  return {
    x: cx + radius * Math.cos(rad),
    y: cy + radius * Math.sin(rad),
  };
}

function fmtNum(n: number): string {
  return Number(n.toFixed(3)).toString();
}

export function getSectorAngles(index: number, count: number): SectorAngleBounds {
  const safeCount = Math.max(1, count);
  const spanDeg = 360 / safeCount;
  const gapDeg = safeCount === 1 ? 1.8 : Math.min(2.2, spanDeg * 0.06);
  const midDeg = index * spanDeg;
  const startDeg = midDeg - spanDeg / 2 + gapDeg;
  const endDeg = midDeg + spanDeg / 2 - gapDeg;
  return {
    index,
    count: safeCount,
    spanDeg,
    gapDeg,
    midDeg,
    startDeg,
    endDeg,
  };
}

/**
 * Port of `Wheel.SectorData` adapted to 340px-in-440px Liquid Glass ring.
 * Produces an SVG path with chamfered/rounded outer and inner corners via Quadratic Bezier fillets.
 */
export function sectorSvgPath(
  index: number,
  count: number,
  options: SectorGeometryOptions = {},
): string {
  if (count <= 0) return "";
  const cx = options.cx ?? WHEEL_CENTER;
  const cy = options.cy ?? WHEEL_CENTER;
  const innerR = options.innerRadius ?? WHEEL_INNER_RADIUS;
  const outerR = options.outerRadius ?? WHEEL_OUTER_RADIUS;
  const cornerInset = options.cornerInset ?? 7;

  const { startDeg, endDeg } = getSectorAngles(index, count);
  const arcSpan = Math.max(1, endDeg - startDeg);
  const largeArc = arcSpan > 180 ? 1 : 0;

  const outerCornerDeg = Math.min(2.4, arcSpan * 0.08);
  const innerCornerDeg = Math.min(4.2, arcSpan * 0.14);

  const p0 = polarToCartesian(outerR - cornerInset, startDeg, cx, cy);
  const p1Ctrl = polarToCartesian(outerR, startDeg, cx, cy);
  const p1End = polarToCartesian(outerR, startDeg + outerCornerDeg, cx, cy);

  const p2Start = polarToCartesian(outerR, endDeg - outerCornerDeg, cx, cy);
  const p3Ctrl = polarToCartesian(outerR, endDeg, cx, cy);
  const p3End = polarToCartesian(outerR - cornerInset, endDeg, cx, cy);

  const p4 = polarToCartesian(innerR + cornerInset, endDeg, cx, cy);
  const p5Ctrl = polarToCartesian(innerR, endDeg, cx, cy);
  const p5End = polarToCartesian(innerR, endDeg - innerCornerDeg, cx, cy);

  const p6Start = polarToCartesian(innerR, startDeg + innerCornerDeg, cx, cy);
  const p7Ctrl = polarToCartesian(innerR, startDeg, cx, cy);
  const p7End = polarToCartesian(innerR + cornerInset, startDeg, cx, cy);

  return [
    `M ${fmtNum(p0.x)} ${fmtNum(p0.y)}`,
    `Q ${fmtNum(p1Ctrl.x)} ${fmtNum(p1Ctrl.y)} ${fmtNum(p1End.x)} ${fmtNum(p1End.y)}`,
    `A ${fmtNum(outerR)} ${fmtNum(outerR)} 0 ${largeArc} 1 ${fmtNum(p2Start.x)} ${fmtNum(p2Start.y)}`,
    `Q ${fmtNum(p3Ctrl.x)} ${fmtNum(p3Ctrl.y)} ${fmtNum(p3End.x)} ${fmtNum(p3End.y)}`,
    `L ${fmtNum(p4.x)} ${fmtNum(p4.y)}`,
    `Q ${fmtNum(p5Ctrl.x)} ${fmtNum(p5Ctrl.y)} ${fmtNum(p5End.x)} ${fmtNum(p5End.y)}`,
    `A ${fmtNum(innerR)} ${fmtNum(innerR)} 0 ${largeArc} 0 ${fmtNum(p6Start.x)} ${fmtNum(p6Start.y)}`,
    `Q ${fmtNum(p7Ctrl.x)} ${fmtNum(p7Ctrl.y)} ${fmtNum(p7End.x)} ${fmtNum(p7End.y)}`,
    "Z",
  ].join(" ");
}

export function getSectorCenter(
  index: number,
  count: number,
  labelRadius: number = (WHEEL_INNER_RADIUS + WHEEL_OUTER_RADIUS) / 2,
  cx: number = WHEEL_CENTER,
  cy: number = WHEEL_CENTER,
): Point2D {
  const { midDeg } = getSectorAngles(index, count);
  return polarToCartesian(labelRadius, midDeg, cx, cy);
}

export function normalizeAngleDeg(deg: number): number {
  return ((deg % 360) + 360) % 360;
}

export function shortestAngularDistanceDeg(a: number, b: number): number {
  const diff = Math.abs(normalizeAngleDeg(a) - normalizeAngleDeg(b));
  return diff > 180 ? 360 - diff : diff;
}

/**
 * Polar angle hit-testing with inner dead-zone (r < 68px), outer max radius (210px),
 * and 3° angular hysteresis to prevent rapid pointer sweeps from flickering on boundaries.
 */
export function getSectorAtPoint(
  x: number,
  y: number,
  count: number,
  options: HitTestOptions = {},
): number {
  if (count <= 0) return -1;
  const cx = options.cx ?? WHEEL_CENTER;
  const cy = options.cy ?? WHEEL_CENTER;
  const innerDead = options.innerDeadZoneRadius ?? WHEEL_DEAD_ZONE_RADIUS;
  const outerMax = options.outerMaxRadius ?? WHEEL_OUTER_MAX_RADIUS;
  const prev = options.previousIndex ?? -1;
  const hysteresis = options.hysteresisDeg ?? WHEEL_HYSTERESIS_DEG;

  const dx = x - cx;
  const dy = y - cy;
  const r = Math.hypot(dx, dy);
  if (!Number.isFinite(r) || r < innerDead || r > outerMax) {
    return -1;
  }

  const deg = normalizeAngleDeg((Math.atan2(dy, dx) * 180) / Math.PI + 90);
  const span = 360 / count;
  const halfSpan = span / 2;
  const rawIndex = Math.floor(((deg + halfSpan) % 360) / span) % count;

  if (prev >= 0 && prev < count && prev !== rawIndex && hysteresis > 0) {
    const clampedHysteresis = Math.min(hysteresis, halfSpan * 0.45);
    const prevMid = prev * span;
    const distFromPrevMid = shortestAngularDistanceDeg(deg, prevMid);
    if (distFromPrevMid <= halfSpan + clampedHysteresis) {
      return prev;
    }
  }

  return rawIndex;
}

/**
 * Port of `Wheel.ProgressArcData`.
 */
export function progressArcSvgPath(
  percent: number,
  radius: number = WHEEL_PROGRESS_RADIUS,
  cx: number = WHEEL_CENTER,
  cy: number = WHEEL_CENTER,
): string {
  const clamped = Math.max(0.8, Math.min(99.9, percent));
  const angleDeg = clamped * 3.6;
  const start = polarToCartesian(radius, 0, cx, cy);
  const end = polarToCartesian(radius, angleDeg, cx, cy);
  const largeArc = clamped > 50 ? 1 : 0;
  return `M ${fmtNum(start.x)} ${fmtNum(start.y)} A ${fmtNum(radius)} ${fmtNum(radius)} 0 ${largeArc} 1 ${fmtNum(end.x)} ${fmtNum(end.y)}`;
}

export function progressTipPoint(
  percent: number,
  radius: number = WHEEL_PROGRESS_RADIUS,
  cx: number = WHEEL_CENTER,
  cy: number = WHEEL_CENTER,
): Point2D {
  const clamped = Math.max(0.8, Math.min(99.9, percent));
  return polarToCartesian(radius, clamped * 3.6, cx, cy);
}

export function progressRaySegment(
  percent: number,
  innerRadius: number = WHEEL_HUB_RADIUS - 2,
  outerRadius: number = WHEEL_PROGRESS_RADIUS,
  cx: number = WHEEL_CENTER,
  cy: number = WHEEL_CENTER,
): { x1: number; y1: number; x2: number; y2: number } {
  const clamped = Math.max(0.8, Math.min(99.9, percent));
  const angleDeg = clamped * 3.6;
  const p1 = polarToCartesian(innerRadius, angleDeg, cx, cy);
  const p2 = polarToCartesian(outerRadius, angleDeg, cx, cy);
  return { x1: p1.x, y1: p1.y, x2: p2.x, y2: p2.y };
}

/**
 * Splits arbitrary sector lists into pages of up to 8 sectors.
 * - <= 8 items: 1 page with all items directly.
 * - > 8 items: page 0 holds 7 items + `More ▸`; middle pages hold 6 items + `Back ◂` + `More ▸`;
 *   final page holds up to 7 remaining items + `Back ◂`.
 */
export function paginateWheelItems<T>(
  items: readonly T[],
  requestedPage: number = 0,
  maxPerPage: number = MAX_SECTORS_PER_PAGE,
): PaginatedRingPage<T> {
  const cap = Math.max(3, maxPerPage);
  if (items.length <= cap) {
    return {
      page: 0,
      totalPages: 1,
      slots: items.map((item, globalIndex) => ({
        kind: "item",
        item,
        globalIndex,
      })),
    };
  }

  const pages: PaginatedSlot<T>[][] = [];
  let cursor = 0;

  while (cursor < items.length) {
    const isFirst = pages.length === 0;
    const remaining = items.length - cursor;
    const slotsForItemsWithBackOnly = cap - 1;

    if (isFirst) {
      const take = cap - 1;
      const chunk = items.slice(cursor, cursor + take);
      const slots: PaginatedSlot<T>[] = chunk.map((item, idx) => ({
        kind: "item",
        item,
        globalIndex: cursor + idx,
      }));
      slots.push({ kind: "more", targetPage: 1 });
      pages.push(slots);
      cursor += take;
    } else if (remaining <= slotsForItemsWithBackOnly) {
      const chunk = items.slice(cursor);
      const slots: PaginatedSlot<T>[] = chunk.map((item, idx) => ({
        kind: "item",
        item,
        globalIndex: cursor + idx,
      }));
      slots.push({ kind: "back", targetPage: pages.length - 1 });
      pages.push(slots);
      cursor = items.length;
    } else {
      const take = cap - 2;
      const chunk = items.slice(cursor, cursor + take);
      const slots: PaginatedSlot<T>[] = chunk.map((item, idx) => ({
        kind: "item",
        item,
        globalIndex: cursor + idx,
      }));
      const pageIdx = pages.length;
      slots.push({ kind: "back", targetPage: pageIdx - 1 });
      slots.push({ kind: "more", targetPage: pageIdx + 1 });
      pages.push(slots);
      cursor += take;
    }
  }

  const safePage = Math.max(0, Math.min(pages.length - 1, Math.trunc(requestedPage)));
  return {
    page: safePage,
    totalPages: pages.length,
    slots: pages[safePage],
  };
}

export interface SectorAnnouncementItem {
  subtitle: string;
  enabled?: boolean;
  disabledReason?: string;
}

export function moveFormatToIndex(
  formats: readonly string[],
  fromIndex: number,
  toIndex: number,
): string[] {
  const list = [...formats];
  if (list.length <= 1) return list;
  const from = Math.trunc(fromIndex);
  const to = Math.max(0, Math.min(list.length - 1, Math.trunc(toIndex)));
  if (from < 0 || from >= list.length || from === to) {
    return list;
  }
  const [moved] = list.splice(from, 1);
  list.splice(to, 0, moved);
  return list;
}

export function reorderFormatList(
  formats: readonly string[],
  index: number,
  direction: -1 | 1,
): string[] {
  return moveFormatToIndex(formats, index, index + direction);
}

export function formatSectorAriaLabel(
  slot: PaginatedSlot<SectorAnnouncementItem>,
  index: number,
  total: number,
): string {
  const safeTotal = Math.max(1, Math.trunc(total));
  const sectorNum = Math.max(1, Math.min(safeTotal, Math.trunc(index) + 1));
  if (slot.kind === "more") {
    return `More options, page ${slot.targetPage + 1}, sector ${sectorNum} of ${safeTotal}`;
  }
  if (slot.kind === "back") {
    return `Previous options, page ${slot.targetPage + 1}, sector ${sectorNum} of ${safeTotal}`;
  }
  const base = `${slot.item.subtitle}, sector ${sectorNum} of ${safeTotal}`;
  if (slot.item.enabled === false) {
    const reason = slot.item.disabledReason?.trim();
    return reason ? `${base}, unavailable (${reason})` : `${base}, unavailable`;
  }
  return base;
}

export function formatWheelOpenAnnouncement(params: {
  mode: "convert" | "new";
  fileCount: number;
  optionCount: number;
  page?: number;
  totalPages?: number;
}): string {
  const page = Math.max(0, Math.trunc(params.page ?? 0));
  const totalPages = Math.max(1, Math.trunc(params.totalPages ?? 1));
  const pageSuffix = totalPages > 1 ? `, page ${page + 1} of ${totalPages}` : "";
  if (params.mode === "new") {
    return `Create New File wheel opened, ${params.optionCount} options${pageSuffix}.`;
  }
  if (params.fileCount <= 0) {
    return "Convert wheel opened. No files selected yet.";
  }
  const noun = params.fileCount === 1 ? "file" : "files";
  if (params.optionCount === 0) {
    return `Convert wheel opened for ${params.fileCount} ${noun}. No compatible output formats.`;
  }
  return `Convert wheel opened for ${params.fileCount} ${noun}, ${params.optionCount} options${pageSuffix}.`;
}

export function formatPageFlipAnnouncement(page: number, totalPages: number): string {
  const safeTotal = Math.max(1, Math.trunc(totalPages));
  const safePage = Math.max(0, Math.min(safeTotal - 1, Math.trunc(page)));
  return `Page ${safePage + 1} of ${safeTotal}`;
}

export function getProgressMilestoneBucket(percent: number): number {
  const clamped = Math.max(0, Math.min(100, Math.round(percent)));
  if (clamped >= 75) return 75;
  if (clamped >= 50) return 50;
  if (clamped >= 25) return 25;
  return 0;
}

export function formatProgressMilestoneAnnouncement(
  targetFormat: string,
  bucketPercent: number,
): string {
  const fmt = targetFormat.trim().toUpperCase();
  if (bucketPercent <= 0) {
    return `Converting to ${fmt}`;
  }
  return `Converting to ${fmt}, ${bucketPercent}% complete`;
}

export function formatCompletionAnnouncement(params: {
  ok: boolean;
  succeeded?: number;
  failed?: number;
  canceled?: boolean;
  outputFormat?: string;
  createdLabel?: string;
  errorMessage?: string;
}): string {
  if (params.createdLabel) {
    return `Created ${params.createdLabel}.`;
  }
  if (params.canceled) {
    return "Conversion canceled.";
  }
  if (params.ok) {
    const count = params.succeeded ?? 1;
    const fmt = params.outputFormat ? ` to ${params.outputFormat.toUpperCase()}` : "";
    return `Converted ${count} ${count === 1 ? "file" : "files"}${fmt}.`;
  }
  const reason = params.errorMessage?.trim();
  if (reason) {
    return `Conversion failed: ${reason}`;
  }
  const failed = params.failed ?? 1;
  return `Conversion failed for ${failed} ${failed === 1 ? "file" : "files"}.`;
}
