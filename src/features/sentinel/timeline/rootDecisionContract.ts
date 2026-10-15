import type { AgentTimelineItem, HumanDirectiveDecisionContext } from "../types";
import type { RootDecisionDisplay, DecisionUsage, PublicDecisionSummary } from "../traces/rootDecisionContract";
import { localToolNames } from "../traces/rootDecisionContract";
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const positive = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v > 0;
const hash = (v: unknown): v is string => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
const text = (v: unknown): v is string => typeof v === "string" && !!v.trim()
  && Array.from(v).length <= 512 && !/[\u0000-\u001f\u007f-\u009f]/u.test(v);
const listKeys = ["observed", "missing", "suggestions", "costNotes", "risks"] as const;
const humanKeys = ["schemaVersion", "directiveId", "draftId", "revision", "draftHash", "confirmationReceiptId", "threadKey", "targetKey"] as const;
function humanContext(v: unknown, item: AgentTimelineItem): HumanDirectiveDecisionContext | undefined {
  if (!object(v) || Object.keys(v).length !== humanKeys.length
    || !humanKeys.every(key => Object.prototype.hasOwnProperty.call(v, key))
    || v.schemaVersion !== 1 || !positive(v.revision) || !hash(v.draftHash)
    || ["directiveId", "draftId", "confirmationReceiptId", "threadKey"].some(key => !text(v[key]) || Array.from(v[key] as string).length > 200)
    || v.threadKey !== item.threadKey || v.targetKey !== item.targetKey) return;
  // Keep original provenance fields; never copy arbitrary/private IPC metadata.
  return { schemaVersion: 1, directiveId: v.directiveId as string, draftId: v.draftId as string,
    revision: v.revision as number, draftHash: v.draftHash as string,
    confirmationReceiptId: v.confirmationReceiptId as string, threadKey: v.threadKey as string,
    targetKey: v.targetKey as string };
}

// Public projection display validation only. Rust verifies original request,
// invoice/publication and immutable chat cursor in one snapshot; this parser
// neither mints Root/C nor equates advice, tokens or UI labels with execution.
export function chatDecisionDisplay(item: AgentTimelineItem): RootDecisionDisplay | undefined {
  const v: unknown = item.decisionRecord;
  if (item.eventType !== "root_decision" || item.fromRole !== "coordinator" || !item.fromRunId
    || item.toRole !== "operator" || item.toRunId !== "" || item.status !== "recorded"
    || item.deliveryState !== "persisted" || item.ackState !== "n/a"
    || item.messageKind !== "root_decision_summary" || item.assignmentId !== "" || item.evidenceRevision !== 0
    || typeof item.targetKey !== "string" || !item.targetKey.trim()
    || !positive(item.sequence) || !object(v)
    || Object.keys(v).length !== 7 + Number(Object.prototype.hasOwnProperty.call(v, "localTools")) + Number(Object.prototype.hasOwnProperty.call(v, "humanDirective"))
    || v.schemaVersion !== 1 || v.advisoryOnly !== true || !positive(v.round) || !positive(v.modelEventSequence)
    || !hash(v.callId) || item.correlationId !== v.callId
    || item.id !== JSON.stringify([item.fromRunId, v.modelEventSequence])) return;
  const hasHuman = Object.prototype.hasOwnProperty.call(v, "humanDirective");
  const humanDirective = hasHuman ? humanContext(v.humanDirective, item) : undefined;
  if (hasHuman ? !humanDirective : item.threadKey !== `coordinator:${item.fromRunId}`) return;
  const localTools = Object.prototype.hasOwnProperty.call(v, "localTools") ? localToolNames(v.localTools) : undefined;
  if (Object.prototype.hasOwnProperty.call(v, "localTools") && !localTools) return;
  const s = v.summary;
  if (!object(s) || s.schemaVersion !== 1 || Object.keys(s).length !== 6
    || listKeys.some(key => !Array.isArray(s[key]) || s[key].length > 16 || !s[key].every(text))) return;
  const usage = v.usage;
  const usageKeys = ["inputTokens", "cachedInputTokens", "outputTokens", "totalTokens", "modelRequests"];
  if (!object(usage) || Object.keys(usage).length !== 5
    || usageKeys.some(key => typeof usage[key] !== "number" || !Number.isSafeInteger(usage[key]) || usage[key] < 0)) return;
  const u = usage as unknown as DecisionUsage;
  if (u.modelRequests !== 1 || u.cachedInputTokens > u.inputTokens || u.inputTokens + u.outputTokens !== u.totalTokens) return;
  // Select only the bounded public fields; never spread raw IPC/body fields.
  const summary: PublicDecisionSummary = { schemaVersion: 1,
    observed: [...s.observed as string[]], missing: [...s.missing as string[]],
    suggestions: [...s.suggestions as string[]], costNotes: [...s.costNotes as string[]], risks: [...s.risks as string[]] };
  return { summary, localTools, humanDirective, usage: { inputTokens: u.inputTokens, cachedInputTokens: u.cachedInputTokens,
    outputTokens: u.outputTokens, totalTokens: u.totalTokens, modelRequests: 1 }, round: v.round, callId: v.callId };
}
