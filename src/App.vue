<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Activity, Bell, BrainCircuit, Braces, CheckCircle2, ChevronDown, CircleAlert, ClipboardCheck, Clock3, Copy, Database,
  AppWindow, FileClock, FolderKanban, Globe2, Languages, LayoutDashboard, ListFilter, Menu, Palette, PlayCircle, Plus, RefreshCw, Search,
  Settings2, ShieldAlert, ShieldCheck, Server, SlidersHorizontal, TerminalSquare, Trash2, X,
} from "@lucide/vue";
import { api } from "./api";
import packageInfo from "../package.json";
import { useI18n } from "./i18n";
import { humanizeScanCheckpoint } from "./features/sentinel/presentation";
import NativeProcessLogView from "./features/sentinel/components/NativeProcessLogView.vue";
import { useRunnerLogPanel } from "./features/sentinel/execution/useRunnerLogPanel";
import { useAssetLogPanel } from "./features/assets/useAssetLogPanel";
import { useInstallLogPanel } from "./features/environment/useInstallLogPanel";
import type { AppSettings, AssetEvent, ConfigProfile, DashboardStats, EnvironmentPreparationStatus, EnvironmentReport, HackerOneEvent, InterruptedJob, JobProgressEvent, JobRun, Project, ProjectImpact, SentinelScan, SentinelScanAttempt, StartupStatus, ToastMessage, ViewKey } from "./types";
const AssetWorkspace = defineAsyncComponent(() => import("./components/AssetWorkspace.vue"));
const AssetOwnershipGate = defineAsyncComponent(() => import("./components/AssetOwnershipGate.vue"));
const ExposureSurface = defineAsyncComponent(() => import("./components/ExposureSurface.vue"));
import AppSettingsDialog from "./components/AppSettingsDialog.vue";
import ConfigDialog from "./components/ConfigDialog.vue";
const HackerOneBoard = defineAsyncComponent(() => import("./components/HackerOneBoard.vue"));
const SentinelBoard = defineAsyncComponent(() => import("./components/SentinelBoard.vue"));
import ProjectDialog from "./components/ProjectDialog.vue";
import InlineConfirm from "./components/InlineConfirm.vue";
import QueryPanel from "./components/QueryPanel.vue";
import ReleaseNotesDialog from "./components/ReleaseNotesDialog.vue";
import WorkerSettingsPanel from "./components/WorkerSettingsPanel.vue";
import defaultBrandIconUrl from "../src-tauri/icons/brand-icon.png";
import "./styles.css";
import "./probe-status.css";
import "./theme-enhancements.css";

const { locale, t, tr, setLocale } = useI18n();
type ModuleKey = "hackerone" | "asset" | "sentinel";
for(const suffix of ["view","module","project","theme","accent"]){
  const current=`oviraptor-${suffix}`;const legacy=`asset-atlas-${suffix}`;
  if(localStorage.getItem(current)===null&&localStorage.getItem(legacy)!==null)localStorage.setItem(current,localStorage.getItem(legacy)!);
  localStorage.removeItem(legacy);
}
const activeView = ref<ViewKey>((localStorage.getItem("oviraptor-view") as ViewKey) || "dashboard");
const activeModule = ref<ModuleKey>((localStorage.getItem("oviraptor-module") as ModuleKey) || (activeView.value === "hackerone" ? "hackerone" : activeView.value === "sentinel" ? "sentinel" : "asset"));
const projects = ref<Project[]>([]); const profiles = ref<ConfigProfile[]>([]); const runs = ref<JobRun[]>([]);
const events = ref<AssetEvent[]>([]); const stats = ref<DashboardStats>();
const runLiveMessages = ref<Record<number,string>>({});
const appSettings = ref<AppSettings>({ reminderDays: 7, customIcon: false, deduplicatedAssets: 0 });
const startup = ref<StartupStatus>({ reminderDays: 7, staleProjects: [], interruptedJobs: [] });
const selectedProjectId = ref<number | undefined>(Number(localStorage.getItem("oviraptor-project")) || undefined);
const { logs, loading: assetLogsLoading, readFailed: assetLogsFailed,
  connectionUnavailable: assetLogConnectionUnavailable, refresh: refreshAssetLogs } = useAssetLogPanel(
  () => selectedProjectId.value, () => activeModule.value === "asset" && activeView.value === "logs",
);
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
const configSection = ref<"profiles" | "workers" | "runtime">("profiles");
const deletingProfileId = ref<number>();
const sidebarCollapsed = ref(false); const globalSearch = ref(""); const loading = ref(true); const toasts = ref<ToastMessage[]>([]);
const deletingProjectId = ref<number>();
const projectImpactLoadingId = ref<number>();
const pendingProjectAction = ref<{ project: Project; impact: ProjectImpact; mode: "delete" | "archive" }>();
const storedTheme=localStorage.getItem("oviraptor-theme")||"cloud";
const themePreset=ref(storedTheme==="cloud"?"cloud":"codex");
if(storedTheme!==themePreset.value)localStorage.setItem("oviraptor-theme",themePreset.value);
const accentColor=ref(localStorage.getItem("oviraptor-accent")||"#2878ff");
const showTheme=ref(false);
const sentinelSearch = ref("");
const sentinelSection = ref<"overview"|"queue"|"results"|"fuse"|"validations"|"workbench"|"dialog"|"help">("dialog");
const sentinelResultView = ref<"summary"|"fingerprint"|"api"|"endpoints"|"vulnerabilities">("summary");
const sentinelWorkbenchMode = ref<"web"|"code"|"greybox"|"cicd"|"skills"|"traces">("code");
const sentinelMenu = ref("dialog");
const sentinelLogScans=ref<SentinelScan[]>([]); const hackerOneLogEvents=ref<HackerOneEvent[]>([]);
const sentinelLogScanId=ref("");
const sentinelLogAttempts=ref<SentinelScanAttempt[]>([]); const sentinelLogAttempt=ref(0); const sentinelLogFilter=ref(""); const sentinelLogPinned=ref(true);
const sentinelRunnerConsole=ref<HTMLElement>();
const { snapshot: sentinelRunnerLog, loading: sentinelRunnerLogLoading,
  readFailed: sentinelRunnerLogError, connectionUnavailable: sentinelLogConnectionUnavailable,
  refresh: refreshSentinelRunnerLog } = useRunnerLogPanel(
  () => ({ scanId: sentinelLogScanId.value, attempt: sentinelLogAttempt.value }),
  () => activeView.value === "logs" && activeModule.value === "sentinel",
);
watch(sentinelRunnerLog, async () => {
  await nextTick();
  const output=sentinelRunnerConsole.value;
  if(sentinelLogPinned.value && output) output.scrollTop=output.scrollHeight;
});
const sentinelAlerts=ref({fuse:0,vulnerabilities:0});
const brandIconUrl=ref(defaultBrandIconUrl);
let unlisten: UnlistenFn | undefined;
let unlistenTray: UnlistenFn | undefined;
let workerSyncTimer: ReturnType<typeof setInterval> | undefined;
let workerAutoSyncRunning=false;

const selectedProject = computed(() => projects.value.find(project => project.id === selectedProjectId.value));
const activeProfile = computed(() => profiles.value.find(profile=>profile.isDefault)||profiles.value[0]);
const hackerOneEnabled = computed(() => Boolean(activeProfile.value?.settings?.hackerOneUsername?.trim() && activeProfile.value?.settings?.hackerOneToken?.trim()));
const viewTitle = computed(() => {
  if (activeView.value === "sentinel") {
    const titles: Record<string, string> = {
      dialog: tr("调查对话", "Investigation dialog"),
      overview: tr("行动中心", "Investigation desk"),
      queue: tr("任务与成本", "Tasks & cost"),
      new_scan: tr("新调查", "New investigation"),
      urls: tr("这次结果", "This investigation"),
      skills: tr("知识与策略", "Knowledge & policy"),
    };
    return titles[sentinelMenu.value] || titles.overview;
  }
  return ({
    dashboard:t.value.dashboard, projects:t.value.projects, query:t.value.query, assets:t.value.assets, ownership:tr("归属门禁","Ownership gate"), exposure:tr("公开暴露面","Public exposure"),
    quarantine:t.value.quarantine, hackerone:"HackerOne SRC", sentinel:"安全总览", changes:t.value.changes, tasks:t.value.tasks, logs:t.value.logs, settings:t.value.settings,
  })[activeView.value];
});
const nav = computed(() => [
  { key:"dashboard" as ViewKey,label:t.value.dashboard,icon:LayoutDashboard },
  { key:"tasks" as ViewKey,label:t.value.tasks,icon:Activity,badge:stats.value?.runningJobs },
  { key:"assets" as ViewKey,label:t.value.assets,icon:Database },
  { key:"ownership" as ViewKey,label:tr("归属门禁","Ownership gate"),icon:ShieldCheck },
  { key:"exposure" as ViewKey,label:tr("公开暴露面","Public exposure"),icon:Globe2 },
  { key:"changes" as ViewKey,label:t.value.changes,icon:FileClock },
  { key:"quarantine" as ViewKey,label:t.value.quarantine,icon:ShieldAlert,badge:stats.value?.blockedCount },
  { key:"query" as ViewKey,label:t.value.query,icon:Plus,cta:true },
].filter(() => activeModule.value === "asset"));
const sentinelNav = computed(()=>[
  {key:"dialog",section:"dialog" as const,label:tr("调查对话","Investigation dialog"),icon:BrainCircuit},
  {key:"urls",section:"results" as const,result:"summary" as const,label:tr("这次结果","This investigation"),icon:Globe2},
  {key:"queue",section:"queue" as const,label:tr("任务与成本","Tasks & cost"),icon:ClipboardCheck,badge:sentinelAlerts.value.fuse},
  {key:"skills",section:"workbench" as const,workbench:"skills" as const,label:tr("知识与策略","Knowledge & policy"),icon:Braces},
]);

function navigate(view: ViewKey) { activeView.value=view; if(view==='hackerone') activeModule.value='hackerone'; if(view==='sentinel')activeModule.value='sentinel'; localStorage.setItem("oviraptor-view",view); localStorage.setItem("oviraptor-module",activeModule.value); window.setTimeout(()=>void refreshSecondary(),0); }
function navigateFromTray(destination: string) {
  if (destination === "assets") {
    switchModule("asset");
    navigate("assets");
  } else if (destination === "agent-tasks") {
    switchModule("sentinel");
    sentinelMenu.value = "queue";
    sentinelSection.value = "queue";
    navigate("sentinel");
  }
}
function switchModule(module: ModuleKey) { if(module==="hackerone"&&!hackerOneEnabled.value){notify("info",tr("请先在配置中心填写 HackerOne API identifier 和 token","Configure HackerOne API credentials first"));return} activeModule.value=module; localStorage.setItem("oviraptor-module",module); navigate(module === "hackerone" ? "hackerone" : module === "sentinel" ? "sentinel" : "dashboard"); }
function openSentinelSection(item:(typeof sentinelNav.value)[number]){sentinelMenu.value=item.key;sentinelSection.value=item.section;if("result" in item&&item.result)sentinelResultView.value=item.result;if("workbench" in item&&item.workbench)sentinelWorkbenchMode.value=item.workbench;navigate("sentinel")}
function selectProject(id?: number) { selectedProjectId.value=id; id ? localStorage.setItem("oviraptor-project",String(id)) : localStorage.removeItem("oviraptor-project"); void refreshProjectData(); }
function notify(type: "success"|"error"|"info", text: string) { const item={id:Date.now()+Math.random(),type,text}; toasts.value.push(item); setTimeout(()=>toasts.value=toasts.value.filter(t=>t.id!==item.id),5000); }
function sentinelErrorDetail(scan: SentinelScan) {
  const checkpoint = scan.currentCheckpoint || "";
  const marker = "报错细节：";
  const markerIndex = checkpoint.indexOf(marker);
  if (markerIndex >= 0) return humanizeScanCheckpoint(checkpoint.slice(markerIndex + marker.length), 800);
  return scan.status === "failed" ? humanizeScanCheckpoint(checkpoint, 800) : "";
}
function sentinelCheckpointSummary(scan: SentinelScan) {
  const checkpoint = scan.currentCheckpoint || tr("等待任务状态", "Waiting for status");
  const markerIndex = checkpoint.indexOf("；报错细节：");
  return humanizeScanCheckpoint(markerIndex >= 0 ? checkpoint.slice(0, markerIndex) : checkpoint, 260);
}
const RUNNER_LOG_STATUSES:Record<string,[string,string]>={
  ready:["已读取","Read"],
  not_created:["本轮日志尚未创建","Not created yet"],
  empty:["日志存在但为空","Empty log"],
  read_failed:["日志读取失败","Read failed"],
};
function runnerLogStatusLabel(){
  const status=sentinelRunnerLog.value?.status||"";
  const label=RUNNER_LOG_STATUSES[status];
  return label?tr(label[0],label[1]):tr("尚未读取","Not loaded yet");
}
const sentinelRunnerLogLines=computed(()=>{
  const lines=sentinelRunnerLog.value?.lines||[];
  const needle=sentinelLogFilter.value.trim().toLowerCase();
  return needle?lines.filter(line=>line.toLowerCase().includes(needle)):lines;
});
const sentinelLogDiagnostics=computed(()=>{
  const lines=sentinelRunnerLog.value?.lines||[];
  const snippets:string[]=[];
  for(const line of lines){
    if(/底层诊断|captureError|captureStatus|runtimeStopReason|cdp_|node_helper|浏览器采集阶段结束.*采集失败|部分侦察证据/.test(line)){
      const trimmed=line.replace(/^\[[^\]]+\]\s*/,"").trim();
      if(trimmed&&!snippets.includes(trimmed))snippets.push(trimmed);
    }
  }
  return snippets.slice(-6);
});
function selectSentinelLogScan(scanId:string){
  sentinelLogScanId.value=scanId;
  // Clear before the next read so a slow response for the previous task can
  // never be mistaken for the newly selected one.
  sentinelRunnerLog.value=undefined; sentinelRunnerLogError.value=false;
  sentinelLogAttempt.value=0; sentinelLogAttempts.value=[];
  void loadSentinelLogAttempts(scanId);
  void refreshSentinelRunnerLog();
}
function selectSentinelLogAttempt(value:number){ sentinelLogAttempt.value=value; sentinelRunnerLog.value=undefined; void refreshSentinelRunnerLog(); }
async function loadSentinelLogAttempts(scanId:string){
  if(!scanId){sentinelLogAttempts.value=[];return;}
  try{
    const rows=await api.listSentinelScanAttempts(scanId);
    if(scanId===sentinelLogScanId.value)sentinelLogAttempts.value=rows;
  }catch{ if(scanId===sentinelLogScanId.value)sentinelLogAttempts.value=[]; }
}
function syncSentinelLogSelection(){
  const selected=sentinelLogScans.value.find(scan=>scan.id===sentinelLogScanId.value);
  if(selected)return;
  const preferred=sentinelLogScans.value.find(scan=>["scanning","pausing","queued"].includes(scan.status))||sentinelLogScans.value[0];
  selectSentinelLogScan(preferred?.id||"");
}
function openSentinelRunnerLog(scanId:string,attempt:number){
  if(!scanId)return;
  activeModule.value="sentinel";
  navigate("logs");
  sentinelLogScanId.value=scanId;
  sentinelRunnerLog.value=undefined; sentinelRunnerLogError.value=false;
  sentinelLogAttempt.value=attempt>0?attempt:0;
  void loadSentinelLogAttempts(scanId);
  void refreshSentinelRunnerLog();
}
function onSentinelLogScroll(){
  const output=sentinelRunnerConsole.value;
  if(!output)return;
  sentinelLogPinned.value=output.scrollHeight-output.scrollTop-output.clientHeight<=24;
}
async function copySentinelRunnerLog(){
  const lines=sentinelRunnerLog.value?.lines||[];
  if(!lines.length){ notify("info",tr("当前没有可复制的日志","No log lines to copy")); return; }
  try{
    await navigator.clipboard.writeText(lines.join("\n"));
    notify("success",tr("已复制脱敏后的日志","Redacted log copied"));
  }catch(error){ notify("error",`${tr("复制失败","Copy failed")}: ${String(error)}`); }
}
function setTheme(preset:string){themePreset.value=preset;localStorage.setItem("oviraptor-theme",preset);showTheme.value=false}
function saveAccent(){localStorage.setItem("oviraptor-accent",accentColor.value)}
let refreshGeneration=0;
async function refreshSecondary(generation=refreshGeneration) {
  const view=activeView.value;
  const module=activeModule.value;
  const projectId=selectedProjectId.value;
  const requests:Promise<void>[]=[];
  if(module==="asset"&&["dashboard","tasks"].includes(view)) requests.push(api.listRuns(projectId,100).then(value=>{if(generation===refreshGeneration)runs.value=value;}));
  if(module==="asset"&&["dashboard","changes"].includes(view)) requests.push(api.listEvents(projectId,undefined,200).then(value=>{if(generation===refreshGeneration)events.value=value;}));
  if(view==="logs"&&module==="asset") requests.push(refreshAssetLogs());
  if(view==="logs"&&module==="sentinel") requests.push(api.listSentinelScans(projectId,300).then(value=>{if(generation===refreshGeneration){sentinelLogScans.value=value;syncSentinelLogSelection();void refreshSentinelRunnerLog();}}));
  if(view==="logs"&&module==="hackerone"&&hackerOneEnabled.value) requests.push(api.listHackerOneEvents(300).then(value=>{if(generation===refreshGeneration)hackerOneLogEvents.value=value;}));
  await Promise.allSettled(requests);
}
async function refreshProjectData(){
  const generation=++refreshGeneration;
  try{stats.value=await api.dashboardStats(selectedProjectId.value);await refreshSecondary(generation);}
  catch(error){notify("error",String(error));}
}
async function refresh(waitForSecondary: boolean | Event=false) {
  const shouldWait=waitForSecondary===true;
  const generation=++refreshGeneration;
  try {
    [projects.value,profiles.value,stats.value,appSettings.value] = await Promise.all([
      api.listProjects(),api.listProfiles(),api.dashboardStats(selectedProjectId.value),api.getAppSettings(),
    ]);
    const secondary=refreshSecondary(generation);
    if(shouldWait)await secondary;
    else void secondary;
    if(hackerOneEnabled.value)void api.listHackerOneEvents(1).then(value=>h1HasChanges.value=value.length>0);
    if(activeModule.value==="hackerone"&&!hackerOneEnabled.value){activeModule.value="asset";activeView.value="dashboard";localStorage.setItem("oviraptor-module","asset");localStorage.setItem("oviraptor-view","dashboard")}
    if (selectedProjectId.value && !projects.value.some(p=>p.id===selectedProjectId.value)){selectedProjectId.value=undefined;localStorage.removeItem("oviraptor-project");}
  } catch(error){ notify("error",String(error)); }
  finally { loading.value=false; }
}
async function projectSaved(id:number){ projectDialog.value=false;editProject.value=undefined;await refresh();selectProject(id); }
async function profileSaved(){ configDialog.value=false;editProfile.value=undefined;await refresh(); }
function openProject(project?:Project){ editProject.value=project;projectDialog.value=true; }
function openProjectWorkspace(project: Project, module: "asset" | "sentinel") {
  selectProject(project.id);
  if (module === "sentinel") {
    switchModule("sentinel");
    sentinelMenu.value = "dialog";
    sentinelSection.value = "dialog";
  } else {
    switchModule("asset");
  }
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
function stageLabel(stage:string){ const zh=({queued:"等待",collect:"采集",import:"入库",refine:"分层",probe:"探测",reprobe:"存活复测",completed:"完成",failed:"失败",cancelled:"取消",interrupted:"已中断",restarted:"已继续"} as Record<string,string>)[stage]; const en=({queued:"Queued",collect:"Collect",import:"Import",refine:"Refine",probe:"Probe",reprobe:"Re-probe",completed:"Completed",failed:"Failed",cancelled:"Cancelled",interrupted:"Interrupted",restarted:"Restarted"} as Record<string,string>)[stage]; return tr(zh||stage,en||stage); }
function statusTone(status:string){ return status==="completed"?"success":["failed","interrupted"].includes(status)?"danger":["running","queued","cancel_requested"].includes(status)?"running":"muted"; }
function eventLabel(type:string){ const zh=({new:"新增",changed:"变化",missing:"未发现",decision:"人工结论",archived:"归档",restored:"恢复"} as Record<string,string>)[type]; const en=({new:"New",changed:"Changed",missing:"Not seen",decision:"Decision",archived:"Archived",restored:"Restored"} as Record<string,string>)[type]; return tr(zh||type,en||type); }
async function quickStartProject(projectId:number, profileId?:number, name?:string, pipeline="full") {
  const running=runs.value.find(run=>run.projectId===projectId && ["running","queued","cancel_requested"].includes(run.status));
  if(running){ notify("info",tr(`项目已有运行中的任务 #${running.id}`,`Project already has running job #${running.id}`)); navigate("tasks"); return; }
  const profile=profiles.value.find(item=>item.id===profileId) || profiles.value.find(item=>item.isDefault) || profiles.value[0];
  if(!profile){ notify("error",tr("请先创建执行配置","Create an execution profile first")); return; }
  try {
    const project=projects.value.find(item=>item.id===projectId);
    const runId=await api.startJob(projectId,profile.id,name||`${project?.name||tr("项目","Project")} · ${tr("快速任务","Quick run")}`,pipeline);
    notify("success",tr(`任务 #${runId} 已加入队列`,`Job #${runId} queued`));
    navigate("tasks");
    await refresh();
  } catch(error){ notify("error",String(error)); }
}
async function resumeInterrupted(job:InterruptedJob){
  try {
    if(job.pipeline==="reprobe"){
      await api.resumeJob(job.runId);
      notify("success",tr(`任务 #${job.runId} 已从断点继续`,`Job #${job.runId} resumed from checkpoint`));
      navigate("tasks");
      await refresh();
    } else {
      await api.acknowledgeInterruptedRun(job.runId);
      await quickStartProject(job.projectId,job.profileId,`${job.name} · ${tr("继续","Resume")}`,job.pipeline||"full");
    }
    startup.value.interruptedJobs=startup.value.interruptedJobs.filter(item=>item.runId!==job.runId);
  } catch(error){ notify("error",String(error)); }
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
function doGlobalSearch(){ if(activeModule.value==="sentinel"){sentinelSection.value="results";navigate("sentinel");return} if(!globalSearch.value.trim())return; navigate("assets"); }
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
  void api.startupStatus().then(value=>startup.value=value).catch(error=>notify("error",String(error)));
  await refresh();
  void loadBrandIcon();
  void autoSyncWorkers();
  workerSyncTimer=setInterval(autoSyncWorkers,5*60*1000);
  unlisten=await listen<JobProgressEvent>("job-progress",event=>{
    const progress=event.payload; const run=runs.value.find(item=>item.id===progress.runId);
    if(run){run.status=progress.status;run.stage=progress.stage;run.progress=progress.progress;}
    runLiveMessages.value={...runLiveMessages.value,[progress.runId]:progress.message};
    if(["completed","failed","cancelled"].includes(progress.status)){ notify(progress.status==="completed"?"success":"error",progress.message);refresh(); }
  });
  unlistenTray=await listen<string>("tray-navigate",event=>navigateFromTray(event.payload));
});
onUnmounted(()=>{unlisten?.();unlistenTray?.();if(workerSyncTimer)clearInterval(workerSyncTimer)});
</script>

<template>
  <div class="app-shell" :class="[{ 'sidebar-collapsed': sidebarCollapsed },`theme-${themePreset}`]" :style="{'--blue':accentColor}">
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark"><img v-if="brandIconUrl" :src="brandIconUrl" alt="Oviraptor" /></div><div class="brand-copy"><strong>{{t.appName}}</strong><span>Eating • Taking</span></div></div>
      <div class="module-switcher" aria-label="Product modules">
        <button v-if="hackerOneEnabled" :class="{active:activeModule==='hackerone'}" @click="switchModule('hackerone')"><span>H1</span><b>HackerOne</b></button>
        <button :class="{active:activeModule==='asset'}" @click="switchModule('asset')"><span>◈</span><b>Asset</b></button>
        <button :class="{active:activeModule==='sentinel'}" @click="switchModule('sentinel')"><span>◈</span><b>Nest</b></button>
      </div>
      <button class="sidebar-toggle" @click="sidebarCollapsed=!sidebarCollapsed"><Menu :size="17" /></button>
      <nav class="nav-list">
        <button v-for="item in nav" :key="item.key" :class="{active:activeView===item.key,accent:'cta' in item&&item.cta}" @click="navigate(item.key)">
          <component :is="item.icon" :size="18" /><span>{{item.label}}</span><em v-if="item.badge">{{item.badge}}</em>
        </button>
        <template v-if="activeModule==='sentinel'">
          <button v-for="item in sentinelNav" :key="item.key" :class="{active:sentinelMenu===item.key,accent:'cta' in item&&item.cta}" @click="openSentinelSection(item)"><component :is="item.icon" :size="18"/><span>{{item.label}}</span><em v-if="'badge' in item&&item.badge">{{item.badge}}</em></button>
        </template>
      </nav>
      <div class="sidebar-fixed-links">
        <button :class="{active:activeView==='logs'}" @click="navigate('logs')"><TerminalSquare :size="17" /><span>{{t.logs}}</span></button>
        <button :class="{active:activeView==='settings'}" @click="navigate('settings')"><Settings2 :size="17" /><span>{{t.settings}}</span></button>
      </div>
      <div class="sidebar-project">
        <span class="sidebar-label">CURRENT WORKSPACE</span>
        <button class="current-project" :title="tr('管理项目与范围','Manage projects and scope')" @click="navigate('projects')"><span class="project-dot"></span><span>{{selectedProject?.name||t.allProjects}}</span><ChevronDown :size="14" /></button>
        <button class="sidebar-create-workspace" @click="openProject()"><Plus :size="15" />{{tr('新建工作空间','New workspace')}}</button>
      </div>
      <footer class="sidebar-footer"><button :title="tr('查看更新说明','View release notes')" @click="releaseNotesDialog=true">v{{packageInfo.version}}</button><ShieldCheck :size="16" /><span>Local-first · SQLite</span></footer>
    </aside>

    <main class="main-area">
      <header class="topbar">
        <div class="page-heading"><span>{{selectedProject?.name||t.allProjects}}</span><h1>{{viewTitle}}</h1></div>
        <div class="topbar-actions">
          <div v-if="activeModule==='asset'&&!['logs','settings'].includes(activeView)" class="global-search"><Search :size="16" /><input v-model="globalSearch" :placeholder="tr('查询资产','Search assets')" @keyup.enter="doGlobalSearch" /></div>
          <div v-else-if="activeView==='sentinel'" class="global-search sentinel-global-search"><Search :size="16" /><input v-model="sentinelSearch" :placeholder="tr('搜索公司、URL、源码或任务','Search company, URL, source or task')" @keyup.enter="doGlobalSearch" /></div>
          <div class="project-switcher-group"><select class="project-switcher" :value="selectedProjectId" @change="selectProject(Number(($event.target as HTMLSelectElement).value)||undefined)"><option value="">{{t.allProjects}}</option><option v-for="project in projects" :key="project.id" :value="project.id">{{project.name}}{{project.status==='archived'?tr('（已归档）',' (archived)'):''}}</option></select><button class="workspace-create-cta" :title="tr('新建公共工作空间','Create shared workspace')" @click="openProject()"><Plus :size="16" /><span>{{tr('新建工作空间','New workspace')}}</span></button></div>
          <button class="top-icon" @click="setLocale(locale==='zh'?'en':'zh')"><Languages :size="18" /><span>{{locale==='zh'?'EN':'中'}}</span></button>
          <div class="theme-picker"><button class="top-icon" :title="tr('主题与强调色','Theme and accent')" @click="showTheme=!showTheme"><Palette :size="18" /></button><div v-if="showTheme" class="theme-popover panel"><strong>{{tr('界面主题','Interface theme')}}</strong><button v-for="item in [['cloud',tr('云白','Cloud')],['codex',tr('Codex','Codex')]]" :key="item[0]" :class="{active:themePreset===item[0]}" @click="setTheme(item[0])">{{item[1]}}</button><label><span>{{tr('自定义强调色','Custom accent')}}</span><input v-model="accentColor" type="color" @change="saveAccent" /></label></div></div>
          <button class="top-icon" :title="tr('查看任务与 Scope 变化','View task and scope changes')" @click="navigate(h1HasChanges?'hackerone':'tasks')"><Bell :size="18" /><i v-if="stats?.runningJobs||h1HasChanges"></i></button>
        </div>
      </header>

      <div class="content-area">
        <section v-if="!loading&&startupNotice&&(startup.interruptedJobs.length||startup.staleProjects.length)" class="startup-alert panel">
          <div class="startup-alert-icon"><CircleAlert :size="20" /></div>
          <div class="startup-alert-copy">
            <strong>{{startup.interruptedJobs.length?tr(`发现 ${startup.interruptedJobs.length} 个中断任务`,`Found ${startup.interruptedJobs.length} interrupted jobs`):tr('项目更新提醒','Project update reminder')}}</strong>
            <span>{{tr('自动更新需要电脑和应用保持运行；关闭窗口后应用会留在 macOS 状态栏。','Automatic updates require the computer and app to stay running; closing the window keeps the app in the macOS menu bar.')}}</span>
            <div v-if="startup.interruptedJobs.length" class="startup-items"><button v-for="job in startup.interruptedJobs.slice(0,4)" :key="job.runId" @click="resumeInterrupted(job)"><PlayCircle :size="14" /> {{job.projectName}} · {{tr('继续更新','Resume')}}</button></div>
            <div v-if="startup.staleProjects.length" class="startup-items"><button v-for="project in startup.staleProjects.slice(0,4)" :key="project.projectId" @click="quickStartProject(project.projectId)"><RefreshCw :size="14" /> {{project.projectName}} · {{project.daysSinceUpdate==null?tr('尚未更新','Never updated'):tr(`${project.daysSinceUpdate} 天未更新`,`${project.daysSinceUpdate} days old`)}}</button></div>
          </div>
          <button class="icon-button subtle" @click="startupNotice=false"><X :size="15" /></button>
        </section>
        <div v-if="loading" class="app-loading"><div class="loader-ring"></div><span>{{tr('正在初始化本地资产数据库…','Initializing local asset database…')}}</span></div>

        <template v-else-if="activeView==='dashboard'">
          <div class="hero-row">
            <div><span class="eyebrow">OVERVIEW</span><h2>{{selectedProject?tr(`${selectedProject.name} 概览`,`${selectedProject.name} overview`):tr('所有项目资产概览','All-project asset overview')}}</h2><p>{{tr('数据保存在本地 SQLite；仅变化事件进入历史，不重复保存完整镜像。','Data stays in local SQLite; history stores changes instead of duplicate full snapshots.')}}</p></div>
            <div class="hero-actions"><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button><button v-if="selectedProject?.status==='archived'" class="button warning" @click="toggleArchive(selectedProject)">{{tr('恢复工作空间后继续','Restore workspace to continue')}}</button><button v-else-if="selectedProjectId" class="button secondary" @click="quickStartProject(selectedProjectId,undefined,undefined,'reprobe')"><RefreshCw :size="16" /> {{tr('复测现有资产','Re-probe existing')}}</button><button v-if="selectedProjectId&&selectedProject?.status!=='archived'" class="button secondary" @click="quickStartProject(selectedProjectId)"><RefreshCw :size="16" /> {{tr('重新采集并探测','Collect and probe')}}</button><button class="button primary" @click="navigate('query')"><Plus :size="16" /> {{t.newQuery}}</button></div>
          </div>
          <div class="stats-grid">
            <article class="stat-card"><div class="stat-icon blue"><Database :size="20" /></div><span>{{tr('资产总数','Total assets')}}</span><strong>{{(stats?.assetCount||0).toLocaleString()}}</strong><small>{{tr(`${stats?.projectCount||0} 个活跃项目`,`${stats?.projectCount||0} active projects`)}}</small></article>
            <article class="stat-card"><div class="stat-icon green"><CheckCircle2 :size="20" /></div><span>{{tr('浏览器可访问','Browser accessible')}}</span><strong>{{(stats?.aliveCount||0).toLocaleString()}}</strong><small>{{tr('不包含仅 TCP 端口存活；旧数据需复测','Excludes TCP-only; re-probe legacy data')}}</small></article>
            <article class="stat-card"><div class="stat-icon violet"><ListFilter :size="20" /></div><span>{{tr('待人工确认','Pending review')}}</span><strong>{{(stats?.pendingCount||0).toLocaleString()}}</strong><small>{{tr('P1/P2/P3 复核队列','P1/P2/P3 review queue')}}</small></article>
            <button class="stat-card stat-button" @click="navigate('quarantine')"><div class="stat-icon amber"><CircleAlert :size="20" /></div><span>{{tr('内容隔离','Content blocked')}}</span><strong>{{(stats?.blockedCount||0).toLocaleString()}}</strong><small>{{tr('点击进入隔离区查看','Open quarantine for review')}}</small></button>
          </div>
          <div class="dashboard-grid">
            <section class="panel recent-changes"><div class="panel-heading"><div><span class="eyebrow">CHANGE FEED</span><h3>{{tr('最近变化','Recent changes')}}</h3></div><button class="text-button" @click="navigate('changes')">{{tr('查看全部','View all')}}</button></div>
              <div v-if="events.length" class="change-list"><div v-for="event in events.slice(0,8)" :key="event.id" class="change-row"><span class="change-type" :class="`event-${event.eventType}`">{{eventLabel(event.eventType)}}</span><div><strong>{{event.company||event.host||event.assetKey}}</strong><span>{{event.summary}}</span></div><time>{{event.createdAt}}</time></div></div><div v-else class="empty-state small">{{tr('完成首次任务后，这里会显示新增和变化资产','New and changed assets appear here after the first job')}}</div>
            </section>
            <section class="panel recent-jobs"><div class="panel-heading"><div><span class="eyebrow">JOBS</span><h3>{{tr('任务状态','Job status')}}</h3></div><button class="text-button" @click="navigate('tasks')">{{tr('任务中心','Job center')}}</button></div>
              <div v-if="runs.length" class="job-mini-list"><div v-for="run in runs.slice(0,6)" :key="run.id" class="job-mini"><div class="job-mini-head"><span class="status-dot" :class="statusTone(run.status)"></span><strong>{{run.name}}</strong><em>{{stageLabel(run.stage)}}</em></div><div class="progress-track"><i :style="{width:`${run.progress}%`}"></i></div><small>{{run.projectName}} · {{run.createdAt}}</small></div></div><div v-else class="empty-state small">{{tr('暂无运行记录','No job history')}}</div>
            </section>
          </div>
        </template>

        <template v-else-if="activeView==='projects'">
          <div class="section-toolbar"><div><span class="eyebrow">SHARED WORKSPACES</span><h2>{{tr('项目与范围','Projects and scope')}}</h2><p>{{tr('一个工作空间统一承载 Asset 范围、Nest 扫描、证据、漏洞结论和知识沉淀。','One workspace owns Asset scope, Nest scans, evidence, conclusions, and learned knowledge.')}}</p></div><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('新建工作空间','New workspace')}}</button></div>
          <div v-if="projects.length" class="project-grid shared-project-grid">
            <article v-for="project in projects" :key="project.id" class="project-card shared-project-card" :class="{selected:selectedProjectId===project.id,archived:project.status==='archived'}">
              <header><div class="project-icon"><FolderKanban :size="20" /></div><span class="state-pill">{{project.status==='active'?tr('活跃','Active'):tr('已归档','Archived')}}</span></header>
              <h3>{{project.name}}</h3><p>{{project.description||tr('暂无范围与授权说明','No scope or authorization note')}}</p>
              <div class="project-metrics shared-project-metrics"><span><strong>{{project.assetCount.toLocaleString()}}</strong>{{tr('资产','Assets')}}</span><span><strong>{{project.scanCount.toLocaleString()}}</strong>Nest</span><span><strong>{{project.vulnerabilityCount.toLocaleString()}}</strong>{{tr('漏洞记录（含历史）','Finding records (incl. history)')}}</span><span><strong>{{project.activeFuseCount.toLocaleString()}}</strong>{{tr('待处置','Stopped')}}</span></div>
              <div class="project-entry-actions"><button class="button ghost compact" @click="openProjectWorkspace(project,'asset')"><Database :size="14" />Asset</button><button class="button secondary compact" @click="openProjectWorkspace(project,'sentinel')"><ShieldCheck :size="14" />Nest</button></div>
              <footer><span>{{tr('最近活动','Last activity')}} {{projectActivity(project)}}</span><div><button class="text-button" @click="openProject(project)">{{tr('编辑','Edit')}}</button><button class="text-button" @click="toggleArchive(project)">{{project.status==='active'?tr('归档','Archive'):tr('恢复','Restore')}}</button><button class="text-button danger-text" :disabled="deletingProjectId!==undefined||projectImpactLoadingId!==undefined" :aria-busy="projectImpactLoadingId===project.id" @click="removeProject(project)">{{projectImpactLoadingId===project.id?tr('检查中…','Checking…'):tr('删除','Delete')}}</button></div></footer>
              <InlineConfirm v-if="pendingProjectAction?.project.id===project.id" :title="pendingProjectAction.mode==='archive'?tr('该工作空间有关联数据，改为归档？','This workspace has linked data. Archive it?'):tr('删除这个空工作空间？','Delete this empty workspace?')" :detail="pendingProjectAction.mode==='archive'?`${projectImpactText(pendingProjectAction.impact)}。归档后禁止创建新任务，但历史和结论继续可查。`:tr('它没有资产、扫描、证据或知识记录，删除后无法恢复。','It has no assets, scans, evidence, or knowledge and cannot be recovered.')" :confirm-text="pendingProjectAction.mode==='archive'?tr('归档并保留历史','Archive and preserve history'):tr('确认删除','Delete')" :busy-text="pendingProjectAction.mode==='archive'?tr('归档中…','Archiving…'):tr('删除中…','Deleting…')" :tone="pendingProjectAction.mode==='archive'?'warning':'danger'" :busy="deletingProjectId===project.id" @cancel="cancelProjectAction" @confirm="confirmProjectAction" />
            </article>
          </div>
          <div v-else class="first-run panel"><div class="first-run-icon"><FolderKanban :size="28" /></div><h2>{{tr('创建第一个公共工作空间','Create your first shared workspace')}}</h2><p>{{tr('无需先采集资产；也可以直接创建 Nest Web、代码审计或灰盒任务。','Asset collection is optional; you can start directly with Nest web, code, or grey-box tasks.')}}</p><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('创建工作空间','Create workspace')}}</button></div>
        </template>

        <QueryPanel v-else-if="activeView==='query'" :projects="projects" :profiles="profiles" :selected-project-id="selectedProjectId" @create-project="openProject()" @project-change="selectProject" @started="navigate('tasks')" @notify="notify" />
        <AssetWorkspace v-else-if="activeView==='assets'" :projects="projects" :selected-project-id="selectedProjectId" :initial-search="globalSearch" @reprobe="quickStartProject($event,undefined,undefined,'reprobe')" @notify="notify" />
        <AssetOwnershipGate v-else-if="activeView==='ownership'" :projects="projects" :selected-project-id="selectedProjectId" @notify="notify" />
        <ExposureSurface v-else-if="activeView==='exposure'" :projects="projects" :selected-project-id="selectedProjectId" @notify="notify" />

        <template v-else-if="activeView==='quarantine'">
          <div class="section-toolbar"><div><span class="eyebrow">QUARANTINE</span><h2>{{tr('内容隔离区','Content quarantine')}}</h2><p>{{tr('集中查看自动规则隔离的赌博、色情和自定义命中；数据不会被物理删除。','Review gambling, adult and custom-rule matches; quarantined data is never physically deleted.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <AssetWorkspace :projects="projects" :selected-project-id="selectedProjectId" quarantine-only @notify="notify" />
        </template>

        <HackerOneBoard v-else-if="activeView==='hackerone'" :projects="projects" :profiles="profiles" @notify="notify" />
        <div v-else-if="activeView==='sentinel'" class="sentinel-persistent-slot" aria-hidden="true"></div>

        <template v-else-if="activeView==='changes'">
          <div class="section-toolbar"><div><span class="eyebrow">DIFF & HISTORY</span><h2>{{tr('变化事件','Change events')}}</h2><p>{{tr('只记录新增、关键字段变化、未再次发现和人工操作，不重复保存完整快照。','Records new assets, key changes, assets not seen again and manual actions without duplicate full snapshots.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <section class="panel list-panel"><div class="event-table-head"><span>{{tr('类型','Type')}}</span><span>{{tr('资产','Asset')}}</span><span>{{tr('变化说明','Change')}}</span><span>{{tr('任务','Job')}}</span><span>{{tr('时间','Time')}}</span></div><div v-for="event in events" :key="event.id" class="event-table-row"><span><em class="change-type" :class="`event-${event.eventType}`">{{eventLabel(event.eventType)}}</em></span><span><strong>{{event.company||'—'}}</strong><small>{{event.host||event.assetKey}}</small></span><span>{{event.summary}}</span><span>#{{event.runId||'—'}}</span><time>{{event.createdAt}}</time></div><div v-if="!events.length" class="empty-state">{{tr('暂无变化事件','No change events')}}</div></section>
        </template>

        <template v-else-if="activeView==='tasks'">
          <div class="section-toolbar"><div><span class="eyebrow">JOB CENTER</span><h2>{{tr('任务中心','Job center')}}</h2><p>{{tr('长时间任务可独立运行、查看结构化日志并安全取消。','Long jobs run independently with structured logs and safe cancellation.')}}</p></div><div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button><button class="button primary" @click="navigate('query')"><Plus :size="16" /> {{tr('新建任务','New job')}}</button></div></div>
          <div class="run-list"><article v-for="run in runs" :key="run.id" class="run-card panel"><div class="run-status-mark" :class="statusTone(run.status)"><component :is="run.status==='completed'?CheckCircle2:['failed','interrupted'].includes(run.status)?CircleAlert:Clock3" :size="20" /></div><div class="run-main"><div class="run-title"><strong>{{run.name}}</strong><span class="run-id">#{{run.id}}</span><span class="run-stage">{{stageLabel(run.stage)}}</span></div><p>{{run.projectName}} · {{tr('创建于','created')}} {{run.createdAt}}</p><div class="run-progress"><div class="progress-track"><i :style="{width:`${run.progress}%`}"></i></div><span>{{Math.round(run.progress)}}%</span></div><small v-if="runLiveMessages[run.id]&&['running','queued','cancel_requested'].includes(run.status)" class="run-live-message">{{runLiveMessages[run.id]}}</small><small v-if="run.error" class="run-error">{{run.error}}</small></div><div class="run-side"><span class="status-chip" :class="statusTone(run.status)">{{stageLabel(run.status)}}</span><button v-if="run.status==='interrupted'||(run.pipeline==='reprobe'&&['failed','cancelled'].includes(run.status))" class="button secondary compact" @click="resumeInterrupted({runId:run.id,projectId:run.projectId,projectName:run.projectName,profileId:run.profileId,name:run.name,pipeline:run.pipeline,createdAt:run.createdAt})">{{tr('从断点继续','Resume checkpoint')}}</button><button v-if="['running','queued','cancel_requested'].includes(run.status)" class="button danger compact" @click="api.cancelJob(run.id).then(()=>refresh()).catch(e=>notify('error',String(e)))">{{tr('取消','Cancel')}}</button><button class="button ghost compact" @click="navigate('logs')">{{tr('日志','Logs')}}</button></div></article><div v-if="!runs.length" class="empty-state panel">{{tr('暂无任务','No jobs')}}</div></div>
        </template>

        <template v-else-if="activeView==='logs'">
          <div class="section-toolbar"><div><span class="eyebrow">AUDIT TRAIL · {{activeModule.toUpperCase()}}</span><h2>{{activeModule==='sentinel'?tr('扫描记录','Scan activity'):activeModule==='hackerone'?tr('HackerOne 同步记录','HackerOne sync activity'):tr('资产采集与操作日志','Asset collection activity')}}</h2><p>{{activeModule==='sentinel'?tr('只显示扫描状态、任务类型、Token 和检查点，不在 URL 概要中堆叠原始日志。','Shows scan status, type, tokens and checkpoints without dumping raw logs into URL summaries.'):tr('当前模块的运行记录；敏感配置不会在界面显示。','Activity for the current module; secrets are not shown.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <section v-if="activeModule==='sentinel'" class="panel module-activity-list"><article v-for="scan in sentinelLogScans" :key="scan.id"><span class="run-status-mark" :class="statusTone(scan.status)"><Activity :size="18"/></span><div><strong>{{scan.taskName||scan.projectName}}</strong><small>{{scan.scanType||'web'}} · {{scan.projectName}} · {{scan.id}}</small><p>{{sentinelCheckpointSummary(scan)}}</p><details v-if="sentinelErrorDetail(scan)" class="activity-error-detail"><summary><CircleAlert :size="13"/>{{tr('报错细节','Error details')}}</summary><pre>{{sentinelErrorDetail(scan)}}</pre></details></div><div class="activity-metric"><b>{{scan.totalTokens.toLocaleString()}}</b><span>Token</span><time>{{scan.updatedAt}}</time></div></article><div v-if="!sentinelLogScans.length" class="empty-state">{{tr('暂无扫描记录','No scan activity')}}</div></section>
          <section v-else-if="activeModule==='hackerone'" class="panel module-activity-list"><article v-for="event in hackerOneLogEvents" :key="event.id"><span class="run-status-mark success"><ShieldCheck :size="18"/></span><div><strong>{{event.programHandle}}</strong><small>{{event.eventType}}</small><p>{{event.summary}}</p></div><time>{{event.createdAt}}</time></article><div v-if="!hackerOneLogEvents.length" class="empty-state">{{tr('暂无 HackerOne 同步记录','No HackerOne sync activity')}}</div></section>
          <section v-else class="panel log-panel">
            <p v-if="assetLogConnectionUnavailable" role="status">{{tr('实时日志订阅暂不可用，正在补查并重连','Live log subscription unavailable; reconciling and reconnecting')}}</p>
            <p v-if="assetLogsFailed" role="alert">{{tr('日志读取失败，保留上次快照并等待重试','Log read failed; keeping the last snapshot until retry')}}</p>
            <div v-for="log in logs" :key="log.id" class="log-row"><time>{{log.createdAt}}</time><span class="log-level" :class="`log-${log.level}`">{{log.level}}</span><span class="log-stage">{{log.stage||'system'}}</span><code>{{log.message}}</code><em v-if="log.runId">#{{log.runId}}</em></div>
            <div v-if="!logs.length&&!assetLogsFailed" class="empty-state">{{assetLogsLoading?tr('读取中','Reading'):tr('暂无日志','No logs')}}</div>
          </section>
        </template>

        <template v-else-if="activeView==='settings'">
          <div class="section-toolbar"><div><span class="eyebrow">CONFIGURATION CENTER</span><h2>{{tr('配置中心','Configuration center')}}</h2><p>{{tr('运行方案、远程 Worker 和本机环境分区管理，避免创建无内容的空配置。','Manage runtime profiles, remote Workers and the local environment without creating empty profiles.')}}</p></div><button class="button ghost" @click="appSettingsDialog=true"><AppWindow :size="16" /> {{tr('应用设置','App settings')}}</button></div>
          <nav class="config-center-nav">
            <button :class="{active:configSection==='profiles'}" @click="configSection='profiles'"><SlidersHorizontal :size="18"/><span><b>{{tr('运行方案','Runtime profiles')}}</b><small>{{tr('账号、模型与扫描参数','Accounts, models and scan policy')}}</small></span></button>
            <button :class="{active:configSection==='workers'}" @click="configSection='workers'"><Server :size="18"/><span><b>Worker {{tr('节点','nodes')}}</b><small>{{tr('Intel Mac / Windows 远程执行','Remote execution on Intel Mac / Windows')}}</small></span></button>
            <button :class="{active:configSection==='runtime'}" @click="configSection='runtime'"><TerminalSquare :size="18"/><span><b>{{tr('运行环境','Runtime environment')}}</b><small>{{tr('检测、安装与实时输出','Check, install and view live output')}}</small></span></button>
          </nav>

          <template v-if="configSection==='profiles'">
            <section class="app-settings-summary panel"><div class="profile-icon"><AppWindow :size="19" /></div><div><strong>{{tr('更新与后台设置','Update and background')}}</strong><span>{{tr(`超过 ${appSettings.reminderDays} 天未更新时提醒 · ${appSettings.customIcon?'自定义图标':'默认图标'}`,`Remind after ${appSettings.reminderDays} days · ${appSettings.customIcon?'custom icon':'default icon'}`)}}</span></div><button class="button ghost compact" @click="appSettingsDialog=true">{{tr('修改','Edit')}}</button></section>
            <div class="profile-section-heading"><div><h3>{{tr('运行方案','Runtime profiles')}}</h3><p>{{tr('系统默认方案始终保留；需要新方案时从默认方案复制，避免产生空配置。','The system default is permanent. Create new profiles by copying it so they always start complete.')}}</p></div><button class="button primary" @click="cloneDefaultProfile"><Plus :size="16"/>{{tr('从默认方案创建','Create from default')}}</button></div>
            <div class="profile-grid"><article v-for="profile in profiles" :key="profile.id" class="profile-card panel"><header><div class="profile-icon"><Settings2 :size="19" /></div><span v-if="profile.isDefault" class="default-pill">SYSTEM DEFAULT</span></header><h3>{{profile.name}}</h3><p>{{profile.description}}</p><dl><div><dt>{{tr('资产引擎','Asset engine')}}</dt><dd>Rust Native</dd></div><div><dt>{{tr('前端采集','Browser capture')}}</dt><dd>Node.js · CDP</dd></div><div><dt>Probe</dt><dd>{{profile.settings.priorityRate}} r/s · {{profile.settings.workers}} workers</dd></div><div><dt>Rules</dt><dd>{{(profile.settings.gamblingKeywords?.length||0)+(profile.settings.pornKeywords?.length||0)}} keywords</dd></div></dl><footer><span>{{tr('更新于','Updated')}} {{profile.updatedAt}}</span><div><button class="button ghost compact" @click="openProfile(profile)">{{tr('编辑','Edit')}}</button><button v-if="!profile.isDefault" class="button danger compact" @click="removeProfile(profile)"><Trash2 :size="13"/>{{deletingProfileId===profile.id?tr('确认删除','Confirm delete'):tr('删除','Delete')}}</button></div></footer></article></div>
          </template>

          <WorkerSettingsPanel v-else-if="configSection==='workers'" @message="notify" />

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
        <!-- The Agent result tree can contain thousands of reactive rows. Keep
             it mounted only while visible so macOS/WebKit does not have to
             restore and repaint a hidden second application after occlusion. -->
        <SentinelBoard v-if="!loading&&activeView==='sentinel'" :active="true" :projects="projects" :project-id="selectedProjectId" :section="sentinelSection" :result-view="sentinelResultView" :workbench-mode="sentinelWorkbenchMode" :search="sentinelSearch" @create-project="openProject()" @section-change="sentinelSection=$event" @open-workbench="sentinelMenu='new_scan'; sentinelSection='workbench'; sentinelWorkbenchMode=$event" @alerts-change="sentinelAlerts=$event" @projects-change="refresh" @notify="notify" @open-runner-log="openSentinelRunnerLog" />
        <section v-if="activeView==='logs'&&activeModule==='sentinel'" class="panel sentinel-runner-console">
          <header class="sentinel-runner-console-header">
            <div><TerminalSquare :size="15"/><strong>{{tr('Nest 运行日志','Nest runner log')}}</strong><span v-if="sentinelRunnerLogLoading">{{tr('读取中','Reading')}}</span><span v-else>{{tr('事件驱动更新，展示已写入的脱敏日志','Event-driven updates from committed, redacted logs')}}</span></div>
            <div class="sentinel-runner-log-tools">
              <select class="toolbar-select" :value="sentinelLogScanId" @change="selectSentinelLogScan(($event.target as HTMLSelectElement).value)"><option v-for="scan in sentinelLogScans" :key="scan.id" :value="scan.id">{{scan.taskName||scan.projectName}} · {{scan.status}} · {{scan.id}}</option></select>
              <select class="toolbar-select" :value="sentinelLogAttempt" @change="selectSentinelLogAttempt(Number(($event.target as HTMLSelectElement).value))"><option :value="0">{{tr('最新 attempt','Latest attempt')}}</option><option v-for="item in sentinelLogAttempts" :key="item.attemptNumber" :value="item.attemptNumber">#{{item.attemptNumber}} · {{item.status}} · {{item.stage}}</option></select>
              <input v-model="sentinelLogFilter" class="toolbar-select sentinel-runner-log-filter" :placeholder="tr('筛选日志关键字','Filter by keyword')" />
              <button class="button ghost compact" :disabled="!sentinelRunnerLogLines.length" @click="copySentinelRunnerLog"><Copy :size="14" /> {{tr('复制','Copy')}}</button>
              <button class="button ghost compact" :disabled="sentinelRunnerLogLoading" @click="refreshSentinelRunnerLog"><RefreshCw :size="14" /> {{tr('刷新','Refresh')}}</button>
            </div>
          </header>
          <p class="sentinel-runner-log-status" :class="`state-${sentinelRunnerLog?.status||'idle'}`">
            <span>{{tr('attempt','Attempt')}} #{{sentinelRunnerLog?.attempt||0}}</span>
            <span>{{runnerLogStatusLabel()}}</span>
            <span v-if="sentinelRunnerLog?.source==='task_root'">{{tr('来源：当前任务目录（不属于任何 attempt）','Source: current task directory (not an attempt)')}}</span>
            <span v-else-if="sentinelRunnerLog?.workDir">{{tr('来源','Source')}}：{{sentinelRunnerLog.workDir}}</span>
            <span v-if="sentinelRunnerLog?.updatedAt">{{tr('记录更新于','attempt updated')}} {{sentinelRunnerLog.updatedAt}}</span>
            <span v-if="sentinelRunnerLog?.lines.length" :class="{muted:sentinelLogFilter.trim().length>0}">{{sentinelRunnerLog.lines.length}}{{tr(' 行，尾部窗口',' tail lines')}}</span>
            <span v-if="sentinelRunnerLog?.message">{{sentinelRunnerLog.message}}</span>
            <span v-if="sentinelRunnerLogError" class="error">{{tr('日志读取失败，保留上次快照并等待重试','Log read failed; keeping the last snapshot until retry')}}</span>
            <span v-if="sentinelLogConnectionUnavailable" class="error">{{tr('实时订阅暂不可用，正在定期补查并重连','Live subscription unavailable; reconciling and reconnecting')}}</span>
          </p>
          <section v-if="sentinelLogDiagnostics.length" class="sentinel-log-diagnostics">
            <header><strong>{{tr('诊断摘要','Diagnostics summary')}}</strong><small>{{tr('从本 attempt 日志抽取，不必跳回结果页','Extracted from this attempt log — no need to leave the log panel')}}</small></header>
            <ul><li v-for="(item,index) in sentinelLogDiagnostics" :key="`${index}-${item.slice(0,24)}`"><code>{{item}}</code></li></ul>
          </section>
          <pre ref="sentinelRunnerConsole" class="sentinel-runner-log" @scroll="onSentinelLogScroll"><code v-for="(line,index) in sentinelRunnerLogLines" :key="`${index}-${line}`">{{line}}</code><code v-if="!sentinelRunnerLogLines.length" class="empty-log">{{sentinelLogFilter.trim()?tr('没有匹配当前筛选的日志行','No log line matches the filter'):tr('这里不会显示任何内容，直到该 attempt 写出第一行日志','Nothing appears here until this attempt writes its first log line')}}</code></pre>
          <NativeProcessLogView :scan-id="sentinelLogScanId" :attempt="sentinelLogAttempt" :active="activeView==='logs' && activeModule==='sentinel'" />
        </section>
      </div>
    </main>

    <ProjectDialog v-if="projectDialog" :project="editProject" @close="projectDialog=false;editProject=undefined" @saved="projectSaved" />
    <ConfigDialog v-if="configDialog" :profile="editProfile" @close="configDialog=false;editProfile=undefined" @saved="profileSaved" />
    <AppSettingsDialog v-if="appSettingsDialog" :settings="appSettings" @close="appSettingsDialog=false" @saved="appSettingsSaved" @icon-changed="loadBrandIcon" />
    <ReleaseNotesDialog v-if="releaseNotesDialog" :version="packageInfo.version" @close="releaseNotesDialog=false" />
    <div class="toast-stack"><div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.type"><CheckCircle2 v-if="toast.type==='success'" :size="18" /><CircleAlert v-else-if="toast.type==='error'" :size="18" /><Bell v-else :size="18" /><span>{{toast.text}}</span><button @click="toasts=toasts.filter(t=>t.id!==toast.id)"><X :size="14" /></button></div></div>
  </div>
</template>
