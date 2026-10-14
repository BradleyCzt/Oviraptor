<script setup lang="ts">
import { computed } from "vue";
import { BookOpen, Braces, Download, Plus, RefreshCw, Save, Sparkles, Trash2, Upload, X } from "@lucide/vue";
import InlineConfirm from "../../../components/InlineConfirm.vue";
import { useI18n } from "../../../i18n";
import type { useWorkbenchSkills } from "./useWorkbenchSkills";

const props = defineProps<{ catalog: ReturnType<typeof useWorkbenchSkills>; standalone?: boolean; excludeSkillIds?: number[] }>();
const { tr } = useI18n();
const { skills, skillBusy, refiningSkillId, deleteSkill, editingSkill, showSkillEditor,
  skillForm, skillPreview, refineSkill, exportSkills, importSkills,
  importInternalSecSkills, editSkill, cloneBuiltinSkill, insertSkillTemplate,
  saveSkill, removeSkill } = props.catalog;
const visibleSkills = computed(() => skills.value.filter(skill => !props.excludeSkillIds?.includes(skill.id)));
</script>

<template>
  <div class="skills-toolbar">
    <div>
      <span class="eyebrow">{{standalone?'SAVED INSTRUCTIONS':'NEST SKILLS'}}</span>
      <h3>{{ standalone?tr("文本指令", "Text instructions"):tr("扫描技能", "Scan skills") }}</h3>
      <p>{{ standalone?tr("创建或导入现有文本指令。技能包在上方管理，不在此编辑。", "Create or import existing text instructions. Package skills are managed above and cannot be edited here."):tr("每个任务自行选择 Skill；代码审计不会再默认加载“业务前端深度分析”。", "Each task chooses its own skills; code audit no longer forces the frontend skill.") }}</p>
    </div>
    <div>
      <button class="button ghost" @click="importSkills"><Upload :size="15" />{{ standalone?tr("导入 JSON", "Import JSON"):tr("导入", "Import") }}</button>
      <button v-if="!standalone" class="button ghost" @click="importInternalSecSkills"><BookOpen :size="15" />{{ tr("导入内部 sec_skills", "Import internal sec_skills") }}</button>
      <button class="button ghost" @click="exportSkills"><Download :size="15" />{{ tr("导出", "Export") }}</button>
      <button class="button primary" @click="editSkill()"><Plus :size="15" />{{ tr("新增技能", "New skill") }}</button>
    </div>
  </div>
  <div class="skill-card-grid">
    <article v-for="skill in visibleSkills" :key="skill.id" class="panel skill-card-v2">
      <header>
        <span :class="{ builtin: skill.builtin }">{{ skill.builtin ? tr("内置", "Built-in") : tr("自定义", "Custom") }}</span>
        <em>{{ skill.enabled ? tr("已启用", "Enabled") : tr("已停用", "Disabled") }}</em>
      </header>
      <h3>{{ skill.name }}</h3>
      <p>{{ skill.description }}</p>
      <details><summary>{{ tr("查看指令", "View instructions") }}</summary><pre>{{ skill.instructions }}</pre></details>
      <footer>
        <button v-if="!standalone" class="button ghost compact" :disabled="refiningSkillId === skill.id" @click="refineSkill(skill)">
          <Sparkles v-if="refiningSkillId !== skill.id" :size="13" />
          <RefreshCw v-else :size="13" class="spinning" />
          {{ refiningSkillId === skill.id ? tr("精炼中", "Refining") : tr("用最新知识精炼", "Refine with latest") }}
        </button>
        <button v-if="skill.builtin" class="button ghost compact" @click="cloneBuiltinSkill(skill)">{{ tr("复制后编辑", "Clone & edit") }}</button>
        <button v-else class="button ghost compact" @click="editSkill(skill)">{{ tr("编辑", "Edit") }}</button>
        <button v-if="!skill.builtin" class="button danger compact" @click="deleteSkill = skill"><Trash2 :size="13" />{{ tr("删除", "Delete") }}</button>
      </footer>
      <InlineConfirm v-if="deleteSkill?.id === skill.id"
        :title="tr(`删除技能「${skill.name}」？`, `Delete skill “${skill.name}”?`)"
        :detail="tr('历史任务不受影响，新任务将不再加载该技能。', 'Existing tasks are unchanged; new tasks will no longer load it.')"
        :busy="skillBusy" @cancel="deleteSkill = undefined" @confirm="removeSkill" />
    </article>
  </div>
  <section v-if="showSkillEditor" class="panel skill-editor">
    <header>
      <div>
        <span class="eyebrow">SKILL EDITOR</span>
        <h3>{{ editingSkill ? tr("编辑自定义技能", "Edit custom skill") : tr("新增自定义技能", "New custom skill") }}</h3>
      </div>
      <div>
        <button class="button ghost compact" @click="insertSkillTemplate"><Braces :size="14" />{{ tr("插入规范模板", "Insert template") }}</button>
        <button class="icon-button" @click="showSkillEditor = false"><X :size="15" /></button>
      </div>
    </header>
    <div class="workbench-fields">
      <label class="field"><span>{{ tr("名称", "Name") }}</span><input v-model="skillForm.name" :placeholder="tr('例如：Java Spring 鉴权审计', 'e.g. Java Spring auth review')" /></label>
      <label class="field"><span>{{ tr("说明", "Description") }}</span><input v-model="skillForm.description" :placeholder="tr('一句话说明适用范围', 'One-line use case')" /></label>
      <div class="skill-format-guide span-two">
        <strong>{{ tr("推荐结构", "Recommended structure") }}</strong>
        <span>Objective → Scope → Analysis workflow → Output requirements</span>
        <p>{{ tr("写清检查目标、包含/排除范围、验证步骤和输出字段。不要在 Skill 中要求跳过授权边界、批量破坏或把普通信息当漏洞。", "Define objective, include/exclude scope, verification steps, and output fields. Do not request scope bypass, destructive bulk actions, or ordinary observations as vulnerabilities.") }}</p>
      </div>
      <label class="field span-two"><span>{{ standalone?tr("指令内容", "Instructions"):tr("传给 Nest 的指令", "Instructions sent to Nest") }}</span>
        <textarea v-model="skillForm.instructions" rows="16" :placeholder="tr('点击“插入规范模板”开始填写', 'Use Insert template to start')"></textarea>
      </label>
      <div class="skill-preview span-two">
        <header><strong>{{ tr("格式化预览", "Formatted preview") }}</strong><small>Markdown sections · {{ skillPreview.length }}</small></header>
        <article v-for="section in skillPreview" :key="section.heading"><h4>{{ section.heading }}</h4><pre>{{ section.body }}</pre></article>
        <p v-if="!skillPreview.length" class="empty-inline">{{ tr("输入指令后这里会显示章节化预览", "A sectioned preview appears as you type") }}</p>
      </div>
      <label class="check-inline"><input v-model="skillForm.enabled" type="checkbox" />{{ tr("启用", "Enabled") }}</label>
    </div>
    <footer>
      <button class="button ghost" @click="showSkillEditor = false">{{ tr("取消", "Cancel") }}</button>
      <button class="button primary" :disabled="skillBusy" @click="saveSkill"><Save :size="14" />{{ tr("保存技能", "Save skill") }}</button>
    </footer>
  </section>
</template>
