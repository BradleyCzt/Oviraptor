<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { Check, FolderOpen, Plus, Save, Zap } from "@lucide/vue";
import { useI18n } from "../i18n";
import type { GapFollowupPreview, ClosureHandoffPreview, SentinelScan } from "../features/sentinel/types";
import type { Project } from "../features/workspaces/types";
import { useClosureHandoff } from "../features/sentinel/workbench/useClosureHandoff";
import { useBrowserAuthSessions } from "../features/sentinel/workbench/useBrowserAuthSessions";
import { useWorkbenchSkills } from "../features/sentinel/workbench/useWorkbenchSkills";
import { useFollowupRecovery } from "../features/sentinel/workbench/useFollowupRecovery";
import { useWorkbenchTaskCreation, type WorkbenchMode } from "../features/sentinel/workbench/useWorkbenchTaskCreation";
import WorkbenchGreyboxSettings from "../features/sentinel/workbench/WorkbenchGreyboxSettings.vue";
import WorkbenchCicdSettings from "../features/sentinel/workbench/WorkbenchCicdSettings.vue";
import WorkbenchSkillsCatalog from "../features/sentinel/workbench/WorkbenchSkillsCatalog.vue";
import WorkbenchOverview from "../features/sentinel/workbench/WorkbenchOverview.vue";
import WorkbenchSkillSelection from "../features/sentinel/workbench/WorkbenchSkillSelection.vue";
import WorkbenchBrowserAuth from "../features/sentinel/workbench/WorkbenchBrowserAuth.vue";
import WorkbenchTaskNotices from "../features/sentinel/workbench/WorkbenchTaskNotices.vue";
import WorkbenchWebPolicy from "../features/sentinel/workbench/WorkbenchWebPolicy.vue";
import WorkbenchToolSelection from "../features/sentinel/workbench/WorkbenchToolSelection.vue";
import WorkbenchBudgetField from "../features/sentinel/workbench/WorkbenchBudgetField.vue";
import { useWorkbenchPresentation } from "../features/sentinel/workbench/useWorkbenchPresentation";

const props = defineProps<{
  projects: Project[];
  scans: SentinelScan[];
  projectId?: number;
  initialMode?: "web" | "code" | "greybox" | "cicd" | "skills";
  followup?: GapFollowupPreview;
  handoff?: ClosureHandoffPreview;
}>();
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  reload: [];
  openScan: [scan: SentinelScan];
  prepareScan: [scan: SentinelScan];
  createProject: [];
}>();
const { tr } = useI18n();
type Mode = WorkbenchMode;
const mode = ref<Mode>(props.initialMode || "code");
const scanModes: Array<{ key: Exclude<Mode, "skills">; label: string; detail: string }> = [
  { key: "web", label: "Web 扫描", detail: "授权 URL 与业务接口" },
  { key: "code", label: "代码审计", detail: "仓库、SAST 与依赖" },
  { key: "greybox", label: "灰盒联测", detail: "运行环境 + 源码关联" },
  { key: "cicd", label: "CI/CD", detail: "流水线与发布门禁" },
];
const busy = ref(false);
const showAdvanced = ref(false);
const activeProjects = computed(() => props.projects.filter((project) => project.status !== "archived"));
const initialProjectId =
  activeProjects.value.find((project) => project.id === (props.handoff?.projectId || props.followup?.projectId || props.projectId))?.id ||
  activeProjects.value[0]?.id ||
  0;
const form = reactive({
  projectId: initialProjectId,
  taskName: props.handoff ? "独立关联任务" : props.followup ? "补充证据任务" : "",
  urls: props.handoff?.targetUrls.join("\n") || props.followup?.targetUrl || "",
  sourcePath: "",
  skillIds: [] as number[],
  instruction: "",
  scanMode: "standard" as "quick" | "standard" | "deep",
  closure: "breadth" as "breadth" | "proof",
  orchestrationMode: "multi" as "single" | "multi",
  homogeneousWorkers: false,
  homogeneousExecution: false,
  approvedToolIds: [] as string[],
  scopeMode: "full" as "auto" | "diff" | "full",
  diffBase: "origin/main",
  maxBudgetUsd: 5 as number | undefined,
  environment: "staging",
  authProfileName: "",
  authType: "none" as "none" | "cookie" | "bearer" | "header",
  authHeaderName: "",
  authValue: "",
  authSessionId: "",
  authSessionIds: [] as string[],
  authLoginUrl: "",
  authSessionName: "",
  ciProvider: "github",
  repositoryUrl: "",
  branch: "main",
  commitSha: "",
  buildId: "",
  maxCritical: 0,
  maxHigh: 5,
  blockRelease: true,
});
const skillCatalog = useWorkbenchSkills({
  tr, notify: (type, text) => emit("notify", type, text),
  clearSelection: () => { form.skillIds = []; },
});
const { skills } = skillCatalog;
let taskViewGeneration = 0;
let disposed = false;
const authControls = useBrowserAuthSessions({
  form, tr, isDisposed: () => disposed, notify: (type, text) => emit("notify", type, text),
});
const { authSessionScopeId, showAuthSessionPicker,
  resetIdentitySelection, loadAuthSessions } = authControls;
const followupControls = useFollowupRecovery({
  source: () => props.followup, generation: () => taskViewGeneration,
  isDisposed: () => disposed, form, busy, resetIdentitySelection,
  notify: (type, text) => emit("notify", type, text),
});
const { pendingFollowupInput, followupSubmission, restoreFollowupSubmission } = followupControls;
const handoffControls = useClosureHandoff({
  source: () => props.handoff,
  hasFollowup: () => Boolean(props.followup),
  isWebMode: () => mode.value === "web",
  generation: () => taskViewGeneration,
  isCurrent: (source, generation) => !disposed && generation === taskViewGeneration && props.handoff === source,
  form, busy, authSessionScopeId, tr,
  resetIdentitySelection: (scopeId) => {
    if (authSessionScopeId.value !== scopeId) return;
    resetIdentitySelection();
  },
  notify: (type, text) => emit("notify", type, text),
  reload: () => emit("reload"),
  prepareScan: (scan) => emit("prepareScan", scan),
  openScan: (scan) => emit("openScan", scan),
});
const { saveClosureHandoff } = handoffControls;
watch(() => [mode.value, form.projectId, props.followup?.sourceScanId,
  props.handoff?.sourceScanId, props.handoff?.closureId, props.handoff?.sourceHash,
  props.followup?.assessmentMessageId, props.followup?.sourceHash,
  props.followup?.projectId, props.followup?.targetUrl],
() => { taskViewGeneration++; }, { flush: "sync" });
const { start } = useWorkbenchTaskCreation({
  mode, form, busy, followup: () => props.followup,
  hasHandoff: () => Boolean(props.handoff), saveClosureHandoff,
  restoreFollowupSubmission, pendingFollowupInput, followupSubmission,
  authSessionScopeId, resetIdentitySelection,
  generation: () => taskViewGeneration, isDisposed: () => disposed,
  tr, notify: (type, text) => emit("notify", type, text),
  reload: () => emit("reload"), prepareScan: (scan) => emit("prepareScan", scan),
  openScan: (scan) => emit("openScan", scan),
});

watch(
  () => props.initialMode,
  (value) => {
    if (value) mode.value = value;
  },
);
watch(
  () => form.projectId,
  async (value) => {
    resetIdentitySelection();
    if (value) await loadAuthSessions();
  },
);
watch(
  () => props.projectId,
  (value) => {
    if (value && activeProjects.value.some((project) => project.id === value))
      form.projectId = value;
    else if (value) form.projectId = 0;
  },
);
watch(
  () => props.projects,
  () => {
    if (!activeProjects.value.some((project) => project.id === form.projectId))
      form.projectId =
        activeProjects.value.find((project) => project.id === props.projectId)?.id ||
        activeProjects.value[0]?.id ||
        0;
  },
  { deep: true },
);
watch(mode, (value) => {
  form.scanMode = value === "cicd" ? "quick" : value === "web" ? "standard" : "deep";
  form.scopeMode = value === "cicd" ? "auto" : "full";
  form.skillIds = [];
  showAdvanced.value = false;
  showAuthSessionPicker.value = false;
});
const { webPreset, applyWebPreset, selectedSkills, webPolicySummary,
  effectiveWebSkillNames, modeLabel, chooseSource } = useWorkbenchPresentation({ mode, form, skills, tr });
onBeforeUnmount(() => {
  disposed = true;
});
</script>

<template>
  <section class="agent-workbench">
    <nav v-if="mode !== 'skills'" class="agent-mode-switch" aria-label="Nest scan type">
      <button v-for="item in scanModes" :key="item.key" type="button" :disabled="Boolean(followup || handoff)" :class="{ active: mode === item.key }" @click="mode = item.key">
        <strong>{{ tr(item.label, item.label) }}</strong><small>{{ tr(item.detail, item.detail) }}</small>
      </button>
    </nav>
    <template v-if="mode !== 'skills'">
      <div class="workbench-grid">
        <section class="panel workbench-form">
          <div class="panel-heading">
            <div>
              <span class="eyebrow">{{ mode.toUpperCase() }}</span>
              <h3>{{ modeLabel(mode) }}</h3>
              <p v-if="mode === 'web' && form.homogeneousWorkers && !form.homogeneousExecution">{{ tr("主管按观测范围动态拆分任务，按需唤起同质 Worker 分析已有证据并汇报；本阶段只读分析，不派发目标验证或确认漏洞。", "The coordinator decomposes observed scope and wakes identical Workers to analyze existing evidence. This phase is read-only; it does not dispatch target verification or confirm findings.") }}</p>
              <p v-else-if="mode === 'web' && form.homogeneousWorkers">{{ tr("主管按目标分派任务，冻结批准工具和权限；解析辅助代码在无网络沙箱中执行，实际结果与覆盖缺口进入聊天。", "The coordinator assigns work by goal with frozen approved tools and permissions. Parsing helpers execute in an offline sandbox; chat shows actual results and coverage gaps.") }}</p>
              <p v-else-if="mode === 'web'">
                {{
                  tr(
                    "输入 URL 后自动完成页面探索、HTTP/参数捕获、JS/指纹、本地知识与确定性验证；标准扫描处理真实运行时 API，深度扫描优先验证高价值证据。",
                    "Explore the page, capture HTTP and parameters, analyze JS/fingerprints, and match local knowledge; Standard scans cover observed runtime APIs while Deep scans prioritize high-value evidence.",
                  )
                }}
              </p>
              <p v-else-if="mode === 'code'">
                {{
                  tr(
                    "完整阅读本地仓库，验证可复现的代码安全问题。",
                    "Review a local repository and verify reproducible security issues.",
                  )
                }}
              </p>
              <p v-else-if="mode === 'greybox'">
                {{
                  tr(
                    "把运行中的 URL 与本地源码同时交给原生 Agent，关联请求与实现。",
                    "Give the native Agent both live URLs and local source to connect requests with implementation.",
                  )
                }}
              </p>
              <p v-else>
                {{
                  tr(
                    "以快速或变更范围模式审计当前分支，适合提交前检查。",
                    "Audit the current branch in quick or diff scope for pre-commit checks.",
                  )
                }}
              </p>
            </div>
          </div>
          <section v-if="mode === 'web'" class="web-investigation-presets">
            <button :class="{ active: webPreset === 'bounded' }" @click="applyWebPreset('bounded')">
              <span>01</span><strong>快速扫描</strong><small>确定性基线侦察，1 USD 上限</small>
            </button>
            <button :class="{ active: webPreset === 'balanced' }" @click="applyWebPreset('balanced')">
              <span>02</span><strong>标准扫描</strong><small>推荐：自动完成调查与验证，5 USD 上限</small>
            </button>
            <button :class="{ active: webPreset === 'evidence' }" @click="applyWebPreset('evidence')">
              <span>03</span><strong>深度扫描</strong><small>围绕高价值证据深入验证，15 USD 上限</small>
            </button>
          </section>
          <section v-if="!activeProjects.length" class="workspace-empty-callout">
            <Plus :size="22" />
            <div><strong>{{ tr("先建立一个工作空间", "Create a workspace first") }}</strong><p>{{ tr("资产、登录会话、扫描、证据和知识都会绑定到同一工作空间，不会再出现孤立任务。", "Assets, login sessions, scans, evidence, and knowledge stay in one workspace.") }}</p></div>
            <button class="button primary" type="button" @click="emit('createProject')">{{ tr("立即新建工作空间", "Create workspace") }}</button>
          </section>
          <WorkbenchTaskNotices :handoff="handoff" :followup="followup" :busy="busy"
            :handoff-controls="handoffControls" :followup-controls="followupControls" />
          <div class="workbench-fields">
            <div class="project-field-with-action">
              <label class="field"
                ><span>{{ tr("归属工作空间", "Workspace") }}</span
                ><select v-model.number="form.projectId" :disabled="Boolean(followup || handoff)">
                  <option :value="0">{{ tr("请选择或新建", "Select or create") }}</option>
                  <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
                </select></label
              >
              <button class="button ghost" type="button" @click="emit('createProject')"><Plus :size="14" />{{ tr("新建空间", "New workspace") }}</button>
            </div>
            <label class="field"
              ><span>{{ tr("任务名称", "Task name") }}</span
              ><input
                v-model="form.taskName"
                :placeholder="`${modeLabel(mode)} · ${new Date().toLocaleDateString()}`"
            /></label>
            <label v-if="mode !== 'web'" class="field span-two"
              ><span>{{ tr("源码目录", "Source directory") }}</span>
              <div class="path-picker">
                <input
                  v-model="form.sourcePath"
                  readonly
                  :placeholder="
                    tr('选择本地仓库目录', 'Choose a local repository')
                  "
                /><button class="button ghost" @click="chooseSource">
                  <FolderOpen :size="15" />{{ tr("选择", "Choose") }}
                </button>
              </div></label
            >
            <label v-if="mode === 'web'" class="field span-two"
              ><span>{{ tr("授权 Web URL（每行一个）", "Authorized Web URLs (one per line)") }}</span
              ><textarea
                v-model="form.urls"
                :readonly="Boolean(followup)"
                rows="8"
                placeholder="https://app.example.com\nhttps://admin.example.com"
              ></textarea>
              <small class="helper">{{ tr("能打开的页面会直接进入调查。尽量多找会留下「已测未发现」；只证明风险时，没有控制请求、测试请求和影响说明的结论不进漏洞列表。", "Reachable pages are investigated. Breadth keeps tested-without-finding notes. Proof only lists findings that bind a control request, a test request, and an impact statement.") }}</small>
            </label>
            <label v-if="mode === 'web' && !followup && !handoff" class="field">
              <span>{{ tr("执行方式", "Execution") }}</span>
              <select v-model="form.orchestrationMode">
                <option value="multi">{{ tr("多智能体协作", "Multi-agent") }}</option>
                <option value="single">{{ tr("单智能体", "Single agent") }}</option>
              </select>
            </label>
            <label v-if="mode === 'web' && !followup && !handoff && form.orchestrationMode === 'multi'" class="field span-two">
              <span><input type="checkbox" v-model="form.homogeneousWorkers" :disabled="busy" /> {{ tr("主管按需分配同质 Worker", "Coordinator assigns homogeneous workers on demand") }}</span>
              <small class="helper">{{ tr("主管按目标拆分任务，Worker 使用相同只读能力、独立上下文；分派、汇报和裁决在聊天中展示。", "The coordinator splits work by goal. Workers share read-only capabilities and independent contexts; chat shows dispatches, reports and decisions.") }}</small>
              <span v-if="form.homogeneousWorkers"><input type="checkbox" v-model="form.homogeneousExecution" :disabled="busy" /> {{ tr("冻结批准工具并使用受控执行", "Freeze approved tools for controlled execution") }}</span>
            </label>
            <WorkbenchToolSelection v-if="mode === 'web' && !followup && !handoff || form.approvedToolIds.length > 0"
              v-model="form.approvedToolIds" :disabled="busy" :eligible="mode === 'web' && !followup && !handoff && form.orchestrationMode === 'multi' && (!form.homogeneousWorkers || form.homogeneousExecution)" />
            <label v-if="mode === 'web'" class="field"
              ><span>{{ tr("收口", "Closure") }}</span>
              <select v-model="form.closure">
                <option value="breadth">{{ tr("尽量多找", "Find broadly") }}</option>
                <option value="proof">{{ tr("只列出能证明的风险", "Only proven risk") }}</option>
              </select>
            </label>
            <label v-if="mode === 'greybox'" class="field span-two"
              ><span>{{
                tr("授权测试 URL（每行一个）", "Authorized URLs (one per line)")
              }}</span
              ><textarea
                v-model="form.urls"
                rows="4"
                placeholder="https://staging.example.com"
              ></textarea>
            </label>
            <WorkbenchBrowserAuth v-if="mode === 'web' || mode === 'greybox'" :form="form" :controls="authControls" />
            <WorkbenchGreyboxSettings v-if="mode === 'greybox'" :form="form" />
            <WorkbenchCicdSettings v-if="mode === 'cicd'" :form="form" />
            <label class="field"
              ><span>{{ tr("扫描强度", "Scan mode") }}</span
              ><select v-model="form.scanMode">
                <option value="quick">{{ tr("快速扫描", "Quick scan") }}</option>
                <option value="standard">{{ tr("标准扫描", "Standard scan") }}</option>
                <option value="deep">{{ tr("深度扫描", "Deep scan") }}</option>
              </select></label
            >
            <label v-if="mode !== 'web'" class="field"
              ><span>{{ tr("代码范围", "Code scope") }}</span
              ><select v-model="form.scopeMode">
                <option value="auto">{{ tr("Auto：增量优先，失败时整仓", "Auto: diff first, full fallback") }}</option>
                <option value="diff">{{ tr("Diff：仅变更文件，不扩大范围", "Diff: changed files only, no widening") }}</option>
                <option value="full">{{ tr("Full：完整源码范围", "Full: complete source scope") }}</option>
              </select></label
            >
            <p v-if="mode !== 'web'" class="form-hint span-two" role="note">
              {{ form.scopeMode === 'auto'
                ? tr("Auto 允许在对比基线或变更清单不可用时转为整仓分析，并在结果中记录原因。选择增量时沿用 Diff 的 CodeQL 项目上下文覆盖限制。", "Auto permits full analysis when the base or change manifest is unavailable; the reason is recorded. When diff is selected, the same CodeQL project-context coverage limit applies.")
                : form.scopeMode === 'diff'
                  ? tr("Diff 基线无效或变更清单不可用时不执行整仓分析。空增量不代表安全；CodeQL 项目上下文尚未支持增量授权，将记录覆盖缺口。", "Diff never falls back to full analysis. An empty diff is not a clean result; CodeQL project context is not yet supported for diff scope and is reported as a gap.")
                  : tr("Full 使用完整源码快照，不使用隐藏的旧对比基线。依赖缓存、构建输出等排除项仍会记录。", "Full uses the complete source snapshot and ignores stale hidden bases. Exclusions such as dependency caches and build output remain recorded.") }}
            </p>
            <label
              v-if="mode !== 'web' && form.scopeMode !== 'full'"
              class="field"
              ><span>{{ tr("对比分支/提交", "Diff base") }}</span
              ><input v-model="form.diffBase" placeholder="origin/main"
            /></label>
            <WorkbenchBudgetField v-model="form.maxBudgetUsd" :mode="mode" :tr="tr" />
            <WorkbenchWebPolicy v-if="mode === 'web' && !form.homogeneousWorkers" :scan-mode="form.scanMode"
              :summary="webPolicySummary" :skill-names="effectiveWebSkillNames" />
            <button
              v-if="mode === 'web'"
              type="button"
              class="workbench-advanced-toggle span-two"
              @click="showAdvanced = !showAdvanced"
            >
              <span>{{ showAdvanced ? '收起高级设置' : '展开高级设置' }}</span>
              <small>Skills、自定义调查要求；默认流程无需配置</small>
            </button>
            <WorkbenchSkillSelection :mode="mode" :expanded="showAdvanced" :skills="skills" :form="form" />
          </div>
          <div class="selected-skills">
            <span v-for="skill in selectedSkills" :key="skill.id"
              ><Check :size="12" />{{ skill.name }}</span
            >
          </div>
          <footer>
            <small>{{
              tr(
                "第三方库默认只做清单与已知版本风险检查；业务 JS 和应用分包才会深度解析。",
                "Third-party libraries are inventoried and version-checked; business JS and app chunks receive deep analysis.",
              )
            }}</small
            ><button v-if="mode === 'web'" class="button ghost" :disabled="busy" @click="start(true)">
              <Save :size="15" />{{ tr("保存草稿 / 配置控制组", "Save draft / configure controls") }}
            </button><button v-if="!followup && !handoff" class="button primary" :disabled="busy" @click="start()">
              <Zap :size="15" />{{
                busy
                  ? tr("启动中…", "Starting…")
                  : mode === 'web'
                    ? tr("启动扫描", "Start scan")
                    : tr("启动 Nest", "Start Nest")
              }}
            </button>
          </footer>
        </section>
        <WorkbenchOverview :scans="scans" :mode="mode" :mode-label="modeLabel" @open-scan="emit('openScan', $event)" />
      </div>
    </template>

    <WorkbenchSkillsCatalog v-else :catalog="skillCatalog" />
  </section>
</template>

<style scoped src="../features/sentinel/components/workbenchPresentation.css"></style>
