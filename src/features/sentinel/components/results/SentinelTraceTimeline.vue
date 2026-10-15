<script setup lang="ts">
import { computed } from "vue";
import { Cpu, Wrench } from "@lucide/vue";
import type { AgentTraceDetail } from "../../types";
import { formatNumber } from "../../presentation";
import RootDecisionSummary from "../RootDecisionSummary.vue";
import { isRootModelRound } from "../../traces/rootDecisionContract";

const props = defineProps<{
  detail?: AgentTraceDetail;
  busy: boolean;
  targetUrl: string;
  taskStatus: string;
}>();
// These are recorded events, not a heartbeat or proof of a currently running tool.
const visible = computed(() => props.detail || props.busy || ["scanning", "pausing"].includes(props.taskStatus));
function normalizedTarget(value: string) {
  try {
    const parsed = new URL(value);
    return `${parsed.protocol}//${parsed.host}${parsed.pathname.replace(/\/+$/, "")}`;
  } catch {
    return value.trim().replace(/\/+$/, "").toLowerCase();
  }
}
const recentEvents = computed(() => (props.detail?.events || [])
  .filter(event => props.targetUrl === "*" || !event.targetUrl
    || normalizedTarget(event.targetUrl) === normalizedTarget(props.targetUrl))
  .slice(-30).reverse());
const latestEvent = computed(() => recentEvents.value[0]);
const sourceLabel = computed(() => props.detail?.summary.sourceAuthority === "historical_external"
  ? "历史外部记录 · 只读" : props.detail ? "Native 账本记录" : "等待轨迹记录");
function eventLabel(value: string) {
  return ({ function_call: "工具调用", function_call_output: "工具结果",
    reasoning: "分析阶段", message: "Agent 消息", model_round_completed: "模型轮次记录" } as Record<string, string>)[value] || value;
}
function eventTitle(event: AgentTraceDetail["events"][number]) {
  if (!event.name) return eventLabel(event.eventType);
  if (event.eventType === "function_call_output") return `${event.name} 返回结果`;
  if (event.eventType === "function_call") return `调用 ${event.name}`;
  return event.name;
}
const sessionLabel = (value: string) => value ? value.slice(0, 8) : "root";
</script>

<template>
  <section v-if="visible" class="result-block agent-live-chain">
    <div class="block-title">
      <Cpu :size="16" />
      <div>
        <strong>运行轨迹 · 事件记录</strong>
        <small>{{ sourceLabel }}；记录不代表智能体当前仍在执行相同步骤。</small>
      </div>
      <span v-if="busy" class="live-chain-state">刷新中</span>
    </div>
    <div class="live-chain-current">
      <span>最近记录</span>
      <strong>{{ latestEvent ? eventTitle(latestEvent) : "尚无可展示的结构化事件" }}</strong>
      <small v-if="latestEvent">
        Agent {{ sessionLabel(latestEvent.sessionId) }}
        <template v-if="latestEvent.targetUrl"> · {{ latestEvent.targetUrl }}</template>
        <template v-if="latestEvent.callId"> · 调用 {{ latestEvent.callId.slice(0, 12) }}</template>
        · {{ latestEvent.createdAt }}
      </small>
    </div>
    <template v-if="detail">
      <p class="live-chain-note">任务级累计统计，不随下方目标筛选变化。</p>
      <div class="live-chain-metrics">
        <article><span>模型请求</span><strong>{{ detail.summary.llmRequests }}</strong></article>
        <article><span>工具调用 / 返回</span><strong>{{ detail.summary.toolCallCount }} / {{ detail.summary.toolResultCount }}</strong></article>
        <article><span>Agent</span><strong>{{ detail.summary.agentCount }}</strong></article>
        <article><span>总 Token</span><strong>{{ formatNumber(detail.summary.totalTokens) }}</strong></article>
      </div>
      <div v-if="detail.summary.tools.length" class="live-chain-tools">
        <span v-for="tool in detail.summary.tools" :key="tool.name">
          <Wrench :size="11" /><b>{{ tool.name }}</b><em>{{ tool.calls }} 调用 / {{ tool.results }} 返回</em>
        </span>
      </div>
    </template>
    <p class="live-chain-note">按接口返回顺序展示当前筛选最近 30 条，最新记录在前；包含未标注目标的任务级事件。</p>
    <div v-if="recentEvents.length" class="live-chain-events">
      <article v-for="(event, index) in recentEvents" :key="event.id" :class="event.eventType">
        <header>
          <span>{{ eventLabel(event.eventType) }}</span><strong>{{ eventTitle(event) }}</strong>
          <em>Agent {{ sessionLabel(event.sessionId) }} · {{ isRootModelRound(event) ? "已记录" : event.status || event.role || "recorded" }}</em>
          <time>{{ event.createdAt }}</time>
        </header>
        <RootDecisionSummary v-if="isRootModelRound(event)" :event="event" :source-authority="detail?.summary.sourceAuthority || ''" />
        <details v-else-if="event.detail" :open="index === 0">
          <summary>
            {{ event.eventType === "function_call" ? "查看调用参数"
              : event.eventType === "function_call_output" ? "查看返回摘要" : "查看阶段摘要" }}
            <span v-if="event.detailTruncated">· 限长预览</span>
          </summary>
          <pre>{{ event.detail }}</pre>
        </details>
      </article>
    </div>
    <div v-else class="empty-inline live-chain-empty">
      {{ busy ? "正在读取 Agent 结构化事件…" : "当前筛选尚无可展示的结构化事件；不能据此判断任务进展或完成情况。" }}
    </div>
    <p class="live-chain-note">详情为后端过滤后的展示预览，不是原始模型请求或私有推理；未标记的自由文本仍需谨慎处理。</p>
  </section>
</template>

<style src="../sentinelTracePresentation.css"></style>
