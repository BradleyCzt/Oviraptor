<script setup lang="ts">
import { computed } from "vue";
import { createSentinelLabels } from "../presentation";
import { useI18n } from "../../../i18n";
import type { SentinelScan } from "../types";
import type { WorkbenchMode } from "./useWorkbenchTaskCreation";

const props = defineProps<{
  scans: SentinelScan[];
  mode: Exclude<WorkbenchMode, "skills">;
  modeLabel: (mode: string) => string;
}>();
const emit = defineEmits<{ openScan: [scan: SentinelScan] }>();
const { tr } = useI18n();
const { statusLabel } = createSentinelLabels(tr);
const recentScans = computed(() => props.scans.filter((scan) => scan.scanType === props.mode).slice(0, 6));
const usage = computed(() => ["web", "code", "greybox", "cicd"].map((key) => {
  const rows = props.scans.filter((scan) => scan.scanType === key);
  return {
    key,
    input: rows.reduce((sum, scan) => sum + scan.inputTokens, 0),
    output: rows.reduce((sum, scan) => sum + scan.outputTokens, 0),
    total: rows.reduce((sum, scan) => sum + (scan.totalTokens || scan.inputTokens + scan.outputTokens), 0),
  };
}));
function format(value: number) {
  return new Intl.NumberFormat("zh-CN", {
    notation: value > 999999 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value || 0);
}
function deploymentLabel(scan: SentinelScan) {
  if (scan.llmDeployment === "local")
    return scan.llmFullPower ? tr("本地模型 · 火力全开", "Local model · full power") : tr("本地模型", "Local model");
  if (scan.llmDeployment === "cloud") return tr("云端 AI", "Cloud AI");
  return tr("模型未记录", "Model not recorded");
}
</script>

<template>
  <aside class="workbench-side overview-side">
    <section class="panel token-by-feature">
      <div class="panel-heading compact"><div><span class="eyebrow">TOKEN USAGE</span><h3>{{ tr("按功能消耗", "Usage by feature") }}</h3></div></div>
      <p class="overview-caption">{{ tr("当前已加载任务的累计 Token", "Cumulative tokens for currently loaded tasks") }}</p>
      <div v-for="item in usage" :key="item.key" class="token-feature-row">
        <header><span>{{ modeLabel(item.key) }}</span><strong>{{ format(item.total) }}</strong></header>
        <dl>
          <div><dt>{{ tr("输入", "Input") }}</dt><dd>{{ format(item.input) }}</dd></div>
          <div><dt>{{ tr("输出", "Output") }}</dt><dd>{{ format(item.output) }}</dd></div>
        </dl>
      </div>
    </section>
    <section class="panel recent-workbench">
      <div class="panel-heading compact"><div><span class="eyebrow">RECENT</span><h3>{{ tr("最近任务", "Recent tasks") }}</h3></div></div>
      <button v-for="scan in recentScans" :key="scan.id" type="button" @click="emit('openScan', scan)">
        <span class="overview-task-heading">
          <span class="overview-task-title">{{ scan.taskName || scan.projectName || tr("未命名任务", "Untitled task") }}</span>
          <span class="overview-task-status" :data-status="scan.status">{{ statusLabel(scan.status) }}</span>
        </span>
        <small class="overview-task-context">{{ deploymentLabel(scan) }} · {{ tr("总计", "Total") }} {{ format(scan.totalTokens) }} Token</small>
        <small class="overview-task-usage">{{ tr("输入", "Input") }} {{ format(scan.inputTokens) }} · {{ tr("输出", "Output") }} {{ format(scan.outputTokens) }}</small>
      </button>
      <div v-if="!recentScans.length" class="empty-state small">{{ tr("暂无此类任务", "No tasks of this type") }}</div>
    </section>
  </aside>
</template>

<style scoped src="./workbenchOverviewPresentation.css"></style>
