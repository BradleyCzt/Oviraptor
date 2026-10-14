<script setup lang="ts">
import {
  Archive,
  ClipboardCheck,
  ClipboardCopy,
  Eye,
  RefreshCw,
  Save,
  ShieldAlert,
} from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelFuseEntry } from "../types";
import InlineConfirm from "../../../components/InlineConfirm.vue";
import {
  createSentinelLabels,
  fuseVerdictLabel,
  json,
  kindLabel,
  kindTone,
} from "../presentation";

const { severityLabel, statusLabel, verdictLabel } =
  createSentinelLabels(useI18n().tr);

const fuseFilter = defineModel<string>("fuseFilter", { required: true });
const fuseCategoryFilter = defineModel<string>("fuseCategoryFilter", {
  required: true,
});
const fuseEditor = defineModel<SentinelFuseEntry>("fuseEditor");
const pendingFuseRemoval = defineModel<SentinelFuseEntry>("pendingFuseRemoval");

defineProps<{
  fuseEntries: SentinelFuseEntry[];
  visibleFuseEntries: SentinelFuseEntry[];
  fuseBusy: boolean;
  fuseForm: Record<string, any>;
  fuseDetailTabs: Array<[string, string]>;
  copyText: (...args: any[]) => any;
  editFuse: (...args: any[]) => any;
  fuseCategoryLabel: (...args: any[]) => any;
  fuseReasonCategory: (...args: any[]) => any;
  fuseReasonParts: (...args: any[]) => any;
  fuseRecommendedAction: (...args: any[]) => any;
  fuseRows: (...args: any[]) => any;
  fuseState: (...args: any[]) => any;
  fuseTarget: (...args: any[]) => any;
  fuseValidationRows: (...args: any[]) => any;
  removeFuse: (...args: any[]) => any;
  sameTargetUrl: (...args: any[]) => any;
  saveFuse: (...args: any[]) => any;
  toggleFuseDetail: (...args: any[]) => any;
}>();
</script>

<template>
  <section class="panel fuse-zone-panel">
    <div class="panel-heading">
      <div>
        <span class="eyebrow">STOP &amp; DISPOSITION</span>
        <h3>停止与 URL 处置队列</h3>
        <p>
          这里处理“为什么停止、是否恢复”，不是判断漏洞真假。原生 Agent 遇到拦截、成本失控或无进展时只停止当前 URL，其余队列继续执行。
        </p>
      </div>
      <div class="fuse-filters">
        <select v-model="fuseCategoryFilter" class="toolbar-select"><option value="all">全部停止原因</option><option value="budget">成本 / 无进展</option><option value="access">缺少访问条件</option><option value="blocked">遭到拦截</option><option value="failure">网络 / 执行异常</option><option value="low_value">价值不足</option></select>
        <select v-model="fuseFilter" class="toolbar-select"><option value="active">待处置</option><option value="archived">已归档</option><option value="all">全部</option></select>
      </div>
    </div>
    <div class="fuse-zone-stats">
      <article class="attention">
        <span>待决定是否恢复</span
        ><strong>{{
          fuseEntries.filter((item) => !item.archived && item.verdict === 'pending').length
        }}</strong>
      </article>
      <article>
        <span>补充条件后重试</span
        ><strong>{{ fuseEntries.filter((item) => !item.archived && item.verdict === 'needs_followup').length }}</strong>
      </article>
      <article>
        <span>已完成归档</span
        ><strong>{{
          fuseEntries.filter((item) => item.archived).length
        }}</strong>
      </article>
      <article>
        <span>当前显示</span
        ><strong>{{ visibleFuseEntries.length }}</strong>
      </article>
    </div>
    <div class="fuse-entry-list">
      <article
        v-for="item in visibleFuseEntries"
        :key="item.id"
        :class="{ archived: item.archived, attention: !item.archived && item.verdict === 'pending' }"
      >
        <header>
          <span class="fuse-icon"><ShieldAlert :size="16" /></span>
          <div>
            <strong>{{ item.company || "未提供公司" }}</strong>
            <div class="long-value-cell">
              <code :title="item.url">{{ item.url }}</code
              ><button
                class="icon-button compact"
                title="复制 URL"
                @click="copyText(item.url)"
              >
                <ClipboardCopy :size="13" />
              </button>
            </div>
          </div>
          <span :class="`fuse-verdict ${item.verdict}`">{{
            fuseVerdictLabel(item.verdict)
          }}</span>
        </header>
        <div class="fuse-disposition-callout">
          <span :class="`fuse-category category-${fuseReasonCategory(item)}`">{{ fuseCategoryLabel(fuseReasonCategory(item)) }}</span>
          <p><b>建议动作</b>{{ fuseRecommendedAction(item) }}</p>
        </div>
        <dl>
          <div class="fuse-reason-column">
            <dt>熔断原因</dt>
            <dd>
              <div class="fuse-reason-parts">
                <span
                  v-for="part in fuseReasonParts(item)"
                  :key="`${part.label}-${part.text}`"
                  :class="`reason-${part.tone}`"
                  ><b>{{ part.label }}</b
                  >{{ part.text }}</span
                >
              </div>
            </dd>
          </div>
          <div>
            <dt>来源任务</dt>
            <dd>
              <code>{{ item.sourceScanId }}</code>
            </dd>
          </div>
          <div>
            <dt>更新时间</dt>
            <dd>{{ item.updatedAt }}</dd>
          </div>
        </dl>
        <div v-if="item.note || item.evidence" class="fuse-review-summary">
          <p>{{ item.note || "无备注" }}</p>
          <pre>{{ item.evidence || "无证据记录" }}</pre>
        </div>
        <footer>
          <button
            class="button secondary compact"
            @click="toggleFuseDetail(item)"
          >
            <Eye :size="13" />{{
              fuseState(item).open ? "收起完整情报" : "查看完整情报"
            }}</button
          ><button class="button primary compact" @click="editFuse(item)">
            <ClipboardCheck :size="13" />{{
              item.verdict === "pending" ? "记录处置决定" : "编辑处置记录"
            }}</button
          ><button
            v-if="!item.archived && item.verdict !== 'pending'"
            class="button ghost compact"
            @click="
              editFuse(item);
              saveFuse(true);
            "
          >
            <Archive :size="13" />完成并归档</button
          ><button
            class="button warning compact"
            @click="pendingFuseRemoval = item"
          >
            <RefreshCw :size="13" />恢复并自动重试
          </button>
        </footer>
        <section v-if="fuseState(item).open" class="fuse-intel-panel">
          <nav class="fuse-intel-tabs" aria-label="熔断目标情报分类">
            <button
              v-for="entry in fuseDetailTabs"
              :key="entry[0]"
              :class="{ active: fuseState(item).tab === entry[0] }"
              @click="fuseState(item).tab = entry[0]"
            >
              {{ entry[1] }}
            </button>
          </nav>
          <div v-if="fuseState(item).loading" class="empty-inline">
            正在加载该 URL 的完整情报…
          </div>
          <template v-else>
            <div
              v-if="fuseState(item).tab === 'summary'"
              class="fuse-intel-summary"
            >
              <dl>
                <div>
                  <dt>URL</dt>
                  <dd class="scroll-value">{{ item.url }}</dd>
                </div>
                <div>
                  <dt>执行状态</dt>
                  <dd>
                    {{ statusLabel(fuseTarget(item)?.status || "limited") }}
                  </dd>
                </div>
                <div>
                  <dt>前端价值</dt>
                  <dd>{{ fuseTarget(item)?.valueScore ?? "—" }} / 100</dd>
                </div>
                <div>
                  <dt>扫描模式</dt>
                  <dd>
                    {{ (fuseTarget(item)?.scanMode || "—").toUpperCase() }}
                  </dd>
                </div>
                <div class="fuse-summary-reason">
                  <dt>熔断原因</dt>
                  <dd>
                    <div class="fuse-reason-parts compact">
                      <span
                        v-for="part in fuseReasonParts(item)"
                        :key="`summary-${part.label}-${part.text}`"
                        :class="`reason-${part.tone}`"
                        ><b>{{ part.label }}</b
                        >{{ part.text }}</span
                      >
                    </div>
                  </dd>
                </div>
                <div>
                  <dt>记录总数</dt>
                  <dd>
                    {{
                      fuseState(item).findings.filter((row: any) =>
                        sameTargetUrl(row.targetUrl, item.url),
                      ).length
                    }}
                  </dd>
                </div>
              </dl>
              <div
                v-for="row in fuseRows(
                  item,
                  'summary_target',
                  'risk_summary',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <strong>{{ row.title || kindLabel(row.kind) }}</strong>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'fingerprint'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'fingerprint',
                  'tech_stack',
                  'security_header',
                  'cookie',
                  'external_service',
                  'info_disclosure',
                  'open_port',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong>{{ row.title || row.recordKey }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'fingerprint',
                    'tech_stack',
                    'security_header',
                    'cookie',
                    'external_service',
                    'info_disclosure',
                    'open_port',
                  ).length
                "
                class="empty-inline"
              >
                没有指纹配置记录
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'assets'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'js_file',
                  'api',
                  'route',
                  'runtime_signal',
                  'crypto_signal',
                  'sensitive_info',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong class="scroll-value">{{
                    row.title || row.recordKey
                  }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'js_file',
                    'api',
                    'route',
                    'runtime_signal',
                    'crypto_signal',
                    'sensitive_info',
                  ).length
                "
                class="empty-inline"
              >
                没有 JS / API 情报
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'endpoints'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'endpoint',
                  'endpoint_expanded',
                  'directory_find',
                  'rest_endpoint',
                  'login_endpoint',
                  'parameter_json',
                  'parameter_xml',
                  'parameter_form',
                  'parameter_upload',
                  'parameter_path',
                  'parameter_query',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong class="scroll-value">{{
                    row.title || row.recordKey
                  }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'endpoint',
                    'endpoint_expanded',
                    'directory_find',
                    'rest_endpoint',
                    'login_endpoint',
                    'parameter_json',
                    'parameter_xml',
                    'parameter_form',
                    'parameter_upload',
                    'parameter_path',
                    'parameter_query',
                  ).length
                "
                class="empty-inline"
              >
                没有端点验证记录
              </div>
            </div>
            <div v-else class="fuse-intel-records">
              <div
                v-for="row in fuseRows(
                  item,
                  'vulnerability',
                  'poc_test',
                  'risk_summary',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span
                    >{{ kindLabel(row.kind) }} ·
                    {{ severityLabel(row.severity) }}</span
                  ><strong>{{ row.title || row.recordKey }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-for="row in fuseValidationRows(item)"
                :key="`validation-${row.id}`"
                class="fuse-intel-record validation"
              >
                <header>
                  <span>人工验证 · {{ verdictLabel(row.verdict) }}</span
                  ><strong>{{ row.findingKind }}</strong>
                </header>
                <p>{{ row.note || "无备注" }}</p>
                <pre>{{ row.evidence || "无证据记录" }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'vulnerability',
                    'poc_test',
                    'risk_summary',
                  ).length && !fuseValidationRows(item).length
                "
                class="empty-inline"
              >
                没有漏洞或证明记录
              </div>
            </div>
          </template>
        </section>
        <div v-if="fuseEditor?.id === item.id" class="fuse-review-editor">
          <label class="field"
            ><span>URL 处置状态</span
            ><select v-model="fuseForm.verdict">
              <option value="pending">暂不决定</option>
              <option value="manual_verified">已人工接管</option>
              <option value="needs_followup">补充条件后重试</option>
              <option value="not_reproducible">保持排除</option>
            </select></label
          >
          <label class="field"
            ><span>处置说明</span
            ><textarea
              v-model="fuseForm.note"
              rows="3"
              placeholder="为什么停止、缺少什么条件、是否值得恢复"
            ></textarea>
          </label>
          <label class="field span-two"
            ><span>补充上下文 / 请求响应 / 证据路径</span
            ><textarea
              v-model="fuseForm.evidence"
              rows="5"
              placeholder="记录关键请求响应或本地证据文件"
            ></textarea>
          </label>
          <footer>
            <button class="button ghost" @click="fuseEditor = undefined">
              取消</button
            ><button
              class="button primary"
              :disabled="fuseBusy"
              @click="saveFuse(false)"
            >
              <Save :size="14" />保存处置</button
            ><button
              class="button secondary"
              :disabled="fuseBusy"
              @click="saveFuse(true)"
            >
              <Archive :size="14" />完成并归档
            </button>
          </footer>
        </div>
        <InlineConfirm
          v-if="pendingFuseRemoval?.id === item.id"
          title="恢复该 URL 并立即重试？"
          detail="会在原任务工作流中创建只包含该 URL 的续跑执行，复用已保存的前端证据；历史记录与累计成本都会保留。"
          :busy="fuseBusy"
          @cancel="pendingFuseRemoval = undefined"
          @confirm="removeFuse"
        />
      </article>
      <div v-if="!visibleFuseEntries.length" class="empty-state">
        当前没有匹配的熔断目标。
      </div>
    </div>
  </section>
</template>
