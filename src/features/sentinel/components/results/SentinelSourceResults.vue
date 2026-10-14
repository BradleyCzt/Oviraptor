<script setup lang="ts">
import { computed, ref, watch } from "vue";
import SourceReviewEvidence from "../SourceReviewEvidence.vue";
import {
  Activity,
  Bug,
  ClipboardCheck,
  Code2,
  Download,
  FileJson,
  Layers3,
  Network,
  Pause,
  Play,
  RefreshCw,
} from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { AppSecScanResult, SentinelFinding, SentinelScan, SentinelScanAttempt, SentinelValidation } from "../../types";
import {
  attemptEndReason,
  attemptStageLabel,
  attemptTime,
  createSentinelLabels,
  formatNumber,
  json,
  scanSummary,
  scanTokenTotal,
  text,
  uncachedInput,
} from "../../presentation";

const {
  retryActionLabel,
  scanTypeLabel,
  severityLabel,
  statusLabel,
  verdictLabel,
} = createSentinelLabels(useI18n().tr);
const emit = defineEmits<{
  "toggle-attempt-history": [];
  "select-finding": [id: number];
}>();
const props = defineProps<{
  selected: SentinelScan;
  detailBusy: boolean;
  scanControlBusy: string;
  scanAttempts: SentinelScanAttempt[];
  visibleScanAttempts: SentinelScanAttempt[];
  attemptModeLabel: (mode: string, attemptNumber: number) => string;
  showAttemptHistory: boolean;
  resultTab: string;
  selectedFindingId?: number;
  isGreyboxScan: boolean;
  isCicdScan: boolean;
  appsecResult?: AppSecScanResult;
  appsecVulnerabilities: any[];
  appsecSourceCounts: Record<string, any>;
  cicdBlockingFindings: any[];
  greyboxCorrelated: Record<string, any>;
  focusedSourceFindingRows: any[];
  sourceFindingRows: any[];
  sourceDependencies: any[];
  sourceFrameworks: any[];
  sourceInventory: Record<string, any>;
  sourceInventoryFinding?: SentinelFinding;
  sourceIssueGroups: any[];
  sourceLanguageRows: any[];
  sourceManifests: any[];
  sourceSeverityCounts: Record<string, any>;
  sourceStats: Record<string, any>;
  appsecSourcesFor: (item: any) => any[];
  authTypeLabel: (value: string) => string;
  ciProviderLabel: (value: string) => string;
  correlationParts: (item: any) => any;
  editValidation: (item: SentinelFinding, verdict?: string) => void;
  effectiveSeverity: (item: any) => string;
  exportProject: (...args: any[]) => any;
  gateStatusLabel: (value: string) => string;
  pauseScan: (...args: any[]) => any;
  rescan: (...args: any[]) => any;
  resumeScan: (...args: any[]) => any;
  sourceLocations: (item: any) => any[];
  sourceTypeLabel: (value: string) => string;
  validationFor: (item: any) => SentinelValidation | undefined;
}>();
const preferredReviewAttempt = ref<number>();
const reviewAttempts = computed(() => [...new Set(props.scanAttempts
  .filter(row => row.scanId === props.selected.id && Number.isSafeInteger(row.attemptNumber) && row.attemptNumber > 0)
  .map(row => row.attemptNumber))].sort((a, b) => b - a));
const reviewAttempt = computed({
  get: () => preferredReviewAttempt.value !== undefined && reviewAttempts.value.includes(preferredReviewAttempt.value)
    ? preferredReviewAttempt.value : reviewAttempts.value[0],
  set: (value: number | undefined) => { preferredReviewAttempt.value = value; },
});
watch(() => props.selected.id, () => { preferredReviewAttempt.value = undefined; }, { flush: "sync" });

// Read the saved payload, not the parent's legacy zero-coerced projection.
// Unknown historical measurements must not look like measured empty files.
function savedCount(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}
function savedCountLabel(value: unknown): string {
  const count = savedCount(value);
  return count === undefined ? "未记录" : formatNumber(count);
}
const savedLineStats = computed(() => {
  const raw = props.sourceInventory.lineStats;
  return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
});
const incompleteLineStats = computed(() => ["physical", "code", "comments", "blank"]
  .some(key => savedCount(savedLineStats.value[key]) === undefined));
const skippedLargeFiles = computed(() => savedCount(savedLineStats.value.skippedLargeFiles));
</script>

<template>
  <header class="url-intelligence-head source-intelligence-head">
    <div>
      <span class="eyebrow"
        >{{ scanTypeLabel(selected.scanType) }} · SOURCE
        INTELLIGENCE</span
      >
      <h3>{{ selected.taskName || selected.projectName }}</h3>
      <p>
        {{ selected.sourcePath || "未提供源码路径" }} ·
        {{ statusLabel(selected.status) }} · {{ selected.id }}
      </p>
      <small
        v-if="scanSummary(selected)"
        class="live-checkpoint"
        ><Activity :size="12" />
        {{ scanSummary(selected) }}</small
      >
    </div>
    <div class="hero-actions">
      <button
        v-if="
          selected.status === 'scanning' ||
          selected.status === 'pausing'
        "
        class="button warning compact"
        :disabled="scanControlBusy === selected.id"
        @click="pauseScan(selected)"
      >
        <Pause :size="14" />{{
          selected.status === "pausing"
            ? "正在停止"
            : "停止并保留"
        }}</button
      ><button
        v-else-if="selected.status === 'paused'"
        class="button secondary compact"
        :disabled="scanControlBusy === selected.id"
        @click="resumeScan(selected)"
      >
        <Play :size="14" />继续扫描</button
      ><button
        v-else-if="
          selected.status !== 'draft' &&
          selected.status !== 'pausing'
        "
        class="button ghost compact"
        :disabled="scanControlBusy === selected.id"
        @click="rescan(selected)"
      >
        <RefreshCw :size="14" />{{ retryActionLabel(selected) }}</button
      ><button class="button ghost compact" @click="exportProject">
        <Download :size="14" />导出项目包
      </button>
    </div>
  </header>
  <div v-if="detailBusy" class="empty-state">正在解析源码结果…</div>
  <div v-else class="source-result-stack">
    <section class="result-block" aria-label="正式源码审查">
      <label v-if="reviewAttempts.length">审查执行轮次
        <select v-model="reviewAttempt" aria-label="源码结果审查轮次">
          <option v-for="attempt in reviewAttempts" :key="attempt" :value="attempt">第 {{ attempt }} 次执行</option>
        </select>
      </label>
      <SourceReviewEvidence v-if="reviewAttempt !== undefined" :key="selected.id" :scan-id="selected.id" :attempt-number="reviewAttempt" />
      <p v-else>尚无可选择的执行轮次；下方历史资料不代表独立审查已确认。</p>
    </section>
    <details class="source-historical-results">
      <summary>历史 / 分析器参考资料（不代表所选轮次独立确认）</summary>
      <p>以下保留原有仓库概况、分析器记录及人工标记，可能跨执行轮次。它们与上方正式裁决分别展示；历史状态、人工复核或旧门禁字段不等于本轮独立 Reviewer 确认。项目包仍是历史资料导出；正式裁决请使用上方“导出本轮审查 JSON”。</p>
    <section class="source-scan-meta">
      <div>
        <span>扫描类型</span
        ><strong>{{ scanTypeLabel(selected.scanType) }}</strong>
      </div>
      <div>
        <span>扫描状态</span
        ><strong>{{ statusLabel(selected.status) }}</strong>
      </div>
      <div>
        <span>源码路径</span
        ><code>{{ selected.sourcePath || "—" }}</code>
      </div>
      <div>
        <span>扫描器 / 技能</span
        ><code>{{ selected.skillNames || "默认规则集" }}</code>
      </div>
      <div v-if="isGreyboxScan">
        <span>联测范围</span><strong>源码 + 运行期证据</strong>
      </div>
      <div v-if="isCicdScan">
        <span>质量门禁</span><strong>变更范围 / 阻断发现</strong>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-architecture-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div>
          <strong>项目架构与技术栈</strong>
          <small
            >只根据源码扩展名和仓库清单识别，不使用漏洞记录里的
            ecosystem 字段推测语言。</small
          >
        </div>
      </div>
      <template v-if="sourceInventoryFinding">
        <div class="source-architecture-grid">
          <article class="architecture-primary">
            <span>应用架构</span>
            <strong>{{
              sourceInventory.architecture || "Source repository"
            }}</strong>
            <small
              >{{ sourceStats.totalFiles }} 个仓库文件 ·
              {{ sourceStats.codeFiles }} 个可识别代码文件</small
            >
          </article>
          <article
            v-for="framework in sourceFrameworks"
            :key="`${framework.layer}-${framework.name}`"
          >
            <span>{{ framework.layer }}</span>
            <strong>{{ framework.name }}</strong>
            <small>清单特征：{{ framework.evidence }}</small>
          </article>
        </div>
        <div class="source-loc-grid">
          <article>
            <span>物理行</span>
            <strong>{{
              savedCountLabel(savedLineStats.physical)
            }}</strong>
          </article>
          <article class="loc-code">
            <span>有效代码</span>
            <strong>{{
              savedCountLabel(savedLineStats.code)
            }}</strong>
          </article>
          <article>
            <span>注释行</span>
            <strong>{{
              savedCountLabel(savedLineStats.comments)
            }}</strong>
          </article>
          <article>
            <span>空行</span>
            <strong>{{
              savedCountLabel(savedLineStats.blank)
            }}</strong>
          </article>
        </div>
        <p class="source-loc-rule">
          显示保存的行数统计；查看历史记录不会重新读取当前源码目录。
          <span v-if="incompleteLineStats">
            部分统计未记录或格式无效，不按零计算。如需更新，请创建新的授权扫描任务。
          </span>
          <span v-if="skippedLargeFiles !== undefined">
            记录中跳过 {{ skippedLargeFiles }} 个超大源码文件。
          </span>
        </p>
        <div
          v-if="sourceManifests.length"
          class="source-manifest-list"
        >
          <span>识别证据</span>
          <code
            v-for="manifest in sourceManifests"
            :key="manifest"
            >{{ manifest }}</code
          >
        </div>
        <div
          v-if="sourceLanguageRows.length"
          class="source-language-table"
        >
          <div class="table-head">
            <span>语言</span><span>文件数</span><span>有效代码</span
            ><span>注释</span><span>空行</span><span>物理行</span
            ><span>代码体积</span><span>文件占比</span>
          </div>
          <div
            v-for="language in sourceLanguageRows"
            :key="language.name"
          >
            <strong>{{ language.name }}</strong>
            <span>{{ formatNumber(language.files) }}</span>
            <span>{{ savedCountLabel(language.codeLines) }}</span>
            <span>{{ savedCountLabel(language.commentLines) }}</span>
            <span>{{ savedCountLabel(language.blankLines) }}</span>
            <span>{{ savedCountLabel(language.lines) }}</span>
            <span>{{ formatNumber(language.bytes) }} B</span>
            <span
              >{{ Number(language.percent || 0).toFixed(1) }}%</span
            >
          </div>
        </div>
      </template>
      <div v-else class="source-inventory-warning">
        没有可用的源码清单，通常表示原源码路径已不可读取。恢复该目录后重新打开任务即可本地补建，不会调用模型；也可以再次扫描生成新结果。
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-token-block">
      <div class="block-title">
        <Activity :size="16" />
        <div>
          <strong>模型与 Token 消耗</strong
          ><small
            >缓存输入属于输入总计；新增输入和模型输出分别展示。</small
          >
        </div>
      </div>
      <div class="source-token-grid">
        <article>
          <span>模型请求</span
          ><strong>{{ formatNumber(selected.llmRequests) }}</strong>
        </article>
        <article>
          <span>输入 Token</span
          ><strong>{{ formatNumber(selected.inputTokens) }}</strong>
        </article>
        <article>
          <span>输出 Token</span
          ><strong>{{
            formatNumber(selected.outputTokens)
          }}</strong>
        </article>
        <article>
          <span>缓存 Token</span
          ><strong>{{
            formatNumber(selected.cachedTokens)
          }}</strong>
        </article>
        <article>
          <span>新增输入 Token</span
          ><strong>{{
            formatNumber(uncachedInput(selected))
          }}</strong>
        </article>
        <article>
          <span>输入 + 输出总计</span
          ><strong>{{ formatNumber(scanTokenTotal(selected)) }}</strong>
        </article>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities' && scanAttempts.length" class="result-block attempt-ledger-block">
      <div class="block-title">
        <RefreshCw :size="16" />
        <div><strong>本次执行结果与增量成本</strong><small>默认只显示最新一次最终状态；旧轮次不会再混入当前结果，需要审计时再展开。</small></div>
        <button v-if="scanAttempts.length > 1" class="attempt-history-toggle" @click="emit('toggle-attempt-history')">{{showAttemptHistory ? '收起历史' : `查看历史 ${scanAttempts.length - 1} 次`}}</button>
      </div>
      <div class="attempt-ledger">
        <article v-for="attempt in visibleScanAttempts" :key="attempt.attemptNumber" :class="[`attempt-${attempt.status}`, { current: attempt.attemptNumber === selected.attemptCount }]">
          <header><span>第 {{attempt.attemptNumber}} 次 · {{attemptModeLabel(attempt.executionMode, attempt.attemptNumber)}}</span><b>{{attemptStageLabel(attempt.stage)}}</b><em class="status-chip" :class="attempt.status">{{statusLabel(attempt.status)}}</em></header>
          <p>{{attempt.checkpoint || '尚无阶段详情'}}</p>
          <div class="attempt-cost"><span>请求 <b>{{formatNumber(attempt.llmRequests)}}</b></span><span>输入 <b>{{formatNumber(attempt.inputTokens)}}</b></span><span>缓存 <b>{{formatNumber(attempt.cachedTokens)}}</b></span><span>输出 <b>{{formatNumber(attempt.outputTokens)}}</b></span><span>本次总计 <b>{{formatNumber(attempt.totalTokens)}}</b></span></div>
          <small>{{attemptTime(attempt)}}</small><code v-if="attempt.workDir" :title="attempt.workDir">{{attempt.workDir}}</code><mark v-if="attemptEndReason(attempt)">结束说明：{{attemptEndReason(attempt)}}</mark>
        </article>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-summary-block">
      <div class="block-title">
        <Code2 :size="16" />
        <div>
          <strong>代码扫描概况</strong
          ><small
            >参考
            SAST、SCA、质量门禁和数据流分析结果标准化展示。</small
          >
        </div>
      </div>
      <div class="source-metric-grid">
        <article>
          <span>历史 / 分析器条目</span
          ><strong>{{ sourceStats.findings }}</strong>
        </article>
        <article>
          <span>受影响文件</span
          ><strong>{{ sourceStats.files }}</strong>
        </article>
        <article>
          <span>代码位置</span
          ><strong>{{ sourceStats.locations }}</strong>
        </article>
        <article>
          <span>命中规则</span
          ><strong>{{ sourceStats.rules }}</strong>
        </article>
        <article>
          <span>仓库代码文件</span
          ><strong>{{ sourceStats.codeFiles }}</strong>
        </article>
      </div>
      <div class="source-severity-grid">
        <article
          v-for="item in sourceSeverityCounts"
          :key="item.severity"
          :class="`source-severity ${item.severity}`"
        >
          <span>{{ severityLabel(item.severity) }}</span
          ><strong>{{ item.count }}</strong>
        </article>
      </div>
    </section>
    <section class="result-block source-finding-index-block">
      <div class="block-title">
        <FileJson :size="16" />
        <div>
          <strong>发现索引</strong
          ><small
            >先在索引选择一条发现，下方只展示当前发现的完整证据。</small
          >
        </div>
      </div>
      <div class="source-finding-index">
        <div class="table-head">
          <span>等级</span><span>标题</span
          ><span>引擎 / Rule ID</span><span>CWE / CVE</span
          ><span>文件</span><span>行号</span>
        </div>
        <div
          v-for="item in sourceFindingRows"
          :key="`index-${item.id}`"
          :class="{ active: selectedFindingId === item.id }"
          role="button"
          tabindex="0"
          @click="emit('select-finding', item.id)"
          @keydown.enter="emit('select-finding', item.id)"
        >
          <span
            :class="`severity-badge ${effectiveSeverity(item)}`"
            >{{ severityLabel(effectiveSeverity(item)) }}</span
          >
          <strong>{{
            item.title ||
            json(item.recordJson).title ||
            item.recordKey
          }}</strong>
          <code
            >{{
              json(item.recordJson).engine ||
              json(item.recordJson).source ||
              "Agent"
            }}
            ·
            {{
              json(item.recordJson).rule_id ||
              json(item.recordJson).ruleId ||
              item.recordKey
            }}</code
          >
          <span
            >{{ json(item.recordJson).cwe || "—" }} /
            {{ json(item.recordJson).cve || "—" }}</span
          >
          <code>{{ sourceLocations(item)[0]?.file || "—" }}</code>
          <span>{{
            sourceLocations(item)[0]?.start_line ||
            sourceLocations(item)[0]?.startLine ||
            "—"
          }}</span>
        </div>
        <div v-if="!sourceFindingRows.length" class="empty-inline">
          没有可索引的安全发现。
        </div>
      </div>
    </section>
    <section class="result-block source-findings-block">
      <div class="block-title">
        <Bug :size="16" />
        <div>
          <strong>代码发现与审计证据</strong
          ><small
            >仅展示结构化安全问题与漏洞证据；
            扫描总结和质量门禁摘要不作为问题加载。</small
          >
        </div>
      </div>
      <div
        v-if="sourceIssueGroups.length"
        class="source-issue-groups"
      >
        <article
          v-for="group in sourceIssueGroups"
          :key="group.name"
        >
          <span>{{ group.name }}</span>
          <strong>{{ group.count }}</strong>
          <small v-if="group.high">{{ group.high }} 个高风险</small>
          <small v-else>无高风险项</small>
        </article>
      </div>
      <div class="source-finding-list">
        <article
          v-for="item in focusedSourceFindingRows"
          :key="item.id"
          :class="`source-finding-card severity-border-${effectiveSeverity(item)}`"
        >
          <header>
            <span
              :class="`severity-badge ${effectiveSeverity(item)}`"
              >{{ severityLabel(effectiveSeverity(item)) }}</span
            >
            <div>
              <strong>{{
                item.title ||
                json(item.recordJson).title ||
                item.recordKey
              }}</strong
              ><small
                >{{
                  json(item.recordJson).rule_id ||
                  json(item.recordJson).ruleId ||
                  "未标注规则"
                }}
                · {{ json(item.recordJson).cwe || "无 CWE" }} ·
                {{ json(item.recordJson).cve || "无 CVE" }} · 置信度
                {{ json(item.recordJson).confidence || "—" }}</small
              >
            </div>
            <span
              v-if="json(item.recordJson).status"
              class="finding-state"
              >{{ json(item.recordJson).status }}</span
            >
          </header>
          <div class="source-finding-grid">
            <div>
              <span>描述</span>
              <p>
                {{
                  json(item.recordJson).description ||
                  json(item.recordJson).message ||
                  "—"
                }}
              </p>
            </div>
            <div>
              <span>技术分析 / 影响</span>
              <p>
                {{
                  json(item.recordJson).technical_analysis ||
                  json(item.recordJson).impact ||
                  json(item.recordJson).detail ||
                  "—"
                }}
              </p>
            </div>
            <div v-if="sourceLocations(item).length">
              <span>文件与行号</span>
              <div
                v-for="location in sourceLocations(item)"
                :key="`${item.id}-${location.file}-${location.start_line}`"
                class="source-location"
              >
                <code>{{ location.file || "—" }}</code
                ><b
                  >L{{
                    location.start_line ||
                    location.startLine ||
                    "?"
                  }}<span
                    v-if="location.end_line || location.endLine"
                    >-{{
                      location.end_line || location.endLine
                    }}</span
                  ></b
                >
                <pre v-if="location.snippet">{{
                  location.snippet
                }}</pre>
              </div>
            </div>
            <div
              v-if="
                json(item.recordJson).data_flow ||
                json(item.recordJson).taint_flow ||
                json(item.recordJson).call_chain
              "
            >
              <span>数据流 / 调用链</span>
              <pre>{{
                text(
                  json(item.recordJson).data_flow ||
                    json(item.recordJson).taint_flow ||
                    json(item.recordJson).call_chain,
                )
              }}</pre>
            </div>
            <div>
              <span>修复建议</span>
              <p>
                {{
                  json(item.recordJson).recommendation ||
                  json(item.recordJson).remediation_steps ||
                  "—"
                }}
              </p>
            </div>
            <div
              v-if="
                json(item.recordJson).fix_before ||
                json(item.recordJson).fix_after
              "
            >
              <span>修复前 / 修复后</span>
              <pre
                >{{
                  json(item.recordJson).fix_before || "—"
                }}\n\n→\n\n{{
                  json(item.recordJson).fix_after || "—"
                }}</pre>
            </div>
            <div
              v-if="
                json(item.recordJson).dependency_metadata ||
                json(item.recordJson).package
              "
            >
              <span>依赖信息</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).dependency_metadata ||
                    json(item.recordJson).package,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div
              v-if="
                json(item.recordJson).evidence ||
                json(item.recordJson).pocRequest
              "
            >
              <span>证据 / 验证</span>
              <pre>{{
                text(
                  json(item.recordJson).evidence ||
                    json(item.recordJson).pocRequest,
                )
              }}</pre>
            </div>
            <div v-if="json(item.recordJson).assumptions">
              <span>前提与限制</span>
              <p>{{ text(json(item.recordJson).assumptions) }}</p>
            </div>
          </div>
          <footer>
            <button
              class="button primary compact"
              @click="editValidation(item)"
            >
              <ClipboardCheck :size="13" />{{
                validationFor(item)
                  ? "修改验证结论"
                  : "开始人工验证"
              }}</button
            ><span
              v-if="validationFor(item)"
              class="validation-saved-note"
              >{{
                verdictLabel(validationFor(item)?.verdict || "")
              }}</span
            >
          </footer>
        </article>
        <div v-if="!sourceFindingRows.length" class="empty-inline">
          当前源码任务没有结构化发现。
        </div>
      </div>
    </section>
    <section v-if="sourceDependencies.length" class="result-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div>
          <strong>依赖与供应链风险</strong
          ><small
            >展示包生态、已安装版本、修复版本和 CVE/CVSS。</small
          >
        </div>
      </div>
      <div class="source-dependency-list">
        <article
          v-for="item in sourceDependencies"
          :key="`dep-${item.id}`"
        >
          <strong>{{
            json(item.recordJson).package_name ||
            json(item.recordJson).package ||
            json(item.recordJson).dependency_metadata?.name ||
            item.title
          }}</strong
          ><span
            >{{
              json(item.recordJson).dependency_metadata
                ?.ecosystem ||
              json(item.recordJson).package_ecosystem ||
              "—"
            }}
            ·
            {{
              json(item.recordJson).dependency_metadata
                ?.installed_version ||
              json(item.recordJson).installed_version ||
              "—"
            }}</span
          ><b
            >{{ json(item.recordJson).cve || "无 CVE" }} · CVSS
            {{ json(item.recordJson).cvss ?? "—" }}</b
          ><em
            >修复版本
            {{
              json(item.recordJson).dependency_metadata
                ?.fixed_version ||
              json(item.recordJson).fixed_version ||
              "—"
            }}</em
          >
        </article>
      </div>
    </section>
    <section
      v-if="isGreyboxScan"
      class="result-block source-runtime-block"
    >
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>灰盒运行期关联</strong
          ><small
            >统一漏洞关联源码位置、运行期端点、参数和验证来源。</small
          >
        </div>
      </div>
      <div class="greybox-overview">
        <article>
          <span>测试环境</span>
          <strong>{{
            appsecResult?.context?.environment || "未记录"
          }}</strong>
        </article>
        <article>
          <span>认证上下文</span>
          <strong>{{
            authTypeLabel(appsecResult?.context?.authType || "none")
          }}</strong>
          <small>{{
            appsecResult?.context?.authenticated
              ? appsecResult?.context?.authProfileName ||
                "本次临时会话"
              : "未提供认证会话"
          }}</small>
        </article>
        <article>
          <span>统一漏洞</span>
          <strong>{{ appsecVulnerabilities.length }}</strong>
        </article>
        <article class="correlated-count">
          <span>SAST + DAST 已关联</span>
          <strong>{{ greyboxCorrelated.length }}</strong>
        </article>
        <article>
          <span>来源记录</span>
          <strong>{{ appsecResult?.sources.length }}</strong>
          <small
            >SAST {{ appsecSourceCounts.sast || 0 }} · DAST
            {{ appsecSourceCounts.dast || 0 }} · IAST
            {{ appsecSourceCounts.iast || 0 }} · SCA
            {{ appsecSourceCounts.sca || 0 }}</small
          >
        </article>
      </div>
      <div class="appsec-correlation-list">
        <article
          v-for="vulnerability in appsecVulnerabilities"
          :key="`appsec-${vulnerability.id}`"
          :class="`appsec-correlation-card severity-border-${vulnerability.severity}`"
        >
          <header>
            <span
              :class="`severity-badge ${vulnerability.severity}`"
              >{{ severityLabel(vulnerability.severity) }}</span
            >
            <div>
              <strong>{{ vulnerability.title }}</strong>
              <small
                >{{
                  vulnerability.vulnerabilityType || "未分类漏洞"
                }}
                · {{ vulnerability.status || "open" }}</small
              >
            </div>
            <span class="correlation-score"
              >{{ vulnerability.correlationScore }}% 关联度</span
            >
          </header>
          <div class="appsec-link-grid">
            <div>
              <span>运行期端点</span>
              <code
                >{{ vulnerability.httpMethod || "—" }}
                {{ vulnerability.url || "—" }}</code
              >
              <small
                >参数：{{ vulnerability.parameter || "—" }}</small
              >
            </div>
            <div>
              <span>代码位置</span>
              <code
                >{{ vulnerability.file || "—"
                }}<template v-if="vulnerability.startLine"
                  >:{{ vulnerability.startLine }}</template
                ></code
              >
              <small>符号：{{ vulnerability.symbol || "—" }}</small>
            </div>
          </div>
          <div class="appsec-source-list">
            <span
              v-for="source in appsecSourcesFor(vulnerability)"
              :key="source.id"
            >
              <b>{{ sourceTypeLabel(source.sourceType) }}</b>
              {{ source.engine || source.sourceKey }}
            </span>
          </div>
          <div
            v-if="vulnerability.correlation?.embeddedEvidence"
            class="embedded-correlation-note"
          >
            同一条已验证记录同时包含源码位置与运行期请求证据。
          </div>
          <div v-else class="correlation-breakdown">
            <span
              v-for="part in correlationParts(vulnerability)"
              :key="part.key"
              :class="{ matched: part.matched }"
            >
              {{ part.label }}
              <b>{{
                part.matched ? `+${part.weight}` : "未匹配"
              }}</b>
            </span>
          </div>
        </article>
        <div
          v-if="!appsecVulnerabilities.length"
          class="empty-inline"
        >
          当前任务没有可统一的结构化漏洞记录。
        </div>
      </div>
    </section>
    <section
      v-if="isCicdScan"
      class="result-block source-runtime-block"
    >
      <div class="block-title">
        <ClipboardCheck :size="16" />
        <div>
          <strong>CI/CD 质量门禁</strong
          ><small
            >流水线只负责触发与门禁，问题来源仍保留 SAST、SCA
            和验证证据。</small
          >
        </div>
      </div>
      <div class="cicd-context-grid">
        <div>
          <span>Provider</span
          ><strong>{{
            ciProviderLabel(appsecResult?.context?.ciProvider || "")
          }}</strong>
        </div>
        <div>
          <span>仓库</span
          ><code>{{
            appsecResult?.context?.repositoryUrl || "未记录"
          }}</code>
        </div>
        <div>
          <span>分支</span
          ><code>{{
            appsecResult?.context?.branch || "未记录"
          }}</code>
        </div>
        <div>
          <span>Commit</span
          ><code>{{
            appsecResult?.context?.commitSha || "未记录"
          }}</code>
        </div>
        <div>
          <span>Build / Pipeline</span
          ><code>{{
            appsecResult?.context?.buildId || "未记录"
          }}</code>
        </div>
        <div>
          <span>环境</span
          ><strong>{{
            appsecResult?.context?.environment || "未记录"
          }}</strong>
        </div>
      </div>
      <div
        :class="`gate-status ${appsecResult?.context?.gateStatus || 'not_evaluated'}`"
      >
        <div>
          <span>发布门禁</span>
          <strong>{{
            gateStatusLabel(
              appsecResult?.context?.gateStatus || "not_evaluated",
            )
          }}</strong>
          <small>{{
            appsecResult?.context?.gateReason ||
            "历史任务没有门禁上下文"
          }}</small>
        </div>
        <dl>
          <div>
            <dt>Critical</dt>
            <dd>
              {{
                appsecVulnerabilities.filter(
                  (item) => item.severity === "critical",
                ).length
              }}
              / {{ appsecResult?.context?.policy?.maxCritical ?? 0 }}
            </dd>
          </div>
          <div>
            <dt>High</dt>
            <dd>
              {{
                appsecVulnerabilities.filter(
                  (item) => item.severity === "high",
                ).length
              }}
              / {{ appsecResult?.context?.policy?.maxHigh ?? 5 }}
            </dd>
          </div>
          <div>
            <dt>策略</dt>
            <dd>
              {{
                appsecResult?.context?.policy?.blockRelease
                  ? "超限阻断"
                  : "仅告警"
              }}
            </dd>
          </div>
        </dl>
      </div>
      <div class="cicd-blocking-table">
        <div class="table-head">
          <span>等级</span><span>问题</span><span>位置 / 端点</span
          ><span>状态</span><span>生命周期</span><span>负责人</span>
        </div>
        <div
          v-for="vulnerability in cicdBlockingFindings"
          :key="`gate-${vulnerability.id}`"
        >
          <span
            :class="`severity-badge ${vulnerability.severity}`"
            >{{ severityLabel(vulnerability.severity) }}</span
          >
          <strong>{{ vulnerability.title }}</strong>
          <code>{{
            vulnerability.file
              ? `${vulnerability.file}${vulnerability.startLine ? `:${vulnerability.startLine}` : ""}`
              : vulnerability.url || "—"
          }}</code>
          <span>{{ vulnerability.status || "open" }}</span>
          <small
            >{{ vulnerability.firstSeen }}<br />{{
              vulnerability.lastSeen
            }}</small
          >
          <span>{{ vulnerability.owner || "未分配" }}</span>
        </div>
        <div
          v-if="!cicdBlockingFindings.length"
          class="empty-inline"
        >
          此历史汇总中没有导致门禁超限的 Critical / High 条目；不代表所选轮次 CI 已通过。
        </div>
      </div>
    </section>
    </details>
  </div>
</template>

<style scoped>
.source-historical-results { min-width: 0; }
.source-historical-results > summary { cursor: pointer; padding: 12px 0; }
.source-historical-results > p { overflow-wrap: anywhere; }
</style>
