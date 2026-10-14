<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { api } from "../../../api";
import type { ScanIdentityMaterial } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ projectId?: number; scanId: string }>();
const { tr } = useI18n();
const materials = ref<ScanIdentityMaterial[]>([]);
const loading = ref(false);
const error = ref("");
let generation = 0;
let disposed = false;

async function load() {
  const current = ++generation;
  const { projectId, scanId } = props;
  materials.value = [];
  error.value = "";
  if (!projectId || !scanId) { loading.value = false; return; }
  loading.value = true;
  try {
    const result = await api.getSentinelScanIdentityMaterial(projectId, scanId);
    if (!disposed && current === generation) materials.value = result;
  } catch (cause) {
    if (!disposed && current === generation) error.value = String(cause);
  } finally {
    if (!disposed && current === generation) loading.value = false;
  }
}
watch(() => [props.projectId, props.scanId], load, { immediate: true });
onUnmounted(() => { disposed = true; ++generation; materials.value = []; });
</script>

<template>
  <details v-if="scanId" class="identity-material">
    <summary>{{ tr('本任务登录身份 · 原始材料', 'Task identities · Original material') }}</summary>
    <button type="button" class="button ghost compact" :disabled="loading" @click="load">{{ tr('刷新身份材料', 'Refresh identity material') }}</button>
    <p v-if="loading" role="status">{{ tr('读取任务身份…', 'Loading task identities…') }}</p>
    <p v-else-if="error" role="alert">{{ error }}</p>
    <p v-else-if="!materials.length">{{ tr('本任务未绑定登录身份。', 'No login identities are bound to this task.') }}</p>
    <section v-for="item in materials" :key="item.session.id">
      <strong>{{ item.session.name }} · {{ item.session.status }}</strong>
      <small>{{ item.session.id }} · {{ item.session.finalUrl || item.session.entryUrl }}</small>
      <pre>{{ JSON.stringify(item.document, null, 2) }}</pre>
    </section>
  </details>
</template>

<style scoped>
.identity-material { margin: 12px 0; padding: 12px; border: 1px solid var(--border); border-radius: 8px; }
summary { cursor: pointer; margin-bottom: 8px; }
small { display: block; overflow-wrap: anywhere; }
pre { max-height: 280px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
</style>
