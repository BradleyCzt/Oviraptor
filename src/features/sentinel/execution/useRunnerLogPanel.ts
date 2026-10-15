import { onBeforeUnmount, ref, watch } from "vue";
import { sentinelApi } from "../api";
import type { SentinelRunnerLogView } from "../types";
import { useLiveRunnerLog, type LogScope } from "./useLiveRunnerLog";

// The global log panel uses the same committed-snapshot delivery as task detail.
// Events never supply display text and identical consecutive lines remain valid.
export function useRunnerLogPanel(selected: () => LogScope, active: () => boolean) {
  const snapshot = ref<SentinelRunnerLogView>();
  const loading = ref(false);
  const readFailed = ref(false);
  const connectionUnavailable = ref(false);
  let generation = 0;
  let disposed = false;
  const key = () => JSON.stringify([selected().scanId, selected().attempt]);

  watch(key, () => {
    generation++;
    snapshot.value = undefined;
    readFailed.value = false;
    loading.value = false;
  }, { flush: "sync" });

  async function refresh() {
    const scope = selected();
    if (disposed || !scope.scanId) return;
    const currentKey = key();
    const request = ++generation;
    const current = () => !disposed && request === generation && key() === currentKey;
    loading.value = true;
    try {
      const view = await sentinelApi.readSentinelRunnerLog(scope.scanId, scope.attempt || undefined, 300);
      if (!current()) return;
      if (view.scanId !== scope.scanId || (scope.attempt > 0 && view.attempt !== scope.attempt)) {
        throw new Error("Log snapshot scope mismatch");
      }
      snapshot.value = view;
      readFailed.value = false;
    } catch {
      if (current()) readFailed.value = true;
      // Keep the last committed snapshot. Never render raw transport errors.
    } finally {
      if (current()) loading.value = false;
    }
  }

  useLiveRunnerLog(
    () => active() && selected().scanId ? selected() : undefined,
    () => loading.value,
    refresh,
    unavailable => { connectionUnavailable.value = unavailable; },
  );
  onBeforeUnmount(() => { disposed = true; generation++; });
  return { snapshot, loading, readFailed, connectionUnavailable, refresh };
}
