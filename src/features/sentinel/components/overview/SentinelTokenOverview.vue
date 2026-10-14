<script setup lang="ts">
import { useI18n } from "../../../../i18n";
import type { SentinelScan } from "../../types";
import type { TokenUsageScope, TokenUsageSummary } from "../../overview/useTokenUsage";
import { createSentinelLabels, formatCompactNumber, formatNumber, scanTitle, scanTokenTotal } from "../../presentation";

defineProps<{ scope: TokenUsageScope; usage: TokenUsageSummary }>();
const emit = defineEmits<{
  "update:scope": [scope: TokenUsageScope];
  open: [scan: SentinelScan];
}>();
const { tr } = useI18n();
const { scanTypeLabel } = createSentinelLabels(tr);
</script>

<template>
  <section class="panel sentinel-token-overview">
    <div class="token-overview-heading">
      <div>
        <span class="eyebrow">TOKEN ACCOUNTING</span>
        <h3>{{ tr("模型 Token 用量", "Model token usage") }}</h3>
      </div>
      <div class="token-scope-switch segmented" role="group" :aria-label="tr('模型部署范围', 'Model deployment scope')">
        <button type="button" :class="{ active: scope === 'all' }" :aria-pressed="scope === 'all'"
          @click="emit('update:scope', 'all')">{{ tr("全部", "All") }}</button>
        <button type="button" :class="{ active: scope === 'cloud' }" :aria-pressed="scope === 'cloud'"
          @click="emit('update:scope', 'cloud')">{{ tr("云端 AI", "Cloud AI") }}</button>
        <button type="button" :class="{ active: scope === 'local' }" :aria-pressed="scope === 'local'"
          @click="emit('update:scope', 'local')">{{ tr("本地模型", "Local model") }}</button>
      </div>
      <p>{{ tr("仅统计已加载任务，并按上方部署范围筛选；不是全库累计。缓存是输入的一部分。", "Only loaded tasks in the selected deployment scope; not database-wide totals. Cached tokens are part of input.") }}</p>
    </div>
    <div class="token-summary-grid">
      <article>
        <span>{{ tr("输入总计", "Input total") }}</span><strong>{{ formatNumber(usage.totalInputTokenUsage) }}</strong>
      </article>
      <article class="cached">
        <span>{{ tr("其中缓存输入", "Cached input") }}</span><strong>{{ formatNumber(usage.totalCachedTokenUsage) }}</strong>
      </article>
      <article class="new-input">
        <span>{{ tr("新增输入", "Uncached input") }}</span><strong>{{ formatNumber(usage.totalUncachedInputUsage) }}</strong>
      </article>
      <article class="output">
        <span>{{ tr("输出", "Output") }}</span><strong>{{ formatNumber(usage.totalOutputTokenUsage) }}</strong>
      </article>
      <article class="total">
        <span>{{ tr("输入 + 输出总计", "Input + output") }}</span><strong>{{ formatNumber(usage.totalTokenUsage) }}</strong>
      </article>
      <article class="requests">
        <span>{{ tr("模型请求", "Model requests") }}</span><strong>{{ formatNumber(usage.totalRequestUsage) }}</strong>
      </article>
    </div>
    <div class="cost-decision-grid">
      <article>
        <span>缓存命中率</span><strong>{{ usage.cacheHitRate }}%</strong>
        <small>{{ tr("缓存输入 / 输入总计；不作为执行质量结论", "Cached input / total input; not an execution-quality verdict") }}</small>
      </article>
      <article>
        <span>{{ tr("确认记录单位成本", "Tokens per confirmed record") }}</span><strong>{{ tr("暂不可计算", "Unavailable") }}</strong>
        <small>{{ tr("消耗与确认数缺少同范围、同快照的聚合，不能相除。", "Usage and confirmations lack a same-scope, same-snapshot aggregate; no ratio is reported.") }}</small>
      </article>
      <article :class="{ warning: usage.zeroYieldScans.length }">
        <span>{{ tr("未关联漏洞记录", "No linked finding records") }}</span><strong>{{ usage.zeroYieldScans.length }}</strong>
        <small>{{ formatNumber(usage.zeroYieldTokenUsage) }} Token · {{ tr("已消耗 Token 的结束状态任务；当前索引未关联记录，不代表无漏洞或执行完整。", "Ended tasks with token usage but no records in the current association index; not proof of no vulnerabilities or complete execution.") }}</small>
      </article>
      <article v-if="usage.highestCostScan" class="highest-cost">
        <span>最高成本任务</span><strong>{{ formatCompactNumber(scanTokenTotal(usage.highestCostScan)) }}</strong>
        <small>{{ scanTitle(usage.highestCostScan) }}</small>
        <button type="button" class="text-button" @click="emit('open', usage.highestCostScan)">查看任务证据</button>
      </article>
    </div>
    <div class="token-type-grid">
      <article v-for="item in usage.tokenTypeRows" :key="item.type">
        <header><span>{{ scanTypeLabel(item.type) }}</span><strong>{{ formatNumber(item.total) }}</strong></header>
        <dl>
          <div><dt>{{ tr("输入", "Input") }}</dt><dd>{{ formatNumber(item.input) }}</dd></div>
          <div><dt>{{ tr("缓存", "Cached") }}</dt><dd>{{ formatNumber(item.cached) }}</dd></div>
          <div><dt>{{ tr("新增输入", "Uncached") }}</dt><dd>{{ formatNumber(item.uncachedInput) }}</dd></div>
          <div><dt>{{ tr("输出", "Output") }}</dt><dd>{{ formatNumber(item.output) }}</dd></div>
          <div><dt>{{ tr("请求", "Requests") }}</dt><dd>{{ formatNumber(item.requests) }}</dd></div>
        </dl>
      </article>
    </div>
  </section>
</template>
