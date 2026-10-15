import { onBeforeUnmount, ref, watch } from "vue";
import { assetApi } from "./api";
import type { LogEntry } from "./types";
import { useCommittedRefresh } from "../../composables/useCommittedRefresh";

export function useAssetLogPanel(project: () => number | undefined, active: () => boolean) {
  const logs = ref<LogEntry[]>([]), loading = ref(false);
  const readFailed = ref(false), connectionUnavailable = ref(false);
  let generation = 0, disposed = false;
  const key = () => JSON.stringify([project() ?? null]);
  watch(key, () => {
    generation++; logs.value = []; loading.value = false; readFailed.value = false;
  }, { flush: "sync" });

  async function refresh() {
    if (disposed || !active()) return;
    const request = ++generation, scope = key();
    const current = () => !disposed && request === generation && scope === key();
    loading.value = true;
    try {
      const rows = await assetApi.listLogs(undefined, 300, project());
      if (!current()) return;
      if (!Array.isArray(rows) || rows.length > 300 || rows.some((row, index) =>
        !row || !Number.isSafeInteger(row.id) || row.id < 1
        || ![row.level, row.stage, row.message, row.createdAt].every(value => typeof value === "string")
        || (row.runId != null && (!Number.isSafeInteger(row.runId) || row.runId < 1))
        || (index > 0 && rows[index - 1].id <= row.id))) throw Error("Invalid log snapshot");
      logs.value = rows;
      readFailed.value = false;
    } catch { if (current()) readFailed.value = true; }
    finally { if (current()) loading.value = false; }
  }
  useCommittedRefresh("asset-log-committed", () => active() ? key() : "", payload => {
    if (!payload || typeof payload !== "object") return false;
    const hint = payload as { logId?: number; runId?: number; projectId?: number | null };
    return Number.isSafeInteger(hint.logId) && (hint.logId ?? 0) > 0
      && Number.isSafeInteger(hint.runId) && (hint.runId ?? 0) > 0
      && (hint.projectId === null || (Number.isSafeInteger(hint.projectId) && (hint.projectId ?? 0) > 0))
      && (project() === undefined || hint.projectId === project());
  }, () => loading.value, refresh, unavailable => { connectionUnavailable.value = unavailable; });
  onBeforeUnmount(() => { disposed = true; generation++; });
  return { logs, loading, readFailed, connectionUnavailable, refresh };
}
