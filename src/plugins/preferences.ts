import { BUILTIN_PLUGINS, isBuiltinPluginId } from "./catalog";
import type { BuiltinPluginId, PluginEnabledState } from "./types";

export const PLUGIN_PREFERENCE_KEY = "oviraptor-builtin-plugins-v1";
type ReadStorage = Pick<Storage, "getItem">;
type WriteStorage = Pick<Storage, "getItem" | "setItem">;
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
export function defaultPluginEnabledState(): PluginEnabledState { return { asset: true, sentinel: true, hackerone: true }; }
export function isPluginEnabled(state: PluginEnabledState, id: BuiltinPluginId): boolean {
  return isBuiltinPluginId(id) && state[id] === true;
}
/** UI availability only. Invalid/absent preferences never mutate existing storage. */
export function loadPluginEnabledState(storage: ReadStorage): PluginEnabledState {
  const state = { ...defaultPluginEnabledState() };
  try {
    const raw: unknown = JSON.parse(storage.getItem(PLUGIN_PREFERENCE_KEY) || "null");
    if (!object(raw) || raw.version !== 1 || !object(raw.enabled)) return state;
    for (const { id } of BUILTIN_PLUGINS) if (typeof raw.enabled[id] === "boolean") state[id] = raw.enabled[id];
  } catch { /* Storage access failure is not a backend permission or deletion. */ }
  return state;
}
/** Save only the plugin namespace; no profile, model, node or legacy setting writes. */
export function savePluginEnabledState(storage: WriteStorage, state: PluginEnabledState): void {
  for (const { id } of BUILTIN_PLUGINS) if (typeof state[id] !== "boolean") throw Error("plugin_preference_invalid");
  const old = storage.getItem(PLUGIN_PREFERENCE_KEY);
  let current: Record<string, unknown> = {};
  if (old !== null) {
    let parsed: unknown;
    try { parsed = JSON.parse(old); } catch { throw Error("plugin_preference_corrupt: Saved plugin display settings are damaged and were preserved. Shared application profiles are unchanged; repair this preference before changing it."); }
    if (!object(parsed) || parsed.version !== 1 || !object(parsed.enabled)) throw Error("plugin_preference_version_unsupported: Saved plugin display settings use an unsupported format and were preserved. Use a compatible application version before changing them.");
    current = parsed;
  }
  const enabled = { ...(object(current.enabled) ? current.enabled : {}) };
  for (const { id } of BUILTIN_PLUGINS) enabled[id] = state[id];
  storage.setItem(PLUGIN_PREFERENCE_KEY, JSON.stringify({ ...current, version: 1, enabled }));
}

/** Host management can explain a fallback instead of hiding damaged preferences. */
export function getPluginPreferenceNotice(storage: ReadStorage): string | undefined {
  try {
    const raw = storage.getItem(PLUGIN_PREFERENCE_KEY);
    if (raw === null) return undefined;
    let parsed: unknown;
    try { parsed = JSON.parse(raw); } catch { return "Saved plugin display settings are damaged. Default display settings are in use; the original record and shared profiles are unchanged. Changes are blocked until this preference is repaired."; }
    if (!object(parsed) || parsed.version !== 1 || !object(parsed.enabled)) return "Saved plugin display settings use an unsupported format. Default display settings are in use; the original record is unchanged. Use a compatible application version before changing them.";
    if (BUILTIN_PLUGINS.some(({ id }) => parsed.enabled !== undefined && object(parsed.enabled) && parsed.enabled[id] !== undefined && typeof parsed.enabled[id] !== "boolean")) return "Some plugin display values are invalid. Those entries use their default display state; shared profiles are unchanged.";
  } catch { return "Plugin display settings could not be read. Defaults are in use; shared profiles are unchanged."; }
  return undefined;
}
