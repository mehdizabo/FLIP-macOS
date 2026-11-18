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
  WHEEL_PROGRESS_RADIUS,
  type PaginatedSlot,
} from "../wheelGeometry";

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
  let lastErrorMsg = "";
  let diagBtnLabel = "Copy Diagnostics";
  let diagBtnTitle = "Copy redacted diagnostics report";
  let lastResults: ConversionResult[] = [];
  let progressRaf: number | null = null;
  let autoCloseTimer: number | null = null;
  let dismissTimer: number | null = null;
  let diagResetTimer: number | null = null;
  let openedAtMs = 0;

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

  // Static shell DOM
  root.innerHTML = `
    <div class="wheel-stage" role="dialog" aria-label="FLIP Radial Wheel">
      <div class="sr-only" id="wheel-live-announcer" aria-live="polite" aria-atomic="true"></div>
      <div class="wheel-shell" id="wheel-shell">
        <div class="wheel-mode-pill" role="tablist" aria-label="Wheel Mode">
          <button type="button" class="wheel-mode-btn" data-mode="convert" role="tab">
            ${UI_ICONS.convert}<span>Convert</span>
          </button>
          <button type="button" class="wheel-mode-btn" data-mode="new" role="tab">
            ${UI_ICONS.newFile}<span>New File</span>
          </button>
          <span class="wheel-mode-tab-hint">Tab</span>
        </div>

        <div class="wheel-glass-ring" aria-hidden="true"></div>
        <div class="wheel-chromatic-rim" aria-hidden="true"></div>

        <svg class="wheel-svg" id="wheel-svg" viewBox="0 0 ${WHEEL_CANVAS_SIZE} ${WHEEL_CANVAS_SIZE}" width="${WHEEL_CANVAS_SIZE}" height="${WHEEL_CANVAS_SIZE}">
          <defs>
            <linearGradient id="flip-sector-rest-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="rgba(255,255,255,0.16)" />
              <stop offset="50%" stop-color="rgba(148,163,184,0.08)" />
              <stop offset="100%" stop-color="rgba(15,23,42,0.28)" />
            </linearGradient>
            <linearGradient id="flip-sector-nav-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="rgba(56,189,248,0.18)" />
              <stop offset="100%" stop-color="rgba(99,102,241,0.16)" />
            </linearGradient>
            <linearGradient id="flip-sector-active-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="rgba(56,189,248,0.68)" />
              <stop offset="55%" stop-color="rgba(14,165,233,0.52)" />
              <stop offset="100%" stop-color="rgba(99,102,241,0.62)" />
            </linearGradient>
            <linearGradient id="flip-progress-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="#38bdf8" />
              <stop offset="100%" stop-color="#818cf8" />
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

        <div class="wheel-hub" id="wheel-hub"></div>
        <div class="wheel-page-dots" id="wheel-page-dots" style="display:none"></div>
      </div>
      ${
        isTauriRuntime()
          ? ""
          : `<div class="wheel-preview-dock" id="wheel-preview-dock">
              <span>Preview:</span>
              <button type="button" data-preview="image">Image</button>
              <button type="button" data-preview="pdf12">PDF (12 fmts)</button>
              <button type="button" data-preview="new">New File</button>
              <button type="button" data-preview="settings">Settings</button>
              <button type="button" data-preview="batch">Batch</button>
            </div>`
      }
    </div>
  `;

  const stageEl = root.querySelector<HTMLElement>(".wheel-stage")!;
  const announcerEl = root.querySelector<HTMLElement>("#wheel-live-announcer")!;
  const shellEl = root.querySelector<HTMLElement>("#wheel-shell")!;
  const svgEl = root.querySelector<SVGSVGElement>("#wheel-svg")!;
  const sectorsLayer = root.querySelector<SVGGElement>("#wheel-sectors-layer")!;
  const progressLayer = root.querySelector<SVGGElement>("#wheel-progress-layer")!;
  const progressArcEl = root.querySelector<SVGPathElement>("#wheel-progress-arc")!;
  const progressRayEl = root.querySelector<SVGLineElement>("#wheel-progress-ray")!;
  const progressTipEl = root.querySelector<SVGCircleElement>("#wheel-progress-tip")!;
  const hubEl = root.querySelector<HTMLElement>("#wheel-hub")!;
  const pageDotsEl = root.querySelector<HTMLElement>("#wheel-page-dots")!;
  const modeButtons = Array.from(root.querySelectorAll<HTMLButtonElement>(".wheel-mode-btn"));

  let sectorGroups: SVGGElement[] = [];

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
        const meta = getNewKindMeta(k.id, k.name, k.extension);
        return {
          id: k.id,
          label: meta.shortLabel,
          subtitle: k.id === "clipboard" ? "From Clipboard" : `New ${k.name}`,
          accent: meta.accent,
          iconSvg: meta.iconSvg,
          enabled: k.available,
          disabledReason: k.disabledReason ?? undefined,
        };
      });

      for (const tmpl of userTemplates) {
        const meta = getNewKindMeta(tmpl.id, tmpl.name, tmpl.extension);
        items.push({
          id: tmpl.id,
          label: tmpl.name,
          subtitle: tmpl.group ? `Template: ${tmpl.group}/${tmpl.name}` : `Template: ${tmpl.name}`,
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
      const meta = getFormatMeta(fmt);
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
          ? "Select or drop files to convert"
          : "Convert files (Tab)";
      } else {
        btn.title = "Create New File (Tab)";
      }
    }
  }

  function renderSectors(): void {
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
      const placeholders = ["DOCX", "PDF", "MD", "HTML", "PNG", "JPG", "WEBP", "MP3"];
      sectorsLayer.innerHTML = placeholders
        .map((label, i) => {
          const d = sectorSvgPath(i, placeholders.length);
          const pos = getSectorCenter(i, placeholders.length, 124);
          return `
            <g class="wheel-sector-group is-disabled" data-slot-index="${i}" aria-hidden="true">
              <path class="wheel-sector-path" d="${d}" />
              <foreignObject x="${pos.x - 36}" y="${pos.y - 26}" width="72" height="52">
                <div class="wheel-sector-content">
                  <span class="wheel-sector-icon">${getFormatMeta(label).iconSvg}</span>
                  <span class="wheel-sector-label">${escapeHtml(label)}</span>
                </div>
              </foreignObject>
            </g>
          `;
        })
        .join("");
      sectorGroups = [];
      return;
    }

    const count = slots.length;
    sectorsLayer.innerHTML = slots
      .map((slot, i) => {
        const d = sectorSvgPath(i, count);
        const pos = getSectorCenter(i, count, 124);
        const isNav = slot.kind === "more" || slot.kind === "back";
        const enabled = isSlotActionable(slot);
        const label =
          slot.kind === "more"
            ? "More ▸"
            : slot.kind === "back"
              ? "Back ◂"
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
        const ariaLabel = formatSectorAriaLabel(slot, i, count);
        const isActive = activeIndex === i && enabled && phase === "idle";

        const classes = [
          "wheel-sector-group",
          isNav ? "is-nav" : "",
          !enabled ? "is-disabled" : "",
          isActive ? "is-active" : "",
          phase === "converting" && activeActionLabel !== label ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ");

        return `
          <g class="${classes}"
             data-slot-index="${i}"
             style="--sector-accent:${escapeHtml(accent)}"
             role="button"
             tabindex="${enabled ? "0" : "-1"}"
             aria-disabled="${enabled ? "false" : "true"}"
             aria-current="${isActive ? "true" : "false"}"
             aria-label="${escapeHtml(ariaLabel)}">
            <path class="wheel-sector-path" d="${d}" />
            <path class="wheel-sector-spec" pathLength="100" d="${d}" />
            <foreignObject x="${pos.x - 36}" y="${pos.y - 26}" width="72" height="52">
              <div class="wheel-sector-content">
                <span class="wheel-sector-icon">${icon}</span>
                <span class="wheel-sector-label">${escapeHtml(label)}</span>
                <span class="wheel-sector-kbd">${i + 1}</span>
              </div>
            </foreignObject>
          </g>
        `;
      })
      .join("");

    sectorGroups = Array.from(sectorsLayer.querySelectorAll<SVGGElement>(".wheel-sector-group"));
  }

  function updateSectorHighlightClasses(): void {
    sectorGroups.forEach((group, idx) => {
      const slot = slots[idx];
      const enabled = isSlotActionable(slot);
      const isActive = idx === activeIndex && enabled && phase === "idle";
      group.classList.toggle("is-active", isActive);
      group.classList.toggle("is-dwelling", idx === dwellSlotIndex && dwellTimer !== null);
      group.setAttribute("aria-current", isActive ? "true" : "false");
    });
  }

  function renderHub(): void {
    hubEl.classList.toggle("is-error-hub", phase === "error");
    const fileCount = state.files.length;
    const primaryName =
      fileCount === 0
        ? ""
        : fileCount === 1
          ? basename(state.files[0])
          : `${fileCount} files`;

    if (phase === "converting") {
      const shownPct = Math.max(1, Math.min(99, Math.round(displayedPercent)));
      hubEl.innerHTML = `
        <div class="wheel-hub-pct" id="wheel-hub-pct">${shownPct}%</div>
        <div class="wheel-hub-title" title="${escapeHtml(primaryName)}">${escapeHtml(primaryName || "Converting")}</div>
        <div class="wheel-hub-subtitle" id="wheel-hub-step">${escapeHtml(stepLabel || `→ ${activeActionLabel}`)}</div>
        <div class="wheel-hub-actions">
          <button type="button" class="wheel-hub-pill-btn is-danger" id="hub-cancel-btn">Cancel</button>
        </div>
      `;
      hubEl.querySelector("#hub-cancel-btn")?.addEventListener("click", (e) => {
        e.stopPropagation();
        void cancelConversion();
      });
      return;
    }

    if (phase === "success") {
      hubEl.innerHTML = `
        <svg class="wheel-check-svg" viewBox="0 0 36 36" aria-hidden="true">
          <circle class="wheel-check-circle" cx="18" cy="18" r="16" />
          <path class="wheel-check-mark" d="M10.5 18.5l5 5 10-10.5" />
        </svg>
        <div class="wheel-hub-title">${escapeHtml(activeActionLabel || "Done")}</div>
        <div class="wheel-hub-subtitle is-success">Completed!</div>
      `;
      return;
    }

    if (phase === "error") {
      const failTitle = state.mode === "new" ? "Creation Failed" : "Conversion Failed";
      const canOpenBatch = state.mode === "convert" && state.files.length > 0;
      hubEl.innerHTML = `
        <div class="wheel-hub-title">${failTitle}</div>
        <div class="wheel-hub-subtitle is-error" title="${escapeHtml(lastErrorMsg)}">${escapeHtml(lastErrorMsg || "An error occurred")}</div>
        <div class="wheel-hub-actions is-error-actions">
          <button type="button" class="wheel-hub-pill-btn is-diag-btn" id="hub-copy-diag" title="${escapeHtml(diagBtnTitle)}">${escapeHtml(diagBtnLabel)}</button>
          <div class="wheel-hub-error-row">
            ${canOpenBatch ? `<button type="button" class="wheel-hub-pill-btn" id="hub-batch-details" title="Open Batch Details">Details</button>` : ""}
            <button type="button" class="wheel-hub-pill-btn" id="hub-dismiss-btn">Close</button>
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
        const btn = e.currentTarget as HTMLButtonElement;
        btn.disabled = true;
        btn.textContent = "Copying…";
        clearDiagResetTimer();
        try {
          const res = await copyDiagnosticsToClipboard();
          if (disposed || phase !== "error") return;
          if (res.ok) {
            diagBtnLabel = "Copied!";
            diagBtnTitle = "Redacted diagnostics copied to clipboard";
          } else {
            diagBtnLabel = "Copy Failed";
            diagBtnTitle =
              res.error || "Clipboard unavailable — open Settings → About → Copy Diagnostics";
          }
          renderHub();
          diagResetTimer = window.setTimeout(() => {
            diagResetTimer = null;
            if (!disposed && phase === "error") {
              diagBtnLabel = "Copy Diagnostics";
              diagBtnTitle = "Copy redacted diagnostics report";
              renderHub();
            }
          }, 2000);
        } catch (err) {
          if (disposed || phase !== "error") return;
          diagBtnLabel = "Copy Failed";
          diagBtnTitle = formatError(err);
          renderHub();
          diagResetTimer = window.setTimeout(() => {
            diagResetTimer = null;
            if (!disposed && phase === "error") {
              diagBtnLabel = "Copy Diagnostics";
              diagBtnTitle = "Copy redacted diagnostics report";
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
      let subtitle = "Esc to cancel";
      let subClass = "is-muted";
      if (hoveredSlot) {
        if (hoveredSlot.kind === "more") {
          subtitle = "More templates →";
          subClass = "";
        } else if (hoveredSlot.kind === "back") {
          subtitle = "← Previous items";
          subClass = "";
        } else if (!hoveredSlot.item.enabled) {
          subtitle = hoveredSlot.item.disabledReason ?? "Unavailable";
          subClass = "is-error";
        } else {
          subtitle = hoveredSlot.item.subtitle;
          subClass = "";
        }
      }
      hubEl.innerHTML = `
        <div class="wheel-hub-thumb-wrap">
          <div class="wheel-hub-thumb-fallback">${UI_ICONS.folder}</div>
        </div>
        <div class="wheel-hub-title" title="${escapeHtml(state.targetFolder)}">${escapeHtml(folderLabel)}</div>
        <div class="wheel-hub-subtitle ${subClass}">${escapeHtml(subtitle)}</div>
      `;
      return;
    }

    // Convert mode hub
    if (fileCount === 0) {
      hubEl.innerHTML = `
        <div class="wheel-hub-thumb-wrap">
          <div class="wheel-hub-thumb-fallback">${UI_ICONS.drop}</div>
        </div>
        <div class="wheel-hub-title">Drop files here</div>
        <div class="wheel-hub-subtitle is-muted">Esc to cancel</div>
      `;
      return;
    }

    let subtitle = "Esc to cancel";
    let subtitleTitle = "";
    let subClass = "is-muted";
    if (state.compatibleFormats.length === 0 && state.batchActions.length === 0) {
      const rawMsg = state.availabilityMessage || "No common format";
      subtitleTitle = rawMsg;
      const lines = rawMsg.split("\n").map((l) => l.trim()).filter(Boolean);
      subtitle = lines.length > 1 && /^\d+\s+files?\s+selected$/i.test(lines[0])
        ? lines.slice(1).join(" · ")
        : lines.join(" · ") || "No common format";
      subClass = "is-error";
    } else if (hoveredSlot) {
      if (hoveredSlot.kind === "more") {
        subtitle = "Next formats →";
        subClass = "";
      } else if (hoveredSlot.kind === "back") {
        subtitle = "← Previous formats";
        subClass = "";
      } else if (!hoveredSlot.item.enabled) {
        subtitle = hoveredSlot.item.disabledReason ?? "Unavailable";
        subClass = "is-error";
      } else {
        subtitle = hoveredSlot.item.subtitle;
        subClass = "";
      }
    }

    const thumbVisual = thumbnailDataUrl
      ? `<img class="wheel-hub-thumb" src="${escapeHtml(thumbnailDataUrl)}" alt="" />`
      : `<div class="wheel-hub-thumb-fallback">${UI_ICONS.document}</div>`;

    const showMultiBtn =
      state.compatibleFormats.length > 0 || state.batchActions.length > 0;

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
               <button type="button" class="wheel-hub-pill-btn" id="hub-multi-btn" title="Open Batch Multi-Format Converter (or Shift+Click any sector)">Multi</button>
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

  function setActiveIndex(nextIndex: number): void {
    if (phase !== "idle") return;
    const bounded = nextIndex >= 0 && nextIndex < slots.length ? nextIndex : -1;
    if (bounded === activeIndex) return;

    activeIndex = bounded;

    if (bounded !== latchedDwellSlot) {
      latchedDwellSlot = -1;
    }

    clearDwell();

    const slot = bounded >= 0 ? slots[bounded] : undefined;
    if (slot) {
      announce(formatSectorAriaLabel(slot, bounded, slots.length));
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
          announce(formatPageFlipAnnouncement(page, totalPages), { force: true });
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
    phase = "converting";
    activeActionLabel = formatLabel;
    activeTargetFormat = formatLabel;
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
    announce(formatProgressMilestoneAnnouncement(formatLabel, 0), { force: true });

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
        payload.error || failedUnit?.error || "One or more files failed to convert.";
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
        }),
        { force: true },
      );
    }
  }

  function getPhase(): WheelPhase {
    return phase;
  }

  async function commitFormatConversion(formatId: string): Promise<void> {
    if (getPhase() !== "idle" || state.files.length === 0) return;
    const filesSnapshot = [...state.files];
    const meta = getFormatMeta(formatId);
    conversionInFlight = true;
    void playWheelFeedback("generic");
    startConversionVisual(meta.shortLabel);

    try {
      const res = await executeConversion(filesSnapshot, [formatId]);
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
    conversionInFlight = false;
    void playWheelFeedback("generic");
    activeActionLabel = kind.label;
    activeTargetFormat = kind.label;
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
    announce(`Creating ${kind.label}`, { force: true });

    try {
      const res = await executeCreateNew(kind.id, state.targetFolder);
      if (disposed) return;
      phase = "success";
      const createdName = basename(res.path);
      activeActionLabel = createdName;
      progressArcEl.classList.remove("is-error");
      progressArcEl.classList.add("is-success");
      progressArcEl.setAttribute("d", progressArcSvgPath(99.9, WHEEL_PROGRESS_RADIUS));
      progressTipEl.style.display = "none";
      progressRayEl.style.display = "none";
      void playWheelFeedback("success");
      renderHub();
      announce(
        formatCompletionAnnouncement({ ok: true, createdLabel: createdName }),
        { force: true },
      );
      clearAutoClose();
      autoCloseTimer = window.setTimeout(() => {
        if (!disposed) requestDismiss("create-new-completed");
      }, 420);
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
        formatCompletionAnnouncement({ ok: false, errorMessage: lastErrorMsg }),
        { force: true },
      );
    }
  }

  function activateSlot(idx: number): void {
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
      announce(formatPageFlipAnnouncement(page, totalPages), { force: true });
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
    group?.classList.add("is-pulsing");

    if (state.mode === "new") {
      void commitCreateNew(slot.item);
    } else {
      void commitFormatConversion(slot.item.id);
    }
  }

  function requestDismiss(reason: string): void {
    if (closing) return;
    closing = true;
    conversionInFlight = false;
    isDragHovering = false;
    latchedDwellSlot = -1;
    clearDwell();
    clearAutoClose();
    clearDismissTimer();
    stopProgressLoop();
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
    lastAnnouncedMilestone = -1;
    lastErrorMsg = "";
    diagBtnLabel = "Copy Diagnostics";
    diagBtnTitle = "Copy redacted diagnostics report";
    lastResults = [];
    clearDwell();
    clearAutoClose();
    clearDiagResetTimer();
    stopProgressLoop();
    progressLayer.style.display = "none";

    const modeOrFilesChanged =
      next.mode !== state.mode ||
      next.files.join("\n") !== state.files.join("\n");

    state = next;
    if (modeOrFilesChanged) {
      page = 0;
      activeIndex = -1;
      latchedDwellSlot = -1;
    }

    // Render immediately so stale sectors from an earlier mode/selection are cleared before async IPC fetches.
    updateModePillUI();
    renderSectors();
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

  // Mode pill listeners
  for (const btn of modeButtons) {
    btn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      btn.blur();
      const m = btn.dataset.mode as WheelMode;
      void toggleMode(m);
    });
  }

  // Pointer hover & click on SVG ring (covers full r=68..210 polar hit region including 2px gaps & outer glow)
  const onPointerMove = (ev: PointerEvent) => {
    if (phase !== "idle" || slots.length === 0) return;
    const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
    const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
      previousIndex: activeIndex,
    });
    setActiveIndex(hit);
  };

  const onPointerLeave = () => {
    if (phase === "idle") {
      setActiveIndex(-1);
    }
  };

  const onSectorFocusIn = (ev: FocusEvent) => {
    if (phase !== "idle" || slots.length === 0) return;
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
      hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
        previousIndex: activeIndex,
      });
      if (hit < 0 && sectorGroupEl) {
        const idxAttr = Number(sectorGroupEl.dataset.slotIndex ?? "-1");
        if (Number.isInteger(idxAttr)) hit = idxAttr;
      }
    }
    if (hit < 0) return;
    const slot = slots[hit];
    if (!slot) return;
    ev.stopPropagation();
    if (
      state.mode === "convert" &&
      slot.kind === "item" &&
      (ev.shiftKey || ev.altKey) &&
      state.files.length > 0
    ) {
      void openBatchWithFiles(state.files).then(() => {
        if (isTauriRuntime() && !disposed) requestDismiss("open-batch");
      });
      return;
    }
    activateSlot(hit);
  };

  svgEl.addEventListener("pointermove", onPointerMove);
  svgEl.addEventListener("pointerleave", onPointerLeave);
  svgEl.addEventListener("focusin", onSectorFocusIn);
  svgEl.addEventListener("click", onSvgClick);

  const onBackdropClick = (ev: MouseEvent) => {
    if (phase !== "idle" || state.isDrag || closing) return;
    const target = ev.target;
    if (
      target instanceof Element &&
      target.closest("#wheel-hub, .wheel-mode-pill, .wheel-page-dots, .wheel-preview-dock, .wheel-sector-group")
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

    if (slots.length === 0) return;

    if (ev.key === "ArrowRight" || ev.key === "ArrowDown") {
      ev.preventDefault();
      const next = findNextActionableSlot(slots, activeIndex, 1);
      setActiveIndex(next);
      return;
    }

    if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") {
      ev.preventDefault();
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
    isDragHovering = true;
    const paths = extractHtml5Paths(ev.dataTransfer);
    if (paths.length > 0 && !isTauriRuntime()) {
      void handleIncomingFiles(paths);
    }
  };

  const onHtmlDragOver = (ev: DragEvent) => {
    if (phase !== "idle") return;
    ev.preventDefault();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
    isDragHovering = true;
    if (slots.length > 0) {
      const pt = clientToCanvasPoint(ev.clientX, ev.clientY);
      const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
        previousIndex: activeIndex,
      });
      setActiveIndex(hit);
    }
  };

  const onHtmlDragLeave = (ev: DragEvent) => {
    if (!shellEl.contains(ev.relatedTarget as Node | null)) {
      isDragHovering = false;
      setActiveIndex(-1);
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
        if (disposed || phase !== "idle" || slots.length === 0) return;
        const pt = windowLogicalToCanvasPoint(payload.localX, payload.localY);
        const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
          previousIndex: activeIndex,
        });
        setActiveIndex(hit);
      }),
      events.onWheelPointerRelease((payload) => {
        if (disposed || phase !== "idle") return;
        const pt = windowLogicalToCanvasPoint(payload.localX, payload.localY);
        const dist = Math.hypot(pt.x - WHEEL_CENTER, pt.y - WHEEL_CENTER);
        const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
          previousIndex: activeIndex,
        });
        if (hit >= 0 && isSlotActionable(slots[hit])) {
          activateSlot(hit);
        } else if (dist > 210) {
          requestDismiss("pointer-release-outside");
        }
        // If inside dead-zone (r < 68), leave wheel open in Idle mode per UX spec
      }),
      events.onWheelEscape(() => {
        if (disposed) return;
        if (phase === "converting") {
          void cancelConversion();
        } else {
          requestDismiss("escape-event");
        }
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
            ),
          );
        }
      }),
      events.onConversionComplete((payload) => {
        if (disposed || !conversionInFlight || phase !== "converting") return;
        handleConversionComplete(payload);
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
          if (disposed || phase !== "idle") return;
          isDragHovering = true;
          lastDragCoords = { x: lx, y: ly };
          void (async () => {
            if (paths.length > 0) {
              await handleIncomingFiles(paths);
            }
            if (disposed || phase !== "idle" || !isDragHovering) return;
            const coords = lastDragCoords ?? { x: lx, y: ly };
            const pt = windowLogicalToCanvasPoint(coords.x, coords.y);
            const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
              previousIndex: activeIndex,
            });
            setActiveIndex(hit);
          })();
        },
        onOver: (lx, ly) => {
          if (disposed || phase !== "idle" || slots.length === 0) return;
          isDragHovering = true;
          lastDragCoords = { x: lx, y: ly };
          const pt = windowLogicalToCanvasPoint(lx, ly);
          const hit = getSectorAtPoint(pt.x, pt.y, slots.length, {
            previousIndex: activeIndex,
          });
          setActiveIndex(hit);
        },
        onDrop: (paths, lx, ly) => {
          if (disposed || phase !== "idle") return;
          isDragHovering = false;
          lastDragCoords = null;
          void (async () => {
            const changed = paths.length > 0 ? await handleIncomingFiles(paths) : false;
            if (disposed || phase !== "idle") return;
            const pt = windowLogicalToCanvasPoint(lx, ly);
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
              } else if (state.isDrag) {
                requestDismiss("rejected-drop");
              }
            } else if (!changed && dist > 210 && state.isDrag) {
              requestDismiss("rejected-drop");
            }
          })();
        },
        onLeave: () => {
          if (disposed) return;
          isDragHovering = false;
          lastDragCoords = null;
          setActiveIndex(-1);
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
    document.body.classList.remove("wheel-browser-preview");
    svgEl.removeEventListener("pointermove", onPointerMove);
    svgEl.removeEventListener("pointerleave", onPointerLeave);
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
