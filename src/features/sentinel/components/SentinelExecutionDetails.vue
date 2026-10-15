<script setup lang="ts">
import type { AgentTargetExecution } from "../types";
import { agentBackendLabel, agentModeLabel } from "../execution/presentation";
import AgentRequestUsage from "./AgentRequestUsage.vue";
import AgentRequestReview from "./AgentRequestReview.vue";

const props = defineProps<{
  execution: AgentTargetExecution;
  scope?: { scanId: string; targetUrl: string } | null;
  scanId?: string;
  targetUrl: string;
}>();

// An unfinished family must never look like a completed negative result.
function gapLabel(gap: { status: string; reasonCode?: string }) {
  if (gap.status === "not_applicable") return "不适用";
  if (gap.reasonCode === "tested_no_finding") return "已测，未发现";
  if (/seed_budget_exhausted|budget|预算|上限|限流/.test(String(gap.reasonCode || ""))) return "因预算未完成";
  if (gap.status === "insufficient_evidence") return "证据不足";
  return "未覆盖";
}

function familyStatus(family: string) {
  const coverage = props.execution.coverage;
  if (!coverage) return "";
  if (coverage.covered.includes(family)) return "已覆盖";
  const gap = (coverage.ledger?.uncoveredFamilies || []).find((row) => row.family === family);
  return gap ? gapLabel(gap) : "未覆盖";
}
</script>

<template>
  <details class="agent-execution-details" :open="execution.backend === 'native'">
    <summary>
      执行计划 · {{ agentBackendLabel(String(execution.backend)) }} ·
      {{ agentModeLabel(String(execution.mode)) }} · {{ execution.targetStatusText }}
    </summary>
    <div class="agent-execution-grid">
      <section>
        <h4>覆盖收口</h4>
        <ul>
          <li v-for="(label, index) in execution.coverage?.requiredLabels || []" :key="label">
            <span>{{ label }}</span>
            <em>{{ familyStatus((execution.coverage?.required || [])[index]) }}</em>
          </li>
        </ul>
        <p v-if="execution.coverage && !execution.coverage.ledgerReported">
          所选后端未输出覆盖账本；完成比例仅按已记录证据估算
        </p>
        <p v-else-if="execution.coverage">
          完成比例 {{ Math.round(execution.coverage.completedRatio * 100) }}% ·
          确认问题 {{ execution.coverage.confirmedFindings }}
        </p>
        <ul v-if="execution.coverage?.ledger?.uncoveredFamilies?.length">
          <li v-for="gap in execution.coverage.ledger.uncoveredFamilies" :key="gap.family">
            <span>{{ gap.label }}（{{ gapLabel(gap) }}）</span>
            <em>{{ gap.reason }}</em>
          </li>
        </ul>
      </section>
      <section>
        <h4>预算与消耗</h4>
        <dl>
          <dt>软预算 · 已用</dt>
          <dd>
            {{ execution.budgets?.softUncachedTokens || 0 }} Token ·
            {{ execution.runtime?.tokenUsage?.inputTokens || 0 }} 输入 /
            {{ execution.runtime?.tokenUsage?.outputTokens || 0 }} 输出
          </dd>
          <dt>硬上限 · 已用</dt>
          <dd>
            {{ execution.hardLimits?.hardTotalTokens || 0 }} Token ·
            {{ execution.runtime?.tokenUsage?.totalTokens || 0 }} 已消耗
          </dd>
          <dt>模型调用</dt>
          <dd>
            软 {{ execution.budgets?.softModelRequests || 0 }} / 硬
            {{ execution.hardLimits?.hardModelRequests || 0 }} ·
            实际 {{ execution.runtime?.tokenUsage?.modelRequests || 0 }}
          </dd>
          <dt>Agent 目标请求</dt>
          <dd>
            <AgentRequestUsage :accounting="execution.runtime?.requestAccounting" />
            <AgentRequestReview
              v-if="scope && scope.scanId === scanId && scope.targetUrl === targetUrl
                && execution.runtime?.requestAccounting?.attemptNumber"
              :scan-id="scope.scanId" :target-url="scope.targetUrl"
              :attempt-number="execution.runtime.requestAccounting.attemptNumber"
            />
          </dd>
          <dt>发现轮次</dt>
          <dd>{{ execution.runtime?.budgetUsage?.discoveryRounds || 0 }}</dd>
          <dt>回合数</dt>
          <dd>{{ execution.runtime?.turns || 0 }} / {{ execution.hardLimits?.maxTurns || 0 }}</dd>
          <dt>最近一次扩容原因</dt>
          <dd>{{ execution.runtime?.lastExpansionReason || "未扩容" }}</dd>
        </dl>
      </section>
      <section>
        <h4>当前动作与终态</h4>
        <dl>
          <dt>当前动作</dt>
          <dd>{{ execution.runtime?.currentAction || "尚未记录当前动作" }}</dd>
          <dt>最近新证据签名</dt>
          <dd>{{ execution.runtime?.progressSignature || "尚无" }}</dd>
          <dt>无进展计数</dt>
          <dd>
            {{ execution.runtime?.noProgressStreak || 0 }} /
            {{ execution.hardLimits?.noProgressWindow || 0 }}
          </dd>
          <dt>最终停止原因</dt>
          <dd>{{ execution.runtime?.terminalReason || "尚未结束" }}</dd>
        </dl>
        <ul v-if="execution.coverage?.ledger?.manualDeepDiveSuggestions?.length">
          <li v-for="tip in execution.coverage.ledger.manualDeepDiveSuggestions" :key="tip">
            人工深入建议：{{ tip }}
          </li>
        </ul>
      </section>
    </div>
  </details>
</template>

<style scoped src="./executionPresentation.css"></style>
