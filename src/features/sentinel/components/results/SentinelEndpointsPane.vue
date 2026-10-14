<script setup lang="ts">
import { Network } from "@lucide/vue";
import type { SentinelFinding } from "../../types";
import { endpointUrl, json, kindLabel, statusTone } from "../../presentation";

defineProps<{
  endpointRows: SentinelFinding[];
  selectedUrl: string;
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block">
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>已验证端点</strong
          ><small>状态码颜色只表示 HTTP 响应状态。</small>
        </div>
      </div>
      <div class="endpoint-table">
        <div class="table-head">
          <span>状态</span><span>方法</span><span>完整 URL</span
          ><span>来源</span><span>耗时 / 大小</span
          ><span>说明</span>
        </div>
        <div v-for="item in endpointRows" :key="item.id">
          <b
            :class="`http-status ${statusTone(json(item.recordJson).statusCode)}`"
            >{{ json(item.recordJson).statusCode || "—" }}</b
          ><span class="method-badge">{{
            json(item.recordJson).method || "GET"
          }}</span
          ><code>{{
            endpointUrl(
              selectedUrl,
              json(item.recordJson).url ||
                json(item.recordJson).path ||
                "/",
            )
          }}</code
          ><span>{{
            json(item.recordJson).source || kindLabel(item.kind)
          }}</span
          ><span
            >{{ json(item.recordJson).responseTime || "—" }} ms ·
            {{ json(item.recordJson).bodyLength || "—" }} B</span
          >
          <p>
            {{
              json(item.recordJson).note ||
              json(item.recordJson).detail ||
              json(item.recordJson).bodySnippet ||
              "—"
            }}
          </p>
        </div>
        <div v-if="!endpointRows.length" class="empty-inline">
          没有端点验证数据
        </div>
      </div>
    </section>
  </div>
</template>
