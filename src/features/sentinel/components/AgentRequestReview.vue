<script setup lang="ts">
import { onUnmounted, ref, shallowRef, watch } from "vue";
import { sentinelApi } from "../api";
import type { AgentRequestDisposition, AgentRequestReviewInput, AgentRequestReviewItem, AgentRequestReviewPage } from "../types";

const props = defineProps<{ scanId: string; attemptNumber: number; targetUrl: string }>();
const page = shallowRef<AgentRequestReviewPage | null>(null);
const selected = shallowRef<AgentRequestReviewItem | null>(null);
const pending = shallowRef<AgentRequestReviewInput | null>(null);
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const notice = ref("");
const note = ref("");
const confirmed = ref(false);
const disposition = ref<AgentRequestDisposition>("still_unknown");
let generation = 0;
const sourceLabels: Record<string, string> = { native_http: "执行器 HTTP", external_surface: "公开面", authorization_probe: "授权对照" };
const receiptLabels: Record<string, string> = {
  no_headers_recorded: "尚无响应头回执（也可能仍在执行）",
  headers_recorded_body_not_asserted: "已收到响应头；不证明响应体完整或目标效果",
  no_response_recorded: "尚无响应记录（不证明未发送）",
  response_recorded: "已记录响应；不代表效果验证完成",
  response_reference_recorded: "已记录响应引用；不保证证据文件仍完整",
};
const dispositionLabels: Record<AgentRequestDisposition, string> = {
  still_unknown: "仍无法确认", effect_observed: "人工观察到目标效果", not_sent_attested: "人工判定未发送",
};
function reset() {
  generation++;
  page.value = null; selected.value = null; pending.value = null;
  loading.value = false; saving.value = false; error.value = ""; notice.value = "";
  note.value = ""; confirmed.value = false; disposition.value = "still_unknown";
}
watch(() => [props.scanId, props.attemptNumber, props.targetUrl], reset, { flush: "sync" });
onUnmounted(() => { generation++; });

async function load() {
  if (saving.value || !props.scanId || props.attemptNumber < 1 || !props.targetUrl) return;
  const version = ++generation;
  const { scanId, attemptNumber, targetUrl } = props;
  loading.value = true; error.value = ""; selected.value = null; pending.value = null; page.value = null;
  try {
    const result = await sentinelApi.getAgentRequestReviews(scanId, attemptNumber, targetUrl);
    if (version !== generation) return;
    if (result.scanId !== scanId || result.attemptNumber !== attemptNumber || result.targetUrl !== targetUrl
      || result.automaticReplayAllowed !== false || result.executionUnlocked !== false || !Array.isArray(result.items)) {
      throw new Error("request_review_scope_mismatch");
    }
    page.value = result;
  } catch (e) { if (version === generation) error.value = String(e); }
  finally { if (version === generation) loading.value = false; }
}
function choose(item: AgentRequestReviewItem) {
  if (saving.value || pending.value || !page.value?.items.includes(item)) return;
  selected.value = item; note.value = ""; confirmed.value = false; disposition.value = "still_unknown";
  error.value = ""; notice.value = "";
}
function sameJsonValue(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (!left || !right || typeof left !== "object" || typeof right !== "object") return false;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right) && left.length === right.length
      && left.every((value, index) => sameJsonValue(value, right[index]));
  }
  const a = left as Record<string, unknown>, b = right as Record<string, unknown>;
  return Object.keys(a).length === Object.keys(b).length
    && Object.keys(a).every(key => Object.prototype.hasOwnProperty.call(b, key) && sameJsonValue(a[key], b[key]));
}
function matchesReviewReceipt(receipt: unknown, input: AgentRequestReviewInput, snapshot: Record<string, unknown>): boolean {
  if (!receipt || typeof receipt !== "object" || Array.isArray(receipt)) return false;
  const value = receipt as Record<string, unknown>;
  // Match the complete frozen submission, not just its UUID. The response
  // cannot introduce execution/replay flags or silently substitute a verdict.
  return Object.keys(value).length === 9 && value.id === input.operationId
    && value.previousReviewId === input.previousReviewId && value.snapshotHash === input.snapshotHash
    && sameJsonValue(value.snapshot, snapshot) && value.disposition === input.disposition
    && value.note === input.note && value.actor === "local_operator" && value.reviewedAttempt === input.attemptNumber
    && typeof value.createdAt === "string" && Number.isFinite(Date.parse(value.createdAt));
}
async function submit() {
  if (saving.value || loading.value || !selected.value || (!pending.value && (!confirmed.value || !note.value.trim()))) return;
  const version = generation;
  if (!pending.value) {
    pending.value = { scanId: props.scanId, attemptNumber: props.attemptNumber, targetUrl: props.targetUrl,
      requestKey: selected.value.requestKey, snapshotHash: selected.value.snapshotHash,
      previousReviewId: selected.value.reviews[selected.value.reviews.length - 1]?.id ?? "", operationId: crypto.randomUUID(),
      disposition: disposition.value, note: note.value.trim(), operatorConfirmed: true };
  }
  const submission = pending.value;
  const snapshot = selected.value.snapshot;
  saving.value = true; error.value = "";
  try {
    const receipt = await sentinelApi.recordAgentRequestReview(submission);
    if (version !== generation) return;
    if (!matchesReviewReceipt(receipt, submission, snapshot)) throw new Error("request_review_receipt_mismatch");
    pending.value = null; selected.value = null;
    notice.value = "人工核对已保存。预算、机器回执和执行停止状态均未改变；没有发送目标请求。";
    saving.value = false;
    await load();
  } catch (e) {
    if (version === generation) error.value = `${String(e)}。可重试同一提交；或刷新核对记录后重新判断。`;
  } finally { if (version === generation) saving.value = false; }
}
</script>

<template>
  <section class="request-review" aria-label="目标请求人工核对">
    <button type="button" :disabled="loading || saving" @click="load">{{ loading ? "正在读取…" : "查看 / 刷新请求核对记录" }}</button>
    <p>人工核对是本地操作者的声明，不是机器验证或重新授权。不会退回预算、补造响应、恢复任务或自动重发。</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-if="notice" role="status">{{ notice }}</p>
    <template v-if="page">
      <p v-if="page.unitemizedExecutorRequests">另有 {{ page.unitemizedExecutorRequests }} 条历史执行器计数没有逐请求账本，无法生成逐条核对记录。</p>
      <p v-if="!page.items.length">该预算范围没有可列出的逐请求记录；不表示历史请求数为零。</p>
      <article v-for="item in page.items" :key="item.requestKey">
        <strong>{{ sourceLabels[item.snapshot.source] ?? item.snapshot.source }} · 尝试 {{ item.snapshot.sourceAttempt }}</strong>
        <p>{{ receiptLabels[item.receiptState] ?? item.receiptState }}</p>
        <small>运行 {{ item.snapshot.runId }}<template v-if="item.snapshot.invocationId"> · 调用 {{ item.snapshot.invocationId }} / 请求 {{ item.snapshot.requestIndex }}</template></small>
        <small v-if="item.snapshot.side">对照侧 {{ item.snapshot.side }} · 合同 {{ item.snapshot.contractKey }}</small>
        <small v-if="item.snapshot.toolStatus">工具 {{ item.snapshot.tool }} · {{ item.snapshot.toolStatus }} · {{ item.snapshot.toolError }}</small>
        <p v-if="item.sourceChangedSinceReview" role="status">机器记录在上次人工核对后发生变化，请重新检查。旧核对仍保留。</p>
        <details v-if="item.reviews.length">
          <summary>人工核对历史（{{ item.reviews.length }}）</summary>
          <div v-for="review in item.reviews" :key="review.id">
            <strong>{{ dispositionLabels[review.disposition] }}</strong>
            <small>{{ review.createdAt }} · 本地操作者（非认证身份） · 核对时尝试 {{ review.reviewedAttempt }}</small>
            <p class="review-note">{{ review.note }}</p>
          </div>
        </details>
        <button type="button" :disabled="saving || !!pending" @click="choose(item)">{{ item.reviews.length ? "追加核对 / 更正" : "人工核对此请求" }}</button>
      </article>
      <form v-if="selected" @submit.prevent="submit">
        <strong>核对 {{ sourceLabels[selected.snapshot.source] }} · {{ selected.snapshot.runId }}</strong>
        <fieldset :disabled="saving || !!pending">
          <label>人工判断
            <select v-model="disposition">
              <option value="still_unknown">仍无法确认</option>
              <option value="effect_observed">人工观察到目标效果</option>
              <option value="not_sent_attested">人工判定未发送（仅声明，不退款）</option>
            </select>
          </label>
          <label>核对依据（不粘贴凭据、Cookie 或原始敏感日志）
            <textarea v-model="note" required maxlength="1000" rows="4" placeholder="记录日志位置、关联编号、核对时间、依据和局限；没有找到日志不等于未发送。" />
          </label>
          <label><input v-model="confirmed" type="checkbox">我确认以上为人工判断，理解它不会解除停止或授权重试。</label>
        </fieldset>
        <button type="submit" :disabled="saving || (!pending && (!confirmed || !note.trim()))">{{ saving ? "提交中…" : pending ? "重试同一提交" : "保存人工核对" }}</button>
      </form>
    </template>
  </section>
</template>

<style scoped>
.request-review { display: grid; gap: 8px; overflow-wrap: anywhere; margin-top: 12px; }
.request-review p { margin: 4px 0; line-height: 1.5; }
.request-review article, .request-review form { border: 1px solid var(--border-color, #7775); border-radius: 8px; padding: 10px; }
.request-review small, .request-review label { display: block; margin: 5px 0; }
.request-review textarea { display: block; width: 100%; box-sizing: border-box; }
.request-review fieldset { border: 0; padding: 0; min-width: 0; }
.review-note { white-space: pre-wrap; }
</style>
