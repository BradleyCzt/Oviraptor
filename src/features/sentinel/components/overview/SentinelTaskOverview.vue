<script setup lang="ts">
import { computed } from "vue";
import { Activity, Eye, Pause, Play, RefreshCw, Trash2 } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelScan } from "../../types";
import {
  createSentinelLabels, formatNumber, latestAttemptLabel, llmDeploymentClass,
  scanSummary, scanTitle, scanTokenTotal, uncachedInput,
} from "../../presentation";

const props = defineProps<{ scans: SentinelScan[]; projectId?: number; busyScanId: string }>();
const emit = defineEmits<{
  open: [scan: SentinelScan];
  pause: [scan: SentinelScan];
  resume: [scan: SentinelScan];
  retry: [scan: SentinelScan];
  remove: [scan: SentinelScan];
}>();
const { tr } = useI18n();
const { scanTypeLabel, statusLabel, llmDeploymentLabel, retryActionLabel } = createSentinelLabels(tr);

const taskGroups = computed(() => {
  const dates = new Map<
    string,
    { date: string; types: { type: string; scans: SentinelScan[] }[] }
  >();
  for (const scan of props.scans) {
    const raw = scan.createdAt || scan.updatedAt || "";
    const date = raw ? raw.slice(0, 10) : "未标注日期";
    let group = dates.get(date);
    if (!group) {
      group = { date, types: [] };
      dates.set(date, group);
    }
    const type = scan.scanType || "web";
    let bucket = group.types.find((item) => item.type === type);
    if (!bucket) {
      bucket = { type, scans: [] };
      group.types.push(bucket);
    }
    bucket.scans.push(scan);
  }
  const order = ["web", "code", "greybox", "cicd"];
  return [...dates.values()]
    .sort((a, b) => b.date.localeCompare(a.date))
    .map((group) => ({
      ...group,
      types: group.types.sort(
        (a, b) => order.indexOf(a.type) - order.indexOf(b.type),
      ),
    }));
});
</script>

<template>
  <section class="panel sentinel-panel">
    <div class="panel-heading">
      <div>
        <span class="eyebrow">LOADED TASKS</span>
        <h3>{{ tr("已加载任务", "Loaded tasks") }}</h3>
        <p class="task-overview-caption">
          仅展示已加载且符合当前项目与搜索条件的任务，不代表全部历史任务。
        </p>
      </div>
      <span class="project-scope-note">{{ props.projectId ? tr("跟随顶部当前项目", "Following current project") : tr("当前为全部项目汇总", "All-project summary") }}</span>
    </div>
    <div class="task-date-groups">
      <section
        v-for="group in taskGroups"
        :key="group.date"
        class="task-date-group"
      >
        <header>
          <strong>{{ group.date }}</strong
          ><span
            >{{
              group.types.reduce(
                (sum, bucket) => sum + bucket.scans.length,
                0,
              )
            }}
            个任务</span
          >
        </header>
        <div
          v-for="bucket in group.types"
          :key="bucket.type"
          class="task-type-group"
        >
          <div class="task-type-heading">
            <span class="scan-type-pill">{{
              scanTypeLabel(bucket.type)
            }}</span
            ><small>{{ bucket.scans.length }} 个</small>
          </div>
          <div class="sentinel-task-grid">
            <div
              v-for="scan in bucket.scans"
              :key="scan.id"
              class="sentinel-task-cell"
            >
              <article
                class="sentinel-task-card"
                role="button"
                tabindex="0"
                @click="emit('open', scan)"
                @keydown.enter="emit('open', scan)"
              >
                <header>
                  <span class="sentinel-status" :class="scan.status"
                    ><Activity :size="15" /></span
                  ><span class="scan-type-pill">{{
                    scanTypeLabel(scan.scanType)
                  }}</span
                  ><span class="llm-deployment-badge" :class="llmDeploymentClass(scan)" :title="scan.llmModel || undefined">
                    {{ llmDeploymentLabel(scan) }}
                  </span
                  ><span class="scan-attempt-result" :class="scan.latestAttemptStatus || scan.status">{{ latestAttemptLabel(scan, statusLabel) }}</span>
                </header>
                <h3>{{ scanTitle(scan) }}</h3>
                <p>{{ scan.projectName }} · {{ scan.id }}</p>
                <small class="task-date-line"
                  >创建于
                  {{ scan.createdAt || scan.updatedAt || "—" }}</small
                ><small
                  v-if="scanSummary(scan)"
                  class="live-checkpoint"
                  >{{
                    scanSummary(scan)
                  }}</small
                >
                <div class="task-token-usage" :class="{ zero: !scan.totalTokens }">
                  <span>{{ scan.totalTokens ? tr("输入", "Input") : tr("模型尚未产生 Token", "No model tokens yet") }}</span
                  ><strong>{{ formatNumber(scan.inputTokens) }}</strong>
                  <dl>
                    <div>
                      <dt>{{ tr("缓存", "Cached") }}</dt>
                      <dd>{{ formatNumber(scan.cachedTokens) }}</dd>
                    </div>
                    <div>
                      <dt>{{ tr("新增输入", "Uncached") }}</dt>
                      <dd>{{ formatNumber(uncachedInput(scan)) }}</dd>
                    </div>
                    <div>
                      <dt>{{ tr("输出", "Output") }}</dt>
                      <dd>{{ formatNumber(scan.outputTokens) }}</dd>
                    </div>
                    <div>
                      <dt>{{ tr("总计", "Total") }}</dt>
                      <dd>{{ formatNumber(scanTokenTotal(scan)) }}</dd>
                    </div>
                  </dl>
                </div>
                <footer class="task-card-actions">
                  <button
                    class="button ghost compact"
                    @click.stop="emit('open', scan)"
                  >
                    <Eye :size="13" /><span>{{
                      tr("查看结果", "Results")
                    }}</span></button
                  ><button
                    v-if="
                      scan.status === 'scanning' ||
                      scan.status === 'pausing'
                    "
                    class="button warning compact"
                    :disabled="busyScanId === scan.id"
                    @click.stop="emit('pause', scan)"
                  >
                    <Pause :size="13" /><span>{{
                      scan.status === "pausing"
                        ? tr("正在停止", "Stopping")
                        : tr("停止并保留", "Stop and preserve")
                    }}</span></button
                  ><button
                    v-else-if="scan.status === 'paused'"
                    class="button secondary compact"
                    :disabled="busyScanId === scan.id"
                    @click.stop="emit('resume', scan)"
                  >
                    <Play :size="13" /><span>{{
                      tr("继续扫描", "Resume")
                    }}</span></button
                  ><button
                    v-else-if="scan.status !== 'draft' && !scan.administrativeClosureRecorded"
                    class="button ghost compact"
                    :disabled="busyScanId === scan.id"
                    @click.stop="emit('retry', scan)"
                  >
                    <RefreshCw :size="13" /><span>{{
                      retryActionLabel(scan)
                    }}</span></button
                  ><button
                    v-if="!scan.administrativeClosureRecorded && !scan.closureHandoffRecorded"
                    class="button danger compact"
                    @click.stop="emit('remove', scan)"
                  >
                    <Trash2 :size="13" /><span>{{
                      tr("删除", "Delete")
                    }}</span>
                  </button>
                </footer>
              </article>
            </div>
          </div>
        </div>
      </section>
    </div>
    <div v-if="!taskGroups.length" class="empty-state">
      {{ tr("暂无任务", "No tasks") }}
    </div>
  </section>
</template>
