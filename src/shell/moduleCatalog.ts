import type { ViewKey } from "../types";
import { BUILTIN_PLUGINS, getBuiltinPlugin } from "../plugins/catalog";
import type { PluginEnabledState } from "../plugins/types";

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
  ...BUILTIN_PLUGINS.map(plugin => ({id:plugin.id,label:plugin.label,icon:(plugin.id==="asset"?"assets":plugin.id==="sentinel"?"scan":"hackerone") as ShellIcon,defaultView:plugin.defaultView,views:plugin.views,hasLogs:plugin.hasLogs,requiresHackerOne:plugin.requirements.includes("hackerone-profile")})),
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
const decorations: Record<string, Pick<WorkspaceNavItem,"icon"|"badge"|"cta"|"result"|"workbench">> = {
  dashboard:{icon:"dashboard"},tasks:{icon:"tasks",badge:"runningJobs"},assets:{icon:"assets"},ownership:{icon:"ownership"},exposure:{icon:"exposure"},changes:{icon:"changes"},quarantine:{icon:"quarantine",badge:"blockedCount"},query:{icon:"query",cta:true},
  dialog:{icon:"dialog"},urls:{icon:"results",result:"summary"},queue:{icon:"tasks",badge:"fuse"},skills:{icon:"skills",workbench:"skills"},
};
const contributedNavigation=(id:"asset"|"sentinel"):readonly WorkspaceNavItem[]=>getBuiltinPlugin(id).pages.filter(page=>page.view!=="logs").map(page=>({key:page.id,view:page.view,label:page.label,section:page.section,...decorations[page.id]}));
const ASSET_NAV=contributedNavigation("asset");
const SCANNING_NAV=contributedNavigation("sentinel");
export const moduleDefinition = (id: ModuleKey) => MODULE_CATALOG.find(item => item.id === id)!;
export const visibleModules = (hackerOneEnabled: boolean, enabled?: PluginEnabledState) => MODULE_CATALOG.filter(item => (item.id==="chat"||!enabled||enabled[item.id]) && (!item.requiresHackerOne || hackerOneEnabled));
export const workspaceNavigation = (id: ModuleKey): readonly WorkspaceNavItem[] => id === "asset" ? ASSET_NAV : id === "sentinel" ? SCANNING_NAV : [];
export const isModuleKey = (value: unknown): value is ModuleKey => MODULE_CATALOG.some(item => item.id === value);
export function moduleForView(view: unknown): ModuleKey | undefined {
  return MODULE_CATALOG.find(item => item.views.some(candidate => candidate === view))?.id;
}
