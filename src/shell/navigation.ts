import type { ViewKey } from "../types";
import { isModuleKey, moduleDefinition, moduleForView, type ModuleKey } from "./moduleCatalog";

export interface ShellRoute { module: ModuleKey; view: ViewKey }
type NavigationStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
const SHARED_VIEWS: readonly ViewKey[] = ["projects", "settings", "logs"];
export const DEFAULT_ROUTE: ShellRoute = { module: "chat", view: "chat" };

export function normalizeRoute(module: unknown, view: unknown): ShellRoute {
  if (view === "skills") return { module: "chat", view: "skills" };
  const owner = moduleForView(view);
  if (owner) return { module: owner, view: view as ViewKey };
  const selected = isModuleKey(module) ? module : "chat";
  if (SHARED_VIEWS.some(candidate => candidate === view)) {
    // Historical logs without a module belonged to the asset workspace.
    const logModule = !isModuleKey(module) ? "asset" : selected;
    if (view === "logs") return moduleDefinition(logModule).hasLogs ? { module: logModule, view } : { ...DEFAULT_ROUTE };
    return { module: selected, view: view as ViewKey };
  }
  return { module: selected, view: moduleDefinition(selected).defaultView };
}
export function migrateLegacyNavigation(storage: NavigationStorage) {
  for (const suffix of ["view", "module", "project", "theme", "accent"]) {
    const current = `oviraptor-${suffix}`;
    const legacy = `asset-atlas-${suffix}`;
    const value = storage.getItem(legacy);
    if (storage.getItem(current) === null && value !== null) storage.setItem(current, value);
    storage.removeItem(legacy);
  }
}
export function persistRoute(storage: NavigationStorage, route: ShellRoute) {
  storage.setItem("oviraptor-module", route.module);
  storage.setItem("oviraptor-view", route.view);
}
export function restoreRoute(storage: NavigationStorage): ShellRoute {
  migrateLegacyNavigation(storage);
  // Startup is always Chat. Saved project/theme preferences still migrate, but
  // a previous business view never implicitly loads a plugin on the next launch.
  // Explicit in-session navigation continues to use normalizeRoute.
  const route = { ...DEFAULT_ROUTE };
  persistRoute(storage, route);
  return route;
}
export function parseProjectId(value: unknown): number | undefined {
  if (typeof value !== "number" && typeof value !== "string") return undefined;
  const id = Number(value);
  return Number.isSafeInteger(id) && id > 0 ? id : undefined;
}
export function trayRoute(destination: string): (ShellRoute & { section?: "queue" }) | undefined {
  if (destination === "assets") return { module: "asset", view: "assets" };
  if (destination === "agent-tasks") return { module: "sentinel", view: "sentinel", section: "queue" };
  return undefined;
}
