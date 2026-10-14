<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import {
  Activity,
  BookOpen,
  BrainCircuit,
  Check,
  Cpu,
  Layers,
  MessagesSquare,
  Sparkles,
  Trash2,
  Wrench,
} from "@lucide/vue";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../../../api";
import { useI18n } from "../../../i18n";
import type { AgentLearningCandidate, AgentKnowledgeEntry, AgentSkill } from "../../skills/catalogTypes";
import type { AgentTraceSummary } from "../types";
import InlineConfirm from "../../../shared/components/InlineConfirm.vue";
import { tracePresentation } from "../traces/presentation";
import { useLiveTrace } from "../traces/useLiveTrace";
import { useTraceSelection } from "../traces/useTraceSelection";
import TraceHubToolbar from "./TraceHubToolbar.vue";
import RootDecisionSummary from "./RootDecisionSummary.vue";
import { isRootModelRound } from "../traces/rootDecisionContract";

const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
}>();
const { tr } = useI18n();
const {
  format, eventLabel, shortSession, candidateItems, patchSummary,
  knowledgeKind, qualityLabel, canConvert, candidateGate,
  candidateProducer, knowledgeProvenance, formatBytes,
} = tracePresentation(tr);
const traces = ref<AgentTraceSummary[]>([]);
const knowledge = ref<AgentKnowledgeEntry[]>([]);
const candidates = ref<AgentLearningCandidate[]>([]);
const skills = ref<AgentSkill[]>([]);
const { selectedId, detail, detailLoading, selectTrace, refreshLiveTrace } = useTraceSelection(
  traces, (error) => emit("notify", "error", String(error)),
);
const loading = ref(false);
const busy = ref("");
const aggregateType = ref("web");
const sourceInput = ref("");
const deleteKnowledge = ref<AgentKnowledgeEntry>();
const deleteCandidate = ref<AgentLearningCandidate>();
let catalogRevision = 0;
let disposed = false;
onUnmounted(() => { disposed = true; catalogRevision++; });

const totals = computed(() => ({
  tasks: traces.value.length,
  messages: traces.value.reduce((sum, item) => sum + item.messageCount, 0),
  tools: traces.value.reduce((sum, item) => sum + item.toolCallCount, 0),
  knowledge: knowledge.value.length,
}));
const selectedCandidate = computed(() =>
  candidates.value.find((item) => item.scanId === selectedId.value),
);

async function load() {
  if (disposed) return;
  const requestRevision = ++catalogRevision;
  loading.value = true;
  try {
    const snapshot = await Promise.all([
      api.listAgentTraces(),
      api.listAgentKnowledge(),
      api.listAgentLearningCandidates(),
      api.listAgentSkills(),
    ]);
    if (disposed || requestRevision !== catalogRevision) return;
    [traces.value, knowledge.value, candidates.value, skills.value] = snapshot;
    if (!selectedId.value && traces.value[0]) {
      await selectTrace(traces.value[0].scanId);
    }
  } catch (error) {
    if (!disposed && requestRevision === catalogRevision) emit("notify", "error", String(error));
  } finally {
    if (!disposed && requestRevision === catalogRevision) loading.value = false;
  }
}
async function generateCandidate() {
  if (!selectedId.value) return;
  busy.value = "candidate";
  try {
    const candidate = await api.generateAgentLearningCandidate(selectedId.value);
    candidates.value = [candidate, ...candidates.value.filter((item) => item.id !== candidate.id)];
    emit("notify", "success", tr("已生成待审核学习候选", "Learning candidate generated for review"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function reviewCandidate(candidate: AgentLearningCandidate, decision: "accepted" | "rejected") {
  busy.value = `candidate-${candidate.id}`;
  try {
    const updated = await api.reviewAgentLearningCandidate(candidate.id, decision, candidate.targetSkillId);
    Object.assign(candidate, updated);
    emit("notify", decision === "accepted" ? "success" : "info", decision === "accepted" ? tr("候选已接受，可沉淀为 Skill", "Candidate accepted; ready to apply as Skill") : tr("候选已拒绝，不会注入后续任务", "Candidate rejected and excluded from future tasks"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function applyCandidate(candidate: AgentLearningCandidate) {
  busy.value = `apply-${candidate.id}`;
  try {
    if (candidate.targetSkillId) {
      const updated = await api.reviewAgentLearningCandidate(candidate.id, "accepted", candidate.targetSkillId);
      Object.assign(candidate, updated);
    }
    const skillId = await api.applyAgentLearningCandidate(candidate.id);
    candidate.status = "applied";
    candidate.targetSkillId = skillId;
    emit("notify", "success", tr(`已沉淀为 Skill #${skillId}，后续扫描会自动注入`, `Applied as Skill #${skillId}; future scans will inject it`));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function removeCandidate() {
  if (!deleteCandidate.value) return;
  const candidate = deleteCandidate.value;
  busy.value = `delete-candidate-${candidate.id}`;
  try {
    await api.deleteAgentLearningCandidate(candidate.id);
    candidates.value = candidates.value.filter((item) => item.id !== candidate.id);
    deleteCandidate.value = undefined;
    emit("notify", "success", tr("学习候选已删除", "Learning candidate deleted"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function aggregate() {
  busy.value = "aggregate";
  try {
    const entry = await api.aggregateAgentKnowledge(aggregateType.value);
    knowledge.value = [entry, ...knowledge.value.filter((item) => item.id !== entry.id)];
    emit("notify", "success", tr("同类轨迹已聚合提炼", "Similar traces aggregated"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function ingestSource(forceRefresh = false) {
  const source = sourceInput.value.trim();
  if (!source) {
    emit("notify", "info", tr("请输入公开文章 URL 或本地 Markdown 路径", "Enter a public article URL or local Markdown path"));
    return;
  }
  busy.value = "source";
  try {
    const entry = await api.ingestAgentKnowledgeSource(source, forceRefresh);
    knowledge.value = [entry, ...knowledge.value.filter((item) => item.id !== entry.id)];
    emit("notify", "success", tr("文章/本地 HTML 已缓存并提炼为方法卡片；后续扫描只读取本地知识", "Article/local HTML cached and distilled into method cards; later scans use local knowledge"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function removeKnowledge() {
  if (!deleteKnowledge.value) return;
  const entry = deleteKnowledge.value;
  busy.value = `delete-${entry.id}`;
  try {
    await api.deleteAgentKnowledge(entry.id);
    knowledge.value = knowledge.value.filter((item) => item.id !== entry.id);
    const trace = traces.value.find((item) => item.scanId === entry.scanId);
    if (trace) trace.knowledgeId = undefined;
    deleteKnowledge.value = undefined;
    emit("notify", "success", tr("知识条目已删除", "Knowledge entry deleted"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function convert(entry: AgentKnowledgeEntry) {
  busy.value = `skill-${entry.id}`;
  try {
    entry.skillId = await api.convertAgentKnowledgeToInstruction(entry.id);
    emit("notify", "success", tr("知识已转换为可选 Skill", "Knowledge converted to a selectable Skill"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = "";
  }
}
async function exportKnowledge() {
  try {
    const path = await api.exportAgentKnowledge();
    emit("notify", "success", tr(`知识库已导出：${path}`, `Knowledge exported: ${path}`));
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function importKnowledge() {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Oviraptor Knowledge", extensions: ["json"] }],
  });
  if (!path) return;
  try {
    const count = await api.importAgentKnowledge(path);
    await load();
    emit("notify", "success", tr(`已导入 ${count} 条知识`, `Imported ${count} knowledge entries`));
  } catch (error) {
    emit("notify", "error", String(error));
  }
}

useLiveTrace(load, refreshLiveTrace, () => selectedId.value, () => {
  emit("notify", "info", tr("轨迹实时通知连接失败，已使用定期补查", "Live trace notifications unavailable; using periodic reconciliation"));
});
</script>

<template>
  <section class="trace-hub">
    <TraceHubToolbar
      v-model:source-input="sourceInput"
      v-model:aggregate-type="aggregateType"
      :busy="busy"
      :loading="loading"
      :knowledge-count="knowledge.length"
      @ingest-source="ingestSource"
      @refresh="load"
      @aggregate="aggregate"
      @import-knowledge="importKnowledge"
      @export-knowledge="exportKnowledge"
    />

    <div class="trace-kpis">
      <article><Activity :size="18" /><span>{{ tr("可分析任务", "Traceable tasks") }}</span><strong>{{ totals.tasks }}</strong></article>
      <article><MessagesSquare :size="18" /><span>{{ tr("消息记录", "Messages") }}</span><strong>{{ format(totals.messages) }}</strong></article>
      <article><Wrench :size="18" /><span>{{ tr("工具调用", "Tool calls") }}</span><strong>{{ format(totals.tools) }}</strong></article>
      <article><BookOpen :size="18" /><span>{{ tr("知识条目", "Knowledge") }}</span><strong>{{ totals.knowledge }}</strong></article>
    </div>

    <div class="trace-layout">
      <aside class="panel trace-task-list">
        <header><strong>{{ tr("扫描任务", "Scan tasks") }}</strong><small>{{ traces.length }}</small></header>
        <button v-for="trace in traces" :key="trace.scanId" :class="{ active: selectedId === trace.scanId }" @click="selectTrace(trace.scanId)">
          <span><strong>{{ trace.taskName || trace.projectName }}</strong><small>{{ trace.scanType }} · {{ trace.status }} · {{ trace.sourceAuthority === 'historical_external' ? tr('历史导入 · 未复核', 'Historical import · unreviewed') : tr('Native 运行账本', 'Native run ledger') }} · {{ trace.createdAt }}</small></span>
          <em v-if="trace.knowledgeId"><BookOpen :size="12" />{{ tr("已沉淀", "Saved") }}</em>
        </button>
        <div v-if="!traces.length && !loading" class="empty-state small">{{ tr("还没有可读取的运行轨迹", "No scan traces available") }}</div>
      </aside>

      <section class="panel trace-detail">
        <div v-if="detail" class="trace-detail-content">
          <header>
            <div><span class="eyebrow">{{ detail.summary.scanId }}</span><h3>{{ detail.summary.taskName || detail.summary.projectName }}</h3><p>{{ detail.summary.projectName }} · {{ detail.summary.scanType }} · {{ detail.summary.status }}</p></div>
            <button class="button primary" :disabled="busy === 'candidate'" @click="generateCandidate"><Sparkles :size="15" />{{ selectedCandidate ? tr("更新候选", "Refresh candidate") : tr("生成候选", "Create candidate") }}</button>
          </header>
          <div class="trace-runtime-grid">
            <div><span>{{ tr("轨迹来源", "Trace source") }}</span><strong>{{ detail.summary.sourceAuthority === 'historical_external' ? tr('历史导入 · 只读 · 不可执行', 'Historical import · read-only · non-executable') : tr('Native 运行账本', 'Native run ledger') }}</strong></div>
            <div><span>{{ tr("模型", "Model") }}</span><strong>{{ detail.summary.model || "—" }}</strong></div>
            <div><span>Agent</span><strong>{{ detail.summary.agentCount }}</strong></div>
            <div><span>{{ tr("模型调用", "Model calls") }}</span><strong>{{ detail.summary.llmRequests }}</strong></div>
            <div><span>{{ tr("总 Token", "Total tokens") }}</span><strong>{{ format(detail.summary.totalTokens) }}<small v-if="detail.summary.tokenUsageEstimated"> · {{ tr("估算", "estimated") }}</small></strong></div>
            <div><span>{{ tr("缓存输入", "Cached input") }}</span><strong>{{ format(detail.summary.cachedTokens) }}</strong></div>
            <div><span>{{ tr("Hook 请求", "Hook requests") }}</span><strong>{{ detail.summary.hookedRequestCount }}</strong></div>
            <div><span>request_usage_entries</span><strong>{{ detail.summary.usageEntryCount }}</strong></div>
            <div><span>usage agents</span><strong>{{ detail.summary.usageAgentCount }}</strong></div>
            <div><span>{{ tr("提示词哈希", "Prompt hash") }}</span><code>{{ detail.summary.instructionHash.slice(0, 16) || "—" }}</code></div>
          </div>
          <section v-if="detail.promptAudit" class="prompt-audit-panel">
            <header>
              <div>
                <strong>{{ tr("提示词审计快照", "Prompt audit snapshot") }}</strong>
                <small>{{ detail.promptAudit.recordedAt }}</small>
              </div>
              <span :class="detail.promptAudit.captureMode">{{
                detail.promptAudit.captureMode === "full"
                  ? tr("本机脱敏内容", "Redacted local content")
                  : tr("仅元数据", "Metadata only")
              }}</span>
            </header>
            <div class="prompt-audit-grid">
              <div><span>{{ tr("捕获层级", "Capture level") }}</span><strong>{{ tr("应用生成指令", "App-generated instruction") }}</strong></div>
              <div><span>{{ tr("模型请求记录", "Model request records") }}</span><strong :class="{ 'not-exact': !detail.summary.exactRequestCapture }">{{ detail.summary.exactRequestCapture ? (detail.summary.sourceAuthority === 'historical_external' ? tr("历史 Hook 导入；脱敏、未复核", "Imported historical hook; redacted, unreviewed") : tr("已捕获；界面脱敏", "Captured; redacted in UI")) : tr("未捕获", "Not captured") }}</strong></div>
              <div><span>{{ tr("部署 / 策略", "Deployment / policy") }}</span><strong>{{ detail.promptAudit.deployment === "local" ? tr("本地", "Local") : tr("云端", "Cloud") }} · {{ detail.promptAudit.fullPower ? tr("火力全开", "Full power") : tr("受控", "Governed") }}</strong></div>
              <div><span>SHA-256 / {{ tr("字符数", "characters") }}</span><code>{{ detail.promptAudit.instructionSha256.slice(0, 16) }} · {{ format(detail.promptAudit.instructionChars) }}</code></div>
            </div>
            <p>{{ detail.promptAudit.notice }}</p>
            <details v-if="detail.promptAudit.instruction" class="prompt-audit-content">
              <summary>{{ tr("查看本应用生成的脱敏指令", "View redacted app-generated instruction") }}</summary>
              <pre>{{ detail.promptAudit.instruction }}</pre>
            </details>
          </section>
          <section v-else class="prompt-audit-empty">
            <strong>{{ tr("提示词审计未记录", "Prompt audit not recorded") }}</strong>
            <span>{{ tr("该任务创建时审计可能处于关闭状态，或属于启用审计前的历史任务。", "Audit may have been off when this task was created, or this is a historical task from before audit support.") }}</span>
          </section>
          <section class="trace-tools">
            <header><strong>{{ tr("工具画像", "Tool profile") }}</strong><small>{{ detail.summary.toolCallCount }} calls · {{ detail.summary.toolResultCount }} results</small></header>
            <div><span v-for="tool in detail.summary.tools" :key="tool.name"><Wrench :size="12" /><b>{{ tool.name }}</b><em>{{ tool.calls }}/{{ tool.results }}</em></span><small v-if="!detail.summary.tools.length">{{ tr("没有结构化工具记录", "No structured tool records") }}</small></div>
          </section>
          <section class="trace-timeline">
            <header><strong>{{ tr("调用时间线", "Invocation timeline") }}</strong><small>{{ detail.events.length }} {{ tr("条本机事件", "local events") }}</small></header>
            <article class="trace-event" v-for="event in detail.events" :key="event.id">
              <span class="trace-event-icon" :class="event.eventType"><Cpu v-if="event.eventType === 'reasoning' || event.eventType === 'model_request'" :size="13" /><Wrench v-else-if="event.eventType.includes('function')" :size="13" /><MessagesSquare v-else :size="13" /></span>
              <div><strong>{{ event.name || eventLabel(event.eventType) }}</strong><small>{{ eventLabel(event.eventType) }} · Agent {{ shortSession(event.sessionId) }}<template v-if="event.targetUrl"> · {{ event.targetUrl }}</template></small></div>
              <em>{{ isRootModelRound(event) ? tr("已记录", "Recorded") : event.status || event.role || "recorded" }}</em><time>{{ event.createdAt }}</time>
              <RootDecisionSummary v-if="isRootModelRound(event)" :event="event" :source-authority="detail.summary.sourceAuthority" />
              <details v-else-if="event.detail" class="trace-event-detail">
                <summary>{{ tr("查看完整详情", "View full detail") }} · {{ formatBytes(event.detailSize) }}<span v-if="event.detailTruncated"> · {{ tr("限长预览", "preview") }}</span></summary>
                <pre>{{ event.detail }}</pre>
              </details>
            </article>
          </section>
        </div>
        <div v-else class="empty-state" role="status"><BrainCircuit :size="30" /><p>{{ detailLoading ? tr("正在读取任务轨迹…", "Loading task trace…") : tr("选择任务查看 Agent 调用轨迹", "Select a task to inspect its Agent trace") }}</p></div>
      </section>
    </div>

    <section v-if="candidates.length" class="knowledge-section learning-candidate-section">
      <header><div><span class="eyebrow">LEARNING LOOP</span><h3>{{ tr("待审核更新候选", "Learning candidates") }}</h3><p>{{ tr("请求、代码和工具结果先规范化成跨模型一致的事实层；当前模型只生成可审核的建议。接受后按 Markdown 章节确定性沉淀，不再调用第二个模型改写。", "Requests, code, and tool results are normalized into a model-independent fact layer; the active model only proposes a reviewable update. Accepted patches are applied deterministically by Markdown section without a second model rewrite.") }}</p></div></header>
      <div class="knowledge-grid">
        <article v-for="candidate in candidates" :key="candidate.id" class="panel knowledge-card learning-candidate-card">
          <header><Sparkles :size="17" /><div><span>{{ candidate.scanType }} · {{ candidate.status }}</span><span class="quality-gate">{{ candidateGate(candidate) }}</span><span v-if="candidate.targetSkillId"><Check :size="12" />Skill #{{ candidate.targetSkillId }}</span></div></header>
          <h3>{{ candidate.title }}</h3><p>{{ candidate.summary }}</p>
          <div class="knowledge-provenance"><Cpu :size="12" /><span>{{ candidateProducer(candidate) }}</span><code>{{ String(candidate.candidate.canonicalKey || candidate.sourceHash).slice(0, 12) }}</code></div>
          <div class="knowledge-patterns"><span v-for="idea in (candidate.candidate.newIdeas || []).slice(0, 5)" :key="idea.title">{{ idea.title }}</span></div>
          <div class="candidate-insights">
            <section v-if="candidateItems(candidate, 'newIdeas').length"><strong>{{ tr("新思路 / 新测试手法", "New ideas / techniques") }}</strong><article v-for="item in candidateItems(candidate, 'newIdeas')" :key="item.title"><b>{{ item.title }}</b><small v-if="item.detail">{{ item.detail }}</small></article></section>
            <section v-if="candidateItems(candidate, 'weakSteps').length"><strong>{{ tr("不足与薄弱步骤", "Weak or missing steps") }}</strong><article v-for="item in candidateItems(candidate, 'weakSteps')" :key="item.title"><b>{{ item.title }}</b><small v-if="item.detail">{{ item.detail }}</small></article></section>
            <section v-if="candidateItems(candidate, 'redundantSteps').length"><strong>{{ tr("多余与累赘", "Redundant work") }}</strong><article v-for="item in candidateItems(candidate, 'redundantSteps')" :key="item.title"><b>{{ item.title }}</b><small v-if="item.detail">{{ item.detail }}</small></article></section>
            <section v-if="candidateItems(candidate, 'externalKnowledgeRequests').length"><strong>{{ tr("建议学习的公开知识", "Suggested public knowledge") }}</strong><article v-for="item in candidateItems(candidate, 'externalKnowledgeRequests')" :key="item.title"><b>{{ item.title }}</b><small v-if="item.detail">{{ item.detail }}</small></article></section>
            <section v-if="candidate.candidate.skillPatch"><strong>{{ tr("拟议 Skill 补丁", "Proposed Skill patch") }}</strong><article><b>{{ patchSummary(candidate) }}</b><small>{{ candidate.candidate.skillPatch.reasoning || tr("应用时按 Markdown 章节合并，不覆盖未涉及内容。", "Applied by Markdown section without overwriting unrelated content.") }}</small></article></section>
          </div>
          <details><summary>{{ tr("查看模型提炼 JSON", "View model refinement JSON") }}</summary><pre>{{ JSON.stringify(candidate.candidate, null, 2) }}</pre></details>
          <div v-if="candidate.status === 'pending' || candidate.status === 'accepted'" class="candidate-target-skill"><label>{{ tr("沉淀目标（可选）", "Target Skill (optional)") }}</label><select v-model.number="candidate.targetSkillId"><option :value="undefined">{{ tr("新建 Skill", "Create a new Skill") }}</option><option v-for="skill in skills" :key="skill.id" :value="skill.id">{{ skill.name }}{{ skill.builtin ? tr("（内置增强副本）", " (built-in clone)") : "" }}</option></select></div>
          <footer><code>{{ candidate.sourceHash.slice(0, 12) }}</code><div><button v-if="candidate.status === 'pending'" class="button ghost compact" :disabled="busy === `candidate-${candidate.id}`" @click="reviewCandidate(candidate, 'rejected')">{{ tr("拒绝", "Reject") }}</button><button v-if="candidate.status === 'pending'" class="button secondary compact" :disabled="busy === `candidate-${candidate.id}`" @click="reviewCandidate(candidate, 'accepted')"><Check :size="13" />{{ tr("接受候选", "Accept") }}</button><button v-if="candidate.status === 'accepted'" class="button primary compact" :disabled="busy === `apply-${candidate.id}`" @click="applyCandidate(candidate)"><Sparkles :size="13" />{{ tr("沉淀为 Skill", "Apply as Skill") }}</button><button class="icon-button danger candidate-delete-button" :title="tr('删除候选', 'Delete candidate')" @click="deleteCandidate = candidate"><Trash2 :size="14" /></button></div></footer>
          <InlineConfirm v-if="deleteCandidate?.id === candidate.id" :title="tr(`删除候选「${candidate.title}」？`, `Delete candidate “${candidate.title}”?`)" :detail="candidate.status === 'applied' ? tr('只删除候选记录，不删除已经生成的 Skill。', 'Only the candidate record is deleted; the generated Skill remains.') : tr('删除后可在补充证据后重新生成，不影响扫描原始记录。', 'You can regenerate it after adding evidence; the original scan is unchanged.')" :busy="busy === `delete-candidate-${candidate.id}`" @cancel="deleteCandidate = undefined" @confirm="removeCandidate" />
        </article>
      </div>
    </section>

    <section class="knowledge-section">
      <header><div><span class="eyebrow">LOCAL KNOWLEDGE</span><h3>{{ tr("候选与聚合知识", "Candidate and aggregated knowledge") }}</h3><p>{{ tr("单任务知识可人工转换，但不会自动影响后续精炼；只有至少两个不同任务支持的规范化模式才进入自动复用。同一任务换模型重复提炼不会增加支持数。", "A task card can be converted manually but cannot influence later refinement automatically; only normalized patterns supported by at least two distinct scans are auto-reused. Re-running another model on the same scan does not add support.") }}</p></div></header>
      <div class="knowledge-grid">
        <article v-for="entry in knowledge" :key="entry.id" class="panel knowledge-card">
          <header><BookOpen :size="17" /><div><span :class="{ aggregate: entry.patterns.knowledgeKind === 'aggregate' }">{{ knowledgeKind(entry) }} · {{ qualityLabel(entry) }}</span><span v-if="entry.skillId"><Check :size="12" />Skill #{{ entry.skillId }}</span></div></header>
          <h3>{{ entry.title }}</h3><p>{{ entry.summary }}</p>
          <div class="knowledge-provenance"><Layers :size="12" /><span>{{ knowledgeProvenance(entry) }}</span><code>{{ String(entry.patterns.canonicalKey || entry.sourceHash).slice(0, 12) }}</code></div>
          <div class="knowledge-patterns"><span v-for="tool in (entry.patterns.tools as string[] || []).slice(0, 6)" :key="tool">{{ tool }}</span></div>
          <footer><code>{{ entry.sourceHash.slice(0, 12) }}</code><div><button class="button ghost compact" :disabled="Boolean(entry.skillId) || !canConvert(entry) || busy === `skill-${entry.id}`" @click="convert(entry)"><Sparkles :size="13" />{{ entry.skillId ? tr("已转为 Skill", "Skill created") : canConvert(entry) ? tr("转为 Skill", "Create Skill") : tr("需重新提炼", "Refine first") }}</button><button class="icon-button danger" :title="tr('删除知识', 'Delete knowledge')" @click="deleteKnowledge = entry"><Trash2 :size="14" /></button></div></footer>
          <InlineConfirm
            v-if="deleteKnowledge?.id === entry.id"
            :title="tr(`删除知识「${entry.title}」？`, `Delete knowledge “${entry.title}”?`)"
            :detail="entry.skillId ? tr('知识会被删除；已经生成的 Skill 将继续保留，可在 Skills 中单独删除。', 'The knowledge will be deleted; its existing Skill remains and can be deleted separately.') : tr('只删除本地知识，不影响历史扫描任务。', 'Only local knowledge is removed; scan history is unchanged.')"
            :busy="busy === `delete-${entry.id}`"
            @cancel="deleteKnowledge = undefined"
            @confirm="removeKnowledge"
          />
        </article>
        <div v-if="!knowledge.length" class="empty-state panel">{{ tr("分析任务后，去目标化经验会保存在这里。", "Analyze a task to save reusable, target-neutral knowledge here.") }}</div>
      </div>
    </section>
  </section>
</template>

<style src="./traceHubPresentation.css"></style>
