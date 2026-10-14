<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "../../../../i18n";
import type { InvestigationOverview } from "../../types";
import type { InvestigationSummaryState } from "../../overview/useInvestigationSummary";
import { formatCompactNumber, formatNumber } from "../../presentation";

// Presentation only. Requests, snapshots and execution authority stay upstream.
const props = defineProps<{
  readyOpportunityCount: number;
  overviewLoading: boolean;
  overviewError: boolean;
  runningScanCount: number;
  totalRequestUsage: number;
  totalTokenUsage: number;
  tokenScopeLabel: string;
  investigationState: InvestigationSummaryState;
  investigationStats: Pick<InvestigationOverview, "averageInformationGain" |
    "tokenWorthyCount" | "targetCount" | "factCount" | "promotedStrategyCount">;
}>();
const { tr } = useI18n();
const opportunityCount = computed(() => props.overviewError
  ? tr("暂不可用", "Unavailable") : props.overviewLoading
    ? tr("核验中", "Verifying") : props.readyOpportunityCount);
</script>

<template>
  <aside class="investigation-sidebar">
    <section class="panel investigation-pipeline">
      <span class="eyebrow">WORKFLOW GUIDE</span>
      <h3>{{ tr("流程说明（非执行进度）", "Workflow guide (not execution progress)") }}</h3>
      <ol>
        <li><b>1</b><div><strong>{{ tr("渲染与功能触发", "Rendering and interactions") }}</strong><small>{{ tr("导航、标签、菜单、详情控件", "Navigation, tabs, menus and detail controls") }}</small></div></li>
        <li><b>2</b><div><strong>HTTP / {{ tr("参数", "Parameters") }} / JS</strong><small>{{ tr("运行时请求与静态 AST", "Runtime requests and static AST") }}</small></div></li>
        <li><b>3</b><div><strong>{{ tr("指纹与本地知识", "Fingerprints and local knowledge") }}</strong><small>{{ tr("关联已采集的证据", "Correlate collected evidence") }}</small></div></li>
        <li><b>4</b><div><strong>{{ tr("审查与收口", "Review and closure") }}</strong><small>{{ tr("查看任务实际结论与停止原因", "Consult actual task conclusions and stop reasons") }}</small></div></li>
      </ol>
    </section>
    <section class="panel investigation-budget">
      <span class="eyebrow">COST & INVESTIGATION SUMMARY</span>
      <h3>{{ tr("成本与调查摘要", "Cost and investigation summary") }}</h3>
      <p>{{ tr("模型用量：已加载任务", "Model usage: loaded tasks") }} · {{ tokenScopeLabel }}</p>
      <dl>
        <div><dt>{{ tr("已加载活跃任务", "Loaded active tasks") }}</dt><dd>{{ runningScanCount }}</dd></div>
        <div><dt>{{ tr("模型请求", "Model requests") }}</dt><dd>{{ formatNumber(totalRequestUsage) }}</dd></div>
        <div><dt>{{ tr("总 Token", "Total tokens") }}</dt><dd :title="formatNumber(totalTokenUsage)">{{ formatCompactNumber(totalTokenUsage) }}</dd></div>
        <div><dt>{{ tr("项目可验证机会", "Project verifiable opportunities") }}</dt><dd>{{ opportunityCount }}</dd></div>
      </dl>
      <p>{{ tr("已加载调查摘要（非实时状态）", "Loaded investigation summary (not live state)") }}</p>
      <dl v-if="investigationState === 'ready'">
        <div><dt>{{ tr("平均信息增益", "Average information gain") }}</dt><dd>{{ investigationStats.averageInformationGain }}/100</dd></div>
        <div><dt>{{ tr("允许模型目标", "Model-eligible targets") }}</dt><dd>{{ investigationStats.tokenWorthyCount }}/{{ investigationStats.targetCount }}</dd></div>
        <div><dt>{{ tr("确定性事实", "Deterministic facts") }}</dt><dd>{{ investigationStats.factCount }}</dd></div>
        <div><dt>{{ tr("已晋升策略", "Promoted strategies") }}</dt><dd>{{ investigationStats.promotedStrategyCount }}</dd></div>
      </dl>
      <p v-else role="status">{{ investigationState === 'loading'
        ? tr("调查摘要核验中", "Verifying investigation summary")
        : investigationState === 'error' ? tr("调查摘要暂不可用", "Investigation summary unavailable")
          : tr("调查摘要尚未加载", "Investigation summary not loaded") }}</p>
      <p>{{ tr("此处仅展示统计与流程说明；当前任务的授权、预算和停止原因以任务详情中的实际记录为准。", "This panel shows statistics and a workflow guide only. Consult task details for actual authorization, budgets and stop reasons.") }}</p>
    </section>
  </aside>
</template>
