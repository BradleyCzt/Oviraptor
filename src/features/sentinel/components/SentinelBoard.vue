<script setup lang="ts">
import { computed, onUnmounted, reactive, ref, shallowRef, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  Bug,
  Code2,
  Download,
  ExternalLink,
  Fingerprint,
  Globe2,
  HelpCircle,
  Layers3,
  Network,
  Pause,
  Play,
  RefreshCw,
  Shield,
  ShieldCheck,
} from "@lucide/vue";
import { api } from "../../../api";
import { useI18n } from "../../../i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import InlineConfirm from "../../../shared/components/InlineConfirm.vue";
import { useBoardLifecycle } from "../results/useBoardLifecycle";
import { useTaskSearch } from "../results/useTaskSearch";
import { useScanPages } from "../results/useScanPages";
import { useTaskTrace } from "../traces/useTaskTrace";
import { useTokenUsage } from "../overview/useTokenUsage";
import { useInvestigationSummary } from "../overview/useInvestigationSummary";
import SentinelTokenOverview from "./overview/SentinelTokenOverview.vue";
import SentinelOverviewSummary from "./overview/SentinelOverviewSummary.vue";
import SentinelOverviewSidebar from "./overview/SentinelOverviewSidebar.vue";
import SentinelTaskOverview from "./overview/SentinelTaskOverview.vue";
import SentinelValidationWorkbench from "./SentinelValidationWorkbench.vue";
import SentinelFingerprintPane from "./results/SentinelFingerprintPane.vue";
import SentinelTraceTimeline from "./results/SentinelTraceTimeline.vue";
import SentinelEndpointsPane from "./results/SentinelEndpointsPane.vue";
import SentinelOpportunitiesPane from "./results/SentinelOpportunitiesPane.vue";
import SentinelVulnerabilitiesPane from "./results/SentinelVulnerabilitiesPane.vue";
import SentinelApiPane from "./results/SentinelApiPane.vue";
import SentinelFuseZone from "./SentinelFuseZone.vue";
import {
  fuseCategoryLabel,
  fuseReasonCategory as classifyFuseReason,
  fuseReasonParts as describeFuseReason,
  fuseRecommendedAction as recommendFuseAction,
} from "../fuse/presentation";
import {
  fuseDetailTabs, sameTargetUrl, useFuseDetails,
} from "../fuse/useFuseDetails";
import { useFuseDisposition } from "../fuse/useFuseDisposition";
import { useApiResults } from "../results/useApiResults";
import SentinelAuthRecoveryPanel from "./SentinelAuthRecoveryPanel.vue";
import InvestigationGraphPanel from "./InvestigationGraphPanel.vue";
import NativeRunStatus from "./NativeRunStatus.vue";
import SentinelExecutionDetails from "./SentinelExecutionDetails.vue";
import {
  AgentDialog,
  AgentTraceHub,
  AgentWorkbench,
  SentinelSourceResults,
  SentinelTaskCenter,
} from "./lazyPanels";
import SentinelRepeater from "./SentinelRepeater.vue";
import {
  attemptEndReason,
  attemptStageLabel,
  attemptTime,
  attemptBackendSummary,
  EXECUTION_STAGE_STEPS,
  executionStageStatusLabel,
  resolveExecutionStage,
  createSentinelLabels,
  displayName,
  displayVersion,
  endpointUrl,
  formatNumber,
  isHttpUrl,
  json,
  kindLabel,
  methodTone,
  routeModeLabel,
  safeSeverity,
  scanSummary,
  scanTokenTotal,
  scanTitle,
  uncachedInput,
  validRouteRecord,
  validSensitiveRecord,
} from "../presentation";
import type { GapFollowupPreview, ClosureHandoffPreview, AppSecScanResult, AppSecVulnerability, BrowserAuthSession, InvestigationGraph, InvestigationHypothesis, InvestigationValidation, SentinelCheckpoint, SentinelFinding, SentinelFuseEntry, SentinelOverviewStats, SentinelOpportunity, SentinelScan, SentinelScanAttempt, SentinelTarget, SentinelValidation, SentinelValidationWorkItem, AgentTargetExecution } from "../types";
import type { Project } from "../../workspaces/types";

const props = withDefaults(
  defineProps<{
    projects: Project[];
    projectId?: number;
    section?: Tab;
    resultView?: ResultTab;
    search?: string;
    workbenchMode?: WorkbenchMode;
    active?: boolean;
  }>(),
  {
    section: "overview",
    resultView: "summary",
    search: "",
    workbenchMode: "code",
    active: true,
  },
);
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  "section-change": [section: Tab];
  "projects-change": [];
  "create-project": [];
  "alerts-change": [alerts: { fuse: number; vulnerabilities: number }];
  "open-runner-log": [scanId: string, attempt: number];
  "open-workbench": [mode: WorkbenchMode];
}>();
const { tr } = useI18n();
type Tab =
  | "overview"
  | "queue"
  | "results"
  | "fuse"
  | "validations"
  | "workbench"
  | "dialog"
  | "help";
type WorkbenchMode = "web" | "code" | "greybox" | "cicd" | "skills" | "traces";
type ResultTab =
  | "summary"
  | "investigation"
  | "opportunities"
  | "fingerprint"
  | "api"
  | "endpoints"
  | "vulnerabilities";

const tab = ref<Tab>(props.section);
const dialogScanId = ref("");
function openScanDialog(scan: SentinelScan) {
  dialogScanId.value = scan.id;
  tab.value = "dialog";
  emit("section-change", "dialog");
}
const workbenchIntent = ref<WorkbenchMode | "">("");
const workbenchFollowup = ref<GapFollowupPreview>();
const workbenchHandoff = ref<ClosureHandoffPreview>();
let preparingHandoff = false;
async function prepareClosureHandoff(scanId: string) {
  if (preparingHandoff || selected.value?.id !== scanId) return;
  preparingHandoff = true;
  const selectedScan = selected.value;
  try {
    const preview = await api.previewWebClosureHandoff(scanId);
    if (selected.value !== selectedScan) return;
    if (preview.sourceScanId !== scanId || preview.attemptNumber !== selectedScan.attemptCount
      || preview.executionSettled !== false || preview.targetRequestsGranted !== 0) throw new Error('closure_handoff_preview_mismatch');
    startInvestigation();
    workbenchHandoff.value = preview;
  } catch (error) {
    if (selected.value === selectedScan) emit('notify', 'error', `无法准备独立关联任务：${String(error)}`);
  } finally { preparingHandoff = false; }
}
function startInvestigation() {
  workbenchFollowup.value = undefined;
  workbenchHandoff.value = undefined;
  workbenchIntent.value = "web";
  tab.value = "workbench";
  emit("section-change", "workbench");
  emit("open-workbench", "web");
}
function prepareGapFollowup(preview: GapFollowupPreview) {
  startInvestigation();
  workbenchFollowup.value = preview;
}
watch(tab, (value, previous) => {
  if (previous === "workbench" && value !== "workbench") {
    workbenchFollowup.value = undefined;
    workbenchHandoff.value = undefined;
  }
});
const resultTab = ref<ResultTab>(props.resultView);
// Follow the single project scope selected in App.vue.
const projectFilter = ref<number | undefined>(props.projectId);
// These records are replaced as immutable snapshots. Shallow refs avoid
// creating thousands of nested Vue proxies for checkpoint/finding payloads.
const scanPages = useScanPages({
  scope: () => projectFilter.value,
  isRefreshing: () => loading.value,
  read: (project, limit, cursor) => api.listSentinelScans(project, limit, cursor),
  onError: (message) => emit("notify", "error", message),
});
const { scans, scanPageSize, scanHasMore, scanLoadingMore,
  mergeScanPage, replaceScanHead, invalidateScanPagination, loadMoreScanHistory } = scanPages;
const vulnerabilityScanIds = shallowRef<string[]>([]);
const targets = shallowRef<SentinelTarget[]>([]);
const opportunities = shallowRef<SentinelOpportunity[]>([]);
const detailOpportunities = shallowRef<SentinelOpportunity[]>([]);
const emptyOverviewStats = (): SentinelOverviewStats => ({
  taskCount: 0,
  urlCount: 0,
  fingerprintCount: 0,
  apiCount: 0,
  endpointCount: 0,
  vulnerabilityCount: 0,
  highRiskCount: 0,
  reviewerConfirmedCount: 0,
  reviewerHighRiskCount: 0,
  sourceReviewerConfirmedCount: 0,
  sourceReviewAuditedTaskCount: 0,
  sourceReviewUnavailableTaskCount: 0,
  sourceReviewUnverifiedTaskCount: 0,
  otherVulnerabilityCount: 0,
  validatedCount: 0,
  pendingVulnerabilityCount: 0,
  vulnerableUrlCount: 0,
  activeFuseCount: 0,
  opportunityCount: 0,
  readyOpportunityCount: 0,
});
const stats = ref<SentinelOverviewStats>(emptyOverviewStats());
const overviewLoading = ref(false);
const overviewError = ref(false);
let overviewGeneration = 0;
let overviewRequests = 0;
let loadGeneration = 0;

async function refreshOverviewStats(project = projectFilter.value) {
  const generation = ++overviewGeneration;
  const current = () => !detailDisposed && generation === overviewGeneration && project === projectFilter.value;
  overviewLoading.value = true;
  overviewError.value = false;
  overviewRequests++;
  try {
    const next = await api.sentinelOverviewStats(project);
    if (current()) stats.value = next;
  } catch (error) {
    if (current()) { overviewError.value = true; throw error; }
  } finally {
    overviewRequests--;
    if (current()) overviewLoading.value = false;
  }
}
const { stats: investigationStats, state: investigationSummaryState,
  refresh: refreshInvestigationSummary } = useInvestigationSummary({
  scope: () => projectFilter.value,
  read: project => api.investigationOverview(project),
});
const opportunityView = ref<"ready" | "all" | "history">("ready");
const opportunityBusy = ref(0);
const loading = ref(false);
const detailBusy = ref(false);
watch(projectFilter, () => {
  ++overviewGeneration;
  ++loadGeneration;
  stats.value = emptyOverviewStats();
  overviewLoading.value = false;
  overviewError.value = false;
}, { flush: "sync" });
const selected = ref<SentinelScan>();
let detailGeneration = 0;
let detailDisposed = false;
let graphGeneration = 0;
const scanAttempts = shallowRef<SentinelScanAttempt[]>([]);
const showAttemptHistory = ref(false);
const visibleScanAttempts = computed(() =>
  showAttemptHistory.value ? scanAttempts.value : scanAttempts.value.slice(0, 1),
);
const attemptModeLabel = (mode: string, attemptNumber: number) =>
  mode === "fresh"
    ? "全新执行"
    : mode === "resume"
      ? "继续未完成阶段"
      : attemptNumber <= 1
        ? "首次执行"
        : "历史执行";
const selectedUrl = ref("");
const previewScan = ref<SentinelScan>();
const previewUrls = ref<string[]>([]);
const checkpoints = shallowRef<SentinelCheckpoint[]>([]);
const findings = shallowRef<SentinelFinding[]>([]);
const validations = shallowRef<SentinelValidation[]>([]);
const investigationValidations = shallowRef<InvestigationValidation[]>([]);
const previousFindings = shallowRef<SentinelFinding[]>([]);
const appsecResult = ref<AppSecScanResult>({
  vulnerabilities: [],
  sources: [],
});
const investigationGraph = shallowRef<InvestigationGraph>();
const investigationBusy = ref(false);
const investigationUpdatingId = ref<number>();
const repeaterOpportunity = shallowRef<SentinelOpportunity>();
const repeaterApi = shallowRef<any>();
const repeaterHypothesis = shallowRef<InvestigationHypothesis>();
const validationWorkItems = shallowRef<SentinelValidationWorkItem[]>([]);
const validationWorkEditor = ref<SentinelValidationWorkItem>();
const fuseEntries = shallowRef<SentinelFuseEntry[]>([]);
const fuseFilter = ref("active");
const fuseCategoryFilter = ref("all");
const { fuseEditor, pendingFuseRemoval, fuseBusy, fuseForm, editFuse, saveFuse, removeFuse } = useFuseDisposition({
  save: (input) => api.saveSentinelFuseReview(input),
  remove: (id) => api.removeSentinelFuseEntry(id),
  reload: () => load(),
  notify: (kind, message) => emit("notify", kind, message),
});
const validationFilter = ref("pending");
const validationEditor = ref<SentinelFinding>();
const selectedFindingId = ref<number>();
const validationForm = reactive({
  verdict: "needs_more",
  severity: "",
  note: "",
  evidence: "",
});
const validationWorkForm = reactive({
  verdict: "needs_more",
  severity: "medium",
  note: "",
  evidence: "",
});
const pendingDelete = ref<SentinelScan>();
const deleting = ref(false);
const scanControlBusy = ref("");
const authRecoveryScan = ref<SentinelScan>();
const authRecoverySessions = ref<BrowserAuthSession[]>([]);
const authRecoveryBusy = ref("");
const authRecoveryAutoContinuing = ref(false);
const { matchedScanIds, applySearch } = useTaskSearch({
  query: () => props.search,
  scope: () => projectFilter.value,
  lookup: (query) => api.searchSentinelScanIds(query),
  showResults: () => { tab.value = "results"; },
  selectMatch: async () => {
    const scan = visibleScans.value[0];
    if (scan && scan.id !== selected.value?.id) await openScan(scan, false);
  },
  onError: (message) => emit("notify", "error", message),
});
const expandedSensitive = ref<number[]>([]);
const { detail: liveTrace, busy: liveTraceBusy, load: loadSelectedTrace, reset: resetSelectedTrace } = useTaskTrace({
  scanId: () => selected.value?.id,
  projectId: () => projectFilter.value,
  read: scanId => api.getAgentTrace(scanId),
  notify: message => emit("notify", "error", message),
});
const { fuseState, fuseRows, fuseTarget, fuseValidationRows, toggleFuseDetail } = useFuseDetails({
  targets,
  loadFindings: (scanId) => api.listSentinelFindings(scanId),
  loadValidations: (scanId) => api.listSentinelValidations(scanId),
  notifyError: (message) => emit("notify", "error", message),
});

const {
  statusLabel,
  retryActionLabel,
  verdictLabel,
  severityLabel,
  scanTypeLabel,
} =
  createSentinelLabels(tr);
function findingKey(item: SentinelFinding) {
  return `${item.stage}:${item.kind}:${item.recordKey}`;
}
const validationByFinding = computed(() => {
  const map=new Map<string,SentinelValidation>();
  for(const item of validations.value) map.set(`${item.url}|${item.findingKey}`,item);
  return map;
});
function validationFor(item: SentinelFinding) {
  return validationByFinding.value.get(`${item.targetUrl}|${findingKey(item)}`);
}
function effectiveSeverity(item: SentinelFinding) {
  const validation = validationFor(item);
  if (validation?.verdict === "false_positive") return "none";
  if (validation && validation.verdict !== "pending" && validation.severity)
    return safeSeverity(validation.severity);
  return safeSeverity(item.severity);
}
async function openTargetUrl(url: string) {
  if (!/^https?:\/\//i.test(url)) return;
  try {
    await openUrl(url);
  } catch (e) {
    emit("notify", "error", `无法打开浏览器：${String(e)}`);
  }
}
function registrationData(item: SentinelFinding) {
  const data = json(item.recordJson);
  return data.registration || data;
}
function runtimeSignalUrl(item: SentinelFinding) {
  const data = json(item.recordJson);
  return data.url || data.source || selectedUrl.value || "";
}
async function copyText(value: string) {
  try {
    await navigator.clipboard.writeText(value);
    emit("notify", "success", "已复制到剪贴板");
  } catch {
    emit("notify", "error", "复制失败");
  }
}
function toggleSensitive(id: number) {
  expandedSensitive.value = expandedSensitive.value.includes(id)
    ? expandedSensitive.value.filter((item) => item !== id)
    : [...expandedSensitive.value, id];
}
const { tokenScope, usage: tokenUsage, totalTokenUsage, totalRequestUsage,
  zeroYieldScans, zeroYieldTokenUsage, cacheHitRate } = useTokenUsage({
  scans, vulnerabilityScanIds,
});
const tokenScopeLabel = computed(() => tokenScope.value === "cloud" ? tr("云端 AI", "Cloud AI")
  : tokenScope.value === "local" ? tr("本地模型", "Local model") : tr("全部部署", "All deployments"));
function fuseReasonParts(item: SentinelFuseEntry) {
  return describeFuseReason(item, fuseTarget(item));
}
function fuseReasonCategory(item: SentinelFuseEntry) {
  return classifyFuseReason(item, fuseTarget(item));
}
function fuseRecommendedAction(item: SentinelFuseEntry) {
  return recommendFuseAction(item, fuseTarget(item));
}

const normalizedSearch = computed(() => props.search.trim().toLowerCase());
const searchableScanIds = computed(
  () =>
    new Set(
      targets.value
        .filter((target) =>
          `${target.company} ${target.url}`
            .toLowerCase()
            .includes(normalizedSearch.value),
        )
        .map((target) => target.scanId)
        .filter(Boolean),
    ),
);
const visibleScans = computed(() =>
  scans.value.filter(
    (scan) =>
      (!projectFilter.value || scan.projectId === projectFilter.value) &&
      (!normalizedSearch.value ||
        matchedScanIds.value.includes(scan.id) ||
        `${scan.projectName} ${scan.id}`
          .toLowerCase()
          .includes(normalizedSearch.value) ||
        searchableScanIds.value.has(scan.id)),
  ),
);

const queueScans = computed(() =>
  [...visibleScans.value].sort(
    (a, b) =>
      (({ draft: 0, queued: 1, scanning: 2, failed: 3, completed: 4 })[
        a.status
      ] ?? 5) -
      ({ draft: 0, queued: 1, scanning: 2, failed: 3, completed: 4 }[
        b.status
      ] ?? 5),
  ),
);
const resultTaskScans = computed(() =>
  resultTab.value === "vulnerabilities"
    ? visibleScans.value.filter((scan) => vulnerabilityScanIds.value.includes(scan.id))
    : visibleScans.value,
);
const scanTargets = computed(() =>
  selected.value
    ? targets.value.filter(
        (t) =>
          t.scanId === selected.value?.id &&
          (selected.value?.scanType !== "web" || isHttpUrl(t.url)),
      )
    : [],
);
const scanTargetByUrl = computed(() => new Map(scanTargets.value.map(item=>[item.url,item])));
const findingsByUrl = computed(() => {
  const map=new Map<string,SentinelFinding[]>();
  for(const finding of findings.value){
    const list=map.get(finding.targetUrl);
    if(list) list.push(finding); else map.set(finding.targetUrl,[finding]);
  }
  return map;
});
const targetUrls = computed(() => {
  const urls = [
    ...scanTargets.value.map((t) => t.url),
    ...findings.value.map((f) => f.targetUrl),
  ].filter(
    (v) =>
      v &&
      v !== "*" &&
      (selected.value?.scanType !== "web" || isHttpUrl(v)),
  );
  const unique = [...new Set(urls)];
  if (findings.value.some((f) => f.targetUrl === "*")) unique.push("*");
  return unique;
});
function companyForUrl(url: string) {
  return (
    scanTargetByUrl.value.get(url)?.company?.trim() ||
    "未提供公司"
  );
}
function targetForUrl(url: string) {
  return scanTargetByUrl.value.get(url);
}
const currentRows = computed(() => findingsByUrl.value.get(selectedUrl.value) || []);
const rows = (...kinds: string[]) =>
  currentRows.value.filter((f) => kinds.includes(f.kind));
const one = (kind: string) => {
  const item = currentRows.value.find((f) => f.kind === kind);
  return item ? json(item.recordJson) : undefined;
};
const urlCards = computed(() =>
  targetUrls.value.map((url) => {
    const list = findingsByUrl.value.get(url) || [];
    const target = targetForUrl(url);
    let fingerprints=0,apis=0,endpoints=0,vulnerabilities=0,pendingVulnerabilities=0,sensitive=0,high=0;
    for(const finding of list){
      if(["fingerprint","wordpress","tech_stack"].includes(finding.kind)) fingerprints++;
      if(["api","route","js_file"].includes(finding.kind)) apis++;
      if(finding.kind.includes("endpoint")||finding.kind==="directory_find") endpoints++;
      if(finding.kind==="sensitive_info"&&validSensitiveRecord(finding)) sensitive++;
      if(finding.kind==="vulnerability"){
        const severity=effectiveSeverity(finding);
        if(severity!=="none"){
          vulnerabilities++;
          const validation=validationFor(finding);
          if(!validation||validation.verdict==="pending") pendingVulnerabilities++;
          if(["critical","high"].includes(severity)) high++;
        }
      }
    }
    return {
      url,
      company: companyForUrl(url),
      status: target?.status || "",
      valueScore: target?.valueScore || 0,
      scanMode: target?.scanMode || "",
      scanCount: target?.scanCount || 0,
      routingReason: target?.routingReason || "",
      total: list.length,
      fingerprints,apis,endpoints,vulnerabilities,pendingVulnerabilities,sensitive,high,
    };
  }),
);
const filteredUrlCards = computed(() =>
  urlCards.value.filter((card) => {
    if (resultTab.value === "vulnerabilities" && card.vulnerabilities === 0)
      return false;
    return (
      !normalizedSearch.value ||
      `${card.company} ${card.url}`
        .toLowerCase()
        .includes(normalizedSearch.value)
    );
  }),
);
const previewTargetRows = computed(() =>
  previewUrls.value.map((url) => {
    const target = targets.value.find(
      (item) => item.scanId === previewScan.value?.id && item.url === url,
    );
    return {
      url,
      company: target?.company?.trim() || "未提供公司",
      highValue:
        (target?.valueScore || 0) >= 80 ||
        target?.scanMode === "deep" ||
        String(target?.routingReason || "").includes("高价值"),
    };
  }),
);
function scanHighValueCount(scanId: string) {
  return targets.value.filter(
    (target) =>
      target.scanId === scanId &&
      (target.valueScore >= 80 ||
        target.scanMode === "deep" ||
        String(target.routingReason || "").includes("高价值")),
  ).length;
}
const currentCard = computed(() =>
  urlCards.value.find((c) => c.url === selectedUrl.value),
);
const currentTarget = computed(() => targetForUrl(selectedUrl.value));
const fingerprint = computed(() => one("fingerprint") || {});
const wordpress = computed(() => one("wordpress") || {});
const techStack = computed(() => one("tech_stack") || {});
const fingerprintCards = computed(() => [
  {
    key: "frontend",
    label: "前端技术",
    data: fingerprint.value.frontend || techStack.value.framework || {},
  },
  { key: "backend", label: "后端框架", data: fingerprint.value.backend || {} },
  {
    key: "server",
    label: "Web 服务器",
    data: fingerprint.value.server || { name: techStack.value.server },
  },
  { key: "waf", label: "WAF", data: fingerprint.value.waf || {} },
  { key: "cdn", label: "CDN", data: fingerprint.value.cdn || {} },
]);
const securityHeaders = computed(() =>
  rows("security_header").map((item) => ({
    item,
    data: json(item.recordJson),
  })),
);
const requestHeaderIntelligence = computed<Record<string, any>>(() =>
  one("request_header_intelligence") || {},
);
const {
  apiRows, expandedApiRows, toggleApiRow, apiUrl, apiPath, apiQuery,
  apiRecord, apiMethod, apiResponseSummary, apiSourceSummary, apiDescription,
  apiRequestPayload, apiResponseHeaders, apiIdentitySummary,
} = useApiResults(selectedUrl, rows);

const realtimeEndpointRows = computed(() => rows("realtime_endpoint"));
const observedRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.observed)
    ? requestHeaderIntelligence.value.observed
    : [],
);
const declaredRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.declared)
    ? requestHeaderIntelligence.value.declared
    : [],
);
const possibleRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.possibleBrowserManaged)
    ? requestHeaderIntelligence.value.possibleBrowserManaged
    : [],
);
function headerDisplayValue(row: Record<string, any>) {
  const values = Array.isArray(row?.values) ? row.values : [];
  return (
    values
      .slice(0, 3)
      .map((item: any) =>
        String(item?.value || (item?.dynamic ? "<dynamic>" : "")),
      )
      .filter(Boolean)
      .join(" / ") || "—"
  );
}
const runtimeFeatureRows = computed(() => rows("runtime_feature"));
const runtimeActionRows = computed(() => rows("runtime_action"));
const observedMutationRows = computed(() => rows("observed_mutation"));
const registrationRows = computed(() => rows("registration_endpoint"));
const routeRows = computed(() => rows("route").filter(validRouteRecord));
const jsRows = computed(() => rows("js_file"));

function executionStageTone(attempt: SentinelScanAttempt, key: string) {
  const resolved = resolveExecutionStage(attempt);
  const order = EXECUTION_STAGE_STEPS.map((step) => step.key as string);
  const currentIndex = order.indexOf(resolved.current);
  const stepIndex = order.indexOf(key);
  if (resolved.historyMissing) return "missing";
  if (stepIndex < currentIndex) return "done";
  if (stepIndex === currentIndex) return "current";
  if (key === "agent" && !resolved.agentStarted) return "idle";
  return "pending";
}

const runtimeDiagnosticsFindings = computed(() => rows("runtime_diagnostics"));

function runtimeDiagnosticRows(item: SentinelFinding) {
  const record = json(item.recordJson) || {};
  const list = Array.isArray(record.runtimeDiagnostics) ? record.runtimeDiagnostics : [];
  return list.map((entry: any, index: number) => ({
    key: `${item.id}-${entry?.identityKey || index}`,
    identity: String(entry?.identityKey || entry?.identityLabel || `identity-${index + 1}`),
    captureStatus: String(entry?.captureStatus || "unknown"),
    captureError: String(entry?.captureError || ""),
    stopReason: String(entry?.runtimeStopReason || entry?.stopReason || ""),
    failedStage: String(entry?.failedStage || entry?.probeStage || ""),
    transport: String(entry?.cdpTransport || ""),
    browser: String(entry?.browserVersion || ""),
    exitCode: entry?.browserExitCode == null ? "" : String(entry.browserExitCode),
    signal: entry?.browserSignal == null ? "" : String(entry.browserSignal),
    stderr: String(entry?.browserStderr || ""),
  }));
}

const runtimeRows = computed(() =>
  rows("runtime_signal").filter(
    (item) =>
      !["cryptojs", "jsencrypt", "sm_crypto", "web_crypto"].includes(
        String(json(item.recordJson).type || ""),
      ),
  ),
);
const sensitiveRows = computed(() =>
  rows("sensitive_info").filter(validSensitiveRecord),
);
const cryptoRows = computed(() => rows("crypto_signal"));
const endpointRows = computed(() =>
  rows(
    "endpoint",
    "endpoint_expanded",
    "directory_find",
    "rest_endpoint",
    "login_endpoint",
  ),
);
const agentCoverageFinding = computed(() =>
  currentRows.value.find((item) => item.kind === "coverage_summary"),
);
const agentCoverage = computed<Record<string, any>>(() =>
  agentCoverageFinding.value
    ? json(agentCoverageFinding.value.recordJson)
    : {},
);
const agentCoverageEntries = computed<Record<string, any>[]>(() =>
  Array.isArray(agentCoverage.value.entries) ? agentCoverage.value.entries : [],
);
const agentCoverageGaps = computed<Record<string, any>[]>(() =>
  Array.isArray(agentCoverage.value.gaps) ? agentCoverage.value.gaps : [],
);
const agentExecution = shallowRef<AgentTargetExecution | null>(null);
const agentExecutionScope = shallowRef<{ scanId: string; targetUrl: string } | null>(null);
let agentExecutionGeneration = 0;

async function loadAgentExecution(scanId: string, url: string) {
  if (detailDisposed || selected.value?.id !== scanId || selectedUrl.value !== url) return;
  const generation = ++agentExecutionGeneration;
  agentExecution.value = null;
  agentExecutionScope.value = null;
  if (!scanId || !url || url === "*") {
    agentExecution.value = null;
    return;
  }
  try {
    const execution = await api.agentTargetExecution(scanId, url);
    if (generation !== agentExecutionGeneration || selected.value?.id !== scanId || selectedUrl.value !== url) return;
    agentExecution.value = execution;
    agentExecutionScope.value = { scanId, targetUrl: url };
  } catch {
    if (generation === agentExecutionGeneration) agentExecution.value = null;
  }
}

// "Nothing found" is only meaningful once the ledger closed; until then the honest
// statement is that verification is incomplete (§10).
const agentUnclosedGaps = computed(
  () =>
    (agentExecution.value?.coverage?.ledger?.uncoveredFamilies || []).filter(
      (row) => row.status !== "not_applicable"
    ).length
);

function agentCoverageOutcomeLabel(outcome: string) {
  return (
    {
      reported: "已形成发现",
      no_issue_found: "已测试，未发现问题",
      ruled_out: "已排除",
      not_applicable: "不适用",
      needs_follow_up: "需要继续验证",
    } as Record<string, string>
  )[String(outcome || "").toLowerCase()] || outcome || "未说明";
}
const vulnerabilityRows = computed(() => rows("vulnerability"));
const pocRows = computed(() => rows("poc_test"));
const isGreyboxScan = computed(() => selected.value?.scanType === "greybox");
const isCicdScan = computed(() => selected.value?.scanType === "cicd");
const sourceInventoryFinding = computed(() =>
  findings.value.find((item) => item.kind === "source_inventory"),
);
const sourceInventory = computed<Record<string, any>>(() =>
  sourceInventoryFinding.value
    ? json(sourceInventoryFinding.value.recordJson)
    : {},
);
const sourceFindingRows = computed(() =>
  findings.value.filter((item) => {
    if (item.kind === "dependency") {
      const data = json(item.recordJson);
      return Boolean(
        data.cve ||
        data.cwe ||
        data.cvss ||
        data.advisory ||
        data.vulnerable === true ||
        data.dependency_metadata?.fixed_version ||
        data.fixed_version,
      );
    }
    return (
      [
        "vulnerability",
        "code_smell",
        "security_hotspot",
        "secret",
        "sast",
        "dast",
        "iast",
      ].includes(item.kind) || item.kind.includes("vulnerability")
    );
  }),
);
const focusedSourceFindingRows = computed(() => {
  const selectedRow = sourceFindingRows.value.find((item) => item.id === selectedFindingId.value);
  return selectedRow ? [selectedRow] : sourceFindingRows.value.slice(0, 1);
});
const focusedVulnerabilityRows = computed(() => {
  const selectedRow = vulnerabilityRows.value.find((item) => item.id === selectedFindingId.value);
  return selectedRow ? [selectedRow] : vulnerabilityRows.value.slice(0, 1);
});
function vulnerabilityUpdateHistory(item: SentinelFinding) {
  const data = json(item.recordJson);
  const history = data.update_history || data.updateHistory;
  return Array.isArray(history) ? history : [];
}
function sourceLocations(item: SentinelFinding) {
  const data = json(item.recordJson);
  const nested = Array.isArray(data.code_locations)
    ? data.code_locations
    : data.code_location
      ? [data.code_location]
      : [];
  if (nested.length) return nested;
  if (data.file)
    return [
      {
        file: data.file,
        start_line: data.start_line ?? data.startLine,
        end_line: data.end_line ?? data.endLine,
        snippet: data.snippet,
      },
    ];
  return [];
}
const sourceLocationRows = computed(() =>
  sourceFindingRows.value.flatMap((item) =>
    sourceLocations(item).map((location: any) => ({
      item,
      data: json(item.recordJson),
      location,
    })),
  ),
);
const sourceSeverityCounts = computed(() =>
  ["critical", "high", "medium", "low", "info"].map((severity) => ({
    severity,
    count: sourceFindingRows.value.filter(
      (item) => effectiveSeverity(item) === severity,
    ).length,
  })),
);
const sourceLanguageRows = computed<any[]>(() =>
  Array.isArray(sourceInventory.value.languages)
    ? sourceInventory.value.languages
    : [],
);
const sourceLanguages = computed(() =>
  sourceLanguageRows.value.map((item) => String(item.name)).filter(Boolean),
);
const sourceFrameworks = computed<any[]>(() =>
  Array.isArray(sourceInventory.value.frameworks)
    ? sourceInventory.value.frameworks
    : [],
);
const sourceManifests = computed<string[]>(() =>
  Array.isArray(sourceInventory.value.manifests)
    ? sourceInventory.value.manifests
    : [],
);
const appsecVulnerabilities = computed(
  () => appsecResult.value.vulnerabilities || [],
);
function appsecSourcesFor(vulnerability: AppSecVulnerability) {
  return (appsecResult.value.sources || []).filter(
    (source) => source.vulnerabilityId === vulnerability.id,
  );
}
function sourceTypeLabel(value: string) {
  return (
    (
      {
        sast: "SAST",
        dast: "DAST",
        sca: "SCA",
        iast: "IAST",
        ai_validation: "AI 验证",
        scanner: "扫描器",
      } as Record<string, string>
    )[value] || value.toUpperCase()
  );
}
function authTypeLabel(value: string) {
  return (
    (
      {
        none: "匿名",
        cookie: "Cookie 会话",
        bearer: "Bearer Token",
        header: "自定义 Header",
      } as Record<string, string>
    )[value] || value
  );
}
function ciProviderLabel(value: string) {
  return (
    (
      {
        github: "GitHub Actions",
        gitlab: "GitLab CI",
        jenkins: "Jenkins",
        azure: "Azure Pipelines",
        other: "Other",
      } as Record<string, string>
    )[value] ||
    value ||
    "未记录"
  );
}
function gateStatusLabel(value: string) {
  return (
    (
      {
        passed: "通过",
        warning: "超限告警",
        blocked: "阻断发布",
        not_evaluated: "未评估",
      } as Record<string, string>
    )[value] || value
  );
}
function correlationParts(vulnerability: AppSecVulnerability) {
  const correlation = vulnerability.correlation || {};
  return [
    { key: "type", label: "漏洞类型 / CWE", ...correlation.type },
    { key: "url", label: "URL", ...correlation.url },
    { key: "parameter", label: "参数", ...correlation.parameter },
    { key: "dataFlow", label: "数据流", ...correlation.dataFlow },
  ];
}
const appsecSourceCounts = computed(() => {
  const counts: Record<string, number> = {};
  for (const source of appsecResult.value.sources || []) {
    counts[source.sourceType] = (counts[source.sourceType] || 0) + 1;
  }
  return counts;
});
const webEffectivePolicy = computed(
  () => appsecResult.value.context?.policy || ({} as Record<string, any>),
);
const webCoverageCatalog = computed(() =>
  Array.isArray(webEffectivePolicy.value.coverageCatalog)
    ? webEffectivePolicy.value.coverageCatalog
    : [],
);
function coverageStatusLabel(status: string) {
  return (
    {
      ready: "自动能力已就绪",
      automatic: "按目标自动准备",
      partial: "需要业务条件",
      not_configured: "当前环境不可测",
      disabled: "已关闭",
    } as Record<string, string>
  )[status] || status;
}
const greyboxCorrelated = computed(() =>
  appsecVulnerabilities.value.filter((vulnerability) => {
    const types = new Set(
      appsecSourcesFor(vulnerability).map((source) => source.sourceType),
    );
    return types.has("sast") && types.has("dast");
  }),
);
const sourceIssueGroups = computed(() => {
  const groups = new Map<
    string,
    { name: string; count: number; high: number }
  >();
  for (const vulnerability of appsecVulnerabilities.value) {
    const name =
      vulnerability.vulnerabilityType || vulnerability.title || "未分类问题";
    const current = groups.get(name) || { name, count: 0, high: 0 };
    current.count += 1;
    if (["critical", "high"].includes(vulnerability.severity))
      current.high += 1;
    groups.set(name, current);
  }
  return [...groups.values()].sort(
    (left, right) => right.high - left.high || right.count - left.count,
  );
});
const cicdBlockingFindings = computed(() => {
  const context = appsecResult.value.context;
  const maxCritical = Number(context?.policy?.maxCritical ?? 0);
  const maxHigh = Number(context?.policy?.maxHigh ?? 5);
  const critical = appsecVulnerabilities.value.filter(
    (item) => item.severity === "critical",
  );
  const high = appsecVulnerabilities.value.filter(
    (item) => item.severity === "high",
  );
  return [
    ...(critical.length > maxCritical ? critical : []),
    ...(high.length > maxHigh ? high : []),
  ];
});
const sourceDependencies = computed(() =>
  sourceFindingRows.value.filter((item) => {
    const data = json(item.recordJson);
    return (
      data.dependency_metadata ||
      data.package ||
      data.package_name ||
      data.installed_version ||
      data.cve
    );
  }),
);
const sourceStats = computed(() => ({
  findings: sourceFindingRows.value.length,
  files: new Set(
    sourceLocationRows.value.map((row) => row.location.file).filter(Boolean),
  ).size,
  locations: sourceLocationRows.value.length,
  rules: new Set(
    sourceFindingRows.value
      .map(
        (item) =>
          json(item.recordJson).rule_id ||
          json(item.recordJson).ruleId ||
          item.recordKey,
      )
      .filter(Boolean),
  ).size,
  totalFiles: Number(sourceInventory.value.totalFiles || 0),
  codeFiles: Number(sourceInventory.value.codeFiles || 0),
}));
const comparison = computed(() => {
  const oldMap = new Map(
    previousFindings.value.map((item) => [
      `${item.targetUrl}|${item.kind}|${item.recordKey}`,
      item,
    ]),
  );
  const newMap = new Map(
    findings.value.map((item) => [
      `${item.targetUrl}|${item.kind}|${item.recordKey}`,
      item,
    ]),
  );
  let added = 0,
    removed = 0,
    changed = 0,
    unchanged = 0;
  for (const [key, item] of newMap) {
    const old = oldMap.get(key);
    if (!old) added++;
    else if (
      old.recordJson !== item.recordJson ||
      old.severity !== item.severity
    )
      changed++;
    else unchanged++;
  }
  for (const key of oldMap.keys()) if (!newMap.has(key)) removed++;
  return { added, removed, changed, unchanged, total: newMap.size };
});
const selectedValidationWorkItems = computed(() =>
  validationWorkItems.value.filter((item) => {
    const pending = !item.validationId || ["pending", "needs_more"].includes(item.verdict);
    if (validationFilter.value === "pending" && !pending) return false;
    if (
      !["all", "pending"].includes(validationFilter.value) &&
      item.verdict !== validationFilter.value
    )
      return false;
    return (
      !normalizedSearch.value ||
      `${item.projectName} ${item.taskName} ${item.url} ${item.scanId} ${item.title}`
        .toLowerCase()
        .includes(normalizedSearch.value)
    );
  }),
);
const validationWorkStats = computed(() => ({
  pending: validationWorkItems.value.filter(
    (item) => !item.validationId || ["pending", "needs_more"].includes(item.verdict),
  ).length,
  confirmed: validationWorkItems.value.filter((item) => item.verdict === "true_positive").length,
  rejected: validationWorkItems.value.filter((item) => item.verdict === "false_positive").length,
}));
const visibleFuseEntries = computed(() =>
  fuseEntries.value.filter(
    (item) =>
      (fuseFilter.value === "all" ||
        (fuseFilter.value === "archived" ? item.archived : !item.archived)) &&
      (fuseCategoryFilter.value === "all" ||
        fuseReasonCategory(item) === fuseCategoryFilter.value) &&
      (!normalizedSearch.value ||
        `${item.company} ${item.url} ${item.reason} ${item.sourceScanId}`
          .toLowerCase()
          .includes(normalizedSearch.value)),
  ),
);
const routeReasonItems = computed(() =>
  String(currentTarget.value?.routingReason || "")
    .split("；")
    .map((item) => item.trim())
    .filter(Boolean),
);
function jumpToResult(next: ResultTab) {
  resultTab.value = next;
  window.requestAnimationFrame(() =>
    document
      .querySelector(".result-subtabs")
      ?.scrollIntoView({ behavior: "smooth", block: "start" }),
  );
}
const attentionTaskCount = computed(() =>
  scans.value.filter((scan) => ["draft", "paused", "partial", "failed"].includes(scan.status)).length,
);
const activeOpportunityStatuses = new Set(["queued", "ready", "in_progress"]);
const isVerifiableOpportunity = (item: SentinelOpportunity) =>
  item.score >= 65 && ["ready", "in_progress"].includes(item.status);
const isNoiseOpportunity = (item: SentinelOpportunity) => {
  const record = item.record || {};
  const method = String(record.method || record.httpMethod || "").toUpperCase();
  const endpoint = `${record.endpoint || record.route || item.targetUrl || ""}`.toLowerCase();
  return method === "OPTIONS" || /data_report_web|sentry|envelope|deviceprofile|telemetry/.test(endpoint);
};
type OpportunityIdentityRow = {
  label: string;
  state: string;
  detail: string;
  tone: "ok" | "warn" | "unknown";
  identityKey?: string;
};
function opportunityIdentityRows(item: SentinelOpportunity): OpportunityIdentityRow[] {
  const record = item.record || {};
  // Identity metadata is persisted on both the opportunity root and its
  // record_json depending on the producer version.  A/B rows must remain
  // symmetric even before a runtime probe has produced per-account results.
  const source: Record<string, any> = { ...(item as any), ...record };
  const runs = (Array.isArray(source.identityRuns) ? source.identityRuns : [])
    .filter((run: any) => String(run?.identityKey || "").trim().toLowerCase() !== "anonymous");
  const keys = [
    ...(Array.isArray(source.identityScopeKeys) ? source.identityScopeKeys : []),
    ...(Array.isArray(source.identityKeys) ? source.identityKeys : []),
    ...(Array.isArray(source.identityMatrix?.identities) ? source.identityMatrix.identities : []),
  ].map((value: any) =>
    typeof value === "string" ? value : String(value?.identityKey || value?.key || ""),
  ).filter((key) => key && key.trim().toLowerCase() !== "anonymous");
  const identityKeys = [...new Set([
    ...keys,
    ...runs.map((run: any) => String(run?.identityKey || "")).filter(Boolean),
  ])];
  if (!identityKeys.length && !runs.length) return [];
  const labels = Array.isArray(source.identityLabels) ? source.identityLabels : [];
  const runByKey = new Map(runs.map((run: any) => [String(run?.identityKey || ""), run]));
  const count = Math.min(Math.max(identityKeys.length, runs.length), 5);
  const displayKey = (key: string) => {
    if (!key) return "未提供身份标识";
    return key.length > 72 ? `${key.slice(0, 34)}…${key.slice(-30)}` : key;
  };
  return Array.from({ length: count }, (_, index) => {
    const key = identityKeys[index] || String(runs[index]?.identityKey || "");
    const run = runByKey.get(key) || (identityKeys.length ? undefined : runs[index]) || {};
    const label = String(run?.identityLabel || (labels.length === count ? labels[index] : "") || `账号 ${String.fromCharCode(65 + index)}`);
    const hasRuntimeResult = Boolean(run && Object.keys(run).length);
    const capture = String(run?.captureStatus || "").toLowerCase();
    const sessionValid = run?.sessionValid;
    const statusCode = run?.statusCode ? `HTTP ${run.statusCode}` : "无状态码";
    if (!hasRuntimeResult) {
      return {
        label,
        state: "待验证",
        detail: `尚未产生同一机会的运行时结果 · ${displayKey(key)}`,
        tone: "unknown",
        identityKey: key,
      };
    }
    if (run.observed === false) {
      return { label, state: "未观察到", detail: `${statusCode} · 该账号未产生同一机会请求`, tone: "unknown", identityKey: key };
    }
    if (sessionValid === false && run.validationReason && run.validationReason !== "runtime_probe_unavailable") {
      return { label, state: "明确失效", detail: `${statusCode} · ${run.validationReason}`, tone: "warn", identityKey: key };
    }
    if (capture === "failed" || capture === "partial" || capture === "unavailable" || sessionValid === null || run.validationReason === "runtime_probe_unavailable") {
      return { label, state: "不可判断", detail: `${statusCode} · runtime 未完成${run.captureError ? ` · ${run.captureError}` : ""}`, tone: "unknown", identityKey: key };
    }
    const apiCount = Number(run.apiCount || 0);
    return {
      label,
      state: capture === "complete" ? "已捕获" : "已关联",
      detail: `${statusCode} · ${apiCount} API · ${displayKey(key || "同请求身份")}`,
      tone: "ok",
      identityKey: key,
    };
  });
}

function opportunityIdentitySummary(item: SentinelOpportunity) {
  const rows = opportunityIdentityRows(item);
  if (!rows.length) return "身份范围未标记";
  const unknown = rows.filter((row) => row.tone === "unknown").length;
  if (rows.length === 1) return unknown ? "单账号已绑定 · 采集待补全" : "单账号已绑定";
  return unknown ? `A/B 已区分 · ${unknown} 个身份待补采` : `A/B 已区分 · ${rows.length} 个身份`;
}
const visibleOpportunities = computed(() =>
  opportunities.value.filter((item) => {
    if (isNoiseOpportunity(item)) return false;
    if (
      opportunityView.value === "ready" &&
      !isVerifiableOpportunity(item)
    )
      return false;
    if (
      opportunityView.value === "history" &&
      !["validated", "dismissed", "exhausted", "needs_more_evidence", "blocked_by_authorization", "closed"].includes(item.status)
    )
      return false;
    if (
      opportunityView.value === "all" &&
      !activeOpportunityStatuses.has(item.status)
    )
      return false;
    if (!normalizedSearch.value) return true;
    return `${item.title} ${item.targetUrl} ${item.category} ${JSON.stringify(item.record)}`
      .toLowerCase()
      .includes(normalizedSearch.value);
  }),
);
const activeOpportunityClues = computed(() =>
  opportunities.value
    .filter(
      (item) =>
        !isNoiseOpportunity(item) &&
        activeOpportunityStatuses.has(item.status) &&
        !isVerifiableOpportunity(item),
    )
    .sort((left, right) => right.score - left.score)
    .slice(0, 5),
);
const selectedUrlOpportunities = computed(() =>
  detailOpportunities.value.filter(
    (item) => !selectedUrl.value || sameTargetUrl(item.targetUrl, selectedUrl.value),
  ),
);
function validationForOpportunity(item: SentinelOpportunity) {
  return investigationValidations.value
    .filter((row) => row.opportunityId === item.id)
    .sort((left, right) => String(right.updatedAt).localeCompare(String(left.updatedAt)))[0];
}
function validationVerdictLabel(value?: string) {
  return ({ confirmed_issue: "确认存在问题", normal: "正常 / 可忽略", needs_more_evidence: "需要更多证据", unauthorized_stop: "权限边界 / 停止", request_failed: "请求失败", not_applicable: "不适用" } as Record<string, string>)[String(value || "")] || value || "未验证";
}
function validationOpportunityStatus(verdict?: string) {
  return ({
    confirmed_issue: "validated",
    normal: "dismissed",
    not_applicable: "dismissed",
    needs_more_evidence: "needs_more_evidence",
    unauthorized_stop: "exhausted",
    request_failed: "in_progress",
  } as Record<string, string>)[String(verdict || "")] || "in_progress";
}
function validationSummary(item: SentinelOpportunity) {
  const validation = validationForOpportunity(item);
  if (!validation) return "尚未验证";
  return `${validationVerdictLabel(validation.verdict)} · ${validation.responseStatus || "—"} · ${validation.updatedAt}`;
}
const evidenceNextAction = computed(() => {
  const pending = vulnerabilityRows.value.filter((item) => !validationFor(item)).length;
  if (pending)
    return { tone: "risk", label: `${pending} 个漏洞等待人工定性`, action: "vulnerabilities" };
  const ready = selectedUrlOpportunities.value.filter(isVerifiableOpportunity).length;
  if (ready)
    return { tone: "opportunity", label: `${ready} 个高价值机会可以直接验证`, action: "opportunities" };
  if (["limited", "protected_stop", "fuse_excluded"].includes(currentTarget.value?.status || ""))
    return { tone: "stopped", label: "该 URL 已停止，查看原因并决定是否恢复", action: "fuse" };
  if (endpointRows.value.length)
    return { tone: "endpoint", label: `${endpointRows.value.length} 个端点已响应，优先分析参数与鉴权`, action: "endpoints" };
  return { tone: "collect", label: "尚无高价值证据，检查 JS/API 情报后再决定是否续跑", action: "api" };
});
function followEvidenceNextAction() {
  if (evidenceNextAction.value.action === "fuse") tab.value = "fuse";
  else resultTab.value = evidenceNextAction.value.action as ResultTab;
}
const runningScanCount = computed(
  () => scans.value.filter((item) => ["queued", "scanning", "pausing"].includes(item.status)).length,
);
const opportunityCategoryLabel = (value: string) =>
  (
    {
      privilege_surface: "权限与管理",
      identity_surface: "身份与账户",
      file_surface: "文件处理",
      api_contract: "接口契约",
      business_transaction: "业务交易",
      administration: "配置与审计",
      data_query: "数据查询",
      api_surface: "接口测试面",
      product_match: "产品知识匹配",
      frontend_feature: "前端功能",
      fallback_discovery: "兜底发现",
    } as Record<string, string>
  )[value] || value;
const opportunityStatusLabel = (value: string) =>
  (
    {
      queued: "待自动补证",
      ready: "可直接验证",
      in_progress: "调查中",
      validated: "已验证",
      dismissed: "已忽略",
      exhausted: "无新增证据",
      needs_more_evidence: "需要更多证据",
      blocked_by_authorization: "已由自动策略接管",
      closed: "已关闭",
    } as Record<string, string>
  )[value] || value;
function opportunityEndpoint(item: SentinelOpportunity) {
  return String(item.record?.endpoint || item.record?.route || item.targetUrl || "");
}
function normalizeReplayTarget(value: string) {
  const raw = String(value || "").trim();
  if (!raw) return "";
  try {
    return new URL(raw, selectedUrl.value || "http://localhost") .pathname.replace(/\/$/, "").toLowerCase();
  } catch {
    return raw.split("?")[0].split("#")[0].replace(/\/$/, "").toLowerCase();
  }
}
function opportunityMethod(item: SentinelOpportunity) {
  return String(item.record?.method || item.record?.httpMethod || item.recommendedAction?.method || "GET").toUpperCase();
}
function findReplayApiFromGraph(item: SentinelOpportunity, graph: InvestigationGraph) {
  const method = opportunityMethod(item);
  const endpoint = opportunityEndpoint(item);
  const endpointPath = normalizeReplayTarget(endpoint);
  const endpointUrl = endpoint.toLowerCase().replace(/\/$/, "");
  const sourceApiKey = String(item.record?.apiKey || item.record?.api_key || "");
  return [...graph.apis].map((api) => {
    const apiUrl = String(api.url || "").toLowerCase().replace(/\/$/, "");
    const apiPath = normalizeReplayTarget(api.url || api.normalizedPath);
    const sameMethod = String(api.method || "").toUpperCase() === method;
    const exactKey = Boolean(sourceApiKey && api.apiKey === sourceApiKey);
    const exactUrl = Boolean(endpointUrl && apiUrl === endpointUrl);
    const exactPath = Boolean(endpointPath && apiPath === endpointPath);
    const relatedPath = Boolean(endpointPath && (apiPath.endsWith(endpointPath) || endpointPath.endsWith(apiPath)));
    const relatedUrl = Boolean(endpointUrl && (apiUrl.endsWith(endpointUrl) || endpointUrl.endsWith(apiUrl)));
    let score = sameMethod ? 1000 : 0;
    if (exactKey) score += 5000;
    if (exactUrl) score += 4000;
    if (exactPath) score += 3000;
    if (relatedPath) score += 1200;
    if (relatedUrl) score += 900;
    return { api, score, matchedTarget: exactKey || exactUrl || exactPath || relatedPath || relatedUrl };
  }).filter((entry) => entry.matchedTarget && entry.score > 0).sort((left, right) => right.score - left.score)[0]?.api;
}
function syntheticOpportunityForReplay(api: any, hypothesis?: InvestigationHypothesis): SentinelOpportunity {
  const method = String(api?.method || hypothesis?.contract?.method || "GET").toUpperCase();
  const endpoint = String(api?.url || hypothesis?.contract?.endpoint || selectedUrl.value || selectedUrl.value || "");
  return {
    id: 0, projectId: selected.value?.projectId, scanId: selected.value?.id || hypothesis?.scanId || "",
    targetUrl: selectedUrl.value || hypothesis?.targetUrl || selectedUrl.value || "",
    opportunityKey: `replay:${api?.apiKey || hypothesis?.hypothesisKey || endpoint}`,
    category: hypothesis?.category || "manual_replay", title: hypothesis?.title || `手动重放 ${method} ${endpoint}`,
    score: hypothesis?.score || 0, status: "in_progress", confidence: hypothesis?.confidence || "medium",
    why: ["来自调查图谱的完整请求证据"], evidence: [], source: "investigation_graph",
    recommendedAction: { method, targetUrl: endpoint }, record: { apiKey: api?.apiKey, method, endpoint, url: endpoint },
    firstSeen: new Date().toISOString(), lastSeen: new Date().toISOString(),
  };
}
function findOpportunityForReplay(api: any, hypothesis?: InvestigationHypothesis) {
  const endpoint = String(api?.url || hypothesis?.contract?.endpoint || "").toLowerCase().replace(/\/$/, "");
  const method = String(api?.method || hypothesis?.contract?.method || "GET").toUpperCase();
  return [...detailOpportunities.value, ...opportunities.value].find((item) => {
    const record = item.record || {};
    const itemEndpoint = String(record.url || record.endpoint || record.route || item.targetUrl || "").toLowerCase().replace(/\/$/, "");
    const itemMethod = String(record.method || record.httpMethod || item.recommendedAction?.method || "GET").toUpperCase();
    return (api?.apiKey && String(record.apiKey || record.api_key || "") === String(api.apiKey)) ||
      (itemMethod === method && (itemEndpoint === endpoint || itemEndpoint.endsWith(endpoint) || endpoint.endsWith(itemEndpoint)));
  });
}
function openRepeaterForGraphApi(api: any, hypothesis?: InvestigationHypothesis) {
  const opportunity = findOpportunityForReplay(api, hypothesis) || syntheticOpportunityForReplay(api, hypothesis);
  repeaterOpportunity.value = opportunity;
  repeaterApi.value = api;
  repeaterHypothesis.value = hypothesis;
}
function openRepeaterForHypothesis(hypothesis: InvestigationHypothesis) {
  const graph = investigationGraph.value;
  if (!graph) { emit("notify", "error", "调查图谱尚未加载，无法打开请求重放"); return; }
  const method = String(hypothesis.contract?.method || "GET").toUpperCase();
  const endpoint = String(hypothesis.contract?.endpoint || "");
  const endpointPath = normalizeReplayTarget(endpoint);
  const endpointUrl = endpoint.trim().toLowerCase().replace(/[?#].*$/, "").replace(/\/$/, "");
  const evidenceApiKey = hypothesis.evidence && !Array.isArray(hypothesis.evidence) ? hypothesis.evidence.apiKey : undefined;
  const sourceApiKey = String(hypothesis.contract?.apiKey || evidenceApiKey || "");
  const api = graph.apis
    .map((candidate) => {
      const candidateMethod = String(candidate.method || "").toUpperCase();
      const candidateUrl = String(candidate.url || "").trim().toLowerCase().replace(/[?#].*$/, "").replace(/\/$/, "");
      const candidatePath = normalizeReplayTarget(candidate.url || candidate.normalizedPath || "");
      const sameMethod = candidateMethod === method;
      const exactKey = Boolean(sourceApiKey && String(candidate.apiKey || "") === sourceApiKey);
      const exactUrl = Boolean(endpointUrl && candidateUrl === endpointUrl);
      const exactPath = Boolean(endpointPath && candidatePath === endpointPath);
      const relatedPath = Boolean(endpointPath && (candidatePath.endsWith(endpointPath) || endpointPath.endsWith(candidatePath)));
      const score = (sameMethod ? 1000 : 0) + (exactKey ? 5000 : 0) + (exactUrl ? 4000 : 0) + (exactPath ? 3000 : 0) + (relatedPath ? 1000 : 0);
      return { candidate, score, reliable: sameMethod && (exactKey || exactUrl || exactPath) };
    })
    .filter((entry) => entry.reliable)
    .sort((left, right) => right.score - left.score)[0]?.candidate;
  openRepeaterForGraphApi(api, hypothesis);
}
async function refreshSelectedEvidenceAfterValidation() {
  const scan = selected.value;
  if (!scan) return;
  const scanId = scan.id;
  const project = projectFilter.value;
  const generation = detailGeneration;
  const [
    nextOpportunities,
    nextDetailOpportunities,
    nextValidations,
    nextInvestigationValidations,
    ,
    ,
    nextVulnerabilityScanIds,
    nextAppsecResult,
  ] = await Promise.all([
    api.listSentinelOpportunities(projectFilter.value, undefined, undefined, 800),
    api.listSentinelOpportunities(undefined, scanId, undefined, 800),
    api.listSentinelValidations(scanId),
    api.listInvestigationValidations(scanId),
    refreshOverviewStats(project),
    refreshInvestigationSummary(),
    api.listSentinelVulnerabilityScanIds(projectFilter.value),
    api.listAppSecScanResult(scanId),
  ]);
  if (detailDisposed || project !== projectFilter.value || selected.value?.id !== scanId || generation !== detailGeneration) return;
  opportunities.value = nextOpportunities;
  detailOpportunities.value = nextDetailOpportunities;
  validations.value = nextValidations;
  investigationValidations.value = nextInvestigationValidations;
  vulnerabilityScanIds.value = nextVulnerabilityScanIds;
  appsecResult.value = nextAppsecResult;
  if (scan.scanType === "web" && selectedUrl.value && selectedUrl.value !== "*") {
    await loadInvestigationGraph(scanId, selectedUrl.value);
  }
}
async function handleRepeaterSaved(validation: InvestigationValidation) {
  // Saving is terminal for the Repeater. Apply the status/count transition
  // optimistically and close immediately; a slow secondary projection (graph,
  // evidence center, action center) must never leave the modal blocking the
  // inbox or make the user click twice.
  const nextStatus = validationOpportunityStatus(validation.verdict);
  const opportunityId = validation.opportunityId || repeaterOpportunity.value?.id;
  const replace = (rows: SentinelOpportunity[]) => rows.map((row) =>
    row.id === opportunityId ? { ...row, status: nextStatus } : row,
  );
  const previous = repeaterOpportunity.value;
  const wasActive = Boolean(previous && activeOpportunityStatuses.has(previous.status));
  const wasReady = Boolean(previous && isVerifiableOpportunity(previous));
  opportunities.value = replace(opportunities.value);
  detailOpportunities.value = replace(detailOpportunities.value);
  if (wasActive || wasReady) {
    stats.value = {
      ...stats.value,
      opportunityCount: Math.max(0, stats.value.opportunityCount - (wasActive ? 1 : 0)),
      readyOpportunityCount: Math.max(0, stats.value.readyOpportunityCount - (wasReady ? 1 : 0)),
      validatedCount: validation.verdict === "confirmed_issue"
        ? stats.value.validatedCount + 1
        : stats.value.validatedCount,
    };
  }
  repeaterOpportunity.value = undefined;
  repeaterApi.value = undefined;
  repeaterHypothesis.value = undefined;
  emit("notify", "success", `验证结论已保存：${validationVerdictLabel(validation.verdict)}；验证器已关闭，机会数量已更新`);
  try {
    await refreshSelectedEvidenceAfterValidation();
    emit("notify", "success", "已同步机会、调查图谱、证据中心和行动中心");
  } catch (error) {
    emit("notify", "info", `验证已保存，页面已即时更新；后台同步稍后重试：${String(error)}`);
  }
}
async function openRepeaterForOpportunity(item: SentinelOpportunity) {
  if (item.status !== "in_progress") {
    await setOpportunityStatus(item, "in_progress");
  }
  // setOpportunityStatus updates the collections asynchronously. Re-read the
  // row so the repeater owns the in_progress snapshot; otherwise a queued row
  // stays queued inside the modal and the terminal save cannot decrement the
  // active/ready counters.
  const current = [...opportunities.value, ...detailOpportunities.value]
    .find((row) => row.id === item.id) || { ...item, status: "in_progress" };
  repeaterOpportunity.value = current;
  repeaterApi.value = undefined;
  repeaterHypothesis.value = undefined;
  try {
    const graph = await api.getInvestigationGraph(item.scanId, item.targetUrl);
    repeaterApi.value = findReplayApiFromGraph(item, graph);
    repeaterHypothesis.value = graph.hypotheses.find((hypothesis) => hypothesis.sourceOpportunityKey === item.opportunityKey)
      || graph.hypotheses.find((hypothesis) => String(hypothesis.contract?.endpoint || "") === opportunityEndpoint(item));
  } catch (error) {
    emit("notify", "info", `已打开独立 Repeater，但未能加载调查 API：${String(error)}`);
  }
}
function opportunityParameters(item: SentinelOpportunity) {
  return Array.isArray(item.record?.parameters)
    ? item.record.parameters.map(String).filter(Boolean)
    : [];
}
function opportunityKnowledge(item: SentinelOpportunity) {
  return Array.isArray(item.record?.knowledgeMatches)
    ? item.record.knowledgeMatches
    : [];
}
function opportunityKnowledgeTitles(item: SentinelOpportunity) {
  return opportunityKnowledge(item)
    .slice(0, 3)
    .map((value: any) => String(value?.title || "未命名知识"))
    .join(" / ");
}
function opportunityEvidenceCount(item: SentinelOpportunity) {
  return Math.max(
    Array.isArray(item.evidence) ? item.evidence.length : 0,
    Array.isArray(item.record?.evidenceRefs)
      ? item.record.evidenceRefs.length
      : 0,
  );
}
const evidenceChainRows = computed(() => {
  type EvidenceRow = {
    key: string;
    method: string;
    url: string;
    sources: string[];
    parameters: string[];
    statusCode: string;
    verified: boolean;
    opportunityScore: number;
    vulnerabilities: number;
  };
  const result = new Map<string, EvidenceRow>();
  const add = (item: SentinelFinding, verified: boolean) => {
    const data = json(item.recordJson);
    const method = String(data.method || "GET").toUpperCase();
    const path = String(data.url || data.path || data.endpoint || "").trim();
    if (!path) return;
    const url = endpointUrl(selectedUrl.value, path);
    const key = `${method}|${url}`;
    const current = result.get(key) || {
      key,
      method,
      url,
      sources: [],
      parameters: [],
      statusCode: "",
      verified: false,
      opportunityScore: 0,
      vulnerabilities: 0,
    };
    current.sources = [...new Set([...current.sources, String(data.source || kindLabel(item.kind))])];
    const parameters = Array.isArray(data.parameters)
      ? data.parameters.map((value: any) => String(value?.name || value)).filter(Boolean)
      : data.parameters && typeof data.parameters === "object"
        ? Object.keys(data.parameters)
        : [];
    current.parameters = [...new Set([...current.parameters, ...parameters])].slice(0, 12);
    current.statusCode = String(data.statusCode || current.statusCode || "");
    current.verified ||= verified || Boolean(data.statusCode);
    result.set(key, current);
  };
  apiRows.value.forEach((item) => add(item, false));
  registrationRows.value.forEach((item) => add(item, false));
  endpointRows.value.forEach((item) => add(item, true));
  for (const row of result.values()) {
    const normalized = row.url.toLowerCase().replace(/\/$/, "");
    row.opportunityScore = Math.max(
      0,
      ...selectedUrlOpportunities.value
        .filter((item) => {
          const endpoint = opportunityEndpoint(item).toLowerCase().replace(/\/$/, "");
          return endpoint && (normalized.endsWith(endpoint) || endpoint.endsWith(normalized));
        })
        .map((item) => item.score),
    );
    row.vulnerabilities = vulnerabilityRows.value.filter((item) => {
      const data = json(item.recordJson);
      const target = String(data.url || data.endpoint || data.path || "").toLowerCase().replace(/\/$/, "");
      return target && (normalized.endsWith(target) || target.endsWith(normalized));
    }).length;
  }
  return [...result.values()]
    .sort((a, b) => b.vulnerabilities - a.vulnerabilities || b.opportunityScore - a.opportunityScore || Number(b.verified) - Number(a.verified))
    .slice(0, 30);
});
async function setOpportunityStatus(item: SentinelOpportunity, status: string) {
  opportunityBusy.value = item.id;
  try {
    await api.updateSentinelOpportunityStatus(item.id, status);
    const replace = (rows: SentinelOpportunity[]) =>
      rows.map((row) => (row.id === item.id ? { ...row, status } : row));
    opportunities.value = replace(opportunities.value);
    detailOpportunities.value = replace(detailOpportunities.value);
    await refreshOverviewStats();
  } catch (error) {
    emit("notify", "error", `机会状态更新失败：${String(error)}`);
  } finally {
    opportunityBusy.value = 0;
  }
}
async function openOpportunity(item: SentinelOpportunity, markInProgress = false) {
  if (markInProgress) {
    await openRepeaterForOpportunity(item);
    return;
  }
  const scan = scans.value.find((value) => value.id === item.scanId);
  if (!scan) {
    emit("notify", "error", "对应任务不在当前项目筛选范围内");
    return;
  }
  await openScan(scan);
  selectedUrl.value = item.targetUrl;
  resultTab.value = "opportunities";
}
const transferProjectId = computed(
  () => projectFilter.value || selected.value?.projectId,
);
let liveSyncing = false;
const authCaptureTimers = new Map<string, number>();
function stopAuthCapturePolling(sessionId: string) {
  const timer = authCaptureTimers.get(sessionId);
  if (timer !== undefined) window.clearInterval(timer);
  authCaptureTimers.delete(sessionId);
}
function closeAuthRecovery() {
  for (const sessionId of authCaptureTimers.keys()) stopAuthCapturePolling(sessionId);
  authRecoveryScan.value = undefined;
  authRecoveryAutoContinuing.value = false;
}
function startAuthCapturePolling(session: BrowserAuthSession) {
  stopAuthCapturePolling(session.id);
  const timer = window.setInterval(async () => {
    if (authRecoveryBusy.value && authRecoveryBusy.value !== session.id) return;
    try {
      const updated = await api.finishBrowserAuthSession(session.id);
      await reloadAuthRecoverySessions();
      if (updated.status === "valid") {
        stopAuthCapturePolling(session.id);
        void maybeAutoContinueAfterAuthRecovery();
      }
    } catch {
      // The user may close the login window manually. Stop polling instead of
      // turning an intentional close into a persistent error notification.
      stopAuthCapturePolling(session.id);
    }
  }, 2500);
  authCaptureTimers.set(session.id, timer);
}
async function load() {
  if (detailDisposed) return;
  invalidateScanPagination();
  const generation = ++loadGeneration;
  const project = projectFilter.value;
  const current = () => !detailDisposed && generation === loadGeneration && project === projectFilter.value;
  loading.value = true;
  try {
    const [
      nextScans,
      nextTargets,
      ,
      ,
      nextVulnerabilityScanIds,
      nextOpportunities,
    ] = await Promise.all([
      api.listSentinelScans(project, scanPageSize),
      api.listSentinelTargets(project),
      refreshOverviewStats(project),
      refreshInvestigationSummary(),
      api.listSentinelVulnerabilityScanIds(project),
      api.listSentinelOpportunities(project, undefined, undefined, 800),
    ]);
    if (!current()) return;
    replaceScanHead(nextScans);
    targets.value = nextTargets;
    vulnerabilityScanIds.value = nextVulnerabilityScanIds;
    opportunities.value = nextOpportunities;
    if (tab.value === "fuse") {
      const entries = await api.listSentinelFuseZone(project);
      if (!current()) return;
      fuseEntries.value = entries;
    }
    if (tab.value === "validations") {
      const items = await api.listSentinelValidationWorkItems(project);
      if (!current()) return;
      validationWorkItems.value = items;
    }
    if (tab.value === "validations" && !validationWorkEditor.value) {
      const first = selectedValidationWorkItems.value[0];
      if (first) editValidationWorkItem(first);
    }
    if (tab.value === "queue" && !previewScan.value && queueScans.value[0])
      await preview(queueScans.value[0]);
    if (!current()) return;
    if (selected.value) {
      const fresh = scans.value.find((s) => s.id === selected.value?.id);
      if (fresh) await openScan(fresh, false);
    } else if (tab.value === "results" && resultTaskScans.value[0])
      await openScan(resultTaskScans.value[0], false);
  } catch (e) {
    if (current()) emit("notify", "error", String(e));
  } finally {
    if (current()) loading.value = false;
  }
}
async function liveSync() {
  const hasActiveScan = scans.value.some((scan) => ["scanning", "pausing"].includes(scan.status));
  if (
    detailDisposed ||
    liveSyncing ||
    loading.value ||
    overviewRequests > 0 ||
    !props.active ||
    document.hidden ||
    (!hasActiveScan && !selected.value)
  )
    return;
  liveSyncing = true;
  const project = projectFilter.value;
  const generation = loadGeneration;
  const current = () => !detailDisposed && project === projectFilter.value && generation === loadGeneration;
  try {
    // Current task state comes from the DB, including late terminal updates.
    // This view never discovers or imports external historical directories.
    if (!current()) return;
    const [
      nextScans,
      nextTargets,
      ,
      ,
      nextVulnerabilityScanIds,
      nextOpportunities,
    ] = await Promise.all([
      api.listSentinelScans(projectFilter.value, scanPageSize),
      api.listSentinelTargets(projectFilter.value),
      refreshOverviewStats(project),
      refreshInvestigationSummary(),
      api.listSentinelVulnerabilityScanIds(projectFilter.value),
      api.listSentinelOpportunities(projectFilter.value, undefined, undefined, 800),
    ]);
    if (!current()) return;
    mergeScanPage(nextScans);
    targets.value = nextTargets;
    vulnerabilityScanIds.value = nextVulnerabilityScanIds;
    opportunities.value = nextOpportunities;
    if (selected.value && !detailBusy.value) {
      const fresh = nextScans.find((scan) => scan.id === selected.value?.id);
      if (fresh) selected.value = fresh;
      const scan = selected.value;
      const generation = ++detailGeneration;
      const details = await readResultDetails(scan);
      if (detailDisposed || generation !== detailGeneration || selected.value?.id !== scan.id) return;
      publishResultDetails(details);
      await loadSelectedTrace(scan.id);
    }
  } catch {
    /* 后台轮询失败时保留上次结果，手动同步会显示具体错误。 */
  } finally {
    liveSyncing = false;
  }
}
function selectResultTask(event: Event) {
  const id = String((event.target as HTMLSelectElement).value || "");
  const scan = resultTaskScans.value.find((item) => item.id === id);
  if (scan) void openScan(scan, false);
}
function selectResultUrl(event: Event) {
  selectedUrl.value = String((event.target as HTMLSelectElement).value || "");
  if (resultTab.value !== "vulnerabilities") resultTab.value = "summary";
}
function clearResultDetails() {
  ++detailGeneration;
  resetSelectedTrace();
  ++graphGeneration;
  ++agentExecutionGeneration;
  checkpoints.value = [];
  findings.value = [];
  validations.value = [];
  investigationValidations.value = [];
  previousFindings.value = [];
  detailOpportunities.value = [];
  scanAttempts.value = [];
  appsecResult.value = { vulnerabilities: [], sources: [] };
  investigationGraph.value = undefined;
  investigationBusy.value = false;
  agentExecution.value = null;
  agentExecutionScope.value = null;
  selectedFindingId.value = undefined;
  detailBusy.value = false;
}
async function readResultDetails(scan: SentinelScan) {
  return Promise.all([
    api.listSentinelCheckpoints(scan.id),
    api.listSentinelFindings(scan.id),
    api.listSentinelValidations(scan.id),
    api.listInvestigationValidations(scan.id),
    scan.previousScanId ? api.listSentinelFindings(scan.previousScanId).catch(() => []) : Promise.resolve([]),
    api.listSentinelOpportunities(undefined, scan.id, undefined, 800),
    api.listSentinelScanAttempts(scan.id),
    scan.scanType === "web" ? Promise.resolve({ vulnerabilities: [], sources: [] }) : api.listAppSecScanResult(scan.id),
  ]);
}
function publishResultDetails(details: Awaited<ReturnType<typeof readResultDetails>>) {
  [checkpoints.value, findings.value, validations.value, investigationValidations.value,
    previousFindings.value, detailOpportunities.value, scanAttempts.value, appsecResult.value] = details;
}
async function openScan(scan: SentinelScan, jump = true) {
  if (selected.value?.id !== scan.id) showAttemptHistory.value = false;
  selected.value = scan;
  clearResultDetails();
  const generation = detailGeneration;
  const current = () => !detailDisposed && generation === detailGeneration && selected.value?.id === scan.id;
  if (jump) {
    tab.value = "results";
    resultTab.value = "summary";
  }
  detailBusy.value = true;
  try {
    const details = await readResultDetails(scan);
    if (!current()) return;
    publishResultDetails(details);
    const selectableUrls =
      resultTab.value === "vulnerabilities"
        ? urlCards.value.filter((card) => card.vulnerabilities > 0).map((card) => card.url)
        : targetUrls.value;
    const withEvidence = selectableUrls.find(
      (url) => (findingsByUrl.value.get(url) || []).length > 0,
    );
    selectedUrl.value = selectableUrls.includes(selectedUrl.value)
      ? selectedUrl.value
      : withEvidence || selectableUrls[0] || "";
    selectedFindingId.value =
      (scan.scanType === "web" ? vulnerabilityRows.value[0] : sourceFindingRows.value[0])?.id;
    if (scan.scanType === "web" && selectedUrl.value && selectedUrl.value !== "*") {
      await loadInvestigationGraph(scan.id, selectedUrl.value);
      if (!current()) return;
      await loadAgentExecution(scan.id, selectedUrl.value);
    } else {
      investigationGraph.value = undefined;
      agentExecution.value = null;
    }
    if (current()) await loadSelectedTrace(scan.id);
  } catch (e) {
    if (current()) emit("notify", "error", String(e));
  } finally {
    if (current()) detailBusy.value = false;
  }
}

async function loadInvestigationGraph(scanId?: string, url?: string) {
  if (detailDisposed || selected.value?.id !== scanId || selectedUrl.value !== url) return;
  const generation = ++graphGeneration;
  const current = () => !detailDisposed && generation === graphGeneration
    && selected.value?.id === scanId && selectedUrl.value === url;
  investigationGraph.value = undefined;
  if (!scanId || !url || url === "*") {
    investigationGraph.value = undefined;
    investigationBusy.value = false;
    return;
  }
  investigationBusy.value = true;
  try {
    const graph = await api.getInvestigationGraph(scanId, url);
    if (current()) investigationGraph.value = graph;
  } catch (error) {
    if (current()) {
      investigationGraph.value = undefined;
      emit("notify", "error", `调查图谱加载失败：${String(error)}`);
    }
  } finally {
    if (current()) investigationBusy.value = false;
  }
}

async function updateInvestigationStatus(item: InvestigationHypothesis, status: string) {
  investigationUpdatingId.value = item.id;
  try {
    await api.updateInvestigationHypothesis(item.id, status);
    await loadInvestigationGraph(item.scanId, item.targetUrl);
    emit("notify", "success", status === "in_progress" ? "已进入有界验证队列" : "调查假设状态已更新");
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    investigationUpdatingId.value = undefined;
  }
}
async function preview(scan: SentinelScan) {
  previewScan.value = scan;
  const own = targets.value
    .filter((t) => t.scanId === scan.id)
    .map((t) => t.url);
  if (own.length) {
    previewUrls.value = [...new Set(own)];
    return;
  }
  try {
    const list = await api.listSentinelFindings(scan.id);
    previewUrls.value = [
      ...new Set(list.map((f) => f.targetUrl).filter((u) => u && u !== "*")),
    ];
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
async function prepareWorkbenchScan(scan: SentinelScan) {
  // This handoff never confirms a task. Refresh even after a start transport
  // error: the backend may already have advanced beyond the returned draft.
  projectFilter.value = scan.projectId;
  mergeScanPage([scan]);
  previewScan.value = scan;
  previewUrls.value = [];
  tab.value = "queue";
  await load();
  if (tab.value !== "queue" || previewScan.value?.id !== scan.id) return;
  await preview(scans.value.find((item) => item.id === scan.id) || scan);
}
function refreshScanReference(scanId: string) {
  const fresh = scans.value.find((item) => item.id === scanId);
  if (!fresh) return;
  if (selected.value?.id === scanId) selected.value = fresh;
  if (previewScan.value?.id === scanId) previewScan.value = fresh;
}
async function onAttemptClosed(scanId: string, attemptNumber: number) {
  // Receipt notification only refreshes the existing view; never call rescan or
  // authentication auto-continuation from a closure or a status poll.
  try {
    await load();
    if (selected.value?.id === scanId && selected.value.attemptCount === attemptNumber) {
      refreshScanReference(scanId);
    }
  } catch (error) {
    emit("notify", "error", `结案后列表刷新失败，请手动刷新：${String(error)}`);
  }
}
function onTaskArchived(scan: SentinelScan) {
  mergeScanPage([scan]);
  refreshScanReference(scan.id);
  emit("notify", "success", scan.archivedAt ? "任务已归档，历史证据仍可查看" : "任务已从归档恢复");
}
async function confirm(scan: SentinelScan) {
  try {
    await api.confirmSentinelScan(scan.id);
    emit("notify", "success", "任务已确认，前端深度解析与原生 Web 调查已启动");
    await load();
    await preview(scans.value.find((s) => s.id === scan.id) || scan);
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
async function pauseScan(scan: SentinelScan) {
  if (!window.confirm("确定请求暂停吗？系统会等待执行线程退出后标记“已暂停”；已保存证据、未知目标效果和未确认清理记录会保留。")) return;
  scanControlBusy.value = scan.id;
  try {
    await api.pauseSentinelScan(scan.id);
    emit(
      "notify",
      "success",
      "暂停请求已接收；等待执行线程退出。未知目标效果不会被撤销，也不会自动重发请求",
    );
    await load();
    refreshScanReference(scan.id);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function resumeScan(scan: SentinelScan) {
  scanControlBusy.value = scan.id;
  try {
    await api.resumeSentinelScan(scan.id);
    emit(
      "notify",
      "success",
      scan.scanType && scan.scanType !== "web"
        ? "任务已在原记录中启动新的续跑尝试；历史发现和累计成本保持不变"
        : "任务已恢复，将从下一个未完成 URL 继续",
    );
    await load();
    refreshScanReference(scan.id);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function executeRescan(scan: SentinelScan) {
  if (scan.scanType && scan.scanType !== "web") {
    const next = await api.rescanWorkbenchScan(scan.id);
    await load();
    await openScan(scans.value.find((item) => item.id === next.id) || next);
    emit(
      "notify",
      "success",
      "已在当前工作台任务中重新执行；运行产物按尝试隔离，累计成本继续保留",
    );
    return;
  }
  const next = await api.rescanSentinelScan(scan.id);
  await load();
  const fresh = scans.value.find((item) => item.id === next.id) || next;
  await openScan(fresh);
  emit(
    "notify",
    "success",
    ["partial", "failed", "limited", "cancelled"].includes(scan.status)
      ? "已继续未完成阶段：保留正式接口、登录会话和可验证证据；旧错误与旧状态仅保留在执行历史中"
      : "已全新执行：当前结果面已清理，只展示本轮状态与新证据；旧轮次仅保留在折叠的执行历史中",
  );
}
async function rescan(scan: SentinelScan) {
  scanControlBusy.value = scan.id;
  try {
    if (!scan.scanType || scan.scanType === "web") {
      const boundSessions = await api.listSentinelScanAuthSessions(scan.id);
      const sessions = await Promise.all(
        boundSessions.map((session) =>
          session.status === "valid"
            ? api.validateBrowserAuthSession(session.id)
            : Promise.resolve(session),
        ),
      );
      if (sessions.some((session) => session.status !== "valid")) {
        authRecoveryScan.value = scan;
        authRecoverySessions.value = sessions;
        authRecoveryAutoContinuing.value = false;
        emit("notify", "info", "登录会话已失效：请在当前任务内重新登录，绿灯后即可自动续扫");
        return;
      }
    }
    await executeRescan(scan);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function reloadAuthRecoverySessions() {
  if (!authRecoveryScan.value) return;
  authRecoverySessions.value = await api.listSentinelScanAuthSessions(authRecoveryScan.value.id);
}
async function reopenAuthRecovery(session: BrowserAuthSession) {
  if (!authRecoveryScan.value) return;
  authRecoveryBusy.value = session.id;
  try {
    const opened = await api.openBrowserAuthSession({
      id: session.id,
      projectId: session.projectId,
      name: session.name,
      entryUrl: session.entryUrl,
      scanId: authRecoveryScan.value.id,
    });
    startAuthCapturePolling(opened);
    await reloadAuthRecoverySessions();
    emit("notify", "info", "登录窗口已打开；登录成功并访问后台功能后，会自动保存身份、显示右上角成功提示并关闭窗口");
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function finishAuthRecovery(session: BrowserAuthSession) {
  authRecoveryBusy.value = session.id;
  try {
    const updated = await api.finishBrowserAuthSession(session.id);
    await reloadAuthRecoverySessions();
    emit(
      "notify",
      updated.status === "valid" ? "success" : "info",
      updated.status === "valid" ? "登录会话已更新，绿灯恢复" : updated.lastError || "会话仍需确认",
    );
    if (updated.status === "valid") void maybeAutoContinueAfterAuthRecovery();
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function validateAuthRecovery(session: BrowserAuthSession) {
  authRecoveryBusy.value = session.id;
  try {
    const updated = await api.validateBrowserAuthSession(session.id);
    await reloadAuthRecoverySessions();
    emit(
      "notify",
      updated.status === "valid" ? "success" : "info",
      updated.status === "valid" ? "会话校验通过，可以继续原任务" : updated.lastError || "会话需要重新登录",
    );
    if (updated.status === "valid") void maybeAutoContinueAfterAuthRecovery();
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function maybeAutoContinueAfterAuthRecovery() {
  if (!authRecoveryScan.value || authRecoveryAutoContinuing.value || authRecoveryBusy.value === "continue") return;
  if (!authRecoverySessions.value.length || authRecoverySessions.value.some((session) => session.status !== "valid")) return;
  authRecoveryAutoContinuing.value = true;
  emit("notify", "success", "登录成功，绑定身份已保存；恢复窗口已关闭，正在自动继续原任务");
  await continueAfterAuthRecovery();
}
async function continueAfterAuthRecovery() {
  const scan = authRecoveryScan.value;
  if (!scan) return;
  authRecoveryBusy.value = "continue";
  scanControlBusy.value = scan.id;
  try {
    await reloadAuthRecoverySessions();
    if (authRecoverySessions.value.some((session) => session.status !== "valid")) {
      emit("notify", "info", "仍有绑定身份未恢复绿色状态，请先完成登录");
      authRecoveryAutoContinuing.value = false;
      return;
    }
    await executeRescan(scan);
    authRecoveryScan.value = undefined;
    authRecoverySessions.value = [];
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
    scanControlBusy.value = "";
    authRecoveryAutoContinuing.value = false;
  }
}
function askRemove(scan: SentinelScan) {
  if (deleting.value) return;
  pendingDelete.value = scan;
}
function cancelRemove() {
  if (!deleting.value) pendingDelete.value = undefined;
}
async function remove() {
  const scan = pendingDelete.value;
  if (!scan || deleting.value) return;
  deleting.value = true;
  try {
    await api.deleteSentinelScan(scan.id);
    if (selected.value?.id === scan.id) {
      selected.value = undefined;
      findings.value = [];
      selectedUrl.value = "";
    }
    if (previewScan.value?.id === scan.id) previewScan.value = undefined;
    pendingDelete.value = undefined;
    emit("notify", "success", "任务及关联记录已删除；历史源文件与任务产物文件已保留");
    await load();
  } catch (e) {
    emit("notify", "error", `删除失败：${String(e)}`);
  } finally {
    deleting.value = false;
  }
}
async function exportProject() {
  if (!transferProjectId.value) {
    emit(
      "notify",
      "info",
      "请先在顶部选择要导出的项目，或打开该项目的一条扫描任务",
    );
    return;
  }
  try {
    emit(
      "notify",
      "success",
      `项目包已导出：${await api.exportSentinelProject(transferProjectId.value)}`,
    );
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
function editValidation(item: SentinelFinding, verdict?: string) {
  validationEditor.value = item;
  const old = validationFor(item);
  validationForm.verdict = verdict || old?.verdict || "needs_more";
  validationForm.severity = old?.severity || safeSeverity(item.severity);
  validationForm.note = old?.note || "";
  validationForm.evidence = old?.evidence || "";
}
async function saveValidation() {
  const item = validationEditor.value;
  if (!selected.value || !item) return;
  const scanId = selected.value.id;
  const project = projectFilter.value;
  const generation = detailGeneration;
  const form = { ...validationForm };
  try {
    await api.saveSentinelValidation({
      scanId,
      url: item.targetUrl,
      findingKey: findingKey(item),
      findingKind: item.kind,
      ...form,
    });
    // The write belongs to its original task even if the user navigated away.
    // Only publish the subsequent read into a still-matching view.
    const [nextValidations] = await Promise.all([
      api.listSentinelValidations(scanId),
      project === projectFilter.value && !detailDisposed ? refreshOverviewStats(project) : Promise.resolve(),
    ]);
    if (!detailDisposed && project === projectFilter.value && scanId === selected.value?.id && generation === detailGeneration) {
      validations.value = nextValidations;
      if (validationEditor.value === item) validationEditor.value = undefined;
    }
    if (tab.value === "validations" && project === projectFilter.value && !detailDisposed) {
      const nextItems = await api.listSentinelValidationWorkItems(project);
      if (!detailDisposed && project === projectFilter.value) validationWorkItems.value = nextItems;
    }
    emit(
      "notify",
      "success",
      `验证结论已保存：${verdictLabel(form.verdict)}，风险等级已更新为 ${severityLabel(form.verdict === "false_positive" ? "none" : form.severity)}`,
    );
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
function editValidationWorkItem(item: SentinelValidationWorkItem) {
  validationWorkEditor.value = item;
  Object.assign(validationWorkForm, {
    verdict: item.validationId ? item.verdict : "needs_more",
    severity: item.confirmedSeverity || safeSeverity(item.originalSeverity),
    note: item.note || "",
    evidence: item.evidence || "",
  });
}
async function saveValidationWorkItem() {
  const item = validationWorkEditor.value;
  if (!item) return;
  const project = projectFilter.value;
  const form = { ...validationWorkForm };
  try {
    await api.saveSentinelValidation({
      scanId: item.scanId,
      url: item.url,
      findingKey: item.findingKey,
      findingKind: item.findingKind,
      ...form,
    });
    const [nextItems] = await Promise.all([
      api.listSentinelValidationWorkItems(project),
      project === projectFilter.value && !detailDisposed ? refreshOverviewStats(project) : Promise.resolve(),
    ]);
    if (!detailDisposed && project === projectFilter.value) {
      validationWorkItems.value = nextItems;
      if (validationWorkEditor.value === item)
        validationWorkEditor.value = nextItems.find((row) => row.findingId === item.findingId);
    }
    emit("notify", "success", `漏洞结论已保存：${verdictLabel(form.verdict)}`);
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function openValidationEvidence(item: SentinelValidationWorkItem) {
  const scan = scans.value.find((row) => row.id === item.scanId);
  if (!scan) {
    emit("notify", "error", "对应任务不在当前项目范围内");
    return;
  }
  await openScan(scan, false);
  selectedUrl.value = item.url;
  resultTab.value = "vulnerabilities";
  tab.value = "results";
}
watch(
  () => props.section,
  (value) => {
    tab.value = value;
    if (value === "results" && !selected.value && resultTaskScans.value[0])
      openScan(resultTaskScans.value[0], false);
  },
);
watch(
  () => props.resultView,
  (value) => {
    resultTab.value = value;
    if (props.section === "results") tab.value = "results";
    if (
      value === "vulnerabilities" &&
      (!selected.value || !vulnerabilityScanIds.value.includes(selected.value.id)) &&
      resultTaskScans.value[0]
    )
      openScan(resultTaskScans.value[0], false);
  },
);
watch(() => selected.value?.id, clearResultDetails, { flush: "sync" });
watch(selectedUrl, (value) => {
  if (resultTab.value === "vulnerabilities")
    selectedFindingId.value = vulnerabilityRows.value[0]?.id;
  if (selected.value?.scanType === "web") {
    loadInvestigationGraph(selected.value.id, value);
    loadAgentExecution(selected.value.id, value);
  }
});
watch(resultTab, (value) => {
  if (value === "vulnerabilities")
    selectedFindingId.value = vulnerabilityRows.value[0]?.id;
});
watch(validationFilter, () => {
  const first = selectedValidationWorkItems.value[0];
  if (
    first &&
    !selectedValidationWorkItems.value.some(
      (item) => item.findingId === validationWorkEditor.value?.findingId,
    )
  )
    editValidationWorkItem(first);
});
watch(
  () => props.projectId,
  async (value) => {
    if (projectFilter.value === value) return;
    projectFilter.value = value;
    selected.value = undefined;
    previewScan.value = undefined;
    selectedUrl.value = "";
    await load();
  },
);
watch(tab, async (value) => {
  emit("section-change", value);
  try {
    if (value === "queue" && !previewScan.value && queueScans.value[0])
      await preview(queueScans.value[0]);
    if (value === "fuse" || value === "queue") fuseEntries.value = await api.listSentinelFuseZone(projectFilter.value);
    if (value === "validations") {
      validationWorkItems.value = await api.listSentinelValidationWorkItems(projectFilter.value);
      const first = selectedValidationWorkItems.value[0];
      if (first && !validationWorkEditor.value) editValidationWorkItem(first);
    }
  } catch (error) {
    emit("notify", "error", String(error));
  }
});
watch(
  () => [stats.value.pendingVulnerabilityCount, stats.value.activeFuseCount] as const,
  ([vulnerabilities, fuse]) => emit("alerts-change", { fuse, vulnerabilities }),
  { immediate: true },
);
watch(
  () => props.active,
  async (active) => {
    if (!active) return;
    // The board stays mounted while the user works in Asset. New drafts created
    // there are already in SQLite, but the in-memory scan list is stale. Reload
    // the lightweight database views whenever the Agent pane becomes visible.
    await load();
  },
);
useBoardLifecycle({
  subscribe: () => listen<BrowserAuthSession>("browser-auth-session-updated", async (event) => {
    if (detailDisposed) return;
    const session = event.payload;
    if (!session) return;
    const recoverySessions = authRecoverySessions.value;
    const belongsToRecovery = recoverySessions.some((item) => item.id === session.id);
    if (!belongsToRecovery) return;
    authRecoverySessions.value = recoverySessions.map((item) => item.id === session.id ? session : item);
    if (session.status === "valid") {
      stopAuthCapturePolling(session.id);
      authRecoveryBusy.value = "";
      await reloadAuthRecoverySessions();
      if (detailDisposed) return;
      emit("notify", "success", `登录成功，${session.name} 已保存身份；登录窗口已关闭`);
      void maybeAutoContinueAfterAuthRecovery();
    }
  }),
  load,
  restoreSearch: async () => { if (props.search.trim()) await applySearch(props.search); },
  refresh: liveSync,
  onSubscriptionError: (message) => emit("notify", "error", message),
});
onUnmounted(() => {
  detailDisposed = true;
  clearResultDetails();
  for (const sessionId of authCaptureTimers.keys()) stopAuthCapturePolling(sessionId);
});
</script>

<template>
<div class="sentinel-page sentinel-v2">
    <div v-if="loading" class="agent-load-state">
      <span class="loader-ring"></span>
      <div>
        <strong>{{
          tr("正在加载本地任务数据", "Loading local task data")
        }}</strong
        ><small>{{
          tr(
            "正在读取当前任务、资产和结果。",
            "Reading current tasks, assets, and results.",
          )
        }}</small>
      </div>
    </div>
    <SentinelAuthRecoveryPanel
      v-if="authRecoveryScan"
      :scan="authRecoveryScan"
      :sessions="authRecoverySessions"
      :busy="authRecoveryBusy"
      @reopen="reopenAuthRecovery"
      @finish="finishAuthRecovery"
      @validate="validateAuthRecovery"
      @continue="continueAfterAuthRecovery"
      @close="closeAuthRecovery"
    />
    <template v-if="tab === 'dialog'">
      <AgentDialog :project-id="projectId" :initial-scan-id="dialogScanId" @start="startInvestigation" @open-results="(scan) => openScan(scan)" @prepare-followup="prepareGapFollowup" />
    </template>
    <template v-if="tab === 'overview'">
      <section class="panel investigation-hero">
        <div class="investigation-hero-copy">
          <span class="eyebrow">AUTONOMOUS INVESTIGATION DESK</span>
          <h2>先给结论与证据，再决定是否消耗 Token</h2>
          <p>
            页面渲染、功能入口触发、HTTP 捕获、参数还原、JS/指纹与本地知识匹配由确定性流程完成；
            只有形成高价值候选后才把有限上下文交给原生调查。
          </p>
        </div>
        <div class="investigation-hero-actions">
          <button class="button primary" @click="tab = 'workbench'">
            <Play :size="15" /> 新建扫描
          </button>
          <button class="button ghost" @click="tab = 'queue'">
            <Activity :size="15" /> 任务与成本
          </button>
        </div>
      </section>
      <div class="investigation-layout">
        <section class="panel opportunity-inbox">
          <div class="panel-heading opportunity-heading">
            <div>
              <span class="eyebrow">OPPORTUNITY INBOX</span>
              <h3>高价值机会收件箱</h3>
              <p>每张卡片都必须说明价值、证据来源、接口参数和下一步；它不是漏洞结论。</p>
            </div>
            <div class="segmented opportunity-filter">
              <button :class="{ active: opportunityView === 'ready' }" @click="opportunityView = 'ready'">
                <span>可验证</span><b v-if="stats.readyOpportunityCount" class="opportunity-count-badge">{{ stats.readyOpportunityCount }}</b>
              </button>
              <button :class="{ active: opportunityView === 'all' }" @click="opportunityView = 'all'">
                <span>活跃</span><b v-if="stats.opportunityCount" class="opportunity-count-badge">{{ stats.opportunityCount }}</b>
              </button>
              <button :class="{ active: opportunityView === 'history' }" @click="opportunityView = 'history'">
                历史
              </button>
            </div>
          </div>
          <div class="opportunity-list">
            <article
              v-for="item in visibleOpportunities.slice(0, 24)"
              :key="item.id"
              class="opportunity-card"
              :class="[`score-${item.score >= 80 ? 'high' : item.score >= 65 ? 'medium' : 'low'}`, item.status]"
            >
              <div class="opportunity-score">
                <strong>{{ item.score }}</strong><small>价值分</small>
              </div>
              <div class="opportunity-content">
                <header>
                  <span>{{ opportunityCategoryLabel(item.category) }}</span>
                  <em :class="`opportunity-status ${item.status}`">{{ opportunityStatusLabel(item.status) }}</em>
                  <b v-if="opportunityEvidenceCount(item)" class="opportunity-evidence-badge">证据 {{ opportunityEvidenceCount(item) }}</b>
                  <small>{{ item.confidence || 'unknown' }} confidence</small>
                </header>
                <h4>{{ item.title }}</h4>
                <code class="opportunity-endpoint">{{ opportunityEndpoint(item) }}</code>
                <div v-if="opportunityIdentityRows(item).length" class="opportunity-identity-scope">
                  <strong>身份范围</strong><span>{{ opportunityIdentitySummary(item) }}</span><span class="identity-compare-label">{{ opportunityIdentityRows(item).length > 1 ? "同一机会 · A/B 分栏" : "单账号证据" }}</span>
                  <template v-for="row in opportunityIdentityRows(item)" :key="`${item.id}-${row.label}`">
                    <em :class="`identity-chip ${row.tone}`" :title="row.identityKey || row.detail">{{ row.label }} · {{ row.state }}<small>{{ row.detail }}</small></em>
                  </template>
                </div>
                <small class="opportunity-validation-summary">{{ validationSummary(item) }}</small>
                <ul>
                  <li v-for="reason in item.why.slice(0, 3)" :key="reason">{{ reason }}</li>
                </ul>
                <div v-if="opportunityParameters(item).length" class="opportunity-params">
                  <span>参数</span><code v-for="parameter in opportunityParameters(item).slice(0, 12)" :key="parameter">{{ parameter }}</code>
                </div>
                <div v-if="opportunityKnowledge(item).length" class="opportunity-knowledge">
                  <Fingerprint :size="14" /> 已命中 {{ opportunityKnowledge(item).length }} 条本地知识：
                  {{ opportunityKnowledgeTitles(item) }}
                </div>
                <footer>
                  <span>{{ item.recommendedAction?.label || '查看完整证据后决定下一步' }}</span>
                  <div>
                    <button class="button primary small" :disabled="opportunityBusy === item.id" @click="openOpportunity(item, true)">
                      开始验证
                    </button>
                    <button class="button ghost small" @click="openOpportunity(item)">证据</button>
                    <button
                      v-if="activeOpportunityStatuses.has(item.status)"
                      class="button ghost small"
                      :disabled="opportunityBusy === item.id"
                      @click="setOpportunityStatus(item, 'dismissed')"
                    >忽略</button>
                  </div>
                </footer>
              </div>
            </article>
            <div v-if="!visibleOpportunities.length" class="empty-state opportunity-empty">
              <template v-if="opportunityView === 'ready' && activeOpportunityClues.length">
                <strong>当前没有达到可直接验证门禁的候选</strong>
                <span>仍保留 {{ activeOpportunityClues.length }} 条有效线索，没有把它们误标成可验证结果。</span>
                <div class="opportunity-clue-preview">
                  <button v-for="item in activeOpportunityClues" :key="item.id" @click="openOpportunity(item)">
                    <b>{{ item.score }}</b><span>{{ item.title }}</span><em>查看证据</em>
                  </button>
                </div>
                <button class="button ghost compact" @click="opportunityView = 'all'">查看全部活跃线索</button>
              </template>
              <template v-else>
                <strong>当前没有匹配的机会卡</strong>
                <span>新任务会先完成浏览器探索与前端侦察；没有 70 分以上候选时自动进入一次性目录/API 兜底发现。</span>
              </template>
            </div>
          </div>
        </section>
        <SentinelOverviewSidebar
          :ready-opportunity-count="stats.readyOpportunityCount"
          :overview-loading="overviewLoading" :overview-error="overviewError"
          :running-scan-count="runningScanCount" :total-request-usage="totalRequestUsage"
          :total-token-usage="totalTokenUsage" :token-scope-label="tokenScopeLabel"
          :investigation-stats="investigationStats"
          :investigation-state="investigationSummaryState"
        />
      </div>
      <SentinelOverviewSummary
        :stats="stats" :overview-loading="overviewLoading" :overview-error="overviewError"
        :scans="scans" :running-scan-count="runningScanCount"
        :total-token-usage="totalTokenUsage" :total-request-usage="totalRequestUsage"
        :token-scope-label="tokenScopeLabel"
      />
      <SentinelTokenOverview v-model:scope="tokenScope" :usage="tokenUsage" @open="openScan" />
      <SentinelTaskOverview
        :scans="visibleScans" :project-id="props.projectId" :busy-scan-id="scanControlBusy"
        @open="openScan" @pause="pauseScan" @resume="resumeScan" @retry="rescan" @remove="askRemove"
      />
    </template>

    <template v-else-if="tab === 'queue'">
    <SentinelTaskCenter
      :scans="queueScans"
      :project-id="projectFilter"
      :has-more="scanHasMore"
      :loading-more="scanLoadingMore"
      :preview="previewScan"
      :preview-targets="previewTargetRows"
      :attention-count="attentionTaskCount"
      :total-tokens="totalTokenUsage"
      :total-requests="totalRequestUsage"
      :token-scope-label="tokenScopeLabel"
      :zero-yield-count="zeroYieldScans.length"
      :zero-yield-tokens="zeroYieldTokenUsage"
      :cache-hit-rate="cacheHitRate"
      :control-busy="scanControlBusy"
      :high-value-count="scanHighValueCount"
      @preview="preview"
      @close="previewScan = undefined"
      @confirm="confirm"
      @pause="pauseScan"
      @resume="resumeScan"
      @retry="rescan"
      @remove="askRemove"
      @open="openScan"
      @dialog="openScanDialog"
      @load-more="loadMoreScanHistory"
      @archived="onTaskArchived"
    />
      <section class="panel fuse-in-tasks">
        <header><h3>{{ tr("被熔断的地址", "Fused targets") }}</h3><p>{{ tr("这些地址不会进入新调查，除非从这里移出。", "These targets stay out of new investigations until removed here.") }}</p></header>
        <SentinelFuseZone
          v-model:fuse-filter="fuseFilter"
          v-model:fuse-category-filter="fuseCategoryFilter"
          v-model:fuse-editor="fuseEditor"
          v-model:pending-fuse-removal="pendingFuseRemoval"
          :fuse-entries="fuseEntries"
          :visible-fuse-entries="visibleFuseEntries"
          :fuse-busy="fuseBusy"
          :fuse-form="fuseForm"
          :fuse-detail-tabs="fuseDetailTabs"
          :copy-text="copyText"
          :edit-fuse="editFuse"
          :fuse-category-label="fuseCategoryLabel"
          :fuse-reason-category="fuseReasonCategory"
          :fuse-reason-parts="fuseReasonParts"
          :fuse-recommended-action="fuseRecommendedAction"
          :fuse-rows="fuseRows"
          :fuse-state="fuseState"
          :fuse-target="fuseTarget"
          :fuse-validation-rows="fuseValidationRows"
          :remove-fuse="removeFuse"
          :same-target-url="sameTargetUrl"
          :save-fuse="saveFuse"
          :toggle-fuse-detail="toggleFuseDetail"
        />
      </section>
    </template>

    <template v-else-if="tab === 'results'">
      <div class="sentinel-result-shell">
        <main class="panel url-intelligence">
          <div class="result-context-bar">
            <label class="result-context-control">
              <span class="eyebrow">TASKS</span>
              <select :value="selected?.id || ''" @change="selectResultTask">
                <option value="" disabled>选择扫描任务</option>
                <option v-for="scan in resultTaskScans" :key="scan.id" :value="scan.id">
                  {{ scan.projectName || "未命名" }} · {{ statusLabel(scan.status) }} · {{ scan.id }}
                </option>
              </select>
            </label>
            <label v-if="selected?.scanType === 'web'" class="result-context-control result-context-target">
              <span class="eyebrow">COMPANY ASSETS</span>
              <select :value="selectedUrl" @change="selectResultUrl">
                <option v-for="card in filteredUrlCards" :key="card.url" :value="card.url">
                  {{ card.url === "*" ? "全部目标 / 全局发现" : card.url }} · {{ card.total }} 条 · {{ card.vulnerabilities }} 漏洞
                </option>
              </select>
            </label>
            <div v-else class="result-source-context">
              <span class="eyebrow">SOURCE CONTEXT</span>
              <strong>{{ selected?.sourcePath || "未提供源码路径" }}</strong>
              <small>{{ sourceInventory.architecture || "等待识别" }} · {{ sourceLanguages.join("、") || "等待识别" }}</small>
            </div>
            <div class="result-context-summary">
              <span>{{ selected ? statusLabel(selected.status) : "未选择任务" }}</span>
              <span v-if="selected?.scanType === 'web'">{{ filteredUrlCards.length }} 个目标</span>
              <span v-else>{{ sourceStats.findings }} 条发现</span>
            </div>
          </div>
          <div v-if="!selected" class="empty-state">请先选择扫描任务。</div>
          <template v-else
            ><template v-if="selected.scanType !== 'web'"
              ><SentinelSourceResults
                :selected="selected"
                :detail-busy="detailBusy"
                :scan-control-busy="scanControlBusy"
                :scan-attempts="scanAttempts"
                :visible-scan-attempts="visibleScanAttempts"
                :show-attempt-history="showAttemptHistory"
                :result-tab="resultTab"
                :selected-finding-id="selectedFindingId"
                :is-greybox-scan="isGreyboxScan"
                :is-cicd-scan="isCicdScan"
                :appsec-result="appsecResult"
                :attempt-mode-label="attemptModeLabel"
                :appsec-vulnerabilities="appsecVulnerabilities"
                :appsec-source-counts="appsecSourceCounts"
                :cicd-blocking-findings="cicdBlockingFindings"
                :greybox-correlated="greyboxCorrelated"
                :focused-source-finding-rows="focusedSourceFindingRows"
                :source-finding-rows="sourceFindingRows"
                :source-dependencies="sourceDependencies"
                :source-frameworks="sourceFrameworks"
                :source-inventory="sourceInventory"
                :source-inventory-finding="sourceInventoryFinding"
                :source-issue-groups="sourceIssueGroups"
                :source-language-rows="sourceLanguageRows"
                :source-manifests="sourceManifests"
                :source-severity-counts="sourceSeverityCounts"
                :source-stats="sourceStats"
                :appsec-sources-for="appsecSourcesFor"
                :auth-type-label="authTypeLabel"
                :ci-provider-label="ciProviderLabel"
                :correlation-parts="correlationParts"
                :edit-validation="editValidation"
                :effective-severity="effectiveSeverity"
                :export-project="exportProject"
                :gate-status-label="gateStatusLabel"
                :pause-scan="pauseScan"
                :rescan="rescan"
                :resume-scan="resumeScan"
                :source-locations="sourceLocations"
                :source-type-label="sourceTypeLabel"
                :validation-for="validationFor"
                @toggle-attempt-history="showAttemptHistory = !showAttemptHistory"
                @select-finding="selectedFindingId = $event"
              /></template
            ><template v-else
              ><header class="url-intelligence-head">
                <div>
                  <span class="eyebrow"
                    >{{ companyForUrl(selectedUrl) }} · 调查工作台</span
                  >
                  <h3>
                    {{
                      selectedUrl === "*"
                        ? "全部目标 / 全局发现"
                        : selectedUrl || "未选择 URL"
                    }}
                  </h3>
                  <p>
                    {{ selected.projectName }} ·
                    {{ statusLabel(selected.status) }} · {{ selected.id }}
                  </p>
                  <small
                    v-if="scanSummary(selected)"
                    class="live-checkpoint"
                    ><Activity :size="12" />
                    {{ scanSummary(selected) }}</small
                  >
                </div>
                <div class="hero-actions">
                  <button
                    v-if="
                      selected.status === 'scanning' ||
                      selected.status === 'pausing'
                    "
                    class="button warning compact"
                    :disabled="scanControlBusy === selected.id"
                    @click="pauseScan(selected)"
                  >
                    <Pause :size="14" />{{
                      selected.status === "pausing"
                        ? "正在停止"
                        : "停止并保留"
                    }}</button
                  ><button
                    v-else-if="selected.status === 'paused' || selected.status === 'partial'"
                    class="button secondary compact"
                    :disabled="scanControlBusy === selected.id"
                    @click="resumeScan(selected)"
                  >
                    <Play :size="14" />{{ selected.status === 'partial' ? '继续未完成' : '继续扫描' }}</button
                  ><button
                    v-else-if="
                      selected.status !== 'draft' &&
                      selected.status !== 'pausing' &&
                      !selected.administrativeClosureRecorded
                    "
                    class="button ghost compact"
                    :disabled="scanControlBusy === selected.id"
                    @click="rescan(selected)"
                  >
                    <RefreshCw :size="14" />{{ retryActionLabel(selected) }}</button
                  ><button class="button ghost compact" @click="exportProject">
                    <Download :size="14" />导出整个项目
                  </button>
                </div>
              </header>
              <NativeRunStatus v-if="selected" class="agent-workspace" :scan-id="selected.id" :attempt="selected.attemptCount" :status="selected.status" @attempt-closed="onAttemptClosed" @prepare-handoff="prepareClosureHandoff" />
              <details class="result-drawer">
                <summary>覆盖、接口和漏洞结果</summary>
              <section v-if="webCoverageCatalog.length" class="web-coverage-ledger">
                <header>
                  <div>
                    <strong>SRC 能力基线与扩展覆盖</strong>
                    <span>{{ webEffectivePolicy.effectiveSkillNames || "业务前端深度分析" }} · 任务模式：{{ routeModeLabel(selected?.requestedScanMode || webEffectivePolicy.webModeCeiling || "standard") }}<template v-if="currentTarget?.scanMode"> · 当前目标：{{ routeModeLabel(currentTarget.scanMode) }}</template></span>
                  </div>
                  <small>能力就绪不等于当前目标已经测试；真实执行结果以调查图谱和证据为准</small>
                </header>
                <div>
                  <article
                    v-for="item in webCoverageCatalog"
                    :key="item.key"
                    :class="`coverage-${item.status}`"
                  >
                    <span>{{ item.label }}</span>
                    <strong>{{ coverageStatusLabel(item.status) }}</strong>
                    <small><template v-if="item.standard">{{ item.standard }} · </template>{{ item.prerequisite }}</small>
                    <details v-if="item.manualFocus"><summary>人工补位重点</summary><p>{{ item.manualFocus }}</p></details>
                  </article>
                </div>
              </section>
              <div v-if="selectedUrl" class="url-metric-strip">
                <span v-if="currentTarget?.scanMode" class="route"
                  ><Activity :size="14" /><b>{{ currentTarget.valueScore }}</b>
                  {{ routeModeLabel(currentTarget.scanMode) }}</span
                ><button
                  type="button"
                  title="查看指纹与配置"
                  @click="jumpToResult('fingerprint')"
                  ><Fingerprint :size="14" /><b>{{
                    currentCard?.fingerprints || 0
                  }}</b>
                  指纹</button
                ><button
                  type="button"
                  title="查看 JS、路由与 API"
                  @click="jumpToResult('api')"
                  ><Code2 :size="14" /><b>{{ currentCard?.apis || 0 }}</b>
                  API/JS</button
                ><button
                  type="button"
                  title="查看端点验证"
                  @click="jumpToResult('endpoints')"
                  ><Network :size="14" /><b>{{
                    currentCard?.endpoints || 0
                  }}</b>
                  端点</button
                ><button
                  type="button"
                  class="risk"
                  title="查看漏洞与人工验证"
                  @click="jumpToResult('vulnerabilities')"
                  ><Bug :size="14" /><b>{{
                    currentCard?.vulnerabilities || 0
                  }}</b>
                  漏洞</button
                ><span class="url-token-metric"
                  ><Activity :size="14" /><b>{{
                    formatNumber(selected.inputTokens)
                  }}</b>
                  输入</span
                ><span class="url-token-metric"
                  ><b>{{ formatNumber(selected.outputTokens) }}</b> 输出</span
                ><span class="url-token-metric"
                  ><RefreshCw :size="14" /><b>{{ selected.attemptCount || 0 }}</b>
                  次执行</span
                >
              </div>
              <button
                v-if="selectedUrl"
                type="button"
                :class="`evidence-next-action ${evidenceNextAction.tone}`"
                @click="followEvidenceNextAction"
              >
                <span>建议下一步</span><strong>{{ evidenceNextAction.label }}</strong><em>打开处理</em>
              </button>
              <nav class="result-subtabs">
                <button
                  v-for="item in [
                    { key: 'summary', label: '概要' },
                    { key: 'investigation', label: '调查图谱' },
                    { key: 'opportunities', label: '机会与下一步' },
                    { key: 'fingerprint', label: '指纹与配置' },
                    { key: 'api', label: 'JS / API' },
                    { key: 'endpoints', label: '端点验证' },
                    { key: 'vulnerabilities', label: '漏洞与验证' },
                  ]"
                  :key="item.key"
                  :class="{ active: resultTab === item.key }"
                  @click="resultTab = item.key as ResultTab"
                >
                  {{ item.label
                  }}<em v-if="item.key === 'investigation'" :title="`信息增益 ${investigationGraph?.metrics?.informationGain || 0}/100`">图 {{
                    investigationGraph?.nodes?.length || 0
                  }}</em><em v-if="item.key === 'opportunities'">{{
                    selectedUrlOpportunities.length
                  }}</em><em v-if="item.key === 'api' && currentCard?.sensitive" class="sensitive-tab-count">{{
                    currentCard.sensitive
                  }}</em><em v-if="item.key === 'vulnerabilities'">{{
                    vulnerabilityRows.length
                  }}</em>
                </button>
              </nav>
              <div v-if="detailBusy" class="empty-state">正在解析结果…</div>
              <div v-else-if="!selectedUrl" class="empty-state">
                该任务没有 URL 结果。可先在任务中心查看待扫 URL。
              </div>

              <div
                v-else-if="resultTab === 'summary'"
                class="result-section-stack"
              >
                <section
                  v-if="selectedUrl !== '*' && selectedUrl"
                  class="result-block url-action-block"
                >
                  <div class="block-title">
                    <Globe2 :size="16" />
                    <div>
                      <button
                        class="url-title-link"
                        type="button"
                        @click="openTargetUrl(selectedUrl)"
                      >
                        {{ selectedUrl }} <ExternalLink :size="14" /></button
                      ><small
                        >{{ companyForUrl(selectedUrl) }} · 历史扫描
                        {{ currentTarget?.scanCount || 0 }} 次</small
                      >
                    </div>
                    <button
                      class="button ghost compact"
                      type="button"
                      @click="openTargetUrl(selectedUrl)"
                    >
                      <ExternalLink :size="14" />在浏览器打开
                    </button>
                  </div>
                </section>
                <section
                  v-if="currentTarget?.scanMode"
                  class="result-block adaptive-route-block"
                >
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>自适应扫描决策</strong
                      ><small
                        >{{ currentTarget.scanMode === 'manual_review'
                          ? '复杂前端只保留高价值线索，等待人工复核。'
                          : '本地前置分析决定候选价值，自动调查只验证高价值证据。' }}</small
                      >
                    </div>
                    <span class="route-mode" :class="currentTarget.scanMode">{{
                      routeModeLabel(currentTarget.scanMode)
                    }}</span>
                  </div>
                  <div class="adaptive-route-summary">
                    <article>
                      <span>前端价值</span
                      ><strong
                        >{{ currentTarget.valueScore
                        }}<small>/ 100</small></strong
                      >
                    </article>
                    <article>
                      <span>任务模式</span
                      ><strong>{{ routeModeLabel(selected?.requestedScanMode || webEffectivePolicy.webModeCeiling || "standard") }}</strong>
                    </article>
                    <article>
                      <span>实际分流</span
                      ><strong>{{ routeModeLabel(currentTarget.scanMode) }}</strong>
                    </article>
                    <article>
                      <span>状态来源</span>
                      <strong v-if="currentTarget.lastAttemptNumber === selected.attemptCount">第 {{ currentTarget.lastAttemptNumber }} 次：{{ statusLabel(currentTarget.status) }}</strong>
                      <strong v-else>历史第 {{ currentTarget.lastAttemptNumber || '—' }} 次：{{ statusLabel(currentTarget.status) }}<small>（本轮未执行）</small></strong>
                    </article>
                  </div>
                  <div class="route-reason-list">
                    <span v-for="reason in routeReasonItems" :key="reason">{{
                      reason
                    }}</span
                    ><span v-if="!routeReasonItems.length">暂无分流依据</span>
                  </div>
                </section>
                <section
                  v-if="agentCoverageFinding"
                  class="result-block agent-coverage-block"
                >
                  <div class="block-title">
                    <ShieldCheck :size="16" />
                    <div>
                      <strong>覆盖与完整性</strong>
                      <small>区分“已测试且未发现”“不适用”和“尚需跟进”；这些覆盖记录不会计入漏洞数量。</small>
                    </div>
                    <span
                      :class="[
                        'coverage-completeness',
                        { complete: agentCoverage.completeness?.complete },
                      ]"
                    >{{
                      agentCoverage.completeness?.complete
                        ? "覆盖记录完整"
                        : "存在覆盖缺口"
                    }}</span>
                  </div>
                  <div class="agent-coverage-summary">
                    <article>
                      <span>已复核攻击面</span>
                      <strong>{{ agentCoverage.summary?.surfaces_reviewed || 0 }}</strong>
                    </article>
                    <article>
                      <span>形成发现</span>
                      <strong>{{ agentCoverage.summary?.findings_filed || 0 }}</strong>
                    </article>
                    <article>
                      <span>待补覆盖</span>
                      <strong>{{ agentCoverage.summary?.gaps || 0 }}</strong>
                    </article>
                    <article>
                      <span>执行 Agent</span>
                      <strong>{{ agentCoverage.machine_observed?.agents?.length || 0 }}</strong>
                    </article>
                  </div>
                  <div
                    v-if="agentCoverage.completeness?.caveats?.length"
                    class="coverage-caveats"
                  >
                    <b>完整性说明</b>
                    <span
                      v-for="caveat in agentCoverage.completeness.caveats"
                      :key="String(caveat)"
                    >{{ caveat }}</span>
                  </div>
                  <div v-if="agentCoverageGaps.length" class="coverage-gap-list">
                    <article
                      v-for="(gap, index) in agentCoverageGaps.slice(0, 8)"
                      :key="`${gap.kind || 'gap'}-${gap.risk_area || gap.riskArea || index}`"
                    >
                      <strong>{{ gap.risk_area || gap.riskArea || gap.kind || "未命名覆盖缺口" }}</strong>
                      <span>{{ gap.surface || gap.detail || gap.reason || "缺少足够执行证据" }}</span>
                      <em>{{ gap.kind || "follow_up" }}</em>
                    </article>
                  </div>
                  <SentinelExecutionDetails
                    v-if="agentExecution"
                    :execution="agentExecution"
                    :scope="agentExecutionScope"
                    :scan-id="selected?.id"
                    :target-url="selectedUrl"
                  />
                  <details v-if="agentCoverageEntries.length" class="coverage-entry-details">
                    <summary>查看 {{ agentCoverageEntries.length }} 条覆盖结论</summary>
                    <div>
                      <article
                        v-for="(entry, index) in agentCoverageEntries"
                        :key="`${entry.risk_area || 'coverage'}-${entry.surface || index}`"
                      >
                        <span>{{ entry.risk_area || "未标注风险域" }}</span>
                        <strong>{{ entry.surface || "未标注攻击面" }}</strong>
                        <em :class="`outcome-${entry.outcome || 'unknown'}`">{{
                          agentCoverageOutcomeLabel(entry.outcome)
                        }}</em>
                        <p>{{ entry.evidence || "未提供证据摘要" }}</p>
                      </article>
                    </div>
                  </details>
                </section>
                <SentinelTraceTimeline
                  :detail="liveTrace" :busy="liveTraceBusy"
                  :target-url="selectedUrl" :task-status="selected.status"
                />
                <section class="result-block">
                  <div class="block-title">
                    <Fingerprint :size="16" />
                    <div>
                      <strong>技术指纹</strong
                      ><small
                        >已将识别结果拆分为名称、版本、置信度和依据，不再显示对象
                        JSON。</small
                      >
                    </div>
                  </div>
                  <div
                    v-if="Object.keys(fingerprint).length"
                    class="fingerprint-grid"
                  >
                    <article v-for="card in fingerprintCards" :key="card.key">
                      <span>{{ card.label }}</span
                      ><strong>{{ displayName(card.data) }}</strong
                      ><small>{{ displayVersion(card.data) }}</small
                      ><em>{{ card.data?.confidence || "未知置信度" }}</em>
                      <p v-if="card.data?.evidence?.length">
                        依据：{{ card.data.evidence.join("、") }}
                      </p>
                    </article>
                  </div>
                  <div v-else class="empty-inline">未解析到技术指纹</div>
                </section>
                <section v-if="selected.previousScanId" class="result-block">
                  <div class="block-title">
                    <RefreshCw :size="16" />
                    <div>
                      <strong>与上一次扫描对比</strong
                      ><small
                        >基于
                        URL、结果类型和稳定记录键比较；内容变化单独计数。</small
                      >
                    </div>
                  </div>
                  <div class="comparison-grid">
                    <article>
                      <span>新增</span><strong>{{ comparison.added }}</strong>
                    </article>
                    <article>
                      <span>消失</span><strong>{{ comparison.removed }}</strong>
                    </article>
                    <article>
                      <span>发生变化</span
                      ><strong>{{ comparison.changed }}</strong>
                    </article>
                    <article>
                      <span>保持不变</span
                      ><strong>{{ comparison.unchanged }}</strong>
                    </article>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>Token 消耗</strong
                      ><small
                        >{{ selected.totalTokens ? `${scanTypeLabel(selected.scanType)} · ${selected.skillNames || "默认扫描策略"}` : "当前模型请求 0、Token 0；前端/CDP 确定性侦察不会消耗模型 Token" }}</small
                      >
                    </div>
                  </div>
                  <div class="comparison-grid token-grid">
                    <article>
                      <span>请求次数</span
                      ><strong>{{ formatNumber(selected.llmRequests) }}</strong>
                    </article>
                    <article>
                      <span>输入 Token</span
                      ><strong>{{ formatNumber(selected.inputTokens) }}</strong>
                    </article>
                    <article>
                      <span>输出 Token</span
                      ><strong>{{
                        formatNumber(selected.outputTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>缓存 Token</span
                      ><strong>{{
                        formatNumber(selected.cachedTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>新增输入 Token</span
                      ><strong>{{
                        formatNumber(uncachedInput(selected))
                      }}</strong>
                    </article>
                    <article>
                      <span>输入 + 输出总计</span
                      ><strong>{{ formatNumber(scanTokenTotal(selected)) }}</strong>
                    </article>
                  </div>
                </section>
                <section v-if="runtimeDiagnosticsFindings.length" class="result-block runtime-diagnostics-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>runtimeDiagnostics</strong>
                      <small>本地浏览器采集失败时保留错误码；不再只显示一句摘要。</small>
                    </div>
                  </div>
                  <div class="runtime-diagnostics-list">
                    <article v-for="finding in runtimeDiagnosticsFindings" :key="finding.id">
                      <header><strong>{{ finding.title }}</strong><em> · {{ kindLabel(finding.kind) }}</em></header>
                      <div v-for="row in runtimeDiagnosticRows(finding)" :key="row.key" class="runtime-diagnostic-row">
                        <div class="runtime-diagnostic-head"><b>{{ row.identity }}</b><span> · </span><code>{{ row.captureStatus }}</code></div>
                        <dl>
                          <div v-if="row.captureError"><dt>captureError</dt><dd><code>{{ row.captureError }}</code></dd></div>
                          <div v-if="row.stopReason"><dt>stopReason</dt><dd><code>{{ row.stopReason }}</code></dd></div>
                          <div v-if="row.failedStage"><dt>failedStage</dt><dd><code>{{ row.failedStage }}</code></dd></div>
                          <div v-if="row.transport"><dt>cdpTransport</dt><dd><code>{{ row.transport }}</code></dd></div>
                          <div v-if="row.browser"><dt>browser</dt><dd><code>{{ row.browser }}</code></dd></div>
                          <div v-if="row.exitCode"><dt>exitCode</dt><dd><code>{{ row.exitCode }}</code></dd></div>
                          <div v-if="row.signal"><dt>signal</dt><dd><code>{{ row.signal }}</code></dd></div>
                        </dl>
                        <pre v-if="row.stderr" class="runtime-diagnostic-stderr">{{ row.stderr }}</pre>
                      </div>
                    </article>
                  </div>
                </section>
                <section v-if="scanAttempts.length" class="result-block attempt-ledger-block">
                  <div class="block-title">
                    <RefreshCw :size="16" />
                    <div><strong>本次执行结果与增量成本</strong><small>默认只显示最新一次最终状态；重新执行与继续未完成阶段按不同规则隔离。</small></div>
                    <button v-if="scanAttempts.length > 1" class="attempt-history-toggle" @click="showAttemptHistory = !showAttemptHistory">{{showAttemptHistory ? '收起历史' : `查看历史 ${scanAttempts.length - 1} 次`}}</button>
                  </div>
                  <div class="attempt-ledger">
                    <article v-for="attempt in visibleScanAttempts" :key="attempt.attemptNumber" :class="[`attempt-${attempt.status}`, { current: attempt.attemptNumber === selected.attemptCount }]">
                      <header><span>第 {{attempt.attemptNumber}} 次 · {{attemptModeLabel(attempt.executionMode, attempt.attemptNumber)}}</span><b>{{attemptStageLabel(attempt.stage)}}</b><em class="status-chip" :class="attempt.status">{{statusLabel(attempt.status)}}</em></header>
                      <ol class="execution-stage-strip" aria-label="执行六段状态">
                        <li v-for="step in EXECUTION_STAGE_STEPS" :key="`${attempt.attemptNumber}-${step.key}`" :class="executionStageTone(attempt, step.key)">{{ step.label }}</li>
                      </ol>
                      <small v-if="executionStageStatusLabel(attempt)" class="execution-stage-note">{{ executionStageStatusLabel(attempt) }}</small>
                      <div class="attempt-backend-grid">
                        <span>执行后端 <b>{{ attemptBackendSummary(attempt).backend }}</b></span>
                        <span>执行环境 <b>{{ attemptBackendSummary(attempt).environment }}</b></span>
                        <span>任务状态 <b>{{ statusLabel(attemptBackendSummary(attempt).taskStatus) }}</b></span>
                      </div>
                      <p>{{attempt.checkpoint || (resolveExecutionStage(attempt).historyMissing ? '历史记录未提供' : '尚无阶段详情')}}</p>
                      <div class="attempt-cost"><span>请求 <b>{{formatNumber(attempt.llmRequests)}}</b></span><span>输入 <b>{{formatNumber(attempt.inputTokens)}}</b></span><span>缓存 <b>{{formatNumber(attempt.cachedTokens)}}</b></span><span>输出 <b>{{formatNumber(attempt.outputTokens)}}</b></span><span>本次总计 <b>{{formatNumber(attempt.totalTokens)}}</b></span></div>
                      <small>{{attemptTime(attempt)}}</small><code v-if="attempt.workDir" :title="attempt.workDir">{{attempt.workDir}}</code><mark v-if="attemptEndReason(attempt)">结束说明：{{attemptEndReason(attempt)}}</mark>
                      <button class="attempt-history-toggle" @click="emit('open-runner-log', selected.id, attempt.attemptNumber)">运行日志与诊断</button>
                    </article>
                  </div>
                </section>
                <section v-if="evidenceChainRows.length" class="result-block evidence-chain-block">
                  <div class="block-title">
                    <Layers3 :size="16" />
                    <div><strong>接口证据链</strong><small>把前端解析、运行时请求、端点验证、机会评分与漏洞结果合并到同一行；不再需要在多个页签之间手工拼接。</small></div>
                  </div>
                  <div class="evidence-chain-table">
                    <div class="table-head"><span>方法</span><span>接口</span><span>来源 / 参数</span><span>验证</span><span>价值</span><span>漏洞</span></div>
                    <div v-for="row in evidenceChainRows" :key="row.key">
                      <b class="method-badge" :class="methodTone(row.method)">{{row.method}}</b>
                      <code :title="row.url">{{row.url}}</code>
                      <span><em>{{row.sources.join(' · ')}}</em><small v-if="row.parameters.length">参数：{{row.parameters.join('、')}}</small><small v-else>尚未还原参数</small></span>
                      <span><b v-if="row.verified" class="evidence-verified">HTTP {{row.statusCode || '已响应'}}</b><i v-else>仅候选</i></span>
                      <strong :class="{ valuable: row.opportunityScore >= 65 }">{{row.opportunityScore || '—'}}</strong>
                      <button :class="{ risk: row.vulnerabilities }" @click="resultTab='vulnerabilities'">{{row.vulnerabilities}}</button>
                    </div>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Shield :size="16" />
                    <div>
                      <strong>风险概况</strong
                      ><small
                        >统计采用人工验证后的等级；误报不再计入风险。</small
                      >
                    </div>
                  </div>
                  <div class="risk-overview">
                    <article>
                      <span>严重 / 高危</span
                      ><strong class="severity-critical">{{
                        vulnerabilityRows.filter((v) =>
                          ["critical", "high"].includes(effectiveSeverity(v)),
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>中危</span
                      ><strong class="severity-medium">{{
                        vulnerabilityRows.filter(
                          (v) => effectiveSeverity(v) === "medium",
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>低危 / 信息</span
                      ><strong>{{
                        vulnerabilityRows.filter((v) =>
                          ["low", "info"].includes(effectiveSeverity(v)),
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>误报 / 无风险</span
                      ><strong>{{
                        vulnerabilityRows.filter(
                          (v) => effectiveSeverity(v) === "none",
                        ).length
                      }}</strong>
                    </article>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>关键信息</strong
                      ><small>按 URL 汇总，已应用人工确认后的风险等级。</small>
                    </div>
                  </div>
                  <div class="key-finding-list">
                    <button
                      v-for="item in vulnerabilityRows.slice(0, 5)"
                      :key="item.id"
                      @click="resultTab = 'vulnerabilities'"
                    >
                      <span
                        :class="`severity-dot ${effectiveSeverity(item)}`"
                      ></span
                      ><strong>{{
                        item.title || json(item.recordJson).title
                      }}</strong
                      ><em>{{ severityLabel(effectiveSeverity(item)) }}</em>
                    </button>
                    <div
                      v-if="!vulnerabilityRows.length"
                      class="empty-inline"
                      :class="{ warning: agentUnclosedGaps > 0 }"
                    >
                      <template v-if="agentUnclosedGaps > 0">
                        验证尚未收口：还有 {{ agentUnclosedGaps }} 个覆盖族未完成，这不等于没有漏洞
                      </template>
                      <template v-else>当前 URL 已完成覆盖账本，且未发现漏洞</template>
                    </div>
                  </div>
                </section>
              </div>

              <div
                v-else-if="resultTab === 'investigation'"
                class="result-section-stack"
              >
                <InvestigationGraphPanel
                  :graph="investigationGraph"
                  :busy="investigationBusy"
                  :updating-id="investigationUpdatingId"
                  @status="updateInvestigationStatus"
                  @replay="openRepeaterForGraphApi"
                  @replay-hypothesis="openRepeaterForHypothesis"
                />
              </div>

              <SentinelOpportunitiesPane
                v-else-if="resultTab === 'opportunities'"
                :selected-url-opportunities="selectedUrlOpportunities"
                :opportunity-category-label="opportunityCategoryLabel"
                :opportunity-status-label="opportunityStatusLabel"
                :opportunity-endpoint="opportunityEndpoint"
                :opportunity-parameters="opportunityParameters"
                :opportunity-knowledge="opportunityKnowledge"
                :opportunity-knowledge-titles="opportunityKnowledgeTitles"
                :opportunity-identity-rows="opportunityIdentityRows"
                :opportunity-identity-summary="opportunityIdentitySummary"
                :open-opportunity="openOpportunity"
                :set-opportunity-status="setOpportunityStatus"
              />

              <SentinelFingerprintPane
                v-else-if="resultTab === 'fingerprint'"
                :fingerprint-cards="fingerprintCards"
                :security-headers="securityHeaders"
                :tech-stack="techStack"
                :wordpress="wordpress"
                :rows="rows"
              />

              <SentinelApiPane
                v-else-if="resultTab === 'api'"
                :api-rows="apiRows"
                :crypto-rows="cryptoRows"
                :declared-request-header-rows="declaredRequestHeaderRows"
                :observed-request-header-rows="observedRequestHeaderRows"
                :possible-request-header-rows="possibleRequestHeaderRows"
                :expanded-api-rows="expandedApiRows"
                :expanded-sensitive="expandedSensitive"
                :js-rows="jsRows"
                :observed-mutation-rows="observedMutationRows"
                :realtime-endpoint-rows="realtimeEndpointRows"
                :registration-rows="registrationRows"
                :request-header-intelligence="requestHeaderIntelligence"
                :route-rows="routeRows"
                :runtime-action-rows="runtimeActionRows"
                :runtime-feature-rows="runtimeFeatureRows"
                :runtime-rows="runtimeRows"
                :sensitive-rows="sensitiveRows"
                :api-url="apiUrl"
                :api-path="apiPath"
                :api-query="apiQuery"
                :api-record="apiRecord"
                :api-method="apiMethod"
                :api-response-summary="apiResponseSummary"
                :api-source-summary="apiSourceSummary"
                :api-description="apiDescription"
                :api-request-payload="apiRequestPayload"
                :api-response-headers="apiResponseHeaders"
                :api-identity-summary="apiIdentitySummary"
                :registration-data="registrationData"
                :runtime-signal-url="runtimeSignalUrl"
                :header-display-value="headerDisplayValue"
                :copy-text="copyText"
                :open-target-url="openTargetUrl"
                :toggle-api-row="toggleApiRow"
                :toggle-sensitive="toggleSensitive"
              />

              <SentinelEndpointsPane
                v-else-if="resultTab === 'endpoints'"
                :endpoint-rows="endpointRows"
                :selected-url="selectedUrl"
              />

              <SentinelVulnerabilitiesPane
                v-else
                :vulnerability-rows="vulnerabilityRows"
                :focused-vulnerability-rows="focusedVulnerabilityRows"
                :poc-rows="pocRows"
                :selected-finding-id="selectedFindingId"
                :selected-url="selectedUrl"
                :validation-editor="validationEditor"
                :validation-form="validationForm"
                :edit-validation="editValidation"
                :save-validation="saveValidation"
                :validation-for="validationFor"
                :effective-severity="effectiveSeverity"
                :vulnerability-update-history="vulnerabilityUpdateHistory"
                @select-finding="selectedFindingId = $event"
                @close-validation="validationEditor = undefined"
              />
              </details>
            </template></template
          >
        </main>
      </div>
    </template>

    <template v-else-if="tab === 'fuse'">
      <SentinelFuseZone
        v-model:fuse-filter="fuseFilter"
        v-model:fuse-category-filter="fuseCategoryFilter"
        v-model:fuse-editor="fuseEditor"
        v-model:pending-fuse-removal="pendingFuseRemoval"
        :fuse-entries="fuseEntries"
        :visible-fuse-entries="visibleFuseEntries"
        :fuse-busy="fuseBusy"
        :fuse-form="fuseForm"
        :fuse-detail-tabs="fuseDetailTabs"
        :copy-text="copyText"
        :edit-fuse="editFuse"
        :fuse-category-label="fuseCategoryLabel"
        :fuse-reason-category="fuseReasonCategory"
        :fuse-reason-parts="fuseReasonParts"
        :fuse-recommended-action="fuseRecommendedAction"
        :fuse-rows="fuseRows"
        :fuse-state="fuseState"
        :fuse-target="fuseTarget"
        :fuse-validation-rows="fuseValidationRows"
        :remove-fuse="removeFuse"
        :same-target-url="sameTargetUrl"
        :save-fuse="saveFuse"
        :toggle-fuse-detail="toggleFuseDetail"
      />
    </template>

    <template v-else-if="tab === 'workbench'">
      <AgentTraceHub
        v-if="(workbenchIntent || props.workbenchMode) === 'traces'"
        @notify="(type, text) => emit('notify', type, text)"
      />
      <AgentWorkbench
        v-else
        :projects="props.projects"
        :project-id="workbenchHandoff?.projectId || workbenchFollowup?.projectId || props.projectId"
        :followup="workbenchFollowup"
        :handoff="workbenchHandoff"
        :scans="scans"
        :initial-mode="(workbenchIntent || props.workbenchMode) as 'web' | 'code' | 'greybox' | 'cicd' | 'skills'"
        @notify="(type, text) => emit('notify', type, text)"
        @reload="load"
        @create-project="emit('create-project')"
        @open-scan="(scan) => openScan(scan)"
        @prepare-scan="prepareWorkbenchScan"
      />
    </template>

    <SentinelValidationWorkbench
      v-else-if="tab === 'validations'"
      :filter="validationFilter"
      :stats="validationWorkStats"
      :items="selectedValidationWorkItems"
      :editor="validationWorkEditor"
      :form="validationWorkForm"
      @update:filter="validationFilter = $event"
      @select="editValidationWorkItem"
      @evidence="openValidationEvidence"
      @save="saveValidationWorkItem"
    />

    <template v-else
      ><section class="panel sentinel-guide">
        <div class="guide-icon"><HelpCircle :size="23" /></div>
        <div>
          <h3>安全分析工作台</h3>
          <p>
            资产 URL 使用 Web 扫描；工作台提供代码审计、URL + 源码灰盒联测和
            CI/CD
            变更范围检查。所有结果统一进入任务、URL/源码目标、漏洞和人工验证页面。
          </p>
        </div>
      </section></template
    >
    <SentinelRepeater
      v-if="repeaterOpportunity"
      :scan-id="repeaterOpportunity.scanId"
      :target-url="repeaterOpportunity.targetUrl"
      :opportunity="repeaterOpportunity"
      :api="repeaterApi"
      :hypothesis="repeaterHypothesis"
      @close="repeaterOpportunity = undefined"
      @saved="handleRepeaterSaved"
    />
    <div
      v-if="pendingDelete"
      class="sentinel-confirm-backdrop"
      @click.self="cancelRemove"
    >
      <InlineConfirm
        class="sentinel-delete-confirm-modal"
        :title="
          `确认删除任务「${scanTitle(pendingDelete)}」？`
        "
        :detail="
          ['scanning', 'pausing'].includes(pendingDelete.status)
            ? '当前任务仍在执行或等待退出，后端将拒绝删除。请先暂停并确认执行退出、请求已核清；删除不会强杀进程或清理磁盘文件。'
            : '这是整任务记录删除：删除关联 URL、解析结果、对话和人工验证记录；如有未知请求或未结算执行，将保留任务并拒绝删除。历史源文件与任务产物文件保留，不释放磁盘空间。仅需整理列表时，请使用归档。'
        "
        :busy="deleting"
        @cancel="cancelRemove"
        @confirm="remove"
      />
    </div>
  </div>
</template>

<style src="../../../sentinel.css">
.runtime-diagnostics-list { display:grid; gap:10px; }
.runtime-diagnostics-list>article { padding:12px; border:1px solid var(--border); border-radius:12px; background:var(--surface); }
.runtime-diagnostics-list>article>header { display:flex; justify-content:space-between; gap:8px; align-items:center; margin-bottom:8px; }
.runtime-diagnostics-list>article>header em { color:var(--muted); font-style:normal; font-size:11px; }
.runtime-diagnostic-row { display:grid; gap:6px; padding:10px; border:1px dashed color-mix(in srgb, var(--border) 80%, #c9d6e6); border-radius:10px; background:var(--panel); }
.runtime-diagnostic-row + .runtime-diagnostic-row { margin-top:8px; }
.runtime-diagnostic-head { display:flex; justify-content:flex-start; gap:8px; align-items:center; }
.runtime-diagnostic-head code { padding:2px 7px; border-radius:999px; background:color-mix(in srgb, var(--warning) 18%, transparent); color:var(--text); font-size:10px; }
.runtime-diagnostic-row dl { display:grid; gap:4px; margin:0; }
.runtime-diagnostic-row dl>div { display:grid; grid-template-columns:110px minmax(0,1fr); gap:8px; }
.runtime-diagnostic-row dt { margin:0; color:var(--muted); font-size:10px; }
.runtime-diagnostic-row dd { margin:0; min-width:0; }
.runtime-diagnostic-row dd code { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:11px; }
.runtime-diagnostic-stderr { margin:0; max-height:110px; overflow:auto; padding:8px; border-radius:8px; background:#091019; color:#b9c8da; font-size:10px; white-space:pre-wrap; word-break:break-word; }

.execution-stage-strip { display:grid; grid-template-columns:repeat(6,minmax(0,1fr)); gap:4px; margin:8px 0 6px; padding:0; list-style:none; }
.execution-stage-strip li { min-width:0; padding:5px 4px; border-radius:8px; border:1px solid var(--border); background:var(--panel); color:var(--muted); font-size:10px; text-align:center; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.execution-stage-strip li.done { border-color:color-mix(in srgb,#3d9a6a 45%,var(--border)); background:color-mix(in srgb,#3d9a6a 12%,transparent); color:#2f6b4d; }
.execution-stage-strip li.current { border-color:color-mix(in srgb,var(--accent) 55%,var(--border)); background:color-mix(in srgb,var(--accent) 14%,transparent); color:var(--text); font-weight:700; }
.execution-stage-strip li.idle,.execution-stage-strip li.pending { opacity:0.72; }
.execution-stage-strip li.missing { opacity:0.5; }
.execution-stage-note { display:block; margin:0 0 6px; color:var(--muted); font-size:11px; }
@media (max-width:900px) { .execution-stage-strip { grid-template-columns:repeat(3,minmax(0,1fr)); } }

.attempt-backend-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:6px;margin:0 0 8px}
.attempt-backend-grid span{min-width:0;padding:6px 8px;border:1px solid var(--border);border-radius:8px;background:var(--panel);color:var(--muted);font-size:10px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.attempt-backend-grid b{color:var(--text);font-weight:700}
@media (max-width:900px){.attempt-backend-grid{grid-template-columns:1fr}}
</style>
