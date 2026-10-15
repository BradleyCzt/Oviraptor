<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { Activity, Archive, Eye, MessageCircle, Pause, Play, RefreshCw, RotateCcw, Trash2, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelScan } from "../types";
import { sentinelApi } from "../api";
import { useTaskExecutionDetails } from "../execution/useTaskExecutionDetails";
import { useHistoricalImportLedger } from "../results/useHistoricalImportLedger";
import { validTaskPage } from "../results/taskPageContract";
import AuthorizationControlForm from "./AuthorizationControlForm.vue";
import NativeProcessLogView from "./NativeProcessLogView.vue";
import NativeSdkLogView from "./NativeSdkLogView.vue";
import SourceReviewEvidence from "./SourceReviewEvidence.vue";
import HistoricalJsonImport from "./HistoricalJsonImport.vue";
import {
  createSentinelLabels,
  formatCompactNumber,
  formatNumber,
  latestAttemptLabel,
  routeModeLabel,
  scanInterruption,
  scanSummary,
  scanTitle,
} from "../presentation";

type PreviewTarget = { company: string; url: string; highValue: boolean };
const props = defineProps<{
  scans: SentinelScan[];
  projectId?: number;
  hasMore: boolean;
  loadingMore: boolean;
  preview?: SentinelScan;
  previewTargets: PreviewTarget[];
  attentionCount: number;
  totalTokens: number;
  totalRequests: number;
  tokenScopeLabel: string;
  zeroYieldCount: number;
  zeroYieldTokens: number;
  cacheHitRate: number;
  controlBusy: string;
  highValueCount: (scanId: string) => number;
}>();
const emit = defineEmits<{
  preview: [scan: SentinelScan];
  close: [];
  confirm: [scan: SentinelScan];
  pause: [scan: SentinelScan];
  resume: [scan: SentinelScan];
  retry: [scan: SentinelScan];
  remove: [scan: SentinelScan];
  open: [scan: SentinelScan];
  dialog: [scan: SentinelScan];
  loadMore: [];
  archived: [scan: SentinelScan];
}>();
const { tr } = useI18n();
const { statusLabel, retryActionLabel, scanTypeLabel, llmDeploymentLabel } = createSentinelLabels(tr);
const view = ref<"attention" | "history" | "all">("attention");
const {
  importedRuns, importedSelected, importedPreviews, importedLoading, importedPreviewLoading,
  importedError, importedHasMore, importedLoaded, loadImportedRuns, selectImportedRun,
} = useHistoricalImportLedger(view);
let disposed = false;
let archiveSerial = 0;
onBeforeUnmount(() => {
  disposed = true;
  ++archiveSerial;
  ++searchSerial;
  if (searchTimer !== undefined) clearTimeout(searchTimer);
  searchTimer = undefined;
});
const search = ref("");
const searchResults = ref<SentinelScan[]>([]);
const searchLoading = ref(false);
const searchError = ref("");
const archiveBusy = ref("");
watch(() => props.projectId, () => {
  ++archiveSerial;
  archiveBusy.value = "";
}, { flush: "sync" });
const searchHasMore = ref(false);
const searchCursor = ref<Pick<SentinelScan, "updatedAt" | "id">>();
const searchPageSize = 100;
let searchSerial = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
async function loadSearchPage(older = false) {
  if (disposed || !search.value.trim() || (older && (!searchHasMore.value || searchLoading.value))) return;
  const serial = ++searchSerial;
  const query = search.value.trim();
  const currentView = view.value;
  const projectId = props.projectId;
  const cursor = older ? searchCursor.value : undefined;
  searchLoading.value = true;
  searchError.value = "";
  try {
    const page = await sentinelApi.searchSentinelScanPage(projectId, query, currentView, searchPageSize, cursor);
    if (serial !== searchSerial || query !== search.value.trim() || currentView !== view.value || projectId !== props.projectId) return;
    if (!validTaskPage(page, searchPageSize, projectId)) throw new Error("task_search_invalid_page");
    searchResults.value = older ? [...searchResults.value, ...page] : page;
    const last = page[page.length - 1];
    searchCursor.value = last ? { id: last.id, updatedAt: last.updatedAt } : undefined;
    searchHasMore.value = page.length === searchPageSize;
  } catch {
    if (serial === searchSerial) searchError.value = "全库搜索失败，请重试";
  } finally {
    if (serial === searchSerial) searchLoading.value = false;
  }
}
watch(() => [search.value, view.value, props.projectId] as const, () => {
  ++searchSerial;
  if (searchTimer !== undefined) clearTimeout(searchTimer);
  searchTimer = undefined;
  searchResults.value = [];
  searchCursor.value = undefined;
  searchHasMore.value = false;
  searchError.value = "";
  searchLoading.value = false;
  if (search.value.trim()) searchTimer = setTimeout(() => { searchTimer = undefined; void loadSearchPage(); }, 250);
});
function loadOlderTasks() {
  if (search.value.trim()) void loadSearchPage(true);
  else emit("loadMore");
}
async function toggleArchive(scan: SentinelScan) {
  if (disposed || !scan.projectId || archiveBusy.value
    || (props.projectId !== undefined && scan.projectId !== props.projectId)) return;
  const { id, projectId } = scan;
  const serial = ++archiveSerial;
  const searchVersion = searchSerial;
  archiveBusy.value = id;
  searchError.value = "";
  try {
    const updated = await sentinelApi.archiveSentinelScan(id, projectId, !scan.archivedAt);
    if (serial !== archiveSerial) return;
    if (!updated || updated.id !== id || updated.projectId !== projectId) throw new Error("archive_scope_mismatch");
    if (searchVersion === searchSerial) {
      searchResults.value = searchResults.value.map((item) => item.id === id && item.projectId === projectId ? updated : item);
    }
    emit("archived", updated);
  } catch {
    if (serial === archiveSerial && searchVersion === searchSerial) searchError.value = "任务归档更新失败，请刷新后重试";
  } finally {
    if (serial === archiveSerial) archiveBusy.value = "";
  }
}
const {
  detailTab, detailLoading, detailErrors, attempts, selectedAttempt,
  runnerLog, runnerLogLoading, runnerLogError, runnerLogConnectionError, mailboxPage, mailboxLoading, mailboxError,
  toolPage, toolLoading, toolError, nativeStatus, historicalPreviews, learningCandidates, knowledge,
  loadTools, loadMailbox, loadAttemptLog, chooseAttempt,
} = useTaskExecutionDetails(props);
const historicalStatuses = new Set(["completed", "completed_with_gaps", "cancelled", "failed"]);
const isHistorical = (scan: SentinelScan) => historicalStatuses.has(scan.status);
const historyCount = computed(() => props.scans.filter(isHistorical).length);
const attentionCountLocal = computed(() => props.scans.length - historyCount.value);
const visibleScans = computed(() => (search.value.trim() ? searchResults.value : props.scans).filter((scan) => {
  if (view.value === "history" && !isHistorical(scan)) return false;
  if (view.value === "attention" && isHistorical(scan)) return false;
  return true;
}).sort((left, right) => Number(Boolean(left.archivedAt)) - Number(Boolean(right.archivedAt))));
</script>

<template>
  <div class="task-cost-strip task-center-cost">
      <article><span>当前任务</span><strong>{{ scans.length }}</strong><small>{{ attentionCount }} 个需要确认、继续或复盘</small></article>
      <article>
        <span>{{ tr("已加载任务 Token", "Loaded-task tokens") }}</span>
        <strong>{{ formatCompactNumber(totalTokens) }}</strong>
        <small>{{ formatNumber(totalRequests) }} 次模型请求</small>
        <small>{{ tr("已加载任务", "Loaded tasks") }} · {{ tokenScopeLabel }}</small>
        <small>{{ tr("用量沿用概览部署范围，不随下方搜索或状态筛选改变。", "Usage follows the overview deployment scope, not the search or status filters below.") }}</small>
      </article>
      <article :class="{ warning: zeroYieldCount }">
        <span>{{ tr("未关联漏洞记录", "No linked finding records") }}</span>
        <strong>{{ zeroYieldCount }}</strong>
        <small>{{ formatNumber(zeroYieldTokens) }} Token · {{ tr("当前索引未关联记录，不代表无漏洞或执行完整。", "No records in the current association index; not proof of no vulnerabilities or complete execution.") }}</small>
      </article>
      <article>
        <span>缓存命中率</span><strong>{{ cacheHitRate }}%</strong>
        <small>{{ tr("缓存输入 / 输入总计；不作为执行质量结论", "Cached input / total input; not an execution-quality verdict") }}</small>
      </article>
  </div>
  <div class="sentinel-queue-layout task-center-stack">
      <section class="panel sentinel-panel">
        <div class="panel-heading">
          <div><span class="eyebrow">TASK CENTER</span><h3>任务中心</h3><p>进行中与待处理任务留在工作台；完整结束的任务进入历史视图，证据与对话仍可查看。</p></div>
        </div>
        <div class="task-view-controls">
          <div class="segmented" role="group" aria-label="任务视图">
            <button type="button" :class="{ active: view === 'attention' }" @click="view = 'attention'">待处理 <span :title="hasMore ? '仅已加载任务数量' : '全部任务数量'">{{ attentionCountLocal }}{{ hasMore ? '+' : '' }}</span></button>
            <button type="button" :class="{ active: view === 'history' }" @click="view = 'history'">已结束 <span :title="hasMore ? '仅已加载任务数量' : '全部任务数量'">{{ historyCount }}{{ hasMore ? '+' : '' }}</span></button>
            <button type="button" :class="{ active: view === 'all' }" @click="view = 'all'">全部 <span :title="hasMore ? '仅已加载任务数量' : '全部任务数量'">{{ scans.length }}{{ hasMore ? '+' : '' }}</span></button>
          </div>
          <input v-model="search" type="search" aria-label="全库搜索任务" placeholder="全库搜索任务、目标或编号" />
        </div>
        <small v-if="searchError" role="alert">{{ searchError }}</small>
        <small v-if="searchLoading" role="status">正在搜索完整任务历史…</small>
        <div class="task-center-list">
          <article v-for="scan in visibleScans" :key="scan.id" :class="{ active: preview?.id === scan.id }" @click="emit('preview', scan)">
            <span class="sentinel-status" :class="scan.status"><Activity :size="15" /></span>
            <div>
              <button type="button" class="task-title-button" @click.stop="emit('preview', scan)">{{ scanTitle(scan) }}</button><b v-if="scan.archivedAt" class="status-chip">已归档</b><b v-if="highValueCount(scan.id)" class="high-value-badge">高 {{ highValueCount(scan.id) }}</b>
              <small>{{ scan.projectName }} · {{ scan.id }}</small>
              <em>{{ scanTypeLabel(scan.scanType) }}<template v-if="scan.scanType === 'web'"> · 任务模式：{{ routeModeLabel(scan.requestedScanMode || 'standard') }}</template> · {{ llmDeploymentLabel(scan) }} · {{ latestAttemptLabel(scan, statusLabel) }} · {{ scan.updatedAt }}</em>
              <div v-if="scanInterruption(scan)" class="task-stop-summary" :class="scanInterruption(scan)?.tone"><b>{{ scanInterruption(scan)?.title }}</b><span>{{ scanInterruption(scan)?.detail }}</span><small>{{ scanInterruption(scan)?.action }}</small></div>
            </div>
            <div class="task-center-actions">
              <button v-if="scan.status === 'draft'" class="button primary compact" :disabled="Boolean(scan.archivedAt)" @click.stop="emit('confirm', scan)"><Play :size="13" />确认扫描</button>
              <button v-if="scan.status === 'scanning' || scan.status === 'pausing'" class="button warning compact" :disabled="controlBusy === scan.id" @click.stop="emit('pause', scan)"><Pause :size="13" />{{ scan.status === "pausing" ? "正在停止" : "停止并保留" }}</button>
              <button v-else-if="scan.status === 'paused' || scan.status === 'partial'" class="button secondary compact" :disabled="controlBusy === scan.id || Boolean(scan.archivedAt)" @click.stop="emit('resume', scan)"><Play :size="13" />{{ scan.status === 'partial' ? '继续未完成' : '继续扫描' }}</button>
              <button v-else-if="scan.status !== 'draft' && !scan.administrativeClosureRecorded" class="button ghost compact" :disabled="controlBusy === scan.id || Boolean(scan.archivedAt)" @click.stop="emit('retry', scan)"><RefreshCw :size="13" />{{ retryActionLabel(scan) }}</button>
              <button v-if="scan.scanType === 'web' && scan.status !== 'draft'" class="button ghost compact" @click.stop="emit('dialog', scan)"><MessageCircle :size="13" />协作</button>
              <button v-if="scan.archivedAt || isHistorical(scan)" class="button ghost compact" :disabled="archiveBusy === scan.id || !scan.projectId" @click.stop="toggleArchive(scan)"><RotateCcw v-if="scan.archivedAt" :size="13" /><Archive v-else :size="13" />{{ scan.archivedAt ? '恢复归档' : '归档' }}</button>
              <button v-if="!scan.administrativeClosureRecorded && !scan.closureHandoffRecorded" class="button danger compact" @click.stop="emit('remove', scan)"><Trash2 :size="13" />{{ ["scanning", "pausing"].includes(scan.status) ? "强制删除" : "删除" }}</button>
            </div>
          </article>
          <div v-if="!visibleScans.length && !searchLoading && !searchError" class="empty-state">{{ search ? '全库没有匹配的任务，请换个关键词。' : scans.length ? '当前筛选没有任务，切换视图即可查看历史记录。' : '暂无任务' }}</div>
          <button v-if="search ? searchHasMore : hasMore" type="button" class="button ghost" :disabled="search ? searchLoading : loadingMore" @click="loadOlderTasks">{{ searchLoading || loadingMore ? '正在读取…' : '加载更早任务' }}</button>
        </div>
      </section>
      <section v-if="view !== 'attention'" class="panel sentinel-panel" aria-label="历史导入产物">
        <div class="panel-heading">
          <div><span class="eyebrow">IMPORTED HISTORY</span><h3>历史产物 · 只读</h3><p>旧 JSON 等产物独立于活动任务展示；候选与覆盖记录未经当前 Reviewer 审核，不计入 Native 已确认漏洞或执行覆盖。</p></div>
          <button type="button" class="button ghost compact" :disabled="importedLoading" @click="loadImportedRuns()"><RefreshCw :size="13" />刷新</button>
        </div>
        <HistoricalJsonImport @imported="loadImportedRuns(false, true)" />
        <p v-if="importedError" role="alert">{{ importedError }}</p>
        <p v-if="importedLoading && !importedLoaded" role="status">正在读取历史导入记录…</p>
        <p v-if="importedLoaded && !importedRuns.length">没有可展示的历史导入记录。</p>
        <p v-if="projectId !== undefined || search.trim()">此清单是全局导入台账，不受上方任务项目筛选或任务搜索影响。</p>
        <div class="task-center-list">
          <article v-for="run in importedRuns" :key="run.rowId" :class="{ active: importedSelected?.rowId === run.rowId }">
            <span class="sentinel-status completed"><Archive :size="15" /></span>
            <div>
              <strong>{{ run.title || run.scanId || '未命名历史产物' }}</strong><b class="status-chip">未审核 · 只读</b>
              <small>{{ run.scanId }} · 记录状态 {{ run.status }} · 第 {{ run.attemptNumber }} 次</small>
              <em>候选 {{ run.findingCandidates }} · 覆盖记录 {{ run.coverageRecords }} · 导入 {{ run.importedAt }}</em>
            </div>
            <div class="task-center-actions"><button type="button" class="button ghost compact" @click="selectImportedRun(run)"><Eye :size="13" />{{ importedSelected?.rowId === run.rowId ? '收起预览' : '查看摘要' }}</button></div>
          </article>
        </div>
        <button v-if="importedHasMore" type="button" class="button ghost compact" :disabled="importedLoading" @click="loadImportedRuns(true)">加载更早的导入产物</button>
        <section v-if="importedSelected" class="task-attempt-log" aria-label="历史产物只读摘要">
          <header><strong>{{ importedSelected.title || importedSelected.scanId }} · 只读摘要</strong></header>
          <p v-if="importedPreviewLoading" role="status">正在读取脱敏摘要…</p>
          <p v-else-if="!importedPreviews.length">暂无当前可展示记录；已撤销的投影不会显示。</p>
          <ol v-else class="task-detail-records task-mailbox-records">
            <li v-for="item in importedPreviews" :key="item.membershipId"><strong>{{ item.title || item.kind }}{{ item.severity ? ` · ${item.severity}` : '' }}</strong><span v-if="item.target">{{ item.target }}</span><small>{{ item.producer }} · {{ item.kind }} · 未审核 / 只读</small></li>
          </ol>
          <small v-if="importedPreviews.length === 300">这里只展示前 300 条脱敏摘要；完整清单仍保留在导入台账中。</small>
        </section>
      </section>
      <aside v-if="preview" class="panel task-preview task-preview-inline">
          <header><div><span class="eyebrow">TASK PREVIEW</span><h3>{{ scanTitle(preview) }}</h3><div class="task-preview-facts"><span class="status-chip" :class="preview.latestAttemptStatus || preview.status">{{ latestAttemptLabel(preview, statusLabel) }}</span><span>{{ scanTypeLabel(preview.scanType) }}</span><span v-if="preview.scanType === 'web'">任务模式：{{ routeModeLabel(preview.requestedScanMode || 'standard') }}</span><code :title="preview.id">{{ preview.id }}</code></div></div><button class="icon-button" type="button" aria-label="关闭任务详情" @click="emit('close')"><X :size="15" /></button></header>
          <div v-if="detailTab === 'overview'" class="preview-url-list">
            <strong>任务目标 · {{ previewTargets.length }}</strong>
            <div v-for="row in previewTargets" :key="row.url" class="preview-target-row"><span>{{ row.company }}</span><code>{{ row.url }}</code><b v-if="row.highValue" class="high-value-badge">高</b></div>
            <div v-if="!previewTargets.length" class="empty-state small">没有可预览目标</div>
          </div>
          <div v-if="detailTab === 'overview' && scanInterruption(preview)" class="task-stop-summary preview-stop-summary" :class="scanInterruption(preview)?.tone"><b>{{ scanInterruption(preview)?.title }}</b><span>{{ scanInterruption(preview)?.detail }}</span><small>{{ scanInterruption(preview)?.action }}</small></div>
          <p v-else-if="detailTab === 'overview'" class="task-preview-summary">{{ scanSummary(preview) || (preview.status === 'completed' ? '本轮已结束，证据、执行尝试与协作消息仍保存在任务中。' : '尚无执行摘要。') }}</p>
          <AuthorizationControlForm v-if="detailTab === 'overview' && preview.status === 'draft' && preview.scanType === 'web'" :scan-id="preview.id" :targets="previewTargets" />
          <details v-if="detailTab === 'overview'" class="task-technical"><summary>任务文件与运行信息</summary><code>{{ preview.taskPath || "确认扫描后生成" }}</code></details>
          <footer>
            <button v-if="preview.status === 'draft'" class="button primary" @click="emit('confirm', preview)"><Play :size="14" />确认并启动扫描</button>
            <button class="button ghost" @click="emit('open', preview)"><Eye :size="14" />打开结果页</button>
            <button v-if="preview.scanType === 'web' && preview.status !== 'draft'" class="button ghost" @click="emit('dialog', preview)"><MessageCircle :size="14" />查看协作对话</button>
          </footer>
          <div class="task-detail-tabs" role="tablist" aria-label="单任务详情">
            <button v-for="tab in (['overview', 'execution', 'evidence', 'learning'] as const)" :key="tab" type="button" role="tab" :aria-selected="detailTab === tab" :class="{ active: detailTab === tab }" @click="detailTab = tab">{{ { overview: '概况', execution: '执行', evidence: '证据', learning: '学习' }[tab] }}</button>
          </div>
          <div class="task-detail-body" role="tabpanel">
            <p v-if="detailLoading">正在读取任务记录…</p>
            <p v-if="detailErrors.length" class="task-detail-error" role="alert">{{ detailErrors.join('；') }}</p>
            <template v-if="detailTab === 'overview'">
              <p>任务编号 {{ preview.id }} · {{ attempts.length }} 次执行 · {{ formatNumber(preview.totalTokens) }} Token。切换标签不会创建或重启任务。</p>
              <p v-if="nativeStatus?.stopDiagnostic && nativeStatus.stopDiagnostic.code !== 'in_progress'">停止原因：{{ nativeStatus.stopDiagnostic.code }} · 下一步：{{ nativeStatus.stopDiagnostic.nextAction }} · 不会自动续跑。</p>
            </template>
            <template v-else-if="detailTab === 'execution'">
              <p v-if="!detailLoading && !attempts.length">尚无执行尝试；草稿任务不会产生运行记录。</p>
              <ol v-else class="task-detail-records">
                <li v-for="attempt in attempts" :key="attempt.attemptNumber"><strong>第 {{ attempt.attemptNumber }} 次 · {{ statusLabel(attempt.status) }}</strong><span>{{ attempt.executionMode }} · {{ attempt.stage }} · {{ attempt.checkpoint || '无检查点' }}</span><small>{{ attempt.startedAt }} → {{ attempt.finishedAt || '进行中' }} · {{ attempt.stopReason || '无停止原因' }}</small><small>{{ formatNumber(attempt.totalTokens) }} Token · {{ attempt.llmRequests }} 次模型请求</small><button type="button" class="button ghost compact" :aria-pressed="selectedAttempt === attempt.attemptNumber" @click="chooseAttempt(attempt.attemptNumber)">{{ selectedAttempt === attempt.attemptNumber ? '当前查看此轮' : '查看此轮日志' }}</button></li>
              </ol>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" aria-label="选中执行轮次的日志">
                <header><strong>第 {{ selectedAttempt }} 次执行 · 日志尾部</strong><button type="button" class="button ghost compact" :disabled="runnerLogLoading" @click="preview && loadAttemptLog(preview.id, selectedAttempt)">刷新日志</button></header>
                <p v-if="runnerLogConnectionError" role="status">{{ runnerLogConnectionError }}</p>
                <p v-if="runnerLogLoading && !runnerLog">正在读取此轮日志…</p>
                <p v-if="runnerLogError" role="alert">{{ runnerLogError }}</p>
                <template v-if="runnerLog">
                  <small>{{ runnerLog.source === 'attempt_work_dir' ? '此轮独立目录' : '当前任务目录' }} · {{ runnerLog.status }} · {{ runnerLog.updatedAt }}</small>
                  <p v-if="runnerLog.status !== 'ready'">{{ runnerLog.message || '此轮没有可展示的日志' }}</p>
                  <pre v-else>{{ runnerLog.lines.join('\n') }}</pre>
                  <small v-if="runnerLog.status === 'ready'">最多显示末尾 200 行；完整证据以原结果页与持久化记录为准。</small>
                </template>
                <NativeProcessLogView v-if="preview" :scan-id="preview.id" :attempt="selectedAttempt" :active="detailTab==='execution'" />
                <NativeSdkLogView v-if="preview" :scan-id="preview.id" :attempt="selectedAttempt" :active="detailTab==='execution'" />
              </section>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" aria-label="选中执行轮次的智能体通信">
                <header><strong>第 {{ selectedAttempt }} 次执行 · 智能体通信</strong><button type="button" class="button ghost compact" :disabled="mailboxLoading" @click="preview && loadMailbox(preview.id, selectedAttempt)">刷新消息</button></header>
                <p v-if="mailboxLoading && !mailboxPage">正在读取此轮消息…</p>
                <p v-if="mailboxError" role="alert">{{ mailboxError }}</p>
                <p v-if="mailboxPage && !mailboxPage.messages.length">此轮没有已保存的智能体通信；旧任务可能没有多智能体记录。</p>
                <ol v-if="mailboxPage?.messages.length" class="task-detail-records task-mailbox-records">
                  <li v-for="message in mailboxPage.messages" :key="message.id">
                    <strong>{{ message.fromRole }} → {{ message.toRole }} · {{ message.kind }}</strong>
                    <span>{{ message.summary }}</span>
                    <small>{{ message.createdAt }} · {{ message.deliveryState }} / {{ message.ackState }} · 证据版本 {{ message.evidenceRevision }}</small>
                    <small v-if="message.correlationId">话题 {{ message.correlationId }} · 分派 {{ message.assignmentId || '无' }}</small>
                  </li>
                </ol>
                <button v-if="mailboxPage?.hasOlder" type="button" class="button ghost compact" :disabled="mailboxLoading" @click="preview && loadMailbox(preview.id, selectedAttempt, true)">读取更早的消息</button>
                <small>只展示此轮已落库的脱敏消息摘要。证据缺口提案与评估不等于已派发补证或已生成新事实；须以新证据版本和 Reviewer 复审结果为准。人工指令与完整证据请到协作页或原结果页核对。</small>
              </section>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" aria-label="选中执行轮次的工具调用">
                <header><strong>第 {{ selectedAttempt }} 次执行 · 工具调用</strong><button type="button" class="button ghost compact" :disabled="toolLoading" @click="preview && loadTools(preview.id, selectedAttempt)">刷新工具记录</button></header>
                <p v-if="toolLoading && !toolPage">正在读取此轮工具记录…</p>
                <p v-if="toolError" role="alert">{{ toolError }}</p>
                <p v-if="toolPage && !toolPage.invocations.length">此轮没有已保存的工具调用。</p>
                <ol v-if="toolPage?.invocations.length" class="task-detail-records task-mailbox-records">
                  <li v-for="call in toolPage.invocations" :key="call.id">
                    <strong>{{ call.role }} · {{ call.toolName }} · {{ call.status }}</strong>
                    <template v-if="call.origin === 'source_receipt'">
                      <small>源码工具计划记录于 {{ call.recordedAt }}（模型响应落库时间，不是执行开始时间）</small>
                      <small v-if="call.status === 'planned'">计划待执行 · 尚无执行完成回执，不代表正在执行。</small>
                      <small v-else-if="call.receiptIntegrity === 'unverified'">回执无法核对 · 不计为已完成，也不能据此重放。</small>
                      <small v-else>完成回执 {{ call.finishedAt }} · {{ call.status === 'refused' ? '工具拒绝，未获得成功结果' : '本地回执一致，不等于独立审查通过' }}</small>
                      <small v-if="call.resultKind === 'control'">阶段结束标记，不计为漏洞证据。</small>
                    </template>
                    <small v-else>{{ call.startedAt }} → {{ call.finishedAt || '尚无终态' }} · 策略 {{ call.policyDecision || '未记录' }}</small>
                    <small>请求证据 {{ call.hasRequestArtifact ? '已登记' : '无引用' }} · 响应证据 {{ call.hasResponseArtifact ? '已登记' : '无引用' }}</small>
                    <span v-if="call.errorClass">失败类型：{{ call.errorClass }}</span>
                    <small>运行 {{ call.runId }} · 调用 {{ call.invocationId }}</small>
                  </li>
                </ol>
                <button v-if="toolPage?.hasOlder" type="button" class="button ghost compact" :disabled="toolLoading" @click="preview && loadTools(preview.id, selectedAttempt, true)">读取更早的调用</button>
                <small>这里只显示持久化状态和证据引用是否存在；“已登记”不等于已审核或可安全重放。不会展示工具输入、响应正文或认证材料。</small>
              </section>
              <p v-if="nativeStatus?.stopDiagnostic?.obligations.length && selectedAttempt === nativeStatus.attemptNumber">当前轮次待核对事项：{{ nativeStatus.stopDiagnostic.obligations.map(item => `${item.kind}: ${item.status}`).join('；') }}{{ nativeStatus.stopDiagnostic.obligationsTruncated ? '（还有未展示项）' : '' }}</p>
            </template>
            <template v-else-if="detailTab === 'evidence'">
              <template v-if="preview.scanType !== 'web'">
                <label v-if="attempts.length">查看源码审查轮次
                  <select :value="selectedAttempt" aria-label="源码审查执行轮次" @change="chooseAttempt(Number(($event.target as HTMLSelectElement).value))"><option v-for="attempt in attempts" :key="attempt.attemptNumber" :value="attempt.attemptNumber">第 {{ attempt.attemptNumber }} 次 · {{ statusLabel(attempt.status) }}</option></select>
                </label>
                <SourceReviewEvidence v-if="selectedAttempt !== undefined" :scan-id="preview.id" :attempt-number="selectedAttempt" />
              </template>
              <p v-if="preview.scanType === 'web' && nativeStatus">候选 {{ nativeStatus.findingCandidateCount }} 条 · 审核 {{ nativeStatus.reviewStatus }} · {{ nativeStatus.reviewGateSatisfied ? '审查门禁已满足' : '审查门禁未满足或不适用' }}。</p>
              <section v-if="historicalPreviews.length" class="task-attempt-log" aria-label="历史导入只读预览">
                <header><strong>历史产物 · 只读预览</strong></header>
                <p>以下 {{ historicalPreviews.length }} 条来自旧 JSON 等产物，均未经过当前独立 Reviewer 审核，不能当作已确认漏洞或新任务授权。仅展示脱敏摘要，完整原件不在此处开放。</p>
                <ol class="task-detail-records task-mailbox-records">
                  <li v-for="item in historicalPreviews" :key="item.membershipId">
                    <strong>{{ item.title || item.kind }}{{ item.severity ? ` · ${item.severity}` : '' }}</strong>
                    <span v-if="item.target">{{ item.target }}</span>
                    <small>{{ item.producer }} · {{ item.kind }} · 第 {{ item.attemptNumber }} 次记录 · 未审核 / 只读</small>
                  </li>
                </ol>
                <small v-if="historicalPreviews.length === 300">最多展示 300 条；其余历史产物仍保留在导入记录中，此处不是完整清单。</small>
              </section>
              <p>事实、反证、接口与漏洞保留在同一任务的结果页；这里不复制第二份结论，也不把待补证当作已确认。</p>
              <button type="button" class="button ghost compact" @click="emit('open', preview)">查看原始证据与历史结果</button>
            </template>
            <template v-else>
              <p>这里只列出本任务已保存的候选与知识；不会自动创建永久 Skill。跨任务复用需独立复核与容量治理。</p>
              <p v-if="!detailLoading && !learningCandidates.length && !knowledge.length">本任务暂无可展示的学习记录，不代表已经完成分析。</p>
              <div class="task-detail-records" v-if="learningCandidates.length || knowledge.length">
                <article v-for="item in learningCandidates" :key="`candidate-${item.id}`"><strong>候选 · {{ item.title }} · {{ item.status }}</strong><p>{{ item.summary }}</p><small>{{ item.updatedAt }} · 来源任务 {{ item.scanId }}</small></article>
                <article v-for="item in knowledge" :key="`knowledge-${item.id}`"><strong>知识 · {{ item.title }}{{ item.skillId ? ' · 已关联技能' : '' }}</strong><p>{{ item.summary }}</p><small>{{ item.updatedAt }} · 来源任务 {{ item.scanId }}</small></article>
              </div>
            </template>
          </div>
      </aside>
  </div>
</template>

<style scoped src="./taskCenterPresentation.css"></style>
