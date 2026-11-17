import { UI_ICONS } from "../icons";
import {
  defaultPreferences,
  escapeHtml,
  formatError,
  getEngineStatus,
  getNewFileKinds,
  getPermissionsStatus,
  getPreferences,
  getRecentConversions,
  openTemplatesFolder,
  openWindow,
  requestPermission,
  revealInFinder,
  revealLogs,
  savePreferences,
  basename,
} from "../ipc";
import type {
  AppPreferences,
  ClipboardStatusInfo,
  EngineStatus,
  NewFileKindInfo,
  NewFileKindsResponse,
  PermissionsStatus,
  RecentConversion,
  UserTemplateInfo,
} from "../types";

type SettingsTab =
  | "general"
  | "shortcuts"
  | "formats"
  | "newfile"
  | "engines"
  | "about";

const DRAG_PRESETS: Array<{ id: string; label: string }> = [
  { id: "Down", label: "Down Arrow" },
  { id: "Space", label: "Space" },
  { id: "Tab", label: "Tab" },
  { id: "Fn", label: "Fn" },
  { id: "F8", label: "F8" },
];

const FAMILY_LABELS: Array<{ key: string; label: string }> = [
  { key: "image", label: "Images" },
  { key: "pdf", label: "PDF Outputs" },
  { key: "document", label: "Documents" },
  { key: "data", label: "Structured Data" },
  { key: "audio", label: "Audio" },
  { key: "video", label: "Video" },
];

export function mountSettingsView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "settings";

  let disposed = false;
  let activeTab: SettingsTab = "general";
  let selectedFamily = "image";
  let prefs: AppPreferences = defaultPreferences();
  let engines: EngineStatus | null = null;
  let permissions: PermissionsStatus | null = null;
  let newFileKinds: NewFileKindInfo[] = [];
  let templates: UserTemplateInfo[] = [];
  let clipboardInfo: ClipboardStatusInfo | undefined;
  let recent: RecentConversion[] = [];

  let recordingTarget: "drag" | "idle" | null = null;
  let recorderFeedback = "";
  let recorderFeedbackTarget: "drag" | "idle" | null = null;
  let saveToastTimer: number | null = null;

  root.innerHTML = `
    <div class="settings-shell">
      <aside class="settings-sidebar" aria-label="Settings Navigation">
        <div class="brand-header">
          <div class="brand-mark">F</div>
          <div>
            <div class="brand-title">FLIP</div>
            <div class="brand-sub">macOS Liquid Glass</div>
          </div>
        </div>
        <nav class="nav-list" id="settings-nav">
          <button type="button" class="nav-item is-active" data-tab="general">${UI_ICONS.settings}<span>General</span></button>
          <button type="button" class="nav-item" data-tab="shortcuts">${UI_ICONS.keyboard}<span>Shortcuts</span></button>
          <button type="button" class="nav-item" data-tab="formats">${UI_ICONS.sliders}<span>Formats &amp; Quality</span></button>
          <button type="button" class="nav-item" data-tab="newfile">${UI_ICONS.newFile}<span>New File &amp; Templates</span></button>
          <button type="button" class="nav-item" data-tab="engines">${UI_ICONS.shield}<span>Engines &amp; Permissions</span></button>
          <button type="button" class="nav-item" data-tab="about">${UI_ICONS.info}<span>About</span></button>
        </nav>
        <div class="sidebar-footer">
          <button type="button" class="btn" id="nav-open-batch">${UI_ICONS.convert}<span>Batch Converter</span></button>
          <button type="button" class="btn" id="nav-open-onboarding"><span>Setup Tour…</span></button>
        </div>
      </aside>
      <main class="settings-main" id="settings-main"></main>
    </div>
  `;

  const navEl = root.querySelector<HTMLElement>("#settings-nav")!;
  const mainEl = root.querySelector<HTMLElement>("#settings-main")!;

  function showSavedToast(label = "Saved"): void {
    const toast = mainEl.querySelector<HTMLElement>("#save-toast");
    if (!toast) return;
    toast.textContent = label;
    toast.classList.add("is-visible");
    if (saveToastTimer !== null) window.clearTimeout(saveToastTimer);
    saveToastTimer = window.setTimeout(() => {
      toast.classList.remove("is-visible");
    }, 1500);
  }

  function clonePrefs(src: AppPreferences): AppPreferences {
    const copiedFormats: Record<string, string[]> = {};
    for (const [k, v] of Object.entries(src.preferredFormats)) {
      copiedFormats[k] = [...v];
    }
    return {
      ...src,
      preferredFormats: copiedFormats,
      newFileVisibleKinds: [...src.newFileVisibleKinds],
      recentConversions: src.recentConversions.map((r) => ({ ...r })),
    };
  }

  async function persistPrefs(mutator: (draft: AppPreferences) => void): Promise<boolean> {
    const draft = clonePrefs(prefs);
    mutator(draft);
    try {
      prefs = await savePreferences(draft);
      if (!disposed) showSavedToast("Saved");
      return true;
    } catch (err) {
      if (!disposed) showSavedToast(`Error: ${formatError(err)}`);
      return false;
    }
  }

  function formatKeycaps(spec: string): string {
    const pretty = spec
      .replace(/Command/gi, "⌘")
      .replace(/Option/gi, "⌥")
      .replace(/Control/gi, "⌃")
      .replace(/Shift/gi, "⇧");
    return pretty
      .split("+")
      .map((part) => `<span class="kbd-cap">${escapeHtml(part.trim())}</span>`)
      .join(" ");
  }

  function renderMain(): void {
    for (const btn of Array.from(navEl.querySelectorAll<HTMLButtonElement>(".nav-item"))) {
      btn.classList.toggle("is-active", btn.dataset.tab === activeTab);
    }

    const headerTitle: Record<SettingsTab, string> = {
      general: "General",
      shortcuts: "Shortcuts & Triggers",
      formats: "Formats & Quality",
      newfile: "New File & Templates",
      engines: "Engines & Permissions",
      about: "About FLIP",
    };

    let bodyHtml = "";
    switch (activeTab) {
      case "general":
        bodyHtml = renderGeneralTab();
        break;
      case "shortcuts":
        bodyHtml = renderShortcutsTab();
        break;
      case "formats":
        bodyHtml = renderFormatsTab();
        break;
      case "newfile":
        bodyHtml = renderNewFileTab();
        break;
      case "engines":
        bodyHtml = renderEnginesTab();
        break;
      case "about":
        bodyHtml = renderAboutTab();
        break;
    }

    mainEl.innerHTML = `
      <div class="page-header">
        <h1 class="page-title">${headerTitle[activeTab]}</h1>
        <span class="save-toast" id="save-toast">Saved</span>
      </div>
      ${bodyHtml}
    `;

    bindTabHandlers();
  }

  function renderGeneralTab(): string {
    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Behavior &amp; Feedback</h2>
            <p class="card-subtitle">Core desktop behavior on macOS</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Launch at login</div>
            <div class="setting-desc">Keep FLIP ready in the menu bar when you sign in.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-launchAtLogin" ${prefs.launchAtLogin ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Wheel tick sound</div>
            <div class="setting-desc">Play crisp acoustic feedback when moving across sectors.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-soundEnabled" ${prefs.soundEnabled ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Trackpad haptics</div>
            <div class="setting-desc">Subtle Force Touch alignment feedback on sector snap.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-hapticsEnabled" ${prefs.hapticsEnabled ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">File Creation &amp; Output Paths</h2>
            <p class="card-subtitle">Conversions keep the original stem without overwriting</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Auto-inline-rename on Create New File</div>
            <div class="setting-desc">Select the new item in Finder and immediately open inline rename.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-autoRenameOnCreate" ${prefs.autoRenameOnCreate ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Open file after creating</div>
            <div class="setting-desc">Launch the created file in its default macOS app right away.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-openAfterCreating" ${prefs.openAfterCreating ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Output subfolder</div>
            <div class="setting-desc">Optional subfolder beside the source file (leave blank for same folder).</div>
          </div>
          <input type="text" class="mac-input" id="pref-outputSubfolder" placeholder="Same folder" value="${escapeHtml(prefs.outputSubfolder ?? "")}" />
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Fallback Create New folder</div>
            <div class="setting-desc">Used when no Finder folder is focused (defaults to Desktop).</div>
          </div>
          <input type="text" class="mac-input" id="pref-fallbackFolder" placeholder="~/Desktop" value="${escapeHtml(prefs.fallbackFolder ?? "")}" />
        </div>
      </section>

      <section class="glass-card">
        <div class="setting-row">
          <div>
            <div class="setting-label">Diagnostic Logs</div>
            <div class="setting-desc">Inspect local JSONL events in ~/Library/Logs/FLIP.</div>
          </div>
          <button type="button" class="btn" id="btn-reveal-logs">${UI_ICONS.finder}<span>Reveal Logs</span></button>
        </div>
      </section>
    `;
  }

  function renderShortcutsTab(): string {
    const mouseVal = prefs.mouseTriggerButton === null ? "" : String(prefs.mouseTriggerButton);
    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Drag Trigger Key</h2>
            <p class="card-subtitle">Press while dragging files from Finder (or hold before starting a drag)</p>
          </div>
          <div>${formatKeycaps(prefs.dragTrigger)}</div>
        </div>
        <div class="recorder-box">
          <div class="recorder-pill ${recordingTarget === "drag" ? "is-recording" : ""}" id="drag-recorder-status">
            ${
              recordingTarget === "drag"
                ? "Press a single key (Esc to cancel)…"
                : `Current: ${escapeHtml(prefs.dragTrigger)}`
            }
          </div>
          <button type="button" class="btn" id="btn-record-drag">
            ${recordingTarget === "drag" ? "Cancel" : "Record Custom Key…"}
          </button>
        </div>
        <div class="preset-row">
          ${DRAG_PRESETS.map(
            (p) =>
              `<button type="button" class="btn btn-sm ${prefs.dragTrigger.toLowerCase() === p.id.toLowerCase() ? "btn-primary" : ""}" data-drag-preset="${escapeHtml(p.id)}">${escapeHtml(p.label)}</button>`,
          ).join("")}
        </div>
        ${
          recorderFeedback && recorderFeedbackTarget === "drag"
            ? `<p class="setting-desc" style="margin-top:8px;color:var(--accent)">${escapeHtml(recorderFeedback)}</p>`
            : ""
        }
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Idle Global Shortcut</h2>
            <p class="card-subtitle">Opens Convert wheel for selected Finder files, or New File wheel in active folder</p>
          </div>
          <div>${formatKeycaps(prefs.idleShortcut)}</div>
        </div>
        <div class="recorder-box">
          <div class="recorder-pill ${recordingTarget === "idle" ? "is-recording" : ""}" id="idle-recorder-status">
            ${
              recordingTarget === "idle"
                ? "Press modifier + key combo (Esc to cancel)…"
                : `Current: ${escapeHtml(prefs.idleShortcut)}`
            }
          </div>
          <button type="button" class="btn" id="btn-record-idle">
            ${recordingTarget === "idle" ? "Cancel" : "Record Shortcut…"}
          </button>
          <button type="button" class="btn btn-sm" id="btn-reset-idle">Reset Default (⌥⇧Space)</button>
        </div>
        ${
          recorderFeedback && recorderFeedbackTarget === "idle"
            ? `<p class="setting-desc" style="margin-top:8px;color:var(--accent)">${escapeHtml(recorderFeedback)}</p>`
            : ""
        }
      </section>

      <section class="glass-card">
        <div class="setting-row">
          <div>
            <div class="setting-label">Mouse Button Trigger (Hold &amp; Release)</div>
            <div class="setting-desc">Optional Logi / multi-button mouse trigger supporting hold-and-release selection.</div>
          </div>
          <select class="mac-select" id="pref-mouseTriggerButton">
            <option value="" ${mouseVal === "" ? "selected" : ""}>Disabled</option>
            <option value="2" ${mouseVal === "2" ? "selected" : ""}>Middle Click (Btn 2)</option>
            <option value="3" ${mouseVal === "3" ? "selected" : ""}>Mouse Button 4</option>
            <option value="4" ${mouseVal === "4" ? "selected" : ""}>Mouse Button 5</option>
          </select>
        </div>
      </section>
    `;
  }

  function renderFormatsTab(): string {
    const familyFormats =
      prefs.preferredFormats[selectedFamily] ??
      defaultPreferences().preferredFormats[selectedFamily] ??
      [];
    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Image Encoding &amp; Privacy</h2>
            <p class="card-subtitle">Applies to JPEG, HEIC, and WebP conversions</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Image quality</div>
            <div class="setting-desc">Higher values preserve more detail at larger file sizes.</div>
          </div>
          <div style="display:flex;align-items:center;gap:10px">
            <input type="range" class="mac-range" id="pref-imageQuality" min="10" max="100" step="1" value="${prefs.imageQuality}" />
            <span class="kbd-cap" id="imageQuality-val">${prefs.imageQuality}%</span>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Preserve EXIF metadata</div>
            <div class="setting-desc">Keep camera, color profile, and orientation tags when supported.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-preserveExif" ${prefs.preserveExif ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Strip GPS location from EXIF</div>
            <div class="setting-desc">Remove geolocation coordinates for safer sharing.</div>
          </div>
          <label class="mac-switch">
            <input type="checkbox" id="pref-stripGpsExif" ${prefs.stripGpsExif ? "checked" : ""} />
            <span class="mac-switch-track"></span>
          </label>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">PDF Page &amp; Render Defaults</h2>
            <p class="card-subtitle">Controls document-to-PDF layout and PDF-to-image resolution</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">PDF page size</div>
            <div class="setting-desc">Page geometry for document and data exports.</div>
          </div>
          <div class="segmented" id="seg-pdfPageSize">
            <button type="button" data-val="a4" class="${prefs.pdfPageSize.toLowerCase() === "a4" ? "is-active" : ""}">A4</button>
            <button type="button" data-val="letter" class="${prefs.pdfPageSize.toLowerCase() === "letter" ? "is-active" : ""}">Letter</button>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">PDF render DPI</div>
            <div class="setting-desc">Resolution when rasterizing PDF pages to PNG, JPG, HEIC, or WebP.</div>
          </div>
          <div class="segmented" id="seg-pdfDpi">
            ${[144, 200, 300]
              .map(
                (dpi) =>
                  `<button type="button" data-val="${dpi}" class="${prefs.pdfDpi === dpi ? "is-active" : ""}">${dpi} DPI</button>`,
              )
              .join("")}
          </div>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Preferred Wheel Format Order</h2>
            <p class="card-subtitle">Top 7 formats appear immediately on Ring Page 1 for each family</p>
          </div>
          <button type="button" class="btn btn-sm" id="btn-reset-family-order">Reset Family</button>
        </div>
        <div class="preset-row" style="margin-bottom:10px">
          ${FAMILY_LABELS.map(
            (f) =>
              `<button type="button" class="btn btn-sm ${selectedFamily === f.key ? "btn-primary" : ""}" data-family-tab="${f.key}">${f.label}</button>`,
          ).join("")}
        </div>
        <div class="format-chips">
          ${familyFormats
            .map(
              (fmt, idx) => `
              <div class="format-order-chip">
                <span style="opacity:0.5;font-size:10.5px">${idx + 1}.</span>
                <span>${escapeHtml(fmt)}</span>
                <button type="button" data-move-fmt="${idx}" data-dir="-1" ${idx === 0 ? "disabled" : ""} title="Move earlier">◀</button>
                <button type="button" data-move-fmt="${idx}" data-dir="1" ${idx === familyFormats.length - 1 ? "disabled" : ""} title="Move later">▶</button>
              </div>
            `,
            )
            .join("")}
        </div>
      </section>
    `;
  }

  function renderNewFileTab(): string {
    const clipBadge = clipboardInfo?.available
      ? `<span class="status-pill is-ok"><span class="status-dot"></span>Ready (.${escapeHtml(clipboardInfo.extension || "txt")})</span>`
      : `<span class="status-pill is-warn"><span class="status-dot"></span>Clipboard empty</span>`;

    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Clipboard Creation Preview</h2>
            <p class="card-subtitle">Create files directly from macOS pasteboard images, PDF, RTF, URLs, or text</p>
          </div>
          ${clipBadge}
        </div>
        <p class="setting-desc">
          ${
            clipboardInfo?.available
              ? `Detected <strong>${escapeHtml(clipboardInfo.kind)}</strong> payload. Choosing "From Clipboard" in the New File wheel will save <code>Clipboard &lt;timestamp&gt;.${escapeHtml(clipboardInfo.extension)}</code>.`
              : "Copy an image, PDF, rich text, URL, or plain text to enable instant Clipboard file creation."
          }
        </p>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Built-In File Kinds</h2>
            <p class="card-subtitle">Pin your favorites to Page 1 of the New File wheel</p>
          </div>
        </div>
        <div class="kinds-grid">
          ${newFileKinds
            .map((k) => {
              const pinned = prefs.newFileVisibleKinds.includes(k.id);
              const extText = k.extension ? `.${k.extension}` : k.category;
              return `
                <label class="kind-chip" title="${escapeHtml(k.disabledReason ?? extText)}">
                  <div>
                    <div style="font-weight:600">${escapeHtml(k.name)}</div>
                    <div style="font-size:11px;color:var(--text-muted)">
                      ${escapeHtml(extText)}${k.available ? "" : " · unavailable"}
                    </div>
                  </div>
                  <input type="checkbox" data-visible-kind="${escapeHtml(k.id)}" ${pinned ? "checked" : ""} />
                </label>
              `;
            })
            .join("")}
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">User Templates</h2>
            <p class="card-subtitle">Drop any file into <code>~/Library/Application Support/FLIP/Templates</code></p>
          </div>
          <button type="button" class="btn btn-primary btn-sm" id="btn-open-templates">
            ${UI_ICONS.folder}<span>Open Templates Folder</span>
          </button>
        </div>
        <p class="setting-desc" style="margin-bottom:10px">
          Text templates automatically expand <code>{{date}}</code>, <code>{{time}}</code>, <code>{{user}}</code>, and <code>{{folder}}</code>.
        </p>
        ${
          templates.length === 0
            ? `<div class="setting-desc">No custom templates added yet. Click "Open Templates Folder" to add your own.</div>`
            : `<div class="batch-file-list">
                ${templates
                  .map(
                    (t) => `
                  <div class="batch-file-item">
                    <div>
                      <strong>${escapeHtml(t.name)}</strong>
                      ${t.group ? `<span class="code-chip" style="margin-left:6px">${escapeHtml(t.group)}</span>` : ""}
                    </div>
                    <span class="code-chip">${escapeHtml(t.extension ? `.${t.extension}` : "file")}</span>
                  </div>
                `,
                  )
                  .join("")}
              </div>`
        }
      </section>
    `;
  }

  function renderEnginesTab(): string {
    const axOk = permissions?.accessibility ?? false;
    const autoState = permissions?.automationFinder ?? "unknown";
    const autoOk = autoState === "granted";

    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">macOS Permissions</h2>
            <p class="card-subtitle">Required for drag-key capture, global shortcuts, and Finder inline rename</p>
          </div>
          <button type="button" class="btn btn-sm" id="btn-refresh-diagnostics">${UI_ICONS.refresh}<span>Refresh</span></button>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Accessibility &amp; Input Monitoring</div>
            <div class="setting-desc">Captures drag trigger key without hijacking normal typing and sends Return for inline rename.</div>
          </div>
          <div style="display:flex;align-items:center;gap:8px">
            <span class="status-pill ${axOk ? "is-ok" : "is-warn"}">
              <span class="status-dot"></span>${axOk ? "Granted" : "Not Granted"}
            </span>
            ${
              axOk
                ? ""
                : `<button type="button" class="btn btn-primary btn-sm" id="btn-grant-ax">Grant Permission</button>`
            }
            <button type="button" class="btn btn-sm" id="btn-open-ax">Open System Settings</button>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Finder Automation</div>
            <div class="setting-desc">Reads selected files and active insertion folder directly from Finder.</div>
          </div>
          <div style="display:flex;align-items:center;gap:8px">
            <span class="status-pill ${autoOk ? "is-ok" : "is-warn"}">
              <span class="status-dot"></span>${escapeHtml(autoState)}
            </span>
            <button type="button" class="btn btn-sm" id="btn-open-automation">Open System Settings</button>
          </div>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Native Apple Frameworks &amp; iWork</h2>
            <p class="card-subtitle">Built-in hardware-accelerated local engines (no internet required)</p>
          </div>
        </div>
        <div class="kinds-grid">
          ${[
            { name: "ImageIO", ok: engines?.nativeFrameworks?.imageIO ?? true },
            { name: "PDFKit", ok: engines?.nativeFrameworks?.pdfKit ?? true },
            { name: "Vision OCR", ok: engines?.nativeFrameworks?.visionOcr ?? true },
            { name: "AVFoundation", ok: engines?.nativeFrameworks?.avFoundation ?? true },
            { name: "Swift Bridge", ok: Boolean(engines?.bridge) },
            { name: "Pages.app", ok: Boolean(engines?.pages) },
            { name: "Numbers.app", ok: Boolean(engines?.numbers) },
            { name: "Keynote.app", ok: Boolean(engines?.keynote) },
          ]
            .map(
              (item) => `
            <div class="kind-chip">
              <span style="font-weight:600">${item.name}</span>
              <span class="status-pill ${item.ok ? "is-ok" : "is-warn"}">
                <span class="status-dot"></span>${item.ok ? "Ready" : "Missing"}
              </span>
            </div>
          `,
            )
            .join("")}
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Optional Local CLI Engines</h2>
            <p class="card-subtitle">Unlocks MKV/WebM/MP3/FLAC/OGG transcoding and EPUB/FB2/RST books</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">FFmpeg ${engines?.ffmpeg ? `<span class="status-pill is-ok" style="margin-left:6px"><span class="status-dot"></span>Detected</span>` : `<span class="status-pill is-warn" style="margin-left:6px"><span class="status-dot"></span>Not Found</span>`}</div>
            <div class="setting-desc">${engines?.ffmpeg ? escapeHtml(engines.ffmpeg) : "Install via Homebrew: <code>brew install ffmpeg</code>"}</div>
          </div>
          <input type="text" class="mac-input" id="pref-ffmpegPath" placeholder="Auto-detect" value="${escapeHtml(prefs.ffmpegPath ?? "")}" />
        </div>
        <div class="setting-row">
          <div>
            <div class="setting-label">Pandoc ${engines?.pandoc ? `<span class="status-pill is-ok" style="margin-left:6px"><span class="status-dot"></span>Detected</span>` : `<span class="status-pill is-warn" style="margin-left:6px"><span class="status-dot"></span>Not Found</span>`}</div>
            <div class="setting-desc">${engines?.pandoc ? escapeHtml(engines.pandoc) : "Install via Homebrew: <code>brew install pandoc</code>"}</div>
          </div>
          <input type="text" class="mac-input" id="pref-pandocPath" placeholder="Auto-detect" value="${escapeHtml(prefs.pandocPath ?? "")}" />
        </div>
      </section>
    `;
  }

  function renderAboutTab(): string {
    return `
      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">FLIP for macOS</h2>
            <p class="card-subtitle">Version 0.1.0 · Target: macOS 27.2.1 Golden Gate · MIT License</p>
          </div>
          <span class="code-chip">The Tiny Foundry</span>
        </div>
        <p class="setting-desc">
          100% local Liquid Glass radial file converter and Create New File utility for Finder.
          Zero cloud uploads, atomic staging, non-overwriting Finder-style naming, and native Apple Vision OCR / PDFKit / AVFoundation support.
        </p>
        <div class="preset-row" style="margin-top:12px">
          <button type="button" class="btn btn-primary btn-sm" id="btn-about-wheel">Preview Wheel</button>
          <button type="button" class="btn btn-sm" id="btn-about-batch">Batch Converter</button>
          <button type="button" class="btn btn-sm" id="btn-about-tour">Welcome Tour</button>
          <button type="button" class="btn btn-sm" id="btn-about-logs">Reveal Logs</button>
        </div>
      </section>

      <section class="glass-card">
        <div class="card-header">
          <div>
            <h2 class="card-title">Recent Conversions</h2>
            <p class="card-subtitle">Quickly locate recently generated files in Finder</p>
          </div>
          <button type="button" class="btn btn-sm" id="btn-refresh-recent">${UI_ICONS.refresh}<span>Refresh</span></button>
        </div>
        ${
          recent.length === 0
            ? `<p class="setting-desc">No conversions recorded yet in this session.</p>`
            : `<div class="batch-results-list">
                ${recent
                  .map(
                    (item, idx) => `
                  <div class="batch-result-row">
                    <div>
                      <div style="font-weight:600">${escapeHtml(item.sourceName)} → ${escapeHtml(item.targetFormat)}</div>
                      <div style="font-size:11.5px;color:var(--text-muted)" title="${escapeHtml(item.outputPath)}">${escapeHtml(basename(item.outputPath))}</div>
                    </div>
                    <button type="button" class="btn btn-sm" data-reveal-recent="${idx}">${UI_ICONS.finder}<span>Show in Finder</span></button>
                  </div>
                `,
                  )
                  .join("")}
              </div>`
        }
      </section>
    `;
  }

  function bindTabHandlers(): void {
    // General
    mainEl.querySelector<HTMLInputElement>("#pref-launchAtLogin")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.launchAtLogin = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-soundEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.soundEnabled = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-hapticsEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.hapticsEnabled = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-autoRenameOnCreate")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.autoRenameOnCreate = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-openAfterCreating")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.openAfterCreating = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-outputSubfolder")?.addEventListener("change", (e) => {
      const input = e.target as HTMLInputElement;
      const raw = input.value.trim().replace(/^\/+|\/+$/g, "");
      if (raw.includes("..")) {
        input.value = prefs.outputSubfolder ?? "";
        showSavedToast("Subfolder cannot contain '..'");
        return;
      }
      const safe = raw || null;
      input.value = safe ?? "";
      void persistPrefs((d) => { d.outputSubfolder = safe; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-fallbackFolder")?.addEventListener("change", (e) => {
      const raw = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => { d.fallbackFolder = raw || null; });
    });
    mainEl.querySelector("#btn-reveal-logs")?.addEventListener("click", () => void revealLogs());

    // Shortcuts
    mainEl.querySelector("#btn-record-drag")?.addEventListener("click", () => {
      recordingTarget = recordingTarget === "drag" ? null : "drag";
      recorderFeedbackTarget = recordingTarget;
      recorderFeedback = recordingTarget ? "Press one key without modifiers." : "";
      renderMain();
    });
    mainEl.querySelector("#btn-record-idle")?.addEventListener("click", () => {
      recordingTarget = recordingTarget === "idle" ? null : "idle";
      recorderFeedbackTarget = recordingTarget;
      recorderFeedback = recordingTarget ? "Press modifier + key (e.g. Option+Shift+Space)." : "";
      renderMain();
    });
    mainEl.querySelector("#btn-reset-idle")?.addEventListener("click", () => {
      recordingTarget = null;
      recorderFeedback = "";
      recorderFeedbackTarget = null;
      void persistPrefs((d) => { d.idleShortcut = "Option+Shift+Space"; }).then(renderMain);
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-drag-preset]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const key = btn.dataset.dragPreset!;
        recordingTarget = null;
        recorderFeedback = "";
        recorderFeedbackTarget = null;
        void persistPrefs((d) => { d.dragTrigger = key; }).then(renderMain);
      });
    });
    mainEl.querySelector<HTMLSelectElement>("#pref-mouseTriggerButton")?.addEventListener("change", (e) => {
      const val = (e.target as HTMLSelectElement).value;
      void persistPrefs((d) => {
        d.mouseTriggerButton = val === "" ? null : Number(val);
      });
    });

    // Formats
    const qualRange = mainEl.querySelector<HTMLInputElement>("#pref-imageQuality");
    const qualVal = mainEl.querySelector<HTMLElement>("#imageQuality-val");
    qualRange?.addEventListener("input", () => {
      if (qualVal) qualVal.textContent = `${qualRange.value}%`;
    });
    qualRange?.addEventListener("change", () => {
      const n = Math.max(10, Math.min(100, Number(qualRange.value)));
      void persistPrefs((d) => { d.imageQuality = n; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-preserveExif")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.preserveExif = checked; });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-stripGpsExif")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => { d.stripGpsExif = checked; });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("#seg-pdfPageSize button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = btn.dataset.val!;
        void persistPrefs((d) => { d.pdfPageSize = val; }).then(renderMain);
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("#seg-pdfDpi button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = Number(btn.dataset.val!);
        void persistPrefs((d) => { d.pdfDpi = val; }).then(renderMain);
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-family-tab]").forEach((btn) => {
      btn.addEventListener("click", () => {
        selectedFamily = btn.dataset.familyTab!;
        renderMain();
      });
    });
    mainEl.querySelector("#btn-reset-family-order")?.addEventListener("click", () => {
      const def = defaultPreferences().preferredFormats[selectedFamily] ?? [];
      void persistPrefs((d) => {
        d.preferredFormats[selectedFamily] = [...def];
      }).then(renderMain);
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-move-fmt]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const idx = Number(btn.dataset.moveFmt!);
        const dir = Number(btn.dataset.dir!);
        const list = [...(prefs.preferredFormats[selectedFamily] ?? defaultPreferences().preferredFormats[selectedFamily] ?? [])];
        const target = idx + dir;
        if (target < 0 || target >= list.length) return;
        const [item] = list.splice(idx, 1);
        list.splice(target, 0, item);
        void persistPrefs((d) => {
          d.preferredFormats[selectedFamily] = list;
        }).then(renderMain);
      });
    });

    // New File & Templates
    mainEl.querySelectorAll<HTMLInputElement>("[data-visible-kind]").forEach((chk) => {
      chk.addEventListener("change", () => {
        const kindId = chk.dataset.visibleKind!;
        const current = new Set(prefs.newFileVisibleKinds);
        if (chk.checked) current.add(kindId);
        else current.delete(kindId);
        void persistPrefs((d) => {
          d.newFileVisibleKinds = Array.from(current);
        });
      });
    });
    mainEl.querySelector("#btn-open-templates")?.addEventListener("click", () => void openTemplatesFolder());

    // Engines & Permissions
    mainEl.querySelector("#btn-refresh-diagnostics")?.addEventListener("click", () => void refreshDiagnostics());
    mainEl.querySelector("#btn-grant-ax")?.addEventListener("click", async () => {
      try {
        permissions = await requestPermission("accessibility_prompt");
        if (!disposed) renderMain();
      } catch (err) {
        if (!disposed) showSavedToast(`Error: ${formatError(err)}`);
      }
    });
    mainEl.querySelector("#btn-open-ax")?.addEventListener("click", async () => {
      try {
        permissions = await requestPermission("open_accessibility");
        if (!disposed) renderMain();
      } catch (err) {
        if (!disposed) showSavedToast(`Error: ${formatError(err)}`);
      }
    });
    mainEl.querySelector("#btn-open-automation")?.addEventListener("click", async () => {
      try {
        permissions = await requestPermission("open_automation");
        if (!disposed) renderMain();
      } catch (err) {
        if (!disposed) showSavedToast(`Error: ${formatError(err)}`);
      }
    });
    mainEl.querySelector<HTMLInputElement>("#pref-ffmpegPath")?.addEventListener("change", (e) => {
      const val = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => { d.ffmpegPath = val || null; }).then(refreshDiagnostics);
    });
    mainEl.querySelector<HTMLInputElement>("#pref-pandocPath")?.addEventListener("change", (e) => {
      const val = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => { d.pandocPath = val || null; }).then(refreshDiagnostics);
    });

    // About
    mainEl.querySelector("#btn-about-wheel")?.addEventListener("click", () => void openWindow("wheel"));
    mainEl.querySelector("#btn-about-batch")?.addEventListener("click", () => void openWindow("batch"));
    mainEl.querySelector("#btn-about-tour")?.addEventListener("click", () => void openWindow("onboarding"));
    mainEl.querySelector("#btn-about-logs")?.addEventListener("click", () => void revealLogs());
    mainEl.querySelector("#btn-refresh-recent")?.addEventListener("click", async () => {
      recent = await getRecentConversions().catch(() => recent);
      if (!disposed) renderMain();
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-reveal-recent]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const item = recent[Number(btn.dataset.revealRecent!)];
        if (item) void revealInFinder([item.outputPath]);
      });
    });
  }

  const onRecorderKeyDown = (ev: KeyboardEvent) => {
    if (!recordingTarget) return;
    const targetEl = ev.target;
    if (targetEl instanceof Element && targetEl.closest("input, textarea, select")) {
      recordingTarget = null;
      recorderFeedback = "";
      recorderFeedbackTarget = null;
      return;
    }

    ev.preventDefault();
    ev.stopPropagation();

    const activeTarget = recordingTarget;

    if (ev.key === "Escape") {
      recordingTarget = null;
      recorderFeedbackTarget = activeTarget;
      recorderFeedback = "Recording cancelled.";
      renderMain();
      return;
    }

    if (ev.key === "Enter") {
      recorderFeedbackTarget = activeTarget;
      recorderFeedback = "Escape and Return are reserved keys.";
      renderMain();
      return;
    }

    // Ignore lone modifier keydowns while waiting for main key
    if (["Meta", "Alt", "Control", "Shift"].includes(ev.key)) {
      return;
    }

    const token = mapBrowserKeyToToken(ev);
    if (!token) {
      recorderFeedbackTarget = activeTarget;
      recorderFeedback = `Unsupported key: ${ev.key}`;
      renderMain();
      return;
    }

    if (activeTarget === "drag") {
      if (ev.metaKey || ev.altKey || ev.ctrlKey || ev.shiftKey) {
        recorderFeedbackTarget = "drag";
        recorderFeedback = "Press one single key without Command, Option, Control, or Shift.";
        renderMain();
        return;
      }
      recordingTarget = null;
      recorderFeedback = "";
      recorderFeedbackTarget = null;
      void persistPrefs((d) => { d.dragTrigger = token; }).then(renderMain);
      return;
    }

    // Idle combo validation (requires >=1 modifier or F1-F12)
    const isFn = /^F([1-9]|1[0-2])$/.test(token);
    if (!ev.metaKey && !ev.altKey && !ev.ctrlKey && !ev.shiftKey && !isFn) {
      recorderFeedbackTarget = "idle";
      recorderFeedback = "Idle shortcut requires at least one modifier (Command, Option, Control, Shift) or F1–F12.";
      renderMain();
      return;
    }

    const parts: string[] = [];
    if (ev.metaKey) parts.push("Command");
    if (ev.altKey) parts.push("Option");
    if (ev.ctrlKey) parts.push("Control");
    if (ev.shiftKey) parts.push("Shift");
    parts.push(token);

    recordingTarget = null;
    recorderFeedback = "";
    recorderFeedbackTarget = null;
    void persistPrefs((d) => { d.idleShortcut = parts.join("+"); }).then(renderMain);
  };

  window.addEventListener("keydown", onRecorderKeyDown, true);

  navEl.addEventListener("click", (ev) => {
    const btn = ev.target instanceof Element
      ? ev.target.closest<HTMLButtonElement>(".nav-item[data-tab]")
      : null;
    if (!btn) return;
    activeTab = btn.dataset.tab as SettingsTab;
    recordingTarget = null;
    recorderFeedback = "";
    recorderFeedbackTarget = null;
    renderMain();
    if (activeTab === "engines" || activeTab === "newfile" || activeTab === "about") {
      void refreshDiagnostics();
    }
  });

  root.querySelector("#nav-open-batch")?.addEventListener("click", () => void openWindow("batch"));
  root.querySelector("#nav-open-onboarding")?.addEventListener("click", () => void openWindow("onboarding"));

  async function refreshDiagnostics(): Promise<void> {
    const [eng, perms, kindsResp, rec] = await Promise.all([
      getEngineStatus().catch(() => null),
      getPermissionsStatus().catch(() => null),
      getNewFileKinds().catch(
        (): NewFileKindsResponse => ({ kinds: [], templates: [] }),
      ),
      getRecentConversions().catch(() => []),
    ]);
    if (disposed) return;
    if (eng) engines = eng;
    if (perms) permissions = perms;
    newFileKinds = kindsResp.kinds;
    templates = kindsResp.templates;
    clipboardInfo = kindsResp.clipboard;
    recent = rec;
    if (!recordingTarget) {
      renderMain();
    }
  }

  void (async () => {
    prefs = await getPreferences().catch(() => defaultPreferences());
    if (disposed) return;
    renderMain();
    await refreshDiagnostics();
  })();

  return () => {
    disposed = true;
    if (saveToastTimer !== null) window.clearTimeout(saveToastTimer);
    window.removeEventListener("keydown", onRecorderKeyDown, true);
  };
}

export function mapBrowserKeyToToken(
  ev: Pick<KeyboardEvent, "key" | "code">,
): string | null {
  if (ev.key === "ArrowDown" || ev.code === "ArrowDown") return "Down";
  if (ev.key === "ArrowUp" || ev.code === "ArrowUp") return "Up";
  if (ev.key === "ArrowLeft" || ev.code === "ArrowLeft") return "Left";
  if (ev.key === "ArrowRight" || ev.code === "ArrowRight") return "Right";
  if (ev.code === "Space" || ev.key === " " || ev.key === "\u00a0") return "Space";
  if (ev.key === "Tab" || ev.code === "Tab") return "Tab";
  if (/^F([1-9]|1[0-2])$/.test(ev.key)) return ev.key;
  if (/^F([1-9]|1[0-2])$/.test(ev.code)) return ev.code;
  if (/^[a-zA-Z0-9]$/.test(ev.key)) return ev.key.toUpperCase();
  const keyMatch = /^Key([A-Z])$/.exec(ev.code);
  if (keyMatch) return keyMatch[1];
  const digitMatch = /^(?:Digit|Numpad)([0-9])$/.exec(ev.code);
  if (digitMatch) return digitMatch[1];
  return null;
}
