<script setup lang="ts">
import { Activity } from "@lucide/vue";
import type { SentinelFinding } from "../../types";
import { json, text } from "../../presentation";

defineProps<{
  runtimeFeatureRows: SentinelFinding[];
  runtimeActionRows: SentinelFinding[];
  observedMutationRows: SentinelFinding[];
}>();
</script>

<template>
  <section class="result-block runtime-exploration-block">
    <div class="block-title">
      <Activity :size="17" />
      <div>
        <strong>自动探索轨迹</strong>
        <small>保留每个页面状态、触发动作及其新增请求；写操作只观察并中止，不自动提交。</small>
      </div>
      <div class="runtime-exploration-counts">
        <span>{{ runtimeFeatureRows.length }} 个状态</span>
        <span>{{ runtimeActionRows.length }} 次动作</span>
        <span>{{ observedMutationRows.length }} 个写请求</span>
      </div>
    </div>
    <div v-if="runtimeFeatureRows.length || runtimeActionRows.length" class="runtime-exploration-grid">
      <article class="runtime-state-column">
        <header>页面与功能状态</header>
        <div v-for="item in runtimeFeatureRows" :key="item.id" class="runtime-trace-card">
          <div>
            <b>{{ json(item.recordJson).stateId || item.title }}</b>
            <span>深度 {{ json(item.recordJson).depth ?? 0 }}</span>
          </div>
          <strong>{{ json(item.recordJson).title || "未命名页面" }}</strong>
          <code :title="json(item.recordJson).url">{{ json(item.recordJson).url }}</code>
          <p v-if="json(item.recordJson).highValueLabels?.length">
            高价值功能：{{ json(item.recordJson).highValueLabels.join("、") }}
          </p>
          <small>
            {{ json(item.recordJson).interactiveCount || 0 }} 个可触发控件 ·
            {{ json(item.recordJson).formCount || 0 }} 个表单
            <template v-if="json(item.recordJson).fieldNames?.length">
              · 字段 {{ json(item.recordJson).fieldNames.join("、") }}
            </template>
          </small>
        </div>
      </article>
      <article class="runtime-action-column">
        <header>触发动作与请求增量</header>
        <div v-for="item in runtimeActionRows" :key="item.id" class="runtime-trace-card">
          <div>
            <b>{{ json(item.recordJson).id || item.title }}</b>
            <span :class="{
              changed: json(item.recordJson).stateChanged,
              failed: json(item.recordJson).outcome === 'error',
            }">{{ json(item.recordJson).outcome || "observed" }}</span>
          </div>
          <strong>{{ json(item.recordJson).label || json(item.recordJson).role || "页面控件" }}</strong>
          <p>
            新增 {{ json(item.recordJson).requestCount || 0 }} 个请求 ·
            观察并拦截 {{ json(item.recordJson).blockedRequestCount || 0 }} 个写请求 ·
            {{ json(item.recordJson).durationMs || 0 }} ms
          </p>
          <code :title="json(item.recordJson).afterUrl">
            {{ json(item.recordJson).beforeUrl }}
            <template v-if="json(item.recordJson).afterUrl && json(item.recordJson).afterUrl !== json(item.recordJson).beforeUrl">
              → {{ json(item.recordJson).afterUrl }}
            </template>
          </code>
        </div>
      </article>
    </div>
    <div v-else class="empty-inline">
      当前是旧扫描记录或页面没有可触发控件；重新运行扫描后会生成轨迹。
    </div>
    <details v-if="observedMutationRows.length" class="runtime-mutation-details">
      <summary>查看 {{ observedMutationRows.length }} 个被观察并中止的写请求</summary>
      <div>
        <article v-for="item in observedMutationRows" :key="item.id">
          <b>{{ json(item.recordJson).method || "WRITE" }}</b>
          <code>{{ json(item.recordJson).url }}</code>
          <small>
            参数：{{ text(json(item.recordJson).bodyKeys || json(item.recordJson).queryKeys) || "未识别" }}
            · 来源动作 {{ json(item.recordJson).actionId || "—" }}
          </small>
          <pre v-if="json(item.recordJson).postData">{{ json(item.recordJson).postData }}</pre>
        </article>
      </div>
    </details>
  </section>
</template>
