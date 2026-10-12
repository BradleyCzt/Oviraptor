<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { protectionRecoveryApi as api } from "../recovery/api";
import { canonical } from "../recovery/contract";
import type { RecoveryFact, RecoveryScope, RecoverySelection } from "../recovery/contract";
import { decimalUsdNanos, pricedApprovalMatches, pricedCancellationMatches, pricedProposalMatches,
  validPricedRecoveryOperation, validRecoveryPricing } from "../recovery/priced";
import type { PricedRecoveryOperation, RecoveryPricingInput } from "../recovery/priced";
const props = defineProps<{ scanId: string; selection?: RecoverySelection; newScope: RecoveryScope | null; disabled?: boolean }>();
const busy = ref(false), blocked = ref(false), error = ref("");
const operation = ref<PricedRecoveryOperation>(), proposal = ref<RecoveryFact>(), approval = ref<RecoveryFact>(), cancellation = ref<RecoveryFact>();
const usd = ref(""), revision = ref(""), inputRate = ref(""), cacheRate = ref(""), outputRate = ref(""), requestRate = ref("");
const consent = ref(false), revokeConsent = ref(false);
let generation = 0, disposed = false;
const storageKey = (scan: string) => `oviraptor.protection-recovery.priced.v2:${scan}`;
const current = (id: number, scan: string) => !disposed && id === generation && props.scanId === scan;
const locked = computed(() => busy.value || blocked.value || !!operation.value || !!props.disabled);
function pricing(): RecoveryPricingInput {
  const rate = (value: string) => { const amount = decimalUsdNanos(value); if (amount === undefined) throw Error("rate_invalid"); return amount.toString(); };
  const value: RecoveryPricingInput = { schemaVersion: 1, maxBudgetUsd: usd.value, priceRevision: revision.value,
    uncachedInput: rate(inputRate.value), cachedInput: rate(cacheRate.value), output: rate(outputRate.value), perRequest: rate(requestRate.value) };
  if (!validRecoveryPricing(value)) throw Error("pricing_invalid");
  return value;
}
function candidate() {
  if (operation.value) return operation.value;
  if (!props.selection || !props.newScope || props.newScope.urls.length !== 1 || props.selection.scanId !== props.scanId) throw Error("scope_missing");
  const value = pricing();
  return { selection: JSON.parse(JSON.stringify(props.selection)) as RecoverySelection,
    newScope: { ...JSON.parse(JSON.stringify(props.newScope)) as RecoveryScope,
      maxBudgetUsd: Number(decimalUsdNanos(value.maxBudgetUsd)) / 1e9 }, pricing: value };
}
function usdRate(value: string): string {
  const amount = BigInt(value), whole = amount / 1000000000n;
  const fraction = (amount % 1000000000n).toString().padStart(9, "0").replace(/0+$/, "");
  return fraction ? `${whole}.${fraction}` : whole.toString();
}
function showOperation(op: PricedRecoveryOperation) {
  operation.value = op;
  usd.value = op.pricing.maxBudgetUsd; revision.value = op.pricing.priceRevision;
  inputRate.value = usdRate(op.pricing.uncachedInput); cacheRate.value = usdRate(op.pricing.cachedInput);
  outputRate.value = usdRate(op.pricing.output); requestRate.value = usdRate(op.pricing.perRequest);
}
async function recoverOriginal() {
  if (busy.value || !props.selection || props.selection.scanId !== props.scanId) return;
  const id = ++generation, scan = props.scanId;
  const selected = JSON.parse(JSON.stringify(props.selection)) as RecoverySelection;
  busy.value = true; error.value = "";
  try {
    const value = await api.pricedApprovalForSource(selected);
    if (!current(id, scan)) return;
    if (value === null) throw Error("original_approval_missing");
    const fact = value as RecoveryFact, original = fact.proposal as RecoveryFact;
    const op = { version: 2, requestId: fact.requestId, proposalHash: fact.proposalHash,
      selection: original?.selection, newScope: original?.newScope, pricing: original?.pricingInput };
    if (!validPricedRecoveryOperation(op, scan) || !pricedApprovalMatches(value, op)
      || canonical(op.selection) !== canonical(selected)) throw Error("original_approval_mismatch");
    const cancelled = await api.pricedCancellation(op.requestId);
    if (!current(id, scan)) return;
    if (canonical(props.selection) !== canonical(selected)
      || cancelled !== null && !pricedCancellationMatches(cancelled, op, value)) throw Error("original_source_changed");
    const saved = localStorage.getItem(storageKey(scan));
    if (saved !== null && canonical(JSON.parse(saved)) !== canonical(op)
      || operation.value && canonical(operation.value) !== canonical(op)) throw Error("saved_operation_changed");
    // Restore only the verified original UUID. Never replace another record.
    localStorage.setItem(storageKey(scan), JSON.stringify(op));
    showOperation(op); approval.value = fact;
    cancellation.value = cancelled === null ? undefined : cancelled as RecoveryFact;
    proposal.value = undefined; consent.value = false; revokeConsent.value = false; blocked.value = false;
  } catch { if (current(id, scan)) error.value = "无法找回该原来源的签名批准。已有操作记录保持不变。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
async function load() {
  if (busy.value) return;
  const id = ++generation, scan = props.scanId;
  busy.value = true; error.value = ""; blocked.value = false;
  try {
    const saved = localStorage.getItem(storageKey(scan));
    if (!saved) {
      if (operation.value) throw Error("saved_operation_missing");
      proposal.value = undefined; consent.value = false; revokeConsent.value = false;
      approval.value = undefined; cancellation.value = undefined; return;
    }
    const op: unknown = JSON.parse(saved);
    if (!validPricedRecoveryOperation(op, scan)) throw Error("saved_priced_operation_invalid");
    if (operation.value && canonical(operation.value) !== canonical(op)) throw Error("saved_operation_changed");
    showOperation(op);
    const [a, c] = await Promise.all([api.pricedApproval(op.requestId), api.pricedCancellation(op.requestId)]);
    if (!current(id, scan)) return;
    if (a !== null && !pricedApprovalMatches(a, op)) throw Error("approval_mismatch");
    if (c !== null && !pricedCancellationMatches(c, op, a)) throw Error("cancellation_mismatch");
    approval.value = a === null ? undefined : a as RecoveryFact;
    cancellation.value = c === null ? undefined : c as RecoveryFact;
    proposal.value = undefined; consent.value = false; revokeConsent.value = false;
  } catch { if (current(id, scan)) { blocked.value = true; error.value = "无法核验原定价操作。保留原 requestId，请找回状态。"; } }
  finally { if (current(id, scan)) busy.value = false; }
}
async function review() {
  if (busy.value || blocked.value || props.disabled || approval.value || cancellation.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = ""; proposal.value = undefined; consent.value = false;
  try {
    const op = candidate();
    const value = await api.pricedPreview(op.selection, op.newScope, op.pricing);
    if (!current(id, scan)) return;
    if (!pricedProposalMatches(value, op.selection, op.newScope, op.pricing)
      || operation.value && value.proposalHash !== operation.value.proposalHash) throw Error("proposal_mismatch");
    // The live parent fields/rates may change while a preview is in flight.
    const now = candidate();
    if (canonical(now.selection) !== canonical(op.selection) || canonical(now.newScope) !== canonical(op.newScope)
      || canonical(now.pricing) !== canonical(op.pricing)) throw Error("scope_changed");
    proposal.value = value;
  } catch (cause) {
    if (current(id, scan)) error.value = String(cause).includes("protection_recovery_fixed_call_exceeds_budget")
      ? "单次固定请求费用超过全部费用预算，请核对金额和费率。本次核验未执行；已有操作请先找回状态。"
      : "定价提案核验未通过。检查单个 URL、精确金额、费率版本与原范围。";
  }
  finally { if (current(id, scan)) busy.value = false; }
}
async function approve() {
  if (busy.value || blocked.value || props.disabled || !consent.value || !proposal.value || approval.value || cancellation.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = "";
  try {
    // Another window may have saved an operation after this view loaded.
    // Never overwrite its recovery UUID, or recreate a missing original record.
    const saved = localStorage.getItem(storageKey(scan));
    if (operation.value) {
      if (!saved || canonical(JSON.parse(saved)) !== canonical(operation.value)) {
        blocked.value = true;
        throw Error("saved_operation_changed");
      }
    } else if (saved !== null) {
      blocked.value = true;
      throw Error("saved_operation_requires_reload");
    }
    const values = candidate();
    const op: PricedRecoveryOperation = operation.value || { version: 2, requestId: crypto.randomUUID(),
      ...values, proposalHash: String(proposal.value.proposalHash) };
    if (!validPricedRecoveryOperation(op, scan) || !pricedProposalMatches(proposal.value, op.selection, op.newScope, op.pricing)
      || proposal.value.proposalHash !== op.proposalHash) throw Error("scope_changed");
    localStorage.setItem(storageKey(scan), JSON.stringify(op)); operation.value = op;
    const value = await api.pricedApprove(op);
    if (!current(id, scan)) return;
    if (!pricedApprovalMatches(value, op)) throw Error("approval_mismatch");
    approval.value = value; consent.value = false;
  } catch { if (current(id, scan)) error.value = "批准结果未确认。找回原状态或沿用原 requestId 重试，未创建替代操作。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
async function revoke() {
  const op = operation.value, approved = approval.value;
  if (busy.value || blocked.value || !revokeConsent.value || !op || !approved || cancellation.value) return;
  const id = generation, scan = props.scanId;
  busy.value = true; error.value = "";
  try {
    const value = await api.pricedCancel(op.requestId, String(approved.approvalId));
    if (!current(id, scan)) return;
    if (!pricedCancellationMatches(value, op, approved)) throw Error("cancellation_mismatch");
    cancellation.value = value; revokeConsent.value = false;
  } catch { if (current(id, scan)) error.value = "撤销结果未确认，请找回原签名状态。原批准与历史保留。"; }
  finally { if (current(id, scan)) busy.value = false; }
}
watch([usd, revision, inputRate, cacheRate, outputRate, requestRate, () => props.selection, () => props.newScope],
  () => { if (!operation.value) { proposal.value = undefined; consent.value = false; } }, { deep: true, flush: "sync" });
watch(() => props.scanId, () => {
  generation++; busy.value = false; blocked.value = false; error.value = "";
  operation.value = undefined; proposal.value = undefined; approval.value = undefined; cancellation.value = undefined;
  consent.value = false; revokeConsent.value = false;
  usd.value = ""; revision.value = ""; inputRate.value = ""; cacheRate.value = ""; outputRate.value = ""; requestRate.value = "";
  void load();
}, { immediate: true });
onUnmounted(() => { disposed = true; generation++; });
</script>
<template>
  <section class="priced-recovery">
    <p>为上方单个 URL 与明确的新范围确认模型费率。保存批准后等待专属出生；当前不会启动扫描。</p>
    <button type="button" :disabled="busy" @click="load">找回定价批准与撤销状态</button>
    <button type="button" :disabled="busy || !selection" @click="recoverOriginal">按原保护来源找回签名批准（缓存丢失时）</button>
    <p v-if="error" role="alert">{{ error }}</p>
    <fieldset :disabled="locked">
      <legend>本次精确定价合同</legend>
      <label>本次费用上限 USD<input v-model="usd" inputmode="decimal" placeholder="例如 5.000000001" /></label>
      <label>费率版本<input v-model="revision" placeholder="已核对的费率版本" /></label>
      <label>非缓存输入 USD / 百万 token<input v-model="inputRate" inputmode="decimal" /></label>
      <label>缓存输入 USD / 百万 token<input v-model="cacheRate" inputmode="decimal" /></label>
      <label>输出 USD / 百万 token<input v-model="outputRate" inputmode="decimal" /></label>
      <label>每次模型请求固定费用 USD<input v-model="requestRate" inputmode="decimal" /></label>
      <p>逐原调用合计向上取整至 nano-USD；缓存输入属于输入子集。费用按确认费率计算，供应商实际美元账单另行核对。</p>
    </fieldset>
    <button v-if="!approval && !cancellation" type="button" :disabled="busy || blocked || disabled" @click="review">核验完整定价提案</button>
    <details v-if="proposal || operation" open><summary>本次完整范围与精确定价</summary>
      <pre>{{ canonical(operation || proposal) }}</pre>
    </details>
    <template v-if="proposal && !approval && !cancellation">
      <label><input v-model="consent" type="checkbox" :disabled="busy || disabled" />我明确批准所显示的新范围、精确费用上限及费率；保留原停机和全部原费用。</label>
      <button type="button" :disabled="busy || blocked || disabled || !consent" @click="approve">{{ operation ? '沿用原 requestId 重试定价批准' : '明确批准范围与定价' }}</button>
    </template>
    <p v-if="operation">定价操作 requestId：{{ operation.requestId }}</p>
    <p v-if="approval">已核验原签名定价批准，目标请求授权为 0，等待专属出生。</p>
    <template v-if="approval && !cancellation">
      <label><input v-model="revokeConsent" type="checkbox" :disabled="busy" />撤销本次尚未出生的定价批准，并保留原批准与全部历史。</label>
      <button type="button" :disabled="busy || blocked || !revokeConsent" @click="revoke">明确撤销定价批准</button>
    </template>
    <p v-if="cancellation">原签名撤销已核验：{{ cancellation.cancellationId }}。此批准不能重试恢复授权。</p>
  </section>
</template>
<style scoped>
.priced-recovery { margin-top: 1rem; padding: .8rem; border: 1px solid var(--border-color, #777); }
fieldset { display: grid; gap: .6rem; margin: .8rem 0; }
label { display: grid; gap: .2rem; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
