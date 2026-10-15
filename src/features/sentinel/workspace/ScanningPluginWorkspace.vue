<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onUnmounted, ref, watch } from "vue";
import { Activity, CircleAlert, Copy, RefreshCw, Search, TerminalSquare } from "@lucide/vue";
import { sentinelApi as api } from "../api";
import { humanizeScanCheckpoint } from "../presentation";
import NativeProcessLogView from "../components/NativeProcessLogView.vue";
import { useRunnerLogPanel } from "../execution/useRunnerLogPanel";
import { useI18n } from "../../../i18n";
import type { SentinelScan, SentinelScanAttempt } from "../types";
import type { PluginWorkspaceProps, PluginWorkspaceEmits } from "../../../shell/workspaceContract";
const SentinelBoard = defineAsyncComponent(() => import("../components/SentinelBoard.vue"));
const props = defineProps<PluginWorkspaceProps>();
const emit = defineEmits<PluginWorkspaceEmits>();
const { t, tr } = useI18n();
const activeView = computed(() => props.view);
const activeModule = ref("sentinel");
const projects = computed(() => props.projects);
const selectedProjectId = computed(() => props.projectId);
const readError = ref("");
let disposed=false, generation=0, attemptGeneration=0;
const notify = (type: "success"|"error"|"info", text:string) => {if(!disposed)emit("notify",type,text);};
const navigate = (view:PluginWorkspaceProps["view"]) => {if(!disposed)emit("navigate",view);};
const sentinelSearch = ref("");
const sentinelSection = ref<"overview"|"queue"|"results"|"fuse"|"validations"|"workbench"|"dialog"|"help">("dialog");
const sentinelResultView = ref<"summary"|"fingerprint"|"api"|"endpoints"|"vulnerabilities">("summary");
const sentinelWorkbenchMode = ref<"web"|"code"|"greybox"|"cicd"|"skills"|"traces">("code");
const sentinelMenu = ref("dialog");
const sentinelLogScans=ref<SentinelScan[]>([]);
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
  const request=++attemptGeneration, projectId=props.projectId;
  try{
    const rows=await api.listSentinelScanAttempts(scanId);
    if(!disposed&&request===attemptGeneration&&projectId===props.projectId&&scanId===sentinelLogScanId.value)sentinelLogAttempts.value=rows;
  }catch{ if(!disposed&&request===attemptGeneration&&projectId===props.projectId&&scanId===sentinelLogScanId.value)sentinelLogAttempts.value=[]; }
}
function syncSentinelLogSelection(){
  const selected=sentinelLogScans.value.find(scan=>scan.id===sentinelLogScanId.value);
  if(selected)return;
  const preferred=sentinelLogScans.value.find(scan=>["scanning","pausing","queued"].includes(scan.status))||sentinelLogScans.value[0];
  selectSentinelLogScan(preferred?.id||"");
}
function openSentinelRunnerLog(scanId:string,attempt:number){
  if(!scanId)return;
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

function statusTone(status:string){return status==="completed"?"success":["failed","interrupted"].includes(status)?"danger":["running","queued","cancel_requested"].includes(status)?"running":"muted";}
const titles = computed(() => ({dialog:tr("调查对话","Investigation dialog"),overview:tr("行动中心","Investigation desk"),queue:tr("任务与成本","Tasks & cost"),new_scan:tr("新调查","New investigation"),urls:tr("这次结果","This investigation"),skills:tr("知识与策略","Knowledge & policy")} as Record<string,string>));
watch(() => [sentinelMenu.value, sentinelAlerts.value.fuse, props.view], () => emit("metadata", {navigationKey:sentinelMenu.value,title:props.view==="logs"?tr("扫描记录","Scan activity"):titles.value[sentinelMenu.value]||titles.value.overview,badges:{fuse:sentinelAlerts.value.fuse}}), {immediate:true});
function doGlobalSearch(){sentinelSection.value="results";sentinelMenu.value="urls";navigate("sentinel");}
function openWorkbench(mode:typeof sentinelWorkbenchMode.value){sentinelMenu.value="new_scan";sentinelSection.value="workbench";sentinelWorkbenchMode.value=mode;}
function onSectionChange(section:typeof sentinelSection.value){sentinelSection.value=section;sentinelMenu.value=section==="results"?"urls":section==="workbench"?"new_scan":section;}
async function refresh(){
  const request=++generation, projectId=props.projectId;
  if(props.view!=="logs")return;
  readError.value="";
  try{
    const rows=await api.listSentinelScans(projectId,300);
    if(disposed||request!==generation||projectId!==props.projectId||props.view!=="logs")return;
    sentinelLogScans.value=rows;syncSentinelLogSelection();void refreshSentinelRunnerLog();
  }catch(error){if(!disposed&&request===generation)readError.value=String(error);}
}
watch(() => props.navigation, item => {
  if(!item?.section)return;
  sentinelMenu.value=item.key;sentinelSection.value=item.section;
  if(item.result)sentinelResultView.value=item.result;
  if(item.workbench)sentinelWorkbenchMode.value=item.workbench;
},{immediate:true});
watch(() => props.projectId,()=>{generation++;attemptGeneration++;sentinelLogScans.value=[];sentinelLogScanId.value="";sentinelLogAttempt.value=0;sentinelLogAttempts.value=[];sentinelRunnerLog.value=undefined;sentinelSearch.value="";},{flush:"sync"});
watch(()=>[props.projectId,props.view,props.refreshToken],()=>{void refresh();},{immediate:true});
onUnmounted(()=>{disposed=true;generation++;attemptGeneration++;});
</script>
<template>
  <section class="scanning-plugin-workspace">
    <div v-if="activeView==='sentinel'" class="plugin-workspace-toolbar"><div class="global-search"><Search :size="16"/><input v-model="sentinelSearch" :placeholder="tr('搜索公司、URL、源码或任务','Search company, URL, source or task')" @keyup.enter="doGlobalSearch"/></div></div>
    <p v-if="readError" class="plugin-read-error" role="alert">{{tr('读取失败，内容可能不是最新状态。','Read failed; displayed data may be stale.')}} {{readError}}</p>
    <!-- Heavy evidence/results trees remain mounted only while visible. -->
    <SentinelBoard v-if="activeView==='sentinel'" :key="selectedProjectId??'all'" :active="true" :projects="projects" :project-id="selectedProjectId" :section="sentinelSection" :result-view="sentinelResultView" :workbench-mode="sentinelWorkbenchMode" :search="sentinelSearch" @create-project="emit('createProject')" @section-change="onSectionChange" @open-workbench="openWorkbench" @alerts-change="sentinelAlerts=$event" @projects-change="emit('projectsChanged')" @notify="notify" @open-runner-log="openSentinelRunnerLog"/>
    <template v-else-if="activeView==='logs'">
      <div class="section-toolbar"><div><span class="eyebrow">SCANNING ACTIVITY</span><h2>{{tr('扫描记录','Scan activity')}}</h2><p>{{tr('只显示扫描状态、任务类型、Token 和检查点，不在 URL 概要中堆叠原始日志。','Shows scan status, type, tokens and checkpoints without dumping raw logs into URL summaries.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16"/>{{t.refresh}}</button></div>
          <section class="panel module-activity-list"><article v-for="scan in sentinelLogScans" :key="scan.id"><span class="run-status-mark" :class="statusTone(scan.status)"><Activity :size="18"/></span><div><strong>{{scan.taskName||scan.projectName}}</strong><small>{{scan.scanType||'web'}} · {{scan.projectName}} · {{scan.id}}</small><p>{{sentinelCheckpointSummary(scan)}}</p><details v-if="sentinelErrorDetail(scan)" class="activity-error-detail"><summary><CircleAlert :size="13"/>{{tr('报错细节','Error details')}}</summary><pre>{{sentinelErrorDetail(scan)}}</pre></details></div><div class="activity-metric"><b>{{scan.totalTokens.toLocaleString()}}</b><span>Token</span><time>{{scan.updatedAt}}</time></div></article><div v-if="!sentinelLogScans.length" class="empty-state">{{tr('暂无扫描记录','No scan activity')}}</div></section>
    </template>
        <section v-if="activeView==='logs'" class="panel sentinel-runner-console">
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
          <NativeProcessLogView :scan-id="sentinelLogScanId" :attempt="sentinelLogAttempt" :active="activeView==='logs'" />
        </section>
  </section>
</template>
<style scoped>
.plugin-workspace-toolbar{display:flex;justify-content:flex-end;margin-bottom:14px}.plugin-read-error{color:var(--red);font-size:12px;overflow-wrap:anywhere}
</style>
