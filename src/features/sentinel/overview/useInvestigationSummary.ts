import { onUnmounted, ref, watch } from "vue";
import type { InvestigationOverview } from "../types";

export type InvestigationSummaryState = "idle" | "loading" | "ready" | "error";
const emptySummary = (): InvestigationOverview => ({
  targetCount: 0, nodeCount: 0, edgeCount: 0, apiCount: 0, parameterCount: 0,
  hypothesisCount: 0, readyHypothesisCount: 0, identityDiffCount: 0,
  tokenWorthyCount: 0, averageInformationGain: 0, factCount: 0, promotedStrategyCount: 0,
});
const metricKeys = Object.keys(emptySummary());

function validSummary(value: unknown): value is InvestigationOverview {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  return metricKeys.every(key => {
    const count = (value as Record<string, unknown>)[key];
    return Object.prototype.hasOwnProperty.call(value, key) && typeof count === "number"
      && Number.isSafeInteger(count) && count >= 0;
  });
}

// Project-scoped, read-only statistics. This snapshot is independent of task
// lists and other panels; no cross-panel database snapshot is implied.
export function useInvestigationSummary(options: {
  scope: () => number | undefined;
  read: (project?: number) => Promise<InvestigationOverview>;
}) {
  const stats = ref<InvestigationOverview>(emptySummary());
  const state = ref<InvestigationSummaryState>("idle");
  let generation = 0;
  let disposed = false;

  watch(options.scope, () => {
    ++generation;
    stats.value = emptySummary();
    state.value = "idle";
  }, { flush: "sync" });

  async function refresh() {
    if (disposed) return;
    const project = options.scope();
    const request = ++generation;
    const current = () => !disposed && request === generation && project === options.scope();
    state.value = "loading";
    try {
      const next = await options.read(project);
      if (!current()) return;
      if (!validSummary(next)) throw new Error("Invalid investigation summary");
      stats.value = next;
      state.value = "ready";
    } catch {
      if (!current()) return;
      state.value = "error";
      throw new Error("调查摘要读取失败，请重试");
    }
  }

  onUnmounted(() => { disposed = true; ++generation; });
  return { stats, state, refresh };
}
