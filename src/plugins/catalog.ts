import type { BuiltinPluginId, BuiltinPluginManifest, PluginCapabilityDeclaration, PluginCommandDeclaration } from "./types";

const command = (id: PluginCommandDeclaration["id"], kind: PluginCommandDeclaration["kind"]): PluginCommandDeclaration => ({
  id, kind, scope: "selected-project", requiresModel: false, requiresNetwork: false,
  permission: kind === "read" ? "existing-project-read" : "host-navigation", implementation: "built-in",
});
const capability = (id: string, label: PluginCapabilityDeclaration["label"], requiresModel: boolean,
  requiresNetwork: boolean, requiredProviders: readonly string[] = []): PluginCapabilityDeclaration => ({
  id, label, requiresModel, requiresNetwork, requiredProviders, authority: "existing-domain-gates", hostCallable: false,
});
function freeze<T>(value: T): T {
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) freeze(item);
    Object.freeze(value);
  }
  return value;
}

/** Compile-time contributions only. No URL, script path, dynamic manifest or installer. */
export const BUILTIN_PLUGINS: readonly BuiltinPluginManifest[] = freeze([
  {
    id: "asset", label: ["资产", "Assets"], description: ["管理资产、归属与公开暴露面；本地管理不需要模型。", "Manage assets, ownership and public exposure. Local management needs no model."],
    defaultView: "dashboard", views: ["dashboard", "tasks", "assets", "ownership", "exposure", "changes", "quarantine", "query"], hasLogs: true, requirements: [],
    pages: [
      { id: "dashboard", view: "dashboard", label: ["仪表盘", "Dashboard"] },
      { id: "tasks", view: "tasks", label: ["任务中心", "Tasks"] },
      { id: "assets", view: "assets", label: ["资产", "Assets"] },
      { id: "ownership", view: "ownership", label: ["归属门禁", "Ownership gate"] },
      { id: "exposure", view: "exposure", label: ["公开暴露面", "Public exposure"] },
      { id: "changes", view: "changes", label: ["变化对比", "Changes"] },
      { id: "quarantine", view: "quarantine", label: ["隔离区", "Quarantine"] },
      { id: "query", view: "query", label: ["新建查询", "New query"] },
      { id: "logs", view: "logs", label: ["资产日志", "Asset logs"] },
    ],
    commands: [command("assets.list-link-candidates", "read"), command("assets.open-workspace", "navigate")],
    capabilities: [
      capability("assets.local-management", ["本地查询与整理", "Local search and management"], false, false),
      capability("assets.fofa-collection", ["FOFA 采集", "FOFA collection"], false, true, ["fofa"]),
      capability("assets.probe", ["授权资产探测", "Authorized asset probing"], false, true),
      capability("assets.ownership", ["规则归属分析", "Rule-based ownership"], false, false),
      capability("assets.exposure", ["授权公开暴露面", "Authorized public exposure"], false, true),
    ],
  },
  {
    id: "sentinel", label: ["扫描", "Scanning"], description: ["查看扫描结果，或进入现有工作台发起 Web、源码、灰盒和 CI 调查。", "Review scan results, or use the existing workbench for Web, source, greybox and CI investigations."],
    defaultView: "sentinel", views: ["sentinel"], hasLogs: true, requirements: [],
    pages: [
      { id: "dialog", view: "sentinel", section: "dialog", label: ["调查对话", "Investigation dialog"] },
      { id: "urls", view: "sentinel", section: "results", label: ["这次结果", "This investigation"] },
      { id: "queue", view: "sentinel", section: "queue", label: ["任务与成本", "Tasks and cost"] },
      { id: "skills", view: "sentinel", section: "workbench", label: ["知识与策略", "Knowledge and policy"] },
      { id: "logs", view: "logs", label: ["扫描日志", "Scan logs"] },
    ],
    commands: [command("scanning.list-link-candidates", "read"), command("scanning.open-workspace", "navigate")],
    capabilities: [
      capability("scanning.saved-results", ["已保存的结果与运行历史", "Saved results and run history"], false, false),
      capability("scanning.web-draft", ["Web 调查草稿", "Web investigation draft"], false, false),
      capability("scanning.native-investigation", ["现有 Native 调查执行", "Existing Native investigation execution"], true, true, ["configured-model"]),
    ],
  },
  {
    id: "hackerone", label: ["HackerOne", "HackerOne"], description: ["保留现有 HackerOne 项目与 Scope 看板及配置门禁。", "Existing HackerOne programs and scope board, with its current configuration gate."],
    defaultView: "hackerone", views: ["hackerone"], hasLogs: true, requirements: ["hackerone-profile"],
    pages: [{ id: "hackerone", view: "hackerone", label: ["HackerOne 项目", "HackerOne programs"] }, { id: "logs", view: "logs", label: ["HackerOne 变化", "HackerOne changes"] }],
    commands: [],
    capabilities: [
      capability("hackerone.cached-records", ["已缓存的项目记录", "Cached program records"], false, false),
      capability("hackerone.sync", ["官方 API 同步", "Official API sync"], false, true, ["hackerone"]),
    ],
  },
] satisfies BuiltinPluginManifest[]);
export function isBuiltinPluginId(value: unknown): value is BuiltinPluginId {
  return typeof value === "string" && BUILTIN_PLUGINS.some(plugin => plugin.id === value);
}
export function getBuiltinPlugin(id: BuiltinPluginId): BuiltinPluginManifest {
  const plugin = BUILTIN_PLUGINS.find(item => item.id === id);
  if (!plugin) throw Error("plugin_unknown");
  return plugin;
}
