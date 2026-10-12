import type { RootContinuationProposal } from "./continuation";
export interface RootContinuationIssuance {
  schema: 1; approvalId: string; proposalHash: string; scanId: string;
  sourceAttemptNumber: number; newAttemptNumber: number; sourceRootRunId: string;
  newRootRunId: string; sourceReceiptId: string;
  status: "issued" | "assessment_published" | "review_required";
  assessmentCompleted: boolean; attemptClosed: boolean; automaticResumeAllowed: false;
  originalSemanticReplayAllowed: false; financialSource: RootContinuationProposal["financialSource"];
  taskDescription: RootContinuationProposal["taskDescription"]; originalDeadline: string;
  reviewReason: "financial_assessment_requires_review" | "financial_continuation_attempt_not_runnable"
    | "authority_revalidation_required" | "financial_continuation_admission_requires_review" | null;
  workerStartAllowed: boolean;
}
const reviewReasons = ["financial_assessment_requires_review", "financial_continuation_attempt_not_runnable",
  "authority_revalidation_required", "financial_continuation_admission_requires_review"];
const keys = ["schema", "approvalId", "proposalHash", "scanId", "sourceAttemptNumber", "newAttemptNumber",
  "sourceRootRunId", "newRootRunId", "sourceReceiptId", "status", "assessmentCompleted", "attemptClosed",
  "automaticResumeAllowed", "originalSemanticReplayAllowed", "financialSource", "taskDescription",
  "originalDeadline", "reviewReason", "workerStartAllowed"];
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const safe = (v: unknown, max = 512): v is string => typeof v === "string" && !!v.trim() && v.length <= max
  && !/[\u0000-\u001f\u007f-\u009f]/u.test(v);
function equal(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => equal(v, b[i]));
  return object(a) && object(b) && Object.keys(a).length === Object.keys(b).length
    && Object.keys(a).every(k => Object.prototype.hasOwnProperty.call(b, k) && equal(a[k], b[k]));
}
export function validRootContinuationIssuance(value: unknown, proposal: RootContinuationProposal,
  previous?: RootContinuationIssuance): value is RootContinuationIssuance {
  if (!object(value) || Object.keys(value).length !== keys.length || !keys.every(k => Object.prototype.hasOwnProperty.call(value, k))
    || value.schema !== 1 || !safe(value.approvalId)
    || !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(value.approvalId)
    || value.proposalHash !== proposal.proposalHash || value.scanId !== proposal.scanId
    || value.sourceAttemptNumber !== proposal.attemptNumber || value.newAttemptNumber !== proposal.proposedAttemptNumber
    || value.sourceRootRunId !== proposal.rootRunId || value.sourceReceiptId !== proposal.receiptId
    || !safe(value.newRootRunId) || value.newRootRunId === proposal.rootRunId
    || value.automaticResumeAllowed !== false || value.originalSemanticReplayAllowed !== false
    || typeof value.assessmentCompleted !== "boolean" || typeof value.attemptClosed !== "boolean"
    || typeof value.workerStartAllowed !== "boolean" || value.attemptClosed && (!value.assessmentCompleted || value.workerStartAllowed)
    || !equal(value.financialSource, proposal.financialSource) || !equal(value.taskDescription, proposal.taskDescription)
    || value.originalDeadline !== proposal.financialSource.originalDeadline
    || previous && (value.approvalId !== previous.approvalId || value.newRootRunId !== previous.newRootRunId
      || previous.assessmentCompleted && !value.assessmentCompleted || previous.attemptClosed && !value.attemptClosed)) return false;
  if (value.status === "review_required") return value.workerStartAllowed === false && !value.attemptClosed
    && typeof value.reviewReason === "string" && reviewReasons.includes(value.reviewReason);
  return value.reviewReason === null && (value.status === "issued" && !value.assessmentCompleted && !value.attemptClosed
    || value.status === "assessment_published" && value.assessmentCompleted);
}
