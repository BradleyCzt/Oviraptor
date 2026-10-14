<script setup lang="ts">
import { ClipboardCopy, FileJson } from "@lucide/vue";
import type { SentinelFinding } from "../../types";
import { formatNumber, json, scriptTone } from "../../presentation";

defineProps<{
  jsRows: SentinelFinding[];
  copyText: (value: string) => Promise<void>;
}>();
</script>

<template>
  <section class="result-block kind-js-file">
    <div class="block-title">
      <FileJson :size="16" />
      <div>
        <strong>JS 文件</strong
        ><small
          >业务包深度分析；runtime 只发现分包；vendor
          与公共依赖不进入原生 Agent。</small
        >
      </div>
    </div>
    <div class="js-file-list">
      <article
        v-for="item in jsRows"
        :key="item.id"
        :class="scriptTone(json(item.recordJson).type)"
      >
        <header>
          <span>{{ json(item.recordJson).type || "script" }}</span
          ><b>{{
            json(item.recordJson).statusCode ||
            json(item.recordJson).priority ||
            "info"
          }}</b>
        </header>
        <div class="long-value-cell">
          <code
            class="scroll-value"
            :title="json(item.recordJson).url"
            >{{ json(item.recordJson).url }}</code
          ><button
            class="icon-button compact"
            title="复制 JS 地址"
            @click="copyText(json(item.recordJson).url)"
          >
            <ClipboardCopy :size="13" />
          </button>
        </div>
        <p>
          {{ formatNumber(json(item.recordJson).size || 0) }}
          bytes ·
          {{ json(item.recordJson).isMinified ? "已压缩" : "未压缩" }}
          <template v-if="json(item.recordJson).discoveredFrom">
            · 来源
            {{
              json(item.recordJson).discoveredFrom === "html"
                ? "HTML"
                : json(item.recordJson).discoveredFrom
            }}
          </template>
        </p>
        <div v-if="json(item.recordJson).analysis" class="js-analysis-tags">
          <span :class="{ active: json(item.recordJson).analysis.sourceMapReference }"
            >Source Map
            {{ json(item.recordJson).analysis.sourceMapReference ? "存在" : "未发现" }}</span
          ><span :class="{ active: json(item.recordJson).analysis.module }"
            >ES Module {{ json(item.recordJson).analysis.module ? "是" : "否" }}</span
          ><span v-if="json(item.recordJson).analysis.moduleCount" class="active"
            >模块 {{ json(item.recordJson).analysis.moduleCount }}</span
          ><span v-if="json(item.recordJson).analysis.businessScore" class="active"
            >业务信号 {{ json(item.recordJson).analysis.businessScore }}</span
          ><span>{{ json(item.recordJson).analysis.extractionEngine || "inventory" }}</span>
        </div>
        <p v-if="json(item.recordJson).error" class="form-error">
          {{ json(item.recordJson).error }}
        </p>
      </article>
      <div v-if="!jsRows.length" class="empty-inline">没有 JS 分析记录</div>
    </div>
  </section>
</template>
