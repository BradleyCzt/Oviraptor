<script setup lang="ts">
import { Activity, Bug, CheckCircle2, ClipboardCheck, Save, X } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelFinding, SentinelValidation } from "../../types";
import {
  createSentinelLabels,
  endpointUrl,
  json,
  safeSeverity,
  text,
} from "../../presentation";

const { severityLabel, verdictLabel } = createSentinelLabels(useI18n().tr);
const emit = defineEmits<{
  "select-finding": [id: number];
  "close-validation": [];
}>();
defineProps<{
  vulnerabilityRows: SentinelFinding[];
  focusedVulnerabilityRows: SentinelFinding[];
  pocRows: SentinelFinding[];
  selectedFindingId?: number;
  selectedUrl: string;
  validationEditor?: SentinelFinding;
  validationForm: { verdict: string; severity: string; note: string; evidence: string };
  editValidation: (item: SentinelFinding, verdict?: string) => void;
  saveValidation: () => Promise<void>;
  validationFor: (item: SentinelFinding) => SentinelValidation | undefined;
  effectiveSeverity: (item: SentinelFinding) => string;
  vulnerabilityUpdateHistory: (item: SentinelFinding) => any[];
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block">
      <div class="block-title">
        <Bug :size="16" />
        <div>
          <strong>漏洞发现</strong
          ><small
            >历史与其他来源记录在此保留供核查；人工验证不等于原生 Reviewer 确认。主看板仅统计 Reviewer 确认。</small
          >
        </div>
      </div>
      <div class="vulnerability-master-detail">
        <aside class="vulnerability-index-list">
          <button
            v-for="item in vulnerabilityRows"
            :key="`vuln-index-${item.id}`"
            :class="{ active: selectedFindingId === item.id }"
            @click="emit('select-finding', item.id)"
          >
            <span :class="`severity-badge ${effectiveSeverity(item)}`">{{ severityLabel(effectiveSeverity(item)) }}</span>
            <div><strong>{{ item.title || json(item.recordJson).title || item.recordKey }}</strong><small>{{ json(item.recordJson).method || "GET" }} {{ json(item.recordJson).url || "/" }}</small></div>
            <em class="validation-chip">原始记录 · 未经原生 Reviewer 审核</em>
            <em v-if="validationFor(item)" :class="`validation-chip ${validationFor(item)?.verdict}`">{{ verdictLabel(validationFor(item)?.verdict || "") }}</em>
          </button>
          <div v-if="!vulnerabilityRows.length" class="empty-inline">当前 URL 没有漏洞记录</div>
        </aside>
      <div class="vulnerability-list focused">
        <article
          v-for="item in focusedVulnerabilityRows"
          :key="item.id"
          :class="`vuln-card severity-border-${effectiveSeverity(item)}`"
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
                >原始记录 · 未经原生 Reviewer 审核 · {{
                  json(item.recordJson).type || "vulnerability"
                }}
                · 原始等级
                {{ severityLabel(safeSeverity(item.severity)) }} ·
                CVSS {{ json(item.recordJson).cvss ?? "—" }} ·
                置信度 {{ json(item.recordJson).confidence || "未说明" }} ·
                {{ json(item.recordJson).method || "GET" }}
                {{ json(item.recordJson).url || "/" }}</small
              ><small
                v-if="
                  json(item.recordJson).cve ||
                  json(item.recordJson).cwe
                "
                >{{ json(item.recordJson).cve || "无 CVE" }} ·
                {{ json(item.recordJson).cwe || "无 CWE" }} ·
                修复工作量
                {{
                  json(item.recordJson).fix_effort || "未知"
                }}</small
              >
            </div>
            <span
              v-if="validationFor(item)"
              :class="`validation-chip ${validationFor(item)?.verdict}`"
              ><CheckCircle2 :size="13" />{{
                verdictLabel(validationFor(item)?.verdict || "")
              }}
              · {{ severityLabel(effectiveSeverity(item)) }}</span
            >
          </header>
          <div class="vuln-columns">
            <div>
              <span>漏洞描述</span>
              <p>{{ json(item.recordJson).description || "—" }}</p>
            </div>
            <div>
              <span>技术分析</span>
              <p>
                {{
                  json(item.recordJson).technical_analysis || "—"
                }}
              </p>
            </div>
            <div>
              <span>证据</span>
              <pre>{{ text(json(item.recordJson).evidence) }}</pre>
            </div>
            <div>
              <span>影响</span>
              <p>
                {{
                  json(item.recordJson).impact ||
                  json(item.recordJson).detail ||
                  "—"
                }}
              </p>
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
            <div>
              <span>PoC / 复现</span>
              <pre>{{
                json(item.recordJson).pocRequest ||
                json(item.recordJson).poc_description ||
                "—"
              }}</pre>
            </div>
            <div
              v-if="
                json(item.recordJson).counterevidence ||
                json(item.recordJson).counterEvidence
              "
            >
              <span>反证检查</span>
              <p>{{
                json(item.recordJson).counterevidence ||
                json(item.recordJson).counterEvidence
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).confidence_rationale ||
                json(item.recordJson).confidenceRationale
              "
            >
              <span>置信度依据</span>
              <p>{{
                json(item.recordJson).confidence_rationale ||
                json(item.recordJson).confidenceRationale
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).severity_change_conditions ||
                json(item.recordJson).severityChangeConditions
              "
            >
              <span>等级变化条件</span>
              <p>{{
                json(item.recordJson).severity_change_conditions ||
                json(item.recordJson).severityChangeConditions
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).fix_verification ||
                json(item.recordJson).fixVerification
              "
            >
              <span>修复验证</span>
              <p>{{
                json(item.recordJson).fix_verification ||
                json(item.recordJson).fixVerification
              }}</p>
            </div>
            <div v-if="json(item.recordJson).cvss_breakdown">
              <span>CVSS 明细</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).cvss_breakdown,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div v-if="json(item.recordJson).code_locations">
              <span>代码位置 / 修复差异</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).code_locations,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div v-if="json(item.recordJson).assumptions">
              <span>前提与限制</span>
              <p>{{ json(item.recordJson).assumptions }}</p>
            </div>
            <div v-if="json(item.recordJson).dependency_metadata">
              <span>依赖信息</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).dependency_metadata,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div
              v-if="vulnerabilityUpdateHistory(item).length"
              class="vulnerability-update-history"
            >
              <span>结论修订历史</span>
              <ol>
                <li
                  v-for="(revision, index) in vulnerabilityUpdateHistory(item)"
                  :key="`${item.id}-revision-${index}`"
                >
                  <b>{{ revision.updated_at || revision.timestamp || `修订 ${index + 1}` }}</b>
                  <p>{{ revision.update_reason || revision.reason || revision.summary || "结论已修订" }}</p>
                  <small v-if="revision.dropped_fields?.length">替换字段：{{ revision.dropped_fields.join("、") }}</small>
                </li>
              </ol>
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
              >已保存：{{
                verdictLabel(validationFor(item)?.verdict || "")
              }}
              / {{ severityLabel(effectiveSeverity(item)) }}</span
            >
          </footer>
          <div
            v-if="validationEditor?.id === item.id"
            class="validation-editor inline-validation-editor"
          >
            <div class="block-title">
              <ClipboardCheck :size="16" />
              <div>
                <strong
                  >人工验证：{{
                    item.title || item.recordKey
                  }}</strong
                ><small>保存后立即更新当前卡片与统计</small>
              </div>
              <button
                class="icon-button"
                @click="emit('close-validation')"
              >
                <X :size="15" />
              </button>
            </div>
            <div class="verdict-picker">
              <button
                v-for="choice in [
                  { v: 'true_positive', l: '真实漏洞' },
                  { v: 'false_positive', l: '误报' },
                  { v: 'needs_more', l: '需要补证' },
                ]"
                :key="choice.v"
                :class="{
                  active: validationForm.verdict === choice.v,
                }"
                @click="validationForm.verdict = choice.v"
              >
                {{ choice.l }}
              </button>
            </div>
            <div class="validation-form-grid">
              <label class="field"
                ><span>确认后严重度</span
                ><select v-model="validationForm.severity">
                  <option value="critical">严重</option>
                  <option value="high">高危</option>
                  <option value="medium">中危</option>
                  <option value="low">低危</option>
                  <option value="info">信息</option>
                </select></label
              ><label class="field"
                ><span>验证备注</span
                ><textarea
                  v-model="validationForm.note"
                  rows="3"
                  placeholder="复现过程、判断理由、限制条件"
                ></textarea></label
              ><label class="field span-two"
                ><span>证据 / 请求响应 / 截图路径</span
                ><textarea
                  v-model="validationForm.evidence"
                  rows="5"
                  placeholder="粘贴关键请求响应，或填写本地证据文件路径"
                ></textarea>
              </label>
            </div>
            <footer>
              <button
                class="button ghost"
                @click="emit('close-validation')"
              >
                取消</button
              ><button
                class="button primary"
                @click="saveValidation"
              >
                <Save :size="14" />保存并更新风险
              </button>
            </footer>
          </div>
        </article>
        <div v-if="!vulnerabilityRows.length" class="empty-inline">
          当前 URL 没有漏洞记录
        </div>
      </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Activity :size="16" />
        <div><strong>PoC 测试记录</strong></div>
      </div>
      <div class="poc-list">
        <article v-for="item in pocRows" :key="item.id">
          <strong>{{
            json(item.recordJson).name || item.title
          }}</strong
          ><span>{{
            json(item.recordJson).result || "unknown"
          }}</span
          ><code
            >{{ json(item.recordJson).method || "GET" }}
            {{
              endpointUrl(selectedUrl, json(item.recordJson).url)
            }}</code
          >
          <p>
            {{
              json(item.recordJson).note ||
              json(item.recordJson).responseSnippet ||
              "—"
            }}
          </p>
        </article>
        <div v-if="!pocRows.length" class="empty-inline">
          没有 PoC 测试记录
        </div>
      </div>
    </section>
  </div>
</template>
