<script setup lang="ts">
import { Fingerprint, Layers3, Server, Shield } from "@lucide/vue";
import type { SentinelFinding } from "../../types";
import {
  displayName,
  displayVersion,
  json,
  kindLabel,
  text,
} from "../../presentation";

defineProps<{
    fingerprintCards: Array<{ key: string; label: string; data: Record<string, any> }>;
  securityHeaders: Array<{ item: SentinelFinding; data: Record<string, any> }>;
  techStack: Record<string, any>;
  wordpress: Record<string, any>;
  rows: (...kinds: string[]) => SentinelFinding[];
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block kind-fingerprint">
      <div class="block-title">
        <Server :size="16" />
        <div>
          <strong>技术栈详情</strong
          ><small
            >名称和证据已规范化；“未识别”表示没有足够证据，不等于不存在。</small
          >
        </div>
      </div>
      <div class="fingerprint-detail-list">
        <article
          v-for="card in fingerprintCards"
          :key="card.key"
          :class="`tone-${card.key}`"
        >
          <header>
            <span>{{ card.label }}</span
            ><strong
              >{{ displayName(card.data) }}
              <small>{{ displayVersion(card.data) }}</small></strong
            ><em>{{ card.data?.confidence || "unknown" }}</em>
          </header>
          <div
            v-if="card.data?.libraries?.length"
            class="fingerprint-tags"
          >
            <span
              v-for="library in card.data.libraries"
              :key="library.name"
              >{{ library.name }} {{ library.version || "" }}</span
            >
          </div>
          <div
            v-if="card.data?.buildTools?.length"
            class="fingerprint-tags"
          >
            <span
              v-for="tool in card.data.buildTools"
              :key="tool"
              >{{ tool }}</span
            >
          </div>
          <p v-if="card.data?.evidence?.length">
            识别依据：{{ card.data.evidence.join("；") }}
          </p>
        </article>
        <article v-if="techStack.baseUrls?.length" class="tone-api">
          <header>
            <span>API 基础地址</span
            ><strong>{{ techStack.baseUrls.length }} 个</strong>
          </header>
          <div class="fingerprint-tags">
            <span v-for="url in techStack.baseUrls" :key="url">{{
              url
            }}</span>
          </div>
        </article>
      </div>
    </section>
    <section
      v-if="Object.keys(wordpress).length"
      class="result-block"
    >
      <div class="block-title">
        <Fingerprint :size="16" />
        <div>
          <strong>WordPress</strong
          ><small>版本、插件、主题与入口</small>
        </div>
      </div>
      <div class="wordpress-summary">
        <article>
          <span>核心版本</span
          ><strong>{{ text(wordpress.version) }}</strong>
        </article>
        <article>
          <span>主题</span
          ><strong
            >{{ text(wordpress.theme?.name) }}
            {{ text(wordpress.theme?.version) }}</strong
          >
        </article>
        <article>
          <span>插件</span
          ><strong>{{ wordpress.plugins?.length || 0 }}</strong>
        </article>
        <article>
          <span>REST / XML-RPC</span
          ><strong
            >{{
              wordpress.restApiEnabled ? "REST 开启" : "REST 未知"
            }}
            ·
            {{
              wordpress.xmlrpcEnabled
                ? "XML-RPC 开启"
                : "XML-RPC 关闭"
            }}</strong
          >
        </article>
      </div>
      <div class="plugin-list">
        <span
          v-for="plugin in wordpress.plugins || []"
          :key="plugin.name"
          ><b>{{ plugin.name }}</b
          >{{ plugin.version || "未知版本" }}</span
        >
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>安全响应头</strong
          ><small>橙色表示配置缺失，不直接判定为漏洞。</small>
        </div>
      </div>
      <div class="security-header-table">
        <div class="table-head">
          <span>响应头</span><span>状态</span><span>当前值</span
          ><span>修复建议</span>
        </div>
        <div v-for="row in securityHeaders" :key="row.item.id">
          <strong>{{ row.item.title }}</strong
          ><span
            :class="
              row.data.present ? 'config-ok' : 'config-missing'
            "
            >{{ row.data.present ? "已配置" : "缺失" }}</span
          ><code>{{ row.data.value || "—" }}</code>
          <p>{{ row.data.recommendation || "—" }}</p>
        </div>
        <div v-if="!securityHeaders.length" class="empty-inline">
          没有安全头数据
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div><strong>Cookie / 外部服务 / 信息披露</strong></div>
      </div>
      <div class="compact-record-grid">
        <article
          v-for="item in rows(
            'cookie',
            'external_service',
            'info_disclosure',
            'open_port',
          )"
          :key="item.id"
        >
          <span>{{ kindLabel(item.kind) }}</span
          ><strong>{{ item.title || item.recordKey }}</strong>
          <pre>{{
            JSON.stringify(json(item.recordJson), null, 2)
          }}</pre>
        </article>
        <div
          v-if="
            !rows(
              'cookie',
              'external_service',
              'info_disclosure',
              'open_port',
            ).length
          "
          class="empty-inline"
        >
          暂无记录
        </div>
      </div>
    </section>
  </div>
</template>
