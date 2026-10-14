<script setup lang="ts">
// O2 决策组队列（设计文档 §7/§8）。分析与分组在后端任务中完成；本组件只读取分页组摘要、按需展开成员，
// 批量动作只提交 groupId + 预览返回的版本/哈希，从不下载或回传资产 ID 列表。所有响应按 projectId + 代次丢弃过期结果。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  ownershipReviewApi, OWNERSHIP_LANE_LABELS, OWNERSHIP_NAV_LABELS,
  type OwnershipAnalysisOverview, type OwnershipGroupAction, type OwnershipGroupDecisionPreview, type OwnershipGroupDecisionResult,
  type OwnershipGroupMember, type OwnershipReviewGroup, type OwnershipRevertPreview,
} from "../ownershipDirectoryApi";

const props = defineProps<{ projectId: number; refreshToken?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; applied: [projectId: number] }>();

const GROUP_PAGE_SIZE = 20;
const MEMBER_PAGE_SIZE = 50;
// O3 轮询退避：有进展时 1 秒；无进展每次 ×1.6，上限 10 秒；读取失败每次 ×2，上限 30 秒；有进展立即回到 1 秒。
const POLL_MIN_MS = 1000;
const POLL_IDLE_MAX_MS = 10000;
const POLL_ERROR_MAX_MS = 30000;
function nextPollDelay(current: number, progressed: boolean, failed: boolean) {
  if (failed) return Math.min(Math.max(current, POLL_MIN_MS) * 2, POLL_ERROR_MAX_MS);
  if (progressed) return POLL_MIN_MS;
  return Math.min(Math.round(Math.max(current, POLL_MIN_MS) * 1.6), POLL_IDLE_MAX_MS);
}
const MODE_REASON_LABELS: Record<string, string> = { requested_full: "手动全量", no_snapshot: "首次分析", base_without_watermark: "基线无水位" };
const ACTION_LABELS: Record<OwnershipGroupAction, string> = { confirm_ownership: "确认归属（不授权）", related: "仅集团/业务关联", third_party: "第三方服务", excluded: "排除" };
const READINESS_LABELS: Record<string, string> = { rules_only: "仅确定规则", model_not_ready: "模型未接入", analysis_running: "分析进行中", stale_snapshot: "旧快照", no_snapshot: "尚无快照" };
const REASON_LABELS: Record<string, string> = {
  deleted_or_unlinked: "已删除或不再关联", left_review_scope: "已离开复核范围", decision_changed: "归属结论/授权在快照后变化",
  evidence_changed: "证据在快照后变化", manual_decision_locked: "已有人工结论（不覆盖）", write_guard_missed: "写入守卫未命中",
  changed_after_operation: "操作之后已被修改（保留现状）", authorization_not_restored: "旧值含允许/准入，撤销后不恢复",
  no_decision_row: "无结论行", already_revoked: "已是撤销状态",
};
const BOUNDARY_LABELS: Record<string, string> = {
  registrable_domain: "可注册域", exact_host: "精确主机（共享域名不泛化）", private_suffix_tenant: "托管平台租户", public_suffix_host: "公共后缀主机", ip: "IP", none: "无主机",
};

const overview = ref<OwnershipAnalysisOverview | null>(null);
const loadedProjectId = ref(0);
const groups = ref<OwnershipReviewGroup[]>([]);
const groupsTotal = ref(0);
const groupsCursor = ref("");
const groupsRunId = ref(0);
const loadingGroups = ref(false);
const laneFilter = ref("");
const navFilter = ref("");
const regionFilter = ref("");
const expanded = ref<Record<number, { items: OwnershipGroupMember[]; total: number; cursor: string; loading: boolean }>>({});
const preview = ref<OwnershipGroupDecisionPreview | null>(null);
const previewGroup = ref<OwnershipReviewGroup | null>(null);
// 每份预览一个 operationId；网络失败后重试沿用同一个（后端幂等），预览失效后丢弃。
const operationId = ref("");
const lastResult = ref<OwnershipGroupDecisionResult | null>(null);
const starting = ref(false);
const cancelling = ref(false);
const previewing = ref(false);
const applying = ref(false);
// O3：大组写入分块进行；首个调用在预算内返回 applying 时，按操作编号轮询真实进度直至完成。
const pendingOperation = ref<OwnershipGroupDecisionResult | null>(null);
const revertPreview = ref<OwnershipRevertPreview | null>(null);
const revertOperationId = ref("");
const reverting = ref(false);
const revokeTarget = ref<OwnershipReviewGroup | null>(null);
const revokeOperationId = ref("");
const revoking = ref(false);
const pollDelay = ref(POLL_MIN_MS);
let lastProgressKey = "";
let generation = 0;
let pollTimer: ReturnType<typeof setTimeout> | null = null;

const ready = computed(() => loadedProjectId.value === props.projectId && !!overview.value);
const snapshot = computed(() => overview.value?.snapshot ?? null);
const activeRun = computed(() => overview.value?.activeRun ?? null);
const queue = computed(() => overview.value?.queue ?? null);
const stale = computed(() => !!overview.value?.readiness.includes("stale_snapshot"));
const increment = computed(() => (queue.value ? queue.value.newSinceSnapshot + queue.value.changedSinceSnapshot : 0));
const isCurrent = (projectId: number, token: number) => token === generation && projectId === props.projectId;
const newOperationId = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `op-${Date.now()}-${Math.random().toString(36).slice(2, 12)}`);

function stopPolling() { if (pollTimer) { clearTimeout(pollTimer); pollTimer = null; } }
function schedulePoll(projectId: number, token: number, progressKey: string | null, failed = false) {
  stopPolling();
  const progressed = progressKey !== null && progressKey !== lastProgressKey;
  if (progressKey !== null) lastProgressKey = progressKey;
  pollDelay.value = nextPollDelay(pollDelay.value, progressed, failed);
  pollTimer = setTimeout(() => {
    if (!isCurrent(projectId, token)) return;
    if (pendingOperation.value) void pollOperation(); else void loadOverview();
  }, pollDelay.value);
}
function runKindText(run: { runKind?: string; baseRunId?: number | null; modeReason?: string; reclassifiedAssets?: number; carriedAssets?: number; resumeCount?: number }) {
  const resumed = run.resumeCount ? ` · 崩溃/重启后接管 ${run.resumeCount} 次` : "";
  if (run.runKind === "incremental") return `增量（基于快照 #${run.baseRunId}，重新分类 ${(run.reclassifiedAssets ?? 0).toLocaleString()} · 沿用 ${(run.carriedAssets ?? 0).toLocaleString()}）${resumed}`;
  const reason = run.modeReason ?? "";
  const label = MODE_REASON_LABELS[reason] ?? (reason.startsWith("versions_changed") ? `版本变化：${reason.split(":").slice(1).join(":")}` : reason);
  return `全量${label ? `（${label}）` : ""}${resumed}`;
}
function resetState() {
  generation++; stopPolling();
  overview.value = null; loadedProjectId.value = 0; groups.value = []; groupsTotal.value = 0; groupsCursor.value = ""; groupsRunId.value = 0;
  expanded.value = {}; preview.value = null; previewGroup.value = null; operationId.value = ""; lastResult.value = null;
  pendingOperation.value = null; revertPreview.value = null; revertOperationId.value = ""; revokeTarget.value = null; revokeOperationId.value = "";
  pollDelay.value = POLL_MIN_MS; lastProgressKey = "";
}
function laneLabel(lane: string) { return OWNERSHIP_LANE_LABELS[lane] ?? lane; }
function navLabel(nav: string) { return OWNERSHIP_NAV_LABELS[nav] ?? nav; }
function groupTitle(group: OwnershipReviewGroup) {
  const who = group.candidateLabel || "未识别单位";
  const tag = group.candidateSource === "label_candidate" ? "（标签候选）" : group.candidateSource === "network_clue" ? "（网络线索，不是法人）" : group.candidateSource === "directory_unbound" ? "（目录候选，项目未绑定）" : "";
  return `${who}${tag} / ${BOUNDARY_LABELS[group.boundaryKind] ?? group.boundaryKind}${group.boundaryValue ? ` ${group.boundaryValue}` : ""}`;
}
function permissionText(group: OwnershipReviewGroup) {
  return `授权 ${group.authorizationStatus}，暴露面准入${group.exposureEligible ? "是" : "否"}${group.decisionSource === "manual" ? `，人工结论 ${group.decisionStatus}` : ""}`;
}

async function loadOverview(options: { reloadGroups?: boolean } = {}) {
  const projectId = props.projectId; const token = generation;
  try {
    const next = await ownershipReviewApi.readOwnershipAnalysis(projectId);
    if (!isCurrent(projectId, token)) return;
    overview.value = next; loadedProjectId.value = projectId;
    const snapshotId = next.snapshot?.id ?? 0;
    if (snapshotId && (options.reloadGroups || snapshotId !== groupsRunId.value)) await loadGroups(true);
    if (!snapshotId) { groups.value = []; groupsTotal.value = 0; groupsCursor.value = ""; groupsRunId.value = 0; }
    stopPolling();
    if (pendingOperation.value) schedulePoll(projectId, token, null);
    else if (next.activeRun) schedulePoll(projectId, token, `${next.activeRun.id}:${next.activeRun.status}:${next.activeRun.processedAssets}`);
    else { pollDelay.value = POLL_MIN_MS; lastProgressKey = ""; }
  } catch (error) {
    if (!isCurrent(projectId, token)) return;
    emit("notify", "error", String(error));
    // 正在跟踪的任务读取失败：按错误退避继续，而不是放弃进度显示或密集重试。
    if (overview.value?.activeRun || pendingOperation.value) schedulePoll(projectId, token, null, true);
  }
}
async function pollOperation() {
  const projectId = props.projectId; const token = generation; const current = pendingOperation.value;
  if (!current) return;
  try {
    const view = await ownershipReviewApi.readOwnershipOperation(projectId, current.operationId);
    if (!isCurrent(projectId, token) || pendingOperation.value?.operationId !== current.operationId) return;
    const result: OwnershipGroupDecisionResult = { ...current, applied: view.applied, skipped: view.skipped, conflicted: view.conflicted, failed: view.failed, reasons: view.reasons, status: view.status === "completed" ? "completed" : view.status === "failed" ? "failed" : "applying", totalItems: view.totalItems, doneItems: view.doneItems };
    if (view.status === "applying") { pendingOperation.value = result; schedulePoll(projectId, token, `${view.operationId}:${view.doneItems}`); return; }
    pendingOperation.value = null; lastResult.value = result; pollDelay.value = POLL_MIN_MS;
    if (view.status === "failed") emit("notify", "error", `组操作 ${view.operationId} 失败：${view.error}（已写入的分块保留，未授予任何权限）`);
    else { emit("notify", "success", `组操作完成：写入 ${result.applied}，跳过 ${result.skipped}，冲突 ${result.conflicted}，失败 ${result.failed}`); emit("applied", projectId); }
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    if (isCurrent(projectId, token)) { emit("notify", "error", String(error)); schedulePoll(projectId, token, null, true); }
  }
}
async function loadGroups(reset: boolean) {
  const projectId = props.projectId; const token = generation; const run = overview.value?.snapshot;
  if (!run || loadingGroups.value && !reset) return;
  loadingGroups.value = true;
  try {
    const page = await ownershipReviewApi.pageOwnershipGroups({ projectId, runId: run.id, lane: laneFilter.value, navBucket: navFilter.value, region: regionFilter.value.trim(), pageSize: GROUP_PAGE_SIZE, cursor: reset ? "" : groupsCursor.value });
    if (!isCurrent(projectId, token) || page.snapshot.id !== overview.value?.snapshot?.id) return;
    groups.value = reset ? page.items : [...groups.value, ...page.items];
    groupsTotal.value = page.total; groupsCursor.value = page.nextCursor; groupsRunId.value = page.snapshot.id;
    if (reset) expanded.value = {};
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) loadingGroups.value = false; }
}
function applyFilters() { void loadGroups(true); }
async function startAnalysis() {
  const projectId = props.projectId; const token = generation;
  if (starting.value) return;
  starting.value = true;
  try {
    await ownershipReviewApi.startOwnershipAnalysis(projectId);
    if (isCurrent(projectId, token)) await loadOverview();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) starting.value = false; }
}
async function cancelAnalysis() {
  const projectId = props.projectId; const token = generation; const run = activeRun.value;
  if (!run || cancelling.value) return;
  cancelling.value = true;
  try {
    await ownershipReviewApi.cancelOwnershipAnalysis(projectId, run.id);
    if (isCurrent(projectId, token)) { emit("notify", "info", `分析任务 #${run.id} 已请求取消`); await loadOverview(); }
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) cancelling.value = false; }
}
async function loadMembers(group: OwnershipReviewGroup, more = false) {
  const projectId = props.projectId; const token = generation;
  const current = expanded.value[group.id];
  if (!more && current) { const next = { ...expanded.value }; delete next[group.id]; expanded.value = next; return; }
  if (current?.loading) return;
  expanded.value = { ...expanded.value, [group.id]: { items: current?.items ?? [], total: current?.total ?? group.memberCount, cursor: current?.cursor ?? "", loading: true } };
  try {
    const page = await ownershipReviewApi.pageOwnershipGroupMembers({ projectId, runId: group.runId, groupId: group.id, pageSize: MEMBER_PAGE_SIZE, cursor: more ? current?.cursor ?? "" : "" });
    if (!isCurrent(projectId, token) || !expanded.value[group.id]) return;
    expanded.value = { ...expanded.value, [group.id]: { items: more ? [...(current?.items ?? []), ...page.items] : page.items, total: page.total, cursor: page.nextCursor, loading: false } };
  } catch (error) {
    if (isCurrent(projectId, token)) { const next = { ...expanded.value }; delete next[group.id]; expanded.value = next; emit("notify", "error", String(error)); }
  }
}
async function previewDecision(group: OwnershipReviewGroup, action: OwnershipGroupAction) {
  const projectId = props.projectId; const token = generation;
  if (previewing.value || applying.value) return;
  previewing.value = true; preview.value = null; previewGroup.value = null; operationId.value = "";
  try {
    const next = await ownershipReviewApi.previewOwnershipGroupDecision({ projectId, runId: group.runId, groupId: group.id, action, note: "" });
    if (!isCurrent(projectId, token)) return;
    preview.value = next; previewGroup.value = group; operationId.value = newOperationId();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) previewing.value = false; }
}
function closePreview() { preview.value = null; previewGroup.value = null; operationId.value = ""; }
async function applyDecision() {
  const projectId = props.projectId; const token = generation; const current = preview.value; const opId = operationId.value;
  if (!current || !opId || applying.value || current.projectId !== projectId) return;
  applying.value = true;
  try {
    const result = await ownershipReviewApi.applyOwnershipGroupDecision({
      operationId: opId, projectId, runId: current.runId, groupId: current.groupId, groupVersion: current.groupVersion,
      policyVersion: current.policyVersion, directoryVersion: current.directoryVersion, bindingVersion: current.bindingVersion,
      action: current.action, note: current.note, expectedActionHash: current.expectedActionHash,
    });
    if (!isCurrent(projectId, token)) { emit("notify", "success", `项目 #${projectId} 的组操作已完成：写入 ${result.applied} 条`); return; }
    closePreview();
    if (result.status === "applying") {
      // 大组：已在预算内提交部分分块，后台继续；只显示真实进度，完成前不向下游发布任何“整组已允许”。
      pendingOperation.value = result; pollDelay.value = POLL_MIN_MS; lastProgressKey = "";
      emit("notify", "info", `${ACTION_LABELS[result.action]}：大组分块写入中 ${result.doneItems ?? 0} / ${result.totalItems ?? 0}（后台继续，可随时离开页面）`);
      schedulePoll(projectId, token, `${result.operationId}:${result.doneItems ?? 0}`);
      return;
    }
    lastResult.value = result;
    emit("notify", "success", `${ACTION_LABELS[result.action]}：写入 ${result.applied}，跳过 ${result.skipped}，冲突 ${result.conflicted}，失败 ${result.failed}${result.replayed ? "（重复提交，未再次写入）" : ""}`);
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    const text = String(error);
    if (!isCurrent(projectId, token)) { emit("notify", "error", `项目 #${projectId}：${text}`); return; }
    if (/OWNERSHIP_(PREVIEW_STALE|SNAPSHOT_STALE|OPERATION_PAYLOAD_MISMATCH|GROUP_ACTION_NOT_ALLOWED)/.test(text)) {
      closePreview();
      emit("notify", "error", `${text}（预览已失效，请重新预览）`);
      await loadOverview({ reloadGroups: true });
    } else {
      emit("notify", "error", `${text}（可重试；同一操作编号重复提交不会重复写入）`);
    }
  } finally { if (token === generation) applying.value = false; }
}

async function startFull() {
  const projectId = props.projectId; const token = generation;
  if (starting.value) return;
  starting.value = true;
  try {
    await ownershipReviewApi.startOwnershipAnalysis(projectId, "full");
    if (isCurrent(projectId, token)) await loadOverview();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) starting.value = false; }
}
// 组级撤销：撤销一次已完成的组归属操作；只恢复之后未被改动的成员，绝不恢复允许/准入。
async function previewRevert() {
  const projectId = props.projectId; const token = generation; const target = lastResult.value;
  if (!target || reverting.value) return;
  reverting.value = true; revertPreview.value = null; revertOperationId.value = "";
  try {
    const next = await ownershipReviewApi.previewOwnershipGroupRevert({ projectId, targetOperationId: target.operationId, note: "" });
    if (!isCurrent(projectId, token)) return;
    revertPreview.value = next; revertOperationId.value = newOperationId();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) reverting.value = false; }
}
function closeRevert() { revertPreview.value = null; revertOperationId.value = ""; }
async function applyRevert() {
  const projectId = props.projectId; const token = generation; const current = revertPreview.value; const opId = revertOperationId.value;
  if (!current || !opId || reverting.value || current.projectId !== projectId) return;
  reverting.value = true;
  try {
    const result = await ownershipReviewApi.applyOwnershipGroupRevert({ operationId: opId, projectId, targetOperationId: current.targetOperationId, note: current.note, expectedRevertHash: current.expectedRevertHash });
    if (!isCurrent(projectId, token)) return;
    closeRevert();
    if (result.status === "applying") { pendingOperation.value = result; schedulePoll(projectId, token, `${result.operationId}:${result.doneItems ?? 0}`); return; }
    lastResult.value = result;
    emit("notify", "success", `已撤销组操作 ${current.targetOperationId}：恢复 ${result.applied}，保留现状 ${result.conflicted}${result.reasons.authorization_not_restored ? `；${result.reasons.authorization_not_restored} 条旧授权未恢复` : ""}`);
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    if (!isCurrent(projectId, token)) return;
    if (/OWNERSHIP_(PREVIEW_STALE|REVERT_NOT_ALLOWED|OPERATION_PAYLOAD_MISMATCH)/.test(String(error))) closeRevert();
    emit("notify", "error", String(error));
  } finally { if (token === generation) reverting.value = false; }
}
// 撤销授权：后端先在一个事务内把成员置为已撤销/不准入（下游立即阻断），再更新展示索引。
function askRevoke(group: OwnershipReviewGroup) { revokeTarget.value = group; revokeOperationId.value = newOperationId(); }
function closeRevoke() { revokeTarget.value = null; revokeOperationId.value = ""; }
async function applyRevoke() {
  const projectId = props.projectId; const token = generation; const group = revokeTarget.value; const opId = revokeOperationId.value;
  if (!group || !opId || revoking.value) return;
  revoking.value = true;
  try {
    const result = await ownershipReviewApi.revokeOwnershipGroupAuthorization({ operationId: opId, projectId, runId: group.runId, groupId: group.id, note: "" });
    if (!isCurrent(projectId, token)) return;
    closeRevoke();
    emit("notify", "success", `已撤销授权 ${result.revoked} 条（已阻断下游暴露面）；已是撤销 ${result.alreadyRevoked}，无结论行 ${result.withoutDecision}`);
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", `${String(error)}（可重试；同一操作编号不会重复撤销）`); }
  finally { if (token === generation) revoking.value = false; }
}

watch(() => props.projectId, () => { resetState(); void loadOverview(); });
watch(() => props.refreshToken, () => { if (ready.value) void loadOverview(); });
onMounted(() => { void loadOverview(); });
onBeforeUnmount(() => { generation++; stopPolling(); });
</script>

<template>
  <section class="panel ownership-groups">
    <header>
      <div><strong>归属决策组队列</strong><small>后台按候选单位、域名/托管边界、支持与反证、人工结论和授权状态分组；组确认只写归属事实，不授予扫描或暴露面权限。</small></div>
      <span>
        <template v-for="item in overview?.readiness ?? []" :key="item"><em class="ownership-groups-tag" :class="`tag-${item}`">{{ READINESS_LABELS[item] ?? item }}</em></template>
        <button class="button ghost" :disabled="!ready || starting || !!activeRun" title="自动：版本未变时按数据水位增量更新，否则全量" @click="startAnalysis">{{ activeRun ? "分析中" : snapshot ? "更新分析" : "开始分析" }}</button>
        <button v-if="snapshot && !activeRun" class="button ghost" :disabled="!ready || starting" @click="startFull">全量重算</button>
        <button v-if="activeRun" class="button ghost" :disabled="cancelling || activeRun.cancelRequested" @click="cancelAnalysis">{{ activeRun.cancelRequested ? "正在取消" : "取消分析" }}</button>
      </span>
    </header>
    <div v-if="!ready" class="ownership-groups-empty">加载分析状态…</div>
    <template v-else>
      <div v-if="activeRun" class="ownership-groups-progress">分析任务 #{{ activeRun.id }} {{ activeRun.status === "queued" ? "排队中" : "运行中" }}：已处理 {{ activeRun.processedAssets.toLocaleString() }} / {{ activeRun.totalAssets.toLocaleString() }} 条 · {{ runKindText(activeRun) }}<template v-if="snapshot">；下方仍为旧快照 #{{ snapshot.id }}</template><small> · 下次刷新 {{ Math.round(pollDelay / 100) / 10 }} 秒</small></div>
      <div v-if="pendingOperation" class="ownership-groups-progress">组操作 {{ pendingOperation.operationId }} 分块写入中：{{ (pendingOperation.doneItems ?? 0).toLocaleString() }} / {{ (pendingOperation.totalItems ?? 0).toLocaleString() }}（写入 {{ pendingOperation.applied }} · 冲突 {{ pendingOperation.conflicted }}）；完成前不发布整组结果<small> · 下次刷新 {{ Math.round(pollDelay / 100) / 10 }} 秒</small></div>
      <div v-if="overview?.latestRun && ['failed','cancelled'].includes(overview.latestRun.status) && overview.latestRun.id !== snapshot?.id" class="ownership-groups-progress">最近一次分析 #{{ overview.latestRun.id }} {{ overview.latestRun.status === "failed" ? `失败：${overview.latestRun.error}` : "已取消" }}（已处理 {{ overview.latestRun.processedAssets.toLocaleString() }} 条，未发布快照）</div>
      <div v-if="!snapshot" class="ownership-groups-empty">尚无完成的分析快照。没有快照不代表没有待办——请先开始分析。</div>
      <template v-else-if="queue">
        <div v-if="stale" class="ownership-groups-stale">旧快照：快照之后 {{ overview?.staleReasons.join("、") }} 已变化。组确认会被拒绝，请重新分析。</div>
        <div class="ownership-groups-stats">
          <div><small>待复核资产</small><strong>{{ queue.pendingAssets.toLocaleString() }}</strong></div>
          <div><small>待复核组</small><strong>{{ queue.pendingGroups.toLocaleString() }}</strong></div>
          <div><small>冲突组</small><strong>{{ queue.conflictGroups.toLocaleString() }}</strong><span>反证组 {{ queue.counterEvidenceGroups.toLocaleString() }}</span></div>
          <div><small>已完成组</small><strong>{{ queue.completedGroups.toLocaleString() }}</strong><span>含人工结论组</span></div>
          <div><small>未处理增量</small><strong>{{ increment.toLocaleString() }}</strong><span>新增 {{ queue.newSinceSnapshot.toLocaleString() }} · 变化 {{ queue.changedSinceSnapshot.toLocaleString() }}</span></div>
        </div>
        <p class="ownership-groups-meta">快照 #{{ snapshot.id }} · {{ runKindText(snapshot) }} · 完成于 {{ snapshot.finishedAt }} · 成员 {{ queue.memberTotal.toLocaleString() }} / 组计数合计 {{ queue.groupMemberSum.toLocaleString() }} · 策略 v{{ snapshot.versions.policyVersion }} · 目录 v{{ snapshot.versions.directoryVersion }} · 绑定 v{{ snapshot.versions.bindingVersion }} · {{ snapshot.versions.algorithmVersion }}</p>
        <div class="ownership-groups-filters">
          <select v-model="laneFilter" @change="applyFilters"><option value="">全部类型</option><option v-for="(label, key) in OWNERSHIP_LANE_LABELS" :key="key" :value="key">{{ label }}</option></select>
          <select v-model="navFilter" @change="applyFilters"><option value="">全部单位导航</option><option v-for="(label, key) in OWNERSHIP_NAV_LABELS" :key="key" :value="key">{{ label }}</option></select>
          <input v-model="regionFilter" placeholder="仅看某地域，例如 浙江" @keyup.enter="applyFilters"/>
          <button class="button ghost" @click="applyFilters">筛选</button>
          <small>共 {{ groupsTotal.toLocaleString() }} 组，已加载 {{ groups.length.toLocaleString() }}；冲突与反证组优先</small>
        </div>
        <div v-if="lastResult" class="ownership-groups-result">最近操作 {{ lastResult.operationId }}：写入 {{ lastResult.applied }} · 跳过 {{ lastResult.skipped }} · 冲突 {{ lastResult.conflicted }} · 失败 {{ lastResult.failed }}<template v-for="(value, key) in lastResult.reasons" :key="key"> · {{ REASON_LABELS[key] ?? key }} {{ value }}</template>
          <button v-if="String(lastResult.action) !== 'revert' && lastResult.status !== 'applying'" :disabled="reverting || applying" @click="previewRevert">预览撤销此次组操作</button></div>
        <div v-if="revertPreview" class="ownership-group-preview" role="dialog" aria-label="组操作撤销预览">
          <header><strong>撤销组操作 {{ revertPreview.targetOperationId }}（{{ revertPreview.targetAction }}）</strong></header>
          <p>该操作写入 {{ revertPreview.appliedInTarget }} 条：可恢复 <strong>{{ revertPreview.revertible }}</strong> 条；之后已被修改、保留现状 {{ revertPreview.conflicted }} 条<template v-for="(value, key) in revertPreview.reasons" :key="key">（{{ REASON_LABELS[key] ?? key }} {{ value }}）</template></p>
          <p>授权：撤销不会恢复任何“允许/准入”；旧值含允许/准入的 {{ revertPreview.authorizationNotRestored }} 条恢复为未授权、不准入。</p>
          <ul><li v-for="note in revertPreview.notes" :key="note">{{ note }}</li></ul>
          <footer><button class="button ghost" :disabled="reverting" @click="closeRevert">取消</button><button class="button primary" :disabled="reverting || !revertPreview.revertible" @click="applyRevert">{{ reverting ? "提交中" : `确认撤销 ${revertPreview.revertible} 条` }}</button></footer>
        </div>
        <div v-if="revokeTarget" class="ownership-group-preview" role="dialog" aria-label="撤销授权确认">
          <header><strong>撤销授权 · {{ groupTitle(revokeTarget) }}</strong></header>
          <p>将把该组 {{ revokeTarget.memberCount.toLocaleString() }} 条成员中已有结论的资产置为：授权 = 已撤销，暴露面准入 = 否。下游暴露面采集立即停止使用这些资产（先阻断，再更新本页显示）；归属结论保持不变。</p>
          <p>撤销后不会被任何后台恢复或组撤销重新授权；需要恢复只能逐条重新授权。</p>
          <footer><button class="button ghost" :disabled="revoking" @click="closeRevoke">取消</button><button class="button primary exclude" :disabled="revoking" @click="applyRevoke">{{ revoking ? "提交中" : "确认撤销授权" }}</button></footer>
        </div>

        <article v-for="group in groups" :key="group.id" class="ownership-group-card" :class="`lane-${group.lane}`">
          <header>
            <div><strong>{{ groupTitle(group) }}</strong><small>{{ navLabel(group.navBucket) }}<template v-if="group.regionClue"> · 地域线索 {{ group.regionClue }}</template> · {{ group.relationKind }}</small></div>
            <span class="ownership-group-lane">{{ laneLabel(group.lane) }}</span>
          </header>
          <p>成员 {{ group.memberCount.toLocaleString() }} 条 · {{ group.distinctHosts.toLocaleString() }} 个主机<template v-if="group.appliedOperations"> · 已执行组操作 {{ group.appliedOperations }} 次</template><template v-if="group.authorizationRevoked"> · 已撤销授权 {{ group.authorizationRevoked }} 条</template></p>
          <p class="ownership-group-reason">{{ group.reason }}</p>
          <p v-if="group.counterKind" class="ownership-group-counter">反证：{{ group.counterKind }}（不隐藏，不能整组确认归属）</p>
          <p class="ownership-group-permission">现有权限：{{ permissionText(group) }}</p>
          <ul class="ownership-group-samples">
            <li v-for="sample in group.samples" :key="sample.assetId"><code>{{ sample.host || sample.link || `#${sample.assetId}` }}</code> {{ sample.company || "无公司" }} · {{ sample.current.status }}<em v-if="sample.changedSinceSnapshot"> · 快照后已变化</em></li>
          </ul>
          <small class="ownership-group-sample-rule">代表样本 {{ group.samples.length }} / {{ group.memberCount }}：{{ group.sampleRule }}</small>
          <footer>
            <button @click="loadMembers(group)">{{ expanded[group.id] ? "收起成员" : `查看全部 ${group.memberCount.toLocaleString()} 条成员` }}</button>
            <button v-for="action in group.allowedActions" :key="action" :class="{ confirm: action === 'confirm_ownership', exclude: action === 'excluded' }" :disabled="previewing || applying || stale" @click="previewDecision(group, action)">预览{{ ACTION_LABELS[action] }}</button>
            <button v-if="group.authorizationStatus === 'allowed' || group.exposureEligible" class="exclude" :disabled="revoking || applying" @click="askRevoke(group)">撤销授权</button>
            <small v-if="!group.allowedActions.length">已有人工结论：请在下方资产明细中逐条修改</small>
          </footer>
          <div v-if="expanded[group.id]" class="ownership-group-members">
            <table>
              <thead><tr><th>资产</th><th>主机 / 可注册域</th><th>公司</th><th>当前结论</th><th>授权 / 准入</th></tr></thead>
              <tbody><tr v-for="member in expanded[group.id].items" :key="member.assetId" :class="{ changed: member.changedSinceSnapshot }">
                <td>#{{ member.assetId }}</td><td><code>{{ member.host }}</code><small> {{ member.registrableDomain }} · {{ member.suffixSection }}</small></td>
                <td>{{ member.company || "无" }}</td><td>{{ member.current.status }}<small v-if="member.current.source"> · {{ member.current.source }}</small><em v-if="member.changedSinceSnapshot"> · 快照后已变化</em></td>
                <td>{{ member.current.authorizationStatus }} / {{ member.current.exposureEligible ? "准入" : "不准入" }}</td>
              </tr></tbody>
            </table>
            <button v-if="expanded[group.id].cursor" :disabled="expanded[group.id].loading" @click="loadMembers(group, true)">加载更多（{{ expanded[group.id].items.length }} / {{ expanded[group.id].total }}）</button>
            <small v-else>已显示全部 {{ expanded[group.id].total }} 条</small>
          </div>
        </article>
        <button v-if="groupsCursor" class="button ghost ownership-groups-more" :disabled="loadingGroups" @click="loadGroups(false)">{{ loadingGroups ? "加载中…" : "加载更多组" }}</button>
      </template>

      <div v-if="preview && previewGroup" class="ownership-group-preview" role="dialog" aria-label="组批量预览">
        <header><strong>预览：{{ ACTION_LABELS[preview.action] }} · {{ groupTitle(previewGroup) }}</strong></header>
        <p>快照 #{{ preview.runId }} · 组 v{{ preview.groupVersion }} · 成员 {{ preview.memberCount }}：将写入 <strong>{{ preview.applicable }}</strong> 条，跳过 {{ preview.skipped }}，冲突（快照后变化，不写入）{{ preview.conflicted }}</p>
        <p v-if="Object.keys(preview.reasons).length">排除原因：<template v-for="(value, key) in preview.reasons" :key="key">{{ REASON_LABELS[key] ?? key }} {{ value }}；</template></p>
        <p v-if="preview.excludedSamples.length">被排除样本：<template v-for="item in preview.excludedSamples" :key="item.assetId">#{{ item.assetId }}（{{ REASON_LABELS[item.reason] ?? item.reason }}） </template></p>
        <p>拟改变：归属 = {{ preview.proposed.status }}（人工锁定）；授权 = {{ preview.proposed.authorization === "unchanged" ? "保持各资产现状" : "拒绝" }}；暴露面准入 = {{ preview.proposed.exposureEligible === "unchanged" ? "保持各资产现状" : "否" }}</p>
        <p>现有授权分布：<template v-for="(value, key) in preview.currentAuthorization" :key="key">{{ key }} {{ value }}；</template></p>
        <ul><li v-for="note in preview.notes" :key="note">{{ note }}</li></ul>
        <footer><button class="button ghost" :disabled="applying" @click="closePreview">取消</button><button class="button primary" :disabled="applying || !preview.applicable" @click="applyDecision">{{ applying ? "提交中" : `确认对 ${preview.applicable} 条执行` }}</button></footer>
      </div>
    </template>
  </section>
</template>

<style scoped>
.ownership-groups { overflow: hidden; }
.ownership-groups > header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 15px 18px; border-bottom: 1px solid #e8edf3; background: #fafbfd; }
.ownership-groups > header > div { display: grid; gap: 3px; }
.ownership-groups header strong { font-size: 12px; }
.ownership-groups header small { color: #8491a3; font-size: 9px; }
.ownership-groups > header > span { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.ownership-groups-tag { font-style: normal; font-size: 9px; padding: 2px 6px; border-radius: 8px; background: #eef2f7; color: #4b5a6e; }
.ownership-groups-tag.tag-stale_snapshot, .ownership-groups-stale { background: #fff4e5; color: #9a5b00; }
.ownership-groups-empty, .ownership-groups-progress, .ownership-groups-stale, .ownership-groups-result, .ownership-groups-meta { margin: 10px 18px; font-size: 11px; color: #5b6878; }
.ownership-groups-stale { padding: 8px 10px; border-radius: 6px; }
.ownership-groups-stats { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; margin: 10px 18px; }
.ownership-groups-stats > div { display: grid; gap: 2px; padding: 10px; border: 1px solid #e8edf3; border-radius: 8px; }
.ownership-groups-stats small, .ownership-groups-stats span { color: #8491a3; font-size: 9px; }
.ownership-groups-stats strong { font-size: 16px; }
.ownership-groups-filters { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin: 0 18px 10px; font-size: 11px; }
.ownership-group-card { margin: 0 18px 10px; padding: 10px 12px; border: 1px solid #e8edf3; border-left: 3px solid #b8c4d4; border-radius: 8px; font-size: 11px; }
.ownership-group-card.lane-conflict { border-left-color: #d64545; }
.ownership-group-card.lane-counter_evidence { border-left-color: #e08a00; }
.ownership-group-card.lane-consistent { border-left-color: #2f9e6b; }
.ownership-group-card > header { display: flex; justify-content: space-between; gap: 8px; }
.ownership-group-card > header > div { display: grid; gap: 2px; }
.ownership-group-card p { margin: 4px 0; }
.ownership-group-counter { color: #b35400; }
.ownership-group-lane { font-size: 10px; white-space: nowrap; }
.ownership-group-samples { margin: 4px 0; padding-left: 16px; }
.ownership-group-sample-rule { color: #8491a3; font-size: 9px; }
.ownership-group-card footer { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
.ownership-group-members { margin-top: 8px; max-height: 320px; overflow: auto; }
.ownership-group-members table { width: 100%; border-collapse: collapse; }
.ownership-group-members td, .ownership-group-members th { padding: 3px 6px; border-bottom: 1px solid #f0f3f7; text-align: left; }
.ownership-group-members tr.changed { background: #fff8ec; }
.ownership-group-preview { margin: 10px 18px; padding: 12px; border: 1px solid #c9d4e3; border-radius: 8px; background: #f8fbff; font-size: 11px; }
.ownership-group-preview footer { display: flex; justify-content: flex-end; gap: 8px; }
.ownership-groups-more { margin: 0 18px 12px; }
</style>
