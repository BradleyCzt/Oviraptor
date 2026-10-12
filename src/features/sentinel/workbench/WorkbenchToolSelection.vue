<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "../../../i18n";
import { toolApprovalApi, type ToolApprovalInventory } from "../../environment/toolApprovalApi";
import { canonicalApprovedToolIds } from "./useWorkbenchTaskCreation";
const props = defineProps<{ modelValue: string[]; disabled: boolean; eligible: boolean }>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();
const { tr } = useI18n();
const text = ref(props.modelValue.join("\n"));
const busy = ref(false);
const previews = ref<ToolApprovalInventory[]>([]);
const readError = ref("");
let generation = 0;
watch(() => props.modelValue, ids => {
  if (JSON.stringify(ids) !== JSON.stringify(tokens(text.value))) text.value = ids.join("\n");
  generation++; previews.value = []; readError.value = "";
});
function tokens(value: string): string[] { return value.split(/[\s,，;；]+/).filter(Boolean); }
function edit(event: Event) {
  text.value = (event.target as HTMLTextAreaElement).value;
  generation++; previews.value = []; readError.value = "";
  emit("update:modelValue", tokens(text.value));
}
const validation = computed(() => {
  try { canonicalApprovedToolIds(tokens(text.value)); return ""; }
  catch (error) { return String(error); }
});
async function read() {
  if (busy.value || props.disabled || validation.value) return;
  const ids = canonicalApprovedToolIds(tokens(text.value));
  const current = generation;
  busy.value = true; previews.value = []; readError.value = "";
  try {
    const records: ToolApprovalInventory[] = [];
    for (const id of ids) {
      const inventory = await toolApprovalApi.read(id);
      if (!inventory) throw new Error(tr("未找到原批准记录", "Original approval record not found"));
      records.push(inventory);
    }
    if (generation === current) previews.value = records;
  } catch (error) { if (generation === current) readError.value = String(error); }
  finally { busy.value = false; }
}
</script>
<template>
  <section class="field span-two tool-selection">
    <label><span>{{ tr('工具准备 · 可选批准收据 ID', 'Tool preparation · optional approval receipt IDs') }}</span>
      <textarea :value="text" :disabled="disabled || busy" rows="3" maxlength="1024" autocomplete="off" spellcheck="false" @input="edit" :placeholder="tr('从设置页的工具批准库存复制 ID，每行一个；最多 16 个', 'Copy approval IDs from Settings, one per line; at most 16')" />
    </label>
    <small>{{ tr('只为新的多智能体 Web 任务准备工具选择，当前不会执行工具。库存读取仅用于预览，创建时由后端验证批准、撤销和原任务绑定。', 'Prepares tool selection for a new Multi Web task; tools will not execute. Inventory is preview only; creation verifies approval, revocation and task binding.') }}</small>
    <small v-if="!eligible" role="alert">{{ tr('当前任务类型不支持工具选择。请主动清空所有 ID，再创建此任务；选择不会自动丢弃。', 'This task type does not support tools. Clear all IDs explicitly; selection is never discarded automatically.') }}</small>
    <small v-if="validation" role="alert">{{ validation }}</small>
    <button class="button ghost compact" type="button" :disabled="disabled || busy || !!validation || !modelValue.length" @click="read">{{ busy ? tr('读取中', 'Reading') : tr('读取批准库存预览', 'Preview approval inventory') }}</button>
    <small v-if="readError" role="alert">{{ readError }}</small>
    <ul v-if="previews.length"><li v-for="item in previews" :key="item.approvalId">
      {{ item.toolId }} / {{ item.version }} / {{ item.platform }} · {{ item.approvalId }} · {{ item.status }}
      <span v-if="item.preparationFencePresent"> · {{ tr('准备门禁尚未解除', 'Preparation fence remains') }}</span>
      · {{ tr('不可执行', 'Execution disabled') }}
    </li></ul>
  </section>
</template>
<style scoped>
.tool-selection { display: grid; gap: 8px; }
.tool-selection label { display: grid; gap: 6px; }
.tool-selection textarea { width: 100%; box-sizing: border-box; }
.tool-selection small { color: var(--text-secondary); overflow-wrap: anywhere; }
.tool-selection [role="alert"] { color: var(--danger, #c0392b); }
.tool-selection ul { margin: 0; padding-left: 20px; overflow-wrap: anywhere; }
</style>
