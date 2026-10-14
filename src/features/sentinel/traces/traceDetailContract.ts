import type { AgentTraceDetail } from "../types";

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  !!value && typeof value === "object" && !Array.isArray(value);
const own = (value: RecordValue, key: string) => Object.prototype.hasOwnProperty.call(value, key);
const strings = (value: RecordValue, keys: string[]) =>
  keys.every(key => own(value, key) && typeof value[key] === "string");
const counts = (value: RecordValue, keys: string[]) => keys.every(key =>
  own(value, key) && typeof value[key] === "number" && Number.isSafeInteger(value[key]) && value[key] >= 0);
const booleans = (value: RecordValue, keys: string[]) =>
  keys.every(key => own(value, key) && typeof value[key] === "boolean");

// Validate the IPC display contract, not provenance, authorization or redaction.
// Historical projections and additional metadata remain supported.
export function isTraceDetailForTask(value: unknown, scanId: string): value is AgentTraceDetail {
  if (!record(value) || !own(value, "summary") || !record(value.summary)
    || !own(value, "events") || !Array.isArray(value.events)) return false;
  const summary = value.summary;
  if (!strings(summary, ["scanId", "taskName", "projectName", "status", "scanType", "model",
    "instructionHash", "createdAt", "updatedAt", "sourceAuthority"])
    || !scanId || summary.scanId !== scanId
    || !["native_ledger", "historical_external"].includes(summary.sourceAuthority as string)
    || !counts(summary, ["runCount", "agentCount", "messageCount", "reasoningCount", "toolCallCount",
      "toolResultCount", "llmRequests", "inputTokens", "outputTokens", "cachedTokens", "totalTokens",
      "hookedRequestCount", "usageEntryCount", "usageAgentCount"])
    || !booleans(summary, ["exactRequestCapture", "tokenUsageEstimated"])
    || !own(summary, "tools") || !Array.isArray(summary.tools)
    || !summary.tools.every(tool => record(tool) && strings(tool, ["name"]) && counts(tool, ["calls", "results"])))
    return false;
  if (!value.events.every(event => record(event)
    && strings(event, ["id", "sessionId", "callId", "targetUrl", "eventType", "role", "name", "status", "detail", "createdAt"])
    && counts(event, ["detailSize"]) && booleans(event, ["detailTruncated"]))) return false;
  if (value.promptAudit != null) {
    const audit = value.promptAudit;
    if (!record(audit) || !strings(audit, ["captureMode", "source", "captureLevel", "model", "deployment",
      "recordedAt", "instructionSha256", "notice"]) || !counts(audit, ["instructionChars"])
      || !booleans(audit, ["exactModelRequest", "fullPower"])
      || (audit.instruction != null && typeof audit.instruction !== "string")) return false;
  }
  return true;
}
