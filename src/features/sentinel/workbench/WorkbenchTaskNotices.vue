<script setup lang="ts">
import { useI18n } from "../../../i18n";
import type { ClosureHandoffPreview, GapFollowupPreview } from "../types";
import InlineConfirm from "../../../shared/components/InlineConfirm.vue";
import type { useClosureHandoff } from "./useClosureHandoff";
import type { useFollowupRecovery } from "./useFollowupRecovery";

const props = defineProps<{
  handoff?: ClosureHandoffPreview;
  followup?: GapFollowupPreview;
  busy: boolean;
  handoffControls: ReturnType<typeof useClosureHandoff>;
  followupControls: ReturnType<typeof useFollowupRecovery>;
}>();
const { tr } = useI18n();
const { handoffConfirmed, pendingHandoff } = props.handoffControls;
const { pendingFollowupInput, followupSubmission, followupRecoveryBusy,
  followupRecoveryError, showReleaseFollowup, restoreFollowupSubmission,
  releaseFollowupSubmission } = props.followupControls;
</script>

<template>
  <section v-if="handoff" class="effective-policy-card" aria-label="独立任务交接">
    <strong>{{ tr('人工结案后的独立任务 · 仅保存草稿', 'Independent task after administrative closure · draft only') }}</strong>
    <p>{{ tr('来源任务／结案回执', 'Source task / closure receipt') }}：{{ handoff.sourceScanId }} · {{ handoff.closureId }}</p>
    <p>{{ tr('不复制原登录身份、授权、预算或未完成指令；旧任务不会恢复。每个结案保留一个正式交接任务，重复提交或重新打开会找回它。原任务的未知效果仍未结清。', 'No old identities, grants, budget or unfinished instructions are copied. The old task never resumes. Each closure retains one canonical handoff; retries and reopening recover it. Source effects remain unsettled.') }}</p>
    <p>{{ tr('可从下方删除不需要的原目标；新增其他目标请另建普通任务。预算为必填，身份必须重新选择。', 'Remove unneeded original targets below; use a separate ordinary task for other targets. Budget is required and identities must be selected afresh.') }}</p>
    <label><input v-model="handoffConfirmed" type="checkbox" :disabled="busy || !!pendingHandoff" />{{ tr('我已重新核对目标、预算和所选身份（未选为匿名）；仅创建草稿，另行确认启动。', 'I reviewed targets, budget and selected identities (none means anonymous). Create a draft only; execution needs separate confirmation.') }}</label>
    <p v-if="pendingHandoff">{{ tr('冻结提交，当前表单编辑不会改变重试内容：', 'Frozen submission; form edits do not change retry content: ') }}{{ pendingHandoff.taskName }} · {{ pendingHandoff.maxBudgetUsd }} USD · {{ pendingHandoff.authSessionIds.length }} {{ tr('个身份', 'identities') }} · {{ pendingHandoff.urls.join(' · ') }}</p>
  </section>
  <section v-if="followup" class="effective-policy-card">
    <strong>{{ tr("补充证据草稿 · 尚未授权执行", "Evidence follow-up draft · not authorized to execute") }}</strong>
    <p>{{ tr("来源任务", "Source task") }}：{{ followup.sourceScanId }} · {{ followup.candidateId }} · rev {{ followup.candidateRevision }}</p>
    <p>{{ tr("缺失证据", "Missing evidence") }}：{{ followup.missingEvidence.join("；") }}</p>
    <p>{{ tr("前置条件", "Prerequisites") }}：{{ followup.proposal.prerequisites.join("；") }}</p>
    <p>{{ tr("只保留原目标与来源关联，不复制旧身份、授权或漏洞结论。保存后登记新控制组，再确认启动。", "Keep the original target and provenance only; no old identities, grants or verdicts are copied. Save, register fresh controls, then confirm start.") }}</p>
    <p v-if="followupRecoveryBusy">{{ tr("正在核对已保存的提交记录，不会自动创建或启动任务。", "Reconciling saved submissions without creating or starting a task.") }}</p>
    <p v-if="followupRecoveryError" role="alert">{{ tr("提交记录暂时无法核对，已阻止新建：", "Creation blocked: unable to reconcile submissions: ") }}{{ followupRecoveryError }}</p>
    <button v-if="followupRecoveryError" class="button ghost compact" :disabled="busy || followupRecoveryBusy" @click="restoreFollowupSubmission">{{ tr("重新核对", "Reconcile again") }}</button>
    <p v-if="followupSubmission?.scan">{{ tr("已找到原提交创建的任务：", "Found the task from the original submission: ") }}{{ followupSubmission.scan.taskName }} · {{ followupSubmission.scan.id }} · {{ followupSubmission.scan.status }}。{{ tr("再次保存只打开该任务，不会重复创建或启动。", "Save again only opens this task; it does not create or start another.") }}</p>
    <p v-else-if="followupSubmission?.createdScanId">{{ tr("原提交曾创建的任务已删除，不会自动重建：", "The original task was deleted and will not be recreated: ") }}{{ followupSubmission.createdScanId }}</p>
    <p v-else-if="pendingFollowupInput">{{ tr("上次提交尚未确认。再次保存沿用原请求和原提交内容；重开页面或应用后先从本地数据库恢复，不会自动提交。", "Submission unconfirmed. Save again reuses the original request and input. Reopening restores it from the local database without automatic submission.") }}</p>
    <p v-if="pendingFollowupInput">{{ tr("冻结提交（下方表单编辑不改变此请求）：", "Frozen submission (form edits do not change this request): ") }}{{ pendingFollowupInput.taskName }} · {{ pendingFollowupInput.scanMode }} · {{ tr("预算", "Budget") }} {{ pendingFollowupInput.maxBudgetUsd ?? "—" }} USD · {{ pendingFollowupInput.authSessionIds.length }} {{ tr("个身份句柄", "identity handles") }}</p>
    <button v-if="followupSubmission && !showReleaseFollowup" class="button ghost compact" :disabled="busy || followupRecoveryBusy" @click="showReleaseFollowup = true">{{ tr("结束原提交并重新准备", "Finish original submission and prepare again") }}</button>
    <InlineConfirm v-if="showReleaseFollowup && followupSubmission" :busy="busy" tone="warning" :title="tr('确认结束原提交？', 'Finish the original submission?')" :detail="tr('已保存的任务会保留；尚未落库的原请求将被禁止继续创建。若后台刚刚创建成功，必须重新核对结果。之后需要重新选择身份，再显式保存另一份草稿。', 'Existing tasks are preserved; uncommitted requests are fenced from creation. A concurrent commit requires reconciliation. Select identities again before explicitly saving another draft.')" :confirm-text="tr('结束原提交', 'Finish submission')" @confirm="releaseFollowupSubmission" @cancel="showReleaseFollowup = false" />
  </section>
</template>
