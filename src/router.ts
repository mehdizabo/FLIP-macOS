import type { RouteName } from "./types";

export function resolveRoute(hash: string): RouteName {
  const clean = hash.replace(/^#\/?/, "").split("?")[0].trim().toLowerCase();
  switch (clean) {
    case "wheel":
      return "wheel";
    case "batch":
      return "batch";
    case "onboarding":
      return "onboarding";
    case "settings":
    default:
      return "settings";
  }
}
