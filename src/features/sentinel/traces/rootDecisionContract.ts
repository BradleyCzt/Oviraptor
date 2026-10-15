import type { AgentTraceEvent, HumanDirectiveDecisionContext } from "../types";

export interface PublicDecisionSummary {
  schemaVersion: 1;
  observed: string[];
  missing: string[];
  suggestions: string[];
  costNotes: string[];
  risks: string[];
}
export interface DecisionUsage {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  modelRequests: number;
}
export interface RootDecisionDisplay {
  humanDirective?: HumanDirectiveDecisionContext;
  summary: PublicDecisionSummary;
  localTools?: LocalToolName[];
  usage?: DecisionUsage;
  round: number;
  callId: string;
}
type RecordValue = Record<string, unknown>;
const object = (value: unknown): value is RecordValue =>
  !!value && typeof value === "object" && !Array.isArray(value);
const count = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const boundedText = (value: unknown): value is string => typeof value === "string"
  && !!value.trim() && Array.from(value).length <= 512 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
const keys = ["observed", "missing", "suggestions", "costNotes", "risks"] as const;

const localToolLabels = {
  "snapshot.read": ["读取任务快照", "Read task snapshot"],
  "evidence.read": ["读取冻结证据", "Read frozen evidence"],
  "capability_budget.read": ["查看预算与能力", "Read budget and capability"],
  "plan.propose": ["提出计划建议", "Propose a plan"],
} as const;
export type LocalToolName = keyof typeof localToolLabels;
export function localToolNames(value: unknown): LocalToolName[] | undefined {
  if (!Array.isArray(value) || value.length < 1 || value.length > 4
    || !value.every(name => typeof name === "string" && Object.prototype.hasOwnProperty.call(localToolLabels, name))) return;
  return [...value] as LocalToolName[];
}
export function localToolLabel(name: LocalToolName, tr: (zh: string, en: string) => string): string {
  const [zh, en] = localToolLabels[name]; return tr(zh, en);
}
function stepToolNames(value: unknown): LocalToolName[] | undefined {
  if (!object(value) || Object.keys(value).length !== 1 || !Array.isArray(value.calls)
    || value.calls.length < 1 || value.calls.length > 4) return;
  const ids = new Set<string>();
  for (const call of value.calls) {
    if (!object(call) || Object.keys(call).length !== 4
      || !["id", "name", "arguments", "result"].every(key => Object.prototype.hasOwnProperty.call(call, key))
      || typeof call.id !== "string" || !/^[A-Za-z0-9_-]{1,64}$/.test(call.id)
      || ids.has(call.id) || !object(call.arguments) || !object(call.result)) return;
    ids.add(call.id);
    if (call.name === "plan.propose") {
      if (Object.keys(call.arguments).length !== 1 || typeof call.arguments.step !== "string"
        || !call.arguments.step.trim() || call.arguments.step.length > 64) return;
    } else if (Object.keys(call.arguments).length) return;
  }
  return localToolNames(value.calls.map(call => call.name));
}

export function isRootModelRound(event: AgentTraceEvent): boolean {
  return event.eventType === "model_round_completed" && event.role === "coordinator";
}

// Parse only the bounded public summary in a persisted Native display envelope.
// This is display validation. Original receipts/Rust retain all authority; a
// readable observation or suggestion proves neither adoption nor execution.
export function rootDecisionDisplay(event: AgentTraceEvent, source: string): RootDecisionDisplay | undefined {
  if (source !== "native_ledger" || !isRootModelRound(event) || event.status !== "recorded"
    || event.detailTruncated || typeof event.detail !== "string" || event.detail.length > 48 * 1024
    || !event.sessionId || !event.id.startsWith(`native:${event.sessionId}:`)) return;
  const sequence = event.id.slice(`native:${event.sessionId}:`.length);
  if (!/^[1-9]\d*$/.test(sequence) || !Number.isSafeInteger(Number(sequence))) return;
  let value: unknown;
  try { value = JSON.parse(event.detail); } catch { return; }
  if (!object(value) || value.rootTickVersion !== 1 || value.advisoryOnly !== true
    || !Array.isArray(value.toolCalls) || value.toolCalls.length
    || value.modelRequests !== 1 || !count(value.turns) || value.turns < 1
    || !boundedText(value.callId) || !boundedText(value.rootControl)
    || ["requestHash", "basisHash", "responseHash"].some(key =>
      typeof value[key] !== "string" || !/^[a-f0-9]{64}$/.test(value[key] as string))) return;
  const localTools = Object.prototype.hasOwnProperty.call(value, "localStep") ? stepToolNames(value.localStep) : undefined;
  if (Object.prototype.hasOwnProperty.call(value, "localStep") && !localTools) return;
  const summary = value.decisionSummary;
  if (!object(summary) || summary.schemaVersion !== 1
    || Object.keys(summary).length !== 6
    || Object.keys(summary).some(key => key !== "schemaVersion" && !keys.includes(key as typeof keys[number]))
    || keys.some(key => !Array.isArray(summary[key]) || summary[key].length > 16
      || !summary[key].every(boundedText))) return;
  const usage = value.usage;
  let savedUsage: DecisionUsage | undefined;
  if (object(usage)
    && ["inputTokens", "cachedInputTokens", "outputTokens", "totalTokens", "modelRequests"].every(key => count(usage[key]))) {
    const invoice = usage as unknown as DecisionUsage;
    if (invoice.modelRequests === 1 && invoice.cachedInputTokens <= invoice.inputTokens
      && invoice.inputTokens + invoice.outputTokens === invoice.totalTokens && value.totalTokens === invoice.totalTokens) {
      savedUsage = { inputTokens: invoice.inputTokens, cachedInputTokens: invoice.cachedInputTokens,
        outputTokens: invoice.outputTokens, totalTokens: invoice.totalTokens, modelRequests: 1 };
    }
  }
  // Deliberately drop unrecognized envelope/body fields, never spread raw IPC.
  return { summary: { schemaVersion: 1, observed: [...summary.observed as string[]],
    missing: [...summary.missing as string[]], suggestions: [...summary.suggestions as string[]],
    costNotes: [...summary.costNotes as string[]], risks: [...summary.risks as string[]] },
    localTools, usage: savedUsage, round: value.turns, callId: value.callId };
}

export function decisionSuggestion(text: string, tr: (zh: string, en: string) => string): string {
  if (text === "dispatch:web_executor") return tr("建议进入 Web 执行阶段", "Suggest the Web execution stage");
  if (text === "dispatch:deep_investigator") return tr("建议交由深度调查角色补证", "Suggest further evidence from the Investigator");
  if (text === "dispatch:spa_api_mapper") return tr("建议交由前端映射角色梳理证据", "Suggest evidence mapping by the SPA/API Mapper");
  if (text === "assess:human_directive") return tr("建议评估已确认的指令", "Suggest assessment of the confirmed directive");
  if (text === "assess:budget_allocation") return tr("建议评估原预算分配", "Suggest assessment of the original budget allocation");
  if (text === "assess:client_side_configuration") return tr("建议评估客户端配置证据", "Suggest assessment of client configuration evidence");
  if (text === "assess:identity_session_metadata") return tr("建议评估身份与会话证据", "Suggest assessment of identity and session evidence");
  if (text === "assess:gap_proposal") return tr("建议评估补充证据提案", "Suggest assessment of an evidence gap proposal");
  return text;
}
