<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ scanId: string; attempt: number; status: string; eligible: boolean; disabled?: boolean }>();
const emit = defineEmits<{ (event: "refresh"): void }>();
const { tr } = useI18n();
const busy = ref(false), message = ref(""), error = ref("");
let generation = 0, disposed = false;
const available = computed(() => props.eligible && props.status === "paused" && !props.disabled
  && !busy.value && !!props.scanId && Number.isSafeInteger(props.attempt) && props.attempt > 0);
watch(() => [props.scanId, props.attempt, props.status], () => { generation++; busy.value = false; message.value = ""; error.value = ""; });
onUnmounted(() => { disposed = true; generation++; });

async function recoverResult() {
  if (!available.value || disposed) return;
  const action = ++generation, scanId = props.scanId, attempt = props.attempt;
  const current = () => !disposed && action === generation && props.scanId === scanId
    && props.attempt === attempt && props.status === "paused";
  busy.value = true; message.value = ""; error.value = "";
  try {
    const receipt = await sentinelApi.recoverNativeSourcePauseResult(scanId, attempt);
    if (!current()) return;
    if (receipt.schemaVersion !== 1 || receipt.scanId !== scanId || receipt.attemptNumber !== attempt
      || receipt.scanStatus !== "paused" || receipt.branchStatus !== "partial" || receipt.executionReplayed !== false
      || typeof receipt.rootRunId !== "string" || !receipt.rootRunId.trim() || typeof receipt.changed !== "boolean") {
      throw new Error("source_pause_recovery_receipt_mismatch");
    }
    message.value = receipt.changed
      ? tr('原结果已恢复为“未完成”；费用保留，任务继续暂停。', 'Original result restored as incomplete. Costs are retained and the task remains paused.')
      : tr('原结果已核对；任务保持暂停。', 'Original result verified; the task remains paused.');
  } catch (reason) {
    if (!current()) return;
    const detail = String(reason);
    error.value = /not_idle|worker_active|cleanup_unconfirmed/.test(detail)
      ? tr('原执行或清理尚未确认结束，请稍后核对。', 'Original execution or cleanup is not confirmed; review again later.')
      : /indeterminate|unknown|uncertain/.test(detail)
        ? tr('费用尚未确认，原结果和暂停状态已保留。', 'Costs are unsettled; the original result and paused state are retained.')
        : tr('原结果恢复未确认，请刷新核对原回执。', 'Original result recovery is unconfirmed; refresh and review its receipts.');
  } finally {
    if (current()) { busy.value = false; emit("refresh"); }
  }
}
</script>

<template>
  <aside v-if="eligible || busy || message || error" class="source-result-recovery" aria-label="源码暂停结果恢复">
    <div v-if="eligible" class="source-result-recovery-heading">
      <div><strong>{{ tr('待恢复的源码结果', 'Source result awaiting recovery') }}</strong>
        <p>{{ tr('源码结果尚未写回。核对原回执后恢复结果，任务仍保持暂停。', 'The source result has not been published. Restore it after checking its original receipts; the task remains paused.') }}</p></div>
      <button type="button" class="button small" :disabled="!available" @click="recoverResult">{{ busy ? tr('核对原回执…', 'Checking original receipts…') : tr('恢复原结果', 'Restore original result') }}</button>
    </div>
    <p v-if="message" class="source-result-recovery-message" role="status">{{ message }}</p>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
  </aside>
</template>

<style scoped>
.source-result-recovery { margin: 12px 0; padding: 14px 16px; border: 1px solid var(--native-line, var(--line, #e3e5e9)); border-radius: 12px; background: var(--native-tint, var(--app-surface, #fff)); }
.source-result-recovery-heading { display: flex; align-items: center; gap: 16px; justify-content: space-between; }
.source-result-recovery-heading strong { font-size: 13px; }
.source-result-recovery p { margin: 6px 0 0; line-height: 1.6; font-size: 12px; color: var(--native-muted, var(--app-muted, #717780)); }
.source-result-recovery .button { flex-shrink: 0; }
.source-result-recovery-message { color: var(--native-ink, var(--app-ink, #25282d)) !important; }
@media (max-width: 640px) { .source-result-recovery-heading { align-items: stretch; flex-direction: column; gap: 10px; } }
</style>
