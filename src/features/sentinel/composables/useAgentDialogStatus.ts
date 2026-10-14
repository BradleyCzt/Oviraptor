import { ref, type Ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sentinelApi } from "../api";
import { timelineIdentity, validTimelineRows } from "../timeline/projectionContract";
import { validStatusCursor, validStatusTimeline } from "../timeline/statusContract";
import { validHumanAssessmentSnapshot } from "../timeline/humanAssessmentContract";
import { validCollaborationEvent, type CollaborationEventNotification } from "../timeline/eventContract";
import type { NativeScanStatus, SentinelScan } from "../types";

type TaskView = { scanId: string; isCurrent: () => boolean };
const timelineCacheLimit = 300;

// The event listener is only a hint. The DB snapshot and its watermark remain
// authoritative; neither event delivery nor a late IPC may publish old state.
export function useAgentDialogStatus(options: {
  scanId: Ref<string>;
  selected: Readonly<Ref<SentinelScan | undefined>>;
  captureTaskView: () => TaskView;
  isDisposed: () => boolean;
  loadScans: () => Promise<void>;
  tr: (zh: string, en: string) => string;
  schedule: typeof setTimeout;
  cancel: typeof clearTimeout;
}) {
  const { scanId, selected, captureTaskView, isDisposed, loadScans, tr, schedule, cancel } = options;
  const state = ref<NativeScanStatus>();
  const latestSequence = ref(0);
  const statusError = ref("");
  const listenerError = ref("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let collaborationTimer: ReturnType<typeof setTimeout> | undefined;
  let collaborationRead: object | undefined;
  let collaborationPending: { view: TaskView; attempt?: number; sequence?: number; related: boolean } | undefined;
  let unlistenCollaboration: UnlistenFn | undefined;
  let subscribing = false;
  let disposed = false;
  const statusReads = new Map<number, TaskView>();
  let statusRequestSerial = 0;
  let appliedStatusRequestSerial = 0;
  let invalidatedStatusRequestSerial = 0;

  const visible = () => !disposed && !isDisposed()
    && (typeof document === "undefined" || !document.hidden);
  const currentRead = () => statusReads.get(statusRequestSerial)?.isCurrent() ?? false;

  function scheduleStatusRefresh() {
    cancel(timer);
    timer = undefined;
    if (!visible() || (unlistenCollaboration && (currentRead() || collaborationRead))) return;
    if (collaborationPending && unlistenCollaboration) { scheduleCollaborationRefresh(); return; }
    if (!selected.value && unlistenCollaboration) return;
    const urgent = !unlistenCollaboration || !!statusError.value;
    timer = schedule(() => {
      timer = undefined;
      void reconcileStatus();
    }, !statusError.value && state.value?.timelineHasMore ? 50 : urgent ? 3000 : 15000);
  }

  async function reconcileStatus() {
    if (!visible()) return;
    if (!unlistenCollaboration) await installCollaborationListener();
    if (visible() && !currentRead() && !collaborationPending)
      await loadStatus(state.value ? latestSequence.value : undefined);
  }

  function catchUp() {
    if (!visible() || !scanId.value) return;
    if (!collaborationPending?.view.isCurrent())
      collaborationPending = { view: captureTaskView(), related: true };
    else collaborationPending.related = true;
    cancel(timer);
    timer = undefined;
    scheduleCollaborationRefresh();
  }

  function onVisibility() {
    if (!visible()) {
      cancel(timer);
      timer = undefined;
      cancel(collaborationTimer);
      collaborationTimer = undefined;
      return;
    }
    void installCollaborationListener();
    catchUp();
  }
  if (typeof document !== "undefined") document.addEventListener("visibilitychange", onVisibility);

  function resetCollaborationRefresh() {
    cancel(collaborationTimer);
    collaborationTimer = undefined;
    collaborationPending = undefined;
    // An obsolete IPC cannot unlock or publish into a new view.
    collaborationRead = undefined;
  }

  function scheduleCollaborationRefresh() {
    if (!visible() || currentRead() || collaborationTimer !== undefined || collaborationRead || !collaborationPending) return;
    collaborationTimer = schedule(() => {
      collaborationTimer = undefined;
      void drainCollaborationRefresh();
    }, 50);
  }

  function queueCollaborationRefresh(payload: CollaborationEventNotification, related = false) {
    if (!scanId.value || disposed || isDisposed()) return;
    if (!collaborationPending?.view.isCurrent()) {
      collaborationPending = { view: captureTaskView(), related: false };
    }
    if (related) collaborationPending.related = true;
    else if (collaborationPending.attempt === undefined || payload.attemptNumber > collaborationPending.attempt
      || (payload.attemptNumber === collaborationPending.attempt && payload.sequence > (collaborationPending.sequence ?? 0))) {
      collaborationPending.attempt = payload.attemptNumber;
      collaborationPending.sequence = payload.sequence;
    }
    cancel(timer);
    timer = undefined;
    scheduleCollaborationRefresh();
  }

  async function drainCollaborationRefresh() {
    if (!visible() || currentRead()) return;
    const hint = collaborationPending;
    collaborationPending = undefined;
    if (!hint?.view.isCurrent()) { scheduleStatusRefresh(); return; }
    const selectedChanged = hint.attempt !== undefined && (!state.value
      || hint.attempt > state.value.attemptNumber
      || (hint.attempt === state.value.attemptNumber && (hint.sequence ?? 0) > latestSequence.value));
    // Related tasks have separate watermarks; refresh their persisted relation.
    if (!selectedChanged && !hint.related) { scheduleStatusRefresh(); return; }
    const token = {};
    collaborationRead = token;
    try {
      await loadStatus(selectedChanged && hint.attempt !== state.value?.attemptNumber
        ? undefined : latestSequence.value || undefined);
    } finally {
      if (collaborationRead === token) {
        collaborationRead = undefined;
        scheduleStatusRefresh();
      }
    }
  }

  async function loadStatus(afterSequence?: number, requestedByUser = false) {
    // Native window occlusion can leave a newly selected view hidden before
    // its visibility event arrives. Explicit navigation still needs one DB
    // snapshot; automatic event/poll reads remain visibility gated.
    if (disposed || isDisposed() || (!requestedByUser && !visible())) return;
    cancel(timer);
    timer = undefined;
    if (!scanId.value) { state.value = undefined; return; }
    const view = captureTaskView();
    const requestSerial = ++statusRequestSerial;
    statusReads.set(requestSerial, view);
    // Draft tasks have no attempt yet; the backend rejects attempt-zero deltas.
    if (state.value?.scanId !== view.scanId || state.value.attemptNumber === 0) afterSequence = undefined;
    const expectedAttemptNumber = afterSequence !== undefined ? state.value?.attemptNumber : undefined;
    try {
      const result = await sentinelApi.getNativeScanStatus(view.scanId, afterSequence, expectedAttemptNumber);
      if (!view.isCurrent() || requestSerial < invalidatedStatusRequestSerial) return;
      if (result?.scanId !== view.scanId) throw new Error("scan_status_identity_mismatch");
      if (afterSequence !== undefined && !state.value) {
        await loadStatus();
        return;
      }
      if (!validStatusCursor(result, afterSequence, expectedAttemptNumber))
        throw new Error("scan_status_invalid_cursor");
      if (!validStatusTimeline(result, afterSequence)) throw new Error("scan_status_invalid_timeline");
      if (!validHumanAssessmentSnapshot(result)) throw new Error("scan_status_invalid_human_assessments");
      // DB snapshot order, not invocation order, determines the visible cursor.
      if (state.value?.scanId === result.scanId) {
        if (result.attemptNumber < state.value.attemptNumber) return;
        if (result.attemptNumber === state.value.attemptNumber
          && (result.latestSequence < latestSequence.value
            || (result.latestSequence === latestSequence.value && requestSerial < appliedStatusRequestSerial))) return;
      }
      if (afterSequence !== undefined && result.isIncremental === true
        && state.value?.scanId === result.scanId
        && state.value.attemptNumber === result.attemptNumber) {
        const merged = new Map(state.value.timeline.map((item) => [timelineIdentity(item), item]));
        for (const item of result.timeline) merged.set(timelineIdentity(item), item);
        if (!validTimelineRows([...merged.values()])) throw new Error("scan_status_invalid_timeline");
        const bySequence = [...merged.values()].sort((left, right) => left.sequence - right.sequence);
        const dropped = bySequence.length > timelineCacheLimit;
        const retained = bySequence.slice(-timelineCacheLimit);
        result.timelineBeforeSequence = dropped
          ? retained[0].sequence
          : state.value.timelineBeforeSequence ?? retained[0]?.sequence ?? 0;
        result.hasEarlierTimeline = dropped || !!state.value.hasEarlierTimeline;
        result.timeline = retained.sort((left, right) =>
          left.timestamp.localeCompare(right.timestamp)
            || left.sequence - right.sequence
            || left.id.localeCompare(right.id));
      }
      appliedStatusRequestSerial = Math.max(appliedStatusRequestSerial, requestSerial);
      state.value = result;
      latestSequence.value = result.latestSequence;
      statusError.value = "";
      if (selected.value && selected.value.status !== result.status) void loadScans();
    } catch (reason) {
      if (view.isCurrent() && requestSerial >= appliedStatusRequestSerial
        && requestSerial >= invalidatedStatusRequestSerial) {
        // Classify internally; IPC errors may contain paths, URLs or credentials.
        const detail = String(reason);
        const invalidReceipt = ["request_review_", "administrative_closure_", "closure_handoff_"]
          .some(prefix => detail.includes(prefix));
        statusError.value = invalidReceipt
          ? tr("任务回执校验失败，已清除缓存，请重新读取。", "Task receipt validation failed; cached messages cleared. Reload the task.")
          : tr("任务状态读取失败，请重试。", "Unable to read task status. Please retry.");
        if (invalidReceipt) {
          // Unverifiable operator receipts revoke the cached chat and fence
          // pending older reads instead of leaving false persisted messages.
          invalidatedStatusRequestSerial = requestSerial;
          state.value = undefined;
          latestSequence.value = 0;
        }
      }
    } finally {
      statusReads.delete(requestSerial);
      if (!view.isCurrent() || requestSerial !== statusRequestSerial) return;
      scheduleStatusRefresh();
    }
  }

  async function installCollaborationListener() {
    if (!visible() || unlistenCollaboration || subscribing) return;
    subscribing = true;
    try {
      const stop = await listen<CollaborationEventNotification>("nest://collaboration-event", (event) => {
        if (disposed || isDisposed()) return;
        const payload = event.payload;
        if (!validCollaborationEvent(payload)) return;
        if (payload.scanId !== scanId.value) {
          if (state.value?.followup?.source?.sourceScanId === payload.scanId
            || state.value?.followup?.tasks.some((task) => task.scanId === payload.scanId)) {
            queueCollaborationRefresh(payload, true);
          }
          return;
        }
        if (state.value) {
          if (payload.attemptNumber < state.value.attemptNumber) return;
          if (payload.attemptNumber === state.value.attemptNumber && payload.sequence <= latestSequence.value) return;
        }
        queueCollaborationRefresh(payload);
      });
      if (disposed || isDisposed()) stop();
      else { unlistenCollaboration = stop; listenerError.value = ""; catchUp(); }
    } catch {
      if (!disposed && !isDisposed()) {
        listenerError.value = tr("实时事件连接失败，使用数据库轮询。", "Live events unavailable; using database polling.");
      }
    } finally {
      subscribing = false;
      scheduleStatusRefresh();
    }
  }

  function resetForTask() {
    resetCollaborationRefresh();
    statusError.value = "";
    cancel(timer);
    latestSequence.value = 0;
    state.value = undefined;
    appliedStatusRequestSerial = ++statusRequestSerial;
  }

  function dispose() {
    disposed = true;
    resetCollaborationRefresh();
    cancel(timer);
    timer = undefined;
    unlistenCollaboration?.();
    unlistenCollaboration = undefined;
    if (typeof document !== "undefined") document.removeEventListener("visibilitychange", onVisibility);
  }

  return { state, latestSequence, statusError, listenerError, loadStatus,
    installCollaborationListener, resetForTask, dispose };
}
