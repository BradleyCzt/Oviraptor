<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { sentinelApi } from "../api";
import type { NativeBudgetDiagnostics } from "../types";
import { useI18n } from "../../../i18n";
import { useCommittedRefresh } from "../../../composables/useCommittedRefresh";
import { validCollaborationEvent } from "../timeline/eventContract";
import NativeBudgetVector from "./NativeBudgetVector.vue";

const props = defineProps<{ scanId: string; attempt: number }>();
const { tr } = useI18n();
const opened = ref(false);
const loading = ref(false);
const report = ref<NativeBudgetDiagnostics>();
const error = ref("");
const connectionUnavailable = ref(false);
let generation = 0, disposed = false;
const validCount = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

function validSingleHistory(value: unknown, root: string): boolean {
  if (value == null) return true;
  if (typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return v.schemaVersion === 1 && v.rootRunId === root
    && ['verified_closed_history', 'not_verified'].includes(v.state as string)
    && v.authoritative === false && v.executionAllowed === false && v.targetRequestsGranted === 0
    && v.basis === 'original_runtime_records_not_provider_dollar_invoice';
}

function validSupervisionCapacity(value: unknown): boolean {
  if (value == null) return true;
  if (typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return v.schemaVersion === 1 && v.authoritative === false && v.executionAllowed === false
    && validCount(v.pendingPhysicalRequests) && v.minimumFollowupModelRequests === 6
    && validCount(v.nextHttpRequiredModelRequests)
    && v.nextHttpRequiredModelRequests === (v.pendingPhysicalRequests as number) + 7
    && (v.remainingSharedModelRequests === null || validCount(v.remainingSharedModelRequests))
    && (v.remainingSharedModelTokens === null || validCount(v.remainingSharedModelTokens))
    && typeof v.nextHttpRequestBalanceSufficient === 'boolean'
    && v.nextHttpRequestBalanceSufficient === (v.remainingSharedModelRequests === null
      || (v.remainingSharedModelRequests as number) >= (v.nextHttpRequiredModelRequests as number))
    && (v.freshWorkBlocked === null || ['cost_reconciliation_required', 'wall_time_exhausted', 'elapsed_finalized'].includes(v.freshWorkBlocked as string))
    && v.futureTokenCapacityGuaranteed === false && v.usdCapacityCoverage === 'not_enforced_by_this_ledger';
}

function supervisionBlockReason(reason: string): string {
  if (reason === 'cost_reconciliation_required')
    return tr('原费用尚未完成对账。', 'Original costs still require reconciliation.');
  if (reason === 'wall_time_exhausted')
    return tr('原任务截止时间已到。', 'The original task deadline has elapsed.');
  return tr('原耗时账目已封存。', 'The original elapsed-time record is finalized.');
}

function validJournal(rows: unknown): boolean {
  if (rows == null) return true;
  const dimensions = ['model_input_tokens', 'model_cached_tokens', 'model_output_tokens', 'model_requests',
    'target_requests', 'browser_actions', 'controlled_writes', 'upload_bytes', 'concurrency_batches', 'wall_time_ms'];
  const scopes = ['assignment_settlement', 'multi_agent_broker', 'multi_agent_lane', 'race_batch_dispatch_unimplemented', 'admission_samples', 'denied_by_contract'];
  return Array.isArray(rows) && rows.length === dimensions.length
    && new Set(rows.map(row => row?.dimension)).size === dimensions.length
    && rows.every(row => row && dimensions.includes(row.dimension) && scopes.includes(row.coverage)
      && (row.hardLimit === null || validCount(row.hardLimit))
      && validCount(row.reserved) && validCount(row.consumed) && validCount(row.indeterminate));
}

async function load() {
  if (!opened.value || disposed) return;
  const request = ++generation, scanId = props.scanId, attempt = props.attempt;
  loading.value = true;
  try {
    const next = await sentinelApi.getNativeBudgetDiagnostics(scanId, attempt);
    if (disposed || !opened.value || request !== generation || scanId !== props.scanId || attempt !== props.attempt) return;
    if (next.schema !== "summary_gap_v1" || next.authoritative !== false || !Array.isArray(next.roots))
      throw new Error("budget_diagnostic_receipt_mismatch");
    if (!next.roots.every(root => root?.gaps && validSingleHistory(root.singleOriginalHistory, root.rootRunId)
      && validJournal(root.gaps.appendJournal)
      && validSupervisionCapacity(root.gaps.supervisionRequestCapacity)
      && (root.gaps.unclosedWebModelCalls === undefined || validCount(root.gaps.unclosedWebModelCalls))))
      throw new Error("budget_journal_receipt_mismatch");
    report.value = next;
    error.value = "";
  } catch {
    // Database and transport errors can contain local paths or credentials.
    if (!disposed && opened.value && request === generation) {
      report.value = undefined;
      error.value = tr('预算盘点读取失败，请重试。', 'Budget inventory unavailable; please retry.');
    }
  } finally {
    if (!disposed && request === generation) loading.value = false;
  }
}

function onToggle(event: Event) {
  opened.value = (event.target as HTMLDetailsElement).open;
  if (!opened.value) { generation++; loading.value = false; report.value = undefined; error.value = ""; }
}

useCommittedRefresh("nest://collaboration-event",
  () => opened.value ? JSON.stringify([props.scanId, props.attempt]) : "",
  payload => validCollaborationEvent(payload) && payload.scanId === props.scanId
    && payload.attemptNumber === props.attempt,
  () => loading.value, load, unavailable => { connectionUnavailable.value = unavailable; });
onUnmounted(() => { disposed = true; generation++; });
</script>

<template>
  <details class="native-budget-diagnostics" @toggle="onToggle">
    <summary>{{ tr('预算盘点（按需只读）', 'Budget inventory (on-demand, read-only)') }}</summary>
    <p v-if="loading" role="status">{{ tr('正在读取当前轮次…', 'Reading the current attempt…') }}</p>
    <p v-if="connectionUnavailable" role="status">{{ tr('实时事件连接不可用，展开期间仍会低频对账。', 'Live events unavailable; low-frequency reconciliation remains active while open.') }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <template v-if="report">
    <p v-if="!report.roots.length">{{ tr('当前轮次没有 Native Coordinator root。', 'No Native Coordinator roots in this attempt.') }}</p>
    <p>{{ tr('本区只读展示旧汇总与已接入的追加账目，不会自动扣费、退款、继续或终止任务。',
      'This view reads the summary and integrated journal entries; it never charges, refunds, resumes or stops a task.') }}</p>
    <p v-if="report.truncated" role="status">{{ tr('共', 'Total') }} {{ report.totalRoots }} · {{ tr('仅显示前 50 个 root；其余未在本页盘点。', 'Showing only the first 50 roots; the rest are not listed here.') }}</p>
    <article v-for="root in report.roots" :key="root.rootRunId">
      <strong><code>{{ root.rootRunId }}</code></strong>
      <div v-if="root.singleOriginalHistory">
        <p>{{ root.singleOriginalHistory.state === 'verified_closed_history'
          ? tr('Single 原账目与原退出记录已核验闭合。', 'Single original costs and exit records form a verified closed history.')
          : tr('Single 原账目尚未核验闭合；正在运行、未决费用或缺失/冲突记录都可能导致此状态。', 'Single original history is not verified as closed; active work, unresolved costs or missing/conflicting records may cause this state.') }}</p>
        <p>{{ tr('仅核验原运行记录，不是真实美元账单；不授予恢复或新的请求额度。',
          'Only original runtime records are audited; this is not a provider dollar invoice and grants no resume permission or new request allowance.') }}</p>
      </div>
      <p>{{ tr('旧汇总预算行', 'Legacy budget summary row') }}: {{ root.gaps.ledgerExists ? tr('存在', 'Present') : tr('缺失', 'Missing') }} ·
        {{ tr('旧 Coordinator 租约匹配', 'Legacy Coordinator lease matches') }}: {{ root.singleOriginalHistory ? tr('不适用（Single）', 'Not applicable (Single)') : root.gaps.coordinatorLeaseFound && root.gaps.fencingMatchesCoordinator ? tr('是', 'Yes') : tr('否/未知', 'No/unknown') }}</p>
      <p>{{ tr('旧汇总 Token 已用/预留/未结算 assignment/差额', 'Legacy summary tokens spent/reserved/unsettled assignments/delta') }}:
        {{ root.gaps.spentTokens }} / {{ root.gaps.reservedTokens }} / {{ root.gaps.unsettledAssignmentTokens }} / {{ root.gaps.reservationTokenDelta ?? '—' }}</p>
      <p>{{ tr('旧汇总模型请求 已用/预留/未结算 assignment/差额', 'Legacy summary model requests spent/reserved/unsettled assignments/delta') }}:
        {{ root.gaps.spentRequests }} / {{ root.gaps.reservedRequests }} / {{ root.gaps.unsettledAssignmentRequests }} / {{ root.gaps.reservationRequestDelta ?? '—' }}</p>
      <p>{{ tr('结果未定的工具调用', 'Indeterminate tool invocations') }}: {{ root.gaps.indeterminateInvocations }} ·
        {{ tr('无合同键调用', 'Invocations without a contract key') }}: {{ root.gaps.unkeyedInvocations }}</p>
      <p>{{ tr('Web 模型待核实调用', 'Unclosed Web model calls') }}: {{ root.gaps.unclosedWebModelCalls ?? '—' }} ·
        {{ tr('尚无匹配的已收到/未发出回执；不得自动重发或退款。已收到但费用为估算的调用另见下方未决账目。',
          'No matching received/unsent receipt; automatic resend or refund is blocked. Estimated received costs appear in the journal below.') }}</p>
      <NativeBudgetVector v-if="root.gaps.appendJournal" :dimensions="root.gaps.appendJournal" />
      <p v-else>{{ tr('此根任务没有追加账目；保留原记录，未自动补造历史费用。', 'This root has no journal entries; historical costs were not synthesized.') }}</p>
      <div v-if="root.gaps.supervisionRequestCapacity">
        <p>{{ tr('待独立付费监督的物理请求', 'Physical requests awaiting independently paid supervision') }}:
          {{ root.gaps.supervisionRequestCapacity.pendingPhysicalRequests }} ·
          {{ tr('下一条 HTTP 所需模型请求容量（含后续最小 6 次）', 'Model request capacity needed for the next HTTP (including at least 6 follow-up calls)') }}:
          {{ root.gaps.supervisionRequestCapacity.nextHttpRequiredModelRequests }}</p>
        <p>{{ tr('当前共享模型请求 / Token 余额', 'Current shared model request / token balance') }}:
          {{ root.gaps.supervisionRequestCapacity.remainingSharedModelRequests ?? tr('不限额', 'Unlimited') }} /
          {{ root.gaps.supervisionRequestCapacity.remainingSharedModelTokens ?? tr('不限额', 'Unlimited') }} ·
          {{ root.gaps.supervisionRequestCapacity.nextHttpRequestBalanceSufficient ? tr('请求次数余额足够', 'Request count balance is sufficient') : tr('请求次数余额不足', 'Request count balance is insufficient') }}</p>
        <p v-if="root.gaps.supervisionRequestCapacity.freshWorkBlocked" role="status">{{ supervisionBlockReason(root.gaps.supervisionRequestCapacity.freshWorkBlocked) }} {{ tr('当前费用或时间门禁阻止新增工作；余额足够也不能继续。', 'Cost or time admission currently blocks new work even when the balance is sufficient.') }}</p>
        <p>{{ tr('容量盘点只读，不授予执行权限，不是财务预留；未来 Token 容量尚未保证，此账本未执行美元容量门禁。',
          'This read-only inventory grants no execution permission and is not a financial reservation; future token capacity is not guaranteed and this ledger does not enforce a USD capacity gate.') }}</p>
      </div>
    </article>
    <p v-if="report.roots.length">{{ tr('旧汇总无法区分的维度', 'Dimensions not separated by the summary') }}: {{ report.roots[0]?.gaps.missingDimensions.join(', ') }}</p>
    </template>
  </details>
</template>

<style scoped>
.native-budget-diagnostics { margin: 12px 0; padding: 12px; border: 1px solid var(--line, #e3e5e9); border-radius: 9px; background: var(--app-surface, #fff); color: var(--app-ink, #25282d); font-size: 12px; overflow-wrap: anywhere; }
.native-budget-diagnostics article { padding: 8px 0; border-top: 1px solid var(--line, #e3e5e9); }
.native-budget-diagnostics p { margin: 6px 0; line-height: 1.55; color: var(--app-muted, #717780); }
summary { cursor: pointer; }
</style>
