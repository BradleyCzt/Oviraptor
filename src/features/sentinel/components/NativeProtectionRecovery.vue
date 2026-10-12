<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import NativePricedRecoveryApproval from "./NativePricedRecoveryApproval.vue";
import { protectionRecoveryApi as api } from "../recovery/api";
import { approvalMatches, cancellationMatches, canonical, draftMatches, proposalMatches, recoveryDimensions, validOperation, validSources } from "../recovery/contract";
import type { RecoveryFact, RecoveryOperation, RecoveryScope, RecoverySources } from "../recovery/contract";
const props = defineProps<{ scanId: string }>();
const pricedOpened = ref(false);
const pricedScope = computed(() => { try { return scope(); } catch { return null; } });
const opened = ref(false), busy = ref(false), error = ref(""), blocked = ref(false);
const sources = ref<RecoverySources>(), selected = ref("");
const operation = ref<RecoveryOperation>(), proposal = ref<RecoveryFact>(), approval = ref<RecoveryFact>();
const draft = ref<RecoveryFact>(), cancellation = ref<RecoveryFact>(), consent = ref(false);
const taskName = ref(""), urls = ref(""), model = ref(""), proxy = ref("");
const identities = ref(""), identityScope = ref(""), skillIds = ref(""), instruction = ref("");
const methods = ref(["GET"]), mode = ref("multi"), scanMode = ref("standard"), closure = ref("proof");
const dollars = ref(5), deadline = ref("");
const limits = ref(["100000", "100000", "100000", "10", "40", "0", "0", "0", "3", "600000"]);
let generation = 0, disposed = false;
const locked = computed(() => busy.value || blocked.value || !!operation.value);
const source = computed(() => sources.value?.items.find(item => item.selection.protectionReceiptId === selected.value));
const lines = (text: string) => text.split(/\r?\n/).filter(v => v !== "").sort();
const storageKey = (scan: string) => `oviraptor.protection-recovery.v1:${scan}`;
const current = (id: number, scan: string) => !disposed && opened.value && generation === id && props.scanId === scan;
function scope(): RecoveryScope {
  if (!sources.value) throw Error("source_missing");
  return { projectId: sources.value.projectId, taskName: taskName.value, urls: lines(urls.value),
    allowedMethods: [...methods.value].sort(), authSessionIds: lines(identities.value), authSessionScopeId: identityScope.value,
    model: model.value, proxyBindingHash: proxy.value || null, orchestrationMode: mode.value,
    scanMode: scanMode.value, closure: closure.value, skillIds: lines(skillIds.value).map(Number).sort((a,b) => a-b),
    instruction: instruction.value, maxBudgetUsd: Number(dollars.value), deadlineUtc: deadline.value,
    limits: limits.value.map((text, i) => { if (!text && i < 3) return null;
      if (!/^\d+$/.test(text) || !Number.isSafeInteger(Number(text))) throw Error("limit_invalid"); return Number(text); }),
  };
}
async function readReceipts(op: RecoveryOperation, id: number, scan: string) {
  const [a, d, c] = await Promise.all([api.approval(op.requestId), api.draft(op.requestId), api.cancellation(op.requestId)]);
  if (!current(id, scan)) return;
  if (a !== null && !approvalMatches(a, op)) throw Error("approval_mismatch");
  if (d !== null && !draftMatches(d, op)) throw Error("draft_mismatch");
  if (d !== null && a === null) throw Error("approval_missing");
  if (c !== null && !cancellationMatches(c, op, d === null ? undefined : d as RecoveryFact)) throw Error("cancellation_mismatch");
  approval.value = a === null ? undefined : a as RecoveryFact;
  draft.value = d === null ? undefined : d as RecoveryFact;
  cancellation.value = c === null ? undefined : c as RecoveryFact;
}
async function load() {
  if (!opened.value || busy.value) return;
  const id = ++generation, scan = props.scanId;
  busy.value = true; error.value = ""; blocked.value = false;
  try {
    const report = await api.sources(scan);
    if (!current(id, scan)) return;
    if (!validSources(report, scan)) throw Error("sources_mismatch");
    sources.value = report;
    const saved = localStorage.getItem(storageKey(scan));
    if (saved) {
      const op: unknown = JSON.parse(saved);
      if (!validOperation(op, scan)) { blocked.value = true; throw Error("saved_operation_invalid"); }
      operation.value = op; selected.value = op.selection.protectionReceiptId;
      await readReceipts(op, id, scan);
    } else {
      operation.value = undefined; selected.value = report.items[0]?.selection.protectionReceiptId || "";
      taskName.value = `${report.taskName} · 新恢复任务`; urls.value = report.items[0]?.targetUrl || "";
      deadline.value = new Date(Date.now() + 3600000).toISOString();
    }
  } catch { if (current(id, scan)) { blocked.value = true; error.value = "无法核验来源或原操作。请找回状态；未生成替代批准。"; } }
  finally { if (current(id, scan)) busy.value = false; }
}
async function review() {
  if (busy.value || blocked.value || !source.value || approval.value || cancellation.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = ""; proposal.value = undefined; consent.value = false;
  try {
    const selection = operation.value?.selection || source.value.selection;
    const newScope = operation.value?.newScope || scope();
    const p = await api.preview(selection, newScope);
    if (!current(id, scan)) return;
    if (!proposalMatches(p, selection, newScope) || (operation.value && p.proposalHash !== operation.value.proposalHash)) throw Error("proposal_changed");
    proposal.value = p;
  } catch { if (current(id, scan)) error.value = "提案核验未通过。原停机与费用保留；请核对身份、模型、范围、额度及截止时间。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
async function approve() {
  if (busy.value || blocked.value || !consent.value || !proposal.value || !source.value || approval.value || cancellation.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = "";
  try {
    const op: RecoveryOperation = operation.value || { version: 1, requestId: crypto.randomUUID(),
      selection: JSON.parse(JSON.stringify(source.value.selection)), newScope: scope(), proposalHash: String(proposal.value.proposalHash) };
    if (!validOperation(op, scan) || !proposalMatches(proposal.value, op.selection, op.newScope) || proposal.value.proposalHash !== op.proposalHash) throw Error("operation_changed");
    // Persist before the first write IPC. A failed/unknown ACK keeps this UUID.
    localStorage.setItem(storageKey(scan), JSON.stringify(op)); operation.value = op;
    const receipt = await api.approve(op);
    if (!current(id, scan)) return;
    if (!approvalMatches(receipt, op)) throw Error("approval_mismatch");
    approval.value = receipt; consent.value = false;
  } catch { if (current(id, scan)) error.value = "批准结果未确认。先找回原操作状态；重试沿用同一 requestId。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
async function createDraft() {
  const op = operation.value;
  if (busy.value || !op || !approval.value || draft.value || cancellation.value || blocked.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = "";
  try {
    const receipt = await api.createDraft(op.requestId);
    if (!current(id, scan)) return;
    if (!draftMatches(receipt, op)) throw Error("draft_mismatch");
    draft.value = receipt;
  } catch { if (current(id, scan)) error.value = "草稿结果未确认，请找回状态。不会创建另一批准或普通启动任务。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
async function cancel() {
  const op = operation.value, scanId = draft.value?.scanId;
  if (busy.value || !op || typeof scanId !== "string" || cancellation.value || blocked.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = "";
  try {
    await api.cancel(scanId);
    await readReceipts(op, id, scan);
    if (current(id, scan) && !cancellation.value) throw Error("cancel_ack_unconfirmed");
  } catch { if (current(id, scan)) error.value = "取消结果未确认，请找回原签名收据。原操作保留。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
watch([taskName, urls, model, proxy, identities, identityScope, skillIds, instruction, methods, mode, scanMode, closure, dollars, deadline, limits, selected],
  () => { proposal.value = undefined; consent.value = false; }, { deep: true, flush: "sync" });
watch(() => props.scanId, () => {
  generation++; busy.value = false; sources.value = undefined; operation.value = undefined;
  proposal.value = undefined; approval.value = undefined; draft.value = undefined; cancellation.value = undefined;
  error.value = ""; blocked.value = false; consent.value = false;
  if (opened.value) void load();
});
watch(opened, value => { if (value) void load(); else { generation++; busy.value = false; } });
onUnmounted(() => { disposed = true; generation++; });
</script>

<template>
  <section class="recovery-panel">
    <button type="button" @click="opened = !opened">{{ opened ? '收起独立恢复批准' : '独立恢复批准' }}</button>
    <template v-if="opened">
      <p>保留原停机与费用，为明确的新范围单独批准。专用出生与执行尚未开放；此处只能批准、准备或取消待出生草稿。</p>
      <button type="button" :disabled="busy" @click="load">找回原操作状态</button>
      <p v-if="error" role="alert">{{ error }}</p>
      <button type="button" @click="pricedOpened = !pricedOpened">{{ pricedOpened ? '收起精确定价批准' : '精确定价批准（单 URL）' }}</button>
      <NativePricedRecoveryApproval v-if="pricedOpened" :scan-id="scanId" :selection="source?.selection" :new-scope="pricedScope" :disabled="busy || blocked || !!operation" />
      <p v-if="sources && !sources.items.length">没有可核验的原保护收据。</p>
      <template v-if="sources?.items.length">
        <fieldset :disabled="locked">
          <legend>明确的新范围</legend>
          <label>原停机来源<select v-model="selected"><option v-for="item in sources.items" :key="item.selection.protectionReceiptId" :value="item.selection.protectionReceiptId">{{ item.targetUrl }} · {{ item.selection.rootRunId }} · {{ item.responseStatus }}</option></select></label>
          <label>新任务名称<input v-model="taskName" /></label>
          <label>目标 URL（每行一条，保留完整 query 和尾斜线）<textarea v-model="urls" /></label>
          <div>允许的方法<label v-for="method in ['GET','HEAD','OPTIONS','POST','PUT','PATCH','DELETE']" :key="method"><input v-model="methods" type="checkbox" :value="method" />{{ method }}</label></div>
          <label>模型<input v-model="model" placeholder="填写当前配置中的精确模型名称" /></label>
          <label>代理绑定 hash（空为无代理）<input v-model="proxy" /></label>
          <label>新身份 ID（每行一条；不复制原任务身份）<textarea v-model="identities" /></label>
          <label>新身份草稿 scope ID<input v-model="identityScope" /></label>
          <label>编排<select v-model="mode"><option value="single">Single</option><option value="multi">Multi</option></select></label>
          <label>扫描深度<select v-model="scanMode"><option value="quick">quick</option><option value="standard">standard</option><option value="deep">deep</option></select></label>
          <label>闭合目标<select v-model="closure"><option value="breadth">breadth</option><option value="proof">proof</option></select></label>
          <label>Skill ID（每行一条）<textarea v-model="skillIds" /></label>
          <label>指令<textarea v-model="instruction" /></label>
          <label>费用上限 USD<input v-model.number="dollars" type="number" min="0.01" step="0.01" /></label>
          <label>绝对截止时间 UTC<input v-model="deadline" placeholder="2026-10-08T12:00:00Z" /></label>
          <label v-for="(dimension, index) in recoveryDimensions" :key="dimension">{{ dimension }}<input v-model="limits[index]" :disabled="index >= 5 && index <= 7" :placeholder="index < 3 ? '留空表示明确无上限；0 表示禁止' : '非负整数'" /></label>
          <p>browser_actions、controlled_writes、upload_bytes 固定为 0。十维额度仍需专用执行消费者，不代表已可消耗。</p>
        </fieldset>
        <button v-if="!approval && !cancellation" type="button" :disabled="busy || blocked || !source" @click="review">核验完整提案</button>
        <details v-if="proposal || operation" open><summary>待批准的完整新范围</summary><pre>{{ canonical(operation?.newScope || proposal?.newScope) }}</pre></details>
        <template v-if="proposal && !approval && !cancellation">
          <label><input v-model="consent" type="checkbox" :disabled="busy" />我明确批准所显示的新范围；保留原停机和全部原费用。此批准不恢复原任务、不允许未知工作重放。</label>
          <button type="button" :disabled="busy || !consent" @click="approve">{{ operation ? '沿用原 requestId 重试批准' : '明确批准新范围' }}</button>
        </template>
        <p v-if="operation">原操作 requestId：{{ operation.requestId }}</p>
        <p v-if="approval">原签名批准已找回，等待专用出生。</p>
        <button v-if="approval && !draft && !cancellation" type="button" :disabled="busy" @click="createDraft">准备唯一待出生草稿</button>
        <template v-if="draft"><p>新草稿：{{ draft.scanId }}。尚未启动 Root 或目标请求。</p><button v-if="!cancellation" type="button" :disabled="busy" @click="cancel">取消草稿并撤销此批准的未出生授权</button></template>
        <p v-if="cancellation">已核验原签名撤权收据：{{ cancellation.cancellationId }}。原停机、费用与历史保留。</p>
      </template>
    </template>
  </section>
</template>
<style scoped>
.recovery-panel { margin-top: 1rem; padding: 1rem; border: 1px solid var(--border-color, #777); border-radius: 8px; }
fieldset { display: grid; gap: .65rem; margin: 1rem 0; }
label { display: grid; gap: .2rem; }
fieldset > div { display: flex; flex-wrap: wrap; gap: .6rem; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; }
textarea { min-height: 4rem; }
</style>
