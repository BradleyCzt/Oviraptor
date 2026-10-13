import type { ViewKey } from "../types";

export type ModuleKey = "chat" | "asset" | "sentinel" | "hackerone";
export type ShellIcon = "chat" | "assets" | "scan" | "hackerone" | "dashboard" | "tasks" | "ownership" | "exposure" | "changes" | "quarantine" | "query" | "dialog" | "results" | "skills";
export interface ModuleDefinition {
  id: ModuleKey;
  label: readonly [string, string];
  icon: ShellIcon;
  defaultView: ViewKey;
  views: readonly ViewKey[];
  requiresHackerOne?: boolean;
  hasLogs: boolean;
}

// Product workspaces only. Runtime/sandbox services are deliberately not routes.
// These stable IDs preserve saved navigation from earlier releases.
export const MODULE_CATALOG: readonly ModuleDefinition[] = [
  { id: "chat", label: ["对话", "Chat"], icon: "chat", defaultView: "chat", views: ["chat"], hasLogs: false },
  { id: "asset", label: ["资产", "Assets"], icon: "assets", defaultView: "dashboard", views: ["dashboard", "tasks", "assets", "ownership", "exposure", "changes", "quarantine", "query"], hasLogs: true },
  { id: "sentinel", label: ["扫描", "Scanning"], icon: "scan", defaultView: "sentinel", views: ["sentinel"], hasLogs: true },
  { id: "hackerone", label: ["HackerOne", "HackerOne"], icon: "hackerone", defaultView: "hackerone", views: ["hackerone"], requiresHackerOne: true, hasLogs: true },
];
export interface WorkspaceNavItem {
  key: string;
  view: ViewKey;
  label: readonly [string, string];
  icon: ShellIcon;
  badge?: "runningJobs" | "blockedCount" | "fuse";
  cta?: boolean;
  section?: "dialog" | "results" | "queue" | "workbench";
  result?: "summary";
  workbench?: "skills";
}
const ASSET_NAV: readonly WorkspaceNavItem[] = [
  { key: "dashboard", view: "dashboard", label: ["仪表盘", "Dashboard"], icon: "dashboard" },
  { key: "tasks", view: "tasks", label: ["任务中心", "Tasks"], icon: "tasks", badge: "runningJobs" },
  { key: "assets", view: "assets", label: ["资产", "Assets"], icon: "assets" },
  { key: "ownership", view: "ownership", label: ["归属门禁", "Ownership gate"], icon: "ownership" },
  { key: "exposure", view: "exposure", label: ["公开暴露面", "Public exposure"], icon: "exposure" },
  { key: "changes", view: "changes", label: ["变化对比", "Changes"], icon: "changes" },
  { key: "quarantine", view: "quarantine", label: ["隔离区", "Quarantine"], icon: "quarantine", badge: "blockedCount" },
  { key: "query", view: "query", label: ["新建查询", "New Query"], icon: "query", cta: true },
];
const SCANNING_NAV: readonly WorkspaceNavItem[] = [
  { key: "dialog", view: "sentinel", section: "dialog", label: ["调查对话", "Investigation dialog"], icon: "dialog" },
  { key: "urls", view: "sentinel", section: "results", result: "summary", label: ["这次结果", "This investigation"], icon: "results" },
  { key: "queue", view: "sentinel", section: "queue", label: ["任务与成本", "Tasks & cost"], icon: "tasks", badge: "fuse" },
  { key: "skills", view: "sentinel", section: "workbench", workbench: "skills", label: ["知识与策略", "Knowledge & policy"], icon: "skills" },
];
export const moduleDefinition = (id: ModuleKey) => MODULE_CATALOG.find(item => item.id === id)!;
export const visibleModules = (hackerOneEnabled: boolean) => MODULE_CATALOG.filter(item => !item.requiresHackerOne || hackerOneEnabled);
export const workspaceNavigation = (id: ModuleKey): readonly WorkspaceNavItem[] => id === "asset" ? ASSET_NAV : id === "sentinel" ? SCANNING_NAV : [];
export const isModuleKey = (value: unknown): value is ModuleKey => MODULE_CATALOG.some(item => item.id === value);
export function moduleForView(view: unknown): ModuleKey | undefined {
  return MODULE_CATALOG.find(item => item.views.some(candidate => candidate === view))?.id;
}
