export interface RecoverySelection {
  scanId: string; attemptNumber: number; rootRunId: string; protectionReceiptId: string;
}
export interface RecoveryScope {
  projectId: number; taskName: string; urls: string[]; allowedMethods: string[];
  authSessionIds: string[]; authSessionScopeId: string; model: string;
  proxyBindingHash: string | null; orchestrationMode: string; scanMode: string;
  closure: string; skillIds: number[]; instruction: string; maxBudgetUsd: number;
  limits: (number | null)[]; deadlineUtc: string;
}
export interface RecoverySource {
  selection: RecoverySelection; targetUrl: string; responseStatus: number;
  originalFactHash: string; sourceVerified: true; readinessAssessed: false; executionAllowed: false;
}
export interface RecoverySources {
  schemaVersion: 1; purpose: string; scanId: string; projectId: number; taskName: string;
  items: RecoverySource[]; approval: false; readinessAssessed: false;
  executionAllowed: false; targetRequestsGranted: 0;
}
export type RecoveryFact = Record<string, unknown>;
export interface RecoveryOperation {
  version: 1; requestId: string; selection: RecoverySelection; newScope: RecoveryScope; proposalHash: string;
}
export const recoveryDimensions = ["model_input_tokens", "model_cached_tokens", "model_output_tokens",
  "model_requests", "target_requests", "browser_actions", "controlled_writes", "upload_bytes", "concurrency_batches", "wall_time_ms"];
export function record(value: unknown): value is RecoveryFact {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (record(value)) return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${canonical(value[k])}`).join(",")}}`;
  return JSON.stringify(value);
}
export function validSources(value: unknown, scan: string): value is RecoverySources {
  return record(value) && value.schemaVersion === 1 && value.purpose === "select_original_signed_protection_source"
    && value.scanId === scan && Number.isSafeInteger(value.projectId) && Number(value.projectId) > 0
    && typeof value.taskName === "string" && value.approval === false && value.readinessAssessed === false
    && value.executionAllowed === false && value.targetRequestsGranted === 0 && Array.isArray(value.items)
    && value.items.length <= 256 && value.items.every(item => record(item) && record(item.selection)
      && item.selection.scanId === scan && Number.isSafeInteger(item.selection.attemptNumber)
      && Number(item.selection.attemptNumber) > 0 && typeof item.selection.rootRunId === "string" && !!item.selection.rootRunId
      && typeof item.selection.protectionReceiptId === "string" && !!item.selection.protectionReceiptId
      && typeof item.targetUrl === "string" && Number.isInteger(item.responseStatus)
      && typeof item.originalFactHash === "string" && /^[0-9a-f]{64}$/.test(item.originalFactHash)
      && item.sourceVerified === true && item.readinessAssessed === false && item.executionAllowed === false)
    && new Set(value.items.map(item => canonical(item.selection))).size === value.items.length;
}
export function proposalMatches(value: unknown, selection: RecoverySelection, scope: RecoveryScope): value is RecoveryFact {
  return record(value) && value.schemaVersion === 1 && value.purpose === "explicit_new_target_work_after_original_protection_stop"
    && canonical(value.selection) === canonical(selection) && canonical(value.newScope) === canonical(scope)
    && typeof value.proposalHash === "string" && /^[0-9a-f]{64}$/.test(value.proposalHash)
    && value.approval === false && value.executionAllowed === false && value.targetRequestsGranted === 0
    && value.oldStopRetained === true && value.oldCostsRetained === true && value.oldScopeRenewed === false;
}
export function approvalMatches(value: unknown, op: RecoveryOperation): value is RecoveryFact {
  return record(value) && value.schemaVersion === 1 && value.purpose === "explicit_new_target_work_approval"
    && value.requestId === op.requestId && value.proposalHash === op.proposalHash
    && record(value.proposal) && proposalMatches(value.proposal, op.selection, op.newScope)
    && value.proposal.proposalHash === op.proposalHash && typeof value.approvalId === "string" && !!value.approvalId
    && value.status === "approved_pending_fresh_birth" && value.executionAllowed === false && value.targetRequestsGranted === 0
    && value.oldScopeRenewed === false;
}
export function draftMatches(value: unknown, op: RecoveryOperation): value is RecoveryFact {
  return record(value) && value.schemaVersion === 1 && value.purpose === "explicit_protection_recovery_new_draft"
    && typeof value.scanId === "string" && !!value.scanId && value.scanId !== op.selection.scanId
    && typeof value.draftId === "string" && !!value.draftId && approvalMatches(value.approval, op)
    && value.status === "draft_pending_dedicated_fresh_birth" && value.executionAllowed === false && value.targetRequestsGranted === 0
    && value.oldScopeRenewed === false && value.identitiesBound === false;
}
export function cancellationMatches(value: unknown, op: RecoveryOperation, draft: RecoveryFact | undefined): value is RecoveryFact {
  return record(value) && value.schemaVersion === 1 && value.purpose === "cancel_unconsumed_protection_recovery_draft"
    && value.requestId === op.requestId && value.authorityRevoked === true && value.executionAllowed === false
    && value.targetRequestsGranted === 0 && value.oldStopRetained === true && value.oldCostsRetained === true
    && typeof value.cancellationId === "string" && typeof value.scanId === "string"
    && !!draft && value.scanId === draft.scanId && value.draftId === draft.draftId
    && record(draft.approval) && value.approvalId === draft.approval.approvalId;
}
export function validOperation(value: unknown, scan: string): value is RecoveryOperation {
  if (!record(value) || value.version !== 1 || typeof value.requestId !== "string"
    || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value.requestId)
    || typeof value.proposalHash !== "string" || !/^[0-9a-f]{64}$/.test(value.proposalHash)
    || !record(value.selection) || value.selection.scanId !== scan
    || !Number.isSafeInteger(value.selection.attemptNumber) || Number(value.selection.attemptNumber) <= 0
    || typeof value.selection.rootRunId !== "string" || !value.selection.rootRunId
    || typeof value.selection.protectionReceiptId !== "string" || !value.selection.protectionReceiptId
    || !record(value.newScope)) return false;
  const s = value.newScope;
  const strings = ["taskName", "authSessionScopeId", "model", "orchestrationMode", "scanMode", "closure", "instruction", "deadlineUtc"];
  return Object.keys(s).length === 16 && strings.every(k => typeof s[k] === "string")
    && Number.isSafeInteger(s.projectId) && Number(s.projectId) > 0
    && ["urls", "allowedMethods", "authSessionIds"].every(k => Array.isArray(s[k]) && (s[k] as unknown[]).every(v => typeof v === "string"))
    && Array.isArray(s.skillIds) && s.skillIds.every(v => Number.isSafeInteger(v) && Number(v) > 0)
    && (s.proxyBindingHash === null || (typeof s.proxyBindingHash === "string" && /^[0-9a-f]{64}$/.test(s.proxyBindingHash)))
    && typeof s.maxBudgetUsd === "number" && Number.isFinite(s.maxBudgetUsd) && s.maxBudgetUsd > 0
    && Array.isArray(s.limits) && s.limits.length === 10 && s.limits.every((n, i) =>
      n === null ? i < 3 : Number.isSafeInteger(n) && Number(n) >= 0);
}
