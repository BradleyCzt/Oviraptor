import { computed, ref, watch, type Ref } from "vue";
import { sentinelApi } from "../api";
import { useAgentDialogHistory } from "./useAgentDialogHistory";
import { timelineIdentity as timelineKey } from "../timeline/projectionContract";
import { humanAssessmentRows } from "../timeline/humanAssessmentContract";
import type { AgentDialogView, AgentTimelineItem, NativeScanStatus } from "../types";

type TaskView = { scanId: string; isCurrent: () => boolean };
const threadKey = (item: AgentTimelineItem) => item.threadKey || item.correlationId
  || item.assignmentId || item.targetKey || "team";

// A reading cursor is a view preference, not mailbox delivery, execution ack,
// or permission to mutate the investigation. Only loaded messages may be read.
export function useAgentDialogReading(options: {
  projectId: () => number | undefined;
  scanId: Ref<string>;
  state: Ref<NativeScanStatus | undefined>;
  captureTaskView: () => TaskView;
  isDisposed: () => boolean;
  tr: (zh: string, en: string) => string;
}) {
  const { projectId, scanId, state, captureTaskView, isDisposed, tr } = options;
  const threadFilter = ref("");
  const dialogView = ref<AgentDialogView>();
  const dialogViewError = ref("");
  const dialogViewBusy = ref(false);
  let dialogViewSerial = 0;
  const history = useAgentDialogHistory({ scanId, state, captureTaskView, isDisposed, tr });

  const threadOptions = computed(() => {
    const counts = new Map<string, number>();
    const labels = new Map<string, string>();
    // Only server-projected recipients may introduce an empty target thread.
    for (const recipient of state.value?.directiveRecipients ?? []) {
      const key = recipient.threadKey || recipient.targetKey;
      if (key) {
        counts.set(key, 0);
        labels.set(key, recipient.targetKey || key);
      }
    }
    // Original readonly obligations may outlive current recipients. Showing the
    // original thread does not make it an execution recipient or a read event.
    for (const item of humanAssessmentRows(state.value)) {
      const key = item.humanDirective.threadKey;
      if (!counts.has(key)) counts.set(key, 0);
      if (!labels.has(key)) labels.set(key, key.startsWith("coordinator:") ? item.targetKey : key);
    }
    for (const item of [...(state.value?.timeline ?? []), ...(history.page.value?.timeline ?? [])]) {
      const key = threadKey(item);
      counts.set(key, (counts.get(key) || 0) + 1);
      if (!labels.has(key) && key.startsWith("coordinator:") && item.targetKey) labels.set(key, item.targetKey);
    }
    return [...counts.entries()].map(([key, count]) => ({ key, count, label: labels.get(key) || key }));
  });
  const visibleTimeline = computed(() => (state.value?.timeline ?? []).filter((item) =>
    !threadFilter.value || threadKey(item) === threadFilter.value));
  // Bound DOM work without discarding evidence. New messages do not displace
  // an anchored history page; only explicit navigation changes the anchor.
  const timelinePageSize = 100;
  const timelinePageAnchor = ref<string>();
  const timelinePageStart = computed(() => {
    if (history.page.value) return 0;
    const items = visibleTimeline.value;
    const anchored = timelinePageAnchor.value === undefined ? -1
      : items.findIndex((item) => timelineKey(item) === timelinePageAnchor.value);
    return anchored < 0 ? Math.max(0, items.length - timelinePageSize) : anchored;
  });
  const timelinePage = computed(() => history.page.value
    ? history.page.value.timeline.filter((item) => !threadFilter.value || threadKey(item) === threadFilter.value)
    : visibleTimeline.value.slice(timelinePageStart.value, timelinePageStart.value + timelinePageSize));
  watch(() => ({ project: projectId(), scan: scanId.value, attempt: state.value?.attemptNumber,
    thread: threadFilter.value, items: visibleTimeline.value }), (next, previous) => {
    if (history.page.value || timelinePageAnchor.value === undefined
      || next.project !== previous.project || next.scan !== previous.scan
      || next.attempt !== previous.attempt || next.thread !== previous.thread) return;
    const start = previous.items.findIndex((item) => timelineKey(item) === timelinePageAnchor.value);
    if (start < 0) return;
    const shown = previous.items.slice(start, start + timelinePageSize);
    const loaded = new Set(next.items.map(timelineKey));
    if (shown.some((item) => !loaded.has(timelineKey(item)))) history.retainLocalPage(shown);
  }, { flush: "sync" });
  function showEarlierMessages() {
    if (history.page.value || timelinePageStart.value === 0) { history.showEarlier(); return; }
    const item = visibleTimeline.value[Math.max(0, timelinePageStart.value - timelinePageSize)];
    if (item) timelinePageAnchor.value = timelineKey(item);
  }
  function showLaterMessages() {
    if (history.page.value) { history.showLater(); return; }
    const start = timelinePageStart.value + timelinePageSize;
    timelinePageAnchor.value = start >= visibleTimeline.value.length - timelinePageSize
      ? undefined : timelineKey(visibleTimeline.value[start]);
  }
  function showLatestMessages() { history.showLatest(); timelinePageAnchor.value = undefined; }
  watch([projectId, scanId, () => state.value?.attemptNumber],
    showLatestMessages, { flush: "sync" });
  // A thread change resets the local loaded-window slice but keeps an explicit
  // historical page in place so its event can witness an old thread selection.
  watch(threadFilter, () => { timelinePageAnchor.value = undefined; }, { flush: "sync" });
  const selectedThreadUnavailable = computed(() => !!threadFilter.value
    && !threadOptions.value.some((thread) => thread.key === threadFilter.value));
  const selectedThreadLabel = computed(() => threadOptions.value.find((thread) => thread.key === threadFilter.value)?.label || threadFilter.value);
  const visibleUnreadCount = computed(() => dialogView.value ? timelinePage.value.filter((item) => {
    const saved = dialogView.value!;
    const key = threadKey(item);
    const threadCursor = Object.prototype.hasOwnProperty.call(saved.threadReadSequences, key)
      ? saved.threadReadSequences[key] : 0;
    return item.sequence > Math.max(saved.allReadSequence, threadCursor);
  }).length : undefined);
  // The persisted cursor marks a whole prefix, not individual rendered rows.
  // Never advance it over an unloaded window or an unshown row in this window.
  const canMarkPageRead = computed(() => {
    const saved = dialogView.value;
    const window = history.page.value ?? state.value;
    const items = timelinePage.value;
    if (history.retainedLocalPage.value || !saved || !window || !items.length || !visibleUnreadCount.value
      || selectedThreadUnavailable.value) return false;
    const baseline = Math.max(saved.allReadSequence,
      threadFilter.value ? saved.threadReadSequences[threadFilter.value] ?? 0 : 0);
    const cursor = Math.max(...items.map((item) => item.sequence));
    const first = window.timelineBeforeSequence;
    if (typeof first !== "number" || !Number.isSafeInteger(first) || first < 1 || (window.hasEarlierTimeline !== false
      && baseline < first - 1)) return false;
    const shown = new Set(items.map(timelineKey));
    const loaded = history.page.value ? history.page.value.timeline : visibleTimeline.value;
    return !loaded.some((item) => (!threadFilter.value || threadKey(item) === threadFilter.value)
      && item.sequence > baseline && item.sequence <= cursor && !shown.has(timelineKey(item)));
  });

  function validDialogView(value: AgentDialogView, attempt: number): boolean {
    const integer = (n: number) => Number.isSafeInteger(n) && n >= 0;
    const max = state.value?.latestSequence ?? 0;
    return value?.scanId === scanId.value && value.attemptNumber === attempt
      && integer(value.revision) && typeof value.selectedThread === "string"
      && value.selectedThread.length <= 2048 && integer(value.allReadSequence)
      && value.allReadSequence <= max && !!value.threadReadSequences
      && typeof value.threadReadSequences === "object" && !Array.isArray(value.threadReadSequences)
      && Object.entries(value.threadReadSequences).length <= 512
      && Object.entries(value.threadReadSequences).every(([key, cursor]) =>
        !!key && key.length <= 2048 && integer(cursor) && cursor <= max);
  }

  // A successful CAS receipt must reflect only this reading operation. Shape
  // validation alone would allow a thread selection to mark other messages read.
  function matchesReadingUpdate(previous: AgentDialogView, result: AgentDialogView,
    selectedThread: string, readThrough?: number): boolean {
    let allReadSequence = previous.allReadSequence;
    const threads = new Map(Object.entries(previous.threadReadSequences));
    if (readThrough !== undefined) {
      if (!selectedThread) {
        allReadSequence = Math.max(allReadSequence, readThrough);
        for (const [key, cursor] of threads) if (cursor <= allReadSequence) threads.delete(key);
      } else if (readThrough > allReadSequence) {
        threads.set(selectedThread, Math.max(threads.get(selectedThread) ?? 0, readThrough));
      }
    }
    const received = Object.entries(result.threadReadSequences);
    return result.allReadSequence === allReadSequence && received.length === threads.size
      && received.every(([key, cursor]) => threads.get(key) === cursor);
  }

  async function loadDialogView() {
    const attempt = state.value?.attemptNumber;
    if (!attempt || isDisposed()) return;
    const view = captureTaskView();
    const serial = ++dialogViewSerial;
    const current = () => view.isCurrent() && serial === dialogViewSerial
      && state.value?.attemptNumber === attempt;
    dialogViewBusy.value = true;
    dialogViewError.value = "";
    try {
      const result = await sentinelApi.getAgentDialogView(view.scanId, attempt);
      if (!current()) return;
      if (!validDialogView(result, attempt)) throw new Error("dialog_view_invalid_response");
      dialogView.value = result;
      threadFilter.value = result.selectedThread;
    } catch {
      if (current()) {
        dialogView.value = undefined;
        dialogViewError.value = tr("阅读偏好读取失败，请重新读取。", "Unable to read reading preferences. Please reload.");
      }
    } finally {
      if (current()) dialogViewBusy.value = false;
    }
  }

  async function saveDialogView(markRead = false) {
    const persisted = dialogView.value;
    const attempt = state.value?.attemptNumber;
    if (!persisted || !attempt || dialogViewBusy.value || dialogViewError.value
      || (markRead && !canMarkPageRead.value)) return;
    const view = captureTaskView();
    const serial = ++dialogViewSerial;
    const current = () => view.isCurrent() && serial === dialogViewSerial
      && state.value?.attemptNumber === attempt;
    const selectedThread = threadFilter.value;
    const historicalWitness = selectedThread && history.page.value?.timeline.find((item) =>
      threadKey(item) === selectedThread && Number.isSafeInteger(item.sequence) && item.sequence > 0);
    const cursor = timelinePage.value.reduce((max, item) =>
      Number.isSafeInteger(item.sequence) && item.sequence > max ? item.sequence : max, 0);
    dialogViewBusy.value = true;
    try {
      const result = await sentinelApi.saveAgentDialogView({ scanId: view.scanId,
        attemptNumber: attempt, expectedRevision: persisted.revision, selectedThread,
        ...(markRead ? { markReadThrough: cursor } : {}),
        ...(historicalWitness ? { selectedThreadSequence: historicalWitness.sequence } : {}),
      });
      if (!current()) return;
      if (!validDialogView(result, attempt) || result.revision !== persisted.revision + 1
        || result.selectedThread !== selectedThread
        || !matchesReadingUpdate(persisted, result, selectedThread, markRead ? cursor : undefined))
        throw new Error("dialog_view_invalid_response");
      dialogView.value = result;
    } catch {
      if (current()) dialogViewError.value = tr("阅读偏好保存失败，请重新读取后再操作。", "Unable to save reading preferences. Reload before trying again.");
    } finally {
      if (current()) dialogViewBusy.value = false;
    }
  }

  watch([projectId, scanId, () => state.value?.attemptNumber], () => {
    dialogViewSerial++;
    dialogView.value = undefined;
    dialogViewError.value = "";
    dialogViewBusy.value = false;
    threadFilter.value = "";
    void loadDialogView();
  }, { flush: "sync" });

  return { threadFilter, dialogView, dialogViewError, dialogViewBusy, threadOptions,
    visibleTimeline, timelinePageSize, timelinePageAnchor, timelinePageStart, timelinePage,
    historyPage: history.page, historyBusy: history.busy, historyError: history.error,
    historyHasEarlier: history.hasEarlier, historyRetainedLocalPage: history.retainedLocalPage,
    showEarlierMessages, showLaterMessages, showLatestMessages, selectedThreadUnavailable,
    selectedThreadLabel, visibleUnreadCount, canMarkPageRead, loadDialogView, saveDialogView };
}
