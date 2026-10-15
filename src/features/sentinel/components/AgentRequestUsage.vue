<script setup lang="ts">
import { computed } from "vue";
import type { AgentRequestAccounting } from "../types";

const props = defineProps<{ accounting?: unknown }>();
const usage = computed(() => {
  const value = props.accounting as Record<string, unknown> | null;
  const count = (n: unknown): n is number => Number.isSafeInteger(n) && Number(n) >= 0;
  if (!value || value.available !== true || value.scope !== "agent_budget_lineage") return null;
  if (value.includesDeterministicRecon !== false || value.automaticReplayAllowed !== false) return null;
  const fields = [value.executorRecordedRequests, value.externalSurfaceReceivedRequests,
    value.externalSurfaceUnresolvedClaims, value.recordedRequests, value.budgetCommittedRequests,
    value.authorizationReceivedRequests, value.authorizationUnresolvedClaims];
  if (!fields.every(count) || value.recordedRequests !== fields[0] + fields[1] + fields[5]
    || value.budgetCommittedRequests !== fields[3] + fields[2] + fields[6]) return null;
  if (value.executorUnresolvedClaims !== undefined
    && (!count(value.executorUnresolvedClaims) || value.executorUnresolvedClaims > fields[0])) return null;
  const attempts = value.includedAttempts;
  if (!Array.isArray(attempts) || !attempts.length || !attempts.every((n) => count(n) && n > 0)
    || new Set(attempts).size !== attempts.length || value.attemptNumber !== attempts[0]
    || attempts.some((n, i) => i > 0 && n >= attempts[i - 1])) return null;
  return value as Extract<AgentRequestAccounting, { available: true }>;
});
</script>

<template>
  <section class="agent-request-usage" aria-label="Agent 目标请求统计">
    <template v-if="usage">
      <strong>已记录合计 {{ usage.recordedRequests }}</strong>
      <span>执行器账本 {{ usage.executorRecordedRequests }} · 公开面已收到响应 {{ usage.externalSurfaceReceivedRequests }}</span>
      <small v-if="usage.executorUnresolvedClaims !== undefined && usage.executorUnresolvedClaims > 0">执行器未决记录 {{ usage.executorUnresolvedClaims }}，已包含在执行器账本和预算中，不重复相加；不能证明已发送或未发送，不自动重试。</small>
      <span>授权对照已记录响应 {{ usage.authorizationReceivedRequests }} · 授权对照未决占用 {{ usage.authorizationUnresolvedClaims }}</span>
      <span>公开面未决占用 {{ usage.externalSurfaceUnresolvedClaims }} · 预算已占用 {{ usage.budgetCommittedRequests }}</span>
      <small>预算范围：尝试 {{ usage.includedAttempts.join("、") }}。不含前置确定性侦察；已记录不代表请求均成功。</small>
      <small v-if="usage.externalSurfaceUnresolvedClaims + usage.authorizationUnresolvedClaims > 0">未决项可能已发送，也可能未发送；只占用预算，不计入已记录合计，不自动重试。</small>
      <details v-if="(usage.executorUnresolvedClaims ?? 0) + usage.externalSurfaceUnresolvedClaims + usage.authorizationUnresolvedClaims > 0"
        aria-label="未决请求核对说明">
        <summary>如何处理未决请求</summary>
        <p>执行中的请求也可能暂时没有回执，本统计不表示任务已经停止。若执行分支因结果未知而停止，请先核对目标访问日志、应用审计日志及本地工具调用记录。</p>
        <p>未决占用不会退回预算；收到响应头也不代表响应体、证据或目标效果已确认。不要通过重复发送来确认，也不要仅因没有找到日志就认定请求未发送。</p>
        <p>可在请求核对记录中保存人工判断和依据，但没有结算退款或忽略后继续的入口。保留原任务记录，明确结果和后续授权后再决定是否创建新任务；新任务也不等于旧请求已解决。</p>
      </details>
    </template>
    <span v-else role="status">请求统计暂不可核验，不显示为 0，也不会自动重试。</span>
  </section>
</template>

<style scoped>
.agent-request-usage { display: grid; gap: 4px; overflow-wrap: anywhere; }
.agent-request-usage small { opacity: .75; line-height: 1.5; }
</style>
