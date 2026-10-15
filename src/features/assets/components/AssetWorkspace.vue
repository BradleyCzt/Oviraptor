<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import {
  Archive,
  ArrowUpDown,
  Check,
  ChevronLeft,
  ChevronRight,
  Columns3,
  Download,
  Filter,
  Layers,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  ShieldQuestion,
  ShieldBan,
  X,
} from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { assetApi } from "../api";
import { sentinelApi } from "../../sentinel/api";
import type { Asset, AssetQuery, AssetSelection, AssetSummary, FilterCondition } from "../types";
import type { Project } from "../../workspaces/types";
import { useI18n } from "../../../i18n";
import TitleRulePopover from "./TitleRulePopover.vue";
import PageIsolationDialog from "./PageIsolationDialog.vue";
import "../../../asset-workspace-enhancements.css";
import "../../../asset-workspace-patch.css";

const props = defineProps<{
  projects: Project[];
  selectedProjectId?: number;
  initialSearch?: string;
  quarantineOnly?: boolean;
  scanningEnabled?: boolean;
}>();
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  reprobe: [projectId: number];
  changed: [];
}>();
const { tr } = useI18n();

const allColumns = [
  ["projectName", "所属项目", "Project"],
  ["assetKey", "资产键", "Asset key"],
  ["company", "公司名称", "Company"],
  ["reviewTier", "复核等级", "Review tier"],
  ["host", "访问入口", "Host"],
  ["link", "原始链接", "Raw link"],
  ["ip", "IP", "IP"],
  ["port", "端口", "Port"],
  ["protocol", "协议", "Protocol"],
  ["domain", "域名", "Domain"],
  ["title", "标题", "Title"],
  ["statusCode", "状态码", "Status"],
  ["probeOutcome", "探测结果", "Probe result"],
  ["hasLogin", "登录口", "Login portal"],
  ["hasRegister", "注册口", "Register portal"],
  ["entrySeen", "是否看过", "Seen"],
  ["entryOpenedAt", "访问入口打开时间", "Entry opened at"],
  ["probeEntryState", "入口状态", "Entry state"],
  ["contentCategory", "内容分类", "Content"],
  ["score", "评分", "Score"],
  ["decision", "人工结论", "Decision"],
  ["note", "结论备注", "Decision note"],
  ["ownershipStatus", "归属状态", "Ownership"],
  ["ownershipConfidence", "归属置信度", "Ownership confidence"],
  ["authorizationStatus", "授权状态", "Authorization"],
  ["exposureEligible", "暴露面准入", "Exposure eligible"],
  ["ownershipReason", "归属依据", "Ownership reason"],
  ["sentinelStatus", "调查状态 / 是否发送扫描", "Investigation state / Sent for scanning"],
  ["sentinelScanCount", "扫描次数", "Scan count"],
  ["sentinelSentAt", "最近送扫", "Last sent"],
  ["firstSeen", "全局首次发现", "Global first seen"],
  ["lastSeen", "全局最后发现", "Global last seen"],
  ["lastAlive", "最后存活", "Last alive"],
  ["projectFirstSeen", "项目首次发现", "Project first seen"],
  ["projectLastSeen", "项目最后发现", "Project last seen"],
  ["lastRunId", "最近任务 ID", "Last run ID"],
  ["isDeleted", "回收状态", "Trash state"],
  ["deletedAt", "移入回收站时间", "Deleted at"],
] as const;

const probeOutcomeOptions = [
  ["web_alive", "Web 可访问", "Web accessible"],
  ["web_restricted", "Web 受限", "Web restricted"],
  ["browser_render_required", "需浏览器渲染", "Browser render required"],
  ["virtual_host_required", "需正确域名", "Virtual host required"],
  ["web_abnormal", "Web 异常", "Web abnormal"],
  ["tcp_alive_non_http", "TCP 非 Web", "TCP non-Web"],
  ["blocked_content", "内容隔离", "Blocked content"],
  ["unreachable", "无法连接", "Unreachable"],
  ["skipped", "已跳过", "Skipped"],
  ["alive_clean", "旧版存活·待复测", "Legacy alive · re-probe"],
] as const;
const decisionOptions = [
  ["pending", "未审核", "Not reviewed"],
  ["uncertain", "待补证据", "Needs evidence"],
  ["confirmed", "已确认有效", "Confirmed valid"],
  ["rejected", "已排除", "Rejected"],
  ["not_applicable", "不适用 Web", "Not applicable"],
] as const;
const reviewTierOptions = [
  ["P1", "P1 高优先级", "P1 high"],
  ["P2", "P2 中优先级", "P2 medium"],
  ["P3", "P3 低优先级", "P3 low"],
] as const;
const sentinelStatusOptions = [
  ["not_sent", "未发送", "Not sent"],
  ["draft", "待确认", "Draft"],
  ["queued", "已排队", "Queued"],
  ["scanning", "扫描中", "Scanning"],
  ["partial", "部分完成", "Partial"],
  ["completed", "已完成", "Completed"],
  ["failed", "失败", "Failed"],
] as const;
const portalPresenceOptions = [
  ["1", "有", "Present"],
  ["0", "无", "Absent"],
  ["-1", "未识别", "Not screened"],
] as const;
const entrySeenOptions = [
  ["1", "已看", "Seen"],
  ["0", "未看", "Not seen"],
] as const;
const selectFieldOptions: Record<string, readonly (readonly [string, string, string])[]> = {
  probeOutcome: probeOutcomeOptions,
  decision: decisionOptions,
  reviewTier: reviewTierOptions,
  sentinelStatus: sentinelStatusOptions,
  isDeleted: [["0", "正常", "Active"], ["1", "回收站", "Trash"]],
  hasLogin: portalPresenceOptions,
  hasRegister: portalPresenceOptions,
  entrySeen: entrySeenOptions,
};
const numericFields = new Set(["port", "statusCode", "score", "sentinelScanCount", "lastRunId", "ownershipConfidence"]);
const dateFields = new Set(["firstSeen", "lastSeen", "lastAlive", "projectFirstSeen", "projectLastSeen", "sentinelSentAt", "deletedAt", "entryOpenedAt"]);
const sortableFields = new Set(["company", "host", "title", "statusCode", "score", "decision", "ownershipStatus", "ownershipConfidence", "exposureEligible", "probeOutcome", "hasLogin", "hasRegister", "entrySeen", "firstSeen", "projectLastSeen", "lastAlive"]);
const defaultColumns = ["projectName", "company", "reviewTier", "host", "ip", "port", "statusCode", "probeOutcome", "hasLogin", "hasRegister", "entrySeen", "title", "decision", "sentinelStatus", "projectLastSeen"];
const defaultColumnWidths: Record<string, number> = {
  projectName: 150, assetKey: 230, company: 170, reviewTier: 90, host: 230, link: 260,
  ip: 140, port: 76, protocol: 92, domain: 190, title: 280, statusCode: 86,
  probeOutcome: 145, hasLogin: 88, hasRegister: 88, entrySeen: 92, entryOpenedAt: 168, probeEntryState: 165, contentCategory: 120, score: 80,
  decision: 120, note: 260, sentinelStatus: 160, sentinelScanCount: 92,
  ownershipStatus: 110, ownershipConfidence: 95, authorizationStatus: 100,
  exposureEligible: 105, ownershipReason: 300,
  sentinelSentAt: 155, firstSeen: 155, lastSeen: 155, lastAlive: 155,
  projectFirstSeen: 155, projectLastSeen: 155, lastRunId: 105, isDeleted: 90, deletedAt: 155,
};
const validColumnKeys = new Set<string>(allColumns.map((item) => item[0]));
const savedColumns = JSON.parse(localStorage.getItem("asset-columns") || "[]") as string[];
const savedVisible = savedColumns.filter((item) => validColumnKeys.has(item));
function withPortalColumns(columns: string[]) {
  if (localStorage.getItem("asset-portal-columns-v1")) return columns;
  const next = [...columns];
  const at = next.indexOf("probeOutcome");
  const insertAt = at >= 0 ? at + 1 : next.length;
  for (const key of ["entrySeen", "hasRegister", "hasLogin"]) {
    if (!next.includes(key)) next.splice(insertAt, 0, key);
  }
  localStorage.setItem("asset-portal-columns-v1", "1");
  return next;
}
const visibleColumns = ref<string[]>(savedVisible.length ? withPortalColumns(savedVisible) : [...defaultColumns]);
const columnWidths = ref<Record<string, number>>({ ...defaultColumnWidths, ...JSON.parse(localStorage.getItem("asset-column-widths") || "{}") });
const showColumns = ref(false);
const showFilters = ref(false);
const loading = ref(false);
const bulkBusy = ref(false);
const savedAssetScanMode = localStorage.getItem("asset-agent-scan-mode");
const assetScanMode = ref<"quick" | "standard" | "deep">(
  savedAssetScanMode === "quick" || savedAssetScanMode === "deep" ? savedAssetScanMode : "standard",
);
const assets = shallowRef<Asset[]>([]);
const total = ref(0);
const emptySummary = (): AssetSummary => ({ all: 0, pending: 0, uncertain: 0, confirmed: 0, rejected: 0, notApplicable: 0, sentToAgent: 0 });
const memberTotal = ref(0);
const summary = ref<AssetSummary>(emptySummary());
const selected = ref<Map<string, Asset>>(new Map());
const titleRuleSelection = ref<{ text: string; x: number; y: number; asset: Asset }>();
const ruleBusy = ref(false);
const pageRuleOpen = ref(false);
const pageRuleAsset = ref<Asset>();
const pageRuleKeyword = ref<string>();
function openPageRules(asset?: Asset, keyword?: string) {
  pageRuleAsset.value = asset; pageRuleKeyword.value = keyword; pageRuleOpen.value = true;
}
async function pageRulesChanged() { clearSelection(); await refresh(); emit('changed'); }
function defaultQuery(projectId = props.selectedProjectId): AssetQuery { return {
  projectId,
  search: props.initialSearch ?? "",
  conditions: [],
  page: 1,
  pageSize: 50,
  includeDeleted: false,
  deletedView: props.quarantineOnly ? "all" : "active",
  probeView: props.quarantineOnly ? "blocked" : "browser_review",
  probeOutcomeView: "all",
  sentinelView: "all",
  decisionView: props.quarantineOnly ? "all" : "review",
  ownershipView: "all",
  hasLoginView: "all",
  hasRegisterView: "all",
  entrySeenView: "all",
  sortBy: "priority",
  sortDirection: "desc",
  collapseIdenticalResponses: !props.quarantineOnly,
  responseFingerprint: "",
}; }
const stateKey = (projectId?: number) => `asset-query-v2:${props.quarantineOnly ? 'quarantine' : 'assets'}:${projectId ?? 'all'}`;
function restoredQuery(projectId?: number): AssetQuery {
  const defaults = defaultQuery(projectId);
  try {
    const saved = JSON.parse(sessionStorage.getItem(stateKey(projectId)) || 'null');
    if (!saved || !Array.isArray(saved.conditions) || !Number.isInteger(saved.page) || saved.page < 1 || ![50,100,200].includes(saved.pageSize)) return defaults;
    return { ...defaults, ...saved, search: props.initialSearch || saved.search || '', ...(props.quarantineOnly ? {probeView:'blocked'} : {}) };
  } catch { return defaults; }
}
const query = ref<AssetQuery>(restoredQuery(props.selectedProjectId));
watch(query, value => { try { sessionStorage.setItem(stateKey(props.selectedProjectId), JSON.stringify(value)); } catch { /* A storage quota failure must not interrupt review. */ } }, { deep:true });

const selectionKey = (asset: Asset) => `${asset.projectId}:${asset.id}`;
const selectedRows = computed(() => [...selected.value.values()]);
const selectedProjectCount = computed(() => new Set(selectedRows.value.map((asset) => asset.projectId)).size);
const assetScanModeLabel = computed(() => ({ quick: tr("快速扫描", "Quick scan"), standard: tr("标准扫描", "Standard scan"), deep: tr("深度扫描", "Deep scan") })[assetScanMode.value]);
const queryProjectArchived = computed(() => props.projects.find((project) => project.id === query.value.projectId)?.status === "archived");
const pages = computed(() => Math.max(1, Math.ceil(total.value / query.value.pageSize)));
const selectedAll = computed({
  get: () => assets.value.length > 0 && assets.value.every((asset) => selected.value.has(selectionKey(asset))),
  set: (checked: boolean) => {
    const next = new Map(selected.value);
    for (const asset of assets.value) {
      const key = selectionKey(asset);
      if (checked) next.set(key, asset);
      else next.delete(key);
    }
    selected.value = next;
  },
});

watch(() => props.selectedProjectId, (projectId) => {
  query.value = restoredQuery(projectId);
  clearSelection();
  void refresh();
});
watch(visibleColumns, (value) => localStorage.setItem("asset-columns", JSON.stringify(value)), { deep: true });

let queryGeneration = 0;
async function refresh() {
  cancelRowDrag();
  const generation = ++queryGeneration;
  const requested = JSON.parse(JSON.stringify(query.value)) as AssetQuery;
  loading.value = true;
  try {
    const page = await assetApi.listAssets(requested);
    if (generation !== queryGeneration) return;
    assets.value = page.items;
    total.value = page.total;
    memberTotal.value = page.memberTotal ?? page.total;
    summary.value = page.summary || emptySummary();
    if (!page.items.length && query.value.page > 1) {
      query.value.page = Math.max(1, Math.ceil(page.total / query.value.pageSize));
      const corrected = await assetApi.listAssets({ ...requested, page:query.value.page });
      if (generation !== queryGeneration) return;
      assets.value = corrected.items;
      total.value = corrected.total;
      memberTotal.value = corrected.memberTotal ?? corrected.total;
      summary.value = corrected.summary || emptySummary();
    }
  } catch (error) {
    if (generation === queryGeneration) emit("notify", "error", String(error));
  } finally {
    if (generation === queryGeneration) loading.value = false;
  }
}
function search(eventOrClear: Event | boolean = true) {
  const clear = typeof eventOrClear === "boolean" ? eventOrClear : true;
  query.value.page = 1;
  if (clear) clearSelection();
  void refresh();
}
function clearSelection() { selected.value = new Map(); }
function responseGroupCount(asset: Asset) {
  return Number(asset.extra?.response_display_group_count) || 1;
}
function showResponseMembers(asset: Asset) {
  const fingerprint = asset.extra?.probe_response_fingerprint;
  if (typeof fingerprint !== "string" || !/^[a-f0-9]{64}$/.test(fingerprint)) return;
  query.value.projectId = asset.projectId;
  query.value.responseFingerprint = fingerprint;
  query.value.collapseIdenticalResponses = false;
  search();
}
function toggleAsset(asset: Asset, checked: boolean) {
  const next = new Map(selected.value);
  if (checked) next.set(selectionKey(asset), asset);
  else next.delete(selectionKey(asset));
  selected.value = next;
}
function onAssetCheckbox(asset: Asset, event: Event) {
  toggleAsset(asset, (event.target as HTMLInputElement).checked);
}
const rowDragging = ref(false);
let rowDrag: { pointerId:number; start:number; x:number; y:number; add:boolean; before:Map<string,Asset> } | undefined;
function rowRange(end:number) {
  if (!rowDrag) return;
  const next=new Map(rowDrag.before);
  for (const asset of assets.value.slice(Math.min(rowDrag.start,end),Math.max(rowDrag.start,end)+1)) {
    if (rowDrag.add) next.set(selectionKey(asset),asset);
    else next.delete(selectionKey(asset));
  }
  selected.value=next;
}
function startRowDrag(event:PointerEvent, index:number) {
  if(event.button!==0 || event.pointerType==='touch' || loading.value || bulkBusy.value) return;
  // Title text remains selectable for rules; links, inputs and actions retain their gestures.
  if((event.target as Element).closest('button,a,input,select,textarea,.title-rule-source')) return;
  rowDrag={pointerId:event.pointerId,start:index,x:event.clientX,y:event.clientY,add:!selected.value.has(selectionKey(assets.value[index])),before:new Map(selected.value)};
  window.addEventListener('pointermove',moveRowDrag);
  window.addEventListener('pointerup',finishRowDrag);
  window.addEventListener('pointercancel',cancelRowDrag);
  window.addEventListener('keydown',escapeRowDrag);
}
function moveRowDrag(event:PointerEvent) {
  if(!rowDrag || event.pointerId!==rowDrag.pointerId)return;
  if(!rowDragging.value && Math.hypot(event.clientX-rowDrag.x,event.clientY-rowDrag.y)<6)return;
  rowDragging.value=true; window.getSelection()?.removeAllRanges(); event.preventDefault();
  const row=document.elementFromPoint(event.clientX,event.clientY)?.closest('tr[data-asset-row]');
  if(row && row.closest('.asset-workspace')===workspace.value) rowRange(Number(row.getAttribute('data-asset-row')));
}
function clearDragListeners() {
  window.removeEventListener('pointermove',moveRowDrag);window.removeEventListener('pointerup',finishRowDrag);
  window.removeEventListener('pointercancel',cancelRowDrag);window.removeEventListener('keydown',escapeRowDrag);
}
function finishRowDrag(event:PointerEvent) {
  if(!rowDrag || event.pointerId!==rowDrag.pointerId)return;
  rowDrag=undefined; rowDragging.value=false; clearDragListeners();
}
function cancelRowDrag() {
  if(rowDrag) selected.value=new Map(rowDrag.before);
  rowDrag=undefined;rowDragging.value=false;clearDragListeners();
}
function escapeRowDrag(event:KeyboardEvent) { if(event.key==='Escape')cancelRowDrag(); }
const workspace=ref<HTMLElement>();
function setDecisionView(value: string) { query.value.decisionView = value; search(); }
function changeProbeOutcome() {
  const outcome = query.value.probeOutcomeView || "all";
  if (outcome === "all") return search();
  if (["web_alive", "alive_clean"].includes(outcome)) query.value.probeView = "browser_accessible";
  else if (["web_restricted", "browser_render_required", "virtual_host_required"].includes(outcome)) query.value.probeView = "restricted";
  else if (outcome === "tcp_alive_non_http") query.value.probeView = "service";
  else if (["web_abnormal", "unreachable", "skipped"].includes(outcome)) query.value.probeView = "abnormal";
  else if (outcome === "blocked_content") query.value.probeView = "blocked";
  search();
}
function resetQueryFilters() {
  query.value.responseFingerprint = "";
  query.value.collapseIdenticalResponses = !props.quarantineOnly;
  query.value.search = "";
  query.value.conditions = [];
  query.value.probeView = props.quarantineOnly ? "blocked" : "browser_review";
  query.value.probeOutcomeView = "all";
  query.value.sentinelView = "all";
  query.value.decisionView = props.quarantineOnly ? "all" : "review";
  query.value.ownershipView = "all";
  query.value.hasLoginView = "all";
  query.value.hasRegisterView = "all";
  query.value.entrySeenView = "all";
  query.value.deletedView = props.quarantineOnly ? "all" : "active";
  query.value.sortBy = "priority";
  query.value.sortDirection = "desc";
  search();
}
function addCondition() { query.value.conditions.push({ field: "company", operator: "contains", value: "", join: "and" }); }
function conditionOptions(field: string) { return selectFieldOptions[field] || []; }
function operatorsFor(field: string) {
  const empty = [["isEmpty", tr("为空", "Is empty")], ["notEmpty", tr("非空", "Is not empty")]] as const;
  if (selectFieldOptions[field]) return [["equals", tr("等于", "Equals")], ["notEquals", tr("不等于", "Not equal")], ...empty] as const;
  if (numericFields.has(field)) return [["equals", tr("等于", "Equals")], ["notEquals", tr("不等于", "Not equal")], ["gte", tr("大于等于", "Greater or equal")], ["lte", tr("小于等于", "Less or equal")], ...empty] as const;
  if (dateFields.has(field)) return [["gte", tr("不早于", "On or after")], ["lte", tr("不晚于", "On or before")], ["equals", tr("等于", "Equals")], ...empty] as const;
  return [["contains", tr("包含", "Contains")], ["notContains", tr("不包含", "Does not contain")], ["equals", tr("等于", "Equals")], ["notEquals", tr("不等于", "Not equal")], ["startsWith", tr("开头为", "Starts with")], ["endsWith", tr("结尾为", "Ends with")], ...empty] as const;
}
function changeConditionField(condition: FilterCondition) {
  condition.value = "";
  condition.operator = selectFieldOptions[condition.field] ? "equals" : dateFields.has(condition.field) ? "gte" : numericFields.has(condition.field) ? "equals" : "contains";
}
function removeCondition(index: number) { query.value.conditions.splice(index, 1); }
function toggleColumn(column: string) { visibleColumns.value = visibleColumns.value.includes(column) ? visibleColumns.value.filter((value) => value !== column) : [...visibleColumns.value, column]; }
function resetColumns() { visibleColumns.value = [...defaultColumns]; }
function saveColumnWidths() { localStorage.setItem("asset-column-widths", JSON.stringify(columnWidths.value)); }
function resetColumnWidths() { columnWidths.value = { ...defaultColumnWidths }; saveColumnWidths(); }
function resetColumnWidth(column: string) { columnWidths.value = { ...columnWidths.value, [column]: defaultColumnWidths[column] || 130 }; saveColumnWidths(); }
function columnWidth(column: string) { return columnWidths.value[column] || defaultColumnWidths[column] || 130; }
function startColumnResize(event: PointerEvent, column: string) {
  event.preventDefault(); event.stopPropagation();
  const startX = event.clientX;
  const startWidth = columnWidth(column);
  document.body.style.userSelect = "none";
  document.body.style.cursor = "col-resize";
  const move = (moveEvent: PointerEvent) => { columnWidths.value = { ...columnWidths.value, [column]: Math.max(68, Math.min(720, startWidth + moveEvent.clientX - startX)) }; };
  const stop = () => {
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", stop);
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    saveColumnWidths();
  };
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", stop, { once: true });
}
function sortColumn(column: string) {
  if (!sortableFields.has(column)) return;
  if (query.value.sortBy === column) query.value.sortDirection = query.value.sortDirection === "asc" ? "desc" : "asc";
  else { query.value.sortBy = column; query.value.sortDirection = "desc"; }
  search(false);
}
function columnLabel(column: string) { const item = allColumns.find((candidate) => candidate[0] === column); return item ? tr(item[1], item[2]) : column; }
function value(asset: Asset, column: string) {
  if (column === "entrySeen") return asset.entryOpenedAt ? "1" : "0";
  if (column === "hasLogin") return asset.hasLogin ?? -1;
  if (column === "hasRegister") return asset.hasRegister ?? -1;
  return (asset as unknown as Record<string, unknown>)[column] ?? "";
}
function decisionLabel(value: string) { const item = decisionOptions.find((option) => option[0] === (value || "pending")); return item ? tr(item[1], item[2]) : value; }
function badgeClass(column: string, raw: string) {
  if (column === "probeOutcome") return `status-${raw || "unknown"}`;
  if (column === "reviewTier") return `tier-${raw.slice(0, 2).toLowerCase()}`;
  if (column === "decision") return `decision-${raw || "pending"}`;
  if (column === "ownershipStatus") return `ownership-${raw || "unreviewed"}`;
  if (column === "exposureEligible") return raw === "true" || raw === "1" ? "decision-confirmed" : "decision-rejected";
  if (column === "contentCategory") return `content-${raw || "unknown"}`;
  if (column === "sentinelStatus") return `sentinel-${raw || "not_sent"}`;
  if (column === "isDeleted") return raw === "true" || raw === "1" ? "decision-rejected" : "decision-confirmed";
  if (column === "hasLogin" || column === "hasRegister") return raw === "1" ? "decision-confirmed" : raw === "0" ? "decision-not_applicable" : "status-unknown";
  if (column === "entrySeen") return raw === "1" ? "decision-confirmed" : "status-unknown";
  return "";
}
function probeLabel(raw: string) { const item = probeOutcomeOptions.find((option) => option[0] === raw); return item ? tr(item[1], item[2]) : raw; }
function sentinelLabel(asset: Asset) {
  if (asset.sentinelStatus === "not_sent") return tr("未发送", "Not sent");
  const item = sentinelStatusOptions.find((option) => option[0] === asset.sentinelStatus);
  const label = item ? tr(item[1], item[2]) : asset.sentinelStatus;
  return asset.sentinelScanCount > 1 ? tr(`${label} · ${asset.sentinelScanCount} 次`, `${label} · ${asset.sentinelScanCount} scans`) : label;
}
function portalLabel(flag: number) {
  if (flag === 1) return tr("有", "Present");
  if (flag === 0) return tr("无", "Absent");
  return tr("未识别", "Not screened");
}
function entryUrl(asset: Asset) {
  const url = asset.extra?.probe_effective_url || asset.link || asset.host;
  return url?.startsWith("http") ? url : "";
}
function stampEntryOpened(asset: Asset, openedAt: string) {
  const stamped = { ...asset, entryOpenedAt: openedAt };
  if (query.value.entrySeenView === "unseen") {
    assets.value = assets.value.filter((item) => !(item.projectId === asset.projectId && item.id === asset.id));
    total.value = Math.max(0, total.value - 1);
  } else {
    assets.value = assets.value.map((item) => item.projectId === asset.projectId && item.id === asset.id ? stamped : item);
  }
  if (selected.value.has(selectionKey(asset))) {
    const next = new Map(selected.value);
    next.set(selectionKey(asset), stamped);
    selected.value = next;
  }
}
function displayValue(asset: Asset, column: string) {
  if (column === "decision") return decisionLabel(asset.decision);
  if (column === "ownershipStatus") return ({ unreviewed: tr("未判断","Unreviewed"), attributed: tr("疑似归属","Attributed"), confirmed: tr("确认归属","Confirmed"), related: tr("仅关联","Related"), third_party: tr("第三方","Third party"), excluded: tr("已排除","Excluded") } as Record<string,string>)[asset.ownershipStatus] || asset.ownershipStatus;
  if (column === "authorizationStatus") return ({ allowed: tr("允许","Allowed"), denied: tr("禁止","Denied"), unknown: tr("未知","Unknown") } as Record<string,string>)[asset.authorizationStatus] || asset.authorizationStatus;
  if (column === "exposureEligible") return asset.exposureEligible ? tr("允许准入","Eligible") : tr("不准入","Not eligible");
  if (column === "probeOutcome") return probeLabel(asset.probeOutcome);
  if (column === "hasLogin") return portalLabel(Number(asset.hasLogin ?? -1));
  if (column === "hasRegister") return portalLabel(Number(asset.hasRegister ?? -1));
  if (column === "entrySeen") return asset.entryOpenedAt ? tr("已看", "Seen") : tr("未看", "Not seen");
  if (column === "isDeleted") return asset.isDeleted ? tr("回收站", "Trash") : tr("正常", "Active");
  return String(value(asset, column) || "—");
}
function selectionPayload(): AssetSelection[] { return selectedRows.value.map((asset) => ({ projectId: asset.projectId, assetId: asset.id })); }
async function act(decision: "confirmed" | "uncertain") {
  if (!selectedRows.value.length || bulkBusy.value) return;
  bulkBusy.value = true;
  const note = decision === "confirmed" ? "人工确认：保留在有效资产范围" : "人工待定：需要补充归属、可访问性或业务价值证据";
  try {
    const changed = await assetApi.updateAssetDecisions(selectionPayload(), decision, note);
    clearSelection();
    await refresh();
    emit("changed");
    emit("notify", "success", decision === "confirmed"
      ? tr(`已确认 ${changed} 条有效资产，并移出待审核队列`, `${changed} assets confirmed and removed from review queue`)
      : tr(`已将 ${changed} 条标记为待补证据，继续保留在复核队列`, `${changed} assets kept in review for more evidence`));
  } catch (error) { emit("notify", "error", String(error)); }
  finally { bulkBusy.value = false; }
}
const scanningActionError = ref("");
let scanActionGeneration=0, scanActionDisposed=false;
let scanOwnsBulkBusy=false;
const scanningAvailable=()=>props.scanningEnabled!==false;
watch(()=>[props.scanningEnabled,props.selectedProjectId],()=>{scanActionGeneration++;if(scanOwnsBulkBusy){scanOwnsBulkBusy=false;bulkBusy.value=false;}},{flush:"sync"});
onUnmounted(()=>{scanActionDisposed=true;scanActionGeneration++;});
async function sendToSentinel() {
  scanningActionError.value="";
  if(!scanningAvailable()){scanningActionError.value=tr("扫描插件已停用，请先在内置插件中启用。","Scanning is disabled. Enable it under Built-in plugins first.");return;}
  if (!selectedRows.value.length || bulkBusy.value) return;
  const action=++scanActionGeneration, contextProject=props.selectedProjectId;
  const current=()=>!scanActionDisposed&&action===scanActionGeneration&&contextProject===props.selectedProjectId&&scanningAvailable();
  bulkBusy.value = true;scanOwnsBulkBusy=true;
  try {
    const groups = new Map<number, number[]>();
    for (const asset of selectedRows.value) groups.set(asset.projectId, [...(groups.get(asset.projectId) || []), asset.id]);
    const scans = [];
    for (const [projectId, ids] of groups) {
      if(!current())return;
      scans.push(await sentinelApi.createSentinelScan(projectId, [...new Set(ids)], assetScanMode.value));
    }
    if(!current())return;
    localStorage.setItem("asset-agent-scan-mode", assetScanMode.value);
    clearSelection();
    await refresh();
    if(!current())return;
    emit("notify", "success", tr(`已按 ${scans.length} 个项目建立 ${assetScanModeLabel.value} 待确认调查任务`, `Created ${assetScanModeLabel.value} investigation drafts for ${scans.length} projects`));
  } catch (error) { if(current()){scanningActionError.value=String(error);emit("notify", "error", String(error));} }
  finally { if(!scanActionDisposed&&action===scanActionGeneration){scanOwnsBulkBusy=false;bulkBusy.value = false;} }
}
async function archive(deleted: boolean) {
  if (!selectedRows.value.length || bulkBusy.value) return;
  bulkBusy.value = true;
  try {
    const changed = await assetApi.softDeleteAssetSelections(selectionPayload(), deleted);
    clearSelection();
    await refresh();
    emit("notify", "success", deleted ? tr(`已将 ${changed} 条移入回收站`, `Moved ${changed} assets to trash`) : tr(`已恢复 ${changed} 条资产`, `Restored ${changed} assets`));
    emit("changed");
  } catch (error) { emit("notify", "error", String(error)); }
  finally { bulkBusy.value = false; }
}
async function exportCurrent() {
  try {
    const result = await assetApi.exportAssets(query.value, visibleColumns.value, true);
    emit("notify", "success", tr(`已导出 ${result.rows} 条：${result.path}`, `Exported ${result.rows} rows: ${result.path}`));
  } catch (error) { emit("notify", "error", String(error)); }
}
async function openLink(asset: Asset) {
  const url = String(value(asset, "link") || "");
  if (!url.startsWith("http")) return;
  try {
    await openUrl(url);
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function openAsset(asset: Asset) {
  const url = entryUrl(asset);
  if (!url) return;
  try {
    await openUrl(url);
  } catch (error) {
    emit("notify", "error", String(error));
    return;
  }
  try {
    const openedAt = await assetApi.markAssetEntryOpened(asset.projectId, asset.id);
    stampEntryOpened(asset, openedAt);
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
function captureTitleSelection(event: MouseEvent, asset: Asset) {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || !selection.rangeCount) return;
  const root = event.currentTarget as HTMLElement;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return;
  const text = selection.toString().replace(/\s+/g, " ").trim();
  if (text.length < 2) return;
  const rect = range.getBoundingClientRect();
  const width = 360;
  titleRuleSelection.value = { text: text.slice(0, 200), asset, x: Math.max(12, Math.min(window.innerWidth - width - 12, rect.left)), y: Math.max(12, Math.min(window.innerHeight - 190, rect.bottom + 7)) };
}
async function applyTitleRule(keyword: string) {
  if (!titleRuleSelection.value) return;
  openPageRules(titleRuleSelection.value.asset, keyword);
  titleRuleSelection.value = undefined;
}
onMounted(refresh);
onUnmounted(() => { ++queryGeneration; cancelRowDrag(); });
</script>

<template>
  <section ref="workspace" class="asset-workspace" :class="{'row-selecting':rowDragging,'has-selection':selectedRows.length}">
    <div class="asset-summary-strip" :aria-label="tr('资产队列概览','Asset queue summary')">
      <button class="asset-summary-card summary-all" :class="{ active: query.decisionView === 'all' }" @click="setDecisionView('all')">
        <span>{{tr('当前范围','Current scope')}}</span><strong>{{summary.all.toLocaleString()}}</strong><small>{{tr('不含回收站筛选之外的数据','Matches current filters')}}</small>
      </button>
      <button class="asset-summary-card summary-pending" :class="{ active: query.decisionView === 'pending' }" @click="setDecisionView('pending')">
        <span>{{tr('未审核','Not reviewed')}}</span><strong>{{summary.pending.toLocaleString()}}</strong><small>{{tr('尚未做人工结论','No manual decision')}}</small>
      </button>
      <button class="asset-summary-card summary-uncertain" :class="{ active: query.decisionView === 'uncertain' }" @click="setDecisionView('uncertain')">
        <span>{{tr('待补证据','Needs evidence')}}</span><strong>{{summary.uncertain.toLocaleString()}}</strong><small>{{tr('保留在复核队列','Kept in review')}}</small>
      </button>
      <button class="asset-summary-card summary-confirmed" :class="{ active: query.decisionView === 'confirmed' }" @click="setDecisionView('confirmed')">
        <span>{{tr('已确认有效','Confirmed valid')}}</span><strong>{{summary.confirmed.toLocaleString()}}</strong><small>{{tr('已移出待审核队列','Removed from review')}}</small>
      </button>
      <button class="asset-summary-card summary-sent" :class="{ active: query.sentinelView === 'sent' }" @click="query.sentinelView = query.sentinelView === 'sent' ? 'all' : 'sent'; search()">
        <span>{{tr('已发起调查','Investigated')}}</span><strong>{{summary.sentToAgent.toLocaleString()}}</strong><small>{{tr('至少生成过一次任务','At least one scan')}}</small>
      </button>
    </div>

    <div class="asset-toolbar panel">
      <div class="asset-toolbar-primary">
        <div class="search-box"><Search :size="17" /><input v-model="query.search" :placeholder="tr('搜索项目、公司、域名、IP、标题或资产键','Search project, company, domain, IP, title or asset key')" @keyup.enter="search" /><button v-if="query.search" @click="query.search='';search()"><X :size="14" /></button></div>
        <select v-model="query.projectId" class="toolbar-select" @change="search"><option :value="undefined">{{tr('全部项目','All projects')}}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{ p.name }}{{p.status==='archived'?tr('（已归档）',' (archived)'):''}}</option></select>
      </div>
      <div class="asset-toolbar-filters">
        <label><span>{{tr('探测队列','Probe queue')}}</span><select v-if="!quarantineOnly" v-model="query.probeView" class="toolbar-select" @change="search"><option value="browser_review">{{tr('Web 人工队列','Web review queue')}}</option><option value="browser_accessible">{{tr('浏览器可访问','Browser accessible')}}</option><option value="restricted">{{tr('受限 / 需渲染 / 需域名','Restricted / render / vhost')}}</option><option value="service">{{tr('TCP 非 Web 服务','TCP non-Web services')}}</option><option value="abnormal">{{tr('异常 / 无法连接','Abnormal / unreachable')}}</option><option value="blocked">{{tr('内容隔离','Blocked content')}}</option><option value="all">{{tr('全部分类','All classes')}}</option></select><strong v-else>{{tr('内容隔离','Blocked content')}}</strong></label>
        <label><span>{{tr('具体探测结果（二次筛选）','Exact probe result')}}</span><select v-model="query.probeOutcomeView" class="toolbar-select" @change="changeProbeOutcome"><option value="all">{{tr('全部具体结果','All exact results')}}</option><option v-for="option in probeOutcomeOptions" :key="option[0]" :value="option[0]">{{tr(option[1],option[2])}}</option></select></label>
        <label><span>{{tr('人工结论','Decision')}}</span><select v-model="query.decisionView" class="toolbar-select" @change="search"><option value="review">{{tr('待复核（未审核 + 待补证据）','Review: pending + needs evidence')}}</option><option value="pending">{{tr('仅未审核','Not reviewed only')}}</option><option value="uncertain">{{tr('仅待补证据','Needs evidence only')}}</option><option value="confirmed">{{tr('已确认有效','Confirmed valid')}}</option><option value="rejected">{{tr('已排除','Rejected')}}</option><option value="not_applicable">{{tr('不适用 Web','Not applicable')}}</option><option value="all">{{tr('全部人工结论','All decisions')}}</option></select></label>
        <label><span>{{tr('调查状态','Investigation state')}}</span><select v-model="query.sentinelView" class="toolbar-select" @change="search"><option value="all">{{tr('全部调查状态','All investigation states')}}</option><option value="sent">{{tr('已发起调查','Investigated')}}</option><option value="not_sent">{{tr('未发起调查','Not investigated')}}</option></select></label>
        <label><span>{{tr('登录口','Login portal')}}</span><select v-model="query.hasLoginView" class="toolbar-select" @change="search"><option value="all">{{tr('全部','All')}}</option><option value="yes">{{tr('有','Present')}}</option><option value="no">{{tr('无','Absent')}}</option><option value="unknown">{{tr('未识别','Not screened')}}</option></select></label>
        <label><span>{{tr('注册口','Register portal')}}</span><select v-model="query.hasRegisterView" class="toolbar-select" @change="search"><option value="all">{{tr('全部','All')}}</option><option value="yes">{{tr('有','Present')}}</option><option value="no">{{tr('无','Absent')}}</option><option value="unknown">{{tr('未识别','Not screened')}}</option></select></label>
        <label><span>{{tr('是否看过','Seen')}}</span><select v-model="query.entrySeenView" class="toolbar-select" @change="search"><option value="all">{{tr('全部','All')}}</option><option value="seen">{{tr('已看','Seen')}}</option><option value="unseen">{{tr('未看','Not seen')}}</option></select></label>
        <label><span>{{tr('数据范围','Data scope')}}</span><select v-model="query.deletedView" class="toolbar-select" @change="search"><option value="active">{{tr('正常资产','Active assets')}}</option><option value="trash">{{tr('仅回收站','Trash only')}}</option><option value="all">{{tr('正常 + 回收站','Active + trash')}}</option></select></label>
        <label><span>{{tr('排序','Sort')}}</span><span class="sort-control"><select v-model="query.sortBy" class="toolbar-select" @change="search(false)"><option value="priority">{{tr('复核优先级','Review priority')}}</option><option value="projectLastSeen">{{tr('项目最后发现','Project last seen')}}</option><option value="lastAlive">{{tr('最后存活','Last alive')}}</option><option value="score">{{tr('评分','Score')}}</option><option value="statusCode">{{tr('状态码','Status')}}</option><option value="company">{{tr('公司名称','Company')}}</option><option value="host">{{tr('访问入口','Host')}}</option><option value="title">{{tr('标题','Title')}}</option><option value="hasLogin">{{tr('登录口','Login portal')}}</option><option value="hasRegister">{{tr('注册口','Register portal')}}</option><option value="entrySeen">{{tr('是否看过','Seen')}}</option></select><button class="sort-direction" :title="query.sortDirection === 'desc' ? tr('当前降序','Descending') : tr('当前升序','Ascending')" @click="query.sortDirection=query.sortDirection==='desc'?'asc':'desc';search(false)">{{query.sortDirection==='desc'?'↓':'↑'}}</button></span></label>
      </div>
      <div class="asset-filter-hint"><span>{{tr('探测队列确定一级范围。登录口、注册口和是否看过可以在这里筛选，高级查询里也能按同样的值过滤。点「访问入口」打开浏览器后，这条资产会记为已看。','Probe queue sets the primary scope. Login portal, register portal and seen state can be filtered here or in advanced query. Opening a browser from Access entry marks that asset as seen.')}}</span><b v-if="query.probeOutcomeView && query.probeOutcomeView !== 'all'">{{tr('当前二次筛选：','Secondary filter: ')}}{{probeLabel(query.probeOutcomeView)}}</b></div>
      <div class="asset-toolbar-actions">
        <label class="check-field"><input v-model="query.collapseIdenticalResponses" type="checkbox" @change="search()" />{{tr('完全相同响应仅显示一条','Show one identical response')}}</label>
        <button class="button ghost compact" @click="openPageRules()"><ShieldBan :size="15" /> {{tr('页面隔离规则','Page rules')}}</button>
        <button v-if="query.responseFingerprint" class="button ghost compact" @click="query.responseFingerprint='';search()">{{tr('清除响应组筛选','Clear response group')}}</button>
        <button class="button ghost compact" :class="{ active: showFilters }" @click="showFilters=!showFilters"><Filter :size="15" /> {{tr('高级查询','Advanced')}} <span v-if="query.conditions.length" class="mini-count">{{query.conditions.length}}</span></button>
        <button class="button ghost compact" :title="tr('清除搜索、探测结果和组合条件','Clear search, probe result and combined conditions')" @click="resetQueryFilters"><RotateCcw :size="14" /> {{tr('重置查询','Reset query')}}</button>
        <div class="dropdown-wrap"><button class="button ghost compact" @click="showColumns=!showColumns"><Columns3 :size="15" /> {{tr('字段','Columns')}}</button>
          <div v-if="showColumns" class="column-menu">
            <header><strong>{{tr('表格与导出字段','Table & export fields')}}</strong><span>{{visibleColumns.length}} / {{allColumns.length}}</span></header>
            <div class="column-options"><label v-for="column in allColumns" :key="column[0]"><input type="checkbox" :checked="visibleColumns.includes(column[0])" @change="toggleColumn(column[0])" /> {{tr(column[1],column[2])}}</label></div>
          </div>
        </div>
        <button class="button ghost compact" :title="tr('恢复默认展示字段和列宽','Reset default columns and widths')" @click="resetColumns();resetColumnWidths()"><RotateCcw :size="14" /> {{tr('重置字段','Reset')}}</button>
        <button class="button ghost compact" :title="tr('刷新当前结果','Refresh results')" @click="refresh"><RefreshCw :size="15" :class="{ spinning: loading }" /> {{tr('刷新','Refresh')}}</button>
        <button v-if="query.projectId&&!queryProjectArchived" class="button secondary compact" @click="emit('reprobe',query.projectId)"><RefreshCw :size="15" /> {{tr('复测现有存活','Re-probe existing')}}</button>
        <span v-else-if="queryProjectArchived" class="archived-workspace-note">{{tr('已归档 · 仅查看','Archived · view only')}}</span>
        <button class="button secondary compact" :title="tr('导出全部匹配URL及当前字段，保留折叠组的各条来源','Export all matching URLs and selected columns, including collapsed members')" @click="exportCurrent"><Download :size="15" /> {{tr('导出全部URL · 当前字段','Export all URLs · columns')}}</button>
      </div>
    </div>

    <div v-if="showFilters" class="advanced-panel panel">
      <div class="advanced-title"><div><strong>{{tr('组合查询','Combined query')}}</strong><span>{{tr('支持 AND/OR、精确、包含、范围和空值查询','AND/OR, exact, contains, range and empty-value filters')}}</span></div><button class="button ghost compact" @click="addCondition">+ {{tr('添加条件','Add condition')}}</button></div>
      <div v-for="(condition,index) in query.conditions" :key="index" class="condition-row">
        <select v-model="condition.join" :disabled="index===0"><option value="and">AND</option><option value="or">OR</option></select>
        <select v-model="condition.field" @change="changeConditionField(condition)"><option v-for="column in allColumns" :key="column[0]" :value="column[0]">{{tr(column[1],column[2])}}</option></select>
        <select v-model="condition.operator"><option v-for="operator in operatorsFor(condition.field)" :key="operator[0]" :value="operator[0]">{{operator[1]}}</option></select>
        <select v-if="conditionOptions(condition.field).length && !['isEmpty','notEmpty'].includes(condition.operator)" v-model="condition.value">
          <option value="" disabled>{{tr('选择查询值','Select value')}}</option>
          <option v-for="option in conditionOptions(condition.field)" :key="option[0]" :value="option[0]">{{tr(option[1],option[2])}}</option>
        </select>
        <input v-else v-model="condition.value" :type="dateFields.has(condition.field)?'date':numericFields.has(condition.field)?'number':'text'" :disabled="['isEmpty','notEmpty'].includes(condition.operator)" :placeholder="tr('查询值','Value')" @keyup.enter="search" />
        <button class="icon-button subtle" @click="removeCondition(index)"><X :size="15" /></button>
      </div>
      <div v-if="!query.conditions.length" class="advanced-empty">{{tr('尚未添加条件','No conditions')}}</div>
      <p class="query-scope-note">{{tr('组合条件按从左到右计算；始终叠加上方项目、探测队列、人工结论及回收范围。查询所有等级记录时，将探测队列和人工结论切换为“全部”。','Conditions combine from left to right within the project, probe queue, decision and trash filters. Choose all probe classes and all decisions to search every tier record.')}}</p>
      <div class="advanced-actions"><button class="button ghost compact" @click="query.conditions=[];search()">{{tr('清空','Clear')}}</button><button class="button primary compact" @click="search">{{tr('应用查询','Apply')}}</button></div>
    </div>

    <div v-if="selectedRows.length" class="bulk-bar">
      <button v-if="selectedRows.length===1" :disabled="bulkBusy" @click="openPageRules(selectedRows[0])"><ShieldBan :size="15" /> {{tr('隔离此类页面','Isolate this page type')}}</button>
      <div class="bulk-context"><strong>{{tr(`已选择 ${selectedRows.length} 条`,`${selectedRows.length} selected`)}}</strong><span v-if="selectedProjectCount > 1">{{tr(`来自 ${selectedProjectCount} 个项目`,`Across ${selectedProjectCount} projects`)}}</span><span v-else>{{tr('选择会跨页保留','Selection persists across pages')}}</span></div><span class="bulk-divider"></span>
      <button class="bulk-confirm" :disabled="bulkBusy" :title="tr('确认属于有效资产；保存后从待复核队列移出','Mark valid and remove from the review queue')" @click="act('confirmed')"><Check :size="15" /><span><strong>{{tr('确认有效','Confirm valid')}}</strong><small>{{tr('移出待审核','Leave review')}}</small></span></button>
      <button class="bulk-uncertain" :disabled="bulkBusy" :title="tr('已经人工看过，但还需要补充证据；继续保留在复核队列','Reviewed but needs more evidence; keep in review')" @click="act('uncertain')"><ShieldQuestion :size="15" /><span><strong>{{tr('保留待核','Keep for review')}}</strong><small>{{tr('等待补证据','Needs evidence')}}</small></span></button>
      <label class="bulk-scan-mode" :title="tr('任务模式会固化到调查任务，确认后仍按该上限执行','The selected mode is stored with the investigation task and remains effective after confirmation')"><span>{{tr('扫描模式','Scan mode')}}</span><select v-model="assetScanMode"><option value="quick">{{tr('快速扫描','Quick')}}</option><option value="standard">{{tr('标准扫描','Standard')}}</option><option value="deep">{{tr('深度扫描','Deep')}}</option></select></label>
      <span v-if="scanningActionError" role="alert">{{scanningActionError}}</span><span v-if="props.scanningEnabled===false">{{tr('扫描插件已停用','Scanning is disabled')}}</span>
      <button class="bulk-send" :disabled="bulkBusy||props.scanningEnabled===false" @click="sendToSentinel"><Send :size="15" /> {{tr(`发起自动调查 · ${assetScanModeLabel}`,`Start investigation · ${assetScanModeLabel}`)}}</button>
      <button v-if="query.deletedView !== 'trash' && !quarantineOnly" :disabled="bulkBusy" @click="archive(true)"><Archive :size="15" /> {{tr('移入回收站','Move to trash')}}</button><button v-else :disabled="bulkBusy" @click="archive(false)"><RotateCcw :size="15" /> {{tr('恢复','Restore')}}</button>
      <button class="bulk-close" :title="tr('清除选择','Clear selection')" @click="clearSelection"><X :size="15" /></button>
    </div>

    <p class="row-selection-hint">{{tr('在选框旁或行内空白处按住鼠标拖过多行，再点击“确认有效”或“移入回收站”。标题文字仍可划词；拖选已选中的行可取消选择。折叠组仅操作当前展示条目，需要整组操作请先展开。','Drag from beside a checkbox or a noninteractive cell across rows, then confirm or move to trash. Title text remains selectable. Drag from a selected row to deselect. Expand response groups to select their individual URLs.')}}</p>
    <div class="table-card panel">
      <div class="table-scroll">
        <table class="asset-table">
          <colgroup><col style="width:40px" /><col v-for="column in visibleColumns" :key="column" :style="{width:`${columnWidth(column)}px`}" /></colgroup>
          <thead><tr><th class="check-cell"><input v-model="selectedAll" type="checkbox" /></th><th v-for="column in visibleColumns" :key="column" :style="{width:`${columnWidth(column)}px`}"><button class="asset-sort-heading" :class="{ sortable: sortableFields.has(column), active: query.sortBy === column }" :disabled="!sortableFields.has(column)" @click="sortColumn(column)"><span>{{columnLabel(column)}}</span><ArrowUpDown v-if="sortableFields.has(column)" :size="12" /><b v-if="query.sortBy === column">{{query.sortDirection === 'desc' ? '↓' : '↑'}}</b></button><i class="column-resizer" :title="tr('拖动调整列宽，双击恢复','Drag to resize; double-click to reset')" @pointerdown="startColumnResize($event,column)" @dblclick.stop="resetColumnWidth(column)"></i></th></tr></thead>
          <tbody>
            <tr v-for="(asset,index) in assets" :key="`${asset.projectId}-${asset.id}`" :data-asset-row="index" @pointerdown="startRowDrag($event,index)" :class="{ selected: selected.has(selectionKey(asset)), deleted: asset.isDeleted, 'entry-seen': !!asset.entryOpenedAt }">
              <td class="check-cell"><input type="checkbox" :checked="selected.has(selectionKey(asset))" @change="onAssetCheckbox(asset,$event)" /></td>
              <td v-for="column in visibleColumns" :key="column" :class="[`col-${column}`]">
                <span v-if="column === 'host' && entryUrl(asset)" class="asset-entry">
                  <button class="asset-link" :title="entryUrl(asset)" @click="openAsset(asset)">{{ displayValue(asset,column) }}</button>
                  <button v-if="responseGroupCount(asset)>1" type="button" class="asset-response-group" :title="tr('查看当前筛选内的相同响应URL，操作各成员独立生效','View identical-response URLs in the current filter; actions affect individual members')" @click="showResponseMembers(asset)">
                    <Layers :size="11" aria-hidden="true" />
                    <span>{{tr(`同响应 ${responseGroupCount(asset)} URL`,`${responseGroupCount(asset)} identical URLs`)}}</span>
                    <ChevronRight :size="11" aria-hidden="true" />
                  </button>
                </span>
                <button v-else-if="column === 'link' && String(value(asset,column)).startsWith('http')" class="asset-link" :title="String(value(asset,column))" @click="openLink(asset)">{{ displayValue(asset,column) }}</button>
                <span v-else-if="['probeOutcome','reviewTier','decision','ownershipStatus','exposureEligible','contentCategory','isDeleted','hasLogin','hasRegister','entrySeen'].includes(column)" class="data-badge" :class="badgeClass(column,String(value(asset,column)))" :title="column === 'entrySeen' ? (asset.entryOpenedAt || '') : ''">{{displayValue(asset,column)}}</span>
                <span v-else-if="column==='sentinelStatus'" class="data-badge" :class="badgeClass(column,asset.sentinelStatus)" :title="asset.sentinelSentAt||''">{{sentinelLabel(asset)}}</span>
                <span v-else-if="column==='title'" class="title-rule-source" :title="String(value(asset,column))" @mouseup="captureTitleSelection($event,asset)">{{value(asset,column)||'—'}}</span>
                <span v-else :title="String(value(asset,column))">{{ displayValue(asset,column) }}</span>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="!loading && !assets.length" class="empty-state">{{tr('没有符合条件的资产','No matching assets')}}</div>
        <div v-if="loading" class="table-loading"><span></span><span></span><span></span></div>
      </div>
      <footer class="pagination"><span>{{tr(`共 ${total.toLocaleString()} 条`,`${total.toLocaleString()} total`)}}<small v-if="query.collapseIdenticalResponses && memberTotal>total">{{tr(`（${memberTotal.toLocaleString()} 个 URL，完全相同响应已折叠）`,` (${memberTotal.toLocaleString()} URLs, identical responses collapsed)`)}}</small></span><span v-if="selectedRows.length" class="pagination-selected">{{tr(`已跨页选择 ${selectedRows.length} 条`,`${selectedRows.length} selected across pages`)}}</span><select v-model="query.pageSize" @change="query.page=1;refresh()"><option :value="50">50 {{tr('/页','/page')}}</option><option :value="100">100 {{tr('/页','/page')}}</option><option :value="200">200 {{tr('/页','/page')}}</option></select><button :disabled="query.page<=1" @click="query.page--;refresh()"><ChevronLeft :size="16" /></button><span>{{query.page}} / {{pages}}</span><button :disabled="query.page>=pages" @click="query.page++;refresh()"><ChevronRight :size="16" /></button></footer>
    </div>
    <TitleRulePopover v-if="titleRuleSelection" :text="titleRuleSelection.text" :x="titleRuleSelection.x" :y="titleRuleSelection.y" :busy="ruleBusy" @apply="applyTitleRule" @close="titleRuleSelection=undefined" />
    <PageIsolationDialog v-if="pageRuleOpen" :projects="projects" :asset="pageRuleAsset" :project-id="query.projectId" :keyword="pageRuleKeyword" @close="pageRuleOpen=false" @changed="pageRulesChanged" @notify="(type,text)=>emit('notify',type,text)" />
  </section>
</template>

<style scoped>
.row-selection-hint,.query-scope-note {color:var(--muted);font-size:11px;line-height:1.7;margin:10px 0;}
.row-selecting :deep(.asset-table) {user-select:none;cursor:crosshair;}
.check-cell {cursor:crosshair;}
.asset-workspace.has-selection {padding-bottom:110px;}
.asset-workspace .bulk-bar {position:fixed;top:auto;bottom:18px;left:268px;right:24px;z-index:25;}
:global(.sidebar-collapsed) .asset-workspace .bulk-bar {left:92px;}
@media(max-width:1200px){.asset-workspace .bulk-bar {left:234px;}}
@media(max-width:820px){.asset-workspace .bulk-bar {left:20px;right:20px;}}
.asset-entry { display:flex; align-items:center; gap:6px; min-width:0; }
.asset-entry .asset-link { flex:1 1 auto; min-width:0; text-align:left; }
.asset-response-group { display:inline-flex; flex:0 0 auto; align-items:center; gap:3px; padding:1px 5px; border:1px solid color-mix(in srgb,var(--blue) 18%,var(--line)); border-radius:5px; background:color-mix(in srgb,var(--blue) 6%,var(--app-surface)); color:var(--blue); font-size:9px; font-weight:600; line-height:16px; white-space:nowrap; transition:background .15s,border-color .15s; }
.asset-response-group:hover { background:color-mix(in srgb,var(--blue) 12%,var(--app-surface)); border-color:color-mix(in srgb,var(--blue) 38%,var(--line)); }
.asset-response-group:focus-visible { outline:2px solid var(--blue); outline-offset:2px; }
.asset-response-group svg { flex-shrink:0; }
</style>
