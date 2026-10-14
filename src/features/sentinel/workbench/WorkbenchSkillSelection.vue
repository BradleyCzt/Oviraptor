<script setup lang="ts">
import { useI18n } from "../../../i18n";
import type { AgentSkill } from "../../skills/catalogTypes";
import type { WorkbenchScanInput } from "../types";
import type { WorkbenchMode } from "./useWorkbenchTaskCreation";

defineProps<{
  mode: Exclude<WorkbenchMode, "skills">;
  expanded: boolean;
  skills: AgentSkill[];
  form: Pick<WorkbenchScanInput, "skillIds" | "instruction">;
}>();
const { tr } = useI18n();
</script>

<template>
  <div v-if="mode !== 'web' || expanded" class="field span-two">
    <span>{{ tr(
      mode === 'web' ? "附加专项 Skills（可多选，默认流程始终启用）" : "本任务使用的 Skills（可多选）",
      mode === 'web' ? "Additional specialist skills (the default workflow is always active)" : "Skills for this task (multi-select)",
    ) }}</span>
    <div class="skill-choice-grid">
      <label v-for="skill in skills.filter((item) => item.enabled && (mode !== 'web' || !item.builtin))"
        :key="skill.id" :class="{ active: form.skillIds.includes(skill.id) }">
        <input v-model="form.skillIds" type="checkbox" :value="skill.id" />
        <span>{{ skill.name }}</span>
        <small>{{ skill.description || tr("无说明", "No description") }}</small>
        <em>{{ skill.builtin ? tr("内置", "Built-in") : tr("自定义", "Custom") }}</em>
      </label>
      <div v-if="!skills.some((item) => item.enabled && (mode !== 'web' || !item.builtin))" class="empty-inline">
        {{ tr(
          mode === 'web' ? "没有附加专项 Skill；系统默认流程仍会执行" : "没有启用的 Skill，可在左侧 Skills 中创建",
          mode === 'web' ? "No additional skill; the system workflow still runs" : "No enabled skills; create one from Skills",
        ) }}
      </div>
    </div>
  </div>
  <label v-if="mode !== 'web' || expanded" class="field span-two">
    <span>{{ tr("本次补充要求（可选）", "Extra instructions (optional)") }}</span>
    <textarea v-model="form.instruction" rows="3"
      :placeholder="tr('例如：重点检查鉴权和文件上传', 'For example: focus on auth and file uploads')"></textarea>
  </label>
</template>
