<script setup lang="ts">
import { useI18n } from "../../i18n";
import ModalShell from "../../shared/components/ModalShell.vue";

defineProps<{ version: string }>();
defineEmits<{ close: [] }>();
const { tr } = useI18n();
</script>

<template>
  <ModalShell :title="tr(`Oviraptor v${version} 更新说明`, `Oviraptor v${version} release notes`)" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow">RELEASE NOTES · NATIVE</span></template>
    <div class="release-notes">
      <article>
        <strong>{{ tr("当前 Native 执行路径", "Current Native execution path") }}</strong>
        <p>{{ tr("当前任务使用 Native 执行路径。当前 Native JSON 与标准 SARIF 保留导入能力；旧后端专属格式不再导入。历史资料归档在项目文档中，原有数据清理需要单独确认。", "Current tasks use Native. Current Native JSON and standard SARIF remain importable; retired backend formats are rejected. Historical notes are archived in project documentation, and existing data cleanup requires separate confirmation.") }}</p>
      </article>
    </div>
  </ModalShell>
</template>
