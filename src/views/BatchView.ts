import { localizeAvailabilityMessage, normalizeLanguage, t } from "../i18n";
import { getFormatMeta, UI_ICONS } from "../icons";
import {
  basename,
  cancelConversion,
  consumeBatchHandoff,
  copyDiagnosticsToClipboard,
  defaultPreferences,
  dirname,
  escapeHtml,
  events,
  executeConversion,
  extLower,
  formatError,
  getPreferences,
  getWheelState,
  inspectFiles,
  isTauriRuntime,
  openWindow,
  revealInFinder,
  subscribeNativeDragDrop,
} from "../ipc";
import type { AppPreferences, ConversionResult, InspectFilesResult } from "../types";

type BatchStatusMode =
  | { kind: "prompt" }
  | { kind: "ready"; count: number }
  | { kind: "noSelection" }
  | { kind: "webkitHidden" }
  | { kind: "starting" }
  | { kind: "cancelling" }
  | { kind: "finishedErr"; err: string }
  | { kind: "completed"; count: number }
  | { kind: "failed"; msg: string }
  | { kind: "diagFailed"; msg: string }
  | { kind: "live"; text: string };

export function mountBatchView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "batch";

  let disposed = false;
  let prefs: AppPreferences = defaultPreferences();
  let files: string[] = [];
  let inspection: InspectFilesResult = {
    files: [],
    compatibleFormats: [],
    batchActions: [],
    availabilityMessage: "",
    family: null,
  };
  let selectedFormats = new Set<string>();
  let rawMode = false;
  let busy = false;
  let progressPct = 0;
  let statusMode: BatchStatusMode = { kind: "prompt" };
  let results: ConversionResult[] = [];
  let lastBatchError: string | null = null;
  let diagState: "idle" | "copied" | "manual" = "idle";
  let diagFallbackText = "";
  let diagResetTimer: number | null = null;

  const lang = () => normalizeLanguage(prefs.language);

  function resolveStatusText(): string {
    const l = lang();
    switch (statusMode.kind) {
      case "prompt":
        return t(l, "batch.statusPrompt");
      case "ready":
        return t(l, "batch.filesReady", { count: statusMode.count });
      case "noSelection":
        return t(l, "batch.noWheelSelection");
      case "webkitHidden":
        return t(l, "batch.webkitPathHidden");
      case "starting":
        return t(l, "batch.starting");
      case "cancelling":
        return t(l, "batch.cancelling");
      case "finishedErr":
        return t(l, "batch.finishedErr", { err: statusMode.err });
      case "completed":
        return t(l, "batch.completedOutputs", { count: statusMode.count });
      case "failed":
        return t(l, "batch.failedMsg", { msg: statusMode.msg });
      case "diagFailed":
        return t(l, "batch.diagExportFailed", { msg: statusMode.msg });
      case "live":
        return statusMode.text;
    }
  }

  root.innerHTML = `
    <div class="batch-shell">
      <div class="page-header">
        <div>
          <h1 class="page-title" id="batch-title"></h1>
          <p class="card-subtitle" id="batch-subtitle"></p>
        </div>
        <div style="display:flex;gap:8px">
          <button type="button" class="btn btn-sm" id="batch-open-settings">${UI_ICONS.settings}<span id="batch-open-settings-label"></span></button>
        </div>
      </div>

      <section class="batch-dropzone" id="batch-dropzone">
        <div style="color:var(--accent)">${UI_ICONS.drop}</div>
        <div style="font-weight:600;font-size:14px" id="batch-drop-title"></div>
        <div class="setting-desc" id="batch-drop-desc"></div>
        <div style="display:flex;gap:8px;flex-wrap:wrap;justify-content:center;margin-top:4px">
          <button type="button" class="btn btn-sm" id="batch-load-wheel"></button>
          <label class="btn btn-sm" style="cursor:pointer">
            <span id="batch-choose-files-label"></span>
            <input type="file" id="batch-file-input" multiple style="display:none" />
          </label>
          <button type="button" class="btn btn-sm" id="batch-clear-files"></button>
        </div>
        <div style="display:flex;gap:6px;width:100%;max-width:520px;margin-top:6px">
          <input type="text" class="mac-input" id="batch-path-input" style="flex:1" />
          <button type="button" class="btn btn-sm" id="batch-add-path"></button>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title" id="batch-selected-title"></h2>
            <p class="card-subtitle" id="batch-selected-sub"></p>
          </div>
          <button type="button" class="btn btn-sm" id="batch-open-source-folder" disabled>${UI_ICONS.finder}<span id="batch-open-source-label"></span></button>
        </div>
        <div class="batch-file-list" id="batch-file-list"></div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title" id="batch-output-title"></h2>
            <p class="card-subtitle" id="batch-compat-subtitle"></p>
          </div>
          <div style="display:flex;gap:6px">
            <button type="button" class="btn btn-sm" id="batch-select-all"></button>
            <button type="button" class="btn btn-sm" id="batch-select-none"></button>
          </div>
        </div>
        <div class="batch-format-grid" id="batch-format-grid"></div>
        <div id="batch-actions-wrap" style="margin-top:12px;display:none">
          <div class="setting-desc" style="margin-bottom:6px;font-weight:600" id="batch-actions-heading"></div>
          <div class="batch-format-grid" id="batch-actions-grid"></div>
        </div>
      </section>

      <section class="glass-card">
        <label class="batch-raw-toggle" for="batch-raw-mode-toggle">
          <input type="checkbox" id="batch-raw-mode-toggle" />
          <span class="batch-raw-toggle-text">
            <span class="batch-raw-toggle-label" id="batch-raw-mode-label"></span>
            <span class="setting-desc" id="batch-raw-mode-hint"></span>
          </span>
        </label>
        <div style="display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap">
          <div style="display:flex;gap:8px;align-items:center">
            <button type="button" class="btn btn-primary" id="batch-run-btn" disabled></button>
            <button type="button" class="btn btn-danger" id="batch-cancel-btn" disabled></button>
            <button type="button" class="btn btn-sm" id="batch-progress-copy-diag" style="display:none"></button>
          </div>
          <div class="setting-desc" id="batch-progress-label"></div>
        </div>
        <div class="progress-bar-wrap" style="margin-top:12px">
          <div class="progress-bar-fill" id="batch-progress-fill" style="width:0%"></div>
        </div>
        <div id="batch-progress-diag-fallback" style="display:none;margin-top:10px"></div>
      </section>

      <section class="glass-card" id="batch-results-card" style="display:none">
        <div class="card-header">
          <div>
            <h2 class="card-title" id="batch-results-title"></h2>
            <p class="card-subtitle" id="batch-results-summary"></p>
          </div>
          <button type="button" class="btn btn-sm" id="batch-results-copy-diag" style="display:none"></button>
        </div>
        <div id="batch-results-diag-fallback" style="display:none;margin-bottom:10px"></div>
        <div class="batch-results-list" id="batch-results-list"></div>
      </section>
    </div>
  `;

  const dropzoneEl = root.querySelector<HTMLElement>("#batch-dropzone")!;
  const fileListEl = root.querySelector<HTMLElement>("#batch-file-list")!;
  const openSourceBtn = root.querySelector<HTMLButtonElement>("#batch-open-source-folder")!;
  const formatGridEl = root.querySelector<HTMLElement>("#batch-format-grid")!;
  const actionsWrapEl = root.querySelector<HTMLElement>("#batch-actions-wrap")!;
  const actionsGridEl = root.querySelector<HTMLElement>("#batch-actions-grid")!;
  const compatSubtitleEl = root.querySelector<HTMLElement>("#batch-compat-subtitle")!;
  const runBtn = root.querySelector<HTMLButtonElement>("#batch-run-btn")!;
  const cancelBtn = root.querySelector<HTMLButtonElement>("#batch-cancel-btn")!;
  const rawModeChk = root.querySelector<HTMLInputElement>("#batch-raw-mode-toggle")!;
  const rawModeLabelEl = root.querySelector<HTMLElement>("#batch-raw-mode-label")!;
  const rawModeHintEl = root.querySelector<HTMLElement>("#batch-raw-mode-hint")!;
  const progressCopyDiagBtn = root.querySelector<HTMLButtonElement>("#batch-progress-copy-diag")!;
  const progressDiagFallbackEl = root.querySelector<HTMLElement>("#batch-progress-diag-fallback")!;
  const progressLabelEl = root.querySelector<HTMLElement>("#batch-progress-label")!;
  const progressFillEl = root.querySelector<HTMLElement>("#batch-progress-fill")!;
  const resultsCardEl = root.querySelector<HTMLElement>("#batch-results-card")!;
  const resultsSummaryEl = root.querySelector<HTMLElement>("#batch-results-summary")!;
  const resultsCopyDiagBtn = root.querySelector<HTMLButtonElement>("#batch-results-copy-diag")!;
  const resultsDiagFallbackEl = root.querySelector<HTMLElement>("#batch-results-diag-fallback")!;
  const resultsListEl = root.querySelector<HTMLElement>("#batch-results-list")!;
  const pathInput = root.querySelector<HTMLInputElement>("#batch-path-input")!;

  function applyStaticLocalization(): void {
    const l = lang();
    root.querySelector("#batch-title")!.textContent = t(l, "batch.title");
    root.querySelector("#batch-subtitle")!.textContent = t(l, "batch.subtitle");
    root.querySelector("#batch-open-settings-label")!.textContent = t(l, "nav.settings");
    root.querySelector("#batch-drop-title")!.textContent = t(l, "batch.dropTitle");
    root.querySelector("#batch-drop-desc")!.textContent = t(l, "batch.dropDesc");
    root.querySelector("#batch-load-wheel")!.textContent = t(l, "batch.loadWheel");
    root.querySelector("#batch-choose-files-label")!.textContent = t(l, "batch.chooseFiles");
    root.querySelector("#batch-clear-files")!.textContent = t(l, "batch.clearList");
    pathInput.placeholder = t(l, "batch.pathPlaceholder");
    root.querySelector("#batch-add-path")!.textContent = t(l, "batch.addPath");
    root.querySelector("#batch-selected-title")!.innerHTML = t(l, "batch.selectedFiles", { count: files.length });
    root.querySelector("#batch-selected-sub")!.textContent = t(l, "batch.selectedSub");
    root.querySelector("#batch-open-source-label")!.textContent = t(l, "batch.openSourceFolder");
    root.querySelector("#batch-output-title")!.textContent = t(l, "batch.outputFormats");
    root.querySelector("#batch-select-all")!.textContent = t(l, "batch.selectAll");
    root.querySelector("#batch-select-none")!.textContent = t(l, "batch.clearSelection");
    root.querySelector("#batch-actions-heading")!.textContent = t(l, "batch.batchActionsHeading");
    runBtn.textContent = t(l, "batch.convertSelected");
    cancelBtn.textContent = t(l, "common.cancel");
    rawModeLabelEl.textContent = t(l, "batch.raw_mode");
    rawModeHintEl.textContent = t(l, "batch.raw_mode_hint");
    root.querySelector("#batch-results-title")!.textContent = t(l, "batch.resultsTitle");
  }

  function render(): void {
    const l = lang();
    applyStaticLocalization();
    openSourceBtn.disabled = files.length === 0;

    if (files.length === 0) {
      fileListEl.innerHTML = `<div class="setting-desc">${escapeHtml(t(l, "batch.noFilesYet"))}</div>`;
    } else {
      fileListEl.innerHTML = files
        .map((f, idx) => {
          const ext = extLower(f).toUpperCase() || "FILE";
          return `
            <div class="batch-file-item">
              <div style="min-width:0;flex:1">
                <div style="font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis" title="${escapeHtml(f)}">${escapeHtml(basename(f))}</div>
                <div style="font-size:11px;color:var(--text-muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${escapeHtml(dirname(f))}</div>
              </div>
              <span class="code-chip">${escapeHtml(ext)}</span>
              <button type="button" class="btn btn-sm" data-remove-file="${idx}" ${busy ? "disabled" : ""} aria-label="${escapeHtml(t(l, "batch.removeFileAria"))}">${UI_ICONS.trash}</button>
            </div>
          `;
        })
        .join("");
    }

    const allOptions = [...inspection.compatibleFormats, ...inspection.batchActions];
    selectedFormats = new Set(Array.from(selectedFormats).filter((f) => allOptions.includes(f)));

    const formatDisplayLabel = (fmt: string): string => {
      switch (fmt) {
        case "COMBINE_PDF":
          return t(l, "wheel.actionCombinePdf");
        case "ANIMATE_GIF":
          return t(l, "wheel.actionAnimateGif");
        case "MERGE_PDF":
          return t(l, "wheel.actionMergePdf");
        case "ZIP":
          return t(l, "wheel.actionZip");
        case "UNZIP":
          return t(l, "wheel.actionUnzip");
        case "REMOVE_BG":
          return t(l, "wheel.actionRemoveBg");
        case "OPTIMIZE_PDF":
          return t(l, "wheel.actionOptimizePdf");
        default:
          return getFormatMeta(fmt, l).shortLabel;
      }
    };

    if (files.length === 0) {
      compatSubtitleEl.textContent = t(l, "batch.addFilesToInspect");
      formatGridEl.innerHTML = `<div class="setting-desc">${escapeHtml(t(l, "batch.formatsWillAppear"))}</div>`;
      actionsWrapEl.style.display = "none";
    } else if (allOptions.length === 0) {
      const msg = localizeAvailabilityMessage(l, inspection.availabilityMessage).replace(/\n/g, " · ");
      compatSubtitleEl.textContent = msg;
      formatGridEl.innerHTML = `<div class="status-pill is-warn">${escapeHtml(msg)}</div>`;
      actionsWrapEl.style.display = "none";
    } else {
      const noun = files.length === 1 ? t(l, "vo.fileSingular") : t(l, "vo.filePlural");
      compatSubtitleEl.textContent = t(l, "batch.sharedCount", {
        count: files.length,
        noun,
        formats: inspection.compatibleFormats.length,
      });
      formatGridEl.innerHTML =
        inspection.compatibleFormats.length === 0
          ? `<div class="setting-desc">${escapeHtml(t(l, "batch.onlyArchiveActions"))}</div>`
          : inspection.compatibleFormats
              .map((fmt) => {
                const checked = selectedFormats.has(fmt);
                return `
                  <label class="format-check-pill">
                    <input type="checkbox" data-format-chk="${escapeHtml(fmt)}" ${checked ? "checked" : ""} ${busy ? "disabled" : ""} />
                    <span>${escapeHtml(formatDisplayLabel(fmt))}</span>
                  </label>
                `;
              })
              .join("");

      if (inspection.batchActions.length > 0) {
        actionsWrapEl.style.display = "block";
        actionsGridEl.innerHTML = inspection.batchActions
          .map((act) => {
            const checked = selectedFormats.has(act);
            return `
              <label class="format-check-pill">
                <input type="checkbox" data-format-chk="${escapeHtml(act)}" ${checked ? "checked" : ""} ${busy ? "disabled" : ""} />
                <span>${escapeHtml(formatDisplayLabel(act))}</span>
              </label>
            `;
          })
          .join("");
      } else {
        actionsWrapEl.style.display = "none";
      }
    }

    rawModeChk.checked = rawMode;
    rawModeChk.disabled = busy;
    runBtn.disabled = busy || files.length === 0 || selectedFormats.size === 0;
    cancelBtn.disabled = !busy;
    progressLabelEl.textContent = resolveStatusText();
    progressFillEl.style.width = `${Math.max(0, Math.min(100, progressPct))}%`;

    const errCount = results.filter((r) => Boolean(r.error)).length;
    const hasTopLevelErr = Boolean(lastBatchError) && errCount === 0;
    const diagBtnText = diagState === "copied" ? t(l, "common.copied") : t(l, "common.copyDiagnostics");

    progressCopyDiagBtn.style.display = hasTopLevelErr ? "inline-flex" : "none";
    progressCopyDiagBtn.textContent = diagBtnText;
    if (hasTopLevelErr && diagState === "manual" && diagFallbackText) {
      progressDiagFallbackEl.style.display = "block";
      progressDiagFallbackEl.innerHTML = `<div class="setting-desc" style="margin-bottom:4px">${escapeHtml(t(l, "common.clipboardRestricted"))}</div><textarea class="mac-input diag-textarea" readonly rows="6">${escapeHtml(diagFallbackText)}</textarea>`;
    } else {
      progressDiagFallbackEl.style.display = "none";
      progressDiagFallbackEl.innerHTML = "";
    }

    if (results.length > 0) {
      resultsCardEl.style.display = "block";
      const okOutputs = results.flatMap((r) => r.outputs);
      resultsSummaryEl.textContent = t(l, "batch.resultsSummary", {
        okCount: okOutputs.length,
        errCount,
      });
      resultsCopyDiagBtn.style.display = errCount > 0 ? "inline-flex" : "none";
      resultsCopyDiagBtn.textContent = diagBtnText;
      if (errCount > 0 && diagState === "manual" && diagFallbackText) {
        resultsDiagFallbackEl.style.display = "block";
        resultsDiagFallbackEl.innerHTML = `<div class="setting-desc" style="margin-bottom:4px">${escapeHtml(t(l, "common.clipboardRestricted"))}</div><textarea class="mac-input diag-textarea" readonly rows="6">${escapeHtml(diagFallbackText)}</textarea>`;
      } else {
        resultsDiagFallbackEl.style.display = "none";
        resultsDiagFallbackEl.innerHTML = "";
      }
      resultsListEl.innerHTML = results
        .map((r, idx) => {
          const hasErr = Boolean(r.error);
          const detail = hasErr
            ? r.error!
            : r.outputs.map((p) => basename(p)).join(", ");
          const fmtLabel = formatDisplayLabel(r.format);
          return `
            <div class="batch-result-row">
              <div style="min-width:0;flex:1">
                <div style="display:flex;align-items:center;gap:8px">
                  <span class="status-pill ${hasErr ? "is-bad" : "is-ok"}">
                    <span class="status-dot"></span>${escapeHtml(hasErr ? t(l, "batch.statusError") : t(l, "batch.statusDone"))}
                  </span>
                  <strong>${escapeHtml(basename(r.source))} → ${escapeHtml(fmtLabel)}</strong>
                </div>
                <div class="setting-desc" title="${escapeHtml(hasErr ? r.error! : r.outputs.join("\n"))}">${escapeHtml(detail)}</div>
              </div>
              ${
                !hasErr && r.outputs.length > 0
                  ? `<button type="button" class="btn btn-sm" data-reveal-result="${idx}">${UI_ICONS.finder}<span>${escapeHtml(t(l, "common.showInFinder"))}</span></button>`
                  : ""
              }
            </div>
          `;
        })
        .join("");
    } else {
      resultsCardEl.style.display = "none";
      resultsCopyDiagBtn.style.display = "none";
      resultsDiagFallbackEl.style.display = "none";
      resultsDiagFallbackEl.innerHTML = "";
    }

    bindDynamicEvents();
  }

  function bindDynamicEvents(): void {
    fileListEl.querySelectorAll<HTMLButtonElement>("[data-remove-file]").forEach((btn) => {
      btn.addEventListener("click", () => {
        if (busy) return;
        const idx = Number(btn.dataset.removeFile!);
        const next = files.filter((_, i) => i !== idx);
        void setFiles(next);
      });
    });

    root.querySelectorAll<HTMLInputElement>("[data-format-chk]").forEach((chk) => {
      chk.addEventListener("change", () => {
        const fmt = chk.dataset.formatChk!;
        if (chk.checked) selectedFormats.add(fmt);
        else selectedFormats.delete(fmt);
        runBtn.disabled = busy || files.length === 0 || selectedFormats.size === 0;
      });
    });

    resultsListEl.querySelectorAll<HTMLButtonElement>("[data-reveal-result]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const r = results[Number(btn.dataset.revealResult!)];
        if (r && r.outputs.length > 0) {
          void revealInFinder(r.outputs);
        }
      });
    });
  }

  async function setFiles(nextPaths: string[], append = false, preserveErrors = false): Promise<void> {
    if (busy) return;
    if (!preserveErrors) {
      lastBatchError = null;
      diagState = "idle";
      diagFallbackText = "";
    }
    const merged = append ? [...files, ...nextPaths] : nextPaths;
    const deduped: string[] = [];
    const seen = new Set<string>();
    for (const p of merged) {
      const clean = p.trim();
      if (clean && !seen.has(clean)) {
        seen.add(clean);
        deduped.push(clean);
      }
    }
    files = deduped;
    inspection = await inspectFiles(files).catch(() => ({
      files,
      compatibleFormats: [],
      batchActions: [],
      availabilityMessage: t(lang(), "batch.inspectFailed"),
      family: null,
    }));
    if (disposed) return;
    files = inspection.files;
    const allOptions = [...inspection.compatibleFormats, ...inspection.batchActions];
    selectedFormats = new Set(Array.from(selectedFormats).filter((f) => allOptions.includes(f)));
    if (selectedFormats.size === 0) {
      if (inspection.compatibleFormats.length > 0) {
        selectedFormats.add(inspection.compatibleFormats[0]);
      } else if (inspection.batchActions.length > 0) {
        selectedFormats.add(inspection.batchActions[0]);
      }
    }
    progressPct = 0;
    statusMode =
      files.length === 0
        ? { kind: "prompt" }
        : { kind: "ready", count: files.length };
    render();
  }

  root.querySelector("#batch-open-settings")?.addEventListener("click", () => void openWindow("settings"));

  root.querySelector("#batch-load-wheel")?.addEventListener("click", async () => {
    const ws = await getWheelState().catch(() => null);
    if (disposed) return;
    if (ws && ws.files.length > 0) {
      results = [];
      lastBatchError = null;
      diagState = "idle";
      diagFallbackText = "";
      await setFiles(ws.files);
    } else {
      statusMode = { kind: "noSelection" };
      render();
    }
  });

  root.querySelector("#batch-clear-files")?.addEventListener("click", () => {
    results = [];
    lastBatchError = null;
    diagState = "idle";
    diagFallbackText = "";
    void setFiles([]);
  });

  openSourceBtn.addEventListener("click", () => {
    if (files.length > 0) {
      void revealInFinder([files[0]]);
    }
  });

  root.querySelector("#batch-select-all")?.addEventListener("click", () => {
    if (busy) return;
    for (const f of inspection.compatibleFormats) selectedFormats.add(f);
    render();
  });

  root.querySelector("#batch-select-none")?.addEventListener("click", () => {
    if (busy) return;
    selectedFormats.clear();
    render();
  });

  const addInputPath = () => {
    const val = pathInput.value.trim();
    if (!val) return;
    pathInput.value = "";
    void setFiles([val], true);
  };
  root.querySelector("#batch-add-path")?.addEventListener("click", addInputPath);
  pathInput.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      addInputPath();
    }
  });

  root.querySelector<HTMLInputElement>("#batch-file-input")?.addEventListener("change", (ev) => {
    const input = ev.target as HTMLInputElement;
    const rawFiles = Array.from(input.files ?? []);
    const picked: string[] = [];
    let missingNativePath = false;
    for (const f of rawFiles) {
      const withPath = f as File & { path?: string };
      if (withPath.path) {
        picked.push(withPath.path);
      } else if (!isTauriRuntime()) {
        picked.push(`/Users/demo/Desktop/${f.name}`);
      } else {
        missingNativePath = true;
      }
    }
    input.value = "";
    if (picked.length > 0) {
      void setFiles(picked, true);
    } else if (missingNativePath) {
      statusMode = { kind: "webkitHidden" };
      render();
    }
  });

  async function triggerBatchCopyDiagnostics(): Promise<void> {
    if (diagResetTimer !== null) {
      window.clearTimeout(diagResetTimer);
      diagResetTimer = null;
    }
    try {
      const res = await copyDiagnosticsToClipboard();
      if (disposed) return;
      if (res.ok) {
        diagState = "copied";
        diagFallbackText = "";
        render();
        diagResetTimer = window.setTimeout(() => {
          diagResetTimer = null;
          if (!disposed && diagState === "copied") {
            diagState = "idle";
            render();
          }
        }, 2200);
      } else {
        diagState = "manual";
        diagFallbackText = res.report;
        render();
      }
    } catch (err) {
      if (!disposed) {
        statusMode = { kind: "diagFailed", msg: formatError(err) };
        render();
      }
    }
  }

  progressCopyDiagBtn.addEventListener("click", () => void triggerBatchCopyDiagnostics());
  resultsCopyDiagBtn.addEventListener("click", () => void triggerBatchCopyDiagnostics());

  function finishBatchRun(res: { success: boolean; results?: ConversionResult[]; error: string | null }): void {
    if (!busy || disposed) return;
    results = res.results ?? [];
    busy = false;
    const createdCount = results.flatMap((r) => r.outputs).length;
    const firstErr = res.error || results.find((r) => Boolean(r.error))?.error || null;
    lastBatchError = !res.success || firstErr ? firstErr || t(lang(), "batch.conversionFailedDefault") : null;
    progressPct = res.error && createdCount === 0 ? 0 : 100;
    statusMode = res.error
      ? { kind: "finishedErr", err: res.error }
      : { kind: "completed", count: createdCount };
    render();
  }

  rawModeChk.addEventListener("change", () => {
    rawMode = rawModeChk.checked;
  });

  runBtn.addEventListener("click", async () => {
    if (busy || files.length === 0 || selectedFormats.size === 0) return;
    busy = true;
    results = [];
    lastBatchError = null;
    diagState = "idle";
    diagFallbackText = "";
    progressPct = 4;
    statusMode = { kind: "starting" };
    render();

    try {
      const formatsList = Array.from(selectedFormats);
      const res = await executeConversion(files, formatsList, { rawMode });
      finishBatchRun(res);
    } catch (err) {
      if (disposed || !busy) return;
      busy = false;
      const errMsg = formatError(err);
      lastBatchError = errMsg;
      progressPct = 0;
      statusMode = { kind: "failed", msg: errMsg };
      render();
    }
  });

  cancelBtn.addEventListener("click", () => {
    if (!busy) return;
    statusMode = { kind: "cancelling" };
    render();
    void cancelConversion();
  });

  // HTML5 Drag/Drop (Browser Preview)
  const onDragOver = (ev: DragEvent) => {
    ev.preventDefault();
    dropzoneEl.classList.add("is-drag-over");
  };
  const onDragLeave = () => {
    dropzoneEl.classList.remove("is-drag-over");
  };
  const onDrop = (ev: DragEvent) => {
    ev.preventDefault();
    dropzoneEl.classList.remove("is-drag-over");
    if (isTauriRuntime()) return;
    const dropped = Array.from(ev.dataTransfer?.files ?? []).map((f) => {
      const withPath = f as File & { path?: string };
      return withPath.path || `/Users/demo/Desktop/${f.name}`;
    });
    if (dropped.length > 0) void setFiles(dropped, true);
  };

  dropzoneEl.addEventListener("dragover", onDragOver);
  dropzoneEl.addEventListener("dragleave", onDragLeave);
  dropzoneEl.addEventListener("drop", onDrop);

  const unlistenFns: Array<() => void> = [];

  void (async () => {
    const [loadedPrefs, subs] = await Promise.all([
      getPreferences().catch(() => defaultPreferences()),
      Promise.all([
        events.onPrefsChanged((updated) => {
          if (disposed) return;
          prefs = updated;
          render();
        }),
        events.onConversionProgress((p) => {
          if (disposed || !busy) return;
          progressPct = Math.max(progressPct, p.percent);
          if (p.label) {
            statusMode = { kind: "live", text: p.label };
          }
          progressFillEl.style.width = `${progressPct}%`;
          progressLabelEl.textContent = resolveStatusText();
        }),
        events.onConversionComplete((p) => {
          finishBatchRun(p);
        }),
        events.onBatchFiles((p) => {
          if (disposed) return;
          consumeBatchHandoff();
          results = p.results ?? [];
          const unitErr = results.find((r) => Boolean(r.error))?.error ?? null;
          lastBatchError = unitErr;
          diagState = "idle";
          diagFallbackText = "";
          if (p.files.length > 0) {
            void setFiles(p.files, false, true);
          } else {
            render();
          }
        }),
        subscribeNativeDragDrop({
          onEnter: () => dropzoneEl.classList.add("is-drag-over"),
          onLeave: () => dropzoneEl.classList.remove("is-drag-over"),
          onDrop: (paths) => {
            dropzoneEl.classList.remove("is-drag-over");
            if (!disposed && paths.length > 0) void setFiles(paths, true);
          },
        }),
      ]),
    ]);

    if (disposed) {
      subs.forEach((u) => u());
      return;
    }
    prefs = loadedPrefs;
    unlistenFns.push(...subs);

    const handoff = consumeBatchHandoff();
    if (handoff.results) results = handoff.results;
    if (handoff.files.length > 0) {
      await setFiles(handoff.files);
    } else if (files.length === 0) {
      const ws = await getWheelState().catch(() => null);
      if (!disposed && files.length === 0 && ws && ws.files.length > 0) {
        await setFiles(ws.files);
      } else if (!disposed && files.length === 0) {
        render();
      }
    }
  })();

  return () => {
    disposed = true;
    if (diagResetTimer !== null) window.clearTimeout(diagResetTimer);
    dropzoneEl.removeEventListener("dragover", onDragOver);
    dropzoneEl.removeEventListener("dragleave", onDragLeave);
    dropzoneEl.removeEventListener("drop", onDrop);
    unlistenFns.forEach((fn) => fn());
  };
}
