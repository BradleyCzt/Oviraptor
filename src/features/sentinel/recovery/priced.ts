import { canonical, record, recoveryDimensions, validOperation } from "./contract";
import type { RecoveryFact, RecoveryScope, RecoverySelection } from "./contract";

// Exact strings cross IPC. This contract never authorizes execution or verifies
// a provider dollar invoice; the backend must validate the original signature.
export interface RecoveryPricingInput {
  schemaVersion: 1; maxBudgetUsd: string; priceRevision: string;
  uncachedInput: string; cachedInput: string; output: string; perRequest: string;
}
export interface PricedRecoveryOperation {
  version: 2; requestId: string; selection: RecoverySelection; newScope: RecoveryScope;
  pricing: RecoveryPricingInput; proposalHash: string;
}
const sqlMax = 9223372036854775807n;
function exact(value: RecoveryFact, keys: string[]): boolean {
  return Object.keys(value).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(value, key));
}
function nanos(value: unknown): bigint | undefined {
  if (typeof value !== "string" || value.length > 19 || !/^(0|[1-9]\d*)$/.test(value)) return;
  const amount = BigInt(value); return amount <= sqlMax ? amount : undefined;
}
export function decimalUsdNanos(value: unknown): bigint | undefined {
  if (typeof value !== "string" || value.length > 30 || !/^(0|[1-9]\d*)(\.\d{1,9})?$/.test(value)) return;
  const [whole, fraction = ""] = value.split(".");
  const amount = BigInt(whole) * 1000000000n + BigInt((fraction + "000000000").slice(0, 9));
  return amount <= sqlMax ? amount : undefined;
}
export function validRecoveryPricing(value: unknown): value is RecoveryPricingInput {
  if (!record(value) || !exact(value, ["schemaVersion", "maxBudgetUsd", "priceRevision", "uncachedInput", "cachedInput", "output", "perRequest"])
    || value.schemaVersion !== 1 || typeof value.priceRevision !== "string"
    || !/^[a-zA-Z0-9_.-]{1,80}$/.test(value.priceRevision)) return false;
  const limit = decimalUsdNanos(value.maxBudgetUsd);
  return limit !== undefined && limit > 0n && limit <= 10000000000000n
    && [value.uncachedInput, value.cachedInput, value.output, value.perRequest].every(rate => nanos(rate) !== undefined);
}
export function validPricedRecoveryOperation(value: unknown, scan: string): value is PricedRecoveryOperation {
  return record(value) && exact(value, ["version", "requestId", "selection", "newScope", "pricing", "proposalHash"])
    && value.version === 2 && validRecoveryPricing(value.pricing)
    && validOperation({ version: 1, requestId: value.requestId, selection: value.selection,
      newScope: value.newScope, proposalHash: value.proposalHash }, scan)
    && record(value.newScope) && Array.isArray(value.newScope.urls) && value.newScope.urls.length === 1
    && value.newScope.maxBudgetUsd === Number(decimalUsdNanos(value.pricing.maxBudgetUsd)) / 1e9;
}
export function pricedProposalMatches(value: unknown, selection: RecoverySelection,
  scope: RecoveryScope, pricing: RecoveryPricingInput): value is RecoveryFact {
  if (!record(value) || !validRecoveryPricing(pricing) || scope.urls.length !== 1
    || value.schemaVersion !== 2 || value.purpose !== "explicit_priced_new_target_work_after_original_protection_stop"
    || canonical(value.selection) !== canonical(selection) || canonical(value.newScope) !== canonical(scope)
    || canonical(value.pricingInput) !== canonical(pricing) || typeof value.proposalHash !== "string"
    || !/^[0-9a-f]{64}$/.test(value.proposalHash) || value.approval !== false || value.executionAllowed !== false
    || value.targetRequestsGranted !== 0 || value.oldStopRetained !== true || value.oldCostsRetained !== true
    || value.oldScopeRenewed !== false || value.unknownOriginalWorkReplayAllowed !== false
    || value.freshBirthRequired !== true || value.legacyScopeBudgetUsdIsDisplayOnly !== true
    || value.pricingBasis !== "approved_rate_card_not_provider_dollar_invoice"
    || canonical(value.budgetDimensionNames) !== canonical(recoveryDimensions)) return false;
  const terms = value.financialTerms;
  const model = value.modelBinding;
  const hash = (v: unknown) => typeof v === "string" && /^[0-9a-f]{64}$/.test(v);
  if (!record(model) || !exact(model, ["model", "endpointBinding", "deployment", "credentialBinding", "proxyBinding",
    "maxContextTokens", "maxOutputTokens", "temperature"]) || model.model !== scope.model
    || !hash(model.endpointBinding) || !hash(model.credentialBinding) || !hash(model.proxyBinding)
    || !["local", "cloud"].includes(String(model.deployment))
    // Zero is existing profile metadata for an unspecified context ceiling.
    // A pending approval may preserve it; it is never a verified input bound.
    || !Number.isSafeInteger(model.maxContextTokens) || Number(model.maxContextTokens) < 0
    || !(model.maxOutputTokens === null || Number.isSafeInteger(model.maxOutputTokens) && Number(model.maxOutputTokens) > 0)
    || typeof model.temperature !== "number" || !Number.isFinite(model.temperature)) return false;
  if (!record(terms) || !exact(terms, ["schemaVersion", "maxNanoUsd", "pricing"]) || terms.schemaVersion !== 1
    || nanos(terms.maxNanoUsd) !== decimalUsdNanos(pricing.maxBudgetUsd)
    || scope.maxBudgetUsd !== Number(decimalUsdNanos(pricing.maxBudgetUsd)) / 1e9) return false;
  const card = terms.pricing;
  return record(card) && exact(card, ["schemaVersion", "basis", "tokenRateUnit", "cachedSemantics", "rounding",
    "modelBindingHash", "priceRevision", "uncachedInput", "cachedInput", "output", "perRequest"])
    && card.schemaVersion === 1 && card.basis === "approved_rate_card"
    && card.tokenRateUnit === "nano_usd_per_million_tokens" && card.cachedSemantics === "subset_of_input"
    && card.rounding === "ceil_total_per_model_call" && typeof card.modelBindingHash === "string"
    && /^[0-9a-f]{64}$/.test(card.modelBindingHash) && card.priceRevision === pricing.priceRevision
    && card.uncachedInput === pricing.uncachedInput && card.cachedInput === pricing.cachedInput
    && card.output === pricing.output && card.perRequest === pricing.perRequest;
}
export function pricedApprovalMatches(value: unknown, op: PricedRecoveryOperation): value is RecoveryFact {
  if (!record(op) || !record(op.selection) || typeof op.selection.scanId !== "string") return false;
  return validPricedRecoveryOperation(op, op.selection.scanId) && record(value) && value.schemaVersion === 2
    && value.purpose === "explicit_priced_new_target_work_approval" && value.requestId === op.requestId
    && value.proposalHash === op.proposalHash && record(value.proposal)
    && pricedProposalMatches(value.proposal, op.selection, op.newScope, op.pricing)
    && value.proposal.proposalHash === op.proposalHash && typeof value.approvalId === "string"
    && /^[0-9a-f]{64}$/.test(value.approvalId) && value.consent === "approve_exact_priced_new_scope_preserve_old_stop_and_costs"
    && value.status === "approved_pending_priced_fresh_birth" && value.executionAllowed === false
    && value.targetRequestsGranted === 0 && value.oldScopeRenewed === false;
}

// Only a matched pending v2 operation and its original approval are accepted.
// Signature and the approval byte hash are verified again by the backend.
export function pricedCancellationMatches(value: unknown, op: PricedRecoveryOperation, approved: unknown): value is RecoveryFact {
  return pricedApprovalMatches(approved, op) && record(value)
    && exact(value, ["schemaVersion", "purpose", "cancellationId", "requestId", "approvalId", "approvalHash",
      "originalModeReceiptId", "reason", "authorityRevoked", "executionAllowed", "targetRequestsGranted", "oldStopRetained", "oldCostsRetained"])
    && value.schemaVersion === 1 && value.purpose === "cancel_unconsumed_priced_recovery_approval"
    && typeof value.cancellationId === "string" && /^[0-9a-f]{64}$/.test(value.cancellationId)
    && value.requestId === op.requestId && value.approvalId === approved.approvalId
    && typeof value.approvalHash === "string" && /^[0-9a-f]{64}$/.test(value.approvalHash)
    && typeof value.originalModeReceiptId === "string" && value.originalModeReceiptId.length > 0
    && value.originalModeReceiptId === approved.originalModeReceiptId
    && value.reason === "user_cancelled_before_priced_fresh_birth" && value.authorityRevoked === true
    && value.executionAllowed === false && value.targetRequestsGranted === 0
    && value.oldStopRetained === true && value.oldCostsRetained === true;
}
