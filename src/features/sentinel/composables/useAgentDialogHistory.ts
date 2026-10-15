import { computed, ref, watch, type Ref } from "vue";
import { sentinelApi } from "../api";
import { validTimelineRows } from "../timeline/projectionContract";
import type { AgentTimelineItem, AgentTimelinePage, NativeScanStatus } from "../types";

type TaskView = { scanId: string; isCurrent: () => boolean };

// One history page at a time; the stack contains only cursor numbers, never
// previous message payloads. History reads are not mailbox/read acknowledgements.
export function useAgentDialogHistory(options: {
  scanId: Ref<string>;
  state: Ref<NativeScanStatus | undefined>;
  captureTaskView: () => TaskView;
  isDisposed: () => boolean;
  tr: (zh: string, en: string) => string;
}) {
  const { scanId, state, captureTaskView, isDisposed, tr } = options;
  const page = ref<AgentTimelinePage>();
  const retainedLocalPage = ref(false);
  const busy = ref(false);
  const error = ref("");
  const cursors = ref<number[]>([]);
  let serial = 0;
  const hasEarlier = computed(() => page.value?.hasEarlierTimeline ?? state.value?.hasEarlierTimeline ?? false);
  const beforeSequence = computed(() => page.value?.timelineBeforeSequence
    ?? state.value?.timelineBeforeSequence
    ?? Math.min(...(state.value?.timeline ?? []).map((item) => item.sequence).filter((n) => n > 0), Infinity));

  function validPage(result: AgentTimelinePage, scan: string, attempt: number, before: number): boolean {
    const positive = (value: number) => Number.isSafeInteger(value) && value > 0;
    return result?.scanId === scan && result.attemptNumber === attempt
      && result.beforeSequence === before && Number.isSafeInteger(result.timelineBeforeSequence)
      && result.timelineBeforeSequence >= 0 && result.timelineBeforeSequence < before
      && typeof result.hasEarlierTimeline === "boolean" && validTimelineRows(result.timeline)
      && result.timeline.length <= 100 && (!result.hasEarlierTimeline || positive(result.timelineBeforeSequence))
      // The cursor covers persisted events, including superseded entities not
      // projected as messages. Sparse/empty pages and timestamp order are valid.
      && (!result.timeline.length || positive(result.timelineBeforeSequence))
      && result.timeline.every((item) => item.sequence < before && item.sequence >= result.timelineBeforeSequence);
  }

  async function readPage(before: number, nextCursors: number[]) {
    const attempt = state.value?.attemptNumber;
    if (!attempt || !Number.isSafeInteger(before) || before <= 0 || busy.value || isDisposed()) return;
    const view = captureTaskView();
    const request = ++serial;
    const current = () => view.isCurrent() && request === serial && state.value?.attemptNumber === attempt;
    busy.value = true;
    error.value = "";
    try {
      const result = await sentinelApi.getNativeScanTimelinePage(view.scanId, attempt, before);
      if (!current()) return;
      if (!validPage(result, view.scanId, attempt, before)) throw new Error("timeline_page_invalid_response");
      page.value = result;
      retainedLocalPage.value = false;
      cursors.value = nextCursors;
    } catch {
      if (current()) error.value = tr("历史消息读取失败，请重试。", "Unable to read message history. Please retry.");
    } finally {
      if (current()) busy.value = false;
    }
  }

  function showEarlier() {
    const before = beforeSequence.value;
    if (!hasEarlier.value || !Number.isSafeInteger(before) || before <= 0) return;
    void readPage(before, [...cursors.value, before]);
  }

  function showLater() {
    if (busy.value || cursors.value.length === 0) return;
    if (cursors.value.length === 1) { showLatest(); return; }
    const previous = cursors.value.slice(0, -1);
    void readPage(previous[previous.length - 1], previous);
  }

  function showLatest() {
    serial++;
    page.value = undefined;
    retainedLocalPage.value = false;
    cursors.value = [];
    busy.value = false;
    error.value = "";
  }

  // Preserve only the rendered page when the recent cache evicts its rows.
  // This snapshot is not a server-certified contiguous read window.
  function retainLocalPage(items: AgentTimelineItem[]) {
    const attempt = state.value?.attemptNumber;
    if (!attempt || !items.length || items.length > 100 || isDisposed()) return;
    const first = Math.min(...items.map((item) => item.sequence));
    const before = Math.max(...items.map((item) => item.sequence)) + 1;
    if (!Number.isSafeInteger(first) || first < 1 || !Number.isSafeInteger(before)) return;
    showLatest();
    page.value = { scanId: scanId.value, attemptNumber: attempt, beforeSequence: before,
      timelineBeforeSequence: first, hasEarlierTimeline: first > 1, timeline: items.slice() };
    retainedLocalPage.value = true;
    cursors.value = [before];
  }

  watch([scanId, () => state.value?.attemptNumber], showLatest, { flush: "sync" });
  return { page, retainedLocalPage, busy, error, cursors, hasEarlier,
    showEarlier, showLater, showLatest, retainLocalPage };
}
