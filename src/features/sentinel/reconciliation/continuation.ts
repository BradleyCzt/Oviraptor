import type { RootCostReceipt } from "./contract";

export const continuationDimensions = ["model_input_tokens", "model_cached_tokens", "model_output_tokens",
  "model_requests", "target_requests", "browser_actions", "controlled_writes", "upload_bytes",
  "concurrency_batches", "wall_time_ms"] as const;
interface SettledFinancialSource {
  version: 1; proofKind: "settled_original_finance"; rootRunId: string; ownerId: string;
  scanId: string; attemptNumber: number; targetKey: string; coordinatorEpoch: number;
  coordinatorFence: string; receiptId: string; remaining: Array<number | null>; dimensions: string[];
  sharedRemainingTokens: number | null; sharedRemainingRequests: number | null;
  originalDeadline: string; rootPhysicalHash: string; ownerPhysicalHash: string;
  financialPhysicalHash: string; executionAllowed: false;
}
export interface RootContinuationProposal {
  schema: 1; scanId: string; attemptNumber: number; rootRunId: string; receiptId: string;
  proposedAttemptNumber: number; targetKey: string; previousStatus: "paused" | "partial";
  financialSource: SettledFinancialSource; mode: "multi"; originalModeReceiptId: string;
  modelBindingHash: string; identityMode: "AnonymousOnly" | "SingleIdentity" | "IdentitySet";
  identityCount: number; evidenceHashes: Record<string, string>; historicalRootsChecked: number;
  taskPurpose: "new_work_after_original_cost_confirmation"; originalSemanticReplayAllowed: false;
  taskDescription: { purpose: "fresh_root_assessment_of_frozen_web_evidence"; targetKey: string;
    originalPlanHash: string; originalSemanticReplayAllowed: false; limits: "remaining_original_budget_and_deadline" };
  executionAllowed: false; automaticResumeAllowed: false; issuanceImplemented: boolean; proposalHash: string;
}
export interface RootContinuationPreview extends RootContinuationProposal { issuanceImplemented: true }
const exact = (value: unknown, keys: readonly string[]): value is Record<string, unknown> => !!value
  && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length
  && keys.every(key => Object.prototype.hasOwnProperty.call(value, key));
const integer = (value: unknown): value is number => typeof value === "number"
  && Number.isSafeInteger(value) && value >= 0;
const positive = (value: unknown): value is number => integer(value) && value > 0;
const text = (value: unknown, max = 200): value is string => typeof value === "string" && !!value.trim()
  && value.length <= max && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
const hash = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const amount = (value: unknown) => value === null || integer(value);
const sourceKeys = ["version", "proofKind", "rootRunId", "ownerId", "scanId", "attemptNumber", "targetKey",
  "coordinatorEpoch", "coordinatorFence", "receiptId", "remaining", "dimensions", "sharedRemainingTokens",
  "sharedRemainingRequests", "originalDeadline", "rootPhysicalHash", "ownerPhysicalHash", "financialPhysicalHash", "executionAllowed"];
const evidenceKeys = ["scope", "authority", "mode", "plan", "model", "context", "sessions", "history"];
const evidence = (value: unknown) => exact(value, evidenceKeys) && evidenceKeys.every(key => hash(value[key]));
const previewKeys = ["schema", "scanId", "attemptNumber", "rootRunId", "receiptId", "proposedAttemptNumber",
  "targetKey", "previousStatus", "financialSource", "mode", "originalModeReceiptId", "modelBindingHash",
  "identityMode", "identityCount", "evidenceHashes", "historicalRootsChecked", "taskPurpose",
  "originalSemanticReplayAllowed", "taskDescription", "executionAllowed", "automaticResumeAllowed", "issuanceImplemented", "proposalHash"];
function deadline(value: unknown): boolean {
  if (typeof value !== "string" || !/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d(?:\.\d{3})?$/.test(value)) return false;
  const date = new Date(value.replace(" ", "T") + "Z");
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 23).replace("T", " ")
    === (value.length === 19 ? `${value}.000` : value);
}
function validProposal(value: unknown, receipt: RootCostReceipt, historical: boolean): value is RootContinuationProposal {
  if (!exact(value, previewKeys) || value.schema !== 1 || value.scanId !== receipt.scanId
    || value.attemptNumber !== receipt.attemptNumber || value.rootRunId !== receipt.rootRunId
    || value.receiptId !== receipt.receiptId || !positive(value.proposedAttemptNumber)
    || value.proposedAttemptNumber !== receipt.attemptNumber + 1 || !text(value.targetKey, 8192)
    || !["paused", "partial"].includes(value.previousStatus as string) || value.mode !== "multi"
    || !text(value.originalModeReceiptId) || !hash(value.modelBindingHash) || !hash(value.proposalHash)
    || !integer(value.identityCount) || !positive(value.historicalRootsChecked)
    || !(value.identityMode === "AnonymousOnly" && value.identityCount === 0
      || value.identityMode === "SingleIdentity" && value.identityCount === 1
      || value.identityMode === "IdentitySet" && value.identityCount > 1)
    || value.taskPurpose !== "new_work_after_original_cost_confirmation" || value.originalSemanticReplayAllowed !== false
    || value.executionAllowed !== false || value.automaticResumeAllowed !== false
    || (historical ? typeof value.issuanceImplemented !== "boolean" : value.issuanceImplemented !== true)
    || !evidence(value.evidenceHashes)) return false;
  const source = value.financialSource;
  const task = value.taskDescription;
  return exact(task, ["purpose", "targetKey", "originalPlanHash", "originalSemanticReplayAllowed", "limits"])
    && task.purpose === "fresh_root_assessment_of_frozen_web_evidence" && task.targetKey === value.targetKey
    && hash(task.originalPlanHash) && task.originalSemanticReplayAllowed === false
    && task.limits === "remaining_original_budget_and_deadline"
    && exact(source, sourceKeys) && source.version === 1 && source.proofKind === "settled_original_finance"
    && source.rootRunId === receipt.rootRunId && source.scanId === receipt.scanId
    && source.attemptNumber === receipt.attemptNumber && source.receiptId === receipt.receiptId
    && source.targetKey === value.targetKey && text(source.ownerId) && positive(source.coordinatorEpoch)
    && text(source.coordinatorFence) && source.executionAllowed === false && deadline(source.originalDeadline)
    && [source.rootPhysicalHash, source.ownerPhysicalHash, source.financialPhysicalHash].every(hash)
    && Array.isArray(source.dimensions) && source.dimensions.length === 10
    && source.dimensions.every((dimension, index) => dimension === continuationDimensions[index])
    && Array.isArray(source.remaining) && source.remaining.length === 10 && source.remaining.every(amount)
    && amount(source.sharedRemainingTokens) && amount(source.sharedRemainingRequests);
}
export function validRootContinuationPreview(value: unknown, receipt: RootCostReceipt): value is RootContinuationPreview {
  return validProposal(value, receipt, false);
}
export function validHistoricalRootContinuationProposal(value: unknown, receipt: RootCostReceipt): value is RootContinuationProposal {
  return validProposal(value, receipt, true);
}
