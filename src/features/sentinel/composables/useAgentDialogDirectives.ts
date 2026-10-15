import { computed, ref, watch, type Ref } from "vue";
import { sentinelApi } from "../api";
import { orderedAssessmentPlan, orderedAssessmentExecution, orderedPlanCoherent } from "../directives/orderedAssessmentPlan";
import type { AgentTimelineItem, HumanDirectiveDraft, NativeScanStatus } from "../types";

type TaskView = { scanId: string; isCurrent: () => boolean };
type HistoricalReceipt = NonNullable<NativeScanStatus["historicalReceiptItems"]>[number];

// IPC types are not runtime validation. A creation receipt may be redacted, but
// its identity, display fields and pending/rejected state must remain verifiable.
function validDraftReceipt(value: unknown, scanId: string, attempt: number, thread: string): value is HumanDirectiveDraft {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const row = value as Record<string, unknown>;
  const strings = ["id", "sourceMessageId", "rootRunId", "targetKey", "recipientRole", "draftHash"];
  const textFields = ["text", "intent", "safeExecutionText", "confirmedDirectiveId", "status",
    "validationResult", "coordinatorDecision", "sideEffectClass"];
  const arrays = ["requestedRoles", "referencedFactIds", "requestedContracts", "priorityChanges",
    "requiredApprovals", "reasonCodes"];
  // A creation acknowledgement cannot already carry execution authority or
  // contradict its rejection. Never consume input on an ambiguous receipt.
  const coherent = row.confirmedDirectiveId === "" && (row.status === "rejected"
    ? row.validationResult === "rejected" && row.coordinatorDecision === "reject" && row.confirmationRequired === false
    : row.validationResult !== "rejected" && row.coordinatorDecision !== "reject"
      && row.confirmationRequired === true && row.sideEffectClass !== "irreversible_blocked");
  return row.scanId === scanId && row.attemptNumber === attempt && row.threadKey === thread
    && coherent && orderedPlanCoherent(row)
    && strings.every((key) => typeof row[key] === "string" && (row[key] as string).trim().length > 0)
    && textFields.every((key) => typeof row[key] === "string")
    && arrays.every((key) => Array.isArray(row[key]) && (row[key] as unknown[]).every((item) => typeof item === "string"))
    && ["revision", "estimatedTokens", "estimatedRequests"].every((key) =>
      Number.isSafeInteger(row[key]) && (row[key] as number) >= (key === "revision" ? 1 : 0))
    && ["drafted", "need_confirmation", "rejected"].includes(row.status as string)
    && ["valid", "confirmation_required", "rejected"].includes(row.validationResult as string)
    && ["accept", "partially_accept", "defer", "reject", "need_confirmation"].includes(row.coordinatorDecision as string)
    && ["read_only", "controlled_write", "irreversible_blocked"].includes(row.sideEffectClass as string)
    && typeof row.confirmationRequired === "boolean"
    && (row.proposedScopeChange == null || typeof row.proposedScopeChange === "string");
}

// User input and local receipt settlement share the same view fencing, but
// neither a draft nor a read-only historical receipt grants execution authority.
export function useAgentDialogDirectives(options: {
  projectId: () => number | undefined;
  scanId: Ref<string>;
  state: Ref<NativeScanStatus | undefined>;
  threadFilter: Ref<string>;
  captureTaskView: () => TaskView;
  isDisposed: () => boolean;
  loadStatus: () => Promise<void>;
  tr: (zh: string, en: string) => string;
}) {
  const { projectId, scanId, state, threadFilter, captureTaskView, isDisposed, loadStatus, tr } = options;
  const draft = ref("");
  const actionError = ref("");
  const sending = ref(false);
  const selectedHistoricalReceipt = ref("");
  const reviewingDraft = ref<HumanDirectiveDraft>();
  const reviewKind = ref<"revise" | "reject">("revise");
  const reviewText = ref("");
  const transientRejectedDraft = ref<HumanDirectiveDraft>();
  let operationSerial = 0;
  const composerKey = () => JSON.stringify([projectId(), scanId.value, threadFilter.value || "team"]);
  let activeComposerKey = composerKey();
  let restoringComposer = false;
  // Each edit creates a new identity, even when text is changed back. Switching
  // views preserves that identity so a receipt settles only its submitted edit.
  const unsentDrafts = new Map<string, { text: string }>();
  const directiveDrafts = computed(() => {
    const persisted = state.value?.directiveDrafts ?? [];
    const rejected = transientRejectedDraft.value;
    return rejected && !persisted.some((item) => item.id === rejected.id)
      ? [...persisted, rejected] : persisted;
  });

  function beginOperation() {
    const view = captureTaskView();
    const serial = ++operationSerial;
    sending.value = true;
    return { ...view, isCurrent: () => view.isCurrent() && serial === operationSerial };
  }

  function directiveActionError(reason: unknown, fallback: string): string {
    // Classify known rejections internally, never display IPC exception text.
    // Unknown failures do not prove whether a server-side write committed.
    const message = String(reason);
    if (message.includes("directive_thread_coordinator_ambiguous"))
      return tr("这个频道对应多个目标，请先选择具体目标或任务线程，再发送；输入已保留。", "This channel has multiple targets. Select a target or task thread before sending; your input is preserved.");
    if (message.includes("directive_thread_has_no_coordinator"))
      return tr("这个线程当前没有可接收消息的 Coordinator，请刷新任务状态并选择有效线程；输入已保留。", "This thread has no available Coordinator. Refresh the task and select a valid thread; your input is preserved.");
    if (message.includes("directive_draft_recipient_not_bound"))
      return tr("草案未绑定有效接收方或执行租约，请在目标可接收指令后重新创建并确认草案；记录保留。", "This draft has no valid recipient or execution lease. Create and confirm a new draft when the target can accept instructions; the record is retained.");
    if (message.includes("directive_draft_stale_fencing_token"))
      return tr("接收方的执行租约已变化，不能沿用这份草案。请刷新状态后重新创建并确认；没有新增执行。", "The recipient's execution lease has changed. Refresh, then create and confirm a new draft; no new work was queued.");
    return fallback;
  }

  function currentDraft(item: HumanDirectiveDraft): HumanDirectiveDraft | undefined {
    const current = state.value;
    if (!item || !current || current.scanId !== scanId.value || item.scanId !== current.scanId
      || !Number.isSafeInteger(current.attemptNumber) || current.attemptNumber < 1
      || item.attemptNumber !== current.attemptNumber
      || !["drafted", "need_confirmation"].includes(item.status)
      || typeof item.id !== "string" || !item.id.trim()
      || !Number.isSafeInteger(item.revision) || item.revision < 1
      || typeof item.draftHash !== "string" || !item.draftHash.trim()
      || !Array.isArray(current.directiveDrafts)) return;
    const matches = current.directiveDrafts.filter((row) => row?.id === item.id);
    if (matches.length !== 1) return;
    const row = matches[0];
    return row.scanId === item.scanId && row.attemptNumber === item.attemptNumber
      && row.status === item.status && row.revision === item.revision && row.draftHash === item.draftHash
      ? row : undefined;
  }

  function canConfirmDirective(item: HumanDirectiveDraft): boolean {
    const row = currentDraft(item);
    // Compare the displayed version, not object identity: an unchanged polling
    // projection is still reviewable. The backend remains the authority at commit.
    return !!row && typeof item.threadKey === "string" && !!item.threadKey.trim()
      && validDraftReceipt(item, scanId.value, row.attemptNumber, row.threadKey)
      && validDraftReceipt(row, scanId.value, row.attemptNumber, row.threadKey)
      && ["rootRunId", "targetKey", "recipientRole", "sourceMessageId", "text", "intent", "safeExecutionText",
        "requestedRoles", "referencedFactIds", "requestedContracts", "priorityChanges", "proposedScopeChange",
        "coordinatorDecision", "validationResult", "sideEffectClass", "requiredApprovals", "reasonCodes",
        "estimatedTokens", "estimatedRequests", "readonlyAssessmentPlan", "confirmationRequired"].every((key) =>
        JSON.stringify(item[key as keyof HumanDirectiveDraft]) === JSON.stringify(row[key as keyof HumanDirectiveDraft]));
  }

  // Cancellation needs a current identity, not executable recipient bindings;
  // otherwise historical unbound drafts could never be dismissed.
  const canCancelDirective = (item: HumanDirectiveDraft) => !!currentDraft(item);
  function reportStaleDraft() {
    actionError.value = tr("草案已变化或无法核实，请刷新后重新核对；本次未提交操作。", "The draft changed or could not be verified. Refresh and review it again; no action was submitted.") + " [directive_draft_not_current]";
  }

  async function send() {
    const text = draft.value.trim();
    if (isDisposed() || !scanId.value || !text || sending.value) return;
    const current = state.value;
    if (!current || current.scanId !== scanId.value || !Number.isSafeInteger(current.attemptNumber) || current.attemptNumber < 1) {
      actionError.value = tr("任务状态尚未核实，请先刷新；输入已保留。", "Task status is unavailable. Refresh first; your input is preserved.") + " [directive_status_unavailable]";
      return;
    }
    const attempt = current.attemptNumber;
    const thread = threadFilter.value || "team";
    const operation = beginOperation();
    const submittedKey = activeComposerKey;
    const submittedDraft = unsentDrafts.get(submittedKey);
    try {
      const result = await sentinelApi.draftScanDirective(operation.scanId, text, thread);
      if (!operation.isCurrent()) return;
      if (!validDraftReceipt(result, operation.scanId, attempt, thread)) {
        actionError.value = tr("草案回执无法核实，输入已保留；请先刷新核对是否已有草案，不要直接重复提交。", "The draft receipt could not be verified. Your input is preserved; refresh to check for an existing draft before resubmitting.") + " [directive_draft_invalid_response]";
        return;
      }
      transientRejectedDraft.value = result.status === "rejected" ? result : undefined;
      if (result.status !== "rejected" && submittedDraft && unsentDrafts.get(submittedKey) === submittedDraft) {
        unsentDrafts.delete(submittedKey);
        if (activeComposerKey === submittedKey) draft.value = "";
      }
      actionError.value = result.status === "rejected"
        ? tr("这条消息未进入执行队列，请按草案卡中的原因修改。", "This message was not queued. Review the draft reasons and revise it.") : "";
      await loadStatus();
    } catch (reason) {
      if (operation.isCurrent()) actionError.value = directiveActionError(reason,
        tr("草案提交结果无法核实，输入已保留；请刷新核对后再操作，不要直接重复提交。", "The draft submission outcome could not be verified. Your input is preserved; refresh and check before resubmitting."));
    } finally {
      if (operation.isCurrent()) sending.value = false;
    }
  }

  async function confirmDirective(item: HumanDirectiveDraft) {
    if (isDisposed() || !item || item.scanId !== scanId.value || item.attemptNumber !== state.value?.attemptNumber
      || item.status === "rejected" || sending.value) return;
    if (!canConfirmDirective(item)) { reportStaleDraft(); return; }
    const operation = beginOperation();
    try {
      await sentinelApi.confirmScanDirective(operation.scanId, item.id, item.revision, item.draftHash);
      if (!operation.isCurrent()) return;
      actionError.value = "";
      await loadStatus();
    } catch (reason) {
      if (operation.isCurrent()) actionError.value = directiveActionError(reason,
        tr("确认结果无法核实；请刷新核对草案和任务状态，不要直接重复确认。", "The confirmation outcome could not be verified. Refresh the draft and task status before confirming again."));
    } finally {
      if (operation.isCurrent()) sending.value = false;
    }
  }

  async function cancelDirective(item: HumanDirectiveDraft) {
    if (isDisposed() || !item || item.scanId !== scanId.value || item.attemptNumber !== state.value?.attemptNumber) return;
    if (item.status === "rejected" || sending.value) {
      if (transientRejectedDraft.value?.id === item.id) transientRejectedDraft.value = undefined;
      return;
    }
    if (!canCancelDirective(item)) { reportStaleDraft(); return; }
    const operation = beginOperation();
    try {
      await sentinelApi.cancelScanDirective(operation.scanId, item.id, item.revision, item.draftHash);
      if (!operation.isCurrent()) return;
      actionError.value = "";
      await loadStatus();
    } catch {
      if (operation.isCurrent()) actionError.value = tr("取消结果无法核实；请刷新核对草案状态后再操作。", "The cancellation outcome could not be verified. Refresh and check the draft status before acting again.");
    } finally {
      if (operation.isCurrent()) sending.value = false;
    }
  }

  function closeReview() { reviewingDraft.value = undefined; reviewText.value = ""; }
  function beginReview(item: HumanDirectiveDraft, kind: "revise" | "reject") {
    if (isDisposed() || sending.value || !canConfirmDirective(item)) return;
    reviewingDraft.value = item; reviewKind.value = kind;
    reviewText.value = kind === "revise" ? item.text : "";
  }
  async function submitReview() {
    const item = reviewingDraft.value;
    const text = reviewText.value.trim();
    const kind = reviewKind.value;
    if (isDisposed() || sending.value || !item || !text) return;
    if (!canConfirmDirective(item)) { reportStaleDraft(); return; }
    const operation = beginOperation();
    try {
      const result = kind === "revise"
        ? await sentinelApi.reviseScanDirective(operation.scanId, item.id, item.revision, item.draftHash, text)
        : await sentinelApi.rejectScanDirective(operation.scanId, item.id, item.revision, item.draftHash, text);
      if (!operation.isCurrent()) return;
      const receipt = result?.receipt;
      const next = result?.draft;
      const valid = receipt?.kind === kind && receipt.draftId === item.id
        && receipt.scanId === item.scanId && receipt.attemptNumber === item.attemptNumber
        && receipt.rootRunId === item.rootRunId && receipt.targetKey === item.targetKey
        && receipt.threadKey === item.threadKey && receipt.revision === item.revision
        && receipt.draftHash === item.draftHash && receipt.terminal === true && receipt.executionCompleted === false
        && typeof receipt.receiptId === "string" && !!receipt.receiptId.trim()
        && typeof receipt.argumentHash === "string" && /^[a-f0-9]{64}$/.test(receipt.argumentHash)
        && typeof receipt.createdAt === "string" && !!receipt.createdAt.trim()
        && Number.isSafeInteger(receipt.sequence) && receipt.sequence > 0 && receipt.directiveId === ""
        && typeof receipt.reason === "string" && Array.isArray(receipt.actions)
        && receipt.actions.length === item.requestedRoles.length
        && receipt.actions.every((action, index) => action && typeof action === "object" && action.order === index + 1
          && action.role === item.requestedRoles[index] && action.reviewDisposition === "not_queued"
          && action.intent === item.intent && action.reasonCode === "human_decision_terminal"
          && action.capabilityState === "blocked" && action.directiveId === ""
          && action.executionState === "not_started" && action.executionReceipt === null)
        && (kind === "reject" ? next == null && receipt.successorDraftId === null
          && receipt.successorRevision === null && receipt.successorHash === null
          && receipt.requiresConfirmation === false && !!receipt.reason.trim()
          : !!next && next.id !== item.id && next.revision === item.revision + 1 && next.draftHash !== item.draftHash
            && next.rootRunId === item.rootRunId && next.targetKey === item.targetKey && next.recipientRole === item.recipientRole
            && next.sourceMessageId !== item.sourceMessageId
            && receipt.successorDraftId === next.id && receipt.successorRevision === next.revision
            && receipt.successorHash === next.draftHash && receipt.requiresConfirmation === next.confirmationRequired
            && validDraftReceipt(next, item.scanId, item.attemptNumber, item.threadKey));
      if (!valid) {
        actionError.value = tr("判断回执无法核实，请保留输入并刷新核对；不要重复提交。", "The decision receipt could not be verified. Keep your input and refresh before resubmitting.");
        return;
      }
      closeReview(); actionError.value = ""; await loadStatus();
    } catch (reason) {
      if (operation.isCurrent()) actionError.value = directiveActionError(reason,
        tr("判断提交结果无法核实，输入已保留；已确认或入队的请求不能在这里撤销，请刷新核对。", "The decision outcome is unverified and your input is preserved. Confirmed or queued requests cannot be revoked here; refresh and check."));
    } finally { if (operation.isCurrent()) sending.value = false; }
  }

  async function reconcileReceipt(item: AgentTimelineItem) {
    const attempt = state.value?.attemptNumber;
    if (isDisposed() || sending.value || !scanId.value || !attempt || item.localReconciliation
      || item.taskClosure?.disposition !== "receipt_pending" || item.proposalAction?.state !== "received"
      || !state.value?.timeline.some((current) => current.id === item.id && current === item)) return;
    const operation = beginOperation();
    try {
      await sentinelApi.reconcileScanDirectiveReceipt(operation.scanId, attempt, item.id);
      if (!operation.isCurrent()) return;
      actionError.value = "";
      await loadStatus();
    } catch {
      if (operation.isCurrent()) actionError.value = tr("本地回执核对结果无法确认；请刷新后核对记录。本操作不会重新执行任务。", "The local reconciliation outcome could not be verified. Refresh and check the record; this action does not re-execute the task.");
    } finally {
      if (operation.isCurrent()) sending.value = false;
    }
  }

  async function reconcileHistoricalReceipt(item: HistoricalReceipt) {
    const current = state.value;
    const key = `${item.attemptNumber}:${item.directiveId}`;
    if (isDisposed() || sending.value || !current || current.scanId !== scanId.value
      || selectedHistoricalReceipt.value !== key || item.verified !== false
      || !Number.isSafeInteger(item.attemptNumber) || item.attemptNumber < 1
      || item.attemptNumber >= current.attemptNumber || !item.directiveId
      || !current.historicalReceiptItems?.some((row) => row.directiveId === item.directiveId
        && row.attemptNumber === item.attemptNumber && row.verified === false)) return;
    const operation = beginOperation();
    const currentAttempt = current.attemptNumber;
    try {
      await sentinelApi.reconcileHistoricalScanDirectiveReceipt(operation.scanId, item.attemptNumber, item.directiveId);
      if (!operation.isCurrent() || state.value?.attemptNumber !== currentAttempt) return;
      selectedHistoricalReceipt.value = "";
      actionError.value = "";
      await loadStatus();
    } catch {
      if (operation.isCurrent() && state.value?.attemptNumber === currentAttempt)
        actionError.value = tr("历史回执核对结果无法确认；请重新读取旧记录。本操作不会重新执行任务。", "The historical reconciliation outcome could not be verified. Reload the old record; this action does not re-execute the task.");
    } finally {
      if (operation.isCurrent()) sending.value = false;
    }
  }

  watch(() => state.value?.attemptNumber, () => {
    // A lost snapshot fences pending callbacks even if the same attempt returns.
    operationSerial++;
    sending.value = false;
    transientRejectedDraft.value = undefined;
    closeReview();
    actionError.value = "";
    selectedHistoricalReceipt.value = "";
  }, { flush: "sync" });
  watch(draft, (text) => {
    if (restoringComposer) return;
    if (text) unsentDrafts.set(activeComposerKey, { text });
    else unsentDrafts.delete(activeComposerKey);
  }, { flush: "sync" });
  watch(composerKey, (key) => {
    // Review editors belong to this exact project/task/thread view. Preserve
    // the existing send cache settlement when no review is in progress.
    if (reviewingDraft.value) {
      operationSerial++;
      sending.value = false;
      actionError.value = "";
    }
    closeReview();
    activeComposerKey = key;
    restoringComposer = true;
    try { draft.value = unsentDrafts.get(key)?.text || ""; }
    finally { restoringComposer = false; }
  }, { flush: "sync" });

  function resetForTask() {
    closeReview();
    operationSerial++;
    sending.value = false;
    actionError.value = "";
    selectedHistoricalReceipt.value = "";
    transientRejectedDraft.value = undefined;
  }

  return { draft, actionError, sending, selectedHistoricalReceipt, directiveDrafts, orderedAssessmentPlan, orderedAssessmentExecution,
    reviewingDraft, reviewKind, reviewText, beginReview, closeReview, submitReview,
    send, canConfirmDirective, canCancelDirective, confirmDirective, cancelDirective,
    reconcileReceipt, reconcileHistoricalReceipt, resetForTask };
}
