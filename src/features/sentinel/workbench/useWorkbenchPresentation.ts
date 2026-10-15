import { computed, ref, watch, type Ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { AgentSkill } from "../../skills/catalogTypes";
import type { WorkbenchMode } from "./useWorkbenchTaskCreation";

type PresentationForm = {
  scanMode: "quick" | "standard" | "deep";
  maxBudgetUsd: number | undefined;
  skillIds: number[];
  sourcePath: string;
};

export function useWorkbenchPresentation(options: {
  mode: Ref<WorkbenchMode>;
  form: PresentationForm;
  skills: Ref<AgentSkill[]>;
  tr: (zh: string, en: string) => string;
}) {
  const { mode, form, skills, tr } = options;
  const webPreset = ref<"bounded" | "balanced" | "evidence">("balanced");
  watch(() => form.scanMode, (value) => {
    if (mode.value === "web")
      webPreset.value = value === "quick" ? "bounded" : value === "deep" ? "evidence" : "balanced";
  });
  function applyWebPreset(value: "bounded" | "balanced" | "evidence") {
    webPreset.value = value;
    form.scanMode = value === "bounded" ? "quick" : value === "balanced" ? "standard" : "deep";
    form.maxBudgetUsd = value === "bounded" ? 1 : value === "balanced" ? 5 : 15;
  }
  const selectedSkills = computed(() => skills.value.filter((skill) => form.skillIds.includes(skill.id)));
  const webPolicySummary = computed(() => form.scanMode === "quick"
    ? { contracts: 4, discovery: 1, verifiers: 1, chain: false }
    : form.scanMode === "deep"
      ? { contracts: 24, discovery: 3, verifiers: 3, chain: true }
      : { contracts: 12, discovery: 2, verifiers: 2, chain: false });
  const effectiveWebSkillNames = computed(() => [
    tr("业务前端深度分析（系统默认）", "Business frontend deep analysis (system default)"),
    ...selectedSkills.value.filter((skill) => !skill.builtin).map((skill) => skill.name),
  ]);
  function modeLabel(value: string) {
    return ({ web: tr("Nest Web 扫描", "Nest Web scan"), code: tr("代码审计", "Code audit"),
      greybox: tr("灰盒联测", "Grey-box"), cicd: "CI/CD" } as Record<string, string>)[value] || value;
  }
  async function chooseSource() {
    const value = await open({ directory: true, multiple: false,
      title: tr("选择源码目录（只读挂载）", "Select source directory (read-only mount)") });
    if (value) form.sourcePath = value;
  }
  return { webPreset, applyWebPreset, selectedSkills, webPolicySummary,
    effectiveWebSkillNames, modeLabel, chooseSource };
}
