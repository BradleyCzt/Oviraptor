<script setup lang="ts">
import { computed } from "vue";
import type { AgentTraceEvent, AgentTimelineItem } from "../types";
import { chatDecisionDisplay } from "../timeline/rootDecisionContract";
import { useI18n } from "../../../i18n";
import { decisionSuggestion, localToolLabel, rootDecisionDisplay } from "../traces/rootDecisionContract";

const props = defineProps<{ event?: AgentTraceEvent; sourceAuthority?: string; chatItem?: AgentTimelineItem }>();
const { tr } = useI18n();
const decision = computed(() => props.chatItem ? chatDecisionDisplay(props.chatItem)
  : props.event ? rootDecisionDisplay(props.event, props.sourceAuthority ?? "") : undefined);
const sections = computed(() => decision.value ? [
  { key: "observed", title: tr("观察", "Observations"), values: decision.value.summary.observed,
    empty: tr("未提供观察", "No observations supplied") },
  { key: "missing", title: tr("缺口", "Missing evidence"), values: decision.value.summary.missing,
    empty: tr("未提供缺口", "No gaps supplied") },
  { key: "suggestions", title: tr("下一步建议", "Suggested next steps"),
    values: decision.value.summary.suggestions.map(text => decisionSuggestion(text, tr)),
    empty: tr("未提供下一步建议", "No next steps supplied") },
  { key: "risks", title: tr("风险与局限", "Risks and limitations"), values: decision.value.summary.risks,
    empty: tr("未提供风险说明；不代表没有风险", "No risks supplied; this does not imply no risk") },
] : []);
</script>

<template>
  <section class="root-decision-summary" :aria-label="tr('Root 决策记录', 'Root decision record')">
    <template v-if="decision">
      <header><strong>{{ tr("已记录决策摘要", "Recorded decision summary") }}</strong>
        <span>{{ tr("第", "Round ") }} {{ decision.round }} {{ tr("轮 · 建议", " · advisory") }}</span></header>
      <p v-if="decision.humanDirective" class="decision-human-context">{{ tr("已确认指令", "Confirmed directive") }} · {{ tr("第", "Revision ") }} {{ decision.humanDirective.revision }} {{ tr("版", "") }}</p>
      <p class="decision-boundary">{{ tr("建议不授予执行权限，也不代表已派发或完成。观察来自本轮保存的摘要，具体结论仍以原证据和执行回执为准。", "Suggestions grant no execution authority and do not prove dispatch or completion. Observations are saved summaries; original evidence and execution receipts remain decisive.") }}</p>
      <section v-if="decision.localTools?.length" class="decision-local-tools" :aria-label="tr('已记录本地工具', 'Recorded local tools')">
        <h4>{{ tr("已记录本地工具", "Recorded local tools") }} <small>{{ tr("仅本地记录", "Local record only") }}</small></h4>
        <ul><li v-for="(name, index) in decision.localTools" :key="index"><span class="tool-record-dot" aria-hidden="true"></span>{{ localToolLabel(name, tr) }}</li></ul>
      </section>
      <div class="decision-sections">
        <section v-for="section in sections" :key="section.key" :class="section.key">
          <h4>{{ section.title }}</h4>
          <ul v-if="section.values.length"><li v-for="(value, index) in section.values" :key="index">{{ value }}</li></ul>
          <p v-else class="decision-empty">{{ section.empty }}</p>
        </section>
      </div>
      <section class="decision-usage"><h4>{{ tr("本轮账本用量", "Recorded round usage") }}</h4>
        <p v-if="decision.usage"><b>{{ decision.usage.totalTokens }} tokens</b> · {{ decision.usage.modelRequests }} {{ tr("次模型请求", "model request") }}
          <small>{{ tr("输入", "Input") }} {{ decision.usage.inputTokens }} · {{ tr("缓存输入", "Cached input") }} {{ decision.usage.cachedInputTokens }} · {{ tr("输出", "Output") }} {{ decision.usage.outputTokens }}</small></p>
        <p v-else class="decision-empty">{{ tr("用量记录无法核验；费用状态未知。", "Usage cannot be verified; cost status is unknown.") }}</p>
        <small>{{ tr("未核验单价，未换算金额；不使用模型费用建议替代账本。", "No verified price or monetary conversion; model cost notes do not replace the ledger.") }}</small>
      </section>
      <details class="decision-cost-notes"><summary>{{ tr("费用说明建议", "Advisory cost notes") }}</summary>
        <ul v-if="decision.summary.costNotes.length"><li v-for="(note, index) in decision.summary.costNotes" :key="index">{{ note }}</li></ul>
        <p v-else>{{ tr("未提供费用说明建议", "No advisory cost notes supplied") }}</p>
      </details>
    </template>
    <p v-else class="decision-empty" role="status">{{ tr("摘要不可核验：记录可能属于旧轮次、历史导入、缺少字段或限长预览。未将正文作为决策展示；这不代表任务失败或完成。", "Summary cannot be verified: older, historical, incomplete or truncated records are possible. Its body is withheld; this implies neither failure nor completion.") }}</p>
  </section>
</template>

<style scoped>
.root-decision-summary { grid-column: 2 / -1; min-width: 0; margin: 8px 0 12px; padding: 14px; border: 1px solid var(--line); border-radius: 10px; background: var(--app-surface, #fff); color: var(--app-ink, #202327); font-size: 12px; line-height: 1.65; }
.root-decision-summary header { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
.root-decision-summary header strong { font-size: 13px; white-space: normal; color: var(--app-ink, #202327); }
.root-decision-summary header span, .decision-boundary, .decision-empty, .root-decision-summary small { color: var(--app-muted, var(--muted, #6c727d)); font-size: 11px; }
.decision-boundary { margin: 8px 0 12px; }
.decision-human-context { display: inline-flex; margin: 8px 0 2px; padding: 3px 9px; border: 1px solid var(--line); border-radius: 6px; color: var(--app-muted, var(--muted, #6c727d)); background: var(--app-bg, transparent); font-size: 11px; }
.decision-local-tools { margin: 12px 0; padding: 10px 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--app-bg, transparent); }
.decision-local-tools h4 { display: flex; align-items: baseline; flex-wrap: wrap; gap: 8px; }
.root-decision-summary .decision-local-tools ul { display: flex; flex-wrap: wrap; gap: 6px; padding: 0; list-style: none; }
.root-decision-summary .decision-local-tools li, .root-decision-summary .decision-local-tools li + li { display: inline-flex; align-items: center; gap: 6px; margin: 0; padding: 3px 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--app-surface, #fff); }
.tool-record-dot { flex: 0 0 5px; width: 5px; height: 5px; border-radius: 50%; background: var(--accent, #5b8ff9); }
.decision-sections { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px 20px; }
.root-decision-summary h4 { margin: 0 0 5px; font-size: 12px; font-weight: 650; }
.root-decision-summary ul { margin: 0; padding-left: 18px; overflow-wrap: anywhere; }
.root-decision-summary li + li { margin-top: 5px; }
.root-decision-summary p { margin: 5px 0; overflow-wrap: anywhere; }
.decision-usage { margin-top: 12px; padding-top: 10px; border-top: 1px solid var(--line); }
.decision-usage small { display: block; margin-top: 3px; }
.decision-cost-notes { margin-top: 10px; }
.decision-cost-notes summary { cursor: pointer; color: var(--app-muted, var(--muted, #6c727d)); font-size: 11px; }
.decision-cost-notes summary:focus-visible { outline: 2px solid var(--accent, #5b8ff9); outline-offset: 3px; border-radius: 3px; }
@media (max-width: 760px) { .decision-sections { grid-template-columns: 1fr; gap: 12px; } .root-decision-summary { padding: 12px; } }
</style>
