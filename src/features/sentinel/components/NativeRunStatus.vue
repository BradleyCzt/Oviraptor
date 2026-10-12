<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { api } from "../../../api";
import type { NativeScanStatus } from "../../../types";
import { useI18n } from "../../../i18n";
import { createNativeRunStatusLabels } from "./nativeRunStatusLabels";
import { useCommittedRefresh } from "../../../composables/useCommittedRefresh";
import { validCollaborationEvent } from "../timeline/eventContract";
import NativeBudgetDiagnostics from "./NativeBudgetDiagnostics.vue";
import NativeRootReconciliation from "./NativeRootReconciliation.vue";
import NativeProtectionRecovery from "./NativeProtectionRecovery.vue";
import NativeSourcePauseRecovery from "./NativeSourcePauseRecovery.vue";

const props = defineProps<{ scanId: string; attempt: number; status: string }>();
const emit = defineEmits<{
  (event: "attempt-closed", scanId: string, attemptNumber: number): void;
  (event: "prepare-handoff", scanId: string): void;
}>();
const { tr } = useI18n();
const { actionLabel, branchDispatchDescription, branchStatusLabel, diagnosticLabel,
  obligationLabel, roleLabel, statusLabel } = createNativeRunStatusLabels(tr);
const state = ref<NativeScanStatus>();
const error = ref("");
const connectionUnavailable = ref(false);
const activeReads = ref(0);
const recoveryConfirmation = ref<{ scanId: string; attemptNumber: number }>();
const recovering = ref(false);
const recoveryMessage = ref("");
const recoveryError = ref("");
const closureConfirmation = ref<{ scanId: string; attemptNumber: number }>();
const closing = ref(false);
const closureMessage = ref("");
const closureError = ref("");
const administrativeConfirmation = ref<{ scanId: string; attemptNumber: number; operationId: string; snapshotHash: string; previousStatus: string }>();
const administrativeBusy = ref(false);
const administrativeError = ref("");
const administrativeMessage = ref("");
let actionGeneration = 0;
let generation = 0;
let disposed = false;
const pending = computed(() => state.value?.administrativeClosure ? [] : state.value?.branches.filter(b => b.status === "pending") ?? []);
const canAdministrativelyClose = computed(() => !administrativeBusy.value && !closing.value && !recovering.value && !error.value
  && state.value?.scanId === props.scanId && state.value.attemptNumber === props.attempt
  && state.value.status === props.status && state.value.manualAdministrativeClosureAvailable === true
  && !state.value.administrativeClosure);

async function requestAdministrativeClosure() {
  if (!canAdministrativelyClose.value) return;
  const selected = { scanId: props.scanId, attemptNumber: props.attempt };
  const action = ++actionGeneration;
  administrativeBusy.value = true;
  administrativeError.value = "";
  administrativeConfirmation.value = undefined;
  const current = () => !disposed && action === actionGeneration && props.scanId === selected.scanId && props.attempt === selected.attemptNumber;
  try {
    const preview = await api.previewWebAdministrativeClosure(selected.scanId, selected.attemptNumber);
    if (!current()) return;
    if (preview.receipt) {
      await load();
      return;
    }
    if (!preview.snapshot || preview.snapshot.scanId !== selected.scanId || preview.snapshot.attemptNumber !== selected.attemptNumber
      || preview.snapshot.schemaVersion !== 1 || preview.snapshot.previousStatus !== props.status
      || typeof preview.snapshotHash !== "string" || !/^[a-f0-9]{64}$/.test(preview.snapshotHash)
      || !Array.isArray(preview.snapshot.records)) throw new Error("administrative_closure_preview_mismatch");
    administrativeConfirmation.value = { ...selected, operationId: crypto.randomUUID(), snapshotHash: preview.snapshotHash,
      previousStatus: preview.snapshot.previousStatus };
  } catch (reason) {
    if (current()) administrativeError.value = String(reason);
  } finally {
    if (current()) administrativeBusy.value = false;
  }
}

async function confirmAdministrativeClosure() {
  const selected = administrativeConfirmation.value;
  if (!selected || administrativeBusy.value || selected.scanId !== props.scanId || selected.attemptNumber !== props.attempt) return;
  const action = ++actionGeneration;
  administrativeBusy.value = true;
  administrativeError.value = "";
  const current = () => !disposed && action === actionGeneration && props.scanId === selected.scanId && props.attempt === selected.attemptNumber;
  try {
    const receipt = await api.closeWebTaskAdministratively(selected.scanId, selected.attemptNumber, selected.operationId, selected.snapshotHash);
    if (!current()) return;
    if (receipt.scanId !== selected.scanId || receipt.attemptNumber !== selected.attemptNumber || receipt.closureId !== selected.operationId
      || receipt.snapshotHash !== selected.snapshotHash || receipt.previousStatus !== selected.previousStatus || receipt.actor !== "local_operator"
      || receipt.executionState !== "administratively_closed_unsettled" || receipt.executionSettled !== false
      || receipt.automaticReplayAllowed !== false || receipt.requiresIndependentTask !== true
      || Object.keys(receipt).length !== 11
      || typeof receipt.closedAt !== "string" || !Number.isFinite(Date.parse(receipt.closedAt))) {
      throw new Error("administrative_closure_receipt_mismatch");
    }
    administrativeConfirmation.value = undefined;
    administrativeMessage.value = tr("已人工结案。原任务不能重启；未知结果、预算占用和证据仍保留。", "Administratively closed. The old task cannot restart; unknown effects, reservations and evidence remain.");
    emit("attempt-closed", selected.scanId, selected.attemptNumber);
    await load();
  } catch (reason) {
    if (current()) administrativeError.value = `${tr("结案未确认。可重试核对同一操作；不会重新执行扫描。", "Closure unconfirmed. Retry checks this same operation; it does not replay the scan.")} ${String(reason)}`;
  } finally {
    if (current()) administrativeBusy.value = false;
  }
}
const canRecover = computed(() => !recovering.value && !closing.value && !error.value
  && props.status === "scanning" && state.value?.status === "scanning"
  && state.value.scanId === props.scanId && state.value.attemptNumber === props.attempt
  && state.value.branches.some(b => b.branch === "web" && b.status === "pending"
    && b.dispatch?.state === "never_claimed" && b.dispatch.manualRecoveryAvailable === true));
const canClose = computed(() => !recovering.value && !closing.value && !error.value
  && props.status === "scanning" && state.value?.status === "scanning"
  && state.value.scanId === props.scanId && state.value.attemptNumber === props.attempt
  && state.value.branches.some(b => b.branch === "web" && b.status === "pending"
    && b.dispatch?.state === "never_claimed" && b.dispatch.manualClosureAvailable === true));

function requestClosure() {
  if (!canClose.value) return;
  recoveryConfirmation.value = undefined;
  closureMessage.value = "";
  closureError.value = "";
  closureConfirmation.value = { scanId: props.scanId, attemptNumber: props.attempt };
}
async function confirmClosure() {
  const selected = closureConfirmation.value;
  if (!selected || !canClose.value || selected.scanId !== props.scanId || selected.attemptNumber !== props.attempt) return;
  const action = ++actionGeneration;
  closing.value = true;
  closureConfirmation.value = undefined;
  const current = () => !disposed && action === actionGeneration
    && props.scanId === selected.scanId && props.attempt === selected.attemptNumber;
  try {
    const result = await api.closeNeverDispatchedWebAttempt(selected.scanId, selected.attemptNumber);
    if (!current()) return;
    if (result.scanId !== selected.scanId || result.attemptNumber !== selected.attemptNumber
      || result.executionState !== "closed_without_dispatch" || result.automaticReplayAllowed !== false
      || typeof result.closureId !== "string" || !result.closureId.trim()
      || typeof result.closedAt !== "string" || !Number.isFinite(Date.parse(result.closedAt))) {
      throw new Error(tr("结案回执不匹配，请刷新核对", "Closure receipt mismatch; refresh and review"));
    }
    closureMessage.value = tr("旧尝试已结束，未执行目标测试。历史记录保留；如需继续，请检查配置后另行点击任务的“重试未完成阶段”，启动新尝试。", "Old attempt closed without target testing. History is preserved; review configuration and separately choose Retry incomplete stages to start a new attempt.");
    emit("attempt-closed", selected.scanId, selected.attemptNumber);
  } catch (reason) {
    if (!current()) return;
    closureError.value = `${tr("结案未确认；请核对当前回执，不会自动重试或启动扫描。", "Closure unconfirmed; review receipts. No automatic retry or scan start.")} ${String(reason)}`;
  } finally {
    if (current()) {
      await load();
      if (current()) closing.value = false;
    }
  }
}
function requestRecovery() {
  if (!canRecover.value) return;
  closureConfirmation.value = undefined;
  recoveryMessage.value = "";
  recoveryError.value = "";
  recoveryConfirmation.value = { scanId: props.scanId, attemptNumber: props.attempt };
}
async function confirmRecovery() {
  const selected = recoveryConfirmation.value;
  if (!selected || !canRecover.value || selected.scanId !== props.scanId || selected.attemptNumber !== props.attempt) return;
  const action = ++actionGeneration;
  recovering.value = true;
  recoveryConfirmation.value = undefined;
  const current = () => !disposed && action === actionGeneration
    && props.scanId === selected.scanId && props.attempt === selected.attemptNumber;
  try {
    const result = await api.recoverNeverDispatchedWebAttempt(selected.scanId, selected.attemptNumber);
    if (!current()) return;
    if (result.scanId !== selected.scanId || result.attemptNumber !== selected.attemptNumber
      || result.dispatchState !== "claimed" || result.executionState !== "submitted") {
      throw new Error(tr("恢复回执不匹配，请刷新核对；不要直接重试", "Recovery receipt mismatch; refresh and review before retrying"));
    }
    recoveryMessage.value = tr("已提交原尝试的派发；不代表仍在运行或已经完成。后续进度以执行回执为准。", "Original attempt submitted for dispatch; this is not proof of liveness or completion. Follow the execution receipts.");
  } catch (reason) {
    if (!current()) return;
    recoveryError.value = `${tr("恢复未确认；请核对当前回执，不会自动重试。", "Recovery unconfirmed; review current receipts. No automatic retry.")} ${String(reason)}`;
  } finally {
    if (current()) {
      await load();
      if (current()) recovering.value = false;
    }
  }
}
const detailLines = (detail: string) =>
  detail.split(/[；;]/).map((item) => item.trim()).filter(Boolean);
const teamRuns = computed(() => (state.value?.timeline ?? []).filter(
  (item) => item.eventType === "agent_run" && item.fromRole !== "operator",
));

async function load() {
  if (disposed) return;
  const request = ++generation;
  const scanId = props.scanId;
  const attempt = props.attempt;
  const current = () => !disposed && request === generation
    && scanId === props.scanId && attempt === props.attempt;
  activeReads.value++;
  try {
    const next = await api.getNativeScanStatus(scanId);
    if (!current()) return;
    if (next.scanId !== scanId || next.attemptNumber !== attempt) {
      state.value = undefined;
      error.value = tr("状态回执的任务或尝试编号不匹配，请刷新核对", "Status receipt task or attempt mismatch; refresh to reconcile");
      return;
    }
    state.value = next;
    error.value = "";
  } catch (reason) {
    if (!current()) return;
    error.value = String(reason);
    if (error.value.includes("administrative_closure_") || error.value.includes("closure_handoff_")) state.value = undefined;
  } finally {
    activeReads.value--;
  }
}
useCommittedRefresh("nest://collaboration-event",
  () => props.scanId ? JSON.stringify([props.scanId, props.attempt, props.status]) : "",
  payload => validCollaborationEvent(payload) && payload.scanId === props.scanId
    && payload.attemptNumber === props.attempt,
  () => activeReads.value > 0 || recovering.value || closing.value || administrativeBusy.value,
  load, unavailable => { connectionUnavailable.value = unavailable; });
watch(() => [props.scanId, props.attempt, props.status], () => {
  actionGeneration++;
  recoveryConfirmation.value = undefined;
  recovering.value = false;
  recoveryMessage.value = "";
  recoveryError.value = "";
  closureConfirmation.value = undefined;
  closing.value = false;
  closureMessage.value = "";
  closureError.value = "";
  administrativeConfirmation.value = undefined;
  administrativeBusy.value = false;
  administrativeError.value = "";
  administrativeMessage.value = "";
  state.value = undefined;
  void load();
}, { immediate: true, flush: "sync" });
onUnmounted(() => { disposed = true; generation++; actionGeneration++; });
</script>

<template>
  <section v-if="error || connectionUnavailable || state?.branches.length || state?.unresolvedContainers.length" class="native-run-status">
    <header>
      <div class="native-overview-heading"><span class="native-eyebrow">{{ tr('任务工作区', 'TASK WORKSPACE') }}</span>
        <strong>{{ tr('执行概览', 'Execution overview') }}</strong>
        <p>{{ tr('查看各分支、智能体和目标的已保存进度。', 'Saved progress across branches, agents and targets.') }}</p></div>
      <button class="button small" type="button" @click="load">{{ tr('刷新', 'Refresh') }}</button>
    </header>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <p v-if="connectionUnavailable" role="status">{{ tr('实时状态通知暂不可用，已启用定期校对；可手动刷新。', 'Live status notifications unavailable; periodic reconciliation is active. You can refresh manually.') }}</p>
    <NativeSourcePauseRecovery v-if="state" :key="`${props.scanId}:${props.attempt}`"
      :scan-id="props.scanId" :attempt="props.attempt" :status="props.status"
      :eligible="!error && state.scanId === props.scanId && state.attemptNumber === props.attempt && state.status === props.status && state.status === 'paused' && pending.some(b => b.branch === 'source')"
      :disabled="recovering || closing || administrativeBusy" @refresh="load" />
    <p v-if="recoveryError" class="form-error" role="alert">{{ recoveryError }}</p>
    <p v-if="recoveryMessage" role="status">{{ recoveryMessage }}</p>
    <p v-if="closureError" class="form-error" role="alert">{{ closureError }}</p>
    <p v-if="closureMessage" role="status">{{ closureMessage }}</p>
    <p v-if="administrativeError" class="form-error" role="alert">{{ administrativeError }}</p>
    <p v-if="administrativeMessage" role="status">{{ administrativeMessage }}</p>
    <aside v-if="state?.closureHandoff?.source || state?.closureHandoff?.successor" class="native-diagnostic" aria-label="独立任务来源关联">
      <p v-if="state.closureHandoff.source">{{ tr('本任务来自已封存任务：', 'This task follows sealed task: ') }}{{ state.closureHandoff.source.sourceScanId }} · {{ tr('原结案回执：', 'Source closure: ') }}{{ state.closureHandoff.source.closureId }}</p>
      <p v-if="state.closureHandoff.successor">{{ tr('已创建的独立交接任务：', 'Independent handoff task: ') }}{{ state.closureHandoff.successor.scanId }} · {{ state.closureHandoff.successor.scan.status }}</p>
      <p>{{ tr('关联不代表原执行结果已结清，也不表示旧请求可以重放。交接记录保留，可归档，不可删除。', 'A link does not settle source effects or permit replay. Handoff records are retained and may be archived, not deleted.') }}</p>
    </aside>
    <aside v-if="state?.administrativeClosure" class="native-diagnostic" aria-label="人工结案回执">
      <strong>{{ tr('已人工结案，不代表执行成功或费用结清', 'Administratively closed, not execution success or cost settlement') }}</strong>
      <p>{{ tr('下方保留的是原始执行状态，不表示仍在运行。旧任务不能恢复、重试或删除；后续工作必须新建独立任务并重新核对授权。', 'Original execution states below are historical, not live activity. This task cannot resume, retry or be deleted; further work requires a separately authorized new task.') }}</p>
      <code>{{ state.administrativeClosure.closureId }}</code> · {{ state.administrativeClosure.closedAt }}
      <button type="button" class="button small" @click="emit('prepare-handoff', props.scanId)">{{ tr('准备独立关联任务／找回已保存草稿', 'Prepare independent task / recover saved draft') }}</button>
    </aside>
    <button v-if="canAdministrativelyClose && !administrativeConfirmation" class="button small" type="button" @click="requestAdministrativeClosure">{{ tr('人工结案并保留未决结果', 'Close administratively, retaining unsettled results') }}</button>
    <aside v-if="administrativeConfirmation" class="native-diagnostic" aria-label="确认人工结案">
      <strong>{{ tr('封存旧任务，不释放未决预算，不重新执行', 'Seal the old task without releasing reservations or replaying execution') }}</strong>
      <p>{{ administrativeConfirmation.scanId }} · {{ tr('尝试', 'Attempt') }} {{ administrativeConfirmation.attemptNumber }}</p>
      <p>{{ tr('后端将再次确认本地执行已退出，并核对你看到的记录快照。结案仅表示不再继续旧任务；远端未知效果、原请求、未完成分支和费用占用仍需核对。此操作不能撤销。', 'The backend rechecks local worker exit and the reviewed snapshot. Closure only ends this task: unknown remote effects, requests, incomplete branches and reservations remain unresolved. This cannot be undone.') }}</p>
      <button class="button small" type="button" :disabled="administrativeBusy" @click="confirmAdministrativeClosure">{{ tr('确认人工结案／核对同一回执', 'Confirm closure / reconcile the same receipt') }}</button>
      <button class="button small" type="button" :disabled="administrativeBusy" @click="administrativeConfirmation = undefined">{{ tr('关闭确认面板', 'Dismiss confirmation') }}</button>
    </aside>
    <aside v-if="closureConfirmation" class="native-diagnostic" aria-label="确认结束未派发尝试">
      <strong>{{ tr('结束旧尝试，不启动新任务', 'Close the old attempt without starting work') }}</strong>
      <p>{{ closureConfirmation.scanId }} · {{ tr('尝试', 'Attempt') }} {{ closureConfirmation.attemptNumber }}</p>
      <p>{{ tr('仅在后端证明从未派发且无执行进度时结案。保留计划、凭据和预算，不会自动重新扫描；配置变化后需另行启动新尝试并重新通过授权检查。', 'Closure requires proof of no dispatch or execution progress. Plans, receipts and budgets are preserved. No automatic rescan; start a new attempt separately with fresh authorization checks after configuration changes.') }}</p>
      <button class="button small" type="button" :disabled="!canClose" @click="confirmClosure">{{ tr('确认结束未派发尝试', 'Confirm closure without dispatch') }}</button>
      <button class="button small" type="button" @click="closureConfirmation = undefined">{{ tr('取消', 'Cancel') }}</button>
    </aside>
    <aside v-if="recoveryConfirmation" class="native-diagnostic" aria-label="确认恢复未派发任务">
      <strong>{{ tr('仅恢复从未派发的原尝试', 'Recover only the never-dispatched original attempt') }}</strong>
      <p>{{ recoveryConfirmation.scanId }} · {{ tr('尝试', 'Attempt') }} {{ recoveryConfirmation.attemptNumber }}</p>
      <p>{{ tr('后端将重新核对启动凭据、目标、身份、预算及工具。配置发生变化会拒绝恢复；不会创建新尝试、扩大范围或启用主机测试。', 'The backend rechecks startup receipts, targets, identity, budgets and tools. Changed inputs deny recovery. This creates no new attempt, expands no scope and enables no host testing.') }}</p>
      <button class="button small" type="button" :disabled="!canRecover" @click="confirmRecovery">{{ tr('确认检查并派发', 'Confirm checks and dispatch') }}</button>
      <button class="button small" type="button" @click="recoveryConfirmation = undefined">{{ tr('取消', 'Cancel') }}</button>
    </aside>
    <template v-if="state">
      <div class="native-overview-summary">
        <span class="native-chip" :class="state.status">{{ statusLabel(state.status) }}</span>
        <dl class="native-metrics">
          <div><dt>{{ tr('尝试', 'Attempt') }}</dt><dd>{{ state.attemptNumber }}</dd></div>
          <div><dt>{{ tr('模型请求', 'Model requests') }}</dt><dd>{{ state.llmRequests || 0 }}</dd></div>
          <div><dt>Token</dt><dd>{{ state.totalTokens || 0 }}</dd></div>
          <div><dt>{{ tr('智能体记录', 'Agent records') }}</dt><dd>{{ teamRuns.length }}</dd></div>
        </dl>
      </div>
      <p v-if="state.status === 'paused' && !state.administrativeClosure" class="native-retained-note" role="status">{{ tr('任务已暂停；下方智能体状态是保留的执行记录，不表示仍在工作。', 'Task paused. Agent states below are retained execution records, not evidence of current activity.') }}</p>
      <aside class="native-diagnostic" :class="state.stopDiagnostic.category" aria-label="运行诊断">
        <strong>{{ diagnosticLabel(state.stopDiagnostic.code) }}</strong>
        <p>{{ actionLabel(state.stopDiagnostic.nextAction) }}。{{ tr('不会仅凭这份诊断自动续跑。', 'This diagnostic does not automatically resume the task.') }}</p>
        <details v-if="state.stopDiagnostic.stage"><summary>{{ tr('诊断记录', 'Diagnostic record') }}</summary><p>{{ tr('阶段', 'Stage') }} · {{ state.stopDiagnostic.stage }}</p></details>
        <details v-if="state.stopDiagnostic.obligations.length">
          <summary>{{ tr('未完成或待核对项目', 'Outstanding or review items') }} · {{ state.stopDiagnostic.obligations.length }}{{ state.stopDiagnostic.obligationsTruncated ? '+' : '' }}</summary>
          <ul><li v-for="(item, index) in state.stopDiagnostic.obligations" :key="`${item.kind}-${item.reference}-${index}`">{{ obligationLabel(item.kind) }} · {{ item.reference }} · {{ item.status }}</li></ul>
        </details>
      </aside>
      <p v-if="pending.length && ['scanning','pausing'].includes(state.status)" class="native-waiting">
        {{ tr('只有全部分支结束，任务才能给出最终状态。仍在等待：', 'Final status requires every branch to finish. Waiting for: ') }}
        {{ pending.map(b => b.branch === 'source' ? tr('源码分析', 'Source analysis') : tr('Web 调查', 'Web investigation')).join('、') }}
      </p>
      <h3 class="native-section-heading">{{ tr('执行分支', 'Branches') }} <span>{{ state.branches.length }}</span></h3>
      <div class="native-branch-grid">
        <article v-for="branch in state.branches" :key="branch.branch" class="native-card">
          <div class="native-card-head">
            <strong>{{ branch.branch === 'source' ? tr('源码分析', 'Source analysis') : tr('Web 调查', 'Web investigation') }}</strong>
            <span class="native-chip" :class="branch.status">{{ branchStatusLabel(branch) }}</span>
          </div>
          <p>{{ branch.checkpoint || tr('尚无完成回执', 'No completion receipt yet') }}</p>
          <p class="native-dispatch-receipt">{{ branchDispatchDescription(branch) }}</p>
          <button v-if="branch.branch === 'web' && canRecover && !recoveryConfirmation" class="button small" type="button" @click="requestRecovery">{{ tr('检查并恢复未派发任务', 'Check and recover never-dispatched task') }}</button>
          <button v-if="branch.branch === 'web' && canClose && !closureConfirmation" class="button small" type="button" @click="requestClosure">{{ tr('结束未派发尝试', 'Close never-dispatched attempt') }}</button>
          <p v-if="branch.branch === 'web' && closing" role="status">{{ tr('正在核对并结案；不会启动扫描', 'Checking and closing; no scan will start') }}</p>
          <p v-if="branch.branch === 'web' && recovering" role="status">{{ tr('正在核对并提交；请勿重复操作', 'Checking and submitting; do not repeat') }}</p>
          <small class="native-card-updated">{{ tr('更新', 'Updated') }} · {{ branch.updatedAt || '—' }}</small>
        </article>
      </div>
      <h3 v-if="teamRuns.length" class="native-section-heading">{{ tr('智能体协作', 'Agent team') }} <span>{{ teamRuns.length }}</span></h3>
      <div v-if="teamRuns.length" class="native-team-grid">
        <article v-for="(run, index) in teamRuns" :key="`${run.fromRole}-${run.id}-${index}`" class="native-card">
          <div class="native-card-head">
            <strong>{{ roleLabel(run.fromRole) }}</strong>
            <span class="native-chip" :class="run.status"><span v-if="state.status === 'paused' || state.administrativeClosure">{{ tr('原执行状态：', 'Recorded state: ') }}</span>{{ statusLabel(run.status) }}</span>
          </div>
          <p>{{ run.summary || run.messageKind }}</p>
        </article>
      </div>
      <NativeBudgetDiagnostics :key="`${props.scanId}:${props.attempt}`" :scan-id="props.scanId" :attempt="props.attempt" />
      <h3 v-if="state.targets?.length" class="native-section-heading">{{ tr('目标进度', 'Targets') }} <span>{{ state.targets.length }}</span></h3>
      <div v-if="state.targets?.length" class="native-target-list">
        <article v-for="target in state.targets" :key="target.url" class="native-card">
          <div class="native-card-head">
            <code>{{ target.url }}</code>
            <span class="native-chip" :class="target.status">{{ statusLabel(target.status) }}</span>
          </div>
          <ul v-if="detailLines(target.detail).length">
            <li v-for="line in detailLines(target.detail)" :key="line">{{ line }}</li>
          </ul>
          <p v-else>{{ tr('没有逐项说明', 'No per-target detail') }}</p>
        </article>
      </div>
      <details v-if="state.sourceGaps.length" class="native-gaps">
        <summary>{{ tr('源码覆盖缺口', 'Source coverage gaps') }} · {{ state.sourceGaps.length }}</summary>
        <ul><li v-for="gap in state.sourceGaps" :key="gap">{{ gap }}</li></ul>
      </details>
      <aside v-if="state.unresolvedContainers.length" role="alert" class="native-cleanup-warning">
        <strong>{{ tr('容器清理尚未确认，源码分析重试已阻止', 'Container cleanup is unconfirmed; source retries are blocked') }}</strong>
        <p>{{ tr('请核对下面的所有权回执。不要清空数据库记录或执行全局 Docker 清理来绕过。', 'Inspect these ownership receipts. Do not bypass the block by deleting receipts or pruning all Docker containers.') }}</p>
        <details v-for="receipt in state.unresolvedContainers" :key="receipt.receiptId">
          <summary>{{ receipt.purpose }} · {{ receipt.status }}</summary>
          <p>{{ receipt.containerName }} · {{ receipt.receiptId }}</p><p>{{ receipt.detail }}</p>
        </details>
      </aside>
    </template>
  </section>
  <NativeRootReconciliation v-if="props.scanId" :key="`cost:${props.scanId}`" :scan-id="props.scanId" :attempt="props.attempt" />
  <NativeProtectionRecovery v-if="props.scanId" :key="`recovery:${props.scanId}`" :scan-id="props.scanId" />
</template>

<style scoped src="./nativeRunStatus.css"></style>
