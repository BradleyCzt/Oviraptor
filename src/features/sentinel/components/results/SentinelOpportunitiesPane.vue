<script setup lang="ts">
import { Fingerprint, ShieldAlert } from "@lucide/vue";
import type { SentinelOpportunity } from "../../types";

defineProps<{
  selectedUrlOpportunities: SentinelOpportunity[];
  opportunityCategoryLabel: (value: string) => string;
  opportunityStatusLabel: (value: string) => string;
  opportunityEndpoint: (item: SentinelOpportunity) => string;
  opportunityParameters: (item: SentinelOpportunity) => string[];
  opportunityKnowledge: (item: SentinelOpportunity) => any[];
  opportunityKnowledgeTitles: (item: SentinelOpportunity) => string;
  opportunityIdentityRows: (item: SentinelOpportunity) => Array<Record<string, any>>;
  opportunityIdentitySummary: (item: SentinelOpportunity) => string;
  openOpportunity: (item: SentinelOpportunity, markInProgress?: boolean) => Promise<void>;
  setOpportunityStatus: (item: SentinelOpportunity, status: string) => Promise<void>;
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block result-opportunity-panel">
      <div class="block-title">
        <ShieldAlert :size="16" />
        <div>
          <strong>为什么值得继续，以及下一步做什么</strong>
          <small>机会卡来自运行时请求、路由、指纹与本地知识匹配；不会伪装成漏洞结论。</small>
        </div>
      </div>
      <div class="opportunity-list detail-opportunity-list">
        <article
          v-for="item in selectedUrlOpportunities"
          :key="item.id"
          class="opportunity-card"
          :class="`score-${item.score >= 80 ? 'high' : item.score >= 65 ? 'medium' : 'low'}`"
        >
          <div class="opportunity-score"><strong>{{ item.score }}</strong><small>价值分</small></div>
          <div class="opportunity-content">
            <header>
              <span>{{ opportunityCategoryLabel(item.category) }}</span>
              <em :class="`opportunity-status ${item.status}`">{{ opportunityStatusLabel(item.status) }}</em>
              <small>{{ item.source }}</small>
            </header>
            <h4>{{ item.title }}</h4>
            <code class="opportunity-endpoint">{{ opportunityEndpoint(item) }}</code>
            <div v-if="opportunityIdentityRows(item).length" class="opportunity-identity-scope">
              <strong>身份范围</strong><span>{{ opportunityIdentitySummary(item) }}</span><span class="identity-compare-label">{{ opportunityIdentityRows(item).length > 1 ? "同一机会 · A/B 分栏" : "单账号证据" }}</span>
              <template v-for="row in opportunityIdentityRows(item)" :key="`${item.id}-${row.label}`">
                <em :class="`identity-chip ${row.tone}`" :title="row.identityKey || row.detail">{{ row.label }} · {{ row.state }}<small>{{ row.detail }}</small></em>
              </template>
            </div>
            <ul><li v-for="reason in item.why" :key="reason">{{ reason }}</li></ul>
            <div v-if="opportunityParameters(item).length" class="opportunity-params">
              <span>已还原参数</span><code v-for="parameter in opportunityParameters(item)" :key="parameter">{{ parameter }}</code>
            </div>
            <div v-if="opportunityKnowledge(item).length" class="opportunity-knowledge">
              <Fingerprint :size="14" /> 本地知识：{{ opportunityKnowledgeTitles(item) }}
            </div>
            <section class="opportunity-next-step">
              <strong>{{ item.recommendedAction?.label || '查看证据并选择验证方法' }}</strong>
              <ol>
                <li v-for="step in item.recommendedAction?.steps || []" :key="step">{{ step }}</li>
              </ol>
            </section>
            <details>
              <summary>原始机会记录 / 请求上下文</summary>
              <pre>{{ JSON.stringify(item.record, null, 2) }}</pre>
            </details>
            <footer>
              <span>{{ item.lastSeen }}</span>
              <div>
                <button class="button secondary small" @click="openOpportunity(item, true)">开始验证</button>
                <button class="button ghost small" @click="openOpportunity(item, true)">查看验证器</button>
                <button class="button ghost small" @click="setOpportunityStatus(item, 'exhausted')">无新增证据</button>
              </div>
            </footer>
          </div>
        </article>
        <div v-if="!selectedUrlOpportunities.length" class="empty-state">
          此 URL 暂无机会卡。旧任务需要重新执行前端侦察后才会生成自动探索与机会数据。
        </div>
      </div>
    </section>
  </div>
</template>
