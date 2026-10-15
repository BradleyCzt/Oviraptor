import type { OrderedAssessmentPlan, OrderedAssessmentExecution, AgentTimelineItem, NativeScanStatus } from "../types";
const marker = "ordered_readonly_assessment_v2";
const blocked = "proposal_ordered_execution_not_connected";
const liveMarker = "ordered_readonly_assessment_v3";
const dispatchChecks = "ordered_original_dispatch_checks_required";
const hash = (value: unknown) => typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
function exact(value: Record<string, unknown>, keys: string[]) {
  return Object.keys(value).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(value, key));
}

// Structural IPC validation is display protection. Confirm still submits the
// original backend revision/hash and performs its own scope and authority check.
export function orderedAssessmentPlan(draft: unknown): OrderedAssessmentPlan | null {
  if (!object(draft) || !Array.isArray(draft.reasonCodes) || !Array.isArray(draft.requestedRoles)) return null;
  const plan = draft.readonlyAssessmentPlan;
  const live = draft.reasonCodes.includes(liveMarker);
  const versionMarker = live ? liveMarker : marker;
  const required = live ? dispatchChecks : blocked;
  const roles = draft.requestedRoles.filter(role => role !== "coordinator");
  if (draft.reasonCodes.filter(reason => reason === versionMarker).length !== 1
    || (live && draft.reasonCodes.includes(marker)) || !draft.reasonCodes.includes(required) || draft.intent !== "agent_proposal_request"
    || draft.sideEffectClass !== "read_only" || draft.validationResult !== "valid"
    || draft.coordinatorDecision !== "accept" || draft.estimatedTokens !== 8000 || draft.estimatedRequests !== 2
    || !Array.isArray(draft.requestedContracts) || draft.requestedContracts.length !== 0 || draft.proposedScopeChange != null
    || roles.length !== 2 || new Set(roles).size !== 2
    || !roles.includes("spa_api_mapper") || !roles.includes("deep_investigator")
    || draft.requestedRoles.length > 3
    || draft.requestedRoles.filter(role => role === "coordinator").length > 1
    || !object(plan) || !exact(plan, ["schemaVersion", "purpose", "bindingHash", "planHash", "dispatchState",
      "totalTokenCeiling", "totalModelRequests", "targetRequests", "actions"])
    || plan.schemaVersion !== (live ? 3 : 2) || plan.purpose !== "human_readonly_assessment" || plan.dispatchState !== (live ? "requires_dispatch_checks" : "not_connected")
    || !hash(plan.bindingHash) || !hash(plan.planHash) || plan.totalTokenCeiling !== 8000
    || plan.totalModelRequests !== 2 || plan.targetRequests !== 0 || !Array.isArray(plan.actions) || plan.actions.length !== 2) return null;
  const ids = new Set();
  for (const [index, action] of plan.actions.entries()) {
    if (!object(action) || !exact(action, ["actionId", "order", "role", "tokenCeiling", "modelRequests", "maxOutputTokens",
      "targetRequests", "previousActionId", "requiredPreviousState", "advisoryOnly", "executionState"])
      || !hash(action.actionId) || ids.has(action.actionId) || action.order !== index + 1 || action.role !== roles[index]
      || action.tokenCeiling !== 4000 || action.modelRequests !== 1 || action.maxOutputTokens !== 512
      || action.targetRequests !== 0 || action.advisoryOnly !== true || action.executionState !== "not_started"
      || action.previousActionId !== (index === 0 ? null : plan.actions[index - 1].actionId)
      || action.requiredPreviousState !== (index === 0 ? "frozen_original_evidence" : "valid_advisory_receipt")) return null;
    ids.add(action.actionId);
  }
  return plan as unknown as OrderedAssessmentPlan;
}

export function orderedPlanCoherent(draft: Record<string, unknown>): boolean {
  return Array.isArray(draft.reasonCodes) && (draft.reasonCodes.includes(marker) || draft.reasonCodes.includes(liveMarker))
    ? orderedAssessmentPlan(draft) !== null : draft.readonlyAssessmentPlan == null;
}


// Display only a verified committed projection. This parser grants no authority
// and never substitutes immutable human review actions with execution state.
export function orderedAssessmentExecution(item: AgentTimelineItem, view: NativeScanStatus | undefined): OrderedAssessmentExecution | null {
  const value = item.orderedAssessmentExecution;
  const text = (v: unknown) => typeof v === "string" && v.length > 0;
  const integer = (v: unknown) => Number.isSafeInteger(v) && (v as number) >= 0;
  if (!view || item.eventType !== "user_directive" || item.deliveryState === "receipt_unverified" || !object(value)
    || !exact(value, ["schemaVersion", "directiveId", "sourceDraftId", "draftHash", "planHash", "scanId", "attemptNumber",
      "rootRunId", "targetKey", "threadKey", "state", "completedAssessments", "plannedAssessments", "advisoryOnly",
      "coverageVerified", "independentReviewApproved", "actions"])
    || value.schemaVersion !== 1 || value.directiveId !== item.id || value.scanId !== view.scanId
    || value.attemptNumber !== view.attemptNumber || value.rootRunId !== item.toRunId
    || value.targetKey !== item.targetKey || value.threadKey !== (item.threadKey || "team")
    || value.state !== item.status || !text(value.sourceDraftId) || !hash(value.draftHash) || !hash(value.planHash)
    || !integer(value.completedAssessments) || value.completedAssessments > 2 || value.plannedAssessments !== 2
    || value.advisoryOnly !== true || value.coverageVerified !== false || value.independentReviewApproved !== false
    || !Array.isArray(value.actions) || value.actions.length !== 2) return null;
  const roles = new Set(), ids = new Set();
  let completed = 0;
  for (const [index, action] of value.actions.entries()) {
    if (!object(action) || !exact(action, ["actionId", "order", "role", "state", "assignmentId", "childRunId", "receipt"])
      || !hash(action.actionId) || ids.has(action.actionId) || action.order !== index + 1
      || !["spa_api_mapper", "deep_investigator"].includes(String(action.role)) || roles.has(action.role)
      || !["not_started", "prepared", "executing", "outcome_unknown", "received", "completed", "failed", "cancelled_before_dispatch"].includes(String(action.state))) return null;
    ids.add(action.actionId); roles.add(action.role);
    const receipt = action.receipt;
    const closure = item.taskClosure;
    if (action.state === "cancelled_before_dispatch" && (value.state !== "deferred"
      || !object(closure) || !exact(closure, ["fromStatus", "disposition", "rootTerminalCode", "requiresReconciliation", "automaticRetry", "closedAt"])
      || closure.fromStatus !== "assigned" || closure.disposition !== "not_applied" || closure.requiresReconciliation !== false
      || closure.automaticRetry !== false || !text(closure.rootTerminalCode) || !text(closure.closedAt))) return null;
    if (action.state === "not_started") {
      if (action.assignmentId !== null || action.childRunId !== null || receipt !== null) return null;
    } else if (!text(action.assignmentId) || !text(action.childRunId)) return null;
    if (!["completed", "failed"].includes(String(action.state))) {
      if (receipt !== null) return null;
      continue;
    }
    if (!object(receipt) || receipt.schemaVersion !== 1 || receipt.kind !== "ordered_readonly_assessment_receipt"
      || !text(receipt.receiptId) || receipt.directiveId !== value.directiveId || receipt.sourceDraftId !== value.sourceDraftId
      || receipt.draftHash !== value.draftHash || receipt.scanId !== value.scanId || receipt.attemptNumber !== value.attemptNumber
      || receipt.rootRunId !== value.rootRunId || receipt.targetKey !== value.targetKey || receipt.planHash !== value.planHash
      || receipt.actionId !== action.actionId || receipt.order !== action.order || receipt.role !== action.role
      || receipt.assignmentId !== action.assignmentId || receipt.childRunId !== action.childRunId
      || !text(receipt.workerId) || !text(receipt.leaseAttemptId) || !hash(receipt.requestHash) || !hash(receipt.responseHash)
      || !integer(receipt.modelEventSequence) || receipt.modelEventSequence === 0 || receipt.usageReported !== true
      || receipt.advisoryOnly !== true || receipt.coverageVerified !== false || receipt.targetRequests !== 0
      || receipt.independentReviewApproved !== false || receipt.reviewerReceipt !== null
      || receipt.outcome !== (action.state === "completed" ? "valid_advisory" : "invalid_assessment")) return null;
    const assessment = receipt.assessment, usage = receipt.usage;
    if (!object(assessment) || !exact(assessment, ["summary", "suggestions", "limitations", "valid"])
      || typeof assessment.summary !== "string" || assessment.summary.length > 4000
      || assessment.valid !== (action.state === "completed")
      || ![assessment.suggestions, assessment.limitations].every(v => Array.isArray(v) && v.length <= 12
        && v.every(text => typeof text === "string" && text.length <= 1000))
      || !object(usage) || !exact(usage, ["inputTokens", "cachedInputTokens", "outputTokens", "totalTokens", "modelRequests"])
      || !Object.values(usage).every(integer) || usage.modelRequests !== 1
      || usage.totalTokens !== Number(usage.inputTokens) + Number(usage.outputTokens)
      || Number(usage.cachedInputTokens) > Number(usage.inputTokens)) return null;
    if (index === 0) { if (receipt.predecessor !== null) return null; }
    else {
      const previous = value.actions[0].receipt;
      if (!object(previous) || previous.outcome !== "valid_advisory" || !object(receipt.predecessor)
        || receipt.predecessor.receiptId !== previous.receiptId || receipt.predecessor.actionId !== value.actions[0].actionId
        || !hash(receipt.predecessor.receiptHash)) return null;
    }
    if (action.state === "completed") completed++;
  }
  if (value.completedAssessments !== completed || (value.state === "completed" && completed !== 2)
    || (value.state === "failed" && !value.actions.some(a => a.state === "failed"))) return null;
  return value as unknown as OrderedAssessmentExecution;
}
