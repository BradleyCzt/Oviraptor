export interface RootCostUsage {
  inputTokens: number; cachedInputTokens: number; outputTokens: number;
  totalTokens: number; modelRequests: number;
}
export type RootCostOperation = "settle_received" | "reconcile_forfeit";
export interface RootCostPreview {
  version: 1; scanId: string; attemptNumber: number; rootRunId: string; callId: string;
  leaseAttemptId: string; round: number; requestHash: string; basisHash: string;
  invoiceHash: string; confirmationHash: string; usage: RootCostUsage; operation: RootCostOperation;
}
export interface RootCostReceipt extends RootCostPreview {
  receiptId: string; actualCostConfirmed: true;
  executionAllowed: false; automaticResumeAllowed: false;
}
export interface RootCostInventory {
  schema: 1; executionAllowed: false; automaticResumeAllowed: false;
  items: RootCostPreview[]; blocked: Array<{ rootRunId: string; callId: string; reason: string }>;
  receipts: RootCostReceipt[]; truncated: boolean;
}

const object = (value: unknown): value is Record<string, unknown> => !!value
  && typeof value === "object" && !Array.isArray(value);
const exact = (value: unknown, keys: readonly string[]): value is Record<string, unknown> => object(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(value, key));
const integer = (value: unknown): value is number => typeof value === "number"
  && Number.isSafeInteger(value) && value >= 0;
const positive = (value: unknown): value is number => integer(value) && value > 0;
const text = (value: unknown): value is string => typeof value === "string" && !!value.trim()
  && value.length <= 200 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
const hash = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const operation = (value: unknown) => value === "settle_received" || value === "reconcile_forfeit";
const usageKeys = ["inputTokens", "cachedInputTokens", "outputTokens", "totalTokens", "modelRequests"];
const previewKeys = ["version", "scanId", "attemptNumber", "rootRunId", "callId", "leaseAttemptId", "round",
  "requestHash", "basisHash", "invoiceHash", "confirmationHash", "usage", "operation"];
const receiptKeys = [...previewKeys, "receiptId", "actualCostConfirmed", "executionAllowed", "automaticResumeAllowed"];

function validUsage(value: unknown): value is RootCostUsage {
  return exact(value, usageKeys) && usageKeys.every(key => integer(value[key]))
    && value.modelRequests === 1 && (value.cachedInputTokens as number) <= (value.inputTokens as number)
    && (value.inputTokens as number) + (value.outputTokens as number) === value.totalTokens;
}
function scope(value: Record<string, unknown>, scanId: string, attempt: number): boolean {
  return value.version === 1 && text(scanId) && positive(attempt)
    && value.scanId === scanId && value.attemptNumber === attempt
    && text(value.rootRunId) && hash(value.callId) && hash(value.invoiceHash)
    && hash(value.confirmationHash) && operation(value.operation) && validUsage(value.usage);
}
function validPreview(value: unknown, scanId: string, attempt: number): value is RootCostPreview {
  return exact(value, previewKeys) && scope(value, scanId, attempt) && text(value.leaseAttemptId)
    && positive(value.round) && hash(value.requestHash) && hash(value.basisHash);
}
export function validRootCostReceipt(value: unknown, scanId: string, attempt: number): value is RootCostReceipt {
  return exact(value, receiptKeys) && scope(value, scanId, attempt)
    && text(value.leaseAttemptId) && positive(value.round) && hash(value.requestHash) && hash(value.basisHash)
    && value.receiptId === value.confirmationHash && value.actualCostConfirmed === true
    && value.executionAllowed === false && value.automaticResumeAllowed === false;
}
export function receiptMatchesPreview(value: unknown, preview: RootCostPreview): value is RootCostReceipt {
  return validRootCostReceipt(value, preview.scanId, preview.attemptNumber)
    && value.rootRunId === preview.rootRunId && value.callId === preview.callId
    && value.confirmationHash === preview.confirmationHash && value.invoiceHash === preview.invoiceHash
    && value.leaseAttemptId === preview.leaseAttemptId && value.round === preview.round
    && value.requestHash === preview.requestHash && value.basisHash === preview.basisHash
    && value.operation === preview.operation && usageKeys.every(key =>
      value.usage[key as keyof RootCostUsage] === preview.usage[key as keyof RootCostUsage]);
}
export function validRootCostInventory(value: unknown, scanId: string, attempt: number): value is RootCostInventory {
  if (!exact(value, ["schema", "executionAllowed", "automaticResumeAllowed", "items", "blocked", "receipts", "truncated"])
    || value.schema !== 1 || value.executionAllowed !== false || value.automaticResumeAllowed !== false
    || !Array.isArray(value.items) || !Array.isArray(value.blocked) || !Array.isArray(value.receipts)
    || typeof value.truncated !== "boolean"
    || [value.items, value.blocked, value.receipts].some(rows => rows.length > 100)
    || !value.items.every(item => validPreview(item, scanId, attempt))
    || !value.receipts.every(item => validRootCostReceipt(item, scanId, attempt))
    || !value.blocked.every(item => exact(item, ["rootRunId", "callId", "reason"])
      && text(item.rootRunId) && hash(item.callId) && text(item.reason))) return false;
  const all = [...value.items, ...value.blocked, ...value.receipts];
  return new Set(all.map(item => item.callId)).size === all.length;
}
