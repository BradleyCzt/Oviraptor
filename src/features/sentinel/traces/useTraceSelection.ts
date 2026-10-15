import { onUnmounted, readonly, ref, type Ref } from "vue";
import { sentinelApi } from "../api";
import type { AgentTraceDetail, AgentTraceSummary } from "../types";
import { isTraceDetailForTask } from "./traceDetailContract";

// Manual reads and polling share one selection revision, including A -> B -> A.
export function useTraceSelection(
  traces: Ref<AgentTraceSummary[]>,
  reportError: (error: unknown) => void,
) {
  const selectedId = ref("");
  const detail = ref<AgentTraceDetail>();
  const detailLoading = ref(false);
  let revision = 0;
  let disposed = false;
  let pollInFlight = false;
  let pendingLiveRefresh = false;
  const current = (requestRevision: number) => !disposed && revision === requestRevision;

  function flushPendingLiveRefresh() {
    if (disposed || pollInFlight || detailLoading.value || !pendingLiveRefresh) return;
    pendingLiveRefresh = false;
    void refreshLiveTrace(true);
  }

  function publish(latest: AgentTraceDetail, scanId: string) {
    if (!isTraceDetailForTask(latest, scanId)) {
      throw new Error("Invalid trace display contract");
    }
    detail.value = latest;
    const index = traces.value.findIndex((item) => item.scanId === scanId);
    if (index >= 0) traces.value[index] = latest.summary;
  }

  async function selectTrace(scanId: string) {
    if (disposed) return;
    const requestRevision = ++revision;
    pendingLiveRefresh = false;
    selectedId.value = scanId;
    detail.value = undefined;
    detailLoading.value = Boolean(scanId);
    if (!scanId) return;
    try {
      const latest = await sentinelApi.getAgentTrace(scanId);
      if (current(requestRevision)) publish(latest, scanId);
    } catch {
      if (current(requestRevision)) reportError("运行轨迹读取失败，请重试");
    } finally {
      if (current(requestRevision)) {
        detailLoading.value = false;
        flushPendingLiveRefresh();
      }
    }
  }

  async function refreshLiveTrace(eventDriven = false) {
    if (!disposed && eventDriven && (detailLoading.value || pollInFlight)) {
      // The initial/manual read may have started before this committed hint.
      pendingLiveRefresh = true;
      return;
    }
    if (
      disposed || pollInFlight || detailLoading.value || !selectedId.value ||
      (!eventDriven && (!detail.value || !["scanning", "pausing"].includes(detail.value.summary.status)))
    ) return;
    const scanId = selectedId.value;
    const requestRevision = revision;
    pollInFlight = true;
    try {
      const latest = await sentinelApi.getAgentTrace(scanId);
      if (current(requestRevision)) publish(latest, scanId);
    } catch {
      // A failed/background read retains the last verified view, without error spam.
    } finally {
      pollInFlight = false;
      flushPendingLiveRefresh();
    }
  }

  onUnmounted(() => {
    disposed = true;
    revision++;
  });
  return { selectedId: readonly(selectedId), detail, detailLoading, selectTrace, refreshLiveTrace };
}
