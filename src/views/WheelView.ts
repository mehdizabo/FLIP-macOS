import {
  localizeAvailabilityMessage,
  localizeDisabledReason,
  localizeNewKindName,
  normalizeLanguage,
  t,
} from "../i18n";
import { getFormatMeta, getNewKindMeta, UI_ICONS } from "../icons";
import {
  cancelConversion,
  copyDiagnosticsToClipboard,
  dismissWheel,
  escapeHtml,
  events,
  executeConversion,
  executeCreateNew,
  formatError,
  getNewFileKinds,
  getPreferences,
  getThumbnail,
  getWheelState,
  inspectFiles,
  isTauriRuntime,
  openBatchWithFiles,
  openWheelMode,
  playWheelFeedback,
  subscribeNativeDragDrop,
  switchWheelMode,
  basename,
  dirname,
  extLower,
} from "../ipc";
import type {
  AppPreferences,
  ConversionCompletePayload,
  ConversionResult,
  NewFileKindInfo,
  UserTemplateInfo,
  WheelMode,
  WheelStatePayload,
} from "../types";
import {
  DRAG_DWELL_MS,
  formatCompletionAnnouncement,
  formatPageFlipAnnouncement,
  formatProgressMilestoneAnnouncement,
  formatSectorAriaLabel,
  formatWheelOpenAnnouncement,
  getProgressMilestoneBucket,
  getSectorAtPoint,
  getSectorCenter,
  orderFormats,
  paginateWheelItems,
  progressArcSvgPath,
  progressRaySegment,
  progressTipPoint,
  sectorSvgPath,
  WHEEL_CANVAS_SIZE,
  WHEEL_CENTER,
  WHEEL_OUTER_MAX_RADIUS,
  WHEEL_OUTER_RADIUS,
  WHEEL_PROGRESS_RADIUS,
  type PaginatedSlot,
} from "../wheelGeometry";
import {
  createWheelSpringField,
  type WheelSpringField,
  type WheelSpringStepResult,
} from "../wheelPhysics";

interface WheelActionItem {
  id: string;
  label: string;
  subtitle: string;
  accent: string;
  iconSvg: string;
  enabled: boolean;
  disabledReason?: string;
}

type WheelPhase = "idle" | "converting" | "success" | "error";

export function mountWheelView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "wheel";
  if (!isTauriRuntime()) {
    document.body.classList.add("wheel-browser-preview");
  } else {
    document.body.classList.remove("wheel-browser-preview");
  }

  let disposed = false;
  let prefs: AppPreferences | null = null;
  let builtInKinds: NewFileKindInfo[] = [];
  let userTemplates: UserTemplateInfo[] = [];

  let state: WheelStatePayload = {
    mode: "convert",
    files: [],
    isDrag: false,
    targetFolder: "/Users/demo/Desktop",
    compatibleFormats: [],
    batchActions: [],
    originX: WHEEL_CENTER,
    originY: WHEEL_CENTER,
    availabilityMessage: "",
    family: null,
  };

  let page = 0;
  let slots: PaginatedSlot<WheelActionItem>[] = [];
  let totalPages = 1;
  let activeIndex = -1;
  let phase: WheelPhase = "idle";
  let closing = false;
  let isDragHovering = false;
  let stateEpoch = 0;

  // Progress animation state
  let displayedPercent = 4;
  let targetPercent = 12;
  let activeActionLabel = "";
  let stepLabel = "";
  let successLocationLabel = "";
  let lastCreatedPath = "";
  let lastErrorMsg = "";
  let diagBtnState: "idle" | "copying" | "copied" | "failed" = "idle";
  let diagBtnFailReason = "";
  let lastResults: ConversionResult[] = [];
  let progressRaf: number | null = null;
  let autoCloseTimer: number | null = null;
  let dismissTimer: number | null = null;
  let diagResetTimer: number | null = null;
  let openedAtMs = 0;

  const lang = () => normalizeLanguage(prefs?.language);

  // QuickLook thumbnail cache for current primary file
  let thumbnailPath = "";
  let thumbnailDataUrl: string | null = null;

  // 300ms drag dwell on pagination sectors
  let dwellTimer: number | null = null;
  let dwellSlotIndex = -1;
  let latchedDwellSlot = -1;
  let conversionInFlight = false;
  let receivedLiveState = false;
  let lastDragCoords: { x: number; y: number } | null = null;
  let lastAnnouncedMilestone = -1;
  let activeTargetFormat = "";
  let currentItemsCount = 0;
  let rawMode = false;
  let lastDomPointerMoveAt = 0;
  let lastMonitorPointerMoveAt = 0;
  let lastReleaseHandledAt = 0;
  let lastMonitorPointerPos: { x: number; y: number } | null = null;
  let lastCanvasPointerPos: { x: number; y: number } | null = null;
  let lastSvgPointerDownAt = 0;
  let isHoveringControl = false;
  let renderedHubKey = "";
  let renderedSlotsSignature = "";
  const handledPointerEvents = new WeakSet<Event>();

  // Static shell DOM
  root.innerHTML = `
    <div class="wheel-stage" role="dialog" aria-label="${escapeHtml(t(lang(), "wheel.ariaLabel"))}">
      <div class="sr-only" id="wheel-live-announcer" aria-live="polite" aria-atomic="true"></div>
      <div class="wheel-shell" id="wheel-shell">
        <div class="wheel-top-bar">
          <div class="wheel-mode-pill" role="tablist" aria-label="${escapeHtml(t(lang(), "wheel.modeListLabel"))}">
            <button type="button" class="wheel-mode-btn" data-mode="convert" role="tab">
              ${UI_ICONS.convert}<span data-mode-label="convert">${escapeHtml(t(lang(), "wheel.modeConvert"))}</span>
            </button>
            <button type="button" class="wheel-mode-btn" data-mode="new" role="tab">
              ${UI_ICONS.newFile}<span data-mode-label="new">${escapeHtml(t(lang(), "wheel.modeNew"))}</span>
            </button>
            <span class="wheel-mode-tab-hint">Tab</span>
          </div>
          <button type="button" class="wheel-raw-toggle" id="wheel-raw-toggle" aria-pressed="false">
            <span class="wheel-raw-dot" aria-hidden="true"></span>
            <span id="wheel-raw-toggle-label">${escapeHtml(t(lang(), "wheel.mode.smart"))}</span>
            <kbd class="wheel-raw-kbd">R</kbd>
          </button>
        </div>

        <div class="wheel-glass-ring" aria-hidden="true"></div>
        <div class="wheel-chromatic-rim" aria-hidden="true"></div>
        <div class="wheel-glass-layer" id="wheel-glass-layer" aria-hidden="true"></div>

        <svg class="wheel-svg" id="wheel-svg" viewBox="0 0 ${WHEEL_CANVAS_SIZE} ${WHEEL_CANVAS_SIZE}" width="${WHEEL_CANVAS_SIZE}" height="${WHEEL_CANVAS_SIZE}">
          <defs>
            <linearGradient id="flip-sector-rest-grad" x1="0%" y1="0%" x2="0%" y2="100%">
              <stop offset="0%" stop-color="rgba(48, 52, 64, 0.94)" />
              <stop offset="100%" stop-color="rgba(26, 29, 38, 0.96)" />
            </linearGradient>
            <linearGradient id="flip-sector-nav-grad" x1="0%" y1="0%" x2="0%" y2="100%">
              <stop offset="0%" stop-color="rgba(54, 60, 76, 0.95)" />
              <stop offset="100%" stop-color="rgba(29, 33, 44, 0.96)" />
            </linearGradient>
            <linearGradient id="flip-sector-active-grad" x1="0%" y1="0%" x2="0%" y2="100%">
              <stop offset="0%" stop-color="rgba(68, 76, 94, 0.98)" />
              <stop offset="100%" stop-color="rgba(38, 43, 56, 0.98)" />
            </linearGradient>
            <linearGradient id="flip-progress-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="#0a84ff" />
              <stop offset="100%" stop-color="#5ac8fa" />
            </linearGradient>
          </defs>
          <g id="wheel-sectors-layer"></g>
          <g id="wheel-progress-layer" style="display:none">
            <circle class="wheel-progress-track" cx="${WHEEL_CENTER}" cy="${WHEEL_CENTER}" r="${WHEEL_PROGRESS_RADIUS}" />
            <line class="wheel-progress-ray" id="wheel-progress-ray" x1="${WHEEL_CENTER}" y1="${WHEEL_CENTER - WHEEL_PROGRESS_RADIUS + 8}" x2="${WHEEL_CENTER}" y2="${WHEEL_CENTER - WHEEL_PROGRESS_RADIUS}" />
            <path class="wheel-progress-arc" id="wheel-progress-arc" d="" />
            <circle class="wheel-progress-tip" id="wheel-progress-tip" cx="${WHEEL_CENTER}" cy="${WHEEL_CENTER - WHEEL_PROGRESS_RADIUS}" r="4.5" />
          </g>
        </svg>

        <div class="wheel-hub-tether" id="wheel-hub-tether" aria-hidden="true"></div>
        <div class="wheel-hub" id="wheel-hub"></div>
        <div class="wheel-page-dots" id="wheel-page-dots" style="display:none"></div>
      </div>
      ${
        isTauriRuntime()
          ? ""
          : `<div class="wheel-preview-dock" id="wheel-preview-dock">
              <span data-preview-label>${escapeHtml(t(lang(), "wheel.previewLabel"))}</span>
              <button type="button" data-preview="image">${escapeHtml(t(lang(), "wheel.previewImage"))}</button>
              <button type="button" data-preview="pdf12">${escapeHtml(t(lang(), "wheel.previewPdf"))}</button>
              <button type="button" data-preview="new">${escapeHtml(t(lang(), "wheel.previewNew"))}</button>
              <button type="button" data-preview="settings">${escapeHtml(t(lang(), "wheel.previewSettings"))}</button>
              <button type="button" data-preview="batch">${escapeHtml(t(lang(), "wheel.previewBatch"))}</button>
            </div>`
      }
    </div>
  `;

  const stageEl = root.querySelector<HTMLElement>(".wheel-stage")!;
  const announcerEl = root.querySelector<HTMLElement>("#wheel-live-announcer")!;
  const shellEl = root.querySelector<HTMLElement>("#wheel-shell")!;
  const glassLayerEl = root.querySelector<HTMLElement>("#wheel-glass-layer")!;
  const svgEl = root.querySelector<SVGSVGElement>("#wheel-svg")!;
  const sectorsLayer = root.querySelector<SVGGElement>("#wheel-sectors-layer")!;
  const progressLayer = root.querySelector<SVGGElement>("#wheel-progress-layer")!;
  const progressArcEl = root.querySelector<SVGPathElement>("#wheel-progress-arc")!;
  const progressRayEl = root.querySelector<SVGLineElement>("#wheel-progress-ray")!;
  const progressTipEl = root.querySelector<SVGCircleElement>("#wheel-progress-tip")!;
  const hubTetherEl = root.querySelector<HTMLElement>("#wheel-hub-tether")!;
  const hubEl = root.querySelector<HTMLElement>("#wheel-hub")!;
  const pageDotsEl = root.querySelector<HTMLElement>("#wheel-page-dots")!;
  const topBarEl = root.querySelector<HTMLElement>(".wheel-top-bar");
  const modeButtons = Array.from(root.querySelectorAll<HTMLButtonElement>(".wheel-mode-btn"));
  const rawToggleBtn = root.querySelector<HTMLButtonElement>("#wheel-raw-toggle")!;
  const rawToggleLabelEl = root.querySelector<HTMLElement>("#wheel-raw-toggle-label")!;

  let sectorGroups: SVGGElement[] = [];
  let sectorBodies: Array<SVGGElement | null> = [];
  let glassPucks: HTMLElement[] = [];
  let springField: WheelSpringField = createWheelSpringField([]);
  let physicsRaf: number | null = null;
  let physicsLastTs = 0;
  let currentPointerCanvas: { x: number; y: number } | null = null;
  const prefersReducedMotion = () =>
    typeof window.matchMedia === "function" &&
    Boolean(window.matchMedia("(prefers-reduced-motion: reduce)")?.matches);

  function clearDwell(): void {
    if (dwellTimer !== null) {
      window.clearTimeout(dwellTimer);
      dwellTimer = null;
    }
    dwellSlotIndex = -1;
  }

  function announce(message: string, options: { force?: boolean } = {}): void {
    const trimmed = message.trim();
    if (!trimmed) return;
    if (announcerEl.textContent === trimmed) {
      if (!options.force) return;
      announcerEl.textContent = "";
      window.requestAnimationFrame(() => {
        if (!disposed) {
          announcerEl.textContent = trimmed;
        }
      });
      return;
    }
    announcerEl.textContent = trimmed;
  }

  function stopProgressLoop(): void {
    if (progressRaf !== null) {
      window.cancelAnimationFrame(progressRaf);
      progressRaf = null;
    }
  }

  function clearAutoClose(): void {
    if (autoCloseTimer !== null) {
      window.clearTimeout(autoCloseTimer);
      autoCloseTimer = null;
    }
  }

  function clearDismissTimer(): void {
    if (dismissTimer !== null) {
      window.clearTimeout(dismissTimer);
      dismissTimer = null;
    }
  }

  function clearDiagResetTimer(): void {
    if (diagResetTimer !== null) {
      window.clearTimeout(diagResetTimer);
      diagResetTimer = null;
    }
  }

  function getPreferredListForState(): string[] | undefined {
    if (!prefs || !state.family || state.family === "archive") return undefined;
    const key = state.family === "iwork" ? "document" : state.family;
    return prefs.preferredFormats[key];
  }

  function buildActionItems(): WheelActionItem[] {
    const l = lang();
    if (state.mode === "new") {
      const visibleOrder = prefs?.newFileVisibleKinds ?? [];
      const orderRank = new Map<string, number>();
      visibleOrder.forEach((id, idx) => orderRank.set(id, idx));

      const sortedBuiltIns = [...builtInKinds].sort((a, b) => {
        const ra = orderRank.has(a.id) ? orderRank.get(a.id)! : 100 + a.page;
        const rb = orderRank.has(b.id) ? orderRank.get(b.id)! : 100 + b.page;
        return ra - rb;
      });

      const items: WheelActionItem[] = sortedBuiltIns.map((k) => {
        const localizedName = localizeNewKindName(l, k.id, k.name);
        const meta = getNewKindMeta(k.id, localizedName, k.extension, l);
        return {
          id: k.id,
          label: meta.shortLabel,
          subtitle:
            k.id === "clipboard"
              ? t(l, "wheel.fromClipboard")
              : t(l, "wheel.newKindSubtitle", { name: localizedName }),
          accent: meta.accent,
          iconSvg: meta.iconSvg,
          enabled: k.available,
          disabledReason: localizeDisabledReason(l, k.disabledReason),
        };
      });

      for (const tmpl of userTemplates) {
        const meta = getNewKindMeta(tmpl.id, tmpl.name, tmpl.extension);
        const templateLabel = tmpl.group ? `${tmpl.group}/${tmpl.name}` : tmpl.name;
        items.push({
          id: tmpl.id,
          label: tmpl.name,
          subtitle: t(l, "wheel.templateSubtitle", { label: templateLabel }),
          accent: meta.accent,
          iconSvg: meta.iconSvg,
          enabled: true,
        });
      }

      return items;
    }

    const orderedFormats = orderFormats(
      state.compatibleFormats,
      getPreferredListForState(),
    );
    const allTargets = Array.from(new Set([...state.batchActions, ...orderedFormats]));

    return allTargets.map((fmt) => {
      const meta = getFormatMeta(fmt, l);
      return {
        id: fmt,
        label: meta.shortLabel,
        subtitle: meta.title,
        accent: meta.accent,
        iconSvg: meta.iconSvg,
        enabled: true,
      };
    });
  }

  function isSlotActionable(slot: PaginatedSlot<WheelActionItem> | undefined): boolean {
    if (!slot) return false;
    if (slot.kind === "more" || slot.kind === "back") return true;
    return slot.item.enabled;
  }

  function updateModePillUI(): void {
    const l = lang();
    shellEl.classList.toggle("is-drag-mode", Boolean(state.isDrag));
    stageEl.setAttribute("aria-label", t(l, "wheel.ariaLabel"));
    root.querySelector(".wheel-mode-pill")?.setAttribute("aria-label", t(l, "wheel.modeListLabel"));
    const convertLabelEl = root.querySelector<HTMLElement>('[data-mode-label="convert"]');
    if (convertLabelEl) convertLabelEl.textContent = t(l, "wheel.modeConvert");
    const newLabelEl = root.querySelector<HTMLElement>('[data-mode-label="new"]');
    if (newLabelEl) newLabelEl.textContent = t(l, "wheel.modeNew");

    const previewDockEl = root.querySelector<HTMLElement>("#wheel-preview-dock");
    if (previewDockEl) {
      const lbl = previewDockEl.querySelector<HTMLElement>("[data-preview-label]");
      if (lbl) lbl.textContent = t(l, "wheel.previewLabel");
      const map: Record<string, Parameters<typeof t>[1]> = {
        image: "wheel.previewImage",
        pdf12: "wheel.previewPdf",
        new: "wheel.previewNew",
        settings: "wheel.previewSettings",
        batch: "wheel.previewBatch",
      };
      for (const [k, msgKey] of Object.entries(map)) {
        const btn = previewDockEl.querySelector<HTMLElement>(`button[data-preview="${k}"]`);
        if (btn) btn.textContent = t(l, msgKey);
      }
    }

    const convertDisabled = state.files.length === 0 && !state.isDrag;
    for (const btn of modeButtons) {
      const btnMode = btn.dataset.mode as WheelMode;
      const active = state.mode === btnMode;
      btn.classList.toggle("is-active", active);
      btn.setAttribute("aria-selected", active ? "true" : "false");
      if (btnMode === "convert") {
        btn.disabled = false;
        btn.setAttribute("aria-disabled", convertDisabled ? "true" : "false");
        btn.title = convertDisabled
          ? t(l, "wheel.modeConvertDisabledTitle")
          : t(l, "wheel.modeConvertTitle");
      } else {
        btn.title = t(l, "wheel.modeNewTitle");
      }
    }

    const showRawToggle = state.mode === "convert";
    rawToggleBtn.style.display = showRawToggle ? "inline-flex" : "none";
    rawToggleBtn.classList.toggle("is-raw", rawMode);
    rawToggleBtn.setAttribute("aria-pressed", rawMode ? "true" : "false");
    rawToggleLabelEl.textContent = rawMode ? t(l, "wheel.mode.raw") : t(l, "wheel.mode.smart");
    rawToggleBtn.title = rawMode ? t(l, "wheel.mode.raw_tooltip") : t(l, "wheel.mode.smart_tooltip");
  }

  function setCssVar(el: Element, name: string, value: string): void {
    const st = (el as HTMLElement | SVGElement).style as CSSStyleDeclaration | undefined;
    if (st && typeof st.setProperty === "function") {
      st.setProperty(name, value);
    }
  }

  function stopPhysicsLoop(): void {
    if (physicsRaf !== null) {
      window.cancelAnimationFrame(physicsRaf);
      physicsRaf = null;
    }
    physicsLastTs = 0;
  }

  function applySpringFrame(frame: WheelSpringStepResult): void {
    sectorGroups.forEach((group, idx) => {
      const b = frame.bubbles[idx];
      if (!b) return;
      const puck = glassPucks[idx];
      const body = sectorBodies[idx];
      const tx = `${b.tx.toFixed(2)}px`;
      const ty = `${b.ty.toFixed(2)}px`;
      const scale = b.scale.toFixed(4);
      const prox = b.prox.toFixed(3);
      const lightX = `${b.lightX.toFixed(1)}%`;
      const lightY = `${b.lightY.toFixed(1)}%`;
      const proxRimPct = `${(b.prox * 65).toFixed(1)}%`;
      const proxIconPct = `${(b.prox * 85).toFixed(1)}%`;
      const proxEarPct = `${(18 + b.prox * 28).toFixed(1)}%`;
      const proxSheenOpacity = (0.22 + b.prox * 0.78).toFixed(3);
      const targets: Element[] = puck ? [group, puck] : [group];
      for (const el of targets) {
        setCssVar(el, "--bubble-tx", tx);
        setCssVar(el, "--bubble-ty", ty);
        setCssVar(el, "--bubble-scale", scale);
        setCssVar(el, "--bubble-prox", prox);
        setCssVar(el, "--light-x", lightX);
        setCssVar(el, "--light-y", lightY);
        setCssVar(el, "--prox-rim-pct", proxRimPct);
        setCssVar(el, "--prox-icon-pct", proxIconPct);
        setCssVar(el, "--prox-ear-pct", proxEarPct);
        setCssVar(el, "--prox-sheen-opacity", proxSheenOpacity);
      }
      if (puck && !puck.classList.contains("is-pulsing")) {
        puck.style.transform = `translate3d(${tx}, ${ty}, 0) scale(${scale})`;
      }
      if (body && !group.classList.contains("is-pulsing")) {
        body.style.transform = `translate(${tx}, ${ty}) scale(${scale})`;
      }
    });
    setCssVar(hubTetherEl, "--tether-angle", `${frame.tether.angleDeg.toFixed(2)}deg`);
    hubTetherEl.style.opacity = phase === "idle" ? frame.tether.opacity.toFixed(3) : "0";
  }

  function physicsTick(now: number): void {
    if (disposed || closing || phase !== "idle") {
      stopPhysicsLoop();
      return;
    }
    const dtSec = physicsLastTs > 0 ? (now - physicsLastTs) / 1000 : 0.016;
    physicsLastTs = now;
    const res = springField.step(dtSec);
    applySpringFrame(res);
    if (res.settled) {
      stopPhysicsLoop();
    } else {
      physicsRaf = window.requestAnimationFrame(physicsTick);
    }
  }

  function schedulePhysicsLoop(): void {
    if (physicsRaf === null && !disposed && !closing && phase === "idle") {
      physicsRaf = window.requestAnimationFrame(physicsTick);
    }
  }

  function stepPhysicsImmediate(): void {
    if (prefersReducedMotion()) {
      let res = springField.step(0.032);
      for (let k = 0; k < 20 && !res.settled; k++) {
        res = springField.step(0.032);
      }
      applySpringFrame(res);
      stopPhysicsLoop();
      return;
    }
    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    const rawDt = physicsLastTs > 0 ? (now - physicsLastTs) / 1000 : 0.016;
    const dt = rawDt < 0.002 ? 0.006 : Math.min(0.032, rawDt);
    physicsLastTs = now;
    const initial = springField.step(dt);
    applySpringFrame(initial);
    if (!initial.settled && physicsRaf === null && !disposed && !closing && phase === "idle") {
      schedulePhysicsLoop();
    }
  }

  function updateCursorProximity(canvasX: number | null, canvasY: number | null): void {
    const nextPos =
      canvasX !== null && canvasY !== null ? { x: canvasX, y: canvasY } : null;
    lastCanvasPointerPos = nextPos;
    currentPointerCanvas = nextPos;
    if (phase !== "idle") {
      springField.setPointer(null, null, -1);
      stopPhysicsLoop();
      hubTetherEl.style.opacity = "0";
      return;
    }
    springField.setPointer(canvasX, canvasY, activeIndex);
    stepPhysicsImmediate();
  }

  function renderGlassPuck(opts: {
    index: number;
    count: number;
    classes: string;
    accent?: string;
  }): string {
    const pos = getSectorCenter(opts.index, opts.count, 124);
    const accentAttr = opts.accent ? `;--sector-accent:${escapeHtml(opts.accent)}` : "";
    return `<div class="${opts.classes}" data-slot-index="${opts.index}" style="left:${pos.x.toFixed(2)}px;top:${pos.y.toFixed(2)}px${accentAttr}"></div>`;
  }

  function renderBubbleSlot(opts: {
    index: number;
    count: number;
    classes: string;
    style: string;
    attrs: string;
    icon: string;
    label: string;
    showKbd: boolean;
    includeSpec: boolean;
    earKind?: "more" | "back" | null;
  }): string {
    const d = sectorSvgPath(opts.index, opts.count, { cornerInset: 9 });
    const pos = getSectorCenter(opts.index, opts.count, 124);
    const cx = pos.x.toFixed(2);
    const cy = pos.y.toFixed(2);
    const midDeg = (opts.index * 360) / opts.count;
    const earMarkup = opts.earKind
      ? `<g class="wheel-bubble-ear-wrap" transform="translate(${cx}, ${cy}) rotate(${(midDeg - 90).toFixed(2)})">
           <path class="wheel-bubble-ear" d="M 30.54 -12.5 C 39 -10 43 -4.5 43 0 C 43 4.5 39 10 30.54 12.5 A 33 33 0 0 0 30.54 -12.5 Z" />
           <path class="wheel-bubble-ear-chevron" d="${opts.earKind === "back" ? "M 38.2 -3 L 35 0 L 38.2 3" : "M 35 -3 L 38.2 0 L 35 3"}" />
         </g>`
      : "";
    return `
      <g class="${opts.classes}"
         data-slot-index="${opts.index}"
         data-cx="${cx}"
         data-cy="${cy}"
         style="--bubble-cx:${cx}px;--bubble-cy:${cy}px;${opts.style}"
         ${opts.attrs}>
        <path class="wheel-sector-path" d="${d}" />
        <g class="wheel-bubble-body" style="transform-origin:${cx}px ${cy}px">
          ${earMarkup}
          <circle class="wheel-bubble-disc" cx="${cx}" cy="${cy}" r="33" />
          ${opts.includeSpec ? `<circle class="wheel-sector-spec" cx="${cx}" cy="${cy}" r="33" pathLength="100" />` : ""}
          <foreignObject x="${(pos.x - 35).toFixed(2)}" y="${(pos.y - 35).toFixed(2)}" width="70" height="70">
            <div class="wheel-sector-content">
              <span class="wheel-sector-icon">${opts.icon}</span>
              <span class="wheel-sector-label">${escapeHtml(opts.label)}</span>
              ${opts.showKbd ? `<span class="wheel-sector-kbd">${opts.index + 1}</span>` : ""}
            </div>
          </foreignObject>
        </g>
      </g>
    `;
  }

  function renderSectors(): void {
    const l = lang();
    const items = buildActionItems();
    currentItemsCount = items.length;
    const paginated = paginateWheelItems(items, page);
    page = paginated.page;
    totalPages = paginated.totalPages;
    slots = paginated.slots;

    // Reset active index if out of bounds
    if (activeIndex >= slots.length) {
      activeIndex = -1;
    }

    // Page dots indicator when paginated
    if (totalPages > 1) {
      pageDotsEl.style.display = "inline-flex";
      pageDotsEl.innerHTML = Array.from({ length: totalPages }, (_, idx) =>
        `<span class="wheel-page-dot ${idx === page ? "is-active" : ""}"></span>`,
      ).join("") + `<span>${page + 1}/${totalPages}</span>`;
    } else {
      pageDotsEl.style.display = "none";
      pageDotsEl.innerHTML = "";
    }

    // Empty Convert placeholder ring when 0 files or incompatible
    if (slots.length === 0) {
      renderedSlotsSignature = "";
      const placeholders = ["DOCX", "PDF", "MD", "HTML", "PNG", "JPG", "WEBP", "MP3"];
      glassLayerEl.innerHTML = placeholders
        .map((_, i) =>
          renderGlassPuck({
            index: i,
            count: placeholders.length,
            classes: "wheel-glass-puck is-disabled",
          }),
        )
        .join("");
      glassPucks = [];
      sectorsLayer.innerHTML = placeholders
        .map((label, i) =>
          renderBubbleSlot({
            index: i,
            count: placeholders.length,
            classes: "wheel-sector-group is-disabled",
            style: "",
            attrs: 'aria-hidden="true"',
            icon: getFormatMeta(label, l).iconSvg,
            label,
            showKbd: false,
            includeSpec: false,
          }),
        )
        .join("");
      sectorGroups = [];
      sectorBodies = [];
      springField = createWheelSpringField([]);
      stopPhysicsLoop();
      hubTetherEl.style.opacity = "0";
      return;
    }

    const slotSignature = JSON.stringify({
      mode: state.mode,
      lang: l,
      phase,
      page,
      totalPages,
      convertingLabel: phase === "converting" ? activeActionLabel : "",
      slots: slots.map((slot) =>
        slot.kind === "item"
          ? [
              "item",
              slot.item.id,
              slot.item.label,
              slot.item.enabled ? 1 : 0,
              slot.item.accent,
              slot.item.subtitle,
            ]
          : [slot.kind, slot.targetPage],
      ),
    });

    if (
      slotSignature === renderedSlotsSignature &&
      sectorGroups.length === slots.length &&
      glassPucks.length === slots.length &&
      sectorBodies.length === slots.length
    ) {
      updateSectorHighlightClasses();
      return;
    }

    renderedSlotsSignature = slotSignature;
    const count = slots.length;
    const puckMarkups: string[] = [];
    sectorsLayer.innerHTML = slots
      .map((slot, i) => {
        const isNav = slot.kind === "more" || slot.kind === "back";
        const enabled = isSlotActionable(slot);
        const label =
          slot.kind === "more"
            ? t(l, "wheel.moreSlot")
            : slot.kind === "back"
              ? t(l, "wheel.backSlot")
              : slot.item.label;
        const icon =
          slot.kind === "more"
            ? UI_ICONS.more
            : slot.kind === "back"
              ? UI_ICONS.back
              : slot.item.iconSvg;
        const accent =
          slot.kind === "more" || slot.kind === "back"
            ? "#38bdf8"
            : slot.item.accent;
        const ariaLabel = formatSectorAriaLabel(slot, i, count, l);
        const isActive = activeIndex === i && enabled && phase === "idle";
        const isDimmed = phase === "converting" && activeActionLabel !== label;

        const classes = [
          "wheel-sector-group",
          isNav ? "is-nav" : "",
          !enabled ? "is-disabled" : "",
          isActive ? "is-active" : "",
          isDimmed ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ");

        const puckClasses = [
          "wheel-glass-puck",
          isNav ? "is-nav" : "",
          !enabled ? "is-disabled" : "",
          isActive ? "is-active" : "",
          isDimmed ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ");

        puckMarkups.push(
          renderGlassPuck({
            index: i,
            count,
            classes: puckClasses,
            accent,
          }),
        );

        return renderBubbleSlot({
          index: i,
          count,
          classes,
          style: `--sector-accent:${escapeHtml(accent)}`,
          attrs: `role="button" tabindex="${enabled ? "0" : "-1"}" aria-disabled="${enabled ? "false" : "true"}" aria-current="${isActive ? "true" : "false"}" aria-label="${escapeHtml(ariaLabel)}"`,
          icon,
          label,
          showKbd: true,
          includeSpec: true,
          earKind: slot.kind === "more" ? "more" : slot.kind === "back" ? "back" : null,
        });
      })
      .join("");

    glassLayerEl.innerHTML = puckMarkups.join("");
    glassPucks = Array.from(glassLayerEl.querySelectorAll<HTMLElement>(".wheel-glass-puck"));
    sectorGroups = Array.from(sectorsLayer.querySelectorAll<SVGGElement>(".wheel-sector-group"));
    sectorBodies = sectorGroups.map((g) => g.querySelector<SVGGElement>(".wheel-bubble-body"));
    const centers = slots.map((_, i) => {
      const pos = getSectorCenter(i, count, 124);
      return { cx: pos.x, cy: pos.y };
    });
    springField = createWheelSpringField(centers);
    if (phase === "idle" && lastCanvasPointerPos) {
      updateCursorProximity(lastCanvasPointerPos.x, lastCanvasPointerPos.y);
    } else if (phase === "idle" && activeIndex >= 0) {
      springField.setPointer(null, null, activeIndex);
      stepPhysicsImmediate();
    } else {
      stopPhysicsLoop();
      hubTetherEl.style.opacity = "0";
    }
  }

  function updateSectorHighlightClasses(): void {
    sectorGroups.forEach((group, idx) => {
      const slot = slots[idx];
      const enabled = isSlotActionable(slot);
      const isActive = idx === activeIndex && enabled && phase === "idle";
      const isDwelling = idx === dwellSlotIndex && dwellTimer !== null;
      group.classList.toggle("is-active", isActive);
      group.classList.toggle("is-dwelling", isDwelling);
      group.setAttribute("aria-current", isActive ? "true" : "false");
      const puck = glassPucks[idx];
      if (puck) {
        puck.classList.toggle("is-active", isActive);
        puck.classList.toggle("is-dwelling", isDwelling);
      }
    });
  }

  function renderHub(): void {
    const l = lang();
    hubEl.classList.toggle("is-error-hub", phase === "error");
    const fileCount = state.files.length;
    const primaryName =
      fileCount === 0
        ? ""
        : fileCount === 1
          ? basename(state.files[0])
          : t(l, "wheel.filesCount", { count: fileCount });

    if (phase === "converting") {
      renderedHubKey = "";
      const shownPct = Math.max(1, Math.min(99, Math.round(displayedPercent)));
      hubEl.innerHTML = `
        <div class="wheel-hub-pct" id="wheel-hub-pct">${shownPct}%</div>
        <div class="wheel-hub-title" title="${escapeHtml(primaryName)}">${escapeHtml(primaryName || t(l, "wheel.converting"))}</div>
        <div class="wheel-hub-subtitle" id="wheel-hub-step">${escapeHtml(stepLabel || `→ ${activeActionLabel}`)}</div>
        <div class="wheel-hub-actions">
          <button type="button" class="wheel-hub-pill-btn is-danger" id="hub-cancel-btn">${escapeHtml(t(l, "common.cancel"))}</button>
        </div>
      `;
      hubEl.querySelector("#hub-cancel-btn")?.addEventListener("click", (e) => {
        e.stopPropagation();
        void cancelConversion();
      });
      return;
    }

    if (phase === "success") {
      renderedHubKey = "";
      const successSubtitle = successLocationLabel
        ? `${t(l, "wheel.completed")} · ${successLocationLabel}`
        : t(l, "wheel.completed");
      const successTitleAttr = lastCreatedPath
        ? ` title="${escapeHtml(lastCreatedPath)}"`
        : "";
      hubEl.innerHTML = `
        <svg class="wheel-check-svg" viewBox="0 0 36 36" aria-hidden="true">
          <circle class="wheel-check-circle" cx="18" cy="18" r="16" />
          <path class="wheel-check-mark" d="M10.5 18.5l5 5 10-10.5" />
        </svg>
        <div class="wheel-hub-title">${escapeHtml(activeActionLabel || t(l, "wheel.done"))}</div>
        <div class="wheel-hub-subtitle is-success"${successTitleAttr}>${escapeHtml(successSubtitle)}</div>
      `;
      return;
    }

    if (phase === "error") {
      renderedHubKey = "";
      const failTitle =
        state.mode === "new" ? t(l, "wheel.creationFailed") : t(l, "wheel.conversionFailed");
      const canOpenBatch = state.mode === "convert" && state.files.length > 0;
      const diagBtnLabel =
        diagBtnState === "copying"
          ? t(l, "common.copying")
          : diagBtnState === "copied"
            ? t(l, "common.copied")
            : diagBtnState === "failed"
              ? t(l, "common.copyFailed")
              : t(l, "common.copyDiagnostics");
      const diagBtnTitle =
        diagBtnState === "copied"
          ? t(l, "wheel.copyDiagSuccessTitle")
          : diagBtnState === "failed"
            ? diagBtnFailReason || t(l, "wheel.copyDiagFailTitle")
            : t(l, "wheel.copyDiagTitle");
      hubEl.innerHTML = `
        <div class="wheel-hub-title">${escapeHtml(failTitle)}</div>
        <div class="wheel-hub-subtitle is-error" title="${escapeHtml(lastErrorMsg)}">${escapeHtml(lastErrorMsg || t(l, "wheel.genericError"))}</div>
        <div class="wheel-hub-actions is-error-actions">
          <button type="button" class="wheel-hub-pill-btn is-diag-btn" id="hub-copy-diag" title="${escapeHtml(diagBtnTitle)}" ${diagBtnState === "copying" ? "disabled" : ""}>${escapeHtml(diagBtnLabel)}</button>
          <div class="wheel-hub-error-row">
            ${canOpenBatch ? `<button type="button" class="wheel-hub-pill-btn" id="hub-batch-details" title="${escapeHtml(t(l, "wheel.openBatchDetailsTitle"))}">${escapeHtml(t(l, "common.details"))}</button>` : ""}
            <button type="button" class="wheel-hub-pill-btn" id="hub-dismiss-btn">${escapeHtml(t(l, "common.close"))}</button>
          </div>
        </div>
      `;
      hubEl.querySelector("#hub-batch-details")?.addEventListener("click", (e) => {
        e.stopPropagation();
        void openBatchWithFiles(state.files, lastResults).then(() => {
          if (isTauriRuntime() && !disposed) requestDismiss("open-batch-details");
        });
      });
      hubEl.querySelector("#hub-copy-diag")?.addEventListener("click", async (e) => {
        e.stopPropagation();
        diagBtnState = "copying";
        diagBtnFailReason = "";
        renderHub();
        clearDiagResetTimer();
        try {
          const res = await copyDiagnosticsToClipboard();
          if (disposed || phase !== "error") return;
          if (res.ok) {
            diagBtnState = "copied";
            diagBtnFailReason = "";
          } else {
            diagBtnState = "failed";
            diagBtnFailReason = res.error || "";
          }
          renderHub();
          diagResetTimer = window.setTimeout(() => {
            diagResetTimer = null;
            if (!disposed && phase === "error") {
              diagBtnState = "idle";
              diagBtnFailReason = "";
              renderHub();
            }
          }, 2000);
        } catch (err) {
          if (disposed || phase !== "error") return;
          diagBtnState = "failed";
          diagBtnFailReason = formatError(err);
          renderHub();
          diagResetTimer = window.setTimeout(() => {
            diagResetTimer = null;
            if (!disposed && phase === "error") {
              diagBtnState = "idle";
              diagBtnFailReason = "";
              renderHub();
            }
          }, 2000);
        }
      });
      hubEl.querySelector("#hub-dismiss-btn")?.addEventListener("click", (e) => {
        e.stopPropagation();
        requestDismiss("error-dismiss");
      });
      return;
    }

    // Idle / Drag state
    const hoveredSlot = activeIndex >= 0 ? slots[activeIndex] : undefined;

    if (state.mode === "new") {
      const folderLabel = basename(state.targetFolder || "Desktop");
      let subtitle = t(l, "wheel.escToCancel");
      let subClass = "is-muted";
      if (hoveredSlot) {
        if (hoveredSlot.kind === "more") {
          subtitle = t(l, "wheel.moreTemplates");
          subClass = "";
        } else if (hoveredSlot.kind === "back") {
          subtitle = t(l, "wheel.prevItems");
          subClass = "";
        } else if (!hoveredSlot.item.enabled) {
          subtitle = hoveredSlot.item.disabledReason ?? t(l, "wheel.unavailable");
          subClass = "is-error";
        } else {
          subtitle = hoveredSlot.item.subtitle;
          subClass = "";
        }
      }
      const nextHubKey = `idle:new:${l}:${state.targetFolder}`;
      const existingTitle = hubEl.querySelector<HTMLElement>(".wheel-hub-title");
      const existingSubtitle = hubEl.querySelector<HTMLElement>(".wheel-hub-subtitle");
      if (renderedHubKey === nextHubKey && existingTitle && existingSubtitle) {
        existingTitle.textContent = folderLabel;
        existingTitle.title = state.targetFolder;
        existingSubtitle.textContent = subtitle;
        existingSubtitle.title = subtitle;
        existingSubtitle.className = subClass ? `wheel-hub-subtitle ${subClass}` : "wheel-hub-subtitle";
        return;
      }
      renderedHubKey = nextHubKey;
      hubEl.innerHTML = `
        <div class="wheel-hub-thumb-wrap">
          <div class="wheel-hub-thumb-fallback">${UI_ICONS.folder}</div>
        </div>
        <div class="wheel-hub-title" title="${escapeHtml(state.targetFolder)}">${escapeHtml(folderLabel)}</div>
        <div class="wheel-hub-subtitle ${subClass}" title="${escapeHtml(subtitle)}">${escapeHtml(subtitle)}</div>
      `;
      return;
    }

    // Convert mode hub
    if (fileCount === 0) {
      const emptyTitle = t(l, "wheel.dropFilesHere");
      const emptySub = t(l, "wheel.escToCancel");
      const nextHubKey = `idle:empty:${l}`;
      const existingTitle = hubEl.querySelector<HTMLElement>(".wheel-hub-title");
      const existingSubtitle = hubEl.querySelector<HTMLElement>(".wheel-hub-subtitle");
      if (renderedHubKey === nextHubKey && existingTitle && existingSubtitle) {
        existingTitle.textContent = emptyTitle;
        existingSubtitle.textContent = emptySub;
        existingSubtitle.className = "wheel-hub-subtitle is-muted";
        return;
      }
      renderedHubKey = nextHubKey;
      hubEl.innerHTML = `
        <div class="wheel-hub-thumb-wrap">
          <div class="wheel-hub-thumb-fallback">${UI_ICONS.drop}</div>
        </div>
        <div class="wheel-hub-title">${escapeHtml(emptyTitle)}</div>
        <div class="wheel-hub-subtitle is-muted">${escapeHtml(emptySub)}</div>
      `;
      return;
    }

    let subtitle = t(l, "wheel.escToCancel");
    let subtitleTitle = "";
    let subClass = "is-muted";
    if (state.compatibleFormats.length === 0 && state.batchActions.length === 0) {
      const rawMsg =
        localizeAvailabilityMessage(l, state.availabilityMessage) ||
        t(l, "wheel.noCommonFormat");
      subtitleTitle = rawMsg;
      const lines = rawMsg.split("\n").map((ln) => ln.trim()).filter(Boolean);
      subtitle =
        lines.length > 1 &&
        (/^\d+\s+files?\s+selected$/i.test(lines[0]) ||
          /^\d+\s+file\s+selezionat[io]$/i.test(lines[0]))
          ? lines.slice(1).join(" · ")
          : lines.join(" · ") || t(l, "wheel.noCommonFormat");
      subClass = "is-error";
    } else if (hoveredSlot) {
      if (hoveredSlot.kind === "more") {
        subtitle = t(l, "wheel.nextFormats");
        subClass = "";
      } else if (hoveredSlot.kind === "back") {
        subtitle = t(l, "wheel.prevFormats");
        subClass = "";
      } else if (!hoveredSlot.item.enabled) {
        subtitle = hoveredSlot.item.disabledReason ?? t(l, "wheel.unavailable");
        subClass = "is-error";
      } else {
        subtitle = hoveredSlot.item.subtitle;
        subClass = "";
      }
    } else if (rawMode) {
      subtitle = t(l, "wheel.mode.raw");
      subClass = "is-raw-hint";
    }

    const thumbVisual = thumbnailDataUrl
      ? `<img class="wheel-hub-thumb" src="${escapeHtml(thumbnailDataUrl)}" alt="" />`
      : `<div class="wheel-hub-thumb-fallback">${UI_ICONS.document}</div>`;

    const showMultiBtn =
      state.compatibleFormats.length > 0 || state.batchActions.length > 0;

    const nextHubKey = `idle:convert:${l}:${fileCount}:${state.files[0] ?? ""}:${thumbnailDataUrl ?? ""}:${showMultiBtn ? 1 : 0}`;
    const existingTitle = hubEl.querySelector<HTMLElement>(".wheel-hub-title");
    const existingSubtitle = hubEl.querySelector<HTMLElement>(".wheel-hub-subtitle");
    if (renderedHubKey === nextHubKey && existingTitle && existingSubtitle) {
      existingTitle.textContent = primaryName;
      existingTitle.title = state.files.join("\n");
      existingSubtitle.textContent = subtitle;
      existingSubtitle.title = subtitleTitle || subtitle;
      existingSubtitle.className = subClass ? `wheel-hub-subtitle ${subClass}` : "wheel-hub-subtitle";
      return;
    }
    renderedHubKey = nextHubKey;

    hubEl.innerHTML = `
      <div class="wheel-hub-thumb-wrap">
        ${fileCount > 1 ? '<div class="wheel-hub-thumb-stack"></div>' : ""}
        ${thumbVisual}
        ${fileCount > 1 ? `<span class="wheel-hub-count-badge">×${fileCount}</span>` : ""}
      </div>
      <div class="wheel-hub-title" title="${escapeHtml(state.files.join("\n"))}">${escapeHtml(primaryName)}</div>
      <div class="wheel-hub-subtitle ${subClass}" title="${escapeHtml(subtitleTitle || subtitle)}">${escapeHtml(subtitle)}</div>
      ${
        showMultiBtn
          ? `<div class="wheel-hub-actions">
               <button type="button" class="wheel-hub-pill-btn" id="hub-multi-btn" title="${escapeHtml(t(l, "wheel.multiBtnTitle"))}">${escapeHtml(t(l, "wheel.multiBtn"))}</button>
             </div>`
          : ""
      }
    `;

    hubEl.querySelector<HTMLButtonElement>("#hub-multi-btn")?.addEventListener("click", (e) => {
      e.stopPropagation();
      (e.currentTarget as HTMLButtonElement).blur();
      void openBatchWithFiles(state.files).then(() => {
        if (isTauriRuntime() && !disposed) requestDismiss("open-batch");
      });
    });
  }

  async function refreshThumbnail(): Promise<void> {
    if (state.files.length === 0) {
      thumbnailPath = "";
      thumbnailDataUrl = null;
      renderHub();
      return;
    }
    const first = state.files[0];
    if (first === thumbnailPath && thumbnailDataUrl) return;
    thumbnailPath = first;
    thumbnailDataUrl = null;
    const res = await getThumbnail(first, 128);
    if (disposed || state.files[0] !== first) return;
    if (res.ok && res.pngBase64) {
      const mime = res.pngBase64.startsWith("PHN2Zy")
        ? "image/svg+xml"
        : "image/png";
      thumbnailDataUrl = `data:${mime};base64,${res.pngBase64}`;
      renderHub();
    }
  }

  function setActiveIndex(nextIndex: number, options: { skipStep?: boolean } = {}): void {
    if (phase !== "idle") return;
    const bounded = nextIndex >= 0 && nextIndex < slots.length ? nextIndex : -1;
    if (bounded === activeIndex) return;

    activeIndex = bounded;
    if (!options.skipStep) {
      springField.setPointer(
        currentPointerCanvas?.x ?? null,
        currentPointerCanvas?.y ?? null,
        activeIndex,
      );
      stepPhysicsImmediate();
    }

    if (bounded !== latchedDwellSlot) {
      latchedDwellSlot = -1;
    }

    clearDwell();

    const slot = bounded >= 0 ? slots[bounded] : undefined;
    if (slot) {
      announce(formatSectorAriaLabel(slot, bounded, slots.length, lang()));
    }
    if (slot && isSlotActionable(slot)) {
      void playWheelFeedback("tick");
      if (
        (slot.kind === "more" || slot.kind === "back") &&
        (state.isDrag || isDragHovering) &&
        latchedDwellSlot !== bounded
      ) {
        dwellSlotIndex = bounded;
        const targetPage = slot.targetPage;
        dwellTimer = window.setTimeout(() => {
          if (disposed || phase !== "idle" || activeIndex !== dwellSlotIndex) return;
          latchedDwellSlot = dwellSlotIndex;
          page = targetPage;
          activeIndex = -1;
          void playWheelFeedback("level");
          renderSectors();
          renderHub();
          announce(formatPageFlipAnnouncement(page, totalPages, lang()), { force: true });
        }, DRAG_DWELL_MS);
      }
    }

    updateSectorHighlightClasses();
    renderHub();
  }

  function updateProgressVisuals(): void {
    const pct = Math.max(0.8, Math.min(99.9, displayedPercent));
    progressArcEl.setAttribute("d", progressArcSvgPath(pct, WHEEL_PROGRESS_RADIUS));
    const tip = progressTipPoint(pct, WHEEL_PROGRESS_RADIUS);
    progressTipEl.setAttribute("cx", tip.x.toFixed(2));
    progressTipEl.setAttribute("cy", tip.y.toFixed(2));
    const ray = progressRaySegment(pct, 60, WHEEL_PROGRESS_RADIUS);
    progressRayEl.setAttribute("x1", ray.x1.toFixed(2));
    progressRayEl.setAttribute("y1", ray.y1.toFixed(2));
    progressRayEl.setAttribute("x2", ray.x2.toFixed(2));
    progressRayEl.setAttribute("y2", ray.y2.toFixed(2));
  }

  function startConversionVisual(formatLabel: string): void {
    clearDwell();
    clearAutoClose();
    stopProgressLoop();
    stopPhysicsLoop();
    lastCanvasPointerPos = null;
    currentPointerCanvas = null;
    springField.snapToRest();
    hubTetherEl.style.opacity = "0";
    phase = "converting";
    activeActionLabel = formatLabel;
    activeTargetFormat = formatLabel;
    successLocationLabel = "";
    lastCreatedPath = "";
    lastAnnouncedMilestone = 0;
    stepLabel = `→ ${formatLabel}`;
    displayedPercent = 5;
    targetPercent = 14;

    progressLayer.style.display = "block";
    progressArcEl.classList.remove("is-success", "is-error");
    progressTipEl.style.display = "block";
    progressRayEl.style.display = "block";
    renderSectors();
    updateProgressVisuals();
    renderHub();
    announce(formatProgressMilestoneAnnouncement(formatLabel, 0, lang()), { force: true });

    const tick = () => {
      if (disposed || phase !== "converting") return;
      if (displayedPercent < targetPercent) {
        displayedPercent = Math.min(
          targetPercent,
          displayedPercent + Math.max(0.6, (targetPercent - displayedPercent) * 0.22),
        );
      } else if (displayedPercent < 88) {
        displayedPercent += (88 - displayedPercent) * 0.018;
      }
      updateProgressVisuals();
      const pctEl = hubEl.querySelector<HTMLElement>("#wheel-hub-pct");
      const stepEl = hubEl.querySelector<HTMLElement>("#wheel-hub-step");
      if (pctEl) {
        pctEl.textContent = `${Math.max(1, Math.min(99, Math.round(displayedPercent)))}%`;
      }
      if (stepEl && stepLabel) {
        stepEl.textContent = stepLabel;
      }
      progressRaf = window.requestAnimationFrame(tick);
    };
    progressRaf = window.requestAnimationFrame(tick);
  }

  function handleConversionComplete(payload: ConversionCompletePayload): void {
    conversionInFlight = false;
    stopProgressLoop();
    lastResults = payload.results ?? [];
    const failedUnit = lastResults.find((r) => Boolean(r.error));
    const ok = payload.success && !payload.error && !failedUnit;

    displayedPercent = 100;
    targetPercent = 100;
    progressLayer.style.display = "block";
    progressArcEl.setAttribute("d", progressArcSvgPath(99.9, WHEEL_PROGRESS_RADIUS));
    progressTipEl.style.display = "none";
    progressRayEl.style.display = "none";

    if (ok) {
      phase = "success";
      progressArcEl.classList.add("is-success");
      progressArcEl.classList.remove("is-error");
      void playWheelFeedback("success");
      renderHub();
      announce(
        formatCompletionAnnouncement({
          ok: true,
          succeeded: lastResults.length || state.files.length || 1,
          outputFormat: activeTargetFormat || activeActionLabel,
          lang: lang(),
        }),
        { force: true },
      );
      clearAutoClose();
      autoCloseTimer = window.setTimeout(() => {
        if (!disposed) requestDismiss("conversion-completed");
      }, 560);
    } else {
      phase = "error";
      progressArcEl.classList.add("is-error");
      progressArcEl.classList.remove("is-success");
      lastErrorMsg =
        payload.error || failedUnit?.error || t(lang(), "wheel.oneOrMoreFailed");
      const canceled = /cancel/i.test(lastErrorMsg);
      const failedCount = lastResults.filter((r) => Boolean(r.error)).length || 1;
      void playWheelFeedback("error");
      shellEl.classList.remove("is-error-shake");
      void shellEl.offsetWidth;
      shellEl.classList.add("is-error-shake");
      renderHub();
      announce(
        formatCompletionAnnouncement({
          ok: false,
          canceled,
          failed: failedCount,
          errorMessage: lastErrorMsg,
          lang: lang(),
        }),
        { force: true },
      );
    }
  }

  function getPhase(): WheelPhase {
    return phase;
  }

  async function commitFormatConversion(formatId: string, forceRaw = false): Promise<void> {
    if (getPhase() !== "idle" || state.files.length === 0) return;
    const filesSnapshot = [...state.files];
    const meta = getFormatMeta(formatId, lang());
    conversionInFlight = true;
    void playWheelFeedback("generic");
    startConversionVisual(meta.shortLabel);

    try {
      const res = await executeConversion(filesSnapshot, [formatId], {
        rawMode: Boolean(rawMode || forceRaw),
      });
      if (disposed) return;
      if (conversionInFlight && getPhase() === "converting") {
        handleConversionComplete(res);
      }
    } catch (err) {
      if (disposed) return;
      if (conversionInFlight && getPhase() === "converting") {
        handleConversionComplete({
          success: false,
          results: [],
          error: formatError(err),
        });
      }
    }
  }

  async function commitCreateNew(kind: WheelActionItem): Promise<void> {
    if (phase !== "idle" || !kind.enabled) return;
    clearDwell();
    clearAutoClose();
    stopProgressLoop();
    stopPhysicsLoop();
    lastCanvasPointerPos = null;
    currentPointerCanvas = null;
    springField.snapToRest();
    hubTetherEl.style.opacity = "0";
    conversionInFlight = false;
    void playWheelFeedback("generic");
    activeActionLabel = kind.label;
    activeTargetFormat = kind.label;
    successLocationLabel = "";
    lastCreatedPath = "";
    lastAnnouncedMilestone = 0;
    phase = "converting";
    stepLabel = kind.subtitle;
    displayedPercent = 40;
    targetPercent = 85;
    progressLayer.style.display = "block";
    progressArcEl.classList.remove("is-success", "is-error");
    progressTipEl.style.display = "block";
    progressRayEl.style.display = "block";
    renderSectors();
    updateProgressVisuals();
    renderHub();
    announce(t(lang(), "wheel.creatingKind", { label: kind.label }), { force: true });

    try {
      const res = await executeCreateNew(kind.id, state.targetFolder);
      if (disposed) return;
      phase = "success";
      const createdName = basename(res.path);
      const parentDir = dirname(res.path);
      activeActionLabel = createdName;
      lastCreatedPath = res.path;
      successLocationLabel =
        parentDir && parentDir !== "/"
          ? basename(parentDir)
          : res.path.startsWith("/")
            ? "/"
            : "";
      progressArcEl.classList.remove("is-error");
      progressArcEl.classList.add("is-success");
      progressArcEl.setAttribute("d", progressArcSvgPath(99.9, WHEEL_PROGRESS_RADIUS));
      progressTipEl.style.display = "none";
      progressRayEl.style.display = "none";
      void playWheelFeedback("success");
      renderHub();
      announce(
        formatCompletionAnnouncement({ ok: true, createdLabel: createdName, lang: lang() }),
        { force: true },
      );
      clearAutoClose();
      const closeDelayMs = res.renameTriggered ? 120 : 420;
      autoCloseTimer = window.setTimeout(() => {
        if (!disposed) requestDismiss("create-new-completed");
      }, closeDelayMs);
    } catch (err) {
      if (disposed) return;
      phase = "error";
      lastErrorMsg = formatError(err);
      progressArcEl.classList.remove("is-success");
      progressArcEl.classList.add("is-error");
      progressArcEl.setAttribute("d", progressArcSvgPath(99.9, WHEEL_PROGRESS_RADIUS));
      progressTipEl.style.display = "none";
      progressRayEl.style.display = "none";
      void playWheelFeedback("error");
      shellEl.classList.remove("is-error-shake");
      void shellEl.offsetWidth;
      shellEl.classList.add("is-error-shake");
      renderHub();
      announce(
        formatCompletionAnnouncement({ ok: false, errorMessage: lastErrorMsg, lang: lang() }),
        { force: true },
      );
    }
  }

  function activateSlot(idx: number, options: { forceRaw?: boolean } = {}): void {
    if (phase !== "idle") return;
    const slot = slots[idx];
    if (!slot) return;

    if (slot.kind === "more" || slot.kind === "back") {
      const wasFocusInsideSectors =
        document.activeElement instanceof Element &&
        sectorsLayer.contains(document.activeElement);
      page = slot.targetPage;
      activeIndex = -1;
      clearDwell();
      void playWheelFeedback("level");
      renderSectors();
      renderHub();
      announce(formatPageFlipAnnouncement(page, totalPages, lang()), { force: true });
      if (wasFocusInsideSectors) {
        const firstFocusIdx = slots.findIndex((s) => isSlotActionable(s));
        if (firstFocusIdx >= 0) {
          sectorGroups[firstFocusIdx]?.focus();
        }
      }
      return;
    }

    if (!slot.item.enabled) return;

    const group = sectorGroups[idx];
    const body = sectorBodies[idx];
    const puck = glassPucks[idx];
    if (body) body.style.transform = "";
    if (puck) puck.style.transform = "";
    group?.classList.add("is-pulsing");
    puck?.classList.add("is-pulsing");

    if (state.mode === "new") {
      void commitCreateNew(slot.item);
    } else {
      void commitFormatConversion(slot.item.id, Boolean(options.forceRaw));
    }
  }

  function resetTransientState(): void {
    conversionInFlight = false;
    isDragHovering = false;
    isHoveringControl = false;
    lastDomPointerMoveAt = 0;
    lastMonitorPointerMoveAt = 0;
    lastReleaseHandledAt = 0;
    lastMonitorPointerPos = null;
    lastCanvasPointerPos = null;
    currentPointerCanvas = null;
    lastDragCoords = null;
    latchedDwellSlot = -1;
    clearDwell();
    clearAutoClose();
    clearDismissTimer();
    clearDiagResetTimer();
    stopProgressLoop();
    stopPhysicsLoop();
    springField.setPointer(null, null, -1);
    springField.snapToRest();
    hubTetherEl.style.opacity = "0";
  }

  function resetClosedLocally(): void {
    closing = false;
    phase = "idle";
    activeIndex = -1;
    resetTransientState();
    shellEl.classList.remove("is-closing", "is-error-shake", "is-reopening");
    progressLayer.style.display = "none";
    for (const group of sectorGroups) {
      group.classList.remove("is-pulsing");
    }
    for (const puck of glassPucks) {
      puck.classList.remove("is-pulsing");
    }
    updateSectorHighlightClasses();
    applySpringFrame(springField.step(0.016));
  }

  function requestDismiss(reason: string): void {
    if (closing) return;
    closing = true;
    resetTransientState();
    shellEl.classList.add("is-closing");
    dismissTimer = window.setTimeout(() => {
      dismissTimer = null;
      if (!disposed) {
        void dismissWheel(reason);
      }
    }, 100);
  }

  function clientToCanvasPoint(clientX: number, clientY: number): { x: number; y: number } {
    const rect = svgEl.getBoundingClientRect();
    const scaleX = rect.width > 0 ? WHEEL_CANVAS_SIZE / rect.width : 1;
    const scaleY = rect.height > 0 ? WHEEL_CANVAS_SIZE / rect.height : 1;
    return {
      x: (clientX - rect.left) * scaleX,
      y: (clientY - rect.top) * scaleY,
    };
  }

  function windowLogicalToCanvasPoint(localX: number, localY: number): { x: number; y: number } {
    if (isTauriRuntime()) {
      return { x: localX, y: localY };
    }
    const rect = svgEl.getBoundingClientRect();
    const scaleX = rect.width > 0 ? WHEEL_CANVAS_SIZE / rect.width : 1;
    const scaleY = rect.height > 0 ? WHEEL_CANVAS_SIZE / rect.height : 1;
    return {
      x: (localX - rect.left) * scaleX,
      y: (localY - rect.top) * scaleY,
    };
  }

  function sameFileList(a: readonly string[], b: readonly string[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return false;
    }
    return true;
  }

  async function applyState(next: WheelStatePayload): Promise<void> {
    const epoch = ++stateEpoch;
    const wasClosing = closing;
    closing = false;
    openedAtMs = performance.now();
    conversionInFlight = false;
    isHoveringControl = false;
    lastDomPointerMoveAt = 0;
    lastReleaseHandledAt = 0;
    clearDismissTimer();
    shellEl.classList.remove("is-closing", "is-error-shake");
    if (wasClosing) {
      shellEl.classList.add("is-reopening");
      void shellEl.offsetWidth;
      shellEl.classList.remove("is-reopening");
    }
    phase = "idle";
    activeActionLabel = "";
    activeTargetFormat = "";
    successLocationLabel = "";
    lastCreatedPath = "";
    lastAnnouncedMilestone = -1;
    lastErrorMsg = "";
    diagBtnState = "idle";
    diagBtnFailReason = "";
    lastResults = [];
    clearDwell();
    clearAutoClose();
    clearDiagResetTimer();
    stopProgressLoop();
    progressLayer.style.display = "none";

    const filesChanged = next.files.join("\n") !== state.files.join("\n");
    const modeOrFilesChanged = next.mode !== state.mode || filesChanged;

    state = next;
    shellEl.classList.toggle("is-drag-mode", Boolean(state.isDrag));
    lastMonitorPointerPos = { x: next.originX, y: next.originY };
    lastMonitorPointerMoveAt = next.isDrag ? performance.now() : 0;
    if (wasClosing || filesChanged) {
      rawMode = false;
    }
    if (modeOrFilesChanged) {
      page = 0;
      activeIndex = -1;
      latchedDwellSlot = -1;
    }

    // Render immediately so stale sectors from an earlier mode/selection are cleared before async IPC fetches.
    lastCanvasPointerPos = null;
    currentPointerCanvas = null;
    for (const group of sectorGroups) {
      group.classList.remove("is-pulsing");
    }
    for (const puck of glassPucks) {
      puck.classList.remove("is-pulsing");
    }
    updateModePillUI();
    renderSectors();
    updateCursorProximity(next.originX, next.originY);
    renderHub();
    void refreshThumbnail();

    if (next.mode === "new") {
      const [latestPrefs, latestKinds] = await Promise.all([
        getPreferences().catch(() => null),
        getNewFileKinds().catch(() => null),
      ]);
      if (disposed || epoch !== stateEpoch) return;
      if (latestPrefs) prefs = latestPrefs;
      if (latestKinds) {
        builtInKinds = latestKinds.kinds;
        userTemplates = latestKinds.templates;
      }
      updateModePillUI();
      renderSectors();
      renderHub();
    } else {
      const latestPrefs = await getPreferences().catch(() => null);
      if (disposed || epoch !== stateEpoch) return;
      if (latestPrefs) {
        prefs = latestPrefs;
      }
      updateModePillUI();
      renderSectors();
      renderHub();
    }

    announce(
      formatWheelOpenAnnouncement({
        mode: state.mode,
        fileCount: state.files.length,
        optionCount: currentItemsCount,
        page,
        totalPages,
        lang: lang(),
      }),
      { force: true },
    );
  }

  async function handleIncomingFiles(paths: string[]): Promise<boolean> {
    if (paths.length === 0) return false;
    if (
      state.mode === "convert" &&
      sameFileList(paths, state.files) &&
      (state.compatibleFormats.length > 0 ||
        state.batchActions.length > 0 ||
        Boolean(state.availabilityMessage))
    ) {
      return false;
    }
    const epochBefore = stateEpoch;
    const inspected = await inspectFiles(paths);
    if (disposed || stateEpoch !== epochBefore) return false;
    await applyState({
      ...state,
      mode: "convert",
      files: inspected.files,
      compatibleFormats: inspected.compatibleFormats,
      batchActions: inspected.batchActions,
      availabilityMessage: inspected.availabilityMessage,
      family: inspected.family ?? null,
    });
    return true;
  }

  async function toggleMode(requested?: WheelMode): Promise<void> {
    if (phase !== "idle") return;
    const target: WheelMode =
      requested ?? (state.mode === "convert" ? "new" : "convert");
    if (target === state.mode) return;
    if (target === "convert" && state.files.length === 0) {
      // Gracefully remain in New File mode if no files are selected
      void playWheelFeedback("error");
      return;
    }
    const epochBefore = stateEpoch;
    try {
      const updated = await switchWheelMode(target);
      if (!disposed && stateEpoch === epochBefore) {
        await applyState(updated);
      }
    } catch {
      if (!disposed && stateEpoch === epochBefore) {
        await applyState({ ...state, mode: target });
      }
    }
  }

  function toggleRawMode(): void {
    if (phase !== "idle" || state.mode !== "convert") return;
    rawMode = !rawMode;
    void playWheelFeedback("tick");
    updateModePillUI();
    renderHub();
    announce(rawMode ? t(lang(), "wheel.mode.raw") : t(lang(), "wheel.mode.smart"), {
      force: true,
    });
  }

  rawToggleBtn.addEventListener("click", (ev) => {
    ev.stopPropagation();
    rawToggleBtn.blur();
    toggleRawMode();
  });

  // Mode pill listeners
  for (const btn of modeButtons) {
    btn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      btn.blur();
      const m = btn.dataset.mode as WheelMode;
      void toggleMode(m);
    });
  }

  // Single authoritative pointer stream: stageEl captures all wheel hover + controls,
  // with WeakSet dedup so dispatching directly on svgEl or stageEl behaves identically.
  const onPointerMove = (ev: PointerEvent) => {
    if (handledPointerEvents.has(ev)) return;
    handledPointerEvents.add(ev);
    lastDomPointerMoveAt = performance.now();
    if (phase !== "idle") return;
    const target = ev.target;
    if (
      target instanceof Element &&
      target.closest(
        ".wheel-top-bar, .wheel-page-dots, .wheel-preview-dock, .wheel-hub-pill-btn",
      )
    ) {
      isHoveringControl = true;
      setActiveIndex(-1, { skipStep: true });
      updateCursorProximity(null, null);
      return;
    }
    isHoveringControl = false;
    if (slots.length === 0) return;
    if (target instanceof Element && target.closest("#wheel-hub")) {
      const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
      currentPointerCanvas = pt;
      lastCanvasPointerPos = pt;
      setActiveIndex(-1, { skipStep: true });
      updateCursorProximity(pt.x, pt.y);
      return;
    }
    const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
    const isDragging = state.isDrag || isDragHovering;
    const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
      previousIndex: activeIndex,
      innerDeadZoneRadius: 72,
      outerMaxRadius: isDragging ? WHEEL_OUTER_MAX_RADIUS : WHEEL_OUTER_RADIUS + 6,
    });
    currentPointerCanvas = pt;
    lastCanvasPointerPos = pt;
    setActiveIndex(hit, { skipStep: true });
    updateCursorProximity(pt.x, pt.y);
  };

  const onStagePointerLeave = () => {
    isHoveringControl = false;
    if (phase === "idle") {
      setActiveIndex(-1, { skipStep: true });
      updateCursorProximity(null, null);
    }
  };

  const onSvgPointerLeave = () => {
    if (phase === "idle") {
      setActiveIndex(-1, { skipStep: true });
      updateCursorProximity(null, null);
    }
  };

  const onSvgPointerDown = () => {
    lastSvgPointerDownAt = performance.now();
  };

  const onSectorFocusIn = (ev: FocusEvent) => {
    if (phase !== "idle" || slots.length === 0) return;
    if (performance.now() - lastSvgPointerDownAt < 250) return;
    const targetEl = ev.target instanceof Element ? ev.target : null;
    const group = targetEl?.closest<SVGGElement>(".wheel-sector-group[data-slot-index]");
    if (!group) return;
    const idx = Number(group.dataset.slotIndex ?? "-1");
    if (Number.isInteger(idx) && idx >= 0 && idx < slots.length && idx !== activeIndex) {
      setActiveIndex(idx);
    }
  };

  const onSvgClick = (ev: MouseEvent) => {
    if (phase !== "idle" || slots.length === 0) return;
    const targetEl = ev.target instanceof Element ? ev.target : null;
    const sectorGroupEl = targetEl?.closest<SVGGElement>(".wheel-sector-group[data-slot-index]");
    const isSyntheticOrZeroCoord = ev.detail === 0 || (ev.clientX === 0 && ev.clientY === 0);
    let hit = -1;
    if (isSyntheticOrZeroCoord && sectorGroupEl) {
      const idxAttr = Number(sectorGroupEl.dataset.slotIndex ?? "-1");
      if (Number.isInteger(idxAttr)) hit = idxAttr;
    } else {
      const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
      const isDragging = state.isDrag || isDragHovering;
      const maxRadius = isDragging ? WHEEL_OUTER_MAX_RADIUS : WHEEL_OUTER_RADIUS + 6;
      hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
        previousIndex: activeIndex,
        innerDeadZoneRadius: 72,
        outerMaxRadius: maxRadius,
      });
      if (hit < 0 && sectorGroupEl) {
        const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
        if (dist >= 72 && dist <= maxRadius) {
          const idxAttr = Number(sectorGroupEl.dataset.slotIndex ?? "-1");
          if (Number.isInteger(idxAttr)) hit = idxAttr;
        }
      }
    }
    if (hit < 0) return;
    const slot = slots[hit];
    if (!slot) return;
    ev.stopPropagation();
    if (
      !isSyntheticOrZeroCoord &&
      document.activeElement instanceof Element &&
      sectorsLayer.contains(document.activeElement)
    ) {
      (document.activeElement as unknown as { blur?: () => void }).blur?.();
    }
    if (
      state.mode === "convert" &&
      slot.kind === "item" &&
      ev.shiftKey &&
      !ev.altKey &&
      state.files.length > 0
    ) {
      void openBatchWithFiles(state.files).then(() => {
        if (isTauriRuntime() && !disposed) requestDismiss("open-batch");
      });
      return;
    }
    activateSlot(hit, { forceRaw: ev.altKey });
  };

  stageEl.addEventListener("pointermove", onPointerMove);
  stageEl.addEventListener("pointerleave", onStagePointerLeave);
  svgEl.addEventListener("pointermove", onPointerMove);
  svgEl.addEventListener("pointerleave", onSvgPointerLeave);
  svgEl.addEventListener("pointerdown", onSvgPointerDown);
  svgEl.addEventListener("mousedown", onSvgPointerDown);
  svgEl.addEventListener("focusin", onSectorFocusIn);
  svgEl.addEventListener("click", onSvgClick);

  const onBackdropClick = (ev: MouseEvent) => {
    if (phase !== "idle" || state.isDrag || closing) return;
    const target = ev.target;
    if (
      target instanceof Element &&
      target.closest(
        "#wheel-hub, .wheel-top-bar, .wheel-mode-pill, #wheel-raw-toggle, .wheel-page-dots, .wheel-preview-dock, .wheel-sector-group",
      )
    ) {
      return;
    }
    const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
    const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
    if (dist > 176) {
      requestDismiss("backdrop-click");
    }
  };
  stageEl.addEventListener("click", onBackdropClick);

  const onWindowBlur = () => {
    if (
      isTauriRuntime() &&
      phase === "idle" &&
      !state.isDrag &&
      !closing &&
      performance.now() - openedAtMs > 180
    ) {
      requestDismiss("window-blur");
    }
  };
  window.addEventListener("blur", onWindowBlur);

  // Keyboard Navigation
  const onKeyDown = (ev: KeyboardEvent) => {
    if (ev.key === "Escape") {
      ev.preventDefault();
      if (phase === "converting") {
        void cancelConversion();
      } else {
        requestDismiss("escape-key");
      }
      return;
    }

    if (phase !== "idle") return;

    if (ev.key === "Tab") {
      ev.preventDefault();
      void toggleMode();
      return;
    }

    if (
      state.mode === "convert" &&
      !ev.repeat &&
      !ev.metaKey &&
      !ev.ctrlKey &&
      !ev.altKey &&
      (ev.key === "r" || ev.key === "R")
    ) {
      ev.preventDefault();
      toggleRawMode();
      return;
    }

    if (slots.length === 0) return;

    if (ev.key === "ArrowRight" || ev.key === "ArrowDown") {
      ev.preventDefault();
      if ((state.isDrag || isDragHovering) && ev.repeat) return;
      const next = findNextActionableSlot(slots, activeIndex, 1);
      setActiveIndex(next);
      return;
    }

    if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") {
      ev.preventDefault();
      if ((state.isDrag || isDragHovering) && ev.repeat) return;
      const prev = findNextActionableSlot(slots, activeIndex, -1);
      setActiveIndex(prev);
      return;
    }

    if (ev.key === "Enter" || ev.key === " ") {
      if (ev.target instanceof HTMLButtonElement) {
        return;
      }
      const focusedGroup =
        ev.target instanceof Element
          ? ev.target.closest<SVGGElement>(".wheel-sector-group[data-slot-index]")
          : null;
      const targetIdx = focusedGroup
        ? Number(focusedGroup.dataset.slotIndex ?? "-1")
        : activeIndex;
      ev.preventDefault();
      if (targetIdx >= 0 && isSlotActionable(slots[targetIdx])) {
        activateSlot(targetIdx);
      }
      return;
    }

    if (/^[1-8]$/.test(ev.key)) {
      const idx = Number(ev.key) - 1;
      if (idx < slots.length && isSlotActionable(slots[idx])) {
        ev.preventDefault();
        setActiveIndex(idx);
        activateSlot(idx);
      }
    }
  };

  window.addEventListener("keydown", onKeyDown);

  // HTML5 Drag & Drop fallback (for Vite browser preview or non-native drops)
  const extractHtml5Paths = (dt: DataTransfer | null): string[] => {
    if (!dt) return [];
    const paths: string[] = [];
    for (const file of Array.from(dt.files)) {
      const withPath = file as File & { path?: string };
      paths.push(withPath.path || `/Users/demo/Desktop/${file.name}`);
    }
    return paths;
  };

  const onHtmlDragEnter = (ev: DragEvent) => {
    if (phase !== "idle") return;
    ev.preventDefault();
    if (isTauriRuntime()) return;
    isDragHovering = true;
    const paths = extractHtml5Paths(ev.dataTransfer);
    if (paths.length > 0) {
      void handleIncomingFiles(paths);
    }
  };

  const onHtmlDragOver = (ev: DragEvent) => {
    if (phase !== "idle") return;
    ev.preventDefault();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
    if (isTauriRuntime()) return;
    isDragHovering = true;
    if (slots.length > 0) {
      const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
      const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
        previousIndex: activeIndex,
      });
      currentPointerCanvas = pt;
      lastCanvasPointerPos = pt;
      setActiveIndex(hit, { skipStep: true });
      updateCursorProximity(pt.x, pt.y);
    }
  };

  const onHtmlDragLeave = (ev: DragEvent) => {
    if (isTauriRuntime()) return;
    if (!shellEl.contains(ev.relatedTarget as Node | null)) {
      isDragHovering = false;
      setActiveIndex(-1, { skipStep: true });
      updateCursorProximity(null, null);
    }
  };

  const onHtmlDrop = (ev: DragEvent) => {
    if (phase !== "idle" || isTauriRuntime()) return;
    ev.preventDefault();
    isDragHovering = false;
    const incoming = extractHtml5Paths(ev.dataTransfer);
    const runDrop = async () => {
      const changed = incoming.length > 0 ? await handleIncomingFiles(incoming) : false;
      if (disposed || phase !== "idle") return;
      const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
      const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
      const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
        previousIndex: activeIndex,
      });
      if (hit >= 0 && slots[hit]) {
        const slot = slots[hit];
        if (slot.kind === "item" && slot.item.enabled) {
          activateSlot(hit);
        } else if (slot.kind === "more" || slot.kind === "back") {
          activateSlot(hit);
        }
      } else if (!changed && dist > 210 && state.isDrag) {
        requestDismiss("rejected-drop");
      }
    };
    void runDrop();
  };

  shellEl.addEventListener("dragenter", onHtmlDragEnter);
  shellEl.addEventListener("dragover", onHtmlDragOver);
  shellEl.addEventListener("dragleave", onHtmlDragLeave);
  shellEl.addEventListener("drop", onHtmlDrop);

  // Browser Preview Dock Handlers
  const previewDock = root.querySelector<HTMLElement>("#wheel-preview-dock");
  previewDock?.addEventListener("click", (ev) => {
    const btn = ev.target instanceof Element
      ? ev.target.closest<HTMLButtonElement>("button[data-preview]")
      : null;
    if (!btn) return;
    const mode = btn.dataset.preview;
    if (mode === "image") {
      void openWheelMode("convert", ["/Users/demo/Desktop/vacation-photo.heic"]);
    } else if (mode === "pdf12") {
      void openWheelMode("convert", ["/Users/demo/Documents/Quarterly-Report.pdf"]);
    } else if (mode === "new") {
      void openWheelMode("new", []);
    } else if (mode === "settings") {
      window.location.hash = "#settings";
    } else if (mode === "batch") {
      void openBatchWithFiles(state.files.length > 0 ? state.files : ["/Users/demo/Desktop/photo.png"]);
    }
  });

  // Tauri IPC Event Subscriptions
  const unlistenFns: Array<() => void> = [];

  void (async () => {
    const subs = await Promise.all([
      events.onWheelState((payload) => {
        if (disposed) return;
        receivedLiveState = true;
        void applyState(payload);
      }),
      events.onWheelDragFiles((payload) => {
        if (!disposed && payload.files.length > 0) {
          receivedLiveState = true;
          void handleIncomingFiles(payload.files);
        }
      }),
      events.onWheelPointerMove((payload) => {
        if (disposed || closing || phase !== "idle" || slots.length === 0) return;
        const isDragging = state.isDrag || isDragHovering;
        const pt = windowLogicalToCanvasPoint(payload.localX, payload.localY);
        if (!isDragging && isHoveringControl) {
          const inTopBar = Boolean(topBarEl) && pt.y <= 44 && pt.x >= 90 && pt.x <= 350;
          const nearCenterHubButton = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER) <= 58;
          if (inTopBar || nearCenterHubButton) {
            setActiveIndex(-1, { skipStep: true });
            updateCursorProximity(null, null);
            return;
          }
          isHoveringControl = false;
        }
        if (
          !isDragging &&
          lastDomPointerMoveAt > 0 &&
          performance.now() - lastDomPointerMoveAt < 350
        ) {
          return;
        }
        lastMonitorPointerMoveAt = performance.now();
        lastMonitorPointerPos = { x: payload.localX, y: payload.localY };
        if (!isDragging && topBarEl && pt.y <= 44 && pt.x >= 90 && pt.x <= 350) {
          setActiveIndex(-1, { skipStep: true });
          updateCursorProximity(null, null);
          return;
        }
        const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
          previousIndex: activeIndex,
          innerDeadZoneRadius: 72,
          outerMaxRadius: isDragging ? WHEEL_OUTER_MAX_RADIUS : WHEEL_OUTER_RADIUS + 6,
        });
        currentPointerCanvas = pt;
        lastCanvasPointerPos = pt;
        setActiveIndex(hit, { skipStep: true });
        updateCursorProximity(pt.x, pt.y);
      }),
      events.onWheelPointerRelease((payload) => {
        if (disposed || closing || phase !== "idle" || conversionInFlight) return;
        if (lastReleaseHandledAt > 0 && performance.now() - lastReleaseHandledAt < 250) return;
        lastReleaseHandledAt = performance.now();
        isDragHovering = false;
        lastDragCoords = null;
        lastMonitorPointerPos = { x: payload.localX, y: payload.localY };
        const pt = windowLogicalToCanvasPoint(payload.localX, payload.localY);
        const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
        const outerMax = state.isDrag ? WHEEL_OUTER_MAX_RADIUS : WHEEL_OUTER_RADIUS + 14;
        let hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
          previousIndex: activeIndex,
          innerDeadZoneRadius: 68,
          outerMaxRadius: outerMax,
        });
        if (hit < 0 && dist >= 68 && dist <= outerMax && activeIndex >= 0) {
          hit = activeIndex;
        }
        if (hit >= 0 && isSlotActionable(slots[hit])) {
          activateSlot(hit);
        } else if (dist > 210 || state.isDrag) {
          requestDismiss("pointer-release-outside");
        }
        // If inside dead-zone (r < 68) in non-drag mode, leave wheel open per UX spec
      }),
      events.onWheelEscape((payload) => {
        if (disposed) return;
        if (phase === "converting") {
          void cancelConversion();
        } else if (payload && payload.alreadyHidden === true) {
          resetClosedLocally();
        } else {
          requestDismiss("escape-event");
        }
      }),
      events.onWheelToggleRaw(() => {
        if (disposed || closing) return;
        toggleRawMode();
      }),
      events.onConversionProgress((payload) => {
        if (disposed || !conversionInFlight || phase !== "converting") return;
        targetPercent = Math.max(targetPercent, Math.min(99, payload.percent));
        if (payload.label) stepLabel = payload.label;
        const milestone = getProgressMilestoneBucket(payload.percent);
        if (milestone > lastAnnouncedMilestone) {
          lastAnnouncedMilestone = milestone;
          announce(
            formatProgressMilestoneAnnouncement(
              activeTargetFormat || activeActionLabel || "output",
              milestone,
              lang(),
            ),
          );
        }
      }),
      events.onConversionComplete((payload) => {
        if (disposed || !conversionInFlight || phase !== "converting") return;
        handleConversionComplete(payload);
      }),
      events.onPrefsChanged((updated) => {
        if (disposed) return;
        prefs = updated;
        updateModePillUI();
        renderSectors();
        renderHub();
      }),
      events.onTemplatesChanged(() => {
        if (disposed) return;
        void getNewFileKinds()
          .then((latestKinds) => {
            if (disposed) return;
            builtInKinds = latestKinds.kinds;
            userTemplates = latestKinds.templates;
            if (state.mode === "new" && phase === "idle") {
              renderSectors();
              renderHub();
            }
          })
          .catch(() => {});
      }),
      subscribeNativeDragDrop({
        onEnter: (paths, lx, ly) => {
          if (disposed || closing || phase !== "idle") return;
          isDragHovering = true;
          lastDragCoords = { x: lx, y: ly };
          const monitorActive =
            state.isDrag || performance.now() - lastMonitorPointerMoveAt < 800;
          const needInspect = paths.length > 0 && !sameFileList(paths, state.files);
          if (!needInspect && monitorActive) return;
          void (async () => {
            if (needInspect) {
              await handleIncomingFiles(paths);
            }
            if (
              disposed ||
              closing ||
              phase !== "idle" ||
              !isDragHovering ||
              state.isDrag ||
              performance.now() - lastMonitorPointerMoveAt < 800
            ) {
              return;
            }
            const coords = lastDragCoords ?? { x: lx, y: ly };
            const pt = windowLogicalToCanvasPoint(coords.x, coords.y);
            const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
              previousIndex: activeIndex,
            });
            currentPointerCanvas = pt;
            lastCanvasPointerPos = pt;
            setActiveIndex(hit, { skipStep: true });
            updateCursorProximity(pt.x, pt.y);
          })();
        },
        onOver: (lx, ly) => {
          if (disposed || closing || phase !== "idle" || slots.length === 0) return;
          isDragHovering = true;
          lastDragCoords = { x: lx, y: ly };
          if (state.isDrag || performance.now() - lastMonitorPointerMoveAt < 800) {
            return;
          }
          const pt = windowLogicalToCanvasPoint(lx, ly);
          const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
            previousIndex: activeIndex,
          });
          currentPointerCanvas = pt;
          lastCanvasPointerPos = pt;
          setActiveIndex(hit, { skipStep: true });
          updateCursorProximity(pt.x, pt.y);
        },
        onDrop: (paths, lx, ly) => {
          if (disposed || closing || phase !== "idle" || conversionInFlight) return;
          if (lastReleaseHandledAt > 0 && performance.now() - lastReleaseHandledAt < 250) return;
          lastReleaseHandledAt = performance.now();
          isDragHovering = false;
          lastDragCoords = null;
          void (async () => {
            const needInspect = paths.length > 0 && !sameFileList(paths, state.files);
            const changed = needInspect ? await handleIncomingFiles(paths) : false;
            if (disposed || closing || phase !== "idle" || conversionInFlight) return;
            const useMonitorPos =
              state.isDrag &&
              performance.now() - lastMonitorPointerMoveAt < 800 &&
              lastMonitorPointerPos !== null;
            const srcCoords = useMonitorPos ? lastMonitorPointerPos! : { x: lx, y: ly };
            const pt = windowLogicalToCanvasPoint(srcCoords.x, srcCoords.y);
            const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
            let hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
              previousIndex: activeIndex,
              innerDeadZoneRadius: 68,
            });
            if (hit < 0 && dist >= 68 && dist <= WHEEL_OUTER_MAX_RADIUS && activeIndex >= 0) {
              hit = activeIndex;
            }
            if (hit >= 0 && slots[hit]) {
              const slot = slots[hit];
              if (slot.kind === "item" && slot.item.enabled) {
                activateSlot(hit);
              } else if (slot.kind === "more" || slot.kind === "back") {
                activateSlot(hit);
              } else if (state.isDrag) {
                requestDismiss("rejected-drop");
              }
            } else if (!changed && state.isDrag) {
              requestDismiss("rejected-drop");
            }
          })();
        },
        onLeave: () => {
          if (disposed || state.isDrag) return;
          isDragHovering = false;
          lastDragCoords = null;
          setActiveIndex(-1, { skipStep: true });
          updateCursorProximity(null, null);
        },
      }),
    ]);

    if (disposed) {
      subs.forEach((u) => u());
      return;
    }
    unlistenFns.push(...subs);

    const [loadedPrefs, kindsResp, initialState] = await Promise.all([
      getPreferences().catch(() => null),
      getNewFileKinds().catch(() => ({ kinds: [], templates: [] })),
      getWheelState().catch(() => state),
    ]);
    if (disposed) return;
    if (loadedPrefs) prefs = loadedPrefs;
    builtInKinds = kindsResp.kinds;
    userTemplates = kindsResp.templates;
    if (!receivedLiveState) {
      await applyState(initialState);
    } else {
      updateModePillUI();
      renderSectors();
      renderHub();
    }
  })();

  return () => {
    disposed = true;
    clearDwell();
    clearAutoClose();
    clearDismissTimer();
    clearDiagResetTimer();
    stopProgressLoop();
    stopPhysicsLoop();
    hubTetherEl.style.opacity = "0";
    document.body.classList.remove("wheel-browser-preview");
    stageEl.removeEventListener("pointermove", onPointerMove);
    stageEl.removeEventListener("pointerleave", onStagePointerLeave);
    svgEl.removeEventListener("pointermove", onPointerMove);
    svgEl.removeEventListener("pointerleave", onSvgPointerLeave);
    svgEl.removeEventListener("pointerdown", onSvgPointerDown);
    svgEl.removeEventListener("mousedown", onSvgPointerDown);
    svgEl.removeEventListener("focusin", onSectorFocusIn);
    svgEl.removeEventListener("click", onSvgClick);
    stageEl.removeEventListener("click", onBackdropClick);
    window.removeEventListener("blur", onWindowBlur);
    window.removeEventListener("keydown", onKeyDown);
    shellEl.removeEventListener("dragenter", onHtmlDragEnter);
    shellEl.removeEventListener("dragover", onHtmlDragOver);
    shellEl.removeEventListener("dragleave", onHtmlDragLeave);
    shellEl.removeEventListener("drop", onHtmlDrop);
    unlistenFns.forEach((fn) => fn());
  };
}

function findNextActionableSlot(
  slots: readonly PaginatedSlot<WheelActionItem>[],
  currentIndex: number,
  direction: 1 | -1,
): number {
  const n = slots.length;
  if (n === 0) return -1;
  const start = currentIndex < 0 ? (direction === 1 ? -1 : 0) : currentIndex;
  for (let step = 1; step <= n; step++) {
    const idx = (start + direction * step + n) % n;
    const s = slots[idx];
    if (s.kind === "more" || s.kind === "back" || s.item.enabled) {
      return idx;
    }
  }
  return -1;
}

export { extLower };
