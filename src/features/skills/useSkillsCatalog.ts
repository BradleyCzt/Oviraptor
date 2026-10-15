import { computed, onMounted, reactive, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { instructionCatalogApi as api, knowledgeCatalogApi } from "./catalogApi";
import type { AgentSkill } from "./catalogTypes";

type Notice = (type: "success" | "error" | "info", text: string) => void;

export function useSkillsCatalog(options: {
  tr: (zh: string, en: string) => string;
  notify: Notice;
  clearSelection: () => void;
  context?: "library";
}) {
  const { tr, notify, clearSelection } = options;
  const skills = ref<AgentSkill[]>([]);
  const skillBusy = ref(false);
  const refiningSkillId = ref<number>();
  const deleteSkill = ref<AgentSkill>();
  const editingSkill = ref<AgentSkill>();
  const showSkillEditor = ref(false);
  const skillForm = reactive({ name: "", description: "", instructions: "", enabled: true });
  const skillPreview = computed(() => {
    const source = skillForm.instructions.trim();
    if (!source) return [] as Array<{ heading: string; body: string }>;
    const sections: Array<{ heading: string; body: string }> = [];
    let current = { heading: tr("未命名章节", "Untitled section"), body: "" };
    for (const line of source.split(/\r?\n/)) {
      const heading = line.match(/^#{1,3}\s+(.+)$/);
      if (heading) {
        if (current.body.trim()) sections.push({ ...current, body: current.body.trim() });
        current = { heading: heading[1].trim(), body: "" };
      } else {
        current.body += `${line}\n`;
      }
    }
    if (current.body.trim()) sections.push({ ...current, body: current.body.trim() });
    return sections;
  });

  async function loadSkills() {
    try {
      skills.value = await api.listAgentSkills();
      clearSelection();
    } catch (error) {
      notify("error", String(error));
    }
  }
  async function refineSkill(skill: AgentSkill) {
    refiningSkillId.value = skill.id;
    try {
      const result = await knowledgeCatalogApi.refineAgentSkillWithKnowledge(skill.id);
      await loadSkills();
      notify("success", result === skill.id
        ? tr("已用最新高质量知识精炼 Skill", "Skill refined with the latest high-quality knowledge")
        : tr("内置 Skill 未被覆盖，已创建增强副本", "Built-in Skill kept intact; an enhanced copy was created"));
    } catch (error) {
      notify("error", String(error));
    } finally {
      refiningSkillId.value = undefined;
    }
  }
  async function exportSkills() {
    try {
      const path = await api.exportAgentSkills();
      notify("success", tr(`Skills 已导出：${path}`, `Skills exported: ${path}`));
    } catch (error) {
      notify("error", String(error));
    }
  }
  async function importSkills() {
    const path = await open({
      multiple: false, directory: false,
      filters: [{ name: "Oviraptor Skills", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      const count = await api.importAgentSkills(path);
      await loadSkills();
      notify("success", tr(`已导入 ${count} 个 Skill`, `Imported ${count} Skills`));
    } catch (error) {
      notify("error", String(error));
    }
  }
  async function importInternalSecSkills() {
    const path = await open({
      multiple: false, directory: true,
      title: tr("选择内部 sec_skills 目录", "Select internal sec_skills directory"),
    });
    if (!path) return;
    try {
      const result = await api.importSecSkillKnowledge(path);
      await loadSkills();
      notify("success", tr(
        `已完整导入 ${String(result.filesScanned || 0)} 个内部 Skill 文件`,
        `Imported ${String(result.filesScanned || 0)} internal Skill files`,
      ));
    } catch (error) {
      notify("error", String(error));
    }
  }
  function editSkill(skill?: AgentSkill) {
    editingSkill.value = skill;
    skillForm.name = skill?.name || "";
    skillForm.description = skill?.description || "";
    skillForm.instructions = skill?.instructions || "";
    skillForm.enabled = skill?.enabled ?? true;
    showSkillEditor.value = true;
  }
  function cloneBuiltinSkill(skill: AgentSkill) {
    editingSkill.value = undefined;
    skillForm.name = `${skill.name} · 自定义增强版`;
    skillForm.description = skill.description;
    skillForm.instructions = skill.instructions;
    skillForm.enabled = true;
    showSkillEditor.value = true;
  }
  function insertSkillTemplate() {
    skillForm.instructions = `## Objective\nDescribe the security outcome this skill should achieve.\n\n## Scope\n- Include: files, frameworks, routes, or vulnerability classes to inspect\n- Exclude: generated files, third-party bundles, destructive actions\n\n## Analysis workflow\n1. Establish evidence and affected target.\n2. Trace data flow or request flow.\n3. Verify impact safely and preserve native Agent proof logic.\n\n## Output requirements\n- Bind every finding to a URL or source location.\n- Include evidence, impact, CVSS/CWE, remediation, and confidence.\n- Do not report reconnaissance-only observations as vulnerabilities.`;
  }
  async function saveSkill() {
    skillBusy.value = true;
    try {
      await api.saveAgentSkill({
        id: editingSkill.value?.id, name: skillForm.name,
        description: skillForm.description, instructions: skillForm.instructions,
        enabled: skillForm.enabled,
      });
      showSkillEditor.value = false;
      await loadSkills();
      notify("success", tr("指令已保存，可在需要时选择使用", "Instructions saved and available to select"));
    } catch (error) {
      notify("error", String(error));
    } finally {
      skillBusy.value = false;
    }
  }
  async function removeSkill() {
    if (!deleteSkill.value) return;
    skillBusy.value = true;
    try {
      await api.deleteAgentSkill(deleteSkill.value.id);
      deleteSkill.value = undefined;
      await loadSkills();
      notify("success", tr("自定义技能已删除", "Custom skill deleted"));
    } catch (error) {
      notify("error", String(error));
    } finally {
      skillBusy.value = false;
    }
  }
  onMounted(() => { void loadSkills(); });
  return {
    skills, skillBusy, refiningSkillId, deleteSkill, editingSkill, showSkillEditor,
    skillForm, skillPreview, loadSkills, refineSkill, exportSkills, importSkills,
    importInternalSecSkills, editSkill, cloneBuiltinSkill, insertSkillTemplate,
    saveSkill, removeSkill,
  };
}
