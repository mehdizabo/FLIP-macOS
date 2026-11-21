import {
  localizeClipboardKind,
  localizeDisabledReason,
  localizeKindCategory,
  localizeNewKindName,
  normalizeLanguage,
  t,
  type TranslationKey,
} from "../i18n";
import { UI_ICONS } from "../icons";
import {
  basename,
  checkForUpdates,
  consumeSettingsNavigation,
  copyDiagnosticsToClipboard,
  copyTextWithFallback,
  defaultPreferences,
  escapeHtml,
  events,
  formatError,
  getEngineStatus,
  getNewFileKinds,
  getPermissionsStatus,
  getPreferences,
  getQuickActionsStatus,
  getRecentConversions,
  installFinderQuickActions,
  isTauriRuntime,
  normalizeHapticIntensity,
  openReleaseUrl,
  openTemplatesFolder,
  openWindow,
  playWheelFeedback,
  requestPermission,
  revealInFinder,
  revealLogs,
  savePreferences,
  uninstallFinderQuickActions,
} from "../ipc";
import type {
  AppPreferences,
  ClipboardStatusInfo,
  EngineStatus,
  NewFileKindInfo,
  NewFileKindsResponse,
  PermissionsStatus,
  QuickActionsStatus,
  RecentConversion,
  SettingsNavigatePayload,
  UpdateCheckResult,
  UserTemplateInfo,
} from "../types";
import { moveFormatToIndex, reorderFormatList } from "../wheelGeometry";

type SettingsTab =
  | "general"
  | "shortcuts"
  | "formats"
  | "newfile"
  | "engines"
  | "about";

const SETTINGS_TABS: SettingsTab[] = [
  "general",
  "shortcuts",
  "formats",
  "newfile",
  "engines",
  "about",
];

const DRAG_PRESETS: Array<{ id: string; key: TranslationKey }> = [
  { id: "Down", key: "settings.shortcuts.presetDown" },
  { id: "Space", key: "settings.shortcuts.presetSpace" },
  { id: "Tab", key: "settings.shortcuts.presetTab" },
  { id: "Fn", key: "settings.shortcuts.presetFn" },
  { id: "F8", key: "settings.shortcuts.presetF8" },
];

const FAMILY_LABELS: Array<{ key: string; labelKey: TranslationKey }> = [
  { key: "image", labelKey: "settings.formats.familyImage" },
  { key: "pdf", labelKey: "settings.formats.familyPdf" },
  { key: "document", labelKey: "settings.formats.familyDocument" },
  { key: "data", labelKey: "settings.formats.familyData" },
  { key: "audio", labelKey: "settings.formats.familyAudio" },
  { key: "video", labelKey: "settings.formats.familyVideo" },
];

function renderStatusInline(ok: boolean, label: string): string {
  return `<span class="status-inline ${ok ? "is-ok" : "is-warn"}"><span class="status-dot"></span>${escapeHtml(label)}</span>`;
}

export function mountSettingsView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "settings";
  document.documentElement.dataset.tauri = isTauriRuntime() ? "1" : "0";

  let disposed = false;
  let activeTab: SettingsTab = "general";
  let lastRenderedTab: SettingsTab | null = null;
  let selectedFamily = "image";
  let prefs: AppPreferences = defaultPreferences();
  let engines: EngineStatus | null = null;
  let permissions: PermissionsStatus | null = null;
  let quickActions: QuickActionsStatus | null = null;
  let newFileKinds: NewFileKindInfo[] = [];
  let templates: UserTemplateInfo[] = [];
  let clipboardInfo: ClipboardStatusInfo | undefined;
  let recent: RecentConversion[] = [];

  let recordingTarget: "drag" | "idle" | null = null;
  let recorderFeedbackKey: TranslationKey | null = null;
  let recorderFeedbackVars: Record<string, string | number> | undefined;
  let recorderFeedbackTarget: "drag" | "idle" | null = null;
  let saveToastTimer: number | null = null;

  let updateChecking = false;
  let updateResult: UpdateCheckResult | null = null;
  let updateError: string | null = null;
  let lastUpdateTriggerMs = 0;
  let diagCopiedState: "idle" | "copied" | "manual" = "idle";
  let diagFallbackText = "";
  let diagCopyTimer: number | null = null;

  const lang = () => normalizeLanguage(prefs.language);

  root.innerHTML = `
    <div class="settings-shell">
      <aside class="settings-sidebar" id="settings-sidebar" aria-label="${escapeHtml(t(lang(), "nav.ariaLabel"))}">
        <div class="sidebar-drag-strip" data-tauri-drag-region></div>
        <div class="brand-header" data-tauri-drag-region>
          <span class="brand-mark-svg">${UI_ICONS.flipMark}</span>
          <div>
            <div class="brand-title">FLIP</div>
            <div class="brand-sub">v0.1.0</div>
          </div>
        </div>
        <nav class="nav-list" id="settings-nav">
          <button type="button" class="nav-item is-active" data-tab="general" aria-current="page">${UI_ICONS.settings}<span data-nav-label="general">${escapeHtml(t(lang(), "nav.general"))}</span></button>
          <button type="button" class="nav-item" data-tab="shortcuts">${UI_ICONS.keyboard}<span data-nav-label="shortcuts">${escapeHtml(t(lang(), "nav.shortcuts"))}</span></button>
          <button type="button" class="nav-item" data-tab="formats">${UI_ICONS.sliders}<span data-nav-label="formats">${escapeHtml(t(lang(), "nav.formats"))}</span></button>
          <button type="button" class="nav-item" data-tab="newfile">${UI_ICONS.newFile}<span data-nav-label="newfile">${escapeHtml(t(lang(), "nav.newFile"))}</span></button>
          <button type="button" class="nav-item" data-tab="engines">${UI_ICONS.shield}<span data-nav-label="engines">${escapeHtml(t(lang(), "nav.engines"))}</span></button>
          <button type="button" class="nav-item" data-tab="about">${UI_ICONS.info}<span data-nav-label="about">${escapeHtml(t(lang(), "nav.about"))}</span></button>
        </nav>
        <div class="sidebar-footer">
          <button type="button" class="sidebar-action-btn" id="nav-open-batch">${UI_ICONS.convert}<span id="nav-batch-label">${escapeHtml(t(lang(), "nav.batchConverter"))}</span></button>
          <button type="button" class="sidebar-action-btn" id="nav-open-onboarding">${UI_ICONS.book}<span id="nav-onboarding-label">${escapeHtml(t(lang(), "nav.setupTour"))}</span></button>
        </div>
      </aside>
      <main class="settings-main" id="settings-main">
        <header class="pane-header" id="settings-pane-header" data-tauri-drag-region>
          <h1 class="page-title" id="settings-page-title"></h1>
          <span class="save-toast" id="save-toast" role="status" aria-live="polite"></span>
        </header>
        <div class="pane-scroll" id="settings-pane-scroll">
          <div class="pane-content" id="settings-pane-content"></div>
        </div>
      </main>
    </div>
  `;

  const sidebarEl = root.querySelector<HTMLElement>("#settings-sidebar")!;
  const navEl = root.querySelector<HTMLElement>("#settings-nav")!;
  const mainEl = root.querySelector<HTMLElement>("#settings-main")!;
  const paneHeaderEl = root.querySelector<HTMLElement>("#settings-pane-header")!;
  const pageTitleEl = root.querySelector<HTMLElement>("#settings-page-title")!;
  const paneScrollEl = root.querySelector<HTMLElement>("#settings-pane-scroll")!;
  const paneContentEl = root.querySelector<HTMLElement>("#settings-pane-content")!;
  const toastEl = root.querySelector<HTMLElement>("#save-toast")!;

  function syncScrollHeader(): void {
    const scrolled = Number((paneScrollEl as { scrollTop?: number }).scrollTop ?? 0) > 2;
    paneHeaderEl.classList.toggle("is-scrolled", scrolled);
  }
  paneScrollEl.addEventListener("scroll", syncScrollHeader, { passive: true });

  function updateSidebarLocalization(): void {
    const l = lang();
    sidebarEl.setAttribute("aria-label", t(l, "nav.ariaLabel"));
    const navMap: Record<SettingsTab, TranslationKey> = {
      general: "nav.general",
      shortcuts: "nav.shortcuts",
      formats: "nav.formats",
      newfile: "nav.newFile",
      engines: "nav.engines",
      about: "nav.about",
    };
    for (const tab of Object.keys(navMap) as SettingsTab[]) {
      const span = navEl.querySelector<HTMLElement>(`[data-nav-label="${tab}"]`);
      if (span) span.textContent = t(l, navMap[tab]);
    }
    const batchLabel = root.querySelector<HTMLElement>("#nav-batch-label");
    if (batchLabel) batchLabel.textContent = t(l, "nav.batchConverter");
    const onboardingLabel = root.querySelector<HTMLElement>("#nav-onboarding-label");
    if (onboardingLabel) onboardingLabel.textContent = t(l, "nav.setupTour");
  }

  function showSavedToast(label = t(lang(), "common.saved"), isError = false): void {
    toastEl.textContent = label;
    toastEl.classList.toggle("is-error", isError);
    toastEl.classList.add("is-visible");
    if (saveToastTimer !== null) window.clearTimeout(saveToastTimer);
    saveToastTimer = window.setTimeout(() => {
      toastEl.classList.remove("is-visible");
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

  let saveQueue: Promise<boolean> = Promise.resolve(true);

  function persistPrefs(mutator: (draft: AppPreferences) => void): Promise<boolean> {
    const run = saveQueue.then(async () => {
      const draft = clonePrefs(prefs);
      mutator(draft);
      try {
        prefs = await savePreferences(draft);
        if (!disposed) showSavedToast(t(lang(), "common.saved"));
        return true;
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
        return false;
      }
    });
    saveQueue = run.catch(() => false);
    return run;
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

  function renderMain(focusSelector?: string): void {
    updateSidebarLocalization();
    for (const btn of Array.from(navEl.querySelectorAll<HTMLButtonElement>(".nav-item"))) {
      const active = btn.dataset.tab === activeTab;
      btn.classList.toggle("is-active", active);
      if (active) btn.setAttribute("aria-current", "page");
      else btn.removeAttribute("aria-current");
    }

    const l = lang();
    const headerTitle: Record<SettingsTab, string> = {
      general: t(l, "settings.title.general"),
      shortcuts: t(l, "settings.title.shortcuts"),
      formats: t(l, "settings.title.formats"),
      newfile: t(l, "settings.title.newfile"),
      engines: t(l, "settings.title.engines"),
      about: t(l, "settings.title.about"),
    };

    const sameTab = lastRenderedTab === activeTab;
    const savedScrollTop = sameTab
      ? Number((paneScrollEl as { scrollTop?: number }).scrollTop ?? 0)
      : 0;
    lastRenderedTab = activeTab;

    pageTitleEl.textContent = headerTitle[activeTab];

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

    paneContentEl.innerHTML = bodyHtml;
    if ("scrollTop" in paneScrollEl) {
      (paneScrollEl as { scrollTop: number }).scrollTop = savedScrollTop;
    }
    syncScrollHeader();

    bindTabHandlers();

    if (focusSelector) {
      mainEl.querySelector<HTMLElement>(focusSelector)?.focus();
    }
  }

  function renderGeneralTab(): string {
    const l = lang();
    return `
      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.general.behaviorTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.languageLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.languageDesc"))}</div>
            </div>
            <select class="mac-select" id="pref-language" aria-label="${escapeHtml(t(l, "settings.general.languageLabel"))}">
              <option value="en" ${l === "en" ? "selected" : ""}>${escapeHtml(t(l, "settings.general.langEn"))}</option>
              <option value="it" ${l === "it" ? "selected" : ""}>${escapeHtml(t(l, "settings.general.langIt"))}</option>
            </select>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.launchLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.launchDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-launchAtLogin" ${prefs.launchAtLogin ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.soundLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.soundDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-soundEnabled" ${prefs.soundEnabled ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.hapticsLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.hapticsDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-hapticsEnabled" ${prefs.hapticsEnabled ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.hapticIntensityLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.hapticIntensityDesc"))}</div>
            </div>
            <div class="segmented" id="seg-hapticIntensity" role="group" aria-label="${escapeHtml(t(l, "settings.general.hapticIntensityLabel"))}">
              ${(["light", "medium", "firm", "strong"] as const)
                .map((level) => {
                  const active = normalizeHapticIntensity(prefs.hapticIntensity) === level;
                  const labelKey =
                    level === "light"
                      ? "settings.general.hapticLight"
                      : level === "medium"
                        ? "settings.general.hapticMedium"
                        : level === "firm"
                          ? "settings.general.hapticFirm"
                          : "settings.general.hapticStrong";
                  return `<button type="button" data-val="${level}" class="${active ? "is-active" : ""}" aria-pressed="${active ? "true" : "false"}" ${prefs.hapticsEnabled ? "" : "disabled"}>${escapeHtml(t(l, labelKey))}</button>`;
                })
                .join("")}
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.auraLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.auraDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-auraEnabled" ${prefs.auraEnabled ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.testFeedbackLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.testFeedbackDesc"))}</div>
            </div>
            <button type="button" class="btn btn-sm" id="btn-test-wheel-feedback">${escapeHtml(t(l, "settings.general.testFeedbackBtn"))}</button>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.notificationsLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.notificationsDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-notificationsEnabled" ${prefs.notificationsEnabled ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
        </div>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.general.pathsTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.autoRenameLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.autoRenameDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-autoRenameOnCreate" ${prefs.autoRenameOnCreate ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.openAfterLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.openAfterDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-openAfterCreating" ${prefs.openAfterCreating ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.subfolderLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.subfolderDesc"))}</div>
            </div>
            <input type="text" class="mac-input" id="pref-outputSubfolder" placeholder="${escapeHtml(t(l, "settings.general.subfolderPlaceholder"))}" value="${escapeHtml(prefs.outputSubfolder ?? "")}" />
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.fallbackLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.fallbackDesc"))}</div>
            </div>
            <input type="text" class="mac-input" id="pref-fallbackFolder" placeholder="~/Desktop" value="${escapeHtml(prefs.fallbackFolder ?? "")}" />
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.general.pathsSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.general.diagnosticsTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.general.logsLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.general.logsDesc"))}</div>
            </div>
            <button type="button" class="btn btn-sm" id="btn-reveal-logs">${UI_ICONS.finder}<span>${escapeHtml(t(l, "settings.general.revealLogs"))}</span></button>
          </div>
        </div>
      </section>
    `;
  }

  function renderShortcutsTab(): string {
    const l = lang();
    const mouseVal = prefs.mouseTriggerButton === null ? "" : String(prefs.mouseTriggerButton);
    const feedbackMsg = recorderFeedbackKey ? t(l, recorderFeedbackKey, recorderFeedbackVars) : "";
    return `
      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.shortcuts.dragTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.shortcuts.dragLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.shortcuts.dragSub"))}</div>
            </div>
            <div class="recorder-pill ${recordingTarget === "drag" ? "is-recording" : ""}" id="drag-recorder-status">
              ${
                recordingTarget === "drag"
                  ? escapeHtml(t(l, "settings.shortcuts.dragPrompt"))
                  : `${formatKeycaps(prefs.dragTrigger)}<span class="sr-only">${escapeHtml(t(l, "settings.shortcuts.current", { value: prefs.dragTrigger }))}</span>`
              }
            </div>
          </div>
          <div class="setting-row shortcut-actions-row">
            <div class="preset-row">
              ${DRAG_PRESETS.map(
                (p) =>
                  `<button type="button" class="btn btn-sm ${prefs.dragTrigger.toLowerCase() === p.id.toLowerCase() ? "btn-primary" : ""}" data-drag-preset="${escapeHtml(p.id)}">${escapeHtml(t(l, p.key))}</button>`,
              ).join("")}
            </div>
            <button type="button" class="btn btn-sm" id="btn-record-drag">
              ${escapeHtml(recordingTarget === "drag" ? t(l, "common.cancel") : t(l, "settings.shortcuts.recordCustom"))}
            </button>
          </div>
          ${
            feedbackMsg && recorderFeedbackTarget === "drag"
              ? `<div class="setting-row-block"><p class="shortcut-feedback">${escapeHtml(feedbackMsg)}</p></div>`
              : ""
          }
        </div>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.shortcuts.idleTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.shortcuts.idleLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.shortcuts.idleSub"))}</div>
            </div>
            <div class="recorder-pill ${recordingTarget === "idle" ? "is-recording" : ""}" id="idle-recorder-status">
              ${
                recordingTarget === "idle"
                  ? escapeHtml(t(l, "settings.shortcuts.idlePrompt"))
                  : `${formatKeycaps(prefs.idleShortcut)}<span class="sr-only">${escapeHtml(t(l, "settings.shortcuts.current", { value: prefs.idleShortcut }))}</span>`
              }
            </div>
          </div>
          <div class="setting-row shortcut-actions-row shortcut-actions-row-end">
            <button type="button" class="btn btn-sm" id="btn-record-idle">
              ${escapeHtml(recordingTarget === "idle" ? t(l, "common.cancel") : t(l, "settings.shortcuts.recordIdle"))}
            </button>
            <button type="button" class="btn btn-sm" id="btn-reset-idle">${escapeHtml(t(l, "settings.shortcuts.resetIdle"))}</button>
          </div>
          ${
            feedbackMsg && recorderFeedbackTarget === "idle"
              ? `<div class="setting-row-block"><p class="shortcut-feedback">${escapeHtml(feedbackMsg)}</p></div>`
              : ""
          }
        </div>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.shortcuts.mouseTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.shortcuts.mouseLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.shortcuts.mouseDesc"))}</div>
            </div>
            <select class="mac-select" id="pref-mouseTriggerButton" aria-label="${escapeHtml(t(l, "settings.shortcuts.mouseTitle"))}">
              <option value="" ${mouseVal === "" ? "selected" : ""}>${escapeHtml(t(l, "settings.shortcuts.mouseDisabled"))}</option>
              <option value="2" ${mouseVal === "2" ? "selected" : ""}>${escapeHtml(t(l, "settings.shortcuts.mouseBtn2"))}</option>
              <option value="3" ${mouseVal === "3" ? "selected" : ""}>${escapeHtml(t(l, "settings.shortcuts.mouseBtn3"))}</option>
              <option value="4" ${mouseVal === "4" ? "selected" : ""}>${escapeHtml(t(l, "settings.shortcuts.mouseBtn4"))}</option>
            </select>
          </div>
        </div>
      </section>
    `;
  }

  function renderFormatsTab(): string {
    const l = lang();
    const familyFormats =
      prefs.preferredFormats[selectedFamily] ??
      defaultPreferences().preferredFormats[selectedFamily] ??
      [];
    const sizeVal = prefs.pdfPageSize.toLowerCase();
    return `
      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.formats.imageTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.formats.qualityLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.formats.qualityDesc"))}</div>
            </div>
            <div class="range-control">
              <input type="range" class="mac-range" id="pref-imageQuality" min="10" max="100" step="1" value="${prefs.imageQuality}" aria-label="${escapeHtml(t(l, "settings.formats.qualityLabel"))}" />
              <span class="range-value tabular-nums" id="imageQuality-val">${prefs.imageQuality}%</span>
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.formats.exifLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.formats.exifDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-preserveExif" ${prefs.preserveExif ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.formats.gpsLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.formats.gpsDesc"))}</div>
            </div>
            <label class="mac-switch">
              <input type="checkbox" id="pref-stripGpsExif" ${prefs.stripGpsExif ? "checked" : ""} />
              <span class="mac-switch-track"></span>
            </label>
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.formats.imageSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.formats.pdfTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.formats.pdfSizeLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.formats.pdfSizeDesc"))}</div>
            </div>
            <div class="segmented" id="seg-pdfPageSize" role="group" aria-label="${escapeHtml(t(l, "settings.formats.pdfSizeLabel"))}">
              <button type="button" data-val="a4" class="${sizeVal === "a4" ? "is-active" : ""}" aria-pressed="${sizeVal === "a4" ? "true" : "false"}">A4</button>
              <button type="button" data-val="letter" class="${sizeVal === "letter" ? "is-active" : ""}" aria-pressed="${sizeVal === "letter" ? "true" : "false"}">Letter</button>
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">${escapeHtml(t(l, "settings.formats.pdfDpiLabel"))}</div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.formats.pdfDpiDesc"))}</div>
            </div>
            <div class="segmented" id="seg-pdfDpi" role="group" aria-label="${escapeHtml(t(l, "settings.formats.pdfDpiLabel"))}">
              ${[144, 200, 300]
                .map((dpi) => {
                  const active = prefs.pdfDpi === dpi;
                  return `<button type="button" data-val="${dpi}" class="${active ? "is-active" : ""}" aria-pressed="${active ? "true" : "false"}">${dpi} DPI</button>`;
                })
                .join("")}
            </div>
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.formats.pdfSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.formats.orderTitle"))}</h2>
          <button type="button" class="btn btn-sm" id="btn-reset-family-order" aria-label="${escapeHtml(t(l, "settings.formats.resetOrderAria"))}">${escapeHtml(t(l, "settings.formats.resetOrder"))}</button>
        </div>
        <div class="settings-group settings-group-pad">
          <div class="family-tabs" role="tablist" aria-label="${escapeHtml(t(l, "settings.formats.familyTablistAria"))}">
            ${FAMILY_LABELS.map(
              (f) =>
                `<button type="button" role="tab" aria-selected="${selectedFamily === f.key ? "true" : "false"}" class="btn btn-sm family-tab-btn ${selectedFamily === f.key ? "is-selected" : ""}" data-family-tab="${f.key}">${escapeHtml(t(l, f.labelKey))}</button>`,
            ).join("")}
          </div>
          <div class="format-chips" role="list" aria-label="${escapeHtml(t(l, FAMILY_LABELS.find((f) => f.key === selectedFamily)?.labelKey ?? "settings.formats.familyImage"))}">
            ${familyFormats
              .map(
                (fmt, idx) => `
                <div class="format-order-chip" role="listitem" draggable="true" data-chip-index="${idx}" data-format-tag="${escapeHtml(fmt)}">
                  <span class="format-chip-index">${idx + 1}.</span>
                  <span class="format-chip-name">${escapeHtml(fmt)}</span>
                  <button type="button" data-move-fmt="${idx}" data-dir="-1" ${idx === 0 ? "disabled" : ""} aria-label="${escapeHtml(t(l, "settings.formats.moveEarlierAria", { fmt, pos: idx + 1, total: familyFormats.length }))}" title="${escapeHtml(t(l, "settings.formats.moveEarlierTitle"))}">${UI_ICONS.chevronLeft}</button>
                  <button type="button" data-move-fmt="${idx}" data-dir="1" ${idx === familyFormats.length - 1 ? "disabled" : ""} aria-label="${escapeHtml(t(l, "settings.formats.moveLaterAria", { fmt, pos: idx + 1, total: familyFormats.length }))}" title="${escapeHtml(t(l, "settings.formats.moveLaterTitle"))}">${UI_ICONS.chevronRight}</button>
                </div>
              `,
              )
              .join("")}
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.formats.orderSub"))}</p>
      </section>
    `;
  }

  function renderNewFileTab(): string {
    const l = lang();
    const clipBadge = clipboardInfo?.available
      ? renderStatusInline(true, t(l, "settings.newfile.clipReady", { ext: clipboardInfo.extension || "txt" }))
      : renderStatusInline(false, t(l, "settings.newfile.clipEmpty"));

    return `
      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.newfile.clipTitle"))}</h2>
          ${clipBadge}
        </div>
        <div class="settings-group settings-group-pad">
          <p class="setting-desc">
            ${
              clipboardInfo?.available
                ? t(l, "settings.newfile.clipDetected", {
                    kind: escapeHtml(localizeClipboardKind(l, clipboardInfo.kind)),
                    ext: escapeHtml(clipboardInfo.extension),
                  })
                : escapeHtml(t(l, "settings.newfile.clipEmptyDesc"))
            }
          </p>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.newfile.clipSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.newfile.kindsTitle"))}</h2>
        </div>
        <div class="settings-group settings-group-pad">
          <div class="kinds-grid">
            ${newFileKinds
              .map((k) => {
                const pinned = prefs.newFileVisibleKinds.includes(k.id);
                const extText = k.extension
                  ? `.${k.extension}`
                  : localizeKindCategory(l, k.category);
                const kindName = localizeNewKindName(l, k.id, k.name);
                const reasonText = localizeDisabledReason(l, k.disabledReason);
                return `
                  <label class="kind-chip" title="${escapeHtml(reasonText ?? extText)}">
                    <div>
                      <div class="kind-chip-title">${escapeHtml(kindName)}</div>
                      <div class="kind-chip-meta">
                        ${escapeHtml(extText)}${k.available ? "" : escapeHtml(t(l, "settings.newfile.unavailableTag"))}
                      </div>
                    </div>
                    <input type="checkbox" class="mac-checkbox" data-visible-kind="${escapeHtml(k.id)}" ${pinned ? "checked" : ""} />
                  </label>
                `;
              })
              .join("")}
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.newfile.kindsSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.newfile.templatesTitle"))}</h2>
          <button type="button" class="btn btn-primary btn-sm" id="btn-open-templates">
            ${UI_ICONS.folder}<span>${escapeHtml(t(l, "settings.newfile.openTemplates"))}</span>
          </button>
        </div>
        <div class="settings-group">
          ${
            templates.length === 0
              ? `<div class="setting-row-block"><div class="setting-desc">${escapeHtml(t(l, "settings.newfile.templatesEmpty"))}</div></div>`
              : `<div class="template-list">
                  ${templates
                    .map(
                      (tmpl) => `
                    <div class="template-row">
                      <div class="template-meta">
                        <span class="template-name">${escapeHtml(tmpl.name)}</span>
                        ${tmpl.group ? `<span class="code-chip">${escapeHtml(tmpl.group)}</span>` : ""}
                      </div>
                      <span class="code-chip">${escapeHtml(tmpl.extension ? `.${tmpl.extension}` : "file")}</span>
                    </div>
                  `,
                    )
                    .join("")}
                </div>`
          }
        </div>
        <p class="settings-group-footer">${t(l, "settings.newfile.templatesSub")} · ${t(l, "settings.newfile.templatesHint")}</p>
      </section>
    `;
  }

  function renderEnginesTab(): string {
    const l = lang();
    const axOk = permissions?.accessibility ?? false;
    const autoState = permissions?.automationFinder ?? "unknown";
    const autoOk = autoState === "granted";
    const autoLabel =
      autoState === "granted"
        ? t(l, "settings.engines.granted")
        : autoState === "not_determined"
          ? t(l, "settings.engines.notDetermined")
          : autoState === "denied"
            ? t(l, "settings.engines.denied")
            : autoState === "unknown"
              ? t(l, "settings.engines.unknown")
              : autoState;
    const qaConvert = quickActions?.convertInstalled ?? false;
    const qaNew = quickActions?.newFileInstalled ?? false;
    const qaAllInstalled = qaConvert && qaNew;
    const qaAnyInstalled = qaConvert || qaNew;
    const qaLabel = qaAllInstalled
      ? t(l, "settings.engines.qaInstalled")
      : qaAnyInstalled
        ? t(l, "settings.engines.qaPartial")
        : t(l, "settings.engines.qaNotInstalled");
    const cliLinked = Boolean(quickActions?.cliSymlinkPath);
    const cliExe = quickActions?.cliExecutable ?? "/Applications/FLIP.app/Contents/MacOS/flip";
    const whisperBin = engines?.whisper ?? null;
    const whisperModel = engines?.whisperModel ?? null;
    const whisperReady = whisperBin !== null && whisperModel !== null;
    const whisperDesc =
      whisperBin !== null && whisperModel !== null
        ? escapeHtml(whisperBin)
        : whisperBin !== null
          ? t(l, "settings.engines.whisperModelMissing")
          : whisperModel !== null
            ? t(l, "settings.engines.whisperBinaryMissing")
            : t(l, "settings.engines.installWhisper");
    const ghostscriptBin = engines?.ghostscript ?? null;
    const ghostscriptDesc =
      ghostscriptBin !== null
        ? escapeHtml(ghostscriptBin)
        : t(l, "settings.engines.installGhostscript");

    return `
      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.engines.permsTitle"))}</h2>
          <button type="button" class="btn btn-sm" id="btn-refresh-diagnostics">${UI_ICONS.refresh}<span>${escapeHtml(t(l, "common.refresh"))}</span></button>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${escapeHtml(t(l, "settings.engines.axLabel"))}</span>
                ${renderStatusInline(axOk, axOk ? t(l, "settings.engines.granted") : t(l, "settings.engines.notGranted"))}
              </div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.engines.axDesc"))}</div>
            </div>
            <div class="setting-row-actions">
              ${
                axOk
                  ? ""
                  : `<button type="button" class="btn btn-primary btn-sm" id="btn-grant-ax">${escapeHtml(t(l, "settings.engines.grantBtn"))}</button>`
              }
              <button type="button" class="btn btn-sm" id="btn-open-ax">${escapeHtml(t(l, "settings.engines.openSystemSettings"))}</button>
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${escapeHtml(t(l, "settings.engines.autoLabel"))}</span>
                ${renderStatusInline(autoOk, autoLabel)}
              </div>
              <div class="setting-desc">${escapeHtml(t(l, "settings.engines.autoDesc"))}</div>
            </div>
            <div class="setting-row-actions">
              <button type="button" class="btn btn-sm" id="btn-open-automation">${escapeHtml(t(l, "settings.engines.openSystemSettings"))}</button>
            </div>
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.engines.permsSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.engines.nativeTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="native-frameworks-grid">
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
              <div class="native-framework-cell">
                <span class="native-framework-name">${item.name}</span>
                ${renderStatusInline(item.ok, item.ok ? t(l, "settings.engines.ready") : t(l, "settings.engines.missing"))}
              </div>
            `,
              )
              .join("")}
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.engines.nativeSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.engines.cliTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>FFmpeg</span>
                ${renderStatusInline(Boolean(engines?.ffmpeg), engines?.ffmpeg ? t(l, "settings.engines.detected") : t(l, "settings.engines.notFound"))}
              </div>
              <div class="setting-desc">${engines?.ffmpeg ? escapeHtml(engines.ffmpeg) : t(l, "settings.engines.installFfmpeg")}</div>
            </div>
            <input type="text" class="mac-input" id="pref-ffmpegPath" placeholder="${escapeHtml(t(l, "settings.engines.autoDetect"))}" value="${escapeHtml(prefs.ffmpegPath ?? "")}" />
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>Pandoc</span>
                ${renderStatusInline(Boolean(engines?.pandoc), engines?.pandoc ? t(l, "settings.engines.detected") : t(l, "settings.engines.notFound"))}
              </div>
              <div class="setting-desc">${engines?.pandoc ? escapeHtml(engines.pandoc) : t(l, "settings.engines.installPandoc")}</div>
            </div>
            <input type="text" class="mac-input" id="pref-pandocPath" placeholder="${escapeHtml(t(l, "settings.engines.autoDetect"))}" value="${escapeHtml(prefs.pandocPath ?? "")}" />
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${escapeHtml(t(l, "settings.engines.whisperLabel"))}</span>
                ${renderStatusInline(whisperReady, whisperReady ? t(l, "settings.engines.detected") : t(l, "settings.engines.notFound"))}
              </div>
              <div class="setting-desc">${whisperDesc}</div>
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${escapeHtml(t(l, "settings.engines.ghostscriptLabel"))}</span>
                ${renderStatusInline(ghostscriptBin !== null, ghostscriptBin !== null ? t(l, "settings.engines.detected") : t(l, "settings.engines.notFound"))}
              </div>
              <div class="setting-desc">${ghostscriptDesc}</div>
            </div>
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.engines.cliSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.engines.qaTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${escapeHtml(t(l, "settings.engines.qaLabel"))}</span>
                ${renderStatusInline(qaAllInstalled, qaLabel)}
              </div>
              <div class="setting-desc">${t(l, "settings.engines.qaDesc")}</div>
            </div>
            <div class="setting-row-actions">
              <button type="button" class="btn btn-primary btn-sm" id="btn-install-quick-actions">
                ${escapeHtml(qaAllInstalled ? t(l, "settings.engines.qaReinstallBtn") : t(l, "settings.engines.qaInstallBtn"))}
              </button>
              ${
                qaAnyInstalled
                  ? `<button type="button" class="btn btn-sm" id="btn-uninstall-quick-actions">${escapeHtml(t(l, "settings.engines.qaRemoveBtn"))}</button>`
                  : ""
              }
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-row-copy">
              <div class="setting-label">
                <span>${t(l, "settings.engines.cliSchemeLabel")}</span>
                ${renderStatusInline(cliLinked, cliLinked ? t(l, "settings.engines.linked") : t(l, "settings.engines.notLinked"))}
              </div>
              <div class="setting-desc">
                ${
                  cliLinked
                    ? t(l, "settings.engines.cliLinkedDesc", {
                        path: escapeHtml(quickActions?.cliSymlinkPath ?? ""),
                      })
                    : t(l, "settings.engines.cliUnlinkedDesc", { exe: escapeHtml(cliExe) })
                }
              </div>
            </div>
            <div class="setting-row-actions">
              <button type="button" class="btn btn-sm" id="btn-install-cli-symlink">
                ${escapeHtml(cliLinked ? t(l, "settings.engines.relinkCli") : t(l, "settings.engines.installCli"))}
              </button>
            </div>
          </div>
        </div>
        <p class="settings-group-footer">${t(l, "settings.engines.qaSub")}</p>
      </section>
    `;
  }

  function renderAboutTab(): string {
    const l = lang();
    let updateBodyHtml = `
      <div class="setting-desc">
        ${t(l, "settings.about.updatesHint")}
      </div>
    `;

    if (updateChecking) {
      updateBodyHtml = `
        <div class="update-status-inline">
          ${renderStatusInline(false, t(l, "settings.about.checking"))}
          <span class="setting-desc">${escapeHtml(t(l, "settings.about.checkingDesc"))}</span>
        </div>
      `;
    } else if (updateError) {
      updateBodyHtml = `
        <div class="update-status-inline">
          ${renderStatusInline(false, t(l, "settings.about.checkFailed"))}
          <span class="setting-desc">${escapeHtml(updateError)}</span>
        </div>
      `;
    } else if (updateResult) {
      const pubDate = updateResult.publishedAt
        ? ` · ${escapeHtml(updateResult.publishedAt.slice(0, 10))}`
        : "";
      if (updateResult.updateAvailable) {
        updateBodyHtml = `
          <div class="update-panel">
            <div class="update-status-inline">
              ${renderStatusInline(true, t(l, "settings.about.updateAvailable", { tag: updateResult.latestTag }))}
              <span class="setting-desc tabular-nums">${escapeHtml(t(l, "settings.about.installedVersion", { version: updateResult.currentVersion, pubDate }))}</span>
            </div>
            ${
              updateResult.body
                ? `<pre class="diag-pre">${escapeHtml(updateResult.body)}</pre>`
                : ""
            }
            <div class="update-actions">
              <button type="button" class="btn btn-primary btn-sm" id="btn-open-release-url">${escapeHtml(t(l, "settings.about.openRelease"))}</button>
              <button type="button" class="btn btn-sm" id="btn-copy-release-url">${escapeHtml(t(l, "settings.about.copyReleaseUrl"))}</button>
            </div>
          </div>
        `;
      } else {
        updateBodyHtml = `
          <div class="update-status-row">
            <div class="update-status-inline">
              ${renderStatusInline(true, t(l, "settings.about.upToDate"))}
              <span class="setting-desc tabular-nums">${t(l, "settings.about.upToDateDesc", { version: escapeHtml(updateResult.currentVersion), tag: escapeHtml(updateResult.latestTag), pubDate })}</span>
            </div>
            <button type="button" class="btn btn-sm" id="btn-open-release-url">${escapeHtml(t(l, "settings.about.releasesPage"))}</button>
          </div>
        `;
      }
    }

    const diagFallbackHtml =
      diagCopiedState === "manual" && diagFallbackText
        ? `<div class="diag-fallback">
             <div class="setting-desc diag-fallback-label">${escapeHtml(t(l, "settings.about.diagFallbackDesc"))}</div>
             <textarea class="mac-input diag-textarea" id="about-diag-fallback" readonly rows="7">${escapeHtml(diagFallbackText)}</textarea>
           </div>`
        : "";

    return `
      <section class="settings-section">
        <div class="settings-group">
          <div class="about-identity">
            <div class="about-brand-lockup">
              <span class="brand-mark-svg">${UI_ICONS.flipMark}</span>
              <div>
                <h2 class="about-brand-title">${escapeHtml(t(l, "settings.about.appTitle"))}</h2>
                <p class="about-brand-sub">${escapeHtml(t(l, "settings.about.appSub"))}</p>
              </div>
            </div>
            <span class="about-studio-name">The Tiny Foundry</span>
          </div>
          <div class="about-desc-row">
            <p class="about-desc">${escapeHtml(t(l, "settings.about.appDesc"))}</p>
            <div class="preset-row">
              <button type="button" class="btn btn-primary btn-sm" id="btn-about-wheel">${escapeHtml(t(l, "settings.about.previewWheel"))}</button>
              <button type="button" class="btn btn-sm" id="btn-about-batch">${escapeHtml(t(l, "nav.batchConverter"))}</button>
              <button type="button" class="btn btn-sm" id="btn-about-tour">${escapeHtml(t(l, "settings.about.welcomeTour"))}</button>
              <button type="button" class="btn btn-sm" id="btn-about-logs">${escapeHtml(t(l, "settings.general.revealLogs"))}</button>
            </div>
          </div>
        </div>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.about.updatesTitle"))}</h2>
        </div>
        <div class="settings-group">
          <div class="setting-row-block">
            <div id="update-status-panel">${updateBodyHtml}</div>
            ${diagFallbackHtml}
          </div>
          <div class="setting-row shortcut-actions-row shortcut-actions-row-end">
            <button type="button" class="btn btn-sm" id="btn-check-updates" ${updateChecking ? "disabled" : ""}>
              ${escapeHtml(updateChecking ? t(l, "settings.about.checking") : t(l, "settings.about.checkUpdates"))}
            </button>
            <button type="button" class="btn btn-sm" id="btn-copy-diagnostics">
              ${escapeHtml(diagCopiedState === "copied" ? t(l, "common.copied") : t(l, "common.copyDiagnostics"))}
            </button>
          </div>
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.about.updatesSub"))}</p>
      </section>

      <section class="settings-section">
        <div class="settings-section-header">
          <h2 class="settings-section-title">${escapeHtml(t(l, "settings.about.recentTitle"))}</h2>
          <button type="button" class="btn btn-sm" id="btn-refresh-recent">${UI_ICONS.refresh}<span>${escapeHtml(t(l, "common.refresh"))}</span></button>
        </div>
        <div class="settings-group">
          ${
            recent.length === 0
              ? `<div class="setting-row-block"><p class="setting-desc">${escapeHtml(t(l, "settings.about.recentEmpty"))}</p></div>`
              : `<div class="batch-results-list">
                  ${recent
                    .map(
                      (item, idx) => `
                    <div class="batch-result-row">
                      <div class="batch-result-info">
                        <div class="batch-result-title">${escapeHtml(item.sourceName)} → ${escapeHtml(item.targetFormat)}</div>
                        <div class="setting-desc" title="${escapeHtml(item.outputPath)}">${escapeHtml(basename(item.outputPath))}</div>
                      </div>
                      <button type="button" class="btn btn-sm" data-reveal-recent="${idx}">${UI_ICONS.finder}<span>${escapeHtml(t(l, "common.showInFinder"))}</span></button>
                    </div>
                  `,
                    )
                    .join("")}
                </div>`
          }
        </div>
        <p class="settings-group-footer">${escapeHtml(t(l, "settings.about.recentSub"))}</p>
      </section>
    `;
  }

  function syncRangeTrack(rangeEl: HTMLInputElement): void {
    const min = Number(rangeEl.min || "0");
    const max = Number(rangeEl.max || "100");
    const val = Number(rangeEl.value || "0");
    const pct = max > min ? ((val - min) / (max - min)) * 100 : 0;
    rangeEl.style.setProperty("--range-pct", `${pct}%`);
  }

  function bindTabHandlers(): void {
    // General
    mainEl.querySelector<HTMLSelectElement>("#pref-language")?.addEventListener("change", (e) => {
      const nextLang = normalizeLanguage((e.target as HTMLSelectElement).value);
      void persistPrefs((d) => {
        d.language = nextLang;
      }).then((ok) => {
        if (ok && !disposed) {
          renderMain("#pref-language");
          showSavedToast(t(lang(), "common.saved"));
        }
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-launchAtLogin")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.launchAtLogin = checked;
      }).then((ok) => {
        if (!ok && !disposed) renderMain("#pref-launchAtLogin");
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-soundEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.soundEnabled = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-hapticsEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      mainEl.querySelectorAll<HTMLButtonElement>("#seg-hapticIntensity button").forEach((btn) => {
        btn.disabled = !checked;
      });
      void persistPrefs((d) => {
        d.hapticsEnabled = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-auraEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.auraEnabled = checked;
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("#seg-hapticIntensity button[data-val]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = normalizeHapticIntensity(btn.dataset.val);
        mainEl.querySelectorAll<HTMLButtonElement>("#seg-hapticIntensity button").forEach((b) => {
          const isAct = b === btn;
          b.classList.toggle("is-active", isAct);
          b.setAttribute("aria-pressed", isAct ? "true" : "false");
        });
        void persistPrefs((d) => {
          d.hapticIntensity = val;
        }).then((ok) => {
          if (ok && !disposed && prefs.hapticsEnabled) {
            void playWheelFeedback("alignment").catch(() => undefined);
          }
        });
      });
    });
    mainEl.querySelector<HTMLButtonElement>("#btn-test-wheel-feedback")?.addEventListener("click", async () => {
      await saveQueue;
      if (disposed) return;
      await playWheelFeedback("tick").catch(() => undefined);
      if (disposed) return;
      showSavedToast(
        prefs.soundEnabled || prefs.hapticsEnabled
          ? t(lang(), "settings.general.testFeedbackPlayed")
          : t(lang(), "settings.general.testFeedbackMuted"),
      );
    });
    mainEl.querySelector<HTMLInputElement>("#pref-notificationsEnabled")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.notificationsEnabled = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-autoRenameOnCreate")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.autoRenameOnCreate = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-openAfterCreating")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.openAfterCreating = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-outputSubfolder")?.addEventListener("change", (e) => {
      const input = e.target as HTMLInputElement;
      const raw = input.value.trim().replace(/^\/+|\/+$/g, "");
      if (raw.includes("..")) {
        input.value = prefs.outputSubfolder ?? "";
        showSavedToast(t(lang(), "settings.general.subfolderInvalid"), true);
        return;
      }
      const safe = raw || null;
      input.value = safe ?? "";
      void persistPrefs((d) => {
        d.outputSubfolder = safe;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-fallbackFolder")?.addEventListener("change", (e) => {
      const raw = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => {
        d.fallbackFolder = raw || null;
      });
    });
    mainEl.querySelector("#btn-reveal-logs")?.addEventListener("click", () => void revealLogs());

    // Shortcuts
    mainEl.querySelector("#btn-record-drag")?.addEventListener("click", () => {
      recordingTarget = recordingTarget === "drag" ? null : "drag";
      recorderFeedbackTarget = recordingTarget;
      recorderFeedbackKey = recordingTarget ? "settings.shortcuts.hintDrag" : null;
      recorderFeedbackVars = undefined;
      renderMain("#btn-record-drag");
    });
    mainEl.querySelector("#btn-record-idle")?.addEventListener("click", () => {
      recordingTarget = recordingTarget === "idle" ? null : "idle";
      recorderFeedbackTarget = recordingTarget;
      recorderFeedbackKey = recordingTarget ? "settings.shortcuts.hintIdle" : null;
      recorderFeedbackVars = undefined;
      renderMain("#btn-record-idle");
    });
    mainEl.querySelector("#btn-reset-idle")?.addEventListener("click", () => {
      recordingTarget = null;
      recorderFeedbackKey = null;
      recorderFeedbackVars = undefined;
      recorderFeedbackTarget = null;
      void persistPrefs((d) => {
        d.idleShortcut = "Option+Shift+Space";
      }).then(() => {
        if (!disposed) renderMain("#btn-reset-idle");
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-drag-preset]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const key = btn.dataset.dragPreset!;
        recordingTarget = null;
        recorderFeedbackKey = null;
        recorderFeedbackVars = undefined;
        recorderFeedbackTarget = null;
        void persistPrefs((d) => {
          d.dragTrigger = key;
        }).then(() => {
          if (!disposed) renderMain(`[data-drag-preset="${key}"]`);
        });
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
    if (qualRange) {
      syncRangeTrack(qualRange);
      qualRange.addEventListener("input", () => {
        if (qualVal) qualVal.textContent = `${qualRange.value}%`;
        syncRangeTrack(qualRange);
      });
      qualRange.addEventListener("change", () => {
        const n = Math.max(10, Math.min(100, Number(qualRange.value)));
        void persistPrefs((d) => {
          d.imageQuality = n;
        });
      });
    }
    mainEl.querySelector<HTMLInputElement>("#pref-preserveExif")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.preserveExif = checked;
      });
    });
    mainEl.querySelector<HTMLInputElement>("#pref-stripGpsExif")?.addEventListener("change", (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      void persistPrefs((d) => {
        d.stripGpsExif = checked;
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("#seg-pdfPageSize button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = btn.dataset.val!;
        void persistPrefs((d) => {
          d.pdfPageSize = val;
        }).then(() => {
          if (!disposed) renderMain(`#seg-pdfPageSize button[data-val="${val}"]`);
        });
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("#seg-pdfDpi button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = Number(btn.dataset.val!);
        void persistPrefs((d) => {
          d.pdfDpi = val;
        }).then(() => {
          if (!disposed) renderMain(`#seg-pdfDpi button[data-val="${val}"]`);
        });
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-family-tab]").forEach((btn) => {
      btn.addEventListener("click", () => {
        selectedFamily = btn.dataset.familyTab!;
        renderMain(`[data-family-tab="${selectedFamily}"]`);
      });
    });
    mainEl.querySelector("#btn-reset-family-order")?.addEventListener("click", () => {
      const family = selectedFamily;
      const def = defaultPreferences().preferredFormats[family] ?? [];
      void persistPrefs((d) => {
        d.preferredFormats[family] = [...def];
      }).then(() => {
        if (disposed) return;
        renderMain("#btn-reset-family-order");
      });
    });
    mainEl.querySelectorAll<HTMLButtonElement>("[data-move-fmt]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const family = selectedFamily;
        const idx = Number(btn.dataset.moveFmt!);
        const dir = Number(btn.dataset.dir!) as -1 | 1;
        const defaultsForFamily = defaultPreferences().preferredFormats[family] ?? [];
        const previewList = prefs.preferredFormats[family] ?? defaultsForFamily;
        const target = idx + dir;
        if (target < 0 || target >= previewList.length) return;
        void persistPrefs((d) => {
          const base = d.preferredFormats[family] ?? defaultsForFamily;
          d.preferredFormats[family] = reorderFormatList(base, idx, dir);
        }).then(() => {
          if (disposed) return;
          renderMain();
          if (selectedFamily !== family) return;
          const selector = `[data-move-fmt="${target}"][data-dir="${dir}"]:not(:disabled)`;
          const movedBtn = mainEl.querySelector<HTMLButtonElement>(selector);
          if (movedBtn) {
            movedBtn.focus();
          } else {
            mainEl
              .querySelector<HTMLButtonElement>(`[data-move-fmt="${target}"]:not(:disabled)`)
              ?.focus();
          }
        });
      });
    });

    let dragSourceIndex: number | null = null;
    mainEl.querySelectorAll<HTMLElement>(".format-order-chip[data-chip-index]").forEach((chip) => {
      const chipIdx = Number(chip.dataset.chipIndex!);
      chip.addEventListener("dragstart", (ev) => {
        dragSourceIndex = chipIdx;
        chip.classList.add("is-dragging");
        if (ev.dataTransfer) {
          ev.dataTransfer.effectAllowed = "move";
          ev.dataTransfer.setData("text/plain", String(chipIdx));
        }
      });
      chip.addEventListener("dragover", (ev) => {
        if (dragSourceIndex === null || dragSourceIndex === chipIdx) return;
        ev.preventDefault();
        if (ev.dataTransfer) ev.dataTransfer.dropEffect = "move";
        chip.classList.add("is-drag-over");
      });
      chip.addEventListener("dragleave", () => {
        chip.classList.remove("is-drag-over");
      });
      chip.addEventListener("drop", (ev) => {
        ev.preventDefault();
        chip.classList.remove("is-drag-over");
        const family = selectedFamily;
        const fromIdx =
          dragSourceIndex !== null
            ? dragSourceIndex
            : Number(ev.dataTransfer?.getData("text/plain") ?? "-1");
        dragSourceIndex = null;
        const defaultsForFamily = defaultPreferences().preferredFormats[family] ?? [];
        const previewList = prefs.preferredFormats[family] ?? defaultsForFamily;
        if (fromIdx < 0 || fromIdx >= previewList.length || fromIdx === chipIdx) return;
        void persistPrefs((d) => {
          const base = d.preferredFormats[family] ?? defaultsForFamily;
          d.preferredFormats[family] = moveFormatToIndex(base, fromIdx, chipIdx);
        }).then(() => {
          if (!disposed) renderMain();
        });
      });
      chip.addEventListener("dragend", () => {
        dragSourceIndex = null;
        mainEl.querySelectorAll(".format-order-chip").forEach((el) => {
          el.classList.remove("is-dragging", "is-drag-over");
        });
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
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector("#btn-open-ax")?.addEventListener("click", async () => {
      try {
        permissions = await requestPermission("open_accessibility");
        if (!disposed) renderMain();
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector("#btn-open-automation")?.addEventListener("click", async () => {
      try {
        permissions = await requestPermission("open_automation");
        if (!disposed) renderMain();
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector<HTMLInputElement>("#pref-ffmpegPath")?.addEventListener("change", (e) => {
      const val = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => {
        d.ffmpegPath = val || null;
      }).then(refreshDiagnostics);
    });
    mainEl.querySelector<HTMLInputElement>("#pref-pandocPath")?.addEventListener("change", (e) => {
      const val = (e.target as HTMLInputElement).value.trim();
      void persistPrefs((d) => {
        d.pandocPath = val || null;
      }).then(refreshDiagnostics);
    });
    mainEl.querySelector("#btn-install-quick-actions")?.addEventListener("click", async () => {
      try {
        quickActions = await installFinderQuickActions(false);
        if (!disposed) {
          renderMain();
          showSavedToast(t(lang(), "settings.engines.qaSavedToast"));
        }
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector("#btn-uninstall-quick-actions")?.addEventListener("click", async () => {
      try {
        quickActions = await uninstallFinderQuickActions(false);
        if (!disposed) {
          renderMain();
          showSavedToast(t(lang(), "settings.engines.qaRemovedToast"));
        }
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector("#btn-install-cli-symlink")?.addEventListener("click", async () => {
      try {
        quickActions = await installFinderQuickActions(true);
        if (!disposed) {
          renderMain();
          showSavedToast(t(lang(), "settings.engines.cliSavedToast"));
        }
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });

    // About
    mainEl.querySelector("#btn-about-wheel")?.addEventListener("click", () => void openWindow("wheel"));
    mainEl.querySelector("#btn-about-batch")?.addEventListener("click", () => void openWindow("batch"));
    mainEl.querySelector("#btn-about-tour")?.addEventListener("click", () => void openWindow("onboarding"));
    mainEl.querySelector("#btn-about-logs")?.addEventListener("click", () => void revealLogs());
    mainEl.querySelector("#btn-check-updates")?.addEventListener("click", () => void runUpdateCheck());
    mainEl.querySelector("#btn-copy-diagnostics")?.addEventListener("click", () => void runCopyDiagnostics());
    mainEl.querySelector("#btn-open-release-url")?.addEventListener("click", async () => {
      if (!updateResult?.htmlUrl) return;
      try {
        await openReleaseUrl(updateResult.htmlUrl);
      } catch (err) {
        if (!disposed) {
          showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
        }
      }
    });
    mainEl.querySelector("#btn-copy-release-url")?.addEventListener("click", async () => {
      if (!updateResult?.htmlUrl) return;
      const copyRes = await copyTextWithFallback(updateResult.htmlUrl);
      if (!disposed) {
        showSavedToast(
          copyRes.ok
            ? t(lang(), "settings.about.releaseUrlCopied")
            : t(lang(), "common.copyFailed"),
          !copyRes.ok,
        );
      }
    });
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

  function selectTab(nextTab: SettingsTab): void {
    activeTab = nextTab;
    recordingTarget = null;
    recorderFeedbackKey = null;
    recorderFeedbackVars = undefined;
    recorderFeedbackTarget = null;
    renderMain();
    if (activeTab === "engines" || activeTab === "newfile" || activeTab === "about") {
      void refreshDiagnostics();
    }
  }

  const onWindowKeyDown = (ev: KeyboardEvent) => {
    if (recordingTarget) {
      const targetEl = ev.target;
      if (targetEl instanceof Element && targetEl.closest("input, textarea, select")) {
        recordingTarget = null;
        recorderFeedbackKey = null;
        recorderFeedbackVars = undefined;
        recorderFeedbackTarget = null;
        return;
      }

      ev.preventDefault();
      ev.stopPropagation();

      const activeTarget = recordingTarget;

      if (ev.key === "Escape") {
        recordingTarget = null;
        recorderFeedbackTarget = activeTarget;
        recorderFeedbackKey = "settings.shortcuts.recCancelled";
        recorderFeedbackVars = undefined;
        renderMain();
        return;
      }

      if (ev.key === "Enter") {
        recorderFeedbackTarget = activeTarget;
        recorderFeedbackKey = "settings.shortcuts.recReserved";
        recorderFeedbackVars = undefined;
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
        recorderFeedbackKey = "settings.shortcuts.recUnsupported";
        recorderFeedbackVars = { key: ev.key };
        renderMain();
        return;
      }

      if (activeTarget === "drag") {
        if (ev.metaKey || ev.altKey || ev.ctrlKey || ev.shiftKey) {
          recorderFeedbackTarget = "drag";
          recorderFeedbackKey = "settings.shortcuts.recDragNoMods";
          recorderFeedbackVars = undefined;
          renderMain();
          return;
        }
        recordingTarget = null;
        recorderFeedbackKey = null;
        recorderFeedbackVars = undefined;
        recorderFeedbackTarget = null;
        void persistPrefs((d) => {
          d.dragTrigger = token;
        }).then(() => {
          if (!disposed) renderMain();
        });
        return;
      }

      // Idle combo validation (requires >=1 modifier or F1-F12)
      const isFn = /^F([1-9]|1[0-2])$/.test(token);
      if (!ev.metaKey && !ev.altKey && !ev.ctrlKey && !ev.shiftKey && !isFn) {
        recorderFeedbackTarget = "idle";
        recorderFeedbackKey = "settings.shortcuts.recIdleNeedMod";
        recorderFeedbackVars = undefined;
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
      recorderFeedbackKey = null;
      recorderFeedbackVars = undefined;
      recorderFeedbackTarget = null;
      void persistPrefs((d) => {
        d.idleShortcut = parts.join("+");
      }).then(() => {
        if (!disposed) renderMain();
      });
      return;
    }

    // Cmd+1..Cmd+6 switches Settings tabs when not typing in a field
    if (ev.metaKey && !ev.ctrlKey && !ev.altKey && !ev.shiftKey) {
      const idx = Number(ev.key) - 1;
      if (idx >= 0 && idx < SETTINGS_TABS.length) {
        const targetEl = ev.target;
        if (targetEl instanceof Element && targetEl.closest("input, textarea, select")) {
          return;
        }
        ev.preventDefault();
        selectTab(SETTINGS_TABS[idx]);
      }
    }
  };

  window.addEventListener("keydown", onWindowKeyDown, true);

  navEl.addEventListener("click", (ev) => {
    const btn = ev.target instanceof Element
      ? ev.target.closest<HTMLButtonElement>(".nav-item[data-tab]")
      : null;
    if (!btn) return;
    selectTab(btn.dataset.tab as SettingsTab);
  });

  navEl.addEventListener("keydown", (ev) => {
    if (ev.key !== "ArrowDown" && ev.key !== "ArrowUp") return;
    const buttons = Array.from(navEl.querySelectorAll<HTMLButtonElement>(".nav-item[data-tab]"));
    if (buttons.length === 0) return;
    const currentIdx = buttons.findIndex((b) => b === document.activeElement);
    if (currentIdx < 0) return;
    ev.preventDefault();
    const delta = ev.key === "ArrowDown" ? 1 : -1;
    const nextBtn = buttons[(currentIdx + delta + buttons.length) % buttons.length];
    nextBtn.focus();
    selectTab(nextBtn.dataset.tab as SettingsTab);
  });

  root.querySelector("#nav-open-batch")?.addEventListener("click", () => void openWindow("batch"));
  root.querySelector("#nav-open-onboarding")?.addEventListener("click", () => void openWindow("onboarding"));

  async function runUpdateCheck(): Promise<void> {
    if (updateChecking || disposed) return;
    lastUpdateTriggerMs = Date.now();
    updateChecking = true;
    updateError = null;
    if (activeTab === "about") renderMain();
    try {
      const res = await checkForUpdates();
      if (disposed) return;
      updateResult = res;
      updateError = null;
    } catch (err) {
      if (disposed) return;
      updateResult = null;
      updateError = formatError(err);
    } finally {
      updateChecking = false;
      if (!disposed && activeTab === "about") renderMain();
    }
  }

  async function runCopyDiagnostics(): Promise<void> {
    try {
      const res = await copyDiagnosticsToClipboard();
      if (disposed) return;
      if (res.ok) {
        diagCopiedState = "copied";
        diagFallbackText = "";
        showSavedToast(t(lang(), "settings.about.diagCopiedToast"));
        if (activeTab === "about") renderMain();
        if (diagCopyTimer !== null) window.clearTimeout(diagCopyTimer);
        diagCopyTimer = window.setTimeout(() => {
          diagCopyTimer = null;
          if (!disposed && diagCopiedState === "copied") {
            diagCopiedState = "idle";
            if (activeTab === "about") renderMain();
          }
        }, 2200);
      } else {
        diagCopiedState = "manual";
        diagFallbackText = res.report;
        showSavedToast(t(lang(), "settings.about.diagManualToast"), true);
        if (activeTab === "about") renderMain();
      }
    } catch (err) {
      if (!disposed) {
        showSavedToast(t(lang(), "common.errorPrefix", { msg: formatError(err) }), true);
      }
    }
  }

  async function refreshDiagnostics(): Promise<void> {
    const [eng, perms, qa, kindsResp, rec] = await Promise.all([
      getEngineStatus().catch(() => null),
      getPermissionsStatus().catch(() => null),
      getQuickActionsStatus().catch(() => null),
      getNewFileKinds().catch(
        (): NewFileKindsResponse => ({ kinds: [], templates: [] }),
      ),
      getRecentConversions().catch(() => []),
    ]);
    if (disposed) return;
    if (eng) engines = eng;
    if (perms) permissions = perms;
    if (qa) quickActions = qa;
    newFileKinds = kindsResp.kinds;
    templates = kindsResp.templates;
    clipboardInfo = kindsResp.clipboard;
    recent = rec;
    if (!recordingTarget) {
      renderMain();
    }
  }

  function handleSettingsNavigate(payload: SettingsNavigatePayload, refreshTab = true): void {
    if (disposed) return;
    if (payload.tab) {
      activeTab = payload.tab;
      recordingTarget = null;
      recorderFeedbackKey = null;
      recorderFeedbackVars = undefined;
      recorderFeedbackTarget = null;
    }
    renderMain();
    if (refreshTab && (activeTab === "engines" || activeTab === "newfile" || activeTab === "about")) {
      void refreshDiagnostics();
    }
    if (payload.checkUpdates && Date.now() - lastUpdateTriggerMs > 600) {
      void runUpdateCheck();
    }
  }

  const hashQuery = window.location.hash.split("?")[1] ?? "";
  const hashParams = new URLSearchParams(hashQuery);
  const initialTab = hashParams.get("tab") as SettingsTab | null;
  if (
    initialTab &&
    ["general", "shortcuts", "formats", "newfile", "engines", "about"].includes(initialTab)
  ) {
    activeTab = initialTab;
  }

  let unlistenNavigate: (() => void) | null = null;
  let unlistenTemplates: (() => void) | null = null;
  let unlistenPrefs: (() => void) | null = null;
  void (async () => {
    const [navFn, tmplFn, prefsFn] = await Promise.all([
      events.onSettingsNavigate((payload) => handleSettingsNavigate(payload, true)),
      events.onTemplatesChanged(() => {
        if (!disposed && activeTab === "newfile") {
          void refreshDiagnostics();
        }
      }),
      events.onPrefsChanged((updated) => {
        if (disposed) return;
        const langChanged = normalizeLanguage(updated.language) !== lang();
        prefs = updated;
        if (!recordingTarget && (langChanged || !mainEl.contains(document.activeElement))) {
          renderMain();
        } else {
          updateSidebarLocalization();
        }
      }),
    ]);
    if (disposed) {
      navFn();
      tmplFn();
      prefsFn();
      return;
    }
    unlistenNavigate = navFn;
    unlistenTemplates = tmplFn;
    unlistenPrefs = prefsFn;

    const pendingNav = await consumeSettingsNavigation().catch(() => null);
    if (disposed) return;
    if (pendingNav) {
      handleSettingsNavigate(pendingNav, false);
    }

    prefs = await getPreferences().catch(() => defaultPreferences());
    if (disposed) return;
    renderMain();
    await refreshDiagnostics();
    if (hashParams.get("checkUpdates") === "1" && activeTab === "about" && !pendingNav?.checkUpdates) {
      void runUpdateCheck();
    }
  })();

  return () => {
    disposed = true;
    if (saveToastTimer !== null) window.clearTimeout(saveToastTimer);
    if (diagCopyTimer !== null) window.clearTimeout(diagCopyTimer);
    if (unlistenNavigate) unlistenNavigate();
    if (unlistenTemplates) unlistenTemplates();
    if (unlistenPrefs) unlistenPrefs();
    window.removeEventListener("keydown", onWindowKeyDown, true);
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
