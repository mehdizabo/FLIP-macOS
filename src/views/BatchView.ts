import { getFormatMeta, UI_ICONS } from "../icons";
import {
  basename,
  cancelConversion,
  consumeBatchHandoff,
  dirname,
  escapeHtml,
  events,
  executeConversion,
  extLower,
  formatError,
  getWheelState,
  inspectFiles,
  isTauriRuntime,
  openWindow,
  revealInFinder,
  subscribeNativeDragDrop,
} from "../ipc";
import type { ConversionResult, InspectFilesResult } from "../types";

export function mountBatchView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "batch";

  let disposed = false;
  let files: string[] = [];
  let inspection: InspectFilesResult = {
    files: [],
    compatibleFormats: [],
    batchActions: [],
    availabilityMessage: "Drop local files here",
    family: null,
  };
  let selectedFormats = new Set<string>();
  let busy = false;
  let progressPct = 0;
  let progressLabel = "Select files and one or more output formats.";
  let results: ConversionResult[] = [];

  root.innerHTML = `
    <div class="batch-shell">
      <div class="page-header">
        <div>
          <h1 class="page-title">Batch &amp; Multi-Format Converter</h1>
          <p class="card-subtitle">Convert multiple files or emit several formats locally — originals are never overwritten.</p>
        </div>
        <div style="display:flex;gap:8px">
          <button type="button" class="btn btn-sm" id="batch-open-settings">${UI_ICONS.settings}<span>Settings</span></button>
        </div>
      </div>

      <section class="batch-dropzone" id="batch-dropzone">
        <div style="color:var(--accent)">${UI_ICONS.drop}</div>
        <div style="font-weight:600;font-size:14px">Drop files from Finder here</div>
        <div class="setting-desc">Or load the current Finder/Wheel selection, or add a local path.</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap;justify-content:center;margin-top:4px">
          <button type="button" class="btn btn-sm" id="batch-load-wheel">Load Wheel / Finder Selection</button>
          <label class="btn btn-sm" style="cursor:pointer">
            <span>Choose Files…</span>
            <input type="file" id="batch-file-input" multiple style="display:none" />
          </label>
          <button type="button" class="btn btn-sm" id="batch-clear-files">Clear List</button>
        </div>
        <div style="display:flex;gap:6px;width:100%;max-width:520px;margin-top:6px">
          <input type="text" class="mac-input" id="batch-path-input" placeholder="/Users/.../file.png (press Enter to add)" style="flex:1" />
          <button type="button" class="btn btn-sm" id="batch-add-path">Add</button>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Selected Files (<span id="batch-file-count">0</span>)</h2>
            <p class="card-subtitle">Compatible formats update automatically for the entire selection</p>
          </div>
          <button type="button" class="btn btn-sm" id="batch-open-source-folder" disabled>${UI_ICONS.finder}<span>Open Source Folder</span></button>
        </div>
        <div class="batch-file-list" id="batch-file-list"></div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Output Formats</h2>
            <p class="card-subtitle" id="batch-compat-subtitle">Choose one or more target formats</p>
          </div>
          <div style="display:flex;gap:6px">
            <button type="button" class="btn btn-sm" id="batch-select-all">Select All</button>
            <button type="button" class="btn btn-sm" id="batch-select-none">Clear</button>
          </div>
        </div>
        <div class="batch-format-grid" id="batch-format-grid"></div>
        <div id="batch-actions-wrap" style="margin-top:12px;display:none">
          <div class="setting-desc" style="margin-bottom:6px;font-weight:600">Batch Combine / Merge Actions</div>
          <div class="batch-format-grid" id="batch-actions-grid"></div>
        </div>
      </section>

      <section class="glass-card">
        <div style="display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap">
          <div style="display:flex;gap:8px;align-items:center">
            <button type="button" class="btn btn-primary" id="batch-run-btn" disabled>Convert Selected</button>
            <button type="button" class="btn btn-danger" id="batch-cancel-btn" disabled>Cancel</button>
          </div>
          <div class="setting-desc" id="batch-progress-label">${escapeHtml(progressLabel)}</div>
        </div>
        <div class="progress-bar-wrap" style="margin-top:12px">
          <div class="progress-bar-fill" id="batch-progress-fill" style="width:0%"></div>
        </div>
      </section>

      <section class="glass-card" id="batch-results-card" style="display:none">
        <div class="card-header">
          <div>
            <h2 class="card-title">Conversion Results</h2>
            <p class="card-subtitle" id="batch-results-summary"></p>
          </div>
        </div>
        <div class="batch-results-list" id="batch-results-list"></div>
      </section>
    </div>
  `;

  const dropzoneEl = root.querySelector<HTMLElement>("#batch-dropzone")!;
  const fileListEl = root.querySelector<HTMLElement>("#batch-file-list")!;
  const fileCountEl = root.querySelector<HTMLElement>("#batch-file-count")!;
  const openSourceBtn = root.querySelector<HTMLButtonElement>("#batch-open-source-folder")!;
  const formatGridEl = root.querySelector<HTMLElement>("#batch-format-grid")!;
  const actionsWrapEl = root.querySelector<HTMLElement>("#batch-actions-wrap")!;
  const actionsGridEl = root.querySelector<HTMLElement>("#batch-actions-grid")!;
  const compatSubtitleEl = root.querySelector<HTMLElement>("#batch-compat-subtitle")!;
  const runBtn = root.querySelector<HTMLButtonElement>("#batch-run-btn")!;
  const cancelBtn = root.querySelector<HTMLButtonElement>("#batch-cancel-btn")!;
  const progressLabelEl = root.querySelector<HTMLElement>("#batch-progress-label")!;
  const progressFillEl = root.querySelector<HTMLElement>("#batch-progress-fill")!;
  const resultsCardEl = root.querySelector<HTMLElement>("#batch-results-card")!;
  const resultsSummaryEl = root.querySelector<HTMLElement>("#batch-results-summary")!;
  const resultsListEl = root.querySelector<HTMLElement>("#batch-results-list")!;

  function render(): void {
    fileCountEl.textContent = String(files.length);
    openSourceBtn.disabled = files.length === 0;

    if (files.length === 0) {
      fileListEl.innerHTML = `<div class="setting-desc">No files added yet.</div>`;
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
              <button type="button" class="btn btn-sm" data-remove-file="${idx}" ${busy ? "disabled" : ""} aria-label="Remove file">${UI_ICONS.trash}</button>
            </div>
          `;
        })
        .join("");
    }

    const allOptions = [...inspection.compatibleFormats, ...inspection.batchActions];
    selectedFormats = new Set(Array.from(selectedFormats).filter((f) => allOptions.includes(f)));

    if (files.length === 0) {
      compatSubtitleEl.textContent = "Add files to inspect compatible formats";
      formatGridEl.innerHTML = `<div class="setting-desc">Compatible output formats will appear here.</div>`;
      actionsWrapEl.style.display = "none";
    } else if (allOptions.length === 0) {
      compatSubtitleEl.textContent = inspection.availabilityMessage.replace(/\n/g, " · ");
      formatGridEl.innerHTML = `<div class="status-pill is-warn">${escapeHtml(inspection.availabilityMessage.replace(/\n/g, " · "))}</div>`;
      actionsWrapEl.style.display = "none";
    } else {
      compatSubtitleEl.textContent = `${files.length} ${files.length === 1 ? "file" : "files"} · ${inspection.compatibleFormats.length} shared formats`;
      formatGridEl.innerHTML = inspection.compatibleFormats
        .map((fmt) => {
          const checked = selectedFormats.has(fmt);
          const meta = getFormatMeta(fmt);
          return `
            <label class="format-check-pill">
              <input type="checkbox" data-format-chk="${escapeHtml(fmt)}" ${checked ? "checked" : ""} ${busy ? "disabled" : ""} />
              <span>${escapeHtml(meta.shortLabel)}</span>
            </label>
          `;
        })
        .join("");

      if (inspection.batchActions.length > 0) {
        actionsWrapEl.style.display = "block";
        actionsGridEl.innerHTML = inspection.batchActions
          .map((act) => {
            const checked = selectedFormats.has(act);
            const label = act === "COMBINE_PDF" ? "Combine into PDF" : act === "MERGE_PDF" ? "Merge PDFs" : act;
            return `
              <label class="format-check-pill">
                <input type="checkbox" data-format-chk="${escapeHtml(act)}" ${checked ? "checked" : ""} ${busy ? "disabled" : ""} />
                <span>${escapeHtml(label)}</span>
              </label>
            `;
          })
          .join("");
      } else {
        actionsWrapEl.style.display = "none";
      }
    }

    runBtn.disabled = busy || files.length === 0 || selectedFormats.size === 0;
    cancelBtn.disabled = !busy;
    progressLabelEl.textContent = progressLabel;
    progressFillEl.style.width = `${Math.max(0, Math.min(100, progressPct))}%`;

    if (results.length > 0) {
      resultsCardEl.style.display = "block";
      const okOutputs = results.flatMap((r) => r.outputs);
      const errCount = results.filter((r) => Boolean(r.error)).length;
      resultsSummaryEl.textContent = `${okOutputs.length} output(s) created, ${errCount} error(s).`;
      resultsListEl.innerHTML = results
        .map((r, idx) => {
          const hasErr = Boolean(r.error);
          const detail = hasErr
            ? r.error!
            : r.outputs.map((p) => basename(p)).join(", ");
          const fmtLabel =
            r.format === "COMBINE_PDF"
              ? "Combine into PDF"
              : r.format === "MERGE_PDF"
                ? "Merge PDFs"
                : r.format;
          return `
            <div class="batch-result-row">
              <div style="min-width:0;flex:1">
                <div style="display:flex;align-items:center;gap:8px">
                  <span class="status-pill ${hasErr ? "is-bad" : "is-ok"}">
                    <span class="status-dot"></span>${hasErr ? "Error" : "Done"}
                  </span>
                  <strong>${escapeHtml(basename(r.source))} → ${escapeHtml(fmtLabel)}</strong>
                </div>
                <div class="setting-desc" title="${escapeHtml(hasErr ? r.error! : r.outputs.join("\n"))}">${escapeHtml(detail)}</div>
              </div>
              ${
                !hasErr && r.outputs.length > 0
                  ? `<button type="button" class="btn btn-sm" data-reveal-result="${idx}">${UI_ICONS.finder}<span>Show in Finder</span></button>`
                  : ""
              }
            </div>
          `;
        })
        .join("");
    } else {
      resultsCardEl.style.display = "none";
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

  async function setFiles(nextPaths: string[], append = false): Promise<void> {
    if (busy) return;
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
      availabilityMessage: "Failed to inspect files",
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
    progressLabel =
      files.length === 0
        ? "Select files and one or more output formats."
        : `${files.length} file(s) ready.`;
    render();
  }

  root.querySelector("#batch-open-settings")?.addEventListener("click", () => void openWindow("settings"));

  root.querySelector("#batch-load-wheel")?.addEventListener("click", async () => {
    const ws = await getWheelState().catch(() => null);
    if (!disposed && ws && ws.files.length > 0) {
      results = [];
      await setFiles(ws.files);
    }
  });

  root.querySelector("#batch-clear-files")?.addEventListener("click", () => {
    results = [];
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

  const pathInput = root.querySelector<HTMLInputElement>("#batch-path-input")!;
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
      progressLabel = "Drag files from Finder or enter a local path above (WebKit hides file paths from standard file inputs).";
      render();
    }
  });

  function finishBatchRun(res: { success: boolean; results?: ConversionResult[]; error: string | null }): void {
    if (!busy || disposed) return;
    results = res.results ?? [];
    busy = false;
    const createdCount = results.flatMap((r) => r.outputs).length;
    progressPct = res.error && createdCount === 0 ? 0 : 100;
    progressLabel = res.error
      ? `Finished with error: ${res.error}`
      : `Completed ${createdCount} output(s).`;
    render();
  }

  runBtn.addEventListener("click", async () => {
    if (busy || files.length === 0 || selectedFormats.size === 0) return;
    busy = true;
    results = [];
    progressPct = 4;
    progressLabel = "Starting conversion…";
    render();

    try {
      const formatsList = Array.from(selectedFormats);
      const res = await executeConversion(files, formatsList);
      finishBatchRun(res);
    } catch (err) {
      if (disposed || !busy) return;
      busy = false;
      progressPct = 0;
      progressLabel = `Failed: ${formatError(err)}`;
      render();
    }
  });

  cancelBtn.addEventListener("click", () => {
    if (!busy) return;
    progressLabel = "Cancelling…";
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
    const handoff = consumeBatchHandoff();
    if (handoff.results) results = handoff.results;
    if (handoff.files.length > 0) {
      await setFiles(handoff.files);
    } else {
      const ws = await getWheelState().catch(() => null);
      if (!disposed && ws && ws.files.length > 0) {
        await setFiles(ws.files);
      } else {
        render();
      }
    }

    const subs = await Promise.all([
      events.onConversionProgress((p) => {
        if (disposed || !busy) return;
        progressPct = Math.max(progressPct, p.percent);
        if (p.label) progressLabel = p.label;
        progressFillEl.style.width = `${progressPct}%`;
        progressLabelEl.textContent = progressLabel;
      }),
      events.onConversionComplete((p) => {
        finishBatchRun(p);
      }),
      events.onBatchFiles((p) => {
        if (disposed) return;
        consumeBatchHandoff();
        results = p.results ?? [];
        if (p.files.length > 0) {
          void setFiles(p.files);
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
    ]);

    if (disposed) {
      subs.forEach((u) => u());
      return;
    }
    unlistenFns.push(...subs);
  })();

  return () => {
    disposed = true;
    dropzoneEl.removeEventListener("dragover", onDragOver);
    dropzoneEl.removeEventListener("dragleave", onDragLeave);
    dropzoneEl.removeEventListener("drop", onDrop);
    unlistenFns.forEach((fn) => fn());
  };
}
