<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Download, ExternalLink, Plus, Search, ShieldAlert, Square, X } from "@lucide/vue";
import { assetApi } from "../api";
import { exposureApi } from "./api";
import type { Project } from "../../workspaces/types";
import type { ExposureFindingDraft } from "./contracts";
import { buildGradeRequest, intelGradeLine, sourceStatusView } from "./contracts";
import { useExposureSurface } from "./useExposureSurface";
import { useExposureEvidence } from "./useExposureEvidence";
import ExposureSettingsPanel from "./ExposureSettingsPanel.vue";
import ExposureClustersPanel from "./ExposureClustersPanel.vue";
import ExposureReviewPanel from "./ExposureReviewPanel.vue";
import ExposureIncidentsPanel from "./ExposureIncidentsPanel.vue";
import ExposureCoverageDrill from "./ExposureCoverageDrill.vue";
import { INFO_CATEGORY_LABELS, coverageNotice, coverageRows, liveRunLines, newOperationId, unitStatisticsLines } from "./incidents";
import "../../../exposure-surface.css";
import "../../../exposure-runs.css";
import "../../../exposure-tabs.css";

const props = defineProps<{ projects: Project[]; selectedProjectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const project = computed(() => props.projects.find((item) => item.id === props.selectedProjectId));
/// The exposure surface talks to its own IPC wrapper; only the asset ledger read for the
/// bound-asset picker comes directly from the Assets owner, without modifying it.
const surfaceApi = { ...exposureApi, listAssets: assetApi.listAssets };
const surface = useExposureSurface({
  projectId: () => props.selectedProjectId,
  active: () => true,
  api: surfaceApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  summary, findings, runs, sources, boundAssets, selectedRunId, statusFilter, searchFilter,
  findingsTotal, hasMore, loading, loadingMore, stale, readError, hasSnapshot, starting,
  boundAssetsTotal, boundAssetsTruncated, activeRun, scopeNotice, beyondPlanNotice, limitsNotice, unitLines,
  lastExport,
  dispositionFilter, dispositionFilters, badgeFor, classificationNotice,
  windowNotice,
  load, loadMore, showSourcesFor, startScan, cancelScan, save, exportAll,
} = surface;

const evidence = useExposureEvidence({
  projectId: () => props.selectedProjectId,
  active: () => true,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  pending, pendingLoading, pendingNotice, evidence: evidencePackage, evidenceFor,
  evidenceLoading, headline, reviewNeeded, resumeRun,
  loadPending, loadMorePending, openEvidence, closeEvidence, readError: pendingError,
} = evidence;

const view = ref<"overview" | "runs" | "findings" | "sources" | "pending" | "clusters" | "review" | "incidents" | "coverage" | "settings">("overview");
/// G4-2 refreshed with the summary on every poll while a round is active; G4-1 matrix and unit lines.
const liveLines = computed(() => (summary.value ? liveRunLines(summary.value.liveRun, summary.value.liveRunError ?? "") : []));
const coverageView = computed(() => (summary.value ? coverageRows(summary.value.coverage) : []));
/// P1 覆盖矩阵钻取：点哪一行就钻取哪个来源（时间窗口、单位、工作项明细在钻取面板里）。
const drillSource = ref("");
const coverageLine = computed(() => (summary.value ? coverageNotice(summary.value.coverage, summary.value.coverageError ?? "") : ""));
const unitStatLines = computed(() => (summary.value ? unitStatisticsLines(summary.value.unitStatistics, summary.value.unitStatisticsError ?? "") : []));
const resumableLive = computed(() => {
  const live = summary.value?.liveRun as { runId?: number; resumable?: boolean } | null | undefined;
  return live && live.resumable === true && typeof live.runId === "number" ? live.runId : null;
});
// §E5 the review panel works on one finding picked from the page already on screen.
const reviewFindingId = ref<number | undefined>(undefined);
const showAdd = ref(false);
const saving = ref(false);
const blankForm = () => ({ assetId: undefined as number | undefined, category: "source_code", title: "", sourceType: "public_web", sourceUrl: "", evidenceExcerpt: "", severity: "medium", confidence: 70 });
const form = ref(blankForm());
const categoryLabel = (value: string) =>
  ({ source_code: "源代码泄漏", document: "内部文档", credential: "凭据线索", storage: "公开存储", repository: "代码仓库", metadata: "敏感元数据", other: "其他" } as Record<string, string>)[value] || value;
const interruptedRun = computed(
  () => runs.value.find((row) => row.status === "interrupted") ?? null,
);
// G1-0: the caps come from the project's own settings, with their basis named.
const limitsLine = computed(() => limitsNotice.value);

async function submit(): Promise<void> {
  if (!props.selectedProjectId || saving.value) return;
  saving.value = true;
  const input: ExposureFindingDraft = { ...form.value, projectId: props.selectedProjectId };
  if (await save(input)) {
    showAdd.value = false;
    form.value = blankForm();
    emit("notify", "success", "公开暴露线索已归并保存，仍需人工复核");
  }
  saving.value = false;
}

// 换工作空间时必须丢掉上一个项目的证据与待归并队列，晚到的响应由 composable 的代号拦截。
watch(
  () => props.selectedProjectId,
  () => {
    closeEvidence();
    if (view.value === "pending") void loadPending();
  },
);

async function openTab(next: typeof view.value): Promise<void> {
  view.value = next;
  if (next === "pending" && !pending.value) await loadPending();
}

async function showEvidence(findingId: number): Promise<void> {
  await openEvidence(findingId);
}

async function chooseRun(event: Event): Promise<void> {
  const raw = (event.target as HTMLSelectElement).value;
  await showSourcesFor(raw === "" ? null : Number(raw));
}

/// G0/P1-4: a finding's review is written only through the scoped panel (revision, operator
/// and reason); the list button just opens that panel on the finding already on screen.
async function openReview(findingId: number): Promise<void> {
  reviewFindingId.value = findingId;
  await openTab("review");
}

/// §E5 the review panel only ever works on a finding that is already on screen.
function pickReviewFinding(event: Event): void {
  const raw = (event.target as HTMLSelectElement).value;
  reviewFindingId.value = raw === "" ? undefined : Number(raw);
}


const gradeActor = ref("");
const gradeReason = ref("");
const gradeChoice = ref("claim_only");
const gradeSample = ref("");
const gradeEntity = ref("");
const gradeBusy = ref(false);
const gradeNotice = ref("");

function positiveId(raw: string): number | undefined {
  const text = raw.trim();
  if (!/^[1-9][0-9]*$/.test(text)) return undefined;
  return Number(text);
}

async function submitGrade(): Promise<void> {
  const pkg = evidencePackage.value;
  const projectId = props.selectedProjectId;
  if (!pkg || !projectId || !pkg.intelGrade) return;
  const built = buildGradeRequest({
    findingId: pkg.finding.id,
    expectedGrade: pkg.intelGrade,
    grade: gradeChoice.value,
    actor: gradeActor.value,
    reason: gradeReason.value,
    sampleObservationId: positiveId(gradeSample.value),
    entityId: positiveId(gradeEntity.value),
  }, newOperationId());
  if (!built.ok) {
    gradeNotice.value = built.error;
    return;
  }
  gradeBusy.value = true;
  try {
    await exposureApi.gradeExposureIntel(projectId, built.payload);
    gradeNotice.value = "已记录改级（不会自动关闭线索，也不会访问来源）";
    gradeReason.value = "";
    await openEvidence(pkg.finding.id);
  } catch (reason) {
    gradeNotice.value = `改级未写入：${reason instanceof Error ? reason.message : String(reason)}`;
  } finally {
    gradeBusy.value = false;
  }
}
</script>

<template>
  <div class="exposure-surface">
    <div class="section-toolbar">
      <div>
        <span class="eyebrow">PUBLIC EXPOSURE</span>
        <h2>公开暴露面</h2>
        <p>{{ scopeNotice }}；只读取已通过归属与授权门禁的公开资产，证书透明度与历史归档只提供线索。</p>
        <small class="exposure-limits">{{ limitsLine }}</small>
        <small v-if="lastExport" class="exposure-limits">{{ lastExport }}</small>
        <p v-if="windowNotice" class="exposure-gap">{{ windowNotice }}</p>
      </div>
      <div>
        <button v-if="activeRun" class="button danger" @click="cancelScan"><Square :size="14" />停止采集</button>
        <button v-else class="button ghost" :disabled="!project || starting" :title="`被动采集受上限约束：${limitsLine}`" @click="startScan">
          <Search :size="15" />{{ starting ? "正在启动…" : "开始被动采集" }}
        </button>
        <button class="button ghost" title="线索 CSV：原 14 列加分类、事件与整改列，导出后读回核验行数与 SHA-256" @click="exportAll('findings')"><Download :size="15" />导出线索</button>
        <button class="button ghost" title="事件台账 CSV：rowKind 区分汇总/事件/位置/单位关联/覆盖，汇总行与明细可对账" @click="exportAll('ledger')"><Download :size="15" />导出事件台账</button>
        <button class="button primary" :disabled="!project" @click="showAdd = !showAdd"><Plus :size="15" />录入线索</button>
      </div>
    </div>

    <div v-if="!project" class="panel exposure-empty"><ShieldAlert :size="25" /><strong>请先选择工作空间</strong><span>暴露面线索不能跨公司混合。</span></div>
    <template v-else>
      <div v-if="stale" class="panel exposure-stale"><strong>显示的是最近一次可信快照</strong><span>{{ readError }}</span></div>
      <nav class="exposure-tabs">
        <button :class="{ active: view === 'overview' }" @click="view = 'overview'">总览</button>
        <button :class="{ active: view === 'runs' }" @click="view = 'runs'">采集任务</button>
        <button :class="{ active: view === 'findings' }" @click="view = 'findings'">线索复核</button>
        <button :class="{ active: view === 'sources' }" @click="view = 'sources'">数据源</button>
        <button :class="{ active: view === 'pending' }" @click="openTab('pending')">待归并</button>
        <button :class="{ active: view === 'clusters' }" @click="view = 'clusters'">内容簇</button>
        <button :class="{ active: view === 'review' }" @click="openTab('review')">复核整改</button>
        <button :class="{ active: view === 'incidents' }" @click="view = 'incidents'">事件台账</button>
        <button :class="{ active: view === 'coverage' }" @click="view = 'coverage'">覆盖与单位</button>
        <button :class="{ active: view === 'settings' }" @click="view = 'settings'">采集配置</button>
      </nav>

      <section v-if="view === 'overview'" class="exposure-summary">
        <button @click="statusFilter = ''; view = 'findings'; load()"><span>准入资产</span><strong>{{ summary?.eligibleAssetsTotal ?? 0 }}</strong></button>
        <button @click="statusFilter = 'new'; view = 'findings'; load()"><span>新线索</span><strong>{{ summary?.findingsNew ?? 0 }}</strong></button>
        <button @click="statusFilter = 'reviewing'; view = 'findings'; load()"><span>复核中</span><strong>{{ summary?.findingsReviewing ?? 0 }}</strong></button>
        <button @click="statusFilter = 'confirmed'; view = 'findings'; load()"><span>已确认</span><strong>{{ summary?.findingsConfirmed ?? 0 }}</strong></button>
        <button class="risk" @click="view = 'findings'"><span>高风险待处置</span><strong>{{ summary?.highRiskOpen ?? 0 }}</strong></button>
        <p v-if="beyondPlanNotice" class="exposure-gap">{{ beyondPlanNotice }}</p>
        <p v-if="classificationNotice" class="exposure-classification">{{ classificationNotice }}</p>
        <ul v-if="unitLines.length" class="exposure-units">
          <li v-for="(line, index) in unitLines" :key="index">{{ line }}</li>
        </ul>
        <ul v-if="liveLines.length" class="exposure-live">
          <li v-for="(line, index) in liveLines" :key="index">{{ line }}</li>
          <li v-if="activeRun"><button @click="cancelScan">停止这一轮（已采集结果保留）</button></li>
          <li v-else-if="resumableLive !== null"><button @click="resumeRun(resumableLive)">按冻结计划恢复第 {{ resumableLive }} 轮</button></li>
        </ul>
        <p v-if="interruptedRun" class="exposure-gap">
          最近一轮 #{{ interruptedRun.id }} 已中断，仍有未闭合工作项：已采到的结果保留。
          <button @click="resumeRun(interruptedRun.id)">按冻结计划恢复该轮</button>
        </p>
      </section>

      <section v-if="view === 'runs'" class="panel exposure-runs">
        <div v-for="run in runs" :key="run.id" :class="{ selected: run.id === selectedRunId }">
          <strong>#{{ run.id }} · {{ run.status }}</strong>
          <span>{{ run.stage }}<template v-if="run.currentSource"> · {{ run.currentSource }}</template> · {{ run.scannedAssets }} / {{ run.eligibleAssets }} 资产 · {{ run.fetchedResources }} 资源 · {{ run.findings }} 新线索</span>
          <small v-if="run.error">{{ run.error }}</small>
          <button @click="showSourcesFor(run.id)">查看该轮数据源</button>
          <button v-if="run.status === 'interrupted'" @click="resumeRun(run.id)">按冻结计划恢复该轮</button>
        </div>
        <div v-if="!runs.length">当前工作空间还没有采集任务记录</div>
      </section>

      <section v-if="view === 'sources'" class="panel exposure-runs">
        <label class="exposure-run-pick">
          <span>查看哪一轮的数据源</span>
          <select :value="selectedRunId ?? ''" @change="chooseRun">
            <option value="">未选择</option>
            <option v-for="run in runs" :key="run.id" :value="run.id">#{{ run.id }} · {{ run.status }}</option>
          </select>
        </label>
        <div v-for="source in sources" :key="source.sourceKey" :class="['exposure-source-row', sourceStatusView(source).tone]">
          <strong>{{ sourceStatusView(source).label }} · {{ sourceStatusView(source).status }}</strong>
          <span>{{ source.itemCount }} 条结果 · {{ source.completedAt }}</span>
          <em v-if="sourceStatusView(source).reason" class="exposure-source-reason">{{ sourceStatusView(source).reason }}</em>
          <small v-if="source.error">{{ source.error }}</small>
        </div>
        <div v-if="!sources.length">所选任务尚无数据源执行记录</div>
      </section>

      <section v-if="view === 'review'" class="panel exposure-review-pick">
        <label>
          <span>复核哪条线索（只列出当前已读到的 {{ findings.length }} / 共 {{ findingsTotal }} 条）</span>
          <select :value="reviewFindingId ?? ''" @change="pickReviewFinding">
            <option value="">未选择</option>
            <option v-for="item in findings" :key="item.id" :value="item.id">{{ item.title }}（{{ item.status }}）</option>
          </select>
        </label>
        <p v-if="hasMore" class="exposure-gap">线索列表还有更多页未读取；这里不会把全部线索拉进页面，请先加载更多或在列表里搜索。</p>
      </section>
      <ExposureReviewPanel
        v-if="view === 'review'"
        :project-id="props.selectedProjectId"
        :finding-id="reviewFindingId"
        @notify="(kind, text) => emit('notify', kind, text)"
      />

      <ExposureIncidentsPanel v-if="view === 'incidents'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <section v-if="view === 'coverage'" class="panel exposure-coverage">
        <header><strong>覆盖矩阵</strong><small>{{ coverageLine }}</small></header>
        <table v-if="coverageView.length">
          <thead>
            <tr><th>来源能力</th><th>状态 · 分母 · 时间窗口</th><th v-for="(label, key) in INFO_CATEGORY_LABELS" :key="key">{{ label }}</th><th>钻取</th></tr>
          </thead>
          <tbody>
            <tr v-for="row in coverageView" :key="row.capability">
              <th>{{ row.label }}<small v-if="row.integrationPending"> · 待接入</small></th>
              <td><strong>{{ row.state }}</strong><small>{{ row.denominator }}</small><small>{{ row.window }}</small><em>{{ row.reason }}</em></td>
              <td v-for="cell in row.cells" :key="cell.category" :class="['exposure-cell', cell.tone]">{{ cell.label }}</td>
              <td><button :data-drill="row.sourceKey" :disabled="!row.sourceKey" @click="drillSource = row.sourceKey">明细</button></td>
            </tr>
          </tbody>
        </table>
        <ExposureCoverageDrill
          :project-id="props.selectedProjectId"
          :rows="coverageView"
          :source="drillSource"
          @close="drillSource = ''"
          @notify="(kind, text) => emit('notify', kind, text)"
        />
        <header><strong>单位统计</strong></header>
        <ul class="exposure-units">
          <li v-for="(line, index) in unitStatLines" :key="index">{{ line }}</li>
        </ul>
      </section>

      <ExposureClustersPanel v-if="view === 'clusters'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <ExposureSettingsPanel v-if="view === 'settings'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <section v-if="showAdd" class="panel exposure-add">
        <header><strong>录入或导入公开线索</strong><button @click="showAdd = false"><X :size="16" /></button></header>
        <div class="exposure-form">
          <label>
            <span>绑定准入资产（可选，共 {{ boundAssetsTotal }} 个）</span>
            <select v-model="form.assetId"><option :value="undefined">主体级线索</option><option v-for="asset in boundAssets" :key="asset.id" :value="asset.id">{{ asset.domain || asset.host || asset.link }}</option></select>
          </label>
          <p v-if="boundAssetsTruncated" class="exposure-gap">下拉只加载了前 {{ boundAssets.length }} 个准入资产，可继续翻页选择更早的条目。</p>
          <label><span>类型</span><select v-model="form.category"><option value="source_code">源代码泄漏</option><option value="document">内部文档</option><option value="credential">凭据线索</option><option value="storage">公开存储</option><option value="repository">代码仓库</option><option value="metadata">敏感元数据</option><option value="other">其他</option></select></label>
          <label><span>风险</span><select v-model="form.severity"><option value="low">低</option><option value="medium">中</option><option value="high">高</option><option value="critical">严重</option></select></label>
          <label class="wide"><span>标题</span><input v-model="form.title" placeholder="面向复核人员的清晰描述" /></label>
          <label class="wide"><span>公开来源 URL</span><input v-model="form.sourceUrl" placeholder="https://…" /></label>
          <label class="wide"><span>证据摘要（后端统一脱敏，请勿粘贴完整密钥）</span><textarea v-model="form.evidenceExcerpt" rows="4" placeholder="记录文件路径、字段名、脱敏片段和可复现位置"></textarea></label>
        </div>
        <footer><button class="button primary" :disabled="saving" @click="submit">保存并自动去重</button></footer>
      </section>

      <section v-if="view === 'overview' || view === 'findings'" class="panel exposure-list">
        <header>
          <div><strong>暴露线索</strong><small>{{ findings.length }} / {{ findingsTotal }} 条当前筛选结果</small></div>
          <label><Search :size="14" /><input v-model="searchFilter" placeholder="搜索标题、来源或证据" @keyup.enter="load" /></label>
          <select v-model="dispositionFilter" class="exposure-disposition-filter" title="G1-3 按分类处置筛选" @change="load()">
            <option v-for="option in dispositionFilters" :key="option.value" :value="option.value">{{ option.label }}</option>
          </select>
        </header>
        <article v-for="item in findings" :key="item.id">
          <div class="exposure-risk" :class="item.severity">{{ item.severity }}</div>
          <div class="exposure-main">
            <div><span>{{ categoryLabel(item.category) }}</span><strong>{{ item.title }}</strong><em>{{ item.confidence }}% 可信度</em></div>
            <a :href="item.sourceUrl" target="_blank">{{ item.sourceUrl }} <ExternalLink :size="11" /></a>
            <p>{{ item.evidenceExcerpt || '尚未填写证据摘要' }}</p>
            <small>{{ item.assetLabel || '主体级公开线索' }} · {{ item.sourceType }} · 首次 {{ item.firstSeenAt }} · 最近 {{ item.lastSeenAt }}</small>
          </div>
          <div class="exposure-actions">
            <span :class="`status-${item.status}`">{{ item.status }}</span>
            <span class="exposure-disposition" :class="badgeFor(item.id).tone" :title="badgeFor(item.id).title">{{ badgeFor(item.id).label }}</span>
            <button title="在复核整改面板中按队列版本、操作者和依据写入结论" @click="openReview(item.id)">复核…</button>
            <button :class="{ active: evidenceFor === item.id }" @click="showEvidence(item.id)">证据</button>
          </div>
        </article>
        <div v-if="loading" class="exposure-empty">正在加载…</div>
        <div v-else-if="readError && !hasSnapshot" class="exposure-error">
          <strong>线索列表没有读到，不能显示为“没有线索”</strong>
          <span>{{ readError }}</span>
        </div>
        <div v-else-if="!findings.length" class="exposure-empty">当前筛选没有公开暴露线索</div>
        <footer v-if="hasMore" class="exposure-more"><button :disabled="loadingMore" @click="loadMore">{{ loadingMore ? "正在加载…" : "加载更多线索" }}</button></footer>
      </section>
      <section v-if="view === 'pending'" class="panel exposure-runs">
        <header><strong>外部索引带回的待归并结果</strong><small>{{ pendingNotice }}</small></header>
        <p class="exposure-gap">这些名字只进入待归并队列：不自动成为资产，不自动通过归属门禁，也不自动获得任何采集授权。</p>
        <div v-for="row in pending?.items ?? []" :key="row.id">
          <strong>{{ row.entity }}</strong>
          <span>{{ row.sourceKey }} · {{ row.kind }} · {{ row.status }} · {{ row.createdAt }}</span>
          <small>来源 {{ row.evidenceUrl }}</small>
        </div>
        <div v-if="pendingLoading" class="exposure-empty">正在加载…</div>
        <div v-else-if="pendingError" class="exposure-error">
          <strong>待归并队列没有读到</strong>
          <span>{{ pendingError }}</span>
        </div>
        <div v-else-if="!pending?.items.length" class="exposure-empty">{{ pendingNotice }}</div>
        <footer v-if="pending?.hasMore" class="exposure-more"><button :disabled="pendingLoading" @click="loadMorePending">加载更多待归并结果</button></footer>
      </section>

      <section v-if="evidencePackage" class="panel exposure-evidence">
        <header>
          <div>
            <strong>证据包 · {{ evidencePackage.finding.title }}</strong>
            <small>{{ headline }}</small>
          </div>
          <button @click="closeEvidence"><X :size="16" /></button>
        </header>
        <p v-if="intelGradeLine(evidencePackage)" class="exposure-gap">{{ intelGradeLine(evidencePackage) }}</p>
        <form v-if="evidencePackage.intelGrade" class="exposure-grade" @submit.prevent="submitGrade">
          <strong>人工改级</strong>
          <p>当前等级只读：{{ evidencePackage.intelGrade }}。改级只写记录，不访问声明的来源，也不自动关闭线索。</p>
          <label>新等级
            <select v-model="gradeChoice">
              <option value="claim_only">仅声称</option>
              <option value="sample_supported">样本支持</option>
              <option value="human_verified">已人工核验</option>
            </select>
          </label>
          <label>操作人<input v-model="gradeActor" maxlength="80" /></label>
          <label>理由<input v-model="gradeReason" maxlength="500" /></label>
          <label>样本观察
            <select v-model="gradeSample">
              <option value="">不选</option>
              <option v-for="row in evidencePackage.observations" :key="row.id" :value="String(row.id)">#{{ row.id }}</option>
            </select>
          </label>
          <label>目录单位编号<input v-model="gradeEntity" inputmode="numeric" placeholder="可选" /></label>
          <button type="submit" :disabled="gradeBusy">写入等级</button>
          <small v-if="gradeNotice">{{ gradeNotice }}</small>
        </form>
        <p v-if="reviewNeeded" class="exposure-gap">内容或规则已变化（版本 {{ reviewNeeded.revision }}，规则 {{ reviewNeeded.ruleVersion }}）：上一人工结论仍保留，但必须重新复核。</p>
        <p v-if="evidenceLoading" class="exposure-empty">正在读取证据…</p>
        <div class="exposure-observations">
          <div v-for="row in evidencePackage.observations" :key="row.id">
            <strong>{{ row.kind }} · {{ row.statusCode || '—' }}</strong>
            <span>{{ row.url }}</span>
            <small>
              第 {{ row.runId }} 轮 · {{ row.sourceKey }} · 抓取 {{ row.queriedAt }}
              · 内容 {{ row.contentHash.slice(0, 12) }} · 响应头 {{ row.headersHash.slice(0, 12) }}
              · {{ row.bodyBytes }} 字节{{ row.truncated ? "（已截断）" : "" }}
              · 当前{{ row.currentAccessible === null ? "可访问性未知（仅索引命中）" : row.currentAccessible ? "可访问" : "不可访问" }}
              <template v-if="row.indexedAt"> · 收录 {{ row.indexedAt }}</template>
              <template v-if="row.boundary"> · 边界 {{ row.boundary }}</template>
              · 规则 {{ row.ruleVersion }} · 脱敏 {{ row.redactionVersion }}
            </small>
          </div>
          <div v-if="!evidencePackage.observations.length" class="exposure-empty">这条线索还没有绑定观察记录（旧版采集留下的线索）</div>
        </div>
        <div class="exposure-versions">
          <strong>版本轨迹</strong>
          <div v-for="row in evidencePackage.versions" :key="row.revision">
            <span>v{{ row.revision }} · {{ row.severity }} · {{ row.contentHash.slice(0, 12) }} · {{ row.createdAt }}</span>
            <small>人工状态 {{ row.manualStatus || "—" }}{{ row.reviewNeeded ? " · 需复核" : "" }} · 规则 {{ row.ruleVersion }}</small>
          </div>
          <div v-if="!evidencePackage.versions.length" class="exposure-empty">尚无版本记录</div>
        </div>
      </section>
    </template>
  </div>
</template>
