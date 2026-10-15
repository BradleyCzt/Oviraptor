<script setup lang="ts">
import { computed } from "vue";
import { Activity, Bug, Cpu, Network, ShieldAlert } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelOverviewStats, SentinelScan } from "../../types";
import { createSentinelLabels, formatCompactNumber, formatNumber } from "../../presentation";

// Read-only projection: the Board owns requests, retained snapshots and scope.
const props = defineProps<{
  stats: SentinelOverviewStats;
  overviewLoading: boolean;
  overviewError: boolean;
  scans: readonly Pick<SentinelScan, "status">[];
  runningScanCount: number;
  totalTokenUsage: number;
  totalRequestUsage: number;
  tokenScopeLabel: string;
}>();
const { tr } = useI18n();
const { statusLabel } = createSentinelLabels(tr);
const statsAvailable = computed(() => !props.overviewLoading && !props.overviewError);
const unavailableLabel = computed(() => props.overviewError
  ? tr("暂不可用", "Unavailable") : tr("核验中", "Verifying"));
const overviewBars = computed(() => [
  { label: tr("指纹", "Technology"), value: props.stats.fingerprintCount, color: "#4f7cff" },
  { label: "API", value: props.stats.apiCount, color: "#2f9dd7" },
  { label: tr("端点", "Endpoints"), value: props.stats.endpointCount, color: "#18a77b" },
  { label: tr("原生确认", "Native confirmed"), value: props.stats.reviewerConfirmedCount, color: "#e65b65" },
  { label: tr("人工验证记录", "Manual validation records"), value: props.stats.validatedCount, color: "#7957d5" },
]);
const overviewMax = computed(() => Math.max(1, ...overviewBars.value.map(item => item.value)));
const taskStatus = computed(() => [
  "draft", "queued", "scanning", "pausing", "paused", "recon_only", "completed", "partial", "failed",
].map(status => ({ status, count: props.scans.filter(scan => scan.status === status).length })));
</script>

<template>
  <div class="sentinel-kpis sentinel-kpis-v2">
    <article class="panel" :class="{ 'has-count-alert': statsAvailable && stats.readyOpportunityCount > 0 }">
      <ShieldAlert :size="18" /><span>{{ tr("可验证机会", "Verifiable opportunities") }}</span>
      <strong>{{ statsAvailable ? stats.readyOpportunityCount : unavailableLabel }}</strong>
      <small v-if="statsAvailable">{{ stats.opportunityCount }} {{ tr("个活跃候选", "active candidates") }}</small>
      <b v-if="statsAvailable && stats.readyOpportunityCount" class="kpi-count-alert">{{ stats.readyOpportunityCount }}</b>
    </article>
    <article class="panel" :class="{ 'has-count-alert': runningScanCount > 0 }">
      <Activity :size="18" /><span>{{ tr("已加载任务正在调查", "Loaded tasks investigating") }}</span>
      <strong>{{ runningScanCount }}</strong>
      <small v-if="statsAvailable">{{ stats.taskCount }} {{ tr("个历史任务", "historical tasks") }}</small>
      <small v-else>{{ tr("项目任务数：", "Project task count: ") }}{{ unavailableLabel }}</small>
      <b v-if="runningScanCount" class="kpi-count-alert">{{ runningScanCount }}</b>
    </article>
    <article class="panel">
      <Network :size="18" /><span>{{ tr("接口与端点", "APIs and endpoints") }}</span>
      <strong>{{ statsAvailable ? stats.apiCount + stats.endpointCount : unavailableLabel }}</strong>
      <small>{{ tr("运行时 + JS / AST + 发现", "Runtime + JS / AST + discovery") }}</small>
    </article>
    <article class="panel">
      <Bug :size="18" /><span>{{ tr("原生审核确认", "Native Reviewer confirmed") }}</span>
      <strong>{{ statsAvailable ? stats.reviewerConfirmedCount : unavailableLabel }}</strong>
      <small v-if="statsAvailable">
        <b class="risk-high">{{ stats.reviewerHighRiskCount }}</b>
        {{ tr("个高危/严重", "high / critical") }} · {{ stats.otherVulnerabilityCount }}
        {{ tr("条其他来源记录（未纳入原生确认）", "other-source records (not Native-confirmed)") }}
      </small>
      <small v-if="statsAvailable">{{ tr(`源码确认 ${stats.sourceReviewerConfirmedCount} 条 · 已核验 ${stats.sourceReviewAuditedTaskCount} 个源码任务`, `Source confirmed ${stats.sourceReviewerConfirmedCount} · ${stats.sourceReviewAuditedTaskCount} source tasks audited`) }}</small>
      <small v-if="statsAvailable">{{ tr(`尚无可用审查 ${stats.sourceReviewUnavailableTaskCount} · 无法核验 ${stats.sourceReviewUnverifiedTaskCount}`, `Review unavailable ${stats.sourceReviewUnavailableTaskCount} · unverified ${stats.sourceReviewUnverifiedTaskCount}`) }}</small>
      <small>{{ tr("仅统计当前轮次；候选审查不代表整体覆盖完成或系统无漏洞。", "Current attempts only; candidate review does not prove complete coverage or a vulnerability-free system.") }}</small>
      <small v-if="overviewError">{{ tr("统计读取失败，当前确认数暂不可用，请刷新重试。", "Stats could not be read; current confirmations are unavailable. Refresh to retry.") }}</small>
    </article>
    <article class="panel cumulative-token-kpi">
      <Cpu :size="18" /><span>{{ tr("已加载任务 Token", "Loaded-task tokens") }}</span>
      <strong class="token-kpi-value" :title="formatNumber(totalTokenUsage)">{{ formatCompactNumber(totalTokenUsage) }}</strong>
      <small>{{ formatNumber(totalRequestUsage) }} {{ tr("次模型请求", "model requests") }}</small>
      <small>{{ tr("已加载任务", "Loaded tasks") }} · {{ tokenScopeLabel }}</small>
    </article>
  </div>
  <div class="sentinel-overview-grid">
    <section class="panel sentinel-chart-card">
      <div class="panel-heading">
        <div>
          <span class="eyebrow">RESULT DISTRIBUTION</span>
          <h3>{{ tr("结构化结果分布", "Structured results") }}</h3>
          <p>{{ tr("长度代表记录数量，颜色代表数据类型，不代表风险。", "Bar length is record count; color identifies the data type, not risk.") }}</p>
        </div>
      </div>
      <div v-if="statsAvailable" class="sentinel-bar-chart">
        <div v-for="bar in overviewBars" :key="bar.label" class="bar-row">
          <span>{{ bar.label }}</span>
          <div><i :style="{ width: `${Math.max(3, (bar.value / overviewMax) * 100)}%`, background: bar.color }"></i></div>
          <strong>{{ bar.value }}</strong>
        </div>
      </div>
      <p v-else role="status">{{ unavailableLabel }} · {{ tr("统计确认后再显示分布。", "Distribution will be shown after statistics are verified.") }}</p>
    </section>
    <section class="panel sentinel-chart-card">
      <div class="panel-heading">
        <div>
          <span class="eyebrow">TASK STATUS</span>
          <h3>{{ tr("已加载任务状态", "Loaded-task status") }}</h3>
          <p>{{ tr("仅统计已加载任务，不代表项目全部任务。", "Counts loaded tasks only, not all tasks in the project.") }}</p>
        </div>
      </div>
      <div class="task-status-chart">
        <div v-for="item in taskStatus" :key="item.status">
          <span :class="`task-dot ${item.status}`"></span><em>{{ statusLabel(item.status) }}</em><strong>{{ item.count }}</strong>
        </div>
      </div>
    </section>
  </div>
</template>
