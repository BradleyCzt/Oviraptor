<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Bell, CheckCircle2, ChevronDown, CircleAlert, Database,
  AppWindow, FolderKanban, Languages, Menu, MessageSquare, Palette, PlayCircle, Plus, Puzzle, RefreshCw,
  Settings2, ShieldCheck, Server, SlidersHorizontal, TerminalSquare, Trash2, X,
} from "@lucide/vue";
import { api } from "./api";
import { BUILTIN_PLUGINS, createBuiltinPluginHost, isBuiltinPluginId, getPluginPreferenceNotice, loadPluginEnabledState, savePluginEnabledState, type BuiltinPluginId, type PluginNavigationIntent } from "./plugins";
import { BUILTIN_WORKSPACES } from "./shell/builtinWorkspaces";
import type { AssetWorkspaceIntent, WorkspaceMetadata, ResumeWorkflowHost, ResumeWorkflowResult } from "./shell/workspaceContract";
import WorkspaceNavigation from "./shell/WorkspaceNavigation.vue";
import { moduleDefinition, type ModuleKey, type WorkspaceNavItem } from "./shell/moduleCatalog";
import { normalizeRoute, parseProjectId, persistRoute, restoreRoute, trayRoute } from "./shell/navigation";
const ConversationWorkspace = defineAsyncComponent(() => import("./features/conversations/ConversationWorkspace.vue"));
import packageInfo from "../package.json";
import { useI18n } from "./i18n";
import { useInstallLogPanel } from "./features/environment/useInstallLogPanel";
import type { AppSettings, ConfigProfile, EnvironmentPreparationStatus, EnvironmentReport, InterruptedJob, JobProgressEvent, Project, ProjectImpact, StartupStatus, ToastMessage, ViewKey } from "./types";
import AppSettingsDialog from "./components/AppSettingsDialog.vue";
import ConfigDialog from "./components/ConfigDialog.vue";
import ProjectDialog from "./components/ProjectDialog.vue";
import InlineConfirm from "./components/InlineConfirm.vue";
import ReleaseNotesDialog from "./components/ReleaseNotesDialog.vue";
import WorkerSettingsPanel from "./components/WorkerSettingsPanel.vue";
import defaultBrandIconUrl from "../src-tauri/icons/brand-icon.png";
import "./styles.css";
import "./probe-status.css";
import "./theme-enhancements.css";
import "./shell/agent-shell.css";

const { locale, t, tr, setLocale } = useI18n();
const pluginEnabled = ref(loadPluginEnabledState(localStorage));
const initialRoute = restoreRoute(localStorage);
if (isBuiltinPluginId(initialRoute.module) && !pluginEnabled.value[initialRoute.module]) {
  initialRoute.module="chat";
  if(!["projects","settings"].includes(initialRoute.view))initialRoute.view="chat";
  persistRoute(localStorage,initialRoute);
}
const activeView = ref<ViewKey>(initialRoute.view);
const activeModule = ref<ModuleKey>(initialRoute.module);
const projects = ref<Project[]>([]); const profiles = ref<ConfigProfile[]>([]);
const pluginRefreshToken=ref(0);
const pluginNavigation=ref<WorkspaceNavItem>();
const workspaceMetadata=ref<WorkspaceMetadata>({});
const pendingWorkspaceIntent=ref<AssetWorkspaceIntent>();
const navigationError=ref("");
const pluginPreferenceError=ref(getPluginPreferenceNotice(localStorage)||"");
let nextWorkspaceIntent=0;
const resumeReceipts=ref(new Map<number,{state:"queued"|"admitted"|"settling";result?:ResumeWorkflowResult}>());
const startupActionError=ref("");
const startupReadError=ref("");
let startupReadGeneration=0;
function discardWorkspaceIntent(){
  const intent=pendingWorkspaceIntent.value;
  if(intent?.kind==="resume"&&resumeReceipts.value.get(intent.job.runId)?.state==="queued")resumeReceipts.value.delete(intent.job.runId);
  pendingWorkspaceIntent.value=undefined;
}
async function refreshStartupStatus(){
  const request=++startupReadGeneration;
  try{
    const value=await api.startupStatus();
    if(request!==startupReadGeneration)return;
    startup.value=value;startupReadError.value="";
    for(const [id,receipt] of resumeReceipts.value)if(receipt.state==="settling")resumeReceipts.value.delete(id);
  }catch(error){if(request===startupReadGeneration)startupReadError.value=tr("恢复状态尚未核实，请刷新后再尝试：","Resume status is unverified. Refresh before retrying: ")+String(error);}
}
const resumeWorkflow:ResumeWorkflowHost={
  begin(job){
    const receipt=resumeReceipts.value.get(job.runId);
    if(receipt&&receipt.state!=="queued")return false;
    resumeReceipts.value.set(job.runId,{state:"admitted"});
    return true;
  },
  settle(job,result){
    resumeReceipts.value.set(job.runId,{state:"settling",result:{...result}});
    if(!result.started)startupActionError.value=`${job.projectName||job.projectId} · #${job.runId}: ${result.error||tr("恢复未完成，请核对任务状态","Resume did not complete; check task status")}`;
    void refreshStartupStatus();
    void refresh();
  },
};
const appSettings = ref<AppSettings>({ reminderDays: 7, customIcon: false, deduplicatedAssets: 0 });
const startup = ref<StartupStatus>({ reminderDays: 7, staleProjects: [], interruptedJobs: [] });
const selectedProjectId = ref<number | undefined>(parseProjectId(localStorage.getItem("oviraptor-project")));
const projectDialog = ref(false); const editProject = ref<Project>(); const configDialog = ref(false); const editProfile = ref<ConfigProfile>();
const appSettingsDialog = ref(false); const startupNotice = ref(true);
const releaseNotesDialog = ref(false);
const h1HasChanges = ref(false);
const environment = ref<EnvironmentReport>();
const environmentChecking = ref(false);
const environmentInstalling = ref(false);
const preparationStatus = ref<EnvironmentPreparationStatus>();
const recoveryAcknowledgement = ref("");
const recoveringPreparation = ref(false);
const environmentInstallState = ref<"idle" | "running" | "success" | "error">("idle");
const environmentInstallError = ref("");
const environmentInstallConsole = ref<HTMLElement>();
const { logs: environmentInstallLogs, evicted: environmentInstallEvicted,
  connectionUnavailable: environmentLogDisconnected, gapPossible: environmentLogGap,
  clear: clearEnvironmentLogs } = useInstallLogPanel(() => {
  const output = environmentInstallConsole.value?.querySelector("pre");
  const follow = !output || output.scrollHeight - output.scrollTop - output.clientHeight < 40;
  void nextTick(() => {
    const current = environmentInstallConsole.value?.querySelector("pre");
    if (follow && current) current.scrollTop = current.scrollHeight;
  });
});
const configSection = ref<"profiles" | "workers" | "runtime" | "plugins">("profiles");
const deletingProfileId = ref<number>();
const sidebarCollapsed = ref(true); const loading = ref(true); const toasts = ref<ToastMessage[]>([]);
const deletingProjectId = ref<number>();
const projectImpactLoadingId = ref<number>();
const pendingProjectAction = ref<{ project: Project; impact: ProjectImpact; mode: "delete" | "archive" }>();
const storedTheme=localStorage.getItem("oviraptor-theme")||"cloud";
const themePreset=ref(storedTheme==="cloud"?"cloud":"codex");
if(storedTheme!==themePreset.value)localStorage.setItem("oviraptor-theme",themePreset.value);
const accentColor=ref(localStorage.getItem("oviraptor-accent")||"#2878ff");
const sidebarElement = ref<HTMLElement>();
const sidebarTrigger = ref<HTMLButtonElement>();
const utilityMenu = ref<HTMLDetailsElement>();
const isMobile = ref(false);
let sidebarOpenedFrom: HTMLElement | undefined;
let mobileQuery: MediaQueryList | undefined;
function updateMobileLayout() { isMobile.value = mobileQuery?.matches ?? false; }
function closeUtilityMenu(restoreFocus = false) { if (utilityMenu.value) { utilityMenu.value.open = false; if (restoreFocus) utilityMenu.value.querySelector<HTMLElement>("summary")?.focus(); } }
function closeSidebar() {
  sidebarCollapsed.value = true;
  void nextTick(() => {
    const target = isMobile.value ? (sidebarOpenedFrom?.isConnected ? sidebarOpenedFrom : sidebarTrigger.value) : sidebarElement.value?.querySelector<HTMLElement>(".sidebar-toggle");
    target?.focus();
  });
}
function toggleSidebar(event?: MouseEvent) {
  if (!sidebarCollapsed.value) { closeSidebar(); return; }
  sidebarOpenedFrom = event?.currentTarget as HTMLElement | undefined;
  sidebarCollapsed.value = false;
  if (isMobile.value) void nextTick(() => sidebarElement.value?.querySelector<HTMLButtonElement>("button")?.focus());
}
function handleSidebarKeys(event: KeyboardEvent) {
  if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeSidebar(); return; }
  if (event.key !== "Tab" || !isMobile.value || sidebarCollapsed.value) return;
  const focusable = Array.from(sidebarElement.value?.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), summary, [tabindex="0"]') ?? []).filter(element => element.getClientRects().length > 0);
  const first = focusable[0], last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}
function historySelected() { if (isMobile.value) closeSidebar(); }
onMounted(() => {
  if (typeof window.matchMedia === "function") {
    mobileQuery = window.matchMedia("(max-width: 760px)");
    updateMobileLayout();
    mobileQuery.addEventListener("change", updateMobileLayout);
  }
});
onUnmounted(() => mobileQuery?.removeEventListener("change", updateMobileLayout));
const brandIconUrl=ref(defaultBrandIconUrl);
let unlisten: UnlistenFn | undefined;
let unlistenTray: UnlistenFn | undefined;
let workerSyncTimer: ReturnType<typeof setInterval> | undefined;
let workerAutoSyncRunning=false;

const selectedProject = computed(() => projects.value.find(project => project.id === selectedProjectId.value));
const activeProfile = computed(() => profiles.value.find(profile=>profile.isDefault)||profiles.value[0]);
const hackerOneEnabled = computed(() => Boolean(activeProfile.value?.settings?.hackerOneUsername?.trim() && activeProfile.value?.settings?.hackerOneToken?.trim()));
const activeWorkspace = computed(() => isBuiltinPluginId(activeModule.value) && pluginAvailable(activeModule.value) && !["projects","settings"].includes(activeView.value) ? BUILTIN_WORKSPACES[activeModule.value] : undefined);
const viewTitle = computed(() => {
  if(activeWorkspace.value&&workspaceMetadata.value.title)return workspaceMetadata.value.title;
  return ({chat:tr("对话","Chat"),dashboard:t.value.dashboard,projects:t.value.projects,query:t.value.query,assets:t.value.assets,ownership:tr("归属门禁","Ownership gate"),exposure:tr("公开暴露面","Public exposure"),quarantine:t.value.quarantine,hackerone:"HackerOne SRC",sentinel:tr("扫描","Scanning"),changes:t.value.changes,tasks:t.value.tasks,logs:t.value.logs,settings:t.value.settings})[activeView.value];
});
function pluginAvailable(module:ModuleKey) { return module==="chat" || (pluginEnabled.value[module] && (module!=="hackerone" || hackerOneEnabled.value)); }
function applyRoute(module:ModuleKey, view:ViewKey, navigation?:WorkspaceNavItem) {
  const route=normalizeRoute(module,view);
  if(["settings","projects"].includes(route.view)&&!pluginAvailable(route.module))route.module="chat";
  if(!pluginAvailable(route.module)) {
    navigationError.value=route.module==="hackerone"&&pluginEnabled.value.hackerone?tr("请先在配置中心填写 HackerOne API identifier 和 token","Configure HackerOne API credentials first"):tr("此内置插件已停用，请在配置中心的内置插件中启用。","This built-in plugin is disabled. Enable it under Settings → Built-in plugins.");
    return false;
  }
  navigationError.value="";
  if(route.module!==activeModule.value){workspaceMetadata.value={};pluginNavigation.value=undefined;discardWorkspaceIntent();}
  if(route.module!=="asset"||route.view!=="tasks")discardWorkspaceIntent();
  activeModule.value=route.module;activeView.value=route.view;
  if(navigation)pluginNavigation.value={...navigation};
  persistRoute(localStorage,route);
  return true;
}
function navigate(view:ViewKey) { applyRoute(activeModule.value,view); }
function navigateFromTray(destination:string) {
  const route=trayRoute(destination);if(!route)return;
  const item=route.section?{key:route.section,view:route.view,label:["任务与成本","Tasks & cost"] as const,icon:"tasks" as const,section:route.section}:undefined;
  applyRoute(route.module,route.view,item);
}
function switchModule(module:ModuleKey) { if (applyRoute(module,moduleDefinition(module).defaultView)) historySelected(); }
function openWorkspaceSection(item:WorkspaceNavItem) { if (applyRoute(activeModule.value,item.view,item)) historySelected(); }
function selectProject(id?:number) {
  const candidate=parseProjectId(id);
  selectedProjectId.value=candidate&&projects.value.some(project=>project.id===candidate)?candidate:undefined;
  selectedProjectId.value?localStorage.setItem("oviraptor-project",String(selectedProjectId.value)):localStorage.removeItem("oviraptor-project");
  discardWorkspaceIntent();workspaceMetadata.value={};
}
function openPluginWorkspace(intent:PluginNavigationIntent) {
  if(intent.projectId!==selectedProjectId.value||!projects.value.some(project=>project.id===intent.projectId))throw Error("plugin_scope_changed");
  const item=intent.pluginId==="sentinel"?{key:intent.section==="results"?"urls":intent.section==="workbench"?"new_scan":intent.section,view:intent.page,label:["扫描","Scanning"] as const,icon:"scan" as const,section:intent.section}:undefined;
  if(!applyRoute(intent.pluginId,intent.page,item))throw Error("plugin_disabled");
}
const pluginHost=createBuiltinPluginHost({api,enabled:()=>pluginEnabled.value,projectId:()=>selectedProjectId.value,projects:()=>projects.value,navigate:openPluginWorkspace});
watch(selectedProjectId,()=>pluginHost.invalidate(),{flush:"sync"});
watch(pluginEnabled,()=>{
  pluginHost.invalidate();
  if(isBuiltinPluginId(activeModule.value)&&!pluginEnabled.value[activeModule.value]){
    discardWorkspaceIntent();
    const keepShared=["settings","projects"].includes(activeView.value);
    applyRoute("chat",keepShared?activeView.value:"chat");
  }
},{deep:true,flush:"sync"});
watch(hackerOneEnabled,enabled=>{if(!enabled&&activeModule.value==="hackerone")applyRoute("chat",["settings","projects"].includes(activeView.value)?activeView.value:"chat");});
function togglePlugin(id:BuiltinPluginId, enabled:boolean) {
  pluginPreferenceError.value="";
  try{const next={...pluginEnabled.value,[id]:enabled};savePluginEnabledState(localStorage,next);pluginEnabled.value=next;}
  catch(error){pluginPreferenceError.value=String(error);}
}
function onPluginToggle(id:BuiltinPluginId,event:Event){const target=event.target as HTMLInputElement;togglePlugin(id,target.checked);target.checked=pluginEnabled.value[id];}
function managePlugins(){configSection.value="plugins";navigate("settings");}
function dispatchAssetIntent(input:Omit<AssetWorkspaceIntent,"id">) {
  if(!pluginAvailable("asset")){applyRoute("asset","tasks");return;}
  if(!projects.value.some(project=>project.id===input.projectId&&project.status==="active")){navigationError.value=tr("请先选择有效的活跃工作空间","Select a valid active workspace first");return;}
  selectProject(input.projectId);
  if(applyRoute("asset","tasks"))pendingWorkspaceIntent.value={...input,id:++nextWorkspaceIntent} as AssetWorkspaceIntent;
}
function quickStartProject(projectId:number){dispatchAssetIntent({kind:"start",projectId});}
function resumeInterrupted(job:InterruptedJob){
  if(resumeReceipts.value.has(job.runId)||!startup.value.interruptedJobs.some(item=>item.runId===job.runId&&item.projectId===job.projectId))return;
  startupActionError.value="";
  dispatchAssetIntent({kind:"resume",projectId:job.projectId,job:{...job}} as AssetWorkspaceIntent);
  if(pendingWorkspaceIntent.value?.kind==="resume"&&pendingWorkspaceIntent.value.job.runId===job.runId)resumeReceipts.value.set(job.runId,{state:"queued"});
}
function consumeWorkspaceIntent(id:number){if(pendingWorkspaceIntent.value?.id===id)pendingWorkspaceIntent.value=undefined;}
function onInterruptedResumed(runId:number){startup.value.interruptedJobs=startup.value.interruptedJobs.filter(job=>job.runId!==runId);}
function notify(type: "success"|"error"|"info", text: string) { const item={id:Date.now()+Math.random(),type,text}; toasts.value.push(item); setTimeout(()=>toasts.value=toasts.value.filter(t=>t.id!==item.id),5000); }
function setTheme(preset:string){themePreset.value=preset;localStorage.setItem("oviraptor-theme",preset)}
function saveAccent(){localStorage.setItem("oviraptor-accent",accentColor.value)}
let workspaceRefreshGeneration=0;
async function refresh(_waitForSecondary:boolean|Event=false){
  const request=++workspaceRefreshGeneration;
  try{
    const [nextProjects,nextProfiles,nextSettings]=await Promise.all([api.listProjects(),api.listProfiles(),api.getAppSettings()]);
    if(request!==workspaceRefreshGeneration)return;
    projects.value=nextProjects;profiles.value=nextProfiles;appSettings.value=nextSettings;
    if(selectedProjectId.value&&!projects.value.some(project=>project.id===selectedProjectId.value))selectProject(undefined);
    if(!pluginAvailable(activeModule.value))applyRoute("chat",["settings","projects"].includes(activeView.value)?activeView.value:"chat");
    pluginRefreshToken.value++;
    if(hackerOneEnabled.value&&pluginEnabled.value.hackerone)void api.listHackerOneEvents(1).then(value=>{if(request===workspaceRefreshGeneration)h1HasChanges.value=value.length>0;}).catch(()=>{});
    else h1HasChanges.value=false;
  }catch(error){if(request===workspaceRefreshGeneration)notify("error",String(error));}
  finally{if(request===workspaceRefreshGeneration)loading.value=false;}
}
async function projectSaved(id:number){ projectDialog.value=false;editProject.value=undefined;await refresh();selectProject(id); }
async function profileSaved(){ configDialog.value=false;editProfile.value=undefined;await refresh(); }
function openProject(project?:Project){ editProject.value=project;projectDialog.value=true; }
function openProjectWorkspace(project:Project,module:ModuleKey){
  if(!pluginAvailable(module)){applyRoute(module,moduleDefinition(module).defaultView);return;}
  selectProject(project.id);
  applyRoute(module,moduleDefinition(module).defaultView,module==="sentinel"?{key:"dialog",view:"sentinel",label:["调查对话","Investigation dialog"],icon:"dialog",section:"dialog"}:undefined);
}
function projectActivity(project: Project) {
  return [project.lastRunAt, project.lastScanAt].filter(Boolean).sort().reverse()[0] || "—";
}
function projectImpactText(impact: ProjectImpact) {
  const parts = [
    impact.assetCount ? `${impact.assetCount} 条资产` : "",
    impact.assetRunCount ? `${impact.assetRunCount} 次资产任务` : "",
    impact.assetEventCount ? `${impact.assetEventCount} 条资产历史` : "",
    impact.targetCount ? `${impact.targetCount} 个采集目标` : "",
    impact.sentinelScanCount ? `${impact.sentinelScanCount} 个 Nest 任务` : "",
    impact.sentinelTargetCount ? `${impact.sentinelTargetCount} 个 Nest 目标` : "",
    impact.opportunityCount ? `${impact.opportunityCount} 条测试机会` : "",
    impact.findingCount ? `${impact.findingCount} 条证据` : "",
    impact.validationCount ? `${impact.validationCount} 条验证` : "",
    impact.appsecVulnerabilityCount ? `${impact.appsecVulnerabilityCount} 条漏洞结论` : "",
    impact.fuseCount ? `${impact.fuseCount} 条停止记录` : "",
    impact.knowledgeCount ? `${impact.knowledgeCount} 条知识` : "",
    impact.savedViewCount ? `${impact.savedViewCount} 个保存视图` : "",
    impact.learningCandidateCount ? `${impact.learningCandidateCount} 个学习候选` : "",
    impact.browserAuthSessionCount ? `${impact.browserAuthSessionCount} 个浏览器会话` : "",
    impact.otherRecordCount ? `${impact.otherRecordCount} 条其他关联记录（调查、知识或配置等）` : "",
  ].filter(Boolean);
  return parts.join("、") || tr("没有关联数据", "No linked data");
}
async function removeProject(project:Project){
  if(deletingProjectId.value!==undefined||projectImpactLoadingId.value!==undefined)return;
  projectImpactLoadingId.value=project.id;
  pendingProjectAction.value=undefined;
  try {
    const impact=await api.projectImpact(project.id);
    pendingProjectAction.value={project,impact,mode:impact.totalRecords>0?"archive":"delete"};
  } catch(error){ notify("error",String(error)); }
  finally{projectImpactLoadingId.value=undefined;}
}
function cancelProjectAction(){
  if(deletingProjectId.value!==undefined)return;
  pendingProjectAction.value=undefined;
}
async function confirmProjectAction(){
  const action=pendingProjectAction.value;
  if(!action||deletingProjectId.value!==undefined||projectImpactLoadingId.value!==undefined)return;
  deletingProjectId.value=action.project.id;
  try {
    if(action.mode==="archive"){
      await api.archiveProject(action.project.id,true);
      notify("success",tr("工作空间已归档，历史数据保持可查","Workspace archived; history remains available"));
    } else {
      await api.deleteProject(action.project.id);
      notify("success",tr("空工作空间已删除","Empty workspace deleted"));
    }
    pendingProjectAction.value=undefined;
    await refresh();
  } catch(error){ notify("error",String(error)); }
  finally{deletingProjectId.value=undefined;}
}
async function toggleArchive(project:Project){
  try { await api.archiveProject(project.id,project.status==="active"); await refresh(); }
  catch(error){ notify("error",String(error)); }
}
function openProfile(profile?:ConfigProfile){ editProfile.value=profile;configDialog.value=true; }
function cloneDefaultProfile(){
  const source=profiles.value.find(item=>item.isDefault)||profiles.value[0];
  if(!source){notify("error",tr("没有可复制的系统配置","No system profile is available to copy"));return;}
  editProfile.value={...source,id:0,name:`${source.name} ${tr("副本","Copy")}`,description:tr("从系统默认配置创建，可独立调整。","Created from the system default and editable independently."),isDefault:false,settings:structuredClone(source.settings),createdAt:"",updatedAt:""};
  configDialog.value=true;
}
async function removeProfile(profile:ConfigProfile){
  if(profile.isDefault){notify("info",tr("系统默认配置不能删除","The system default profile cannot be deleted"));return;}
  if(deletingProfileId.value!==profile.id){deletingProfileId.value=profile.id;notify("info",tr("再次点击删除以确认","Click Delete again to confirm"));return;}
  try{await api.deleteProfile(profile.id);await refresh();notify("success",tr("配置已删除","Profile deleted"));}
  catch(error){notify("error",String(error));}
  finally{deletingProfileId.value=undefined;}
}
async function loadBrandIcon(){
  if(!appSettings.value.customIcon){brandIconUrl.value=defaultBrandIconUrl;return;}
  try{brandIconUrl.value=await api.getAppIconDataUrl();}
  catch{brandIconUrl.value=defaultBrandIconUrl;}
}
async function appSettingsSaved(){ appSettingsDialog.value=false; await refresh(); await loadBrandIcon(); }
async function checkEnvironment(){
  environmentChecking.value=true;
  try {
    await refreshPreparationStatus();
    environment.value=await api.checkEnvironment(profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id);
    notify("success",tr("环境检查完成","Environment check completed"));
  } catch(error){ notify("error",String(error)); }
  finally { environmentChecking.value=false; }
}
async function refreshPreparationStatus(){
  try { preparationStatus.value=await api.getEnvironmentPreparationStatus(); }
  catch(error){ notify("error",String(error)); }
}
async function recoverPreparation(){
  const owner=preparationStatus.value?.owner;
  if(!owner || recoveryAcknowledgement.value!=="已确认安装子进程停止")return;
  recoveringPreparation.value=true;
  try {
    await api.recoverEnvironmentPreparation(owner,recoveryAcknowledgement.value);
    recoveryAcknowledgement.value="";
    notify("success",tr("环境准备租约已人工恢复，事件已记录","Preparation lease recovered; event recorded"));
  } catch(error){ notify("error",String(error)); }
  finally { recoveringPreparation.value=false; await refreshPreparationStatus(); }
}
watch(configSection,section=>{ if(section==="runtime")void refreshPreparationStatus(); });
async function installEnvironment(){
  environmentInstalling.value=true;
  environmentInstallError.value="";
  environmentInstallState.value="running";
  try {
    await clearEnvironmentLogs();
    const profileId=profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id;
    const message=await api.installEnvironmentDependencies(profileId);
    environment.value=await api.checkEnvironment(profileId);
    environmentInstallState.value="success";
    notify("success",message);
  } catch {
    const message=tr("安装或环境校验未完成，请检查脱敏日志与环境准备状态。","Installation or verification did not finish. Check the redacted logs and preparation status.");
    environmentInstallError.value=message;
    environmentInstallState.value="error";
    notify("error",message);
  } finally { environmentInstalling.value=false; await refreshPreparationStatus(); }
}
async function autoSyncWorkers(){
  if(workerAutoSyncRunning)return;
  workerAutoSyncRunning=true;
  try{
    const nodes=await api.listWorkerNodes();
    // Import one potentially large Worker bundle at a time. Parallel imports
    // compete for SQLite and memory with active scans and previously caused a
    // burst exactly when a suspended macOS window became visible again.
    for(const node of nodes.filter(node=>node.enabled)){
      try{await api.syncWorkerNode(node.id);}catch{/* 节点错误已写入节点状态。 */}
    }
  }catch{/* 尚未配置 Worker 时保持安静。 */}
  finally{workerAutoSyncRunning=false;}
}

onMounted(async()=>{
  void refreshStartupStatus();
  await refresh();
  void loadBrandIcon();
  void autoSyncWorkers();
  workerSyncTimer=setInterval(autoSyncWorkers,5*60*1000);
  unlisten=await listen<JobProgressEvent>("job-progress",event=>{
    const progress=event.payload;
    if(["completed","failed","cancelled"].includes(progress.status)){notify(progress.status==="completed"?"success":"error",progress.message);void refresh();}
  });
  unlistenTray=await listen<string>("tray-navigate",event=>navigateFromTray(event.payload));
});
onUnmounted(()=>{unlisten?.();unlistenTray?.();if(workerSyncTimer)clearInterval(workerSyncTimer)});
</script>

<template>
  <div class="app-shell agent-shell" :class="[{ 'sidebar-collapsed': sidebarCollapsed, 'chat-route': activeView==='chat' },`theme-${themePreset}`]" :style="{'--blue':accentColor}">
    <button v-if="!sidebarCollapsed" class="sidebar-backdrop" tabindex="-1" :aria-label="tr('关闭侧边栏','Close sidebar')" @click="closeSidebar"></button>
    <aside id="app-navigation" ref="sidebarElement" class="sidebar" :inert="isMobile&&sidebarCollapsed" :role="isMobile&&!sidebarCollapsed?'dialog':undefined" :aria-modal="isMobile&&!sidebarCollapsed?true:undefined" :aria-label="tr('导航和历史记录','Navigation and history')" @keydown="handleSidebarKeys">
      <div class="sidebar-heading">
        <div class="brand"><div class="brand-mark"><img v-if="brandIconUrl" :src="brandIconUrl" alt="" /></div><div class="brand-copy"><strong>{{t.appName}}</strong></div></div>
        <button class="sidebar-toggle" :aria-expanded="!sidebarCollapsed" aria-controls="sidebar-expanded-content" :aria-label="sidebarCollapsed?tr('展开侧边栏','Expand sidebar'):tr('收起侧边栏','Collapse sidebar')" :title="sidebarCollapsed?tr('展开侧边栏','Expand sidebar'):tr('收起侧边栏','Collapse sidebar')" @click="toggleSidebar"><Menu :size="18" /></button>
      </div>
      <WorkspaceNavigation :collapsed="sidebarCollapsed" :module="activeModule" :view="activeView" :sentinel-menu="workspaceMetadata.navigationKey||pluginNavigation?.key||'dialog'" :hacker-one-enabled="hackerOneEnabled" :enabled-plugins="pluginEnabled" :running-jobs="workspaceMetadata.badges?.runningJobs" :blocked-count="workspaceMetadata.badges?.blockedCount" :fuse="workspaceMetadata.badges?.fuse" @module-change="switchModule" @navigate="openWorkspaceSection" />
      <div id="sidebar-expanded-content" class="sidebar-expanded-content" v-show="!sidebarCollapsed">
        <div id="conversation-history" v-show="activeView==='chat'" class="shell-conversation-history"></div>
        <div class="sidebar-project">
          <span class="sidebar-label">{{tr('当前工作空间','WORKSPACE')}}</span>
          <button class="current-project" :title="tr('管理项目与范围','Manage projects and scope')" @click="navigate('projects');historySelected()"><span class="project-dot"></span><span>{{selectedProject?.name||t.allProjects}}</span><ChevronDown :size="14" /></button>
          <button class="sidebar-create-workspace" @click="openProject()"><Plus :size="15" />{{tr('新建工作空间','New workspace')}}</button>
        </div>
      </div>
      <div class="sidebar-fixed-links">
        <button v-if="moduleDefinition(activeModule).hasLogs" :title="t.logs" :aria-label="t.logs" :class="{active:activeView==='logs'}" @click="navigate('logs');historySelected()"><TerminalSquare :size="17" /><span>{{t.logs}}</span></button>
        <button :title="t.settings" :aria-label="t.settings" :class="{active:activeView==='settings'}" @click="navigate('settings');historySelected()"><Settings2 :size="17" /><span>{{t.settings}}</span></button>
      </div>
      <footer class="sidebar-footer"><button :title="tr('查看更新说明','View release notes')" @click="releaseNotesDialog=true">v{{packageInfo.version}}</button><span>Local-first</span></footer>
    </aside>

    <main class="main-area" :inert="isMobile&&!sidebarCollapsed">
      <header class="topbar">
        <div class="topbar-leading"><button ref="sidebarTrigger" class="top-icon main-sidebar-toggle" :aria-expanded="!sidebarCollapsed" aria-controls="app-navigation" :aria-label="sidebarCollapsed?tr('展开侧边栏','Expand sidebar'):tr('收起侧边栏','Collapse sidebar')" @click="toggleSidebar"><Menu :size="19"/></button><div class="page-heading"><h1>{{viewTitle}}</h1><span v-if="activeView!=='chat'">{{selectedProject?.name||t.allProjects}}</span></div></div>
        <div class="topbar-actions">
          <div class="project-switcher-group"><select class="project-switcher" :aria-label="tr('选择工作空间','Choose workspace')" :value="selectedProjectId" @change="selectProject(Number(($event.target as HTMLSelectElement).value)||undefined)"><option value="">{{t.allProjects}}</option><option v-for="project in projects" :key="project.id" :value="project.id">{{project.name}}{{project.status==='archived'?tr('（已归档）',' (archived)'):''}}</option></select></div>
          <details ref="utilityMenu" class="shell-utility-menu" @keydown.esc.stop="closeUtilityMenu(true)">
            <summary class="top-icon" :aria-label="tr('应用选项','App options')" :title="tr('应用选项','App options')"><SlidersHorizontal :size="18"/></summary>
            <div class="shell-utility-panel">
              <div class="utility-theme-heading"><Palette :size="16"/><strong>{{tr('外观','Appearance')}}</strong></div>
              <div class="utility-theme-options"><button v-for="item in [['cloud',tr('浅色','Light')],['codex',tr('深色','Dark')]]" :key="item[0]" :class="{active:themePreset===item[0]}" :aria-pressed="themePreset===item[0]" @click="setTheme(item[0])">{{item[1]}}</button></div>
              <label class="utility-accent"><span>{{tr('强调色','Accent color')}}</span><input v-model="accentColor" type="color" @change="saveAccent" /></label>
              <button @click="setLocale(locale==='zh'?'en':'zh')"><Languages :size="16"/>{{locale==='zh'?'English':'中文'}}</button>
              <button @click="openProject();closeUtilityMenu()"><Plus :size="16"/>{{tr('新建工作空间','New workspace')}}</button>
              <button @click="managePlugins();closeUtilityMenu()"><Puzzle :size="16"/>{{tr('内置插件','Built-in plugins')}}</button>
              <button @click="navigate('settings');closeUtilityMenu()"><Settings2 :size="16"/>{{t.settings}}</button>
              <button v-if="activeModule!=='chat'" @click="navigate(h1HasChanges?'hackerone':'tasks');closeUtilityMenu()"><Bell :size="16"/>{{tr('任务与变化','Tasks & changes')}}</button>
            </div>
          </details>
        </div>
      </header>

      <div class="content-area">
        <p v-if="navigationError" class="host-action-error" role="alert">{{navigationError}} <button class="text-button" @click="managePlugins">{{tr('管理内置插件','Manage built-in plugins')}}</button></p>
        <p v-if="startupActionError||startupReadError" class="host-action-error" role="alert">{{startupActionError}} <span v-if="startupReadError">{{startupReadError}}</span> <button class="text-button" @click="refreshStartupStatus">{{tr('刷新任务状态','Refresh task status')}}</button></p>
        <section v-if="!loading&&startupNotice&&(startup.interruptedJobs.length||startup.staleProjects.length)" class="startup-alert panel">
          <div class="startup-alert-icon"><CircleAlert :size="20" /></div>
          <div class="startup-alert-copy">
            <strong>{{startup.interruptedJobs.length?tr(`发现 ${startup.interruptedJobs.length} 个中断任务`,`Found ${startup.interruptedJobs.length} interrupted jobs`):tr('项目更新提醒','Project update reminder')}}</strong>
            <span>{{tr('自动更新需要电脑和应用保持运行；关闭窗口后应用会留在 macOS 状态栏。','Automatic updates require the computer and app to stay running; closing the window keeps the app in the macOS menu bar.')}}</span>
            <div v-if="startup.interruptedJobs.length" class="startup-items"><button v-for="job in startup.interruptedJobs.slice(0,4)" :key="job.runId" :disabled="resumeReceipts.has(job.runId)" @click="resumeInterrupted(job)"><PlayCircle :size="14" /> {{job.projectName}} · {{tr('继续更新','Resume')}}</button></div>
            <div v-if="startup.staleProjects.length" class="startup-items"><button v-for="project in startup.staleProjects.slice(0,4)" :key="project.projectId" @click="quickStartProject(project.projectId)"><RefreshCw :size="14" /> {{project.projectName}} · {{project.daysSinceUpdate==null?tr('尚未更新','Never updated'):tr(`${project.daysSinceUpdate} 天未更新`,`${project.daysSinceUpdate} days old`)}}</button></div>
          </div>
          <button class="icon-button subtle" @click="startupNotice=false"><X :size="15" /></button>
        </section>
        <div v-if="loading" class="app-loading"><div class="loader-ring"></div><span>{{tr('正在初始化本地资产数据库…','Initializing local asset database…')}}</span></div>

        <ConversationWorkspace v-else-if="activeView==='chat'" :projects="projects" :project-id="selectedProjectId" :plugin-host="pluginHost" history-target="#conversation-history" @history-selected="historySelected" @select-project="selectProject" @create-project="openProject()" />

        <component v-else-if="activeWorkspace" :is="activeWorkspace" :key="activeModule" :projects="projects" :profiles="profiles" :project-id="selectedProjectId" :view="activeView" :refresh-token="pluginRefreshToken" :navigation="pluginNavigation" :intent="pendingWorkspaceIntent" :scanning-enabled="pluginEnabled.sentinel" :resume-workflow="resumeWorkflow" @navigate="navigate" @select-project="selectProject" @create-project="openProject()" @restore-project="toggleArchive" @projects-changed="refresh" @notify="notify" @metadata="workspaceMetadata=$event" @intent-consumed="consumeWorkspaceIntent" @interrupted-resumed="onInterruptedResumed" />

        <template v-else-if="activeView==='projects'">
          <div class="section-toolbar"><div><span class="eyebrow">SHARED WORKSPACES</span><h2>{{tr('项目与范围','Projects and scope')}}</h2><p>{{tr('一个工作空间共享对话、资产范围、扫描、证据、漏洞结论和知识沉淀。','One workspace holds conversations, asset scope, scans, evidence, conclusions, and learned knowledge.')}}</p></div><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('新建工作空间','New workspace')}}</button></div>
          <div v-if="projects.length" class="project-grid shared-project-grid">
            <article v-for="project in projects" :key="project.id" class="project-card shared-project-card" :class="{selected:selectedProjectId===project.id,archived:project.status==='archived'}">
              <header><div class="project-icon"><FolderKanban :size="20" /></div><span class="state-pill">{{project.status==='active'?tr('活跃','Active'):tr('已归档','Archived')}}</span></header>
              <h3>{{project.name}}</h3><p>{{project.description||tr('暂无范围与授权说明','No scope or authorization note')}}</p>
              <div class="project-metrics shared-project-metrics"><span><strong>{{project.assetCount.toLocaleString()}}</strong>{{tr('资产','Assets')}}</span><span><strong>{{project.scanCount.toLocaleString()}}</strong>Nest</span><span><strong>{{project.vulnerabilityCount.toLocaleString()}}</strong>{{tr('漏洞记录（含历史）','Finding records (incl. history)')}}</span><span><strong>{{project.activeFuseCount.toLocaleString()}}</strong>{{tr('待处置','Stopped')}}</span></div>
              <div class="project-entry-actions"><button class="button ghost compact" @click="openProjectWorkspace(project,'chat')"><MessageSquare :size="14" />{{tr('对话','Chat')}}</button><button class="button ghost compact" @click="openProjectWorkspace(project,'asset')"><Database :size="14" />Asset</button><button class="button secondary compact" @click="openProjectWorkspace(project,'sentinel')"><ShieldCheck :size="14" />Nest</button></div>
              <footer><span>{{tr('最近活动','Last activity')}} {{projectActivity(project)}}</span><div><button class="text-button" @click="openProject(project)">{{tr('编辑','Edit')}}</button><button class="text-button" @click="toggleArchive(project)">{{project.status==='active'?tr('归档','Archive'):tr('恢复','Restore')}}</button><button class="text-button danger-text" :disabled="deletingProjectId!==undefined||projectImpactLoadingId!==undefined" :aria-busy="projectImpactLoadingId===project.id" @click="removeProject(project)">{{projectImpactLoadingId===project.id?tr('检查中…','Checking…'):tr('删除','Delete')}}</button></div></footer>
              <InlineConfirm v-if="pendingProjectAction?.project.id===project.id" :title="pendingProjectAction.mode==='archive'?tr('该工作空间有关联数据，改为归档？','This workspace has linked data. Archive it?'):tr('删除这个空工作空间？','Delete this empty workspace?')" :detail="pendingProjectAction.mode==='archive'?`${projectImpactText(pendingProjectAction.impact)}。归档后禁止创建新任务，但历史和结论继续可查。`:tr('它没有资产、扫描、证据或知识记录，删除后无法恢复。','It has no assets, scans, evidence, or knowledge and cannot be recovered.')" :confirm-text="pendingProjectAction.mode==='archive'?tr('归档并保留历史','Archive and preserve history'):tr('确认删除','Delete')" :busy-text="pendingProjectAction.mode==='archive'?tr('归档中…','Archiving…'):tr('删除中…','Deleting…')" :tone="pendingProjectAction.mode==='archive'?'warning':'danger'" :busy="deletingProjectId===project.id" @cancel="cancelProjectAction" @confirm="confirmProjectAction" />
            </article>
          </div>
          <div v-else class="first-run panel"><div class="first-run-icon"><FolderKanban :size="28" /></div><h2>{{tr('创建第一个公共工作空间','Create your first shared workspace')}}</h2><p>{{tr('无需先采集资产；也可以直接创建 Nest Web、代码审计或灰盒任务。','Asset collection is optional; you can start directly with Nest web, code, or grey-box tasks.')}}</p><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('创建工作空间','Create workspace')}}</button></div>
        </template>

        <template v-else-if="activeView==='settings'">
          <div class="section-toolbar"><div><span class="eyebrow">CONFIGURATION CENTER</span><h2>{{tr('配置中心','Configuration center')}}</h2><p>{{tr('运行方案、远程 Worker 和本机环境分区管理，避免创建无内容的空配置。','Manage runtime profiles, remote Workers and the local environment without creating empty profiles.')}}</p></div><button class="button ghost" @click="appSettingsDialog=true"><AppWindow :size="16" /> {{tr('应用设置','App settings')}}</button></div>
          <nav class="config-center-nav">
            <button :class="{active:configSection==='profiles'}" @click="configSection='profiles'"><SlidersHorizontal :size="18"/><span><b>{{tr('运行方案','Runtime profiles')}}</b><small>{{tr('账号、模型与扫描参数','Accounts, models and scan policy')}}</small></span></button>
            <button :class="{active:configSection==='workers'}" @click="configSection='workers'"><Server :size="18"/><span><b>Worker {{tr('节点','nodes')}}</b><small>{{tr('Intel Mac / Windows 远程执行','Remote execution on Intel Mac / Windows')}}</small></span></button>
            <button :class="{active:configSection==='plugins'}" @click="configSection='plugins'"><Puzzle :size="18"/><span><b>{{tr('内置插件','Built-in plugins')}}</b><small>{{tr('当前应用中的能力模块','Capability modules in this app')}}</small></span></button>
            <button :class="{active:configSection==='runtime'}" @click="configSection='runtime'"><TerminalSquare :size="18"/><span><b>{{tr('运行环境','Runtime environment')}}</b><small>{{tr('检测、安装与实时输出','Check, install and view live output')}}</small></span></button>
          </nav>

          <template v-if="configSection==='profiles'">
            <section class="app-settings-summary panel"><div class="profile-icon"><AppWindow :size="19" /></div><div><strong>{{tr('更新与后台设置','Update and background')}}</strong><span>{{tr(`超过 ${appSettings.reminderDays} 天未更新时提醒 · ${appSettings.customIcon?'自定义图标':'默认图标'}`,`Remind after ${appSettings.reminderDays} days · ${appSettings.customIcon?'custom icon':'default icon'}`)}}</span></div><button class="button ghost compact" @click="appSettingsDialog=true">{{tr('修改','Edit')}}</button></section>
            <div class="profile-section-heading"><div><h3>{{tr('运行方案','Runtime profiles')}}</h3><p>{{tr('系统默认方案始终保留；需要新方案时从默认方案复制，避免产生空配置。','The system default is permanent. Create new profiles by copying it so they always start complete.')}}</p></div><button class="button primary" @click="cloneDefaultProfile"><Plus :size="16"/>{{tr('从默认方案创建','Create from default')}}</button></div>
            <div class="profile-grid"><article v-for="profile in profiles" :key="profile.id" class="profile-card panel"><header><div class="profile-icon"><Settings2 :size="19" /></div><span v-if="profile.isDefault" class="default-pill">SYSTEM DEFAULT</span></header><h3>{{profile.name}}</h3><p>{{profile.description}}</p><dl><div><dt>{{tr('资产引擎','Asset engine')}}</dt><dd>Rust Native</dd></div><div><dt>{{tr('前端采集','Browser capture')}}</dt><dd>Node.js · CDP</dd></div><div><dt>Probe</dt><dd>{{profile.settings.priorityRate}} r/s · {{profile.settings.workers}} workers</dd></div><div><dt>Rules</dt><dd>{{(profile.settings.gamblingKeywords?.length||0)+(profile.settings.pornKeywords?.length||0)}} keywords</dd></div></dl><footer><span>{{tr('更新于','Updated')}} {{profile.updatedAt}}</span><div><button class="button ghost compact" @click="openProfile(profile)">{{tr('编辑','Edit')}}</button><button v-if="!profile.isDefault" class="button danger compact" @click="removeProfile(profile)"><Trash2 :size="13"/>{{deletingProfileId===profile.id?tr('确认删除','Confirm delete'):tr('删除','Delete')}}</button></div></footer></article></div>
          </template>

          <WorkerSettingsPanel v-else-if="configSection==='workers'" @message="notify" />

          <section v-else-if="configSection==='plugins'" class="panel builtin-plugin-manager">
            <header><h3>{{tr('内置插件','Built-in plugins')}}</h3><p>{{tr('插件随应用提供。停用只隐藏入口并阻止新界面操作；已接受的操作与后台任务继续完成，数据与配置保留。','Plugins are bundled with this app. Disabling hides their entry points and blocks new UI actions; already accepted operations and background jobs continue, and data and configuration are kept.')}}</p></header>
            <p v-if="pluginPreferenceError" class="host-action-error" role="alert">{{pluginPreferenceError}}</p>
            <article v-for="plugin in BUILTIN_PLUGINS" :key="plugin.id"><div><h4>{{tr(...plugin.label)}}</h4><p>{{tr(...plugin.description)}}</p><small v-if="plugin.id==='hackerone'&&!hackerOneEnabled">{{tr('还需要配置 HackerOne API 凭据','HackerOne API credentials are also required')}}</small></div><label><input type="checkbox" :checked="pluginEnabled[plugin.id]" :aria-label="tr(...plugin.label)" @change="onPluginToggle(plugin.id,$event)"/>{{pluginEnabled[plugin.id]?tr('已启用','Enabled'):tr('已停用','Disabled')}}</label></article>
          </section>

          <section v-else class="panel environment-card">
            <div class="panel-heading">
              <div><span class="eyebrow">RUNTIME CHECK</span><h3>{{tr('运行环境检测','Environment check')}}</h3><p>{{tr('检查随包原生工具与浏览器/AST需要的 Node.js；资产采集和 Native HTTP 不需要 Python、Redis 或 Docker。Docker 仅用于可选源码分析器，检测成功不代表工具已批准或已隔离。','Check bundled native tools and Node.js for browser/AST helpers. Asset collection and Native HTTP need neither Python, Redis nor Docker. Docker is only used by optional source analyzers; detection does not approve or isolate tools.')}}</p></div>
              <div class="hero-actions">
                <button class="button ghost compact" :disabled="environmentChecking||environmentInstalling" @click="checkEnvironment"><RefreshCw :size="14" :class="{spinning:environmentChecking}" /> {{environmentChecking?tr('检测中…','Checking…'):tr('检测','Check')}}</button>
                <button v-if="environment&&['macOS','Windows'].includes(environment.os)" class="button secondary compact" :disabled="environmentChecking||environmentInstalling" @click="installEnvironment"><RefreshCw v-if="environmentInstalling" :size="14" class="spinning" /> {{environmentInstalling?tr('安装中，请查看日志','Installing — see log'):environment.os==='Windows'?tr('Windows 自动安装','Install on Windows'):tr('Mac 自动安装','Install on Mac')}}</button>
              </div>
            </div>
            <div v-if="preparationStatus?.state==='installing'" class="environment-preparation-warning" role="status">
              {{tr('环境安装正在进行，扫描队列暂时锁定。请等待安装完成。','Environment installation is active; the scan queue is locked until it finishes.')}}
            </div>
            <div v-if="preparationStatus?.state==='requires_manual_recovery'" class="environment-preparation-warning" role="alert">
              <strong>{{tr('上次环境安装未正常结束，扫描队列保持锁定','Previous installation ended unexpectedly; scan queue remains locked')}}</strong>
              <p>{{tr('请在操作系统中核对 brew/winget/pip 等安装子进程确已停止；仅关闭应用或发现锁空闲并不能证明安全。确认后输入下方文字解除租约。此操作记录恢复事件，但不自动验证进程，也不替代管理员核对。','Check in the operating system that all brew/winget/pip installer processes have stopped. An unlocked file does not prove safety. Enter the confirmation below to release the lease. This records an event, but does not verify processes or replace operator review.')}}</p>
              <small>{{tr('租约','Lease')}} {{preparationStatus.owner}} · {{preparationStatus.createdAt}}</small>
              <label>{{tr('输入“已确认安装子进程停止”','Type “已确认安装子进程停止”')}}<input v-model="recoveryAcknowledgement" autocomplete="off" /></label>
              <button class="button danger compact" :disabled="recoveringPreparation||recoveryAcknowledgement!=='已确认安装子进程停止'" @click="recoverPreparation">{{recoveringPreparation?tr('恢复中…','Recovering…'):tr('人工解除环境锁定','Manually release environment lock')}}</button>
            </div>
            <div v-if="environment" class="environment-grid"><div><span>OS</span><strong>{{environment.os}} · {{environment.arch}}</strong></div><div><span>Node.js</span><strong :title="environment.node">{{environment.node}}</strong></div><div v-for="dep in environment.dependencies" :key="dep.name"><span>{{dep.name}} · {{dep.command}}</span><strong :class="dep.available?'env-ok':'env-bad'" :title="dep.detail">{{dep.available?'OK':dep.version==='unsupported_sandbox'?'未隔离':'缺失'}} · {{dep.version}}</strong><small v-if="!dep.available">{{dep.detail}}</small></div></div>
            <div v-else class="empty-state small">{{tr('点击检测查看依赖状态','Click Check to inspect dependencies')}}</div>
            <div v-if="environmentInstallState!=='idle'||environmentInstallLogs.length" ref="environmentInstallConsole" class="environment-install-console">
              <header>
                <div><TerminalSquare :size="15" /><strong>{{tr('安装实时输出','Live installation output')}}</strong></div>
                <span :class="environmentInstallState">{{environmentInstallState==='running'?tr('执行中','RUNNING'):environmentInstallState==='success'?tr('已完成','SUCCESS'):environmentInstallState==='error'?tr('失败','FAILED'):tr('待执行','IDLE')}}</span>
              </header>
              <p v-if="environmentLogDisconnected">{{tr('安装日志连接或补读失败，正在重试；不能据此判断安装完成。','Installation log subscription or replay failed; retrying. Logs do not confirm completion.')}}</p>
              <p v-if="environmentLogGap">{{tr('部分安装日志已超出持久保留窗口，视图存在缺口；请以安装结果和环境准备状态为准。','Some installation logs fell outside the retained journal; this view has a gap. Check the installation result and preparation status.')}}</p>
              <p v-if="environmentInstallEvicted">{{tr(`只显示最近 300 条，已移出视图 ${environmentInstallEvicted} 条。`,`Showing the latest 300 entries; ${environmentInstallEvicted} entries have left the view.`)}}</p>
              <pre><code v-for="line in environmentInstallLogs" :key="line.id" :class="line.stream"><time>{{line.time}}</time><b>[{{line.stage}}]</b> {{line.message}}</code><code v-if="environmentInstalling" class="pending">▌</code></pre>
              <p v-if="environmentInstallError">{{environmentInstallError}}</p>
            </div>
          </section>
        </template>
      </div>
    </main>

    <ProjectDialog v-if="projectDialog" :project="editProject" @close="projectDialog=false;editProject=undefined" @saved="projectSaved" />
    <ConfigDialog v-if="configDialog" :profile="editProfile" @close="configDialog=false;editProfile=undefined" @saved="profileSaved" />
    <AppSettingsDialog v-if="appSettingsDialog" :settings="appSettings" @close="appSettingsDialog=false" @saved="appSettingsSaved" @icon-changed="loadBrandIcon" />
    <ReleaseNotesDialog v-if="releaseNotesDialog" :version="packageInfo.version" @close="releaseNotesDialog=false" />
    <div class="toast-stack"><div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.type"><CheckCircle2 v-if="toast.type==='success'" :size="18" /><CircleAlert v-else-if="toast.type==='error'" :size="18" /><Bell v-else :size="18" /><span>{{toast.text}}</span><button @click="toasts=toasts.filter(t=>t.id!==toast.id)"><X :size="14" /></button></div></div>
  </div>
</template>

<style scoped>
.page-heading{min-width:0;max-width:40%}.page-heading>span,.page-heading h1{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.topbar-actions{min-width:0;flex-wrap:wrap}.host-action-error{color:var(--red);font-size:12px;line-height:1.6;overflow-wrap:anywhere}.builtin-plugin-manager{padding:22px}.builtin-plugin-manager header p,.builtin-plugin-manager article p{font-size:12px;color:var(--muted);line-height:1.7}.builtin-plugin-manager article{display:flex;justify-content:space-between;align-items:center;gap:20px;padding:18px 0;border-top:1px solid var(--line)}.builtin-plugin-manager h4{margin:0}.builtin-plugin-manager article label{display:flex;gap:8px;align-items:center;white-space:nowrap;font-size:12px}
.project-entry-actions { grid-template-columns: repeat(3, minmax(0, 1fr)); }
</style>
