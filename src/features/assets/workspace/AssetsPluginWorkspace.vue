<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { CheckCircle2, CircleAlert, Clock3, Database, ListFilter, Plus, RefreshCw, Search } from "@lucide/vue";
import { assetApi } from "../api";
import { workspaceApi } from "../../workspaces/api";
import { useAssetLogPanel } from "../useAssetLogPanel";
import { useI18n } from "../../../i18n";
import type { AssetEvent, DashboardStats, InterruptedJob, JobProgressEvent, JobRun } from "../../../types";
import type { PluginWorkspaceProps, PluginWorkspaceEmits, ResumeWorkflowResult } from "../../../shell/workspaceContract";
const AssetWorkspace = defineAsyncComponent(() => import("../../../components/AssetWorkspace.vue"));
const AssetOwnershipGate = defineAsyncComponent(() => import("../../../components/AssetOwnershipGate.vue"));
const ExposureSurface = defineAsyncComponent(() => import("../../../components/ExposureSurface.vue"));
const QueryPanel = defineAsyncComponent(() => import("../../../components/QueryPanel.vue"));
const props = defineProps<PluginWorkspaceProps>();
const emit = defineEmits<PluginWorkspaceEmits>();
const { t, tr } = useI18n();
const api = { ...assetApi, dashboardStats: workspaceApi.dashboardStats, acknowledgeInterruptedRun: workspaceApi.acknowledgeInterruptedRun };
const activeView = computed(() => props.view);
const projects = computed(() => props.projects);
const profiles = computed(() => props.profiles);
const selectedProjectId = computed(() => props.projectId);
const selectedProject = computed(() => props.projects.find(project => project.id === props.projectId));
const runs = ref<JobRun[]>([]);
const events = ref<AssetEvent[]>([]);
const stats = ref<DashboardStats>();
const runLiveMessages = ref<Record<number,string>>({});
const globalSearch = ref("");
const readError = ref("");
const actionError = ref("");
const assetSearchKey = ref(0);
const starting = ref(false);
const { logs, loading: assetLogsLoading, readFailed: assetLogsFailed, connectionUnavailable: assetLogConnectionUnavailable, refresh: refreshAssetLogs } = useAssetLogPanel(() => props.projectId, () => props.view === "logs");
let disposed = false, generation = 0, actionGeneration = 0;
let unlisten: UnlistenFn | undefined;
const notify = (type: "success" | "error" | "info", text: string) => { if (!disposed) emit("notify", type, text); };
const navigate = (view: PluginWorkspaceProps["view"]) => { if (!disposed) emit("navigate", view); };
const openProject = () => emit("createProject");
const selectProject = (id?: number) => emit("selectProject", id);
const toggleArchive = (project: PluginWorkspaceProps["projects"][number]) => emit("restoreProject", project);
function doGlobalSearch() { if (globalSearch.value.trim()) { assetSearchKey.value++; navigate("assets"); } }
async function refresh() {
  const request = ++generation, projectId = props.projectId, view = props.view;
  const current = () => !disposed && request === generation && props.projectId === projectId && props.view === view;
  readError.value = "";
  const reads = [api.dashboardStats(projectId).then(value => { if(current()) { stats.value=value; emit("metadata", { badges: { runningJobs:value.runningJobs, blockedCount:value.blockedCount } }); } })];
  if (["dashboard","tasks"].includes(view)) reads.push(api.listRuns(projectId,100).then(value => { if(current()) runs.value=value; }));
  if (["dashboard","changes"].includes(view)) reads.push(api.listEvents(projectId,undefined,200).then(value => { if(current()) events.value=value; }));
  if (view === "logs") reads.push(refreshAssetLogs());
  const results = await Promise.allSettled(reads);
  if (current()) { const failed=results.find(result => result.status === "rejected"); if(failed?.status === "rejected") readError.value=String(failed.reason); }
}
async function quickStartProject(projectId:number, profileId?:number, name?:string, pipeline="full") {
  const project=projects.value.find(item=>item.id===projectId);
  if (!project || project.status !== "active" || starting.value) return;
  const running=runs.value.find(run=>run.projectId===projectId && ["running","queued","cancel_requested"].includes(run.status));
  if(running){ notify("info",tr(`项目已有运行中的任务 #${running.id}`,`Project already has running job #${running.id}`)); navigate("tasks"); return; }
  const profile=profiles.value.find(item=>item.id===profileId)||profiles.value.find(item=>item.isDefault)||profiles.value[0];
  if(!profile){notify("error",tr("请先创建执行配置","Create an execution profile first"));return;}
  const action=++actionGeneration, contextProject=props.projectId;
  starting.value=true;actionError.value="";
  try {
    const runId=await api.startJob(projectId,profile.id,name||`${project.name} · ${tr("快速任务","Quick run")}`,pipeline);
    if(disposed||action!==actionGeneration||contextProject!==props.projectId)return;
    notify("success",tr(`任务 #${runId} 已加入队列`,`Job #${runId} queued`));navigate("tasks");emit("projectsChanged");void refresh();
  } catch(error){if(!disposed&&action===actionGeneration&&contextProject===props.projectId){actionError.value=String(error);notify("error",String(error));}}
  finally{if(!disposed&&action===actionGeneration)starting.value=false;}
}
async function resumeInterrupted(job:InterruptedJob, fromIntent=false) {
  const admitted={...job};
  let acknowledged=false;
  const project=projects.value.find(item=>item.id===admitted.projectId);
  const workflow=props.resumeWorkflow;
  if(!project||project.status!=="active"||starting.value){
    actionError.value=tr("未接受恢复操作，请核对工作空间和当前任务状态。","Resume was not admitted. Check the workspace and current task status.");
    if(fromIntent)workflow?.settle(admitted,{started:false,acknowledged:false,error:actionError.value});
    return;
  }
  if(workflow&&!workflow.begin(admitted)){actionError.value=tr("此任务正在恢复或等待状态核对","This task is already resuming or awaiting status verification");return;}
  const result:ResumeWorkflowResult={started:false,acknowledged:false};
  const contextProject=props.projectId, action=++actionGeneration;
  starting.value=true;actionError.value="";
  try {
    if(admitted.pipeline==="reprobe") await api.resumeJob(admitted.runId);
    else {
      const profile=profiles.value.find(item=>item.id===admitted.profileId)||profiles.value.find(item=>item.isDefault)||profiles.value[0];
      if(!profile)throw Error(tr("请先创建执行配置","Create an execution profile first"));
      const profileId=profile.id;
      await api.acknowledgeInterruptedRun(admitted.runId);
      acknowledged=true;result.acknowledged=true;
      // Acknowledgement is durable. Complete this exact already-admitted workflow;
      // disabling the UI must not strand it or re-admit another run.
      await api.startJob(admitted.projectId,profileId,`${admitted.name} · ${tr("继续","Resume")}`,admitted.pipeline||"full");
    }
    result.started=true;
    if(disposed||action!==actionGeneration||contextProject!==props.projectId)return;
    notify("success",tr(`任务 #${admitted.runId} 已继续`,`Job #${admitted.runId} resumed`));emit("interruptedResumed",admitted.runId);navigate("tasks");emit("projectsChanged");void refresh();
  } catch(error){result.error=acknowledged?`${tr("旧任务已确认，但替代任务未启动：","Old task acknowledged, but its replacement did not start: ")}${String(error)}`:String(error);if(!disposed&&action===actionGeneration&&contextProject===props.projectId){actionError.value=acknowledged?`${tr("旧任务已确认，但替代任务未启动：","Old task acknowledged, but its replacement did not start: ")}${String(error)}`:String(error);notify("error",actionError.value);}}
  finally{workflow?.settle(admitted,result);if(!disposed&&action===actionGeneration)starting.value=false;}
}
async function cancelJob(id:number) {
  const request=generation, projectId=props.projectId;
  try{await api.cancelJob(id);if(!disposed&&request===generation&&projectId===props.projectId)void refresh();}
  catch(error){if(!disposed&&request===generation){actionError.value=String(error);notify("error",String(error));}}
}
watch(() => props.projectId, () => { generation++; actionGeneration++; starting.value=false; actionError.value=""; stats.value=undefined;runs.value=[];events.value=[];runLiveMessages.value={};globalSearch.value="";emit("metadata",{badges:{runningJobs:0,blockedCount:0}}); }, {flush:"sync"});
watch(() => [props.projectId, props.view, props.refreshToken], () => { void refresh(); }, {immediate:true});
let consumedIntent=0;
watch(() => props.intent, intent => {
  if(!intent||intent.id===consumedIntent)return;
  consumedIntent=intent.id;emit("intentConsumed",intent.id);
  if(intent.projectId!==props.projectId||(intent.kind==="resume"&&intent.job.projectId!==intent.projectId)){actionError.value=tr("工作空间已变化，请重新操作","Workspace changed; try the action again");notify("error",actionError.value);if(intent.kind==="resume")props.resumeWorkflow?.settle({...intent.job},{started:false,acknowledged:false,error:actionError.value});return;}
  if(intent.kind==="resume")void resumeInterrupted(intent.job,true);
  else void quickStartProject(intent.projectId,intent.profileId,intent.name,intent.pipeline);
},{immediate:true});
onMounted(async()=>{
  try {
    const stop=await listen<JobProgressEvent>("job-progress",event=>{
      if(disposed)return;
      const progress=event.payload, run=runs.value.find(item=>item.id===progress.runId);
      if(run){run.status=progress.status;run.stage=progress.stage;run.progress=progress.progress;runLiveMessages.value={...runLiveMessages.value,[run.id]:progress.message};}
      if(["completed","failed","cancelled"].includes(progress.status))void refresh();
    });
    if(disposed)stop();else unlisten=stop;
  }catch(error){if(!disposed)readError.value=String(error);}
});
onUnmounted(()=>{disposed=true;generation++;actionGeneration++;unlisten?.();});
function stageLabel(stage:string){ const zh=({queued:"等待",collect:"采集",import:"入库",refine:"分层",probe:"探测",reprobe:"存活复测",completed:"完成",failed:"失败",cancelled:"取消",interrupted:"已中断",restarted:"已继续"} as Record<string,string>)[stage]; const en=({queued:"Queued",collect:"Collect",import:"Import",refine:"Refine",probe:"Probe",reprobe:"Re-probe",completed:"Completed",failed:"Failed",cancelled:"Cancelled",interrupted:"Interrupted",restarted:"Restarted"} as Record<string,string>)[stage]; return tr(zh||stage,en||stage); }
function statusTone(status:string){ return status==="completed"?"success":["failed","interrupted"].includes(status)?"danger":["running","queued","cancel_requested"].includes(status)?"running":"muted"; }
function eventLabel(type:string){ const zh=({new:"新增",changed:"变化",missing:"未发现",decision:"人工结论",archived:"归档",restored:"恢复"} as Record<string,string>)[type]; const en=({new:"New",changed:"Changed",missing:"Not seen",decision:"Decision",archived:"Archived",restored:"Restored"} as Record<string,string>)[type]; return tr(zh||type,en||type); }
</script>
<template>
  <section class="asset-plugin-workspace">
    <div v-if="activeView!=='logs'" class="plugin-workspace-toolbar"><div class="global-search"><Search :size="16"/><input v-model="globalSearch" :placeholder="tr('查询资产','Search assets')" @keyup.enter="doGlobalSearch"/></div></div>
    <p v-if="actionError" class="plugin-read-error" role="alert">{{actionError}}</p>
    <p v-if="readError" class="plugin-read-error" role="alert">{{tr('读取失败，内容可能不是最新状态。','Read failed; displayed data may be stale.')}} {{readError}}</p>
        <template v-if="activeView==='dashboard'">
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

        <QueryPanel :key="selectedProjectId??'all'" v-else-if="activeView==='query'" :projects="projects" :profiles="profiles" :selected-project-id="selectedProjectId" @create-project="openProject()" @project-change="selectProject" @started="navigate('tasks')" @notify="notify" />
        <AssetWorkspace v-else-if="activeView==='assets'" :key="`${selectedProjectId??'all'}:${assetSearchKey}`" :scanning-enabled="props.scanningEnabled" :projects="projects" :selected-project-id="selectedProjectId" :initial-search="globalSearch" @reprobe="quickStartProject($event,undefined,undefined,'reprobe')" @notify="notify" />
        <AssetOwnershipGate :key="selectedProjectId??'all'" v-else-if="activeView==='ownership'" :projects="projects" :selected-project-id="selectedProjectId" @notify="notify" />
        <ExposureSurface :key="selectedProjectId??'all'" v-else-if="activeView==='exposure'" :projects="projects" :selected-project-id="selectedProjectId" @notify="notify" />

        <template v-else-if="activeView==='quarantine'">
          <div class="section-toolbar"><div><span class="eyebrow">QUARANTINE</span><h2>{{tr('内容隔离区','Content quarantine')}}</h2><p>{{tr('集中查看自动规则隔离的赌博、色情和自定义命中；数据不会被物理删除。','Review gambling, adult and custom-rule matches; quarantined data is never physically deleted.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <AssetWorkspace :key="selectedProjectId??'all'" :scanning-enabled="props.scanningEnabled" :projects="projects" :selected-project-id="selectedProjectId" quarantine-only @notify="notify" />
        </template>

        <template v-else-if="activeView==='changes'">
          <div class="section-toolbar"><div><span class="eyebrow">DIFF & HISTORY</span><h2>{{tr('变化事件','Change events')}}</h2><p>{{tr('只记录新增、关键字段变化、未再次发现和人工操作，不重复保存完整快照。','Records new assets, key changes, assets not seen again and manual actions without duplicate full snapshots.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <section class="panel list-panel"><div class="event-table-head"><span>{{tr('类型','Type')}}</span><span>{{tr('资产','Asset')}}</span><span>{{tr('变化说明','Change')}}</span><span>{{tr('任务','Job')}}</span><span>{{tr('时间','Time')}}</span></div><div v-for="event in events" :key="event.id" class="event-table-row"><span><em class="change-type" :class="`event-${event.eventType}`">{{eventLabel(event.eventType)}}</em></span><span><strong>{{event.company||'—'}}</strong><small>{{event.host||event.assetKey}}</small></span><span>{{event.summary}}</span><span>#{{event.runId||'—'}}</span><time>{{event.createdAt}}</time></div><div v-if="!events.length" class="empty-state">{{tr('暂无变化事件','No change events')}}</div></section>
        </template>

        <template v-else-if="activeView==='tasks'">
          <div class="section-toolbar"><div><span class="eyebrow">JOB CENTER</span><h2>{{tr('任务中心','Job center')}}</h2><p>{{tr('长时间任务可独立运行、查看结构化日志并安全取消。','Long jobs run independently with structured logs and safe cancellation.')}}</p></div><div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button><button class="button primary" @click="navigate('query')"><Plus :size="16" /> {{tr('新建任务','New job')}}</button></div></div>
          <div class="run-list"><article v-for="run in runs" :key="run.id" class="run-card panel"><div class="run-status-mark" :class="statusTone(run.status)"><component :is="run.status==='completed'?CheckCircle2:['failed','interrupted'].includes(run.status)?CircleAlert:Clock3" :size="20" /></div><div class="run-main"><div class="run-title"><strong>{{run.name}}</strong><span class="run-id">#{{run.id}}</span><span class="run-stage">{{stageLabel(run.stage)}}</span></div><p>{{run.projectName}} · {{tr('创建于','created')}} {{run.createdAt}}</p><div class="run-progress"><div class="progress-track"><i :style="{width:`${run.progress}%`}"></i></div><span>{{Math.round(run.progress)}}%</span></div><small v-if="runLiveMessages[run.id]&&['running','queued','cancel_requested'].includes(run.status)" class="run-live-message">{{runLiveMessages[run.id]}}</small><small v-if="run.error" class="run-error">{{run.error}}</small></div><div class="run-side"><span class="status-chip" :class="statusTone(run.status)">{{stageLabel(run.status)}}</span><button v-if="run.status==='interrupted'||(run.pipeline==='reprobe'&&['failed','cancelled'].includes(run.status))" class="button secondary compact" @click="resumeInterrupted({runId:run.id,projectId:run.projectId,projectName:run.projectName,profileId:run.profileId,name:run.name,pipeline:run.pipeline,createdAt:run.createdAt})">{{tr('从断点继续','Resume checkpoint')}}</button><button v-if="['running','queued','cancel_requested'].includes(run.status)" class="button danger compact" @click="cancelJob(run.id)">{{tr('取消','Cancel')}}</button><button class="button ghost compact" @click="navigate('logs')">{{tr('日志','Logs')}}</button></div></article><div v-if="!runs.length" class="empty-state panel">{{tr('暂无任务','No jobs')}}</div></div>
        </template>

        <template v-else-if="activeView==='logs'">
          <div class="section-toolbar"><div><span class="eyebrow">ASSET ACTIVITY</span><h2>{{tr('资产采集与操作日志','Asset collection activity')}}</h2></div><button class="button ghost" @click="refresh"><RefreshCw :size="16"/>{{t.refresh}}</button></div>
          <section class="panel log-panel">
            <p v-if="assetLogConnectionUnavailable" role="status">{{tr('实时日志订阅暂不可用，正在补查并重连','Live log subscription unavailable; reconciling and reconnecting')}}</p>
            <p v-if="assetLogsFailed" role="alert">{{tr('日志读取失败，保留上次快照并等待重试','Log read failed; keeping the last snapshot until retry')}}</p>
            <div v-for="log in logs" :key="log.id" class="log-row"><time>{{log.createdAt}}</time><span class="log-level" :class="`log-${log.level}`">{{log.level}}</span><span class="log-stage">{{log.stage||'system'}}</span><code>{{log.message}}</code><em v-if="log.runId">#{{log.runId}}</em></div>
            <div v-if="!logs.length&&!assetLogsFailed" class="empty-state">{{assetLogsLoading?tr('读取中','Reading'):tr('暂无日志','No logs')}}</div>
          </section>
        </template>

  </section>
</template>
<style scoped>
.plugin-workspace-toolbar{display:flex;justify-content:flex-end;margin-bottom:14px}.plugin-read-error{color:var(--red);font-size:12px;overflow-wrap:anywhere}
</style>
