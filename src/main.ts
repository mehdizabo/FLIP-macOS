import "./styles/wheel.css";
import "./styles/app.css";
import { resolveRoute } from "./router";
import { mountBatchView } from "./views/BatchView";
import { mountOnboardingView } from "./views/OnboardingView";
import { mountSettingsView } from "./views/SettingsView";
import { mountWheelView } from "./views/WheelView";

let cleanupCurrentView: (() => void) | null = null;

function renderRoute(): void {
  const root = document.getElementById("app");
  if (!root) return;

  if (cleanupCurrentView) {
    cleanupCurrentView();
    cleanupCurrentView = null;
  }

  const route = resolveRoute(window.location.hash);
  document.documentElement.dataset.route = route;

  switch (route) {
    case "wheel":
      cleanupCurrentView = mountWheelView(root);
      break;
    case "batch":
      cleanupCurrentView = mountBatchView(root);
      break;
    case "onboarding":
      cleanupCurrentView = mountOnboardingView(root);
      break;
    case "settings":
      cleanupCurrentView = mountSettingsView(root);
      break;
  }
}

window.addEventListener("hashchange", renderRoute);
renderRoute();
