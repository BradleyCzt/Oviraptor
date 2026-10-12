import { validRootCostReceipt } from "./contract";
import type { RootCostReceipt } from "./contract";
import { validHistoricalRootContinuationProposal } from "./continuation";
import type { RootContinuationProposal } from "./continuation";
import { validRootContinuationIssuance } from "./issuance";
import type { RootContinuationIssuance } from "./issuance";

export interface RootContinuationDiscovery {
  schema: 1; scanId: string; attemptNumber: number;
  issued: null | { originalReceipt: RootCostReceipt; proposal: RootContinuationProposal;
    status: RootContinuationIssuance; historicalProposal: true };
}
const exact = (v: unknown, keys: string[]): v is Record<string, unknown> => !!v && typeof v === "object"
  && !Array.isArray(v) && Object.keys(v).length === keys.length
  && keys.every(k => Object.prototype.hasOwnProperty.call(v, k));
function equal(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => equal(v, b[i]));
  if (!a || !b || typeof a !== "object" || typeof b !== "object" || Array.isArray(a) || Array.isArray(b)) return false;
  const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
  return Object.keys(left).length === Object.keys(right).length && Object.keys(left).every(k =>
    Object.prototype.hasOwnProperty.call(right, k) && equal(left[k], right[k]));
}
const immutableStatusKeys = ["schema", "approvalId", "proposalHash", "scanId", "sourceAttemptNumber",
  "newAttemptNumber", "sourceRootRunId", "newRootRunId", "sourceReceiptId", "automaticResumeAllowed",
  "originalSemanticReplayAllowed", "financialSource", "taskDescription", "originalDeadline"] as const;
export function validRootContinuationDiscovery(value: unknown, scanId: string, attempt: number,
  previous?: RootContinuationIssuance, previousProposal?: RootContinuationProposal): value is RootContinuationDiscovery {
  if (!exact(value, ["schema", "scanId", "attemptNumber", "issued"]) || value.schema !== 1
    || !scanId.trim() || value.scanId !== scanId || value.attemptNumber !== attempt
    || !Number.isSafeInteger(attempt) || attempt <= 0) return false;
  if (value.issued === null) return true;
  const issued = value.issued;
  if (!exact(issued, ["originalReceipt", "proposal", "status", "historicalProposal"])
    || issued.historicalProposal !== true || attempt <= 1
    || !validRootCostReceipt(issued.originalReceipt, scanId, attempt - 1)
    || !validHistoricalRootContinuationProposal(issued.proposal, issued.originalReceipt)
    || issued.proposal.proposedAttemptNumber !== attempt
    || !validRootContinuationIssuance(issued.status, issued.proposal, previous)) return false;
  const status = issued.status;
  return status.scanId === scanId && status.newAttemptNumber === attempt
    && (!previousProposal || equal(previousProposal, issued.proposal))
    && (!previous || immutableStatusKeys.every(k => equal(previous[k], status[k])));
}
