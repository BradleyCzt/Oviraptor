<script setup lang="ts">
import { Eye, Save } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelValidationWorkItem } from "../types";
import {
  createSentinelLabels,
  json,
  safeSeverity,
  text,
} from "../presentation";

defineProps<{
  filter: string;
  stats: { pending: number; confirmed: number; rejected: number };
  items: SentinelValidationWorkItem[];
  editor?: SentinelValidationWorkItem;
  form: { verdict: string; severity: string; note: string; evidence: string };
}>();
const emit = defineEmits<{
  "update:filter": [value: string];
  select: [item: SentinelValidationWorkItem];
  evidence: [item: SentinelValidationWorkItem];
  save: [];
}>();
const { tr } = useI18n();
const { severityLabel, verdictLabel } = createSentinelLabels(tr);
</script>

<template>
  <section class="panel sentinel-panel validation-workbench-panel">
    <div class="panel-heading">
      <div>
        <span class="eyebrow">VALIDATION WORKBENCH</span>
        <h3>漏洞验证工作台</h3>
        <p>把扫描发现和人工结论放在同一条队列；未验证、需补证和已定性的漏洞都不会再丢在结果页里。</p>
      </div>
      <select :value="filter" class="toolbar-select" @change="emit('update:filter', ($event.target as HTMLSelectElement).value)">
        <option value="pending">待处理与需补证</option>
        <option value="true_positive">真实漏洞</option>
        <option value="false_positive">误报</option>
        <option value="all">全部漏洞</option>
      </select>
    </div>
    <div class="validation-work-stats">
      <article class="attention"><span>待处理 / 补证</span><strong>{{ stats.pending }}</strong></article>
      <article><span>确认真实漏洞</span><strong>{{ stats.confirmed }}</strong></article>
      <article><span>确认误报</span><strong>{{ stats.rejected }}</strong></article>
      <article><span>当前显示</span><strong>{{ items.length }}</strong></article>
    </div>
    <div class="validation-workbench-layout">
      <div class="validation-work-queue">
        <button
          v-for="item in items"
          :key="item.findingId"
          :class="{ active: editor?.findingId === item.findingId }"
          @click="emit('select', item)"
        >
          <span :class="`severity-badge ${safeSeverity(item.confirmedSeverity || item.originalSeverity)}`">{{ severityLabel(item.confirmedSeverity || item.originalSeverity) }}</span>
          <div>
            <strong>{{ item.title || "未命名漏洞" }}</strong>
            <code>{{ item.url || "全局 / 源码发现" }}</code>
            <small>{{ item.projectName }} · {{ item.taskName || item.scanId }} · {{ item.updatedAt }}</small>
          </div>
          <span :class="`validation-chip ${item.validationId ? item.verdict : 'pending'}`">{{ item.validationId ? verdictLabel(item.verdict) : "待验证" }}</span>
        </button>
        <div v-if="!items.length" class="empty-state">
          当前筛选下没有待处理漏洞；切换到“全部漏洞”可查看历史结论。
        </div>
      </div>
      <aside class="validation-work-detail">
        <template v-if="editor">
          <header>
            <div><span class="eyebrow">SELECTED FINDING</span><h3>{{ editor.title || "未命名漏洞" }}</h3></div>
            <button class="button ghost compact" @click="emit('evidence', editor)"><Eye :size="13" />打开完整证据</button>
          </header>
          <dl class="validation-context-grid">
            <div><dt>项目 / 任务</dt><dd>{{ editor.projectName }} · {{ editor.taskName || editor.scanId }}</dd></div>
            <div><dt>目标</dt><dd><code>{{ editor.url || "全局 / 源码发现" }}</code></dd></div>
            <div><dt>扫描严重度</dt><dd>{{ severityLabel(editor.originalSeverity) }}</dd></div>
            <div><dt>Finding Key</dt><dd><code>{{ editor.findingKey }}</code></dd></div>
          </dl>
          <div class="validation-evidence-preview">
            <article><span>漏洞描述</span><p>{{ json(editor.recordJson).description || json(editor.recordJson).detail || "扫描结果未提供结构化描述" }}</p></article>
            <article><span>关键证据 / 复现</span><pre>{{ text(json(editor.recordJson).evidence || json(editor.recordJson).pocRequest || json(editor.recordJson).poc_description || "尚无证据摘要，请打开完整证据查看上下文") }}</pre></article>
          </div>
          <div class="verdict-picker validation-work-verdicts">
            <button v-for="choice in [{ v: 'true_positive', l: '确认真实漏洞' }, { v: 'false_positive', l: '确认误报' }, { v: 'needs_more', l: '需要补证' }]" :key="choice.v" :class="{ active: form.verdict === choice.v }" @click="form.verdict = choice.v">{{ choice.l }}</button>
          </div>
          <div class="validation-form-grid validation-work-form">
            <label class="field"><span>确认后严重度</span><select v-model="form.severity"><option value="critical">严重</option><option value="high">高危</option><option value="medium">中危</option><option value="low">低危</option><option value="info">信息</option></select></label>
            <label class="field"><span>结论与复现备注</span><textarea v-model="form.note" rows="4" placeholder="写清复现步骤、判断理由、限制条件和下一步"></textarea></label>
            <label class="field span-two"><span>请求响应 / 截图 / 本地证据路径</span><textarea v-model="form.evidence" rows="6" placeholder="粘贴关键请求响应，或填写本地证据文件路径"></textarea></label>
          </div>
          <footer><span>保存会立即更新风险统计，不会修改扫描原始证据。</span><button class="button primary" @click="emit('save')"><Save :size="14" />保存漏洞结论</button></footer>
        </template>
        <div v-else class="empty-state">从左侧选择一个漏洞开始验证。</div>
      </aside>
    </div>
  </section>
</template>
