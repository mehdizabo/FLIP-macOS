import { normalizeLanguage, t } from "../i18n";
import { UI_ICONS } from "../icons";
import {
  escapeHtml,
  events,
  formatError,
  getPermissionsStatus,
  getPreferences,
  openWindow,
  requestPermission,
} from "../ipc";
import type { AppPreferences, PermissionsStatus } from "../types";

export const ONBOARDING_COMPLETED_KEY = "flip:onboardingCompleted";

export function mountOnboardingView(root: HTMLElement): () => void {
  document.documentElement.dataset.route = "onboarding";

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
          <span class="status-pill is-ok" style="align-self:flex-start">${escapeHtml(t(l, "ob.step1Badge"))}</span>
          <h1>${escapeHtml(t(l, "ob.step1Title"))}</h1>
          <p>${t(l, "ob.step1Body", { dragKey: escapeHtml(dragKey) })}</p>
          <div class="glass-card" style="background:var(--bg-inset);box-shadow:none">
            <div style="font-weight:600;margin-bottom:4px">${escapeHtml(t(l, "ob.step1CardTitle"))}</div>
            <div class="setting-desc">${t(l, "ob.step1CardDesc")}</div>
          </div>
        </div>
      `;
    } else if (step === 1) {
      contentHtml = `
        <div class="onboarding-hero">
          <span class="status-pill is-ok" style="align-self:flex-start">${escapeHtml(t(l, "ob.step2Badge"))}</span>
          <h1>${escapeHtml(t(l, "ob.step2Title"))}</h1>
          <p>${t(l, "ob.step2Body", { idleCombo: escapeHtml(idleCombo) })}</p>
          <div class="glass-card" style="background:var(--bg-inset);box-shadow:none">
            <div style="font-weight:600;margin-bottom:4px">${escapeHtml(t(l, "ob.step2CardTitle"))}</div>
            <div class="setting-desc">${escapeHtml(t(l, "ob.step2CardDesc"))}</div>
          </div>
        </div>
      `;
    } else {
      contentHtml = `
        <div class="onboarding-hero">
          <span class="status-pill ${axGranted && autoGranted ? "is-ok" : "is-warn"}" style="align-self:flex-start">${escapeHtml(t(l, "ob.step3Badge"))}</span>
          <h1>${escapeHtml(t(l, "ob.step3Title"))}</h1>
          <p>${escapeHtml(t(l, "ob.step3Body"))}</p>
          <div class="glass-card" style="padding:12px 14px">
            <div class="setting-row">
              <div>
                <div class="setting-label">${escapeHtml(t(l, "ob.axLabel"))}</div>
                <div class="setting-desc">${escapeHtml(t(l, "ob.axDesc"))}</div>
              </div>
              <div style="display:flex;gap:8px;align-items:center">
                <span class="status-pill ${axGranted ? "is-ok" : "is-warn"}">${escapeHtml(axGranted ? t(l, "settings.engines.granted") : t(l, "ob.required"))}</span>
                ${axGranted ? "" : `<button type="button" class="btn btn-primary btn-sm" id="ob-grant-ax">${escapeHtml(t(l, "settings.engines.grantBtn"))}</button>`}
                <button type="button" class="btn btn-sm" id="ob-open-ax">${escapeHtml(t(l, "ob.systemSettings"))}</button>
              </div>
            </div>
            <div class="setting-row">
              <div>
                <div class="setting-label">${escapeHtml(t(l, "ob.autoLabel"))}</div>
                <div class="setting-desc">${escapeHtml(t(l, "ob.autoDesc"))}</div>
              </div>
              <div style="display:flex;gap:8px;align-items:center">
                <span class="status-pill ${autoGranted ? "is-ok" : "is-warn"}">${escapeHtml(autoLabel)}</span>
                <button type="button" class="btn btn-sm" id="ob-open-auto">${escapeHtml(t(l, "ob.systemSettings"))}</button>
              </div>
            </div>
          </div>
          <div class="setting-desc">${t(l, "ob.limitedHint")}</div>
          ${permError ? `<div class="status-pill is-bad" style="align-self:flex-start">${escapeHtml(permError)}</div>` : ""}
        </div>
      `;
    }

    root.innerHTML = `
      <div class="onboarding-shell">
        <div class="glass-card onboarding-card">
          <div style="display:flex;align-items:center;justify-content:space-between;gap:12px">
            <div class="brand-header" style="padding:0">
              <div class="brand-mark">F</div>
              <div>
                <div class="brand-title">${escapeHtml(t(l, "ob.welcomeTitle"))}</div>
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
            <div style="display:flex;gap:8px">
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
