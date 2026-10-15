<script setup lang="ts">
import {
  Activity,
  ChevronDown,
  ClipboardCopy,
  Code2,
  ExternalLink,
  Network,
  Shield,
  ShieldAlert,
} from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import SentinelJsFiles from "./SentinelJsFiles.vue";
import SentinelRequestHeaders from "./SentinelRequestHeaders.vue";
import SentinelRuntimeExploration from "./SentinelRuntimeExploration.vue";
import type { SentinelFinding } from "../../types";
import {
  createSentinelLabels,
  cryptoCategory,
  isHttpUrl,
  json,
  kindLabel,
  methodTone,
  safeSeverity,
  sensitiveType,
  text,
} from "../../presentation";

const { severityLabel } = createSentinelLabels(useI18n().tr);

defineProps<{
apiRows: SentinelFinding[];
  cryptoRows: SentinelFinding[];
  declaredRequestHeaderRows: Array<Record<string, any>>;
  observedRequestHeaderRows: Array<Record<string, any>>;
  possibleRequestHeaderRows: Array<Record<string, any>>;
  expandedApiRows: number[];
  expandedSensitive: number[];
  jsRows: SentinelFinding[];
  observedMutationRows: SentinelFinding[];
  realtimeEndpointRows: SentinelFinding[];
  registrationRows: SentinelFinding[];
  requestHeaderIntelligence: Record<string, any>;
  routeRows: SentinelFinding[];
  runtimeActionRows: SentinelFinding[];
  runtimeFeatureRows: SentinelFinding[];
  runtimeRows: SentinelFinding[];
  sensitiveRows: SentinelFinding[];
  apiUrl: (item: SentinelFinding) => string;
  apiPath: (item: SentinelFinding) => string;
  apiQuery: (item: SentinelFinding) => string[];
  apiRecord: (item: SentinelFinding) => Record<string, any>;
  apiMethod: (item: SentinelFinding) => string;
  apiResponseSummary: (item: SentinelFinding) => string;
  apiSourceSummary: (item: SentinelFinding) => string;
  apiDescription: (item: SentinelFinding) => string;
  apiRequestPayload: (item: SentinelFinding) => any;
  apiResponseHeaders: (item: SentinelFinding) => any;
  apiIdentitySummary: (item: SentinelFinding) => string;
  registrationData: (item: SentinelFinding) => Record<string, any>;
  runtimeSignalUrl: (item: SentinelFinding) => string;
  headerDisplayValue: (row: Record<string, any>) => string;
  copyText: (value: string) => Promise<void>;
  openTargetUrl: (url: string) => Promise<void>;
  toggleApiRow: (id: number) => void;
  toggleSensitive: (id: number) => void;
}>();
</script>

<template>
  <div class="result-section-stack">
    <SentinelRuntimeExploration
      :runtime-feature-rows="runtimeFeatureRows"
      :runtime-action-rows="runtimeActionRows"
      :observed-mutation-rows="observedMutationRows"
    />
    <section
      v-if="registrationRows.length"
      class="result-block registration-alert"
    >
      <div class="block-title">
        <ShieldAlert :size="17" />
        <div>
          <strong
            >发现 {{ registrationRows.length }} 个注册 / 创建账户入口</strong
          ><small
            >这是前端无明显漏洞时应优先人工验证的高价值入口；Oviraptor
            不会自动提交注册或创建账户。</small
          >
        </div>
      </div>
      <div class="registration-entry-list">
        <article
          v-for="item in registrationRows"
          :key="item.id"
        >
          <header>
            <span>{{ registrationData(item).title || "注册入口" }}</span>
            <em>{{ registrationData(item).confidence || "unknown" }}</em>
            <b>{{ registrationData(item).sourceType || "candidate" }}</b>
          </header>
          <div class="long-value-cell">
            <code
              class="scroll-value"
              :title="registrationData(item).url"
              >{{ registrationData(item).url }}</code
            >
            <button
              class="icon-button compact"
              title="复制入口"
              @click="copyText(registrationData(item).url)"
            >
              <ClipboardCopy :size="13" />
            </button>
          </div>
          <p>
            {{ registrationData(item).note }}
            <span v-if="registrationData(item).matchedTerms?.length">
              · 命中：{{ registrationData(item).matchedTerms.join("、") }}
            </span>
          </p>
        </article>
      </div>
    </section>
    <SentinelRequestHeaders
      :observed-request-header-rows="observedRequestHeaderRows"
      :declared-request-header-rows="declaredRequestHeaderRows"
      :possible-request-header-rows="possibleRequestHeaderRows"
      :request-header-intelligence="requestHeaderIntelligence"
      :header-display-value="headerDisplayValue"
    />
    <section v-if="realtimeEndpointRows.length" class="result-block realtime-endpoint-block">
      <div class="block-title">
        <Activity :size="17" />
        <div><strong>实时通信接口</strong><small>WebSocket / EventSource 握手及其生效请求头。</small></div>
      </div>
      <div class="realtime-endpoint-list">
        <article v-for="item in realtimeEndpointRows" :key="item.id">
          <b>{{ json(item.recordJson).transport || 'Realtime' }}</b>
          <code>{{ json(item.recordJson).url }}</code>
          <span>HTTP {{ json(item.recordJson).statusCode || '—' }} · 动作 {{ json(item.recordJson).actionId || 'initial' }}</span>
          <small>请求头：{{ text(Object.keys(json(item.recordJson).requestHeaders || {})) || '未捕获' }}</small>
        </article>
      </div>
    </section>
    <section class="result-block kind-api api-explorer-block">
      <div class="block-title api-explorer-heading">
        <Code2 :size="16" />
        <div>
          <strong>API Explorer</strong>
          <small>共 {{ apiRows.length }} 个接口 · 路径规范化展示；完整 URL、参数、响应和来源按条目展开。</small>
        </div>
        <div class="api-explorer-stats"><b>{{ apiRows.length }}</b><span>接口</span><b>{{ new Set(apiRows.map((item) => String(json(item.recordJson).method || "GET").toUpperCase())).size }}</b><span>方法</span></div>
      </div>
      <div v-if="apiRows.length" class="api-explorer">
        <div class="api-explorer-list">
          <article v-for="item in apiRows" :key="item.id" class="api-explorer-row" :class="{ expanded: expandedApiRows.includes(item.id) }">
            <button class="api-row-main" type="button" @click="toggleApiRow(item.id)">
              <b class="method-badge" :class="methodTone(json(item.recordJson).method)">{{ json(item.recordJson).method || "UNKNOWN" }}</b>
              <span class="api-path" :title="apiUrl(item)">{{ apiPath(item) }}</span>
              <span class="api-row-meta">{{ apiQuery(item).length }} 参数 · {{ json(item.recordJson).statusCode || json(item.recordJson).status || "—" }}</span>
              <ChevronDown :size="15" class="api-row-chevron" />
            </button>
            <div class="api-row-actions">
              <button class="icon-button compact" title="复制完整 URL" @click.stop="copyText(apiUrl(item))"><ClipboardCopy :size="13" /></button>
            </div>
            <div v-if="expandedApiRows.includes(item.id)" class="api-row-detail">
              <div class="api-detail-grid">
                <div><span>完整 URL</span><code class="api-long-value">{{ apiUrl(item) }}</code></div>
                <div><span>方法 / 来源 / 置信度</span><p>{{ apiMethod(item) }} · {{ apiSourceSummary(item) }} · {{ apiRecord(item).confidence || "unknown" }}</p></div>
                <div><span>接口说明</span><p>{{ apiDescription(item) }}</p></div>
                <div><span>身份上下文</span><p>{{ apiIdentitySummary(item) }}</p></div>
                <div><span>请求参数</span><p>{{ apiQuery(item).join("、") || "无" }}</p></div>
                <div><span>响应字段</span><p>{{ apiResponseSummary(item) }}</p></div>
                <div><span>响应状态</span><p>HTTP {{ apiRecord(item).statusCode || apiRecord(item).status || "未记录" }} · {{ apiRecord(item).captureStatus || "采集状态未记录" }}</p></div>
              </div>
              <details><summary>请求头 / 请求体 / 发起位置</summary><pre>{{ JSON.stringify({ headers: apiRecord(item).requestHeaders || {}, body: apiRequestPayload(item), initiator: apiRecord(item).initiator || null }, null, 2) }}</pre></details>
              <details><summary>响应头 / 响应体</summary><pre>{{ JSON.stringify({ headers: apiResponseHeaders(item), body: apiRecord(item).decodedBody || apiRecord(item).responseBody || apiRecord(item).responsePreview || null }, null, 2) }}</pre></details>
              <details><summary>原始证据</summary><pre>{{ JSON.stringify(apiRecord(item), null, 2) }}</pre></details>
            </div>
          </article>
        </div>
      </div>
      <div v-else class="empty-inline">没有发现可信 API；运行期 Hook 建议会显示在请求头和实时通信区域。</div>
    </section>
    <SentinelJsFiles :js-rows="jsRows" :copy-text="copyText" />
    <section class="result-block">
      <div class="block-title">
        <Activity :size="16" />
        <div>
          <strong>运行期信号</strong
          ><small
            >只记录静态证据提示；是否启动浏览器 Hook 由调查过程
            针对单个候选决定。</small
          >
        </div>
      </div>
      <div class="runtime-signal-grid">
        <article v-for="item in runtimeRows" :key="item.id">
          <dl>
            <div>
              <dt>Single runtime Hook recommendation</dt>
              <dd>
                <strong>{{
                  json(item.recordJson).label ||
                  kindLabel(json(item.recordJson).type)
                }}</strong>
              </dd>
            </div>
            <div>
              <dt>URL</dt>
              <dd>
                <button
                  v-if="isHttpUrl(runtimeSignalUrl(item))"
                  class="runtime-url-link scroll-value"
                  :title="runtimeSignalUrl(item)"
                  @click="openTargetUrl(runtimeSignalUrl(item))"
                >
                  {{ runtimeSignalUrl(item) }}
                  <ExternalLink :size="12" />
                </button>
                <code
                  v-else
                  class="scroll-value"
                  :title="runtimeSignalUrl(item)"
                  >{{ runtimeSignalUrl(item) || "—" }}</code
                >
              </dd>
            </div>
            <div>
              <dt>展示信息</dt>
              <dd>
                <pre class="scroll-value scroll-value-pre">{{
                  json(item.recordJson).context ||
                  json(item.recordJson).evidence ||
                  json(item.recordJson).reason ||
                  "—"
                }}</pre>
              </dd>
            </div>
          </dl>
        </article>
        <div v-if="!runtimeRows.length" class="empty-inline">
          没有需要运行期采样的信号
        </div>
      </div>
    </section>
    <section class="result-block kind-route">
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>前端路由</strong
          ><small
            >只保留具有路由结构证据的有效路径；旧任务中的 SVG
            属性和单字符结果会自动隐藏。</small
          >
        </div>
      </div>
      <div class="route-chip-list">
        <span
          v-for="item in routeRows"
          :key="item.id"
          class="route-record"
          ><code
            class="scroll-value"
            :title="json(item.recordJson).path"
            >{{ json(item.recordJson).path }}</code
          ><em>{{ json(item.recordJson).type || "route" }}</em
          ><small
            class="scroll-value"
            :title="json(item.recordJson).source"
            >{{ json(item.recordJson).source || "—" }}</small
          ></span
        >
        <div v-if="!routeRows.length" class="empty-inline">
          没有可信路由记录
        </div>
      </div>
    </section>
    <section class="result-block kind-crypto-signal">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>加密方式</strong
          ><small
            >由本地静态分析分类，仅用于展示，不会发送给
            原生 Agent。</small
          >
        </div>
      </div>
      <div class="crypto-table">
        <div class="table-head">
          <span>类别</span><span>算法</span><span>操作</span
          ><span>来源</span><span>证据</span>
        </div>
        <div v-for="item in cryptoRows" :key="item.id">
          <span>{{
            cryptoCategory(json(item.recordJson).category)
          }}</span
          ><strong>{{ json(item.recordJson).algorithm }}</strong
          ><span>{{ json(item.recordJson).operation }}</span
          ><code
            class="scroll-value"
            :title="json(item.recordJson).source"
            >{{ json(item.recordJson).source || "—" }}</code
          ><code
            class="scroll-value"
            :title="
              json(item.recordJson).context ||
              json(item.recordJson).evidence
            "
            >{{
              json(item.recordJson).evidence ||
              json(item.recordJson).context ||
              "—"
            }}</code
          >
        </div>
        <div v-if="!cryptoRows.length" class="empty-inline">
          没有识别到本地加密算法调用
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>敏感信息线索</strong
          ><small
            >仅匹配凭据或个人信息；普通 href、图片、CSS 和普通 URL
            不再进入此处。点击查看原始值与 200 字符上下文。</small
          >
        </div>
      </div>
      <div class="sensitive-table">
        <div class="table-head">
          <span>等级</span><span>类型</span><span>完整值</span
          ><span>来源文件</span><span>SHA-256</span>
        </div>
        <button
          v-for="item in sensitiveRows"
          :key="item.id"
          type="button"
          :class="{ expanded: expandedSensitive.includes(item.id) }"
          @click="toggleSensitive(item.id)"
        >
          <b
            :class="`severity-badge ${safeSeverity(item.severity)}`"
            >{{ severityLabel(item.severity) }}</b
          ><strong>{{
            sensitiveType(json(item.recordJson).type)
          }}</strong
          ><code>{{
            json(item.recordJson).value ||
            json(item.recordJson).maskedValue
          }}</code
          ><code>{{ json(item.recordJson).source }}</code
          ><code>{{ json(item.recordJson).sha256 }}</code>
          <div
            v-if="expandedSensitive.includes(item.id)"
            class="sensitive-context"
          >
            <span>原始上下文（最多 200 字符）</span>
            <pre>{{
              json(item.recordJson).context ||
              "旧结果没有上下文，请重新扫描该 URL。"
            }}</pre>
            <small v-if="json(item.recordJson).scope"
              >IP 类型：{{
                json(item.recordJson).scope === "private"
                  ? "内网"
                  : "公网"
              }}</small
            >
          </div>
        </button>
        <div v-if="!sensitiveRows.length" class="empty-inline">
          没有发现敏感信息线索
        </div>
      </div>
    </section>
  </div>
</template>
