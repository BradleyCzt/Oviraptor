<script setup lang="ts">
import { computed, onUnmounted, watch } from "vue";
import RootDecisionSummary from "./RootDecisionSummary.vue";
import AgentIdentityMaterial from "./AgentIdentityMaterial.vue";
import { useAgentDialogTasks } from "../composables/useAgentDialogTasks";
import { useAgentDialogReading } from "../composables/useAgentDialogReading";
import { useAgentDialogStatus } from "../composables/useAgentDialogStatus";
import { useAgentDialogDirectives } from "../composables/useAgentDialogDirectives";
import { useAgentDialogFollowup } from "../composables/useAgentDialogFollowup";
import { agentDialogLabels, dialogContextPresentation, humanReviewPresentation } from "./agentDialogLabels";
import { timelineIdentity } from "../timeline/projectionContract";
import { humanAssessmentRows } from "../timeline/humanAssessmentContract";
import type { AgentTimelineItem, RootHumanAssessmentObligation } from "../../../types";
import type { GapFollowupPreview, SentinelScan } from "../../../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ projectId?: number; initialScanId?: string }>();
const emit = defineEmits<{ start: []; openResults: [scan: SentinelScan]; prepareFollowup: [preview: GapFollowupPreview] }>();
const { tr } = useI18n();
const error = computed(() => [actionError.value, statusError.value, listError.value,
  pageError.value, listenerError.value].filter(Boolean).join("\n"));
let disposed = false;
const taskNavigation = useAgentDialogTasks(props, tr, () => disposed);
const { scans, scanId, scanSearch, listError, pageError, hasMoreScans,
  loadingMoreScans, selectionError, selectionNotice, selectionBusy,
  selected, visibleScans, loadTaskSelection, selectScan, loadScans, loadMoreScans,
  resetForProject } = taskNavigation;
const statusSync = useAgentDialogStatus({
  scanId, selected, captureTaskView, isDisposed: () => disposed, loadScans, tr,
  schedule: setTimeout, cancel: clearTimeout,
});
const { state, statusError, listenerError, loadStatus,
  installCollaborationListener, resetForTask, dispose: disposeStatus } = statusSync;
const { threadFilter, dialogView, dialogViewError, dialogViewBusy, threadOptions,
  visibleTimeline, timelinePageSize, timelinePageAnchor, timelinePageStart, timelinePage,
  historyPage, historyBusy, historyError, historyHasEarlier, historyRetainedLocalPage,
  showEarlierMessages, showLaterMessages, showLatestMessages, selectedThreadUnavailable,
  selectedThreadLabel, visibleUnreadCount, canMarkPageRead, loadDialogView, saveDialogView } = useAgentDialogReading({
  projectId: () => props.projectId, scanId, state, captureTaskView, isDisposed: () => disposed, tr,
});
const { draft, actionError, sending, selectedHistoricalReceipt, directiveDrafts, orderedAssessmentPlan, orderedAssessmentExecution,
  reviewingDraft, reviewKind, reviewText, beginReview, closeReview, submitReview,
  send, canConfirmDirective, canCancelDirective, confirmDirective, cancelDirective, reconcileReceipt, reconcileHistoricalReceipt,
  resetForTask: resetDirectivesForTask } = useAgentDialogDirectives({
  projectId: () => props.projectId, scanId, state, threadFilter, captureTaskView,
  isDisposed: () => disposed, loadStatus: () => loadStatus(undefined, true), tr,
});
const { preparingFollowup, prepareFollowup } = useAgentDialogFollowup({
  projectId: () => props.projectId, scanId, state, captureTaskView, actionError,
  onPreview: (preview) => emit("prepareFollowup", preview), tr,
});
let taskViewGeneration = 0;

// A -> B -> A is a different view, even though the scan ID matches again.
function captureTaskView() {
  const generation = taskViewGeneration;
  const projectId = props.projectId;
  const requestedScanId = scanId.value;
  return {
    scanId: requestedScanId,
    isCurrent: () => !disposed && generation === taskViewGeneration
      && projectId === props.projectId && requestedScanId === scanId.value,
  };
}

const visibleHumanAssessments = computed(() => humanAssessmentRows(state.value).filter(item =>
  !threadFilter.value || item.humanDirective.threadKey === threadFilter.value));
function humanAssessmentLabel(state: RootHumanAssessmentObligation["state"]) {
  switch (state) {
    case "cost_unconfirmed": return tr("费用回执待核对", "Cost receipt needs reconciliation");
    case "dispatch_unconfirmed": return tr("派发结果尚未确认", "Dispatch outcome unconfirmed");
    case "not_sent": return tr("请求未发送", "Request not sent");
    case "awaiting_receipt": return tr("等待原调用回执", "Awaiting original call receipt");
    case "assessment_unpublished": return tr("评估尚未发布", "Assessment unpublished");
  }
}
const observedRoles = computed(() => [...new Set((state.value?.timeline ?? [])
  .flatMap((item) => [item.fromRole, item.toRole])
  .filter((role) => role && role !== "operator"))]);
const { roleLabel, eventLabel, diagnosticLabel, gapStepLabel, gapReasonLabel } = agentDialogLabels(tr);
const { reviewLabel, actionLabel, actionReasonLabel, decisionLabel, validationLabel, effectLabel, directiveReasonLabel } = humanReviewPresentation(tr);
const { statusLabel, reviewStatusLabel, stageLabel, nextActionLabel, timestampLabel } = dialogContextPresentation(tr);

function homogeneousGuidanceStatus(item: AgentTimelineItem) {
  const source = item.eventType === 'user_directive' ? item.id : item.humanReview?.directiveId;
  if (!source) return undefined;
  return state.value?.timeline.find((row) => row.eventType === 'homogeneous_guidance'
    && row.sourceUserDirectiveId === source)?.status;
}

void installCollaborationListener();
watch(() => props.projectId, () => {
  resetForProject();
  void loadScans().then(() => loadStatus(undefined, true));
}, { immediate: true, flush: "sync" });
watch(() => props.initialScanId, (id) => {
  if (id) void selectScan(id);
});
watch([() => props.projectId, scanId], () => {
  taskViewGeneration++;
  resetForTask();
  resetDirectivesForTask();
  threadFilter.value = "";
  void loadStatus(undefined, true);
}, { flush: "sync" });
onUnmounted(() => {
  disposed = true;
  disposeStatus();
});
</script>

<template>
  <section class="agent-dialog-page">
    <header>
      <div>
        <span class="eyebrow">AGENT DIALOG</span>
        <h2>{{ tr("调查对话", "Investigation dialog") }}</h2>
        <p>{{ tr("采集、调查和检查的话出现在这里。你的修正进入正在跑或下一次继续的调查，不能扩大域名或预算。", "Collector, investigator and checker notes appear here. Your correction enters the running or next resumed investigation and cannot widen scope or budget.") }}</p>
      </div>
      <div class="button-row">
        <button class="button ghost" type="button" @click="loadScans().then(() => loadStatus(undefined, true))">{{ tr("刷新消息", "Refresh messages") }}</button>
        <button class="button primary" type="button" @click="emit('start')">{{ tr("新调查", "New investigation") }}</button>
      </div>
    </header>
    <div v-if="error" class="form-error" role="alert">
      <p>{{ error }}</p>
      <button v-if="listError || statusError" class="button ghost compact" type="button" @click="loadScans().then(() => loadStatus(undefined, true))">{{ tr("重新读取", "Retry loading") }}</button>
    </div>
    <div v-if="selectionNotice || selectionBusy || selectionError" class="dialog-selection-status">
      <p v-if="selectionNotice" role="status">{{ selectionNotice }}</p>
      <p v-if="selectionBusy" role="status">{{ tr("正在保存任务选择…", "Saving task selection…") }}</p>
      <p v-if="selectionError" role="status">{{ tr("任务选择未同步，不影响任务执行：", "Task selection not synchronized; execution unaffected: ") }}{{ selectionError }}</p>
      <button v-if="selectionError" type="button" class="button ghost compact" :disabled="selectionBusy" @click="loadTaskSelection">{{ tr("重新读取任务选择", "Reload task selection") }}</button>
    </div>
    <div v-if="!scans.length" class="empty-state">{{ tr("还没有调查。点右上角「新调查」提交 URL。", "No investigation yet. Use New investigation to submit a URL.") }}</div>
    <div v-else class="agent-dialog-layout">
      <aside>
        <input v-model="scanSearch" type="search" :placeholder="tr('搜索历史任务', 'Search tasks')" :aria-label="tr('搜索历史任务', 'Search tasks')" />
        <button v-for="scan in visibleScans" :key="scan.id" type="button" :class="{ active: scan.id === scanId }" @click="selectScan(scan.id)">
          <strong>{{ scan.taskName || scan.projectName || scan.id.slice(0, 8) }}</strong>
          <small>{{ statusLabel(scan.status) }} · {{ tr("第", "Attempt ") }}{{ scan.latestAttemptNumber || 0 }}{{ tr(" 次", "") }}</small>
        </button>
        <button v-if="hasMoreScans" type="button" class="button ghost compact" :disabled="loadingMoreScans" @click="loadMoreScans">{{ loadingMoreScans ? tr('正在读取…', 'Loading…') : tr('加载更早任务', 'Load older tasks') }}</button>
        <small v-if="scanSearch && hasMoreScans">{{ tr('搜索仅覆盖已加载任务；可继续加载更早记录。', 'Search covers loaded tasks; load older records to search further.') }}</small>
      </aside>
      <div class="agent-dialog-thread">
        <div v-if="selected" class="dialog-task-header">
          <div><strong>{{ selected.taskName || selected.projectName || selected.id }}</strong><small>{{ statusLabel(selected.status) }} · {{ selected.latestAttemptStopReason || selected.latestAttemptCheckpoint || selected.currentCheckpoint || tr("暂无运行摘要", "No run summary recorded") }}</small></div>
          <button type="button" class="button ghost compact" @click="emit('openResults', selected)">{{ tr("查看证据与尝试", "Evidence and attempts") }}</button>
        </div>
        <div v-if="state?.followup?.source" class="gap-assessment-note">
          {{ tr("关联来源任务", "Linked source task") }}：{{ state.followup.source.sourceScanId }} · {{ state.followup.source.candidateId }} · rev {{ state.followup.source.candidateRevision }}
          <p v-if="state.followup.review?.status === 'resolved'">{{ tr("原缺证项已由新证据及独立 Reviewer 逐项复核。此结论不改变原任务历史状态，也不等于确认全部漏洞。", "The original missing items were reviewed against fresh evidence by an independent Reviewer. This does not change historical task status or confirm all findings.") }}</p>
          <p v-else>{{ tr("原证据缺口尚未收口；任务完成状态不能替代独立复核。", "The original gap remains open; task completion is not independent review.") }}</p>
          <p v-if="state.followup.review">{{ state.followup.review.status }} · {{ tr("原假设", "Original hypothesis") }}：{{ state.followup.review.hypothesisVerdict || '' }} · {{ state.followup.review.summary || state.followup.review.reasonCode || '' }}</p>
          <ul v-if="state.followup.review?.assessment">
            <li v-for="item in state.followup.review.assessment.items" :key="item.index">{{ item.missingEvidence }} · {{ item.status }} · {{ item.reason }} <small>{{ item.factRefs.join(' · ') }}</small></li>
          </ul>
        </div>
        <div v-if="state?.followup?.tasks.length" class="gap-assessment-note">
          <strong>{{ tr("已关联的补充任务（不代表缺口已解决）", "Linked follow-ups (not proof of gap resolution)") }}</strong>
          <p v-for="task in state.followup.tasks" :key="task.scanId">{{ task.taskName || task.scanId }} · {{ task.scanId }} · {{ task.status }}<br>{{ tr("缺口独立复核", "Independent gap review") }}：{{ task.review?.status || 'pending_review' }} · {{ tr("原假设", "Original hypothesis") }}：{{ task.review?.hypothesisVerdict || '' }} · {{ task.review?.summary || task.review?.reasonCode || '' }}</p>
        </div>
        <div v-if="state?.historicalPendingReceipts" class="gap-assessment-note" role="status">
          <strong>{{ tr('旧尝试仍有待核对记录', 'Earlier attempts have unsettled records') }}：{{ state.historicalPendingReceipts }}</strong>
          <p>{{ tr('此列表只读且不代表旧响应已经核验；旧记录不在当前聊天时间线中。逐条选择后可尝试独立的历史本地对账；仅核验原记录并结算原账本，不借当前尝试租约或预算，也不会重新调用模型或目标。核验失败保持原状。', 'This identity list is unverified and outside the current chat. Select an item for explicit historical local reconciliation; it verifies the saved record and settles only its original ledger, without the current lease or budget, model retry or target I/O. Failed verification leaves it unchanged.') }}</p>
          <div v-for="item in state.historicalReceiptItems || []" :key="`${item.attemptNumber}:${item.directiveId}`">
            <span>{{ tr('第', 'Attempt ') }}{{ item.attemptNumber }}{{ tr('次', '') }} · {{ item.createdAt }} · {{ item.directiveId }}</span>
            <button type="button" class="button ghost compact" :disabled="sending" @click="selectedHistoricalReceipt = `${item.attemptNumber}:${item.directiveId}`">{{ tr('选择旧记录', 'Select old record') }}</button>
            <div v-if="selectedHistoricalReceipt === `${item.attemptNumber}:${item.directiveId}`">
              <small>{{ tr('只结算此扫描、此尝试、此建议的已保存只读评估；不执行扫描。', 'Settle only this scan, attempt and saved read-only assessment; no scan execution.') }}</small>
              <button type="button" class="button compact" :disabled="sending" @click="reconcileHistoricalReceipt(item)">{{ tr('确认本地对账', 'Confirm local reconciliation') }}</button>
              <button type="button" class="button ghost compact" :disabled="sending" @click="selectedHistoricalReceipt = ''">{{ tr('取消', 'Cancel') }}</button>
            </div>
          </div>
          <small v-if="state.historicalPendingReceipts > (state.historicalReceiptItems?.length || 0)">{{ tr('这里只显示最近 50 条；其余记录仍保留且未自动处理。', 'Only the latest 50 appear here; other records remain untouched.') }}</small>
          <button v-if="selected" type="button" class="button ghost compact" @click="emit('openResults', selected)">{{ tr('查看证据与尝试', 'Evidence and attempts') }}</button>
        </div>
        <div v-if="state && state.stopDiagnostic.code !== 'in_progress'" class="dialog-task-header" role="status">
          <div>
            <strong>{{ tr('运行诊断', 'Run diagnostic') }} · {{ diagnosticLabel(state.stopDiagnostic.code) }}</strong>
            <small>{{ stageLabel(state.stopDiagnostic.stage) }} · {{ tr('待核对项目', 'Review items') }} {{ state.stopDiagnostic.obligations.length }}{{ state.stopDiagnostic.obligationsTruncated ? '+' : '' }} · {{ tr('不会自动续跑', 'No automatic resume') }}</small>
          </div>
          <button v-if="selected" type="button" class="button ghost compact" @click="emit('openResults', selected)">{{ tr('查看处理建议', 'Inspect next action') }}</button>
        </div>
        <div v-if="state" class="readiness-strip">
          <span :class="{ ready: state.coordinatorLeaseValid }">{{ tr("协调租约", "Coordinator lease") }}</span>
          <span :class="{ ready: state.childRunsStarted }">{{ tr("子智能体", "Child agents") }}</span>
          <span :class="{ ready: state.mailboxConsumerActive }">{{ tr("通信确认", "Mailbox ack") }}</span>
          <span v-if="state.findingCandidateCount === 0">{{ tr("暂无漏洞候选 · 不代表覆盖审查完成", "No finding candidate · coverage review not implied") }}</span>
          <span v-else :class="{ ready: state.reviewGateSatisfied }">{{ state.reviewStatus === 'rejected' && state.reviewGateSatisfied
            ? tr('候选已驳回 · 未发布漏洞', 'Candidate rejected · no finding published')
            : state.reviewStatus === 'insufficient_evidence'
              ? tr('审核需补证', 'Review needs evidence')
              : state.reviewGateSatisfied
                ? tr('漏洞候选已确认', 'Finding confirmed')
                : tr('漏洞候选审查中', 'Finding review pending') }}</span>
        </div>
        <AgentIdentityMaterial :project-id="selected?.projectId ?? props.projectId" :scan-id="scanId" />
        <div v-if="state" class="dialog-thread-filter">
          <label for="dialog-thread-select">{{ tr("协作线程", "Collaboration thread") }}</label>
          <select id="dialog-thread-select" v-model="threadFilter" :disabled="dialogViewBusy" @change="saveDialogView()">
            <option value="">{{ tr("全部消息", "All messages") }} · {{ state?.timeline.length || 0 }}</option>
            <option v-if="selectedThreadUnavailable" :value="threadFilter">{{ threadFilter }} · {{ tr("当前没有可显示消息", "No loaded messages") }}</option>
            <option v-for="thread in threadOptions" :key="thread.key" :value="thread.key">{{ thread.key === 'team' ? tr('团队频道', 'Team') : thread.label }} · {{ thread.count }}</option>
          </select>
          <span>{{ tr("本页未读", "Unread on this page") }} · {{ visibleUnreadCount ?? '—' }}</span>
          <button type="button" class="button ghost compact" :disabled="!dialogView || dialogViewBusy || !!dialogViewError || !canMarkPageRead" @click="saveDialogView(true)">{{ tr("标记当前页消息为已读", "Mark this page read") }}</button>
          <small v-if="historyRetainedLocalPage">{{ tr("当前保留阅读快照，不自动标读；请翻阅服务端历史页后再标记已读。", "Reading snapshot retained; browse server history before marking messages read.") }}</small>
          <small v-else-if="visibleUnreadCount && !canMarkPageRead && !dialogViewBusy && !dialogViewError">{{ tr("请先从更早的消息页依次标记已读；此操作会标记当前线程在该序号之前的全部消息。", "Mark earlier pages first; this action reads every prior message in this thread through the selected sequence.") }}</small>
          <span v-if="dialogViewError" role="status">{{ tr("阅读状态未同步，不影响任务执行：", "Reading state not synchronized; execution unaffected: ") }}{{ dialogViewError }}</span>
          <button v-if="dialogViewError" type="button" class="button ghost compact" :disabled="dialogViewBusy" @click="loadDialogView">{{ tr("重新加载阅读状态", "Reload reading state") }}</button>
        </div>
        <section v-if="visibleHumanAssessments.length || (!threadFilter && state?.humanAssessmentObligations?.truncated)" class="human-assessment-obligations" :aria-label="tr('原评估待核对', 'Original assessments to reconcile')">
          <header><strong>{{ tr('原评估待核对', 'Original assessments to reconcile') }}</strong><small>{{ tr('不会自动续跑', 'No automatic resume') }}</small></header>
          <article v-for="item in visibleHumanAssessments" :key="item.callId">
            <div><b>{{ humanAssessmentLabel(item.state) }}</b><span>{{ tr('已确认指令 · 第 ', 'Confirmed directive · revision ') }}{{ item.humanDirective.revision }}{{ tr(' 版', '') }}</span></div>
            <small>{{ item.humanDirective.threadKey === 'team' ? tr('团队频道', 'Team') : item.humanDirective.threadKey }} · {{ item.createdAt }}</small>
            <p v-if="item.reportedUsage">{{ tr('供应商报告用量（未对账）', 'Supplier-reported usage (unsettled)') }}：{{ tr('输入 ', 'Input ') }}{{ item.reportedUsage.inputTokens }} · {{ tr('缓存 ', 'Cached ') }}{{ item.reportedUsage.cachedInputTokens }} · {{ tr('输出 ', 'Output ') }}{{ item.reportedUsage.outputTokens }} · {{ tr('总计 ', 'Total ') }}{{ item.reportedUsage.totalTokens }} tokens · {{ tr('调用 ', 'Requests ') }}{{ item.reportedUsage.modelRequests }}</p>
            <p v-else>{{ tr('用量尚未确认', 'Usage unconfirmed') }}</p>
          </article>
          <p>{{ tr('按原调用保留待核对记录；需要核验原回执并明确对账后，再判断是否允许恢复。', 'Records remain with their original calls. Verify the original receipt and explicitly reconcile before deciding whether recovery is allowed.') }}</p>
          <small v-if="state?.humanAssessmentObligations?.truncated">{{ tr('仅显示前 50 条，其余待核对记录仍保留。', 'Only the first 50 are shown; other obligations remain.') }}</small>
        </section>
        <nav v-if="visibleTimeline.length > timelinePageSize || historyHasEarlier || !!historyPage" class="dialog-timeline-pages" :aria-label="tr('消息分页', 'Message pages')">
          <span v-if="historyPage">{{ tr('历史页 · 本页已加载', 'History · loaded on this page') }} {{ timelinePage.length }}</span>
          <span v-else>{{ tr('显示', 'Showing') }} {{ timelinePageStart + 1 }}–{{ timelinePageStart + timelinePage.length }} / {{ visibleTimeline.length }} {{ tr('条已加载消息', 'loaded messages') }}</span>
          <button type="button" class="button ghost compact" :disabled="historyBusy || (!historyHasEarlier && (!!historyPage || timelinePageStart === 0))" @click="showEarlierMessages">{{ tr('更早一页', 'Earlier page') }}</button>
          <button type="button" class="button ghost compact" :disabled="historyBusy || (!historyPage && timelinePageStart + timelinePage.length >= visibleTimeline.length)" @click="showLaterMessages">{{ tr('更新一页', 'Later page') }}</button>
          <button type="button" class="button ghost compact" :disabled="historyBusy || (!historyPage && timelinePageAnchor === undefined)" @click="showLatestMessages">{{ tr('最新一页', 'Latest page') }}</button>
        </nav>
        <p v-if="historyError" role="status">{{ tr('历史消息读取失败，可重试：', 'History read failed; retry: ') }}{{ historyError }}</p>
        <div class="dialog-timeline" role="region" :aria-label="tr('聊天消息时间线', 'Chat message timeline')" tabindex="0">
        <article
          v-for="item in timelinePage"
          :key="timelineIdentity(item)"
          class="agent-bubble"
          :class="{ operator: item.fromRole === 'operator', reviewer: item.eventType === 'review_gate' || item.fromRole === 'evidence_reviewer' }"
        >
          <header>
            <span class="dialog-role-mark" aria-hidden="true">{{ roleLabel(item.fromRole)?.slice(0, 1) || '·' }}</span>
            <b>{{ ['homogeneous_decision', 'homogeneous_guidance'].includes(item.eventType) ? tr('主管', 'Coordinator') : roleLabel(item.fromRole) }}</b>
            <span>{{ eventLabel(item.eventType) }}</span>
          </header>
          <RootDecisionSummary v-if="item.eventType === 'root_decision'" :chat-item="item" />
          <p v-else>{{ item.summary || item.messageKind }}</p>
          <small v-if="item.workerId">Worker {{ item.workerId.slice(-8) }} · {{ tr('切片', 'Slice') }} {{ item.sliceId?.slice(-8) }}<template v-if="item.eventType === 'worker_report'"> · {{ item.modelRequests }} {{ tr('次模型请求', 'model request(s)') }} · {{ item.totalTokens }} tokens</template></small>
          <p v-if="item.zeroIntent" class="dialog-message-meta">{{ item.zeroIntent.reason === 'blocked_on_inflight_output' ? tr('等待正在执行的汇报', 'Waiting for running workers') : tr('已有任务覆盖本轮目标', 'Existing work covers this turn') }} · {{ item.zeroIntent.evidence_assignment_ids.length }} {{ tr('条在途分派', 'inflight assignments') }}</p>
          <div v-if="item.humanReview" class="human-review-summary">
            <span class="dialog-state-pill">{{ reviewLabel(item.humanReview.kind) }}</span>
            <span v-if="item.humanReview.requiresConfirmation" class="dialog-state-pill pending">{{ tr("新草案待确认", "New draft awaits confirmation") }}</span>
            <p v-if="item.humanReview.reason">{{ item.humanReview.reason }}</p>
            <p v-if="item.humanReview.kind === 'revise'">{{ tr("旧草案已终止；修改本身未入队，新草案的后续状态以独立确认回执为准。", "The old draft is closed. Revision did not queue work; later confirmation has its own receipt.") }}</p>
            <ol class="human-review-actions">
              <li v-for="action in item.humanReview.actions" :key="action.order">
                <b>{{ roleLabel(action.role) }}</b><span>{{ actionLabel(action) }}</span>
                <small v-if="action.reasonCode">{{ actionReasonLabel(action.reasonCode) }}</small>
              </li>
            </ol>
            <p class="review-boundary">{{ tr("人工判断和入队回执不代表角色执行、建议采纳或验证完成。", "Human decisions and queue receipts do not prove role execution, adoption, or completed verification.") }}</p>
            <details class="dialog-record-details">
              <summary>{{ tr("查看判断回执", "Inspect decision receipt") }}</summary>
              <code>{{ item.humanReview.receiptId }} · {{ item.humanReview.kind }} · rev {{ item.humanReview.revision }}</code>
              <p v-for="action in item.humanReview.actions" :key="action.order">{{ action.order }} · {{ action.role }} · {{ action.reviewDisposition }} · {{ action.executionState }} · {{ action.reasonCode }}</p>
            </details>
          </div>
          <details v-if="item.messageKind === 'source_review_result'" class="dialog-notice-details"><summary>{{ tr("源码候选审查 · 不等于漏洞确认或 CI 通过", "Source candidate review · no finding or CI verdict implied") }}</summary><p>{{ tr("这是源码候选审查回执，不代表总体覆盖完成、漏洞已确认或 CI 通过；具体裁决以逐候选证据为准。", "This is a source-candidate review receipt, not proof of complete coverage, confirmed findings or a passing CI gate; each decision requires its own evidence.") }}</p></details>
          <details v-if="item.messageKind === 'source_coverage_review_result'" class="dialog-notice-details"><summary>{{ tr("源码覆盖审查 · 查看本轮裁决与保留缺口", "Source coverage review · inspect this attempt’s verdict and gaps") }}</summary><p>{{ tr("这是源码覆盖审查回执；审查已交付不等于覆盖充分或 CI 通过，请查看本轮经核验的覆盖裁决与保留缺口。", "This is a source-coverage review receipt. Delivery does not imply sufficient coverage or a passing CI gate; consult this attempt's audited decision and outstanding gaps.") }}</p></details>
          <details v-if="item.eventType === 'host_boundary_candidate'" class="dialog-notice-details"><summary>{{ tr("仅记录主机边界 · 当前 Web 任务不能批准主机执行", "Host boundary recorded · this Web task cannot approve host execution") }}</summary><p>{{ tr("这不是主机测试授权，也没有批准按钮。当前任务仍限 Web；如需验证 Linux 主机，须另案明确目标、动作和时窗，并等待独立后端支持。", "This is not host-testing approval and has no approval button. This attempt remains Web-only; Linux host verification requires a separate target, action and time window, and an independent backend not yet available.") }}</p></details>
          <div v-if="item.gapDetail" class="gap-message-detail">
            <strong>{{ tr("补证建议 · 尚未授权或执行", "Evidence proposal · not authorized or executed") }}</strong>
            <p>{{ gapStepLabel(item.gapDetail.nextStep) }} · {{ item.gapDetail.gapCode }}</p>
            <dl>
              <div><dt>{{ tr("缺失证据", "Missing evidence") }}</dt><dd>{{ item.gapDetail.missingEvidence.join("；") || tr("未列出", "Not specified") }}</dd></div>
              <div><dt>{{ tr("前置条件", "Prerequisites") }}</dt><dd>{{ item.gapDetail.prerequisites.join("；") || tr("未列出", "Not specified") }}</dd></div>
              <div><dt>{{ tr("建议合同", "Proposed contracts") }}</dt><dd>{{ item.gapDetail.proposedContracts.join("、") || tr("无", "None") }}</dd></div>
              <div><dt>{{ tr("提案引用", "Proposal references") }}</dt><dd>{{ item.gapDetail.supportingFactRefs.join("、") || tr("无引用", "No references") }}</dd></div>
              <div><dt>{{ tr("估算上限", "Estimated ceiling") }}</dt><dd>{{ item.gapDetail.estimatedCost.modelTokens }} tokens · {{ item.gapDetail.estimatedCost.modelRequests }} model · {{ item.gapDetail.estimatedCost.targetRequests }} HTTP</dd></div>
            </dl>
          </div>
          <p v-if="item.gapAssessment" class="gap-assessment-note">{{ gapReasonLabel(item.gapAssessment.reasonCode) }}</p>
          <div v-if="item.gapAssessment?.newAttemptRequired" class="gap-assessment-note">
            <p>{{ tr("只准备关联草稿，不批准执行。新任务需重新采集身份、登记授权控制组，并由你确认启动。", "Prepare a linked draft only, not execution approval. Capture fresh identities, register controls, and explicitly confirm start.") }}</p>
            <button type="button" class="button ghost" :disabled="preparingFollowup" @click="prepareFollowup(item)">{{ preparingFollowup ? tr("正在核验证据…", "Verifying evidence…") : tr("准备补充任务", "Prepare follow-up") }}</button>
          </div>
          <time class="dialog-message-time">{{ timestampLabel(item.timestamp) }}</time>
          <details class="dialog-record-details">
            <summary>{{ tr("送达与回执详情", "Delivery and receipt details") }}</summary>
            <small>{{ item.timestamp }}</small>
            <small>{{ item.status }} · {{ item.deliveryState }}<template v-if="item.ackState !== 'n/a'"> · {{ item.ackState }}</template><template v-if="item.evidenceRevision"> · rev {{ item.evidenceRevision }}</template></small>
            <small v-if="item.reasonCodes?.length" class="reason-codes">{{ item.reasonCodes.map(directiveReasonLabel).join(" · ") }}</small>
          </details>
          <details v-if="item.sourceGuidance?.state === 'model_received'" class="dialog-notice-details"><summary>{{ tr("分析建议已送达 · 采纳与执行尚未确认", "Analysis guidance delivered · adoption and execution unconfirmed") }}</summary><p>{{ tr("源码分析建议已送达实际子任务", "Source analysis guidance delivered to the actual child") }}：{{ item.sourceGuidance.phase }} · {{ item.sourceGuidance.childRunId }}。{{ tr("已记录模型响应；这不代表建议被采纳、工具动作完成或漏洞确认。后发消息不会改写该阶段的冻结输入。", "A model response is recorded; this does not prove adoption, tool execution or a confirmed finding. Later messages cannot rewrite this phase's frozen input.") }}</p></details>
          <details v-if="item.reasonCodes?.includes('source_guidance_requires_dedicated_action')" class="dialog-notice-details"><summary>{{ tr("需要独立动作或范围变更 · 此条未执行", "Dedicated action or scope change required · not executed") }}</summary><p>{{ tr("该请求需要独立的源码动作或任务范围变更，未作为普通分析建议执行。暂停请使用任务控制；扩大范围或新增角色任务需单独申请。指定关注点可使用 @source_analyst 请关注…，仅送达该角色下一未冻结阶段。", "This request needs a dedicated source action or scope change; it was not executed as ordinary analysis guidance. Use task controls to pause; scope expansion or a new role assignment needs a separate request. Use @source_analyst focus on … for its next unfrozen phase only.") }}</p></details>
          <details v-if="item.reasonCodes?.includes('source_guidance_receipt_unverified')" class="dialog-notice-details"><summary>{{ tr("送达回执需核对 · 尚不能确认已送达", "Delivery receipt requires review · delivery unconfirmed") }}</summary><p>{{ tr("源码建议的送达记录与实际执行证据不一致，尚不能确认已送达，请核对执行记录。", "The source guidance receipt does not match execution evidence. Delivery is unverified; inspect execution records.") }}</p></details>
          <p v-if="item.reasonCodes?.includes('priority_no_matching_pending_work')" class="directive-warning">{{ tr("当前没有匹配的待办，优先级未应用。不会在以后静默生效；如仍需要，请重新提交并确认。", "No matching pending work; priority was not applied. It will not silently activate later. Submit and confirm a new request if still needed.") }}</p>
          <details v-if="item.reasonCodes?.includes('queue_action_receipt_unverified')" class="dialog-notice-details"><summary>{{ tr("队列回执无法核验 · 不会自动重放", "Queue receipt unverified · no automatic replay") }}</summary><p>{{ tr("队列优先级回执无法核验；历史状态不代表动作已可靠完成，恢复时不会静默重放。请核对原始确认与执行记录。", "Queue priority receipt could not be verified. Historical status is not proof of reliable completion; recovery will not silently replay it. Check the original confirmation and execution records.") }}</p></details>
          <p v-else-if="item.queueAction" class="directive-warning">{{ tr("已落实队列优先级", "Queue priority applied") }}：{{ item.queueAction.family }} · {{ item.queueAction.matchedItems }} {{ tr("项匹配待办", "matching pending items") }}。{{ item.queueAction.changedOrder ? tr("已调整顺序。", "Order changed.") : tr("匹配项原已在前，优先级已保存。", "Matching items were already first; preference saved.") }} {{ tr("只代表调度动作完成，不代表扫描、漏洞验证或复核完成。", "Only the scheduling action is complete, not scanning, vulnerability verification or review.") }}</p>
          <details v-if="item.reasonCodes?.includes('proposal_receipt_unverified')" class="dialog-notice-details"><summary>{{ tr("评估回执无法核验 · 不会自动重试模型", "Assessment receipt unverified · no automatic model retry") }}</summary><p>{{ tr("评估交付回执无法核验；不再作为有效结果展示或送入模型，也不会自动重试模型。请核对原始确认、消息与结算记录。", "Assessment delivery receipt could not be verified. It is not displayed or supplied to the model as a valid result, and no model retry is automatic. Check confirmation, messages and settlement records.") }}</p></details>
          <div v-else-if="item.localReconciliation" class="directive-warning">
            <strong>{{ tr("已补齐本地回执", "Local receipt reconciled") }}</strong>
            <p v-if="item.localReconciliation.status === 'completed'">{{ tr("已接收保存的只读评估并完成费用结算。建议未执行，不代表漏洞验证或 Reviewer 通过。", "Saved assessment received and usage settled. Suggestions remain unexecuted, not verified findings or Reviewer approval.") }}</p>
            <p v-else>{{ tr("保存的模型响应不是有效评估；已记录失败并结算该次调用，未执行任何建议。", "Saved response was not a valid assessment; its failure and usage are recorded. No suggestions executed.") }}</p>
            <p>{{ tr("本次补齐未调用模型、未请求目标；原任务结束记录保留。", "Reconciliation made no model or target requests; original task closure is preserved.") }}</p>
            <p v-if="item.proposalAction?.summary">{{ item.proposalAction.summary }}</p>
            <small>{{ item.localReconciliation.completedAt }}</small>
          </div>
          <div v-else-if="item.taskClosure" class="directive-warning">
            <strong>{{ tr("本目标任务已结束", "This target task has ended") }}</strong>
            <p v-if="item.taskClosure.disposition === 'not_started'">{{ tr("评估子任务尚未启动，已取消并释放其未用预算。", "Assessment never started; cancelled and its unused reservation released.") }}</p>
            <p v-else-if="item.taskClosure.disposition === 'analysis_guidance_delivered' && item.sourceGuidance">{{ tr("分析建议的送达已核验并归档；仅完成建议送达，不代表建议被采纳或检查通过。", "Analysis guidance delivery is verified and archived; only delivery is complete, not adoption or a passed check.") }}</p>
            <template v-else-if="item.taskClosure.disposition === 'not_applied' && homogeneousGuidanceStatus(item)">
              <p v-if="homogeneousGuidanceStatus(item) === 'applied'">{{ tr("主管批准的分析约束已被 Worker 读取。独立动作执行和验证结果请查看对应回执。", "Workers read the coordinator-approved analysis constraint. Consult the corresponding receipts for independent actions and verification.") }}</p>
              <p v-else>{{ tr("主管引导状态见对应回执；本轮没有确认 Worker 已读取该约束。", "Consult the coordinator guidance receipt; worker reading of this constraint is unconfirmed for this attempt.") }}</p>
            </template>
            <p v-else-if="item.taskClosure.disposition === 'not_applied'">{{ tr("指令未取得动作落实回执，不视为已执行。", "No action receipt was recorded; this instruction is not treated as executed.") }}</p>
            <p v-else-if="item.taskClosure.disposition === 'outcome_unknown'">{{ tr("模型调用结果未知，预算预留保留，需核对后处理。", "Model outcome unknown; reservation retained for reconciliation.") }}</p>
            <template v-else-if="item.taskClosure.disposition === 'receipt_pending'">
              <p>{{ tr("模型响应已保存，但完成回执尚未提交，需本地对账；不要重新调用模型。", "Model response saved, but completion receipt is pending local reconciliation; do not call the model again.") }}</p>
              <button v-if="item.proposalAction?.state === 'received'" type="button" class="button" :disabled="sending" @click="reconcileReceipt(item)">{{ tr("补齐本地回执（不重试模型）", "Reconcile locally (no model retry)") }}</button>
            </template>
            <p v-else>{{ tr("执行回执或租约归属需核对，未将指令标记为成功。", "Execution receipt or lease ownership requires reconciliation; no success is claimed.") }}</p>
            <p>{{ tr("不会自动重试或在后续任务静默生效。需继续时，请先处理待对账项，再创建并确认新任务。", "No automatic retry or silent carry-over. Reconcile pending items before creating and confirming a new task.") }}</p>
          </div>
          <section v-if="orderedAssessmentExecution(item, state)" class="dialog-notice-details" aria-label="Ordered assessment receipts">
            <b>{{ tr("有序只读评估", "Ordered read-only assessments") }} · {{ orderedAssessmentExecution(item, state)?.completedAssessments }}/2 {{ tr("有效评估回执", "valid assessment receipts") }}</b>
            <ol>
              <li v-for="action in orderedAssessmentExecution(item, state)?.actions" :key="action.actionId">
                {{ roleLabel(action.role) }} · {{ action.state === 'completed' ? tr('评估已保存', 'Assessment saved') : action.state === 'failed' ? tr('评估无效', 'Invalid assessment') : action.state === 'outcome_unknown' ? tr('结果未知，未自动重试', 'Outcome unknown; no retry') : action.state === 'executing' ? tr('原模型调用进行中', 'Original model request active') : action.state === 'received' ? tr('费用已保存，待本地回执', 'Paid receipt saved; local delivery pending') : action.state === 'prepared' ? tr('等待原权限核对', 'Waiting for original authority checks') : action.state === 'cancelled_before_dispatch' ? tr('已关闭，未派发', 'Closed before dispatch') : tr('尚未派发', 'Not dispatched') }}
                <template v-if="action.receipt">
                  <p>{{ action.receipt.assessment.summary }}</p>
                  <p class="input-hint">{{ tr('实际已报告用量', 'Actual reported usage') }} · {{ action.receipt.usage.totalTokens }} tokens · {{ action.receipt.usage.modelRequests }} {{ tr('次模型请求', 'model request') }}</p>
                  <details v-if="action.receipt.assessment.limitations.length"><summary>{{ tr('评估局限', 'Assessment limitations') }}</summary><p v-for="limit in action.receipt.assessment.limitations" :key="limit">{{ limit }}</p></details>
                </template>
              </li>
            </ol>
            <p class="input-hint">{{ tr('以上是已保存的只读评估回执；建议未执行、覆盖未验证，独立 Reviewer 未审核。', 'These are saved read-only assessment receipts. Suggestions are not executed, coverage is unverified and no independent Reviewer approval exists.') }}</p>
          </section>
          <p v-else-if="item.orderedAssessmentExecution" class="directive-warning">{{ tr('有序评估回执无法核实，未显示结果或推断完成。', 'Ordered assessment receipts cannot be verified; no result or completion is inferred.') }}</p>
          <div v-else-if="item.proposalAction && !item.taskClosure && !item.localReconciliation && !item.reasonCodes?.includes('proposal_receipt_unverified')" class="directive-warning">
            <p v-if="item.proposalAction.state === 'completed'">{{ tr("独立 Agent 已提交只读评估，Coordinator 已接收。建议尚未执行，不代表漏洞验证或 Reviewer 复核完成。", "Independent assessment received by Coordinator. Suggestions are not executed, verified findings or Reviewer approval.") }}</p>
            <p v-else-if="item.proposalAction.state === 'uncertain'">{{ tr("子任务模型调用结果未知，未自动重试；预算预留保留，需核对后处理。其他已授权 Web 工作可继续。", "Child model outcome unknown; no automatic retry. Reservation retained for reconciliation. Other authorized Web work may continue.") }}</p>
            <p v-else-if="item.proposalAction.state === 'failed'">{{ tr("子任务未返回有效评估，未将其标记为完成，也未执行任何建议。", "Child returned no valid assessment. It is not completed and no suggestions were executed.") }}</p>
            <p v-else>{{ tr("只读评估子任务", "Read-only assessment child") }} · {{ item.proposalAction.state }} · {{ tr("尚未取得完成回执", "No completion receipt yet") }}</p>
            <p v-if="item.proposalAction.summary">{{ item.proposalAction.summary }}</p>
          </div>
          <p v-if="!item.reasonCodes?.includes('proposal_receipt_unverified') && item.reasonCodes?.some(code => code.startsWith('proposal_'))" class="directive-warning">{{ tr("评估请求暂未派发。请根据原因检查预算、角色或证据后重新提交确认；不会自动扩大权限或跳过复核。", "Assessment deferred. Check the budget, role or evidence and submit a new confirmation. No automatic scope expansion or review bypass.") }}</p>
          <details v-if="item.eventType === 'user_directive' && item.status === 'accepted' && !item.sourceGuidance" class="dialog-notice-details"><summary>{{ tr("请求已接受 · 尚无动作执行回执", "Request accepted · no action execution receipt") }}</summary><p>{{ item.deliveryState === 'model_received' ? tr("请求已送达模型，但尚无队列调整、任务派发或动作落实回执；不能视为已执行。", "The model received this request, but no queue change, assignment or action receipt has been recorded. This is not proof of execution.") : tr("请求已接受，尚未取得模型送达回执，也未确认动作执行。", "Request accepted; model delivery and action execution have not been confirmed.") }}</p></details>
        </article>
        <p v-if="!visibleTimeline.length" class="input-hint">{{ tr('此线程尚无已加载消息。', 'No loaded messages in this thread.') }}</p>
        </div>
        <p v-if="!(state?.timeline?.length)" class="empty-state">{{ tr("这个调查还没有持久化的智能体事件。", "This investigation has no persisted agent events yet.") }}</p>
        <section v-for="pendingDraft in directiveDrafts" :key="pendingDraft.id" class="directive-card" :class="{ rejected: pendingDraft.status === 'rejected', risky: pendingDraft.sideEffectClass !== 'read_only' }">
          <header>
            <div>
              <b>{{ tr("指令解析草案", "Parsed directive draft") }}</b>
              <span>{{ decisionLabel(pendingDraft.coordinatorDecision) }} · {{ validationLabel(pendingDraft.validationResult) }}</span>
            </div>
            <code>rev {{ pendingDraft.revision }}</code>
          </header>
          <p>{{ pendingDraft.text }}</p>
          <p class="input-hint">{{ tr("所属线程", "Thread") }}：{{ pendingDraft.threadKey === "team" ? tr("团队频道", "Team") : pendingDraft.threadKey }}</p>
          <dl>
            <div><dt>{{ tr("意图", "Intent") }}</dt><dd>{{ pendingDraft.intent }}</dd></div>
            <div v-if="pendingDraft.intent !== 'source_analysis_focus' && !pendingDraft.reasonCodes.includes('source_guidance_tool_round_eligible') && pendingDraft.priorityChanges?.length"><dt>{{ tr("调度动作", "Scheduling action") }}</dt><dd>{{ pendingDraft.priorityChanges.join("、") }}</dd></div>
            <div><dt>{{ tr("目标角色", "Requested roles") }}</dt><dd>{{ pendingDraft.requestedRoles.map(roleLabel).join("、") }}</dd></div>
            <div><dt>{{ tr("影响等级", "Side effect") }}</dt><dd>{{ effectLabel(pendingDraft.sideEffectClass) }}</dd></div>
            <div><dt>{{ tr("估算预算", "Estimated budget") }}</dt><dd>{{ pendingDraft.estimatedTokens }} tokens · {{ pendingDraft.estimatedRequests }} request</dd></div>
            <div v-if="pendingDraft.requiredApprovals.length"><dt>{{ tr("所需批准", "Required approvals") }}</dt><dd>{{ pendingDraft.requiredApprovals.join("、") }}</dd></div>
            <div v-if="pendingDraft.reasonCodes.length"><dt>{{ tr("原因码", "Reason codes") }}</dt><dd>{{ pendingDraft.reasonCodes.join("、") }}</dd></div>
          </dl>
          <section v-if="orderedAssessmentPlan(pendingDraft)" class="dialog-notice-details" aria-label="Ordered assessment budget">
            <b>{{ tr("有序评估预算", "Ordered assessment budget") }}</b>
            <ol>
              <li v-for="action in orderedAssessmentPlan(pendingDraft)?.actions" :key="action.actionId">
                {{ roleLabel(action.role) }} · {{ action.tokenCeiling }} tokens · {{ action.modelRequests }} {{ tr("次模型请求", "model request") }}
                <span v-if="action.previousActionId"> · {{ tr("需前一步有效评估回执", "requires the previous valid assessment receipt") }}</span>
              </li>
            </ol>
            <p class="input-hint">{{ tr("合计上限 8000 tokens · 2 次模型请求 · 0 次目标请求；这是确认预算，实际费用以原调用账本为准。", "Combined ceiling: 8000 tokens, 2 model requests, 0 target requests. This is the confirmation budget; original call receipts determine actual usage.") }}</p>
            <p v-if="orderedAssessmentPlan(pendingDraft)?.schemaVersion === 3" class="directive-warning">{{ tr("确认授权按所示顺序逐项派发只读模型评估，仍须原任务权限、预算和有效租约。第二项需前一项真实有效回执；建议不会执行，独立 Reviewer 未审核。", "Confirmation authorizes sequential read-only model assessments under the original scope, budget and valid lease. The second action needs the first valid committed receipt. Suggestions are not executed and no independent Reviewer approval is granted.") }}</p>
            <p v-else class="directive-warning">{{ tr("有序角色执行尚未接线。确认仅保存原计划；请求将暂缓，不会派发评估或执行建议。", "Ordered role execution is not connected. Confirmation only saves this plan; the request is deferred without dispatching assessments or executing suggestions.") }}</p>
          </section>
          <details class="dialog-record-details"><summary>{{ tr("原始解析详情", "Saved parsing details") }}</summary>
            <code>{{ pendingDraft.coordinatorDecision }} · {{ pendingDraft.validationResult }} · {{ pendingDraft.sideEffectClass }}</code>
          </details>
          <p v-if="pendingDraft.priorityChanges?.some(action => action.startsWith('prioritize_family:'))" class="input-hint">{{ tr("确认后，将匹配类别的现有待办提前，并保存本轮优先级；不新增工作、不增加扫描预算、不跳过其他类别。无匹配待办时不应用。", "Confirmation promotes existing matching work and saves this attempt’s priority. No work or scan budget is added and no other category is skipped. No matching work means no action.") }}</p>
          <p v-if="pendingDraft.proposedScopeChange" class="directive-warning">{{ tr("该请求只能形成授权变更提案；确认不会扩大当前冻结范围或预算。", "This request can only create an authorization-change proposal; confirmation will not widen the frozen scope or budget.") }}</p>
          <p v-if="pendingDraft.intent !== 'source_analysis_focus' && pendingDraft.reasonCodes.includes('source_guidance_tool_round_eligible') && pendingDraft.status !== 'rejected'" class="directive-warning">{{ tr("这是源码分析关注点，只进入下一尚未冻结的适用阶段或工具轮次；不新增模型请求、不改变权限和已发出的请求。没有后续阶段时不会送达，送达不代表采纳或执行完成。", "This source analysis focus can enter only the next unfrozen applicable phase or tool round. It adds no model request or permissions and cannot alter dispatched requests. No remaining phase means no delivery; delivery is not adoption or completed execution.") }}</p>
          <p v-if="pendingDraft.intent === 'source_analysis_focus' && pendingDraft.status !== 'rejected'" class="directive-warning">{{ tr("仅发送给指定源码角色下一次尚未冻结的阶段（含工具阶段后续轮次）；不新增 Agent 或模型请求。已经发出的请求保持不变，后续轮次仍受原预算约束。若该角色没有后续阶段，则不会送达，也不会改投其他角色。送达不代表采纳建议或作出裁决。", "Only the requested source role's next unfrozen phase, including later tool rounds, may receive this focus. No new Agent or model request is created. Dispatched requests remain unchanged and later rounds keep the original budget. If no phase remains, it is not delivered or rerouted. Delivery is not adoption or a verdict.") }}</p>
          <p v-if="pendingDraft.reasonCodes.includes('host_boundary_not_supported')" class="directive-warning">{{ tr("当前任务仅限 Web；主机验证、远程命令和经 Web 参数触发的系统命令均不会执行。当前没有主机审批后端或执行器，聊天确认不能开通。", "This task is Web-only. Host verification, remote commands and OS commands triggered through Web parameters will not run. No host approval backend or executor exists; chat confirmation cannot enable them.") }}</p>
          <p v-else-if="pendingDraft.status === 'rejected'" class="directive-warning">{{ tr("草案被拒绝，不会进入主管队列；请查看原因。", "Draft rejected; it cannot enter the supervisor queue. Inspect the reason.") }}</p>
          <div v-if="reviewingDraft?.id === pendingDraft.id" class="directive-review-editor">
            <p>{{ reviewKind === 'revise' ? tr("保存后重新解释并生成待确认草案，不会执行。", "Saving reparses a new draft for confirmation; it does not execute.") : tr("拒绝原因将保存到终态回执。", "The rejection reason is saved in a terminal receipt.") }}</p>
            <textarea v-model="reviewText" :aria-label="reviewKind === 'revise' ? tr('修改草案内容', 'Revised draft text') : tr('拒绝理由', 'Rejection reason')" rows="3" :maxlength="reviewKind === 'revise' ? 2000 : 500" :disabled="sending"></textarea>
            <button class="button" type="button" :disabled="sending" @click="closeReview">{{ tr("收起", "Close editor") }}</button>
            <button class="button primary" type="button" :disabled="sending || !reviewText.trim()" @click="submitReview">{{ reviewKind === 'revise' ? tr("保存修改草案", "Save revised draft") : tr("保存拒绝理由", "Save rejection") }}</button>
          </div>
          <footer>
            <button v-if="pendingDraft.status !== 'rejected'" class="button" type="button" :disabled="sending || !canConfirmDirective(pendingDraft)" @click="beginReview(pendingDraft, 'revise')">{{ tr("修改", "Revise") }}</button>
            <button v-if="pendingDraft.status !== 'rejected'" class="button" type="button" :disabled="sending || !canConfirmDirective(pendingDraft)" @click="beginReview(pendingDraft, 'reject')">{{ tr("拒绝并记录理由", "Reject with reason") }}</button>
            <button class="button" type="button" :disabled="pendingDraft.status !== 'rejected' && (sending || !canCancelDirective(pendingDraft))" @click="cancelDirective(pendingDraft)">{{ pendingDraft.status === "rejected" ? tr("关闭", "Close") : tr("取消", "Cancel") }}</button>
            <button v-if="pendingDraft.status !== 'rejected'" class="button primary" type="button" :disabled="sending || !canConfirmDirective(pendingDraft)" @click="confirmDirective(pendingDraft)">{{ sending ? tr("确认中", "Confirming") : tr("确认并入队", "Confirm and queue") }}</button>
          </footer>
        </section>
        <form class="dialog-composer" @submit.prevent="send">
          <p class="input-hint">{{ tr("发送到", "Send to") }}：{{ !threadFilter || threadFilter === "team" ? tr("团队频道", "Team") : selectedThreadLabel }}</p>
          <textarea v-model="draft" :aria-label="tr('调查方向与补充说明', 'Investigation correction')" rows="3" maxlength="2000" :placeholder="tr('写下要修正的方向，例如先验证登录接口，不要停在 404', 'Write a correction, for example keep testing the login endpoint instead of stopping on 404')"></textarea>
          <p class="input-hint">{{ tr("新建同质任务保留用于身份比对的 Cookie、Token、密码等原文。发送生成草案，确认后由主管处理。", "New homogeneous tasks preserve original cookies, tokens and passwords for identity comparison. Send creates a draft; the supervisor handles it after confirmation.") }}</p>
          <button class="button" type="submit" :disabled="sending || !draft.trim() || directiveDrafts.some((item) => item.status !== 'rejected')">{{ sending ? tr("解析中", "Parsing") : tr("解析指令", "Parse directive") }}</button>
        </form>
      </div>
      <aside class="agent-dialog-context" aria-label="任务上下文与待办">
        <strong>{{ tr('任务上下文', 'Task context') }}</strong>
        <p v-if="!state">{{ tr('选择任务后读取已落库的运行状态。', 'Select a task to read its persisted status.') }}</p>
        <template v-else>
          <dl>
            <div><dt>{{ tr('尝试', 'Attempt') }}</dt><dd>#{{ state.attemptNumber }}</dd></div>
            <div><dt>{{ tr('状态', 'Status') }}</dt><dd>{{ statusLabel(state.status) }}</dd></div>
            <div><dt>{{ tr('模型调用', 'Model requests') }}</dt><dd>{{ state.llmRequests ?? '—' }}</dd></div>
            <div><dt>Token</dt><dd>{{ state.totalTokens ?? '—' }}</dd></div>
            <div><dt>{{ tr('候选', 'Candidates') }}</dt><dd>{{ state.findingCandidateCount }}</dd></div>
            <div><dt>{{ tr('审核', 'Review') }}</dt><dd>{{ reviewStatusLabel(state.reviewStatus) }}</dd></div>
          </dl>
          <strong>{{ tr('本轮出现的角色', 'Observed roles this attempt') }}</strong>
          <p>{{ observedRoles.map(roleLabel).join(' · ') || tr('尚无真实子智能体消息', 'No child-agent messages yet') }}</p>
          <strong>{{ tr('停止与下一步', 'Stop and next action') }}</strong>
          <p>{{ diagnosticLabel(state.stopDiagnostic.code) }} · {{ nextActionLabel(state.stopDiagnostic.nextAction) }}</p>
          <p v-if="state.stopDiagnostic.obligations.length">{{ tr("仍有待核对事项", "Open items remain") }} · {{ state.stopDiagnostic.obligations.length }}{{ state.stopDiagnostic.obligationsTruncated ? "+" : "" }}</p>
          <details class="dialog-record-details"><summary>{{ tr("原始状态与诊断", "Saved status and diagnostic") }}</summary>
            <small>{{ selected?.id }} · {{ selected?.status }}</small>
            <small>{{ state.status }} · {{ state.reviewStatus }} · {{ state.stopDiagnostic.code }} · {{ state.stopDiagnostic.stage }} · {{ state.stopDiagnostic.nextAction }}</small>
            <p v-for="(obligation, index) in state.stopDiagnostic.obligations" :key="index">{{ obligation.kind }} · {{ obligation.status }}</p>
          </details>
          <p class="context-caution">{{ tr('消息是建议，不等于授权。人工补充会先形成草案，确认后再由控制面核验。', 'Messages are proposals, not authorization. Human input becomes a draft and is checked before dispatch.') }}</p>
        </template>
      </aside>
    </div>
  </section>
</template>

<style scoped src="./agentDialogPresentation.css"></style>
