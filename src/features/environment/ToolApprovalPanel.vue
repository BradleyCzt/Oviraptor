<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "../../i18n";
import { toolApprovalApi, readToolApprovalFile, TOOL_APPROVAL_JSON_LIMIT, TOOL_APPROVAL_PACKAGE_LIMIT,
  type ToolApprovalInventory, type ToolRevocationInventory } from "./toolApprovalApi";

const { tr } = useI18n();
const decisionFile = ref<File | null>(null);
const manifestFile = ref<File | null>(null);
const packageFile = ref<File | null>(null);
const revokeFile = ref<File | null>(null);
const preview = ref<Record<string, string> | null>(null);
const approvalId = ref("");
const inventory = ref<ToolApprovalInventory | null>(null);
const revocation = ref<ToolRevocationInventory | null>(null);
const busy = ref("");
const error = ref("");
const message = ref("");
const ready = computed(() => !!decisionFile.value && !!manifestFile.value && !!packageFile.value && !busy.value);
const preparationFencePresent = computed(() => inventory.value?.preparationFencePresent === true || revocation.value?.preparationFencePresent === true);
const errorContext = computed(() => {
  if (error.value.includes("tool_administrator_trust_unconfigured")) return tr("管理员信任配置尚未部署，请由任务外管理员完成配置后再提交。", "Administrator trust is not configured. An administrator must deploy it outside a task.");
  if (error.value.startsWith("tool_administrator_receipt_saved_preparation_incomplete:")) return tr("批准收据已保存，但环境准备阶段未完成。请保留此错误，由管理员核对准备状态；库存不具备执行权限。", "The approval receipt was saved, but preparation did not complete. Keep this error for administrator review; inventory cannot execute.");
  if (error.value.startsWith("tool_administrator_revocation_saved_preparation_incomplete:")) return tr("撤销收据已保存，但环境准备阶段未完成。请由管理员核对准备状态。", "The revocation receipt was saved, but preparation did not complete. Administrator review is required.");
  return "";
});
const object = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
function previewManifest(bytes: Uint8Array): Record<string, string> {
  const raw: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  if (!object(raw) || !object(raw.payload)) throw new Error("工具清单需要包含 payload 对象 / Manifest must contain a payload object");
  const selected: Record<string, string> = {};
  for (const key of ["id", "version", "platform", "digest", "entrypoint", "networkPolicy", "filesystemPolicy"]) {
    const value = raw.payload[key];
    if (typeof value === "string") selected[key] = value.slice(0, 512);
  }
  return selected;
}
async function choose(event: Event, kind: "decision" | "manifest" | "package" | "revoke") {
  const input = event.target as HTMLInputElement;
  if (busy.value) { input.value = ""; return; }
  const file = input.files?.[0] ?? null;
  if (kind === "manifest") { manifestFile.value = null; preview.value = null; }
  else if (kind === "decision") decisionFile.value = null;
  else if (kind === "package") packageFile.value = null;
  else revokeFile.value = null;
  error.value = ""; message.value = "";
  if (!file) return;
  busy.value = "select";
  try {
    const limit = kind === "package" ? TOOL_APPROVAL_PACKAGE_LIMIT : TOOL_APPROVAL_JSON_LIMIT;
    if (file.size === 0 || file.size > limit) throw new Error(`${file.name}: ${tr("文件为空或超过大小上限", "File is empty or exceeds the size limit")}`);
    if (kind === "manifest") { preview.value = previewManifest(await readToolApprovalFile(file, limit)); manifestFile.value = file; }
    else if (kind === "decision") decisionFile.value = file;
    else if (kind === "package") packageFile.value = file;
    else revokeFile.value = file;
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = ""; input.value = ""; }
}
async function approve() {
  if (!ready.value || !decisionFile.value || !manifestFile.value || !packageFile.value) return;
  busy.value = "approve"; error.value = ""; message.value = ""; inventory.value = null; revocation.value = null;
  try {
    const decision = await readToolApprovalFile(decisionFile.value, TOOL_APPROVAL_JSON_LIMIT);
    const manifest = await readToolApprovalFile(manifestFile.value, TOOL_APPROVAL_JSON_LIMIT);
    const bytes = await readToolApprovalFile(packageFile.value, TOOL_APPROVAL_PACKAGE_LIMIT);
    inventory.value = await toolApprovalApi.approve(decision, manifest, bytes);
    approvalId.value = inventory.value.approvalId;
    message.value = tr("签名与摘要经后端验证，批准库存已保存；仍不可执行。", "Backend signature and digest verification succeeded. Approval inventory was saved; execution remains disabled.");
  } catch (reason) {
    error.value = String(reason);
    const saved = error.value.match(/^tool_administrator_receipt_saved_preparation_incomplete:([0-9a-f-]{36}):/i);
    if (saved?.[1]) approvalId.value = saved[1];
  } finally { busy.value = ""; }
}
async function revoke() {
  if (busy.value || !revokeFile.value) return;
  busy.value = "revoke"; error.value = ""; message.value = ""; inventory.value = null; revocation.value = null;
  try {
    revocation.value = await toolApprovalApi.revoke(await readToolApprovalFile(revokeFile.value, TOOL_APPROVAL_JSON_LIMIT));
    inventory.value = null;
    message.value = tr("原签名撤销收据已保存；未授予任何执行权限。", "The signed revocation receipt was saved; no execution permission was granted.");
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = ""; }
}
async function readInventory() {
  if (busy.value || !approvalId.value.trim()) return;
  busy.value = "read"; error.value = ""; message.value = ""; inventory.value = null; revocation.value = null;
  try {
    inventory.value = await toolApprovalApi.read(approvalId.value.trim());
    if (!inventory.value) message.value = tr("没有找到该批准收据。", "No approval receipt was found.");
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = ""; }
}
</script>

<template>
  <section class="tool-approval-panel form-stack" aria-labelledby="tool-approval-title">
    <div><h3 id="tool-approval-title">{{ tr('离线工具批准库存', 'Offline tool approval inventory') }}</h3>
      <p class="helper">{{ tr('导入任务外管理员已签名的决定、发布者清单和固定工具包。库存批准不代表已解包、已冻结到任务或可执行；当前执行权限为 false。', 'Import a decision already signed by an administrator, a publisher manifest, and a fixed package. Inventory approval does not mean extraction, attempt freezing, or execution; executionAllowed is false.') }}</p></div>
    <fieldset :disabled="!!busy" class="approval-files">
      <legend>{{ tr('保存签名批准决定', 'Save a signed approval decision') }}</legend>
      <label class="field"><span>{{ tr('管理员签名批准决定 JSON · 最大 16 KiB', 'Administrator-signed approval JSON · max 16 KiB') }}</span><input type="file" accept=".json,application/json" @change="choose($event, 'decision')" /><small>{{ decisionFile?.name }}</small></label>
      <label class="field"><span>{{ tr('发布者签名清单 JSON · 最大 16 KiB', 'Publisher-signed manifest JSON · max 16 KiB') }}</span><input type="file" accept=".json,application/json" @change="choose($event, 'manifest')" /><small>{{ manifestFile?.name }}</small></label>
      <label class="field"><span>{{ tr('固定工具包 · 最大 64 MiB', 'Fixed tool package · max 64 MiB') }}</span><input type="file" @change="choose($event, 'package')" /><small>{{ packageFile ? `${packageFile.name} · ${packageFile.size} bytes` : '' }}</small></label>
    </fieldset>
    <div v-if="preview" class="approval-preview"><strong>{{ tr('未验证的清单预览', 'Unverified manifest preview') }}</strong><p class="helper">{{ tr('以下为文件自报内容；签名、平台和工具包摘要由后端验证。', 'These are file-declared values. The backend verifies signatures, platform, and package digest.') }}</p><dl><template v-for="(value, key) in preview" :key="key"><dt>{{ key }}</dt><dd>{{ value }}</dd></template></dl></div>
    <div class="approval-actions"><button class="button secondary compact" type="button" :disabled="!ready" @click="approve">{{ busy === 'approve' ? tr('验证并保存中…', 'Verifying and saving…') : tr('验证并保存批准库存', 'Verify and save approval inventory') }}</button></div>
    <fieldset :disabled="!!busy" class="approval-files"><legend>{{ tr('签名撤销决定', 'Signed revocation decision') }}</legend><label class="field"><span>{{ tr('管理员签名撤销 JSON · 最大 16 KiB', 'Administrator-signed revocation JSON · max 16 KiB') }}</span><input type="file" accept=".json,application/json" @change="choose($event, 'revoke')" /><small>{{ revokeFile?.name }}</small></label><button class="button ghost compact" type="button" :disabled="!revokeFile || !!busy" @click="revoke">{{ busy === 'revoke' ? tr('撤销中…', 'Revoking…') : tr('验证并保存撤销', 'Verify and save revocation') }}</button></fieldset>
    <div class="approval-query"><label class="field"><span>{{ tr('批准收据 ID', 'Approval receipt ID') }}</span><input v-model="approvalId" :disabled="!!busy" maxlength="36" autocomplete="off" spellcheck="false" /></label><button class="button ghost compact" type="button" :disabled="!!busy || !approvalId.trim()" @click="readInventory">{{ tr('读取原库存', 'Read original inventory') }}</button></div>
    <div v-if="inventory" class="approval-preview"><strong>{{ tr('后端验证的原批准库存 · 不可执行', 'Backend-verified original approval inventory · execution disabled') }}</strong><dl><dt>ID</dt><dd>{{ inventory.approvalId }}</dd><dt>{{ tr('工具 / 版本 / 平台', 'Tool / version / platform') }}</dt><dd>{{ inventory.toolId }} / {{ inventory.version }} / {{ inventory.platform }}</dd><dt>{{ tr('工具包摘要', 'Package digest') }}</dt><dd>{{ inventory.packageDigest }}</dd><dt>{{ tr('运行时适配器 / 摘要', 'Runtime adapter / digest') }}</dt><dd>{{ inventory.runtimeAdapter }} / {{ inventory.runtimeDigest }}</dd><dt>{{ tr('批准原文哈希', 'Approval hash') }}</dt><dd>{{ inventory.approvalHash }}</dd><dt>status</dt><dd>{{ inventory.status }}</dd><dt>executionAllowed</dt><dd>{{ inventory.executionAllowed }}</dd></dl></div>
    <p v-if="revocation" class="helper">{{ tr('原撤销收据', 'Original revocation receipt') }}: {{ revocation.revocationId }} · {{ revocation.status }} · executionAllowed={{ revocation.executionAllowed }}</p>
    <p v-if="preparationFencePresent" class="form-error" role="status">{{ tr('批准/撤销记录已保存；环境准备门禁尚未解除，请先检查准备状态。', 'Approval/revocation records are saved; the environment preparation fence remains. Check preparation status first.') }}</p>
    <p v-if="message" class="helper" role="status">{{ message }}</p>
    <div v-if="error" role="alert"><p v-if="errorContext" class="form-error">{{ errorContext }}</p><p class="form-error approval-error">{{ error }}</p></div>
  </section>
</template>

<style scoped>
.tool-approval-panel { border-top: 1px solid var(--border-color, #ddd); padding-top: 1rem; }
h3 { margin: 0 0 .5rem; font-size: 1rem; }
.approval-files { border: 1px solid var(--border-color, #ddd); border-radius: 8px; padding: .8rem; display: grid; gap: .7rem; min-width: 0; }
.approval-files legend { padding: 0 .3rem; font-size: .85rem; }
.approval-preview { padding: .8rem; border: 1px solid var(--border-color, #ddd); border-radius: 8px; }
dl { display: grid; grid-template-columns: minmax(5rem, 1fr) minmax(0, 3fr); gap: .4rem .7rem; margin-bottom: 0; font-size: .8rem; }
dt { opacity: .7; } dd { margin: 0; overflow-wrap: anywhere; }
.approval-query { display: flex; gap: .6rem; align-items: end; flex-wrap: wrap; }
.approval-query .field { flex: 1; min-width: 12rem; }
.approval-error, small { overflow-wrap: anywhere; }
</style>
