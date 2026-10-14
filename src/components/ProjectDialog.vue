<script setup lang="ts">
import { reactive, ref } from "vue";
import { FolderPlus } from "@lucide/vue";
import ModalShell from "./ModalShell.vue";
import { api } from "../api";
import type { Project } from "../features/workspaces/types";
import { useI18n } from "../i18n";

const props = defineProps<{ project?: Project }>();
const emit = defineEmits<{ close: []; saved: [id: number] }>();
const { tr } = useI18n();
const form = reactive({ name: props.project?.name ?? "", description: props.project?.description ?? "" });
const busy = ref(false);
const error = ref("");

async function save() {
  if (!form.name.trim()) { error.value = tr("请输入工作空间名称", "Enter a workspace name"); return; }
  busy.value = true; error.value = "";
  try {
    const id = await api.saveProject({ id: props.project?.id, name: form.name, description: form.description });
    emit("saved", id);
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
</script>

<template>
  <ModalShell :title="project ? tr('编辑工作空间','Edit workspace') : tr('创建工作空间','Create workspace')" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow"><FolderPlus :size="14" /> SHARED PROJECT</span></template>
    <div class="form-stack">
      <div class="shared-project-note"><strong>{{tr('Asset 与 Agent 共用','Shared by Asset and Agent')}}</strong><p>{{tr('资产范围、扫描任务、证据、漏洞结论和知识沉淀都会绑定到这个工作空间。','Asset scope, scans, evidence, conclusions, and learned knowledge all belong to this workspace.')}}</p></div>
      <label class="field"><span>{{tr('工作空间名称','Workspace name')}}</span><input v-model="form.name" autofocus :placeholder="tr('例如：某企业 SRC 或专项代码审计','e.g. Company SRC or focused code audit')" /></label>
      <label class="field"><span>{{tr('范围与说明','Scope and description')}}</span><textarea v-model="form.description" rows="4" :placeholder="tr('记录测试范围、用途、授权边界或目标来源','Record scope, purpose, authorization boundary, or target source')"></textarea></label>
      <p v-if="error" class="form-error">{{ error }}</p>
    </div>
    <template #footer>
      <button class="button ghost" @click="$emit('close')">{{tr('取消','Cancel')}}</button>
      <button class="button primary" :disabled="busy" @click="save">{{ busy ? tr('保存中…','Saving…') : tr('保存工作空间','Save workspace') }}</button>
    </template>
  </ModalShell>
</template>
