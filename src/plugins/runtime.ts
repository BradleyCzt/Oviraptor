import { isBuiltinPluginId } from "./catalog";
import { isPluginEnabled } from "./preferences";
import type { BuiltinPluginHost, BuiltinPluginHostOptions, BuiltinPluginId, PluginLinkCursor, PluginLinkInput, PluginLinkPage, PluginNavigationIntent } from "./types";

const PAGE_SIZE = 50;
const sections = ["dialog", "results", "queue", "workbench"];
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const positiveId = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value > 0;
const text = (value: unknown, maximum: number): value is string => typeof value === "string" && value.length > 0 && value.length <= maximum && !/[\u0000-\u001f\u007f]/.test(value);
const exactKeys = (value: object, allowed: readonly string[]) => Object.keys(value).every(key => allowed.includes(key));
const label = (...values: unknown[]) => {
  const value = values.find(item => typeof item === "string" && item.trim());
  return typeof value === "string" ? value.replace(/[\u0000-\u001f\u007f]/g, " ").trim().slice(0, 240) : "";
};

function normalizeInput(input: PluginLinkInput): PluginLinkInput & { search: string } {
  if (!object(input) || !exactKeys(input, ["pluginId", "projectId", "search", "cursor"])
    || !["asset", "sentinel"].includes(input.pluginId) || !positiveId(input.projectId)
    || (input.search !== undefined && (typeof input.search !== "string" || input.search.length > 500 || /[\u0000-\u001f\u007f]/.test(input.search)))) throw Error("plugin_input_invalid");
  const search = (input.search ?? "").trim();
  if (input.cursor !== undefined) {
    const cursor = input.cursor;
    if (!object(cursor) || cursor.pluginId !== input.pluginId || cursor.projectId !== input.projectId || cursor.search !== search) throw Error("plugin_cursor_scope_mismatch");
    if (cursor.pluginId === "asset") {
      if (!exactKeys(cursor, ["pluginId", "projectId", "search", "page"]) || !positiveId(cursor.page) || cursor.page > 1_000_000) throw Error("plugin_cursor_invalid");
    } else if (!exactKeys(cursor, ["pluginId", "projectId", "search", "before"]) || !object(cursor.before)
      || !exactKeys(cursor.before, ["updatedAt", "id"]) || !text(cursor.before.updatedAt, 100) || !text(cursor.before.id, 200)) throw Error("plugin_cursor_invalid");
  }
  // Snapshot mutable inputs before the first await; subsequent UI edits cannot
  // retarget or relabel the original read response.
  const cursor: PluginLinkCursor | undefined = input.cursor?.pluginId === "asset" ? { ...input.cursor }
    : input.cursor?.pluginId === "sentinel" ? { ...input.cursor, before: { ...input.cursor.before } } : undefined;
  return { pluginId: input.pluginId, projectId: input.projectId, search, cursor };
}

/**
 * A narrow host adapter, not an agent tool registry. There is no arbitrary IPC,
 * model dispatch, shell, scan start, provider call or background polling here.
 * The host must invalidate synchronously on project/enabled preference changes.
 * Picker-local search/selection epochs remain the picker's responsibility.
 */
export function createBuiltinPluginHost(options: BuiltinPluginHostOptions): BuiltinPluginHost {
  let epoch = 0;
  const isEnabled = (id: BuiltinPluginId) => isBuiltinPluginId(id) && isPluginEnabled(options.enabled(), id);
  function scope(pluginId: BuiltinPluginId, projectId: number) {
    if (!isBuiltinPluginId(pluginId) || !isEnabled(pluginId)) throw Error("plugin_disabled");
    if (!positiveId(projectId) || options.projectId() !== projectId) throw Error("plugin_project_scope_mismatch");
    const project = options.projects().find(item => item.id === projectId);
    if (!project || !text(project.status, 50)) throw Error("plugin_project_unavailable");
    return project.status;
  }
  function openWorkspace(intent: PluginNavigationIntent) {
    if (!object(intent) || !["asset", "sentinel"].includes(intent.pluginId) || !positiveId(intent.projectId)) throw Error("plugin_navigation_invalid");
    if (intent.pluginId === "asset") {
      if (!exactKeys(intent, ["pluginId", "projectId", "page"]) || intent.page !== "assets") throw Error("plugin_navigation_invalid");
    } else if (!exactKeys(intent, ["pluginId", "projectId", "page", "section"]) || intent.page !== "sentinel" || !sections.includes(intent.section)) throw Error("plugin_navigation_invalid");
    scope(intent.pluginId, intent.projectId);
    // This is a direct user-requested workspace transition, never an entity deep
    // link, queue entry, deferred intent, or proof that a backend action ran.
    options.navigate({ ...intent });
  }
  async function listLinkCandidates(raw: PluginLinkInput): Promise<PluginLinkPage> {
    const input = normalizeInput(raw);
    const status = scope(input.pluginId, input.projectId);
    const capturedEpoch = epoch;
    const current = () => {
      if (capturedEpoch !== epoch) throw Error("plugin_scope_changed");
      if (scope(input.pluginId, input.projectId) !== status) throw Error("plugin_scope_changed");
    };
    if (input.pluginId === "asset") {
      const page = input.cursor?.pluginId === "asset" ? input.cursor.page : 1;
      let value;
      try {
        value = await options.api.listAssets({ projectId: input.projectId, search: input.search, conditions: [], page, pageSize: PAGE_SIZE, includeDeleted: false, deletedView: "active" });
      } catch { current(); throw Error("plugin_read_failed"); }
      current();
      if (!value || !Array.isArray(value.items) || value.items.length > PAGE_SIZE || value.page !== page || value.pageSize !== PAGE_SIZE
        || !Number.isSafeInteger(value.total) || value.total < value.items.length) throw Error("plugin_response_invalid");
      const seen = new Set<number>();
      const items = value.items.map(row => {
        if (!row || !positiveId(row.id) || row.projectId !== input.projectId || row.isDeleted !== false || seen.has(row.id)) throw Error("plugin_response_scope_invalid");
        seen.add(row.id);
        return { kind: "asset" as const, targetId: String(row.id), projectId: row.projectId,
          label: label(row.title, row.host, row.link, row.assetKey) || `Asset #${row.id}`,
          detail: label(row.host, row.link, row.assetKey) };
      });
      const nextCursor: PluginLinkCursor | undefined = items.length > 0 && page * PAGE_SIZE < value.total && page < 1_000_000
        ? { pluginId: "asset", projectId: input.projectId, search: input.search, page: page + 1 } : undefined;
      return { items, ...(nextCursor ? { nextCursor } : {}) };
    }
    const before = input.cursor?.pluginId === "sentinel" ? input.cursor.before : undefined;
    let rows;
    try { rows = await options.api.searchSentinelScanPage(input.projectId, input.search, "all", PAGE_SIZE, before); }
    catch { current(); throw Error("plugin_read_failed"); }
    current();
    if (!Array.isArray(rows) || rows.length > PAGE_SIZE) throw Error("plugin_response_invalid");
    const seen = new Set<string>();
    const items = rows.map(row => {
      if (!row || !text(row.id, 200) || row.projectId !== input.projectId || !text(row.updatedAt, 100) || seen.has(row.id)) throw Error("plugin_response_scope_invalid");
      seen.add(row.id);
      return { kind: "scan" as const, targetId: row.id, projectId: input.projectId,
        label: label(row.taskName, row.projectName) || row.id,
        detail: [label(row.scanType), label(row.status)].filter(Boolean).join(" · ") };
    });
    const last = rows[rows.length - 1];
    if (last && before && last.updatedAt === before.updatedAt && last.id === before.id) throw Error("plugin_cursor_not_advanced");
    const nextCursor: PluginLinkCursor | undefined = rows.length === PAGE_SIZE
      ? { pluginId: "sentinel", projectId: input.projectId, search: input.search, before: { updatedAt: last.updatedAt, id: last.id } } : undefined;
    return { items, ...(nextCursor ? { nextCursor } : {}) };
  }
  return { isEnabled, listLinkCandidates, openWorkspace, invalidate() { epoch++; } };
}
