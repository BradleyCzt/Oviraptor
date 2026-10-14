import type { AgentKnowledgeEntry, AgentLearningCandidate } from "../../skills/catalogTypes";

// Formatting belongs to the trace/knowledge surface, not the global UI layer.
export function tracePresentation(tr: (zh: string, en: string) => string) {
  function format(value: number) {
    return new Intl.NumberFormat("zh-CN", {
      notation: value > 999999 ? "compact" : "standard",
      maximumFractionDigits: 1,
    }).format(value || 0);
  }

  function eventLabel(value: string) {
    return (
      {
        message: tr("消息", "Message"),
        reasoning: tr("推理记录", "Reasoning"),
        function_call: tr("工具调用", "Tool call"),
        function_call_output: tr("工具结果", "Tool result"),
        model_request: tr("模型请求", "Model request"),
        model_round_completed: tr("模型轮次记录", "Model round record"),
      } as Record<string, string>
    )[value] || value;
  }

  function shortSession(value: string) {
    return value ? value.slice(0, 8) : "root";
  }

  function candidateItems(candidate: AgentLearningCandidate, key: string) {
    const values = candidate.candidate[key];
    if (!Array.isArray(values)) return [];
    return values.slice(0, 8).map((value: any, index: number) => {
      if (typeof value === "string") return { title: value, detail: "" };
      return {
        title: String(value?.title || value?.name || value?.step || `${key} ${index + 1}`),
        detail: String(value?.problem || value?.reason || value?.action || value?.query || value?.detail || ""),
      };
    });
  }

  function patchSummary(candidate: AgentLearningCandidate) {
    const patch = candidate.candidate.skillPatch || {};
    const count = (key: string) => Array.isArray(patch[key]) ? patch[key].length : 0;
    return [
      `${tr("新增", "add")} ${count("addSections")}`,
      `${tr("替换", "replace")} ${count("replaceSections")}`,
      `${tr("删除", "remove")} ${count("removeSections")}`,
    ].join(" · ");
  }

  function knowledgeKind(entry: AgentKnowledgeEntry) {
    if (entry.patterns.knowledgeKind === "aggregate") return tr("多任务聚合", "Aggregated");
    if (entry.patterns.knowledgeKind === "external_source") return tr("公开来源卡片", "External source cards");
    return tr("单任务候选", "Task candidate");
  }

  function qualityLabel(entry: AgentKnowledgeEntry) {
    const score = Number(entry.patterns.qualityScore || 0);
    return score > 0 ? `${score}/100` : tr("旧版未评分", "Legacy unrated");
  }

  function canConvert(entry: AgentKnowledgeEntry) {
    return Boolean(entry.patterns.knowledgeKind) && Number(entry.patterns.qualityScore || 0) >= 70;
  }

  function candidateGate(candidate: AgentLearningCandidate) {
    const gate = candidate.candidate.qualityGate || {};
    const disposition = String(gate.disposition || "unknown");
    const score = Number(gate.score || 0);
    return `${disposition} · ${score}/100`;
  }

  function candidateProducer(candidate: AgentLearningCandidate) {
    const producer = candidate.candidate.producer || {};
    const model = String(producer.model || "legacy");
    const deployment = String(producer.deployment || "unknown");
    const normalizer = String(candidate.candidate.normalizerVersion || "legacy");
    return `${model} · ${deployment} · ${normalizer}`;
  }

  function knowledgeProvenance(entry: AgentKnowledgeEntry) {
    const support = (entry.patterns.support || {}) as Record<string, any>;
    const scans = Number(support.distinctScans || entry.patterns.sourceScans || 1);
    const models = Number(support.distinctModels || (entry.patterns.model ? 1 : 0));
    const normalizer = String(entry.patterns.normalizerVersion || "legacy");
    return `${scans} 个独立任务 · ${models || "—"} 个模型来源 · ${normalizer}`;
  }

  function formatBytes(value: number) {
    if (value < 1024) return `${value} B`;
    if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
    return `${(value / 1024 / 1024).toFixed(1)} MB`;
  }

  return {
    format, eventLabel, shortSession, candidateItems, patchSummary,
    knowledgeKind, qualityLabel, canConvert, candidateGate,
    candidateProducer, knowledgeProvenance, formatBytes,
  };
}
