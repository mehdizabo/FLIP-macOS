import { normalizeLanguage, t } from "../i18n";
import { UI_ICONS } from "../icons";
import {
  escapeHtml,
  events,
  formatError,
  getPermissionsStatus,
  getPreferences,
  isTauriRuntime,
  openWindow,
  requestPermission,
} from "../ipc";
import type { AppPreferences, PermissionsStatus } from "../types";

export const ONBOARDING_COMPLETED_KEY = "flip:onboardingCompleted";

export function mountOnboardingView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "onboarding";
  document.documentElement.dataset.tauri = isTauriRuntime() ? "1" : "0";

  let disposed = false;
  let step = 0; // 0, 1, 2
  let prefs: AppPreferences | null = null;
  let permError = "";
  let perms: PermissionsStatus = {
    accessibility: false,
    automationFinder: "not_determined",
    inputMonitoring: false,
  };

  const lang = () => normalizeLanguage(prefs?.language);

  function render(): void {
    const l = lang();
    const dragKey = prefs?.dragTrigger ?? "Down";
    const idleCombo = prefs?.idleShortcut ?? "Option+Shift+Space";
    const axGranted = perms.accessibility;
    const autoGranted = perms.automationFinder === "granted";
    const autoLabel =
      perms.automationFinder === "granted"
        ? t(l, "settings.engines.granted")
        : perms.automationFinder === "not_determined"
          ? t(l, "settings.engines.notDetermined")
          : perms.automationFinder === "denied"
            ? t(l, "settings.engines.denied")
            : perms.automationFinder === "unknown"
              ? t(l, "settings.engines.unknown")
              : perms.automationFinder;

    let contentHtml = "";
    if (step === 0) {
      contentHtml = `
        <div class="onboarding-hero">
          <div class="onboarding-step-label">${escapeHtml(t(l, "ob.step1Badge"))}</div>
          <h1>${escapeHtml(t(l, "ob.step1Title"))}</h1>
          <p>${t(l, "ob.step1Body", { dragKey: escapeHtml(dragKey) })}</p>
          <div class="settings-group onboarding-callout">
            <div class="onboarding-callout-title">${escapeHtml(t(l, "ob.step1CardTitle"))}</div>
            <div class="setting-desc">${t(l, "ob.step1CardDesc")}</div>
          </div>
        </div>
      `;
    } else if (step === 1) {
      contentHtml = `
        <div class="onboarding-hero">
          <div class="onboarding-step-label">${escapeHtml(t(l, "ob.step2Badge"))}</div>
          <h1>${escapeHtml(t(l, "ob.step2Title"))}</h1>
          <p>${t(l, "ob.step2Body", { idleCombo: escapeHtml(idleCombo) })}</p>
          <div class="settings-group onboarding-callout">
            <div class="onboarding-callout-title">${escapeHtml(t(l, "ob.step2CardTitle"))}</div>
            <div class="setting-desc">${escapeHtml(t(l, "ob.step2CardDesc"))}</div>
          </div>
        </div>
      `;
    } else {
      contentHtml = `
        <div class="onboarding-hero">
          <div class="onboarding-step-label">${escapeHtml(t(l, "ob.step3Badge"))}</div>
          <h1>${escapeHtml(t(l, "ob.step3Title"))}</h1>
          <p>${escapeHtml(t(l, "ob.step3Body"))}</p>
          <div class="settings-group">
            <div class="setting-row">
              <div class="setting-row-copy">
                <div class="setting-label">
                  <span>${escapeHtml(t(l, "ob.axLabel"))}</span>
                  <span class="status-inline ${axGranted ? "is-ok" : "is-warn"}"><span class="status-dot"></span>${escapeHtml(axGranted ? t(l, "settings.engines.granted") : t(l, "ob.required"))}</span>
                </div>
                <div class="setting-desc">${escapeHtml(t(l, "ob.axDesc"))}</div>
              </div>
              <div class="onboarding-perm-actions">
                ${axGranted ? "" : `<button type="button" class="btn btn-primary btn-sm" id="ob-grant-ax">${escapeHtml(t(l, "settings.engines.grantBtn"))}</button>`}
                <button type="button" class="btn btn-sm" id="ob-open-ax">${escapeHtml(t(l, "ob.systemSettings"))}</button>
              </div>
            </div>
            <div class="setting-row">
              <div class="setting-row-copy">
                <div class="setting-label">
                  <span>${escapeHtml(t(l, "ob.autoLabel"))}</span>
                  <span class="status-inline ${autoGranted ? "is-ok" : "is-warn"}"><span class="status-dot"></span>${escapeHtml(autoLabel)}</span>
                </div>
                <div class="setting-desc">${escapeHtml(t(l, "ob.autoDesc"))}</div>
              </div>
              <div class="onboarding-perm-actions">
                <button type="button" class="btn btn-sm" id="ob-open-auto">${escapeHtml(t(l, "ob.systemSettings"))}</button>
              </div>
            </div>
          </div>
          <div class="setting-desc">${t(l, "ob.limitedHint")}</div>
          ${permError ? `<div class="status-pill is-bad onboarding-error-pill"><span class="status-dot"></span>${escapeHtml(permError)}</div>` : ""}
        </div>
      `;
    }

    root.innerHTML = `
      <div class="onboarding-shell">
        <div class="onboarding-card">
          <div class="onboarding-header" data-tauri-drag-region>
            <div class="onboarding-brand">
              <span class="brand-mark-svg">${UI_ICONS.flipMark}</span>
              <div>
                <div class="onboarding-title">${escapeHtml(t(l, "ob.welcomeTitle"))}</div>
                <div class="brand-sub">${escapeHtml(t(l, "ob.welcomeSub"))}</div>
              </div>
            </div>
            <button type="button" class="btn btn-sm" id="ob-refresh" aria-label="${escapeHtml(t(l, "common.refresh"))}">${UI_ICONS.refresh}</button>
          </div>

          <div class="step-indicators">
            ${[0, 1, 2].map((i) => `<div class="step-pip ${i <= step ? "is-active" : ""}"></div>`).join("")}
          </div>

          ${contentHtml}

          <div class="onboarding-footer">
            <button type="button" class="btn" id="ob-skip">${escapeHtml(t(l, "ob.skipLimited"))}</button>
            <div class="onboarding-footer-nav">
              ${step > 0 ? `<button type="button" class="btn" id="ob-back">${escapeHtml(t(l, "ob.back"))}</button>` : ""}
              ${
                step < 2
                  ? `<button type="button" class="btn btn-primary" id="ob-next">${escapeHtml(t(l, "ob.next"))}</button>`
                  : `<button type="button" class="btn btn-primary" id="ob-finish">${escapeHtml(t(l, "ob.finish"))}</button>`
              }
            </div>
          </div>
        </div>
      </div>
    `;

    root.querySelector("#ob-refresh")?.addEventListener("click", () => void refreshPerms());
    root.querySelector("#ob-grant-ax")?.addEventListener("click", async () => {
      try {
        permError = "";
        perms = await requestPermission("accessibility_prompt");
      } catch (err) {
        permError = formatError(err);
      }
      if (!disposed) render();
    });
    root.querySelector("#ob-open-ax")?.addEventListener("click", async () => {
      try {
        permError = "";
        perms = await requestPermission("open_accessibility");
      } catch (err) {
        permError = formatError(err);
      }
      if (!disposed) render();
    });
    root.querySelector("#ob-open-auto")?.addEventListener("click", async () => {
      try {
        permError = "";
        perms = await requestPermission("open_automation");
      } catch (err) {
        permError = formatError(err);
      }
      if (!disposed) render();
    });
    root.querySelector("#ob-back")?.addEventListener("click", () => {
      step = Math.max(0, step - 1);
      render();
    });
    root.querySelector("#ob-next")?.addEventListener("click", () => {
      step = Math.min(2, step + 1);
      render();
    });
    root.querySelector("#ob-skip")?.addEventListener("click", () => finishOnboarding());
    root.querySelector("#ob-finish")?.addEventListener("click", () => finishOnboarding());
  }

  function finishOnboarding(): void {
    try {
      localStorage.setItem(ONBOARDING_COMPLETED_KEY, "true");
    } catch {
      // ignore storage errors
    }
    void openWindow("settings");
  }

  async function refreshPerms(): Promise<void> {
    const [p, perm] = await Promise.all([
      getPreferences().catch(() => null),
      getPermissionsStatus().catch(() => perms),
    ]);
    if (disposed) return;
    if (p) prefs = p;
    perms = perm;
    render();
  }

  let unlistenPrefs: (() => void) | null = null;
  void events.onPrefsChanged((updated) => {
    if (!disposed) {
      prefs = updated;
      render();
    }
  }).then((fn) => {
    if (disposed) fn();
    else unlistenPrefs = fn;
  });

  render();
  void refreshPerms();

  return () => {
    disposed = true;
    if (unlistenPrefs) unlistenPrefs();
  };
}
