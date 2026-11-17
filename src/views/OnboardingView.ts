import { UI_ICONS } from "../icons";
import {
  escapeHtml,
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

  function render(): void {
    const dragKey = prefs?.dragTrigger ?? "Down";
    const idleCombo = prefs?.idleShortcut ?? "Option+Shift+Space";
    const axGranted = perms.accessibility;
    const autoGranted = perms.automationFinder === "granted";

    let contentHtml = "";
    if (step === 0) {
      contentHtml = `
        <div class="onboarding-hero">
          <span class="status-pill is-ok" style="align-self:flex-start">Step 1 of 3 · Drag &amp; Flip</span>
          <h1>Convert any Finder file mid-drag</h1>
          <p>
            Start dragging one or more files from Finder or your Desktop and press
            <span class="kbd-cap">${escapeHtml(dragKey)}</span> (or hold the key first, then drag).
            FLIP's Liquid Glass wheel appears right under your cursor — drop onto any format to save a non-overwriting copy next to the original.
          </p>
          <div class="glass-card" style="background:var(--bg-inset);box-shadow:none">
            <div style="font-weight:600;margin-bottom:4px">100% Local · Original Stem Preserved</div>
            <div class="setting-desc">
              <code>photo.heic</code> → <code>photo.png</code> (or <code>photo 2.png</code> if one already exists). Zero cloud uploads.
            </div>
          </div>
        </div>
      `;
    } else if (step === 1) {
      contentHtml = `
        <div class="onboarding-hero">
          <span class="status-pill is-ok" style="align-self:flex-start">Step 2 of 3 · Idle &amp; Create New File</span>
          <h1>Create New Files in Finder from anywhere</h1>
          <p>
            Press <span class="kbd-cap">${escapeHtml(idleCombo)}</span> without dragging:
            if files are selected in Finder, FLIP opens in <strong>Convert</strong> mode; otherwise it opens in
            <strong>New File</strong> mode targeting your active Finder folder.
          </p>
          <div class="glass-card" style="background:var(--bg-inset);box-shadow:none">
            <div style="font-weight:600;margin-bottom:4px">Instant Finder Inline Rename</div>
            <div class="setting-desc">
              Create Folders, Markdown, Word, Spreadsheets, Scripts, Clipboard captures, or custom Templates — then start typing the final filename immediately.
            </div>
          </div>
        </div>
      `;
    } else {
      contentHtml = `
        <div class="onboarding-hero">
          <span class="status-pill ${axGranted && autoGranted ? "is-ok" : "is-warn"}" style="align-self:flex-start">Step 3 of 3 · macOS Permissions</span>
          <h1>Enable Accessibility &amp; Finder Automation</h1>
          <p>
            Accessibility lets FLIP intercept your drag key only during a drag (without affecting normal typing) and trigger Finder inline rename.
            Automation lets FLIP read the active Finder selection and folder.
          </p>
          <div class="glass-card" style="padding:12px 14px">
            <div class="setting-row">
              <div>
                <div class="setting-label">Accessibility</div>
                <div class="setting-desc">Drag trigger key &amp; inline Finder rename</div>
              </div>
              <div style="display:flex;gap:8px;align-items:center">
                <span class="status-pill ${axGranted ? "is-ok" : "is-warn"}">${axGranted ? "Granted" : "Required"}</span>
                ${axGranted ? "" : `<button type="button" class="btn btn-primary btn-sm" id="ob-grant-ax">Grant Permission</button>`}
                <button type="button" class="btn btn-sm" id="ob-open-ax">System Settings</button>
              </div>
            </div>
            <div class="setting-row">
              <div>
                <div class="setting-label">Finder Automation</div>
                <div class="setting-desc">Active Finder folder &amp; selected items</div>
              </div>
              <div style="display:flex;gap:8px;align-items:center">
                <span class="status-pill ${autoGranted ? "is-ok" : "is-warn"}">${escapeHtml(perms.automationFinder)}</span>
                <button type="button" class="btn btn-sm" id="ob-open-auto">System Settings</button>
              </div>
            </div>
          </div>
          <div class="setting-desc">
            Without permissions, FLIP still works in <strong>Limited Mode</strong>: summon the wheel with your global hotkey and drop files directly onto it.
          </div>
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
                <div class="brand-title">Welcome to FLIP</div>
                <div class="brand-sub">Liquid Glass File Converter &amp; New File Ring</div>
              </div>
            </div>
            <button type="button" class="btn btn-sm" id="ob-refresh">${UI_ICONS.refresh}</button>
          </div>

          <div class="step-indicators">
            ${[0, 1, 2].map((i) => `<div class="step-pip ${i <= step ? "is-active" : ""}"></div>`).join("")}
          </div>

          ${contentHtml}

          <div class="onboarding-footer">
            <button type="button" class="btn" id="ob-skip">Skip (Limited Mode)</button>
            <div style="display:flex;gap:8px">
              ${step > 0 ? `<button type="button" class="btn" id="ob-back">Back</button>` : ""}
              ${
                step < 2
                  ? `<button type="button" class="btn btn-primary" id="ob-next">Next</button>`
                  : `<button type="button" class="btn btn-primary" id="ob-finish">Finish Setup</button>`
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

  render();
  void refreshPerms();

  return () => {
    disposed = true;
  };
}
