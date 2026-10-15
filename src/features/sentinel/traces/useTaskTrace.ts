import { onUnmounted, ref, watch } from "vue";
import type { AgentTraceDetail } from "../types";
import { isTraceDetailForTask } from "./traceDetailContract";

// Read-only Board state. No execution, polling or automatic retry is added.
export function useTaskTrace(options: {
  scanId: () => string | undefined;
  projectId: () => number | undefined;
  read: (scanId: string) => Promise<AgentTraceDetail>;
  notify: (message: string) => void;
}) {
  const detail = ref<AgentTraceDetail>();
  const busy = ref(false);
  let generation = 0;
  let disposed = false;

  function reset() {
    ++generation;
    detail.value = undefined;
    busy.value = false;
  }
  watch([options.scanId, options.projectId], reset, { flush: "sync" });

  async function load(scanId: string, notify = false) {
    if (disposed || !scanId || options.scanId() !== scanId) return;
    const request = ++generation;
    const project = options.projectId();
    const current = () => !disposed && request === generation
      && options.scanId() === scanId && options.projectId() === project;
    busy.value = true;
    try {
      const next = await options.read(scanId);
      if (!current()) return;
      if (!isTraceDetailForTask(next, scanId)) throw new Error("Invalid trace display contract");
      detail.value = next;
    } catch {
      if (current()) {
        detail.value = undefined;
        if (notify) options.notify("运行轨迹读取失败，请重试");
      }
    } finally {
      if (current()) busy.value = false;
    }
  }

  onUnmounted(() => { disposed = true; reset(); });
  return { detail, busy, load, reset };
}
