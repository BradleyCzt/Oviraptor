<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { rootCostReconciliationApi } from "../reconciliation/api";
import { useI18n } from "../../../i18n";
import { receiptMatchesPreview, validRootCostInventory } from "../reconciliation/contract";
import type { RootCostInventory, RootCostPreview, RootCostReceipt } from "../reconciliation/contract";
import { validRootContinuationPreview } from "../reconciliation/continuation";
import type { RootContinuationPreview, RootContinuationProposal } from "../reconciliation/continuation";
import { validRootContinuationIssuance } from "../reconciliation/issuance";
import type { RootContinuationIssuance } from "../reconciliation/issuance";
import { validRootContinuationDiscovery } from "../reconciliation/discovery";

const props = defineProps<{ scanId: string; attempt: number }>();
const { tr } = useI18n();
const opened = ref(false), loading = ref(false), confirming = ref(false);
const inventory = ref<RootCostInventory>(), selected = ref<RootCostPreview>();
const error = ref(""), message = ref("");
const continuation = ref<RootContinuationPreview>(), continuationLoading = ref(false), continuationError = ref("");
const approvalChecked = ref(false), approveAttempted = ref(false), approving = ref(false), statusLoading = ref(false);
const approvedProposal = ref<RootContinuationProposal>(), issuance = ref<RootContinuationIssuance>(), issuanceError = ref("");
const discoveryLoading = ref(false), discoveryMessage = ref(""), discoveryError = ref(""), recoveredOperation = ref(false);
const recoveredReceipt = ref<RootCostReceipt>();
let discoveryGeneration = 0, operationGeneration = 0;
let generation = 0, disposed = false;
const operationScope = (request: number, proposal: RootContinuationProposal) => !disposed && opened.value
  && request === operationGeneration && approveAttempted.value && approvedProposal.value?.proposalHash === proposal.proposalHash
  && props.scanId === proposal.scanId && [proposal.attemptNumber, proposal.proposedAttemptNumber].includes(props.attempt);
const currentScope = (request: number, scanId: string, attempt: number) => !disposed && opened.value
  && request === generation && scanId === props.scanId && attempt === props.attempt;

async function load() {
  if (!opened.value || loading.value || confirming.value || continuationLoading.value || approving.value || statusLoading.value || discoveryLoading.value || disposed) return;
  const request = ++generation, scanId = props.scanId, attempt = props.attempt;
  loading.value = true; error.value = "";
  if (!approveAttempted.value) { continuation.value = undefined; continuationError.value = ""; approvalChecked.value = false; }
  try {
    const report = await rootCostReconciliationApi.getNativeRootCostReconciliations(scanId, attempt);
    if (!currentScope(request, scanId, attempt)) return;
    if (!validRootCostInventory(report, scanId, attempt)) throw Error("root_cost_inventory_mismatch");
    inventory.value = report;
    if (selected.value && !report.items.some(item => item.confirmationHash === selected.value?.confirmationHash))
      selected.value = undefined;
  } catch {
    if (currentScope(request, scanId, attempt)) {
      inventory.value = undefined; selected.value = undefined;
      error.value = tr("原费用读取未确认，请刷新核对。", "Original cost inventory unconfirmed; refresh and review.");
    }
  } finally {
    if (request === generation && !disposed) loading.value = false;
  }
}
function select(item: RootCostPreview) {
  if (loading.value || confirming.value || continuationLoading.value || approving.value || statusLoading.value || discoveryLoading.value || !inventory.value?.items.includes(item)) return;
  selected.value = item; error.value = ""; message.value = "";
}
async function confirm() {
  const item = selected.value;
  if (!item || confirming.value || loading.value || continuationLoading.value || approving.value || statusLoading.value || discoveryLoading.value || disposed || !opened.value
    || item.scanId !== props.scanId || item.attemptNumber !== props.attempt
    || !inventory.value?.items.some(row => row.confirmationHash === item.confirmationHash)) return;
  const request = ++generation;
  confirming.value = true; error.value = ""; message.value = "";
  try {
    const receipt = await rootCostReconciliationApi.confirmNativeRootCostReconciliation(item.scanId, item.attemptNumber,
      item.rootRunId, item.callId, item.confirmationHash);
    if (!currentScope(request, item.scanId, item.attemptNumber)) return;
    if (!receiptMatchesPreview(receipt, item)) throw Error("root_cost_confirmation_receipt_mismatch");
    const report = inventory.value;
    if (!report) return;
    inventory.value = { ...report, items: report.items.filter(row => row.callId !== item.callId),
      receipts: [...report.receipts.filter(row => row.callId !== item.callId), receipt] };
    selected.value = undefined;
    message.value = tr("原费用已精确对账。此操作不恢复执行；继续新工作仍需独立核验授权和预算。",
      "Original costs reconciled precisely. Execution is not resumed; new work requires independent authorization and budget checks.");
  } catch {
    if (currentScope(request, item.scanId, item.attemptNumber)) error.value = tr(
      "费用确认结果尚未核实。可刷新读取原回执，或显式重试同一账单；不会自动重发请求。",
      "Cost confirmation remains unverified. Refresh to read the original receipt, or explicitly retry this bill; requests are never resent automatically.");
  } finally {
    if (request === generation && !disposed) confirming.value = false;
  }
}
async function previewContinuation(receipt: RootCostReceipt) {
  if (disposed || !opened.value || loading.value || confirming.value || continuationLoading.value || discoveryLoading.value || approveAttempted.value
    || receipt.scanId !== props.scanId || receipt.attemptNumber !== props.attempt
    || !inventory.value?.receipts.includes(receipt)) return;
  const request = ++generation;
  approvalChecked.value = false;
  continuationLoading.value = true; continuation.value = undefined; continuationError.value = "";
  try {
    const preview = await rootCostReconciliationApi.previewNativeRootFinancialContinuation(receipt.scanId, receipt.attemptNumber,
      receipt.rootRunId, receipt.receiptId);
    if (!currentScope(request, receipt.scanId, receipt.attemptNumber)) return;
    if (!validRootContinuationPreview(preview, receipt)) throw Error("financial_continuation_preview_mismatch");
    continuation.value = preview;
  } catch {
    if (currentScope(request, receipt.scanId, receipt.attemptNumber)) continuationError.value = tr(
      "剩余授权预览未通过核验。原费用回执保留；未创建新工作或恢复执行。",
      "Remaining authorization preview failed verification. The original cost receipt remains; no new work or execution was started.");
  } finally {
    if (request === generation && !disposed) continuationLoading.value = false;
  }
}
async function approveContinuation() {
  const proposal = approvedProposal.value || continuation.value;
  if (!proposal || !approvalChecked.value || disposed || !opened.value || approving.value || statusLoading.value
    || loading.value || confirming.value || continuationLoading.value || discoveryLoading.value || proposal.historicalRootsChecked !== 1
    || props.scanId !== proposal.scanId || ![proposal.attemptNumber, proposal.proposedAttemptNumber].includes(props.attempt)
    || !approveAttempted.value && props.attempt !== proposal.attemptNumber
    || issuance.value && (!issuance.value.workerStartAllowed || issuance.value.status === "review_required" || issuance.value.attemptClosed)) return;
  if (!approveAttempted.value) approvedProposal.value = JSON.parse(JSON.stringify(proposal)) as RootContinuationProposal;
  const bound = approvedProposal.value!;
  approveAttempted.value = true; approving.value = true; issuanceError.value = "";
  const request = ++operationGeneration;
  try {
    const result = await rootCostReconciliationApi.approveNativeRootFinancialContinuation(bound.scanId, bound.attemptNumber,
      bound.rootRunId, bound.receiptId, bound.proposalHash);
    if (!operationScope(request, bound)) return;
    if (!validRootContinuationIssuance(result, bound, issuance.value)) throw Error("financial_issuance_mismatch");
    issuance.value = result;
  } catch {
    if (operationScope(request, bound)) issuanceError.value = tr(
      "批准响应尚未核实，请先只读查询同一批准回执；不会自动重试或重发 SDK。",
      "Approval response remains unverified. Read the same approval receipt first; no automatic retries or SDK resends.");
  } finally { if (request === operationGeneration && !disposed) approving.value = false; }
}
async function readContinuationStatus() {
  const bound = approvedProposal.value;
  if (!bound || approving.value || statusLoading.value || discoveryLoading.value || !operationScope(operationGeneration, bound)) return;
  const request = ++operationGeneration; statusLoading.value = true; issuanceError.value = "";
  try {
    const result = await rootCostReconciliationApi.getNativeRootFinancialContinuation(bound.scanId, bound.attemptNumber,
      bound.rootRunId, bound.receiptId, bound.proposalHash);
    if (!operationScope(request, bound)) return;
    if (!validRootContinuationIssuance(result, bound, issuance.value)) throw Error("financial_status_mismatch");
    issuance.value = result;
  } catch {
    if (operationScope(request, bound)) issuanceError.value = tr("批准状态仍未核实；保留原绑定，仅可显式查询或重试同一批准。",
      "Approval status remains unverified; the original binding is retained for explicit reads or retries only.");
  } finally { if (request === operationGeneration && !disposed) statusLoading.value = false; }
}
async function discoverContinuation() {
  if (!opened.value || disposed || discoveryLoading.value || loading.value || confirming.value
    || continuationLoading.value || approving.value || statusLoading.value) return;
  const request = ++discoveryGeneration, scanId = props.scanId, attempt = props.attempt, operation = operationGeneration;
  const current = () => !disposed && opened.value && request === discoveryGeneration
    && scanId === props.scanId && attempt === props.attempt && operation === operationGeneration;
  discoveryLoading.value = true; discoveryMessage.value = ""; discoveryError.value = "";
  try {
    const report = await rootCostReconciliationApi.discoverNativeRootFinancialContinuation(scanId, attempt);
    if (!current()) return;
    if (!validRootContinuationDiscovery(report, scanId, attempt, issuance.value, approvedProposal.value)
      || report.issued && recoveredReceipt.value && !receiptMatchesPreview(report.issued.originalReceipt, recoveredReceipt.value))
      throw Error("financial_discovery_mismatch");
    if (!report.issued) {
      discoveryMessage.value = tr("未找到该尝试签发记录。", "No issuance record found for this attempt.");
      return;
    }
    approvedProposal.value = JSON.parse(JSON.stringify(report.issued.proposal)) as RootContinuationProposal;
    issuance.value = report.issued.status; recoveredReceipt.value = report.issued.originalReceipt;
    approveAttempted.value = true; recoveredOperation.value = true;
    approvalChecked.value = false; issuanceError.value = ""; operationGeneration++;
    discoveryMessage.value = tr("已只读找回原批准回执；没有启动工作或重新批准。", "Original approval receipt recovered read-only; no work started or approval repeated.");
  } catch {
    if (current()) discoveryError.value = tr("已批准评估回执未通过核验；原记录与当前绑定保留，没有执行任何工作。",
      "Approval receipt discovery failed verification; original records and the current binding remain, without execution.");
  } finally { if (request === discoveryGeneration && !disposed) discoveryLoading.value = false; }
}
function resetDiscovery() {
  discoveryGeneration++; discoveryLoading.value = false; discoveryMessage.value = ""; discoveryError.value = "";
}
function resetApproval() {
  operationGeneration++; approvedProposal.value = undefined; issuance.value = undefined; issuanceError.value = "";
  approvalChecked.value = false; approveAttempted.value = false; approving.value = false; statusLoading.value = false;
  recoveredOperation.value = false; recoveredReceipt.value = undefined;
}
const dimensionLabels = [
  ['模型输入 tokens', 'Model input tokens'], ['模型缓存 tokens', 'Model cached tokens'], ['模型输出 tokens', 'Model output tokens'],
  ['模型请求', 'Model requests'], ['目标请求', 'Target requests'], ['浏览器动作', 'Browser actions'],
  ['受控写入', 'Controlled writes'], ['上传字节', 'Upload bytes'], ['并发批次', 'Concurrency batches'], ['账本时间毫秒', 'Journal wall time ms'],
];
const remainingLabel = (value: number | null) => value === null
  ? tr('原合同未设硬顶', 'Original contract has no hard ceiling') : String(value);
function resetContinuation() {
  continuation.value = undefined; continuationLoading.value = false; continuationError.value = "";
}
function onToggle(event: Event) {
  opened.value = (event.target as HTMLDetailsElement).open;
  if (opened.value) { void load(); return; }
  generation++; loading.value = false; confirming.value = false;
  inventory.value = undefined; selected.value = undefined; error.value = ""; message.value = "";
  resetContinuation(); resetApproval(); resetDiscovery();
}
watch(() => [props.scanId, props.attempt], () => {
  const bound = approvedProposal.value;
  const keepOperation = approveAttempted.value && bound?.scanId === props.scanId
    && [bound.attemptNumber, bound.proposedAttemptNumber].includes(props.attempt);
  resetDiscovery();
  generation++; inventory.value = undefined; selected.value = undefined;
  loading.value = false; confirming.value = false; error.value = ""; message.value = "";
  if (!keepOperation) { resetContinuation(); resetApproval(); }
}, { flush: "sync" });
onUnmounted(() => { disposed = true; generation++; resetApproval(); resetDiscovery(); });
</script>

<template>
  <details class="native-root-reconciliation" @toggle="onToggle">
    <summary>{{ tr('原费用对账（需显式确认）', 'Original costs (explicit confirmation required)') }}</summary>
    <p>{{ tr('核对原账单后，只结算已经收到的真实费用。费用确认不会续跑任务或重发原调用。',
      'Review the original bill to settle received actual costs. Confirmation never resumes the task or resends its original call.') }}</p>
    <button type="button" class="button small" :disabled="loading || confirming || continuationLoading || approving || statusLoading || discoveryLoading" @click="load">{{ tr('刷新原账单和回执', 'Refresh original bills and receipts') }}</button>
    <button type="button" class="button small" :disabled="loading || confirming || continuationLoading || approving || statusLoading || discoveryLoading" @click="discoverContinuation">{{ tr('读取已批准评估回执', 'Read previously approved assessment receipt') }}</button>
    <p v-if="discoveryLoading" role="status">{{ tr('正在只读找回签发记录…', 'Recovering issuance records read-only…') }}</p>
    <p v-if="discoveryMessage" role="status">{{ discoveryMessage }}</p>
    <p v-if="discoveryError" role="alert">{{ discoveryError }}</p>
    <p v-if="loading" role="status">{{ tr('正在读取原费用…', 'Reading original costs…') }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-if="message" role="status">{{ message }}</p>
    <template v-if="inventory">
      <p v-if="inventory.truncated" role="status">{{ tr('优先显示待核对原账单；确认本页费用后可刷新查看后续账单。仅显示部分记录，未列出的账单和回执仍保留，不能据此认为全部费用已结清。', 'Unconfirmed original bills are shown first. After confirming this page, refresh to review later bills. Only part of the records are shown; unlisted bills and receipts remain, and this view does not prove all costs are settled.') }}</p>
      <p v-if="!inventory.items.length && !inventory.blocked.length && !inventory.receipts.length">{{ tr('此尝试没有可确认的 Root 原费用。', 'No original Root costs to confirm in this attempt.') }}</p>
      <article v-for="item in inventory.items" :key="item.callId">
        <strong>{{ tr('待核对原账单', 'Original bill awaiting review') }}</strong>
        <p>{{ item.scanId }} · {{ tr('尝试', 'Attempt') }} {{ item.attemptNumber }} · Root <code>{{ item.rootRunId }}</code> · {{ tr('轮次', 'Round') }} {{ item.round }}</p>
        <p>{{ tr('原调用', 'Original call') }}: <code>{{ item.callId }}</code> · {{ tr('原租约尝试', 'Original lease attempt') }}: <code>{{ item.leaseAttemptId }}</code></p>
        <p>{{ tr('供应商实际用量', 'Supplier actual usage') }}: {{ tr('输入', 'Input') }} {{ item.usage.inputTokens }} · {{ tr('缓存', 'Cached') }} {{ item.usage.cachedInputTokens }} · {{ tr('输出', 'Output') }} {{ item.usage.outputTokens }} · {{ tr('总计', 'Total') }} {{ item.usage.totalTokens }} · {{ tr('模型请求', 'Model requests') }} {{ item.usage.modelRequests }}</p>
        <p>{{ tr('原请求哈希', 'Original request hash') }}: <code>{{ item.requestHash }}</code></p>
        <p>{{ tr('冻结依据哈希', 'Frozen basis hash') }}: <code>{{ item.basisHash }}</code></p>
        <p>{{ tr('账单哈希', 'Invoice hash') }}: <code>{{ item.invoiceHash }}</code></p>
        <p>{{ tr('确认绑定哈希', 'Confirmation binding hash') }}: <code>{{ item.confirmationHash }}</code></p>
        <p>{{ item.operation === 'settle_received' ? tr('结算已收到的账单', 'Settle received bill') : tr('将匹配的原未决费用转为实际已用', 'Reconcile matching original unresolved costs') }}</p>
        <button type="button" class="button small" :disabled="loading || confirming" @click="select(item)">{{ tr('核对这张原账单', 'Review this original bill') }}</button>
      </article>
      <section v-if="selected" class="root-cost-confirmation" role="group" :aria-label="tr('确认原费用', 'Confirm original costs')">
        <strong>{{ tr('确认上述实际费用属于这次原调用？', 'Confirm these actual costs belong to this original call?') }}</strong>
        <p>{{ selected.scanId }} · {{ tr('尝试', 'Attempt') }} {{ selected.attemptNumber }} · <code>{{ selected.callId }}</code></p>
        <p>{{ tr('确认绑定哈希', 'Confirmation binding hash') }}: <code>{{ selected.confirmationHash }}</code></p>
        <button type="button" class="button small" :disabled="loading || confirming" @click="confirm">{{ confirming ? tr('正在确认…', 'Confirming…') : tr('确认原费用对账', 'Confirm original cost reconciliation') }}</button>
        <button type="button" class="button ghost small" :disabled="confirming" @click="selected = undefined">{{ tr('取消', 'Cancel') }}</button>
      </section>
      <article v-for="item in inventory.blocked" :key="item.callId" class="root-cost-blocked">
        <strong>{{ tr('原费用仍未解决', 'Original costs remain unresolved') }}</strong>
        <p>Root <code>{{ item.rootRunId }}</code> · <code>{{ item.callId }}</code></p>
        <p>{{ tr('原证据不足、费用不匹配或预算不允许确认。原记录和未决费用保持，不可据此恢复执行。',
          'Original evidence, cost matching or budget checks do not permit confirmation. Original records and unresolved costs remain; execution is not enabled.') }}</p>
      </article>
      <article v-for="receipt in inventory.receipts" :key="receipt.receiptId" class="root-cost-receipt">
        <strong>{{ tr('原费用已确认', 'Original actual costs confirmed') }}</strong>
        <p>{{ receipt.scanId }} · {{ tr('尝试', 'Attempt') }} {{ receipt.attemptNumber }} · Root <code>{{ receipt.rootRunId }}</code> · <code>{{ receipt.callId }}</code></p>
        <p>{{ tr('不可变对账回执', 'Immutable reconciliation receipt') }}: <code>{{ receipt.receiptId }}</code></p>
        <p>{{ tr('原租约尝试', 'Original lease attempt') }}: <code>{{ receipt.leaseAttemptId }}</code> · {{ tr('轮次', 'Round') }} {{ receipt.round }}</p>
        <p>{{ tr('原请求哈希', 'Original request hash') }}: <code>{{ receipt.requestHash }}</code></p>
        <p>{{ tr('冻结依据哈希', 'Frozen basis hash') }}: <code>{{ receipt.basisHash }}</code></p>
        <p>{{ tr('账单哈希', 'Invoice hash') }}: <code>{{ receipt.invoiceHash }}</code></p>
        <p>{{ tr('实际用量', 'Actual usage') }}: {{ receipt.usage.inputTokens }} / {{ receipt.usage.cachedInputTokens }} / {{ receipt.usage.outputTokens }} · {{ tr('总计', 'Total') }} {{ receipt.usage.totalTokens }} · {{ tr('模型请求', 'Model requests') }} {{ receipt.usage.modelRequests }}</p>
        <p>{{ tr('费用已对账；此回执不授予执行权限，也不会自动续跑。', 'Costs reconciled; this receipt grants no execution permission or automatic resume.') }}</p>
        <button type="button" class="button small" :disabled="loading || confirming || continuationLoading || approving || statusLoading" @click="previewContinuation(receipt)">{{ tr('预览新工作剩余授权', 'Preview remaining authorization for new work') }}</button>
      </article>
      <p v-if="continuationLoading" role="status">{{ tr('正在只读核验原授权和剩余额…', 'Checking original authorization and remaining balances read-only…') }}</p>
      <p v-if="continuationError" role="alert">{{ continuationError }}</p>
      <section v-if="continuation && !approveAttempted" class="root-continuation-preview" :aria-label="tr('新工作只读预览', 'Read-only new work preview')">
        <strong>{{ tr('新工作只读预览 · 仍未可执行', 'Read-only new work preview · execution unavailable') }}</strong>
        <p>{{ tr('用原冻结 Web 证据开始一次新的 Root 评估，沿用剩余预算与原截止时间。', 'Begin a fresh Root assessment of the original frozen Web evidence, retaining remaining budgets and the original deadline.') }}</p>
        <p>{{ continuation.scanId }} · {{ tr('原尝试', 'Original attempt') }} {{ continuation.attemptNumber }} · {{ tr('拟新尝试（尚未创建）', 'Proposed attempt (not created)') }} {{ continuation.proposedAttemptNumber }}</p>
        <p>{{ continuation.targetKey }} · Root <code>{{ continuation.rootRunId }}</code></p>
        <p>{{ tr('原截止时点', 'Original deadline') }}: {{ continuation.financialSource.originalDeadline }}</p>
        <p>{{ tr('模型共享剩余 tokens / 请求', 'Shared remaining model tokens / requests') }}: {{ remainingLabel(continuation.financialSource.sharedRemainingTokens) }} / {{ remainingLabel(continuation.financialSource.sharedRemainingRequests) }}</p>
        <ul><li v-for="(amount, index) in continuation.financialSource.remaining" :key="index">{{ tr(dimensionLabels[index][0], dimensionLabels[index][1]) }}: {{ remainingLabel(amount) }}</li></ul>
        <p>{{ tr('账本时间余额不是新超时额度；新工作必须沿用原截止时点。', 'Journal wall time balance is not a new timeout allowance; new work must retain the original deadline.') }}</p>
        <p>{{ tr('已核验历史根任务数', 'Historical roots checked') }}: {{ continuation.historicalRootsChecked }} · {{ tr('读取时有效身份数', 'Valid identities at read time') }}: {{ continuation.identityCount }}</p>
        <p>{{ tr('原模式回执', 'Original mode receipt') }}: <code>{{ continuation.originalModeReceiptId }}</code></p>
        <p>{{ tr('原冻结计划哈希', 'Original frozen plan hash') }}: <code>{{ continuation.taskDescription.originalPlanHash }}</code></p>
        <p>{{ tr('原模型绑定哈希', 'Original model binding hash') }}: <code>{{ continuation.modelBindingHash }}</code></p>
        <p>{{ tr('历史财务证明哈希', 'Historical financial proof hash') }}: <code>{{ continuation.financialSource.financialPhysicalHash }}</code></p>
        <p>{{ tr('预览绑定哈希', 'Preview binding hash') }}: <code>{{ continuation.proposalHash }}</code></p>
        <p>{{ tr('这是读取时的授权快照，没有创建新尝试、Root 或执行权限；独立批准前会再次核验，不会续跑或重放原请求。', 'This authorization snapshot created no attempt, Root or execution permission. Independent approval revalidates it; original requests are never resumed or replayed.') }}</p>
        <p v-if="continuation.historicalRootsChecked !== 1">{{ tr('多历史 Root 授权链尚未证明，目前不能批准新评估。', 'Multiple historical Root grant chains are not proven; approval is unavailable.') }}</p>
        <template v-else-if="!approveAttempted">
          <label><input v-model="approvalChecked" type="checkbox">{{ tr('我批准一次新的冻结 Web 证据评估，沿用上述原剩余预算与截止时间；旧请求不重放，不执行目标剩余工作。', 'I approve one fresh frozen Web evidence assessment with the original remaining budgets and deadline; old requests are not replayed and remaining target work is not executed.') }}</label>
          <button type="button" class="button small" :disabled="!approvalChecked || approving || statusLoading" @click="approveContinuation">{{ tr('批准新的 Root 评估', 'Approve fresh Root assessment') }}</button>
        </template>
      </section>
    </template>
    <section v-if="approveAttempted && approvedProposal" class="root-continuation-issuance" role="status">
      <strong>{{ tr('独立批准与新评估状态', 'Independent approval and fresh assessment status') }}</strong>
      <p>{{ approvedProposal.scanId }} · {{ tr('原尝试 / 新尝试', 'Source / new attempt') }} {{ approvedProposal.attemptNumber }} / {{ approvedProposal.proposedAttemptNumber }} · <code>{{ approvedProposal.proposalHash }}</code></p>
      <p>{{ tr('用原冻结 Web 证据开始一次新的 Root 评估，沿用剩余预算与原截止时间；旧请求不重放。', 'Fresh Root assessment of original frozen Web evidence with remaining original budgets and deadline; old requests are not replayed.') }}</p>
      <p>{{ approvedProposal.taskDescription.targetKey }} · {{ tr('原 Root / 费用回执', 'Source Root / cost receipt') }} <code>{{ approvedProposal.rootRunId }}</code> / <code>{{ approvedProposal.receiptId }}</code></p>
      <template v-if="recoveredReceipt">
        <p>{{ tr('原调用 / 账单哈希', 'Original call / invoice hash') }}: <code>{{ recoveredReceipt.callId }}</code> / <code>{{ recoveredReceipt.invoiceHash }}</code></p>
        <p>{{ tr('原实际输入 / 缓存 / 输出 / 请求', 'Original actual input / cached / output / requests') }}: {{ recoveredReceipt.usage.inputTokens }} / {{ recoveredReceipt.usage.cachedInputTokens }} / {{ recoveredReceipt.usage.outputTokens }} / {{ recoveredReceipt.usage.modelRequests }}</p>
      </template>
      <p>{{ tr('原冻结计划哈希', 'Original frozen plan hash') }}: <code>{{ approvedProposal.taskDescription.originalPlanHash }}</code></p>
      <p>{{ tr('原截止时点', 'Original deadline') }}: {{ approvedProposal.financialSource.originalDeadline }}</p>
      <p>{{ tr('批准时原模型共享剩余 tokens / 请求', 'Original shared remaining model tokens / requests at approval') }}: {{ remainingLabel(approvedProposal.financialSource.sharedRemainingTokens) }} / {{ remainingLabel(approvedProposal.financialSource.sharedRemainingRequests) }}</p>
      <ul><li v-for="(amount, index) in approvedProposal.financialSource.remaining" :key="index">{{ tr(dimensionLabels[index][0], dimensionLabels[index][1]) }}: {{ remainingLabel(amount) }}</li></ul>
      <p v-if="approving">{{ tr('正在核验独立批准…', 'Verifying independent approval…') }}</p>
      <p v-if="issuanceError" role="alert">{{ issuanceError }}</p>
      <template v-if="issuance">
        <p>{{ tr('批准回执 / 新 Root', 'Approval receipt / new Root') }}: <code>{{ issuance.approvalId }}</code> / <code>{{ issuance.newRootRunId }}</code></p>
        <p v-if="issuance.status === 'issued'">{{ tr('已发行新 Root；评估发布尚未证实。', 'Fresh Root issued; assessment publication remains unconfirmed.') }}</p>
        <p v-if="issuance.assessmentCompleted">{{ tr('新评估已实际发布；整个 Web 目标尚未完成，未执行剩余目标工作。', 'Fresh assessment actually published; the Web target remains incomplete and remaining target work was not executed.') }}</p>
        <p v-if="issuance.assessmentCompleted && !issuance.attemptClosed">{{ tr('发布已核验，尝试收口尚未完成。', 'Publication verified; attempt closure remains pending.') }}</p>
        <p v-if="issuance.attemptClosed">{{ tr('新评估尝试已核验收口；扫描保留 partial。', 'Assessment attempt closure verified; scan remains partial.') }}</p>
        <p v-if="issuance.status === 'review_required'">{{ tr('新工作需人工复核，原费用与回执保留；不会自动重试。', 'New work requires review; original costs and receipts remain, without automatic retries.') }}</p>
      </template>
      <label v-if="recoveredOperation && issuance?.workerStartAllowed && !issuance.attemptClosed && issuance.status !== 'review_required'"><input v-model="approvalChecked" type="checkbox">{{ tr('我重新确认上述原冻结评估、剩余预算和截止时间，仅继续同一已批准评估或收口；旧请求不重放。', 'I reconfirm the frozen assessment, original remaining budgets and deadline, only to continue the same approved assessment or closure; old requests are not replayed.') }}</label>
      <button type="button" class="button small" :disabled="approving || statusLoading || discoveryLoading" @click="readContinuationStatus">{{ tr('只读查询同一批准状态', 'Read the same approval status') }}</button>
      <button v-if="!issuance || issuance.workerStartAllowed && !issuance.attemptClosed && issuance.status !== 'review_required'" type="button" class="button small" :disabled="approving || statusLoading || discoveryLoading || !approvalChecked" @click="approveContinuation">{{ tr('显式重试同一批准', 'Explicitly retry the same approval') }}</button>
    </section>
  </details>
</template>

<style scoped>
.native-root-reconciliation { margin: 12px 0; padding: 12px; border: 1px solid var(--line, #e3e5e9); border-radius: 9px; font-size: 12px; overflow-wrap: anywhere; }
.native-root-reconciliation article, .root-cost-confirmation, .root-continuation-preview { margin-top: 10px; padding: 10px 0; border-top: 1px solid var(--line, #e3e5e9); }
.native-root-reconciliation p { margin: 6px 0; line-height: 1.55; }
.native-root-reconciliation summary { cursor: pointer; }
.native-root-reconciliation button { margin-right: 8px; }
</style>
