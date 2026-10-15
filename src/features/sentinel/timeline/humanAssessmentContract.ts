import type { NativeScanStatus, RootHumanAssessmentObligation } from "../types";
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const exact = (v: unknown, keys: readonly string[]): v is Record<string, unknown> => object(v)
  && Object.keys(v).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(v, key));
const positive = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v > 0;
const hash = (v: unknown): v is string => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
const text = (v: unknown, max = 200): v is string => typeof v === "string" && !!v.trim()
  && Array.from(v).length <= max && !/[\u0000-\u001f\u007f-\u009f]/u.test(v);
const rowKeys = ["rootRunId", "callId", "round", "targetKey", "humanDirective", "state", "reportedUsage", "createdAt"];
const humanKeys = ["schemaVersion", "directiveId", "draftId", "revision", "draftHash", "confirmationReceiptId", "threadKey", "targetKey"];
const usageKeys = ["inputTokens", "cachedInputTokens", "outputTokens", "totalTokens", "modelRequests"];
const states = ["cost_unconfirmed", "assessment_unpublished", "dispatch_unconfirmed", "not_sent", "awaiting_receipt"];
function timestamp(v: unknown): boolean {
  if (typeof v !== "string" || !/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d$/.test(v)) return false;
  const date = new Date(v.replace(" ", "T") + "Z");
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 19).replace("T", " ") === v;
}
function row(v: unknown): v is RootHumanAssessmentObligation {
  if (!exact(v, rowKeys) || !text(v.rootRunId) || !hash(v.callId) || !positive(v.round)
    || !text(v.targetKey, 8192) || !states.includes(v.state as string) || !timestamp(v.createdAt)) return false;
  const h = v.humanDirective;
  if (!exact(h, humanKeys) || h.schemaVersion !== 1 || !positive(h.revision) || !hash(h.draftHash)
    || ["directiveId", "draftId", "confirmationReceiptId", "threadKey"].some(key => !text(h[key]))
    || h.targetKey !== v.targetKey || ((h.threadKey as string).startsWith("coordinator:")
      && h.threadKey !== `coordinator:${v.rootRunId}`)) return false;
  // These are supplier-reported values, not a settled bill. Inconsistent totals
  // can be the reason for the obligation; retain them without inventing zero.
  const usage = v.reportedUsage;
  if (!["cost_unconfirmed", "assessment_unpublished"].includes(v.state as string) && usage !== null) return false;
  return usage === null || (exact(usage, usageKeys)
    && usageKeys.every(key => typeof usage[key] === "number"
      && Number.isSafeInteger(usage[key]) && usage[key] >= 0));
}
// Optional additive status metadata keeps existing Native JSON readable. Rust
// proves original request/receipt scope; this display parser grants no authority.
export function validHumanAssessmentSnapshot(status: NativeScanStatus): boolean {
  if (!Object.prototype.hasOwnProperty.call(status, "humanAssessmentObligations")) return true;
  const v: unknown = status.humanAssessmentObligations;
  if (!exact(v, ["schemaVersion", "executionAllowed", "automaticResumeAllowed", "items", "truncated"])
    || v.schemaVersion !== 1 || v.executionAllowed !== false || v.automaticResumeAllowed !== false
    || typeof v.truncated !== "boolean" || !Array.isArray(v.items) || v.items.length > 50 || !v.items.every(row)) return false;
  return new Set(v.items.map(item => item.callId)).size === v.items.length;
}
export function humanAssessmentRows(status: NativeScanStatus | undefined): RootHumanAssessmentObligation[] {
  return status && validHumanAssessmentSnapshot(status) ? status.humanAssessmentObligations?.items ?? [] : [];
}
