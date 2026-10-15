<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ExternalLink, RefreshCw, Search, Star, UploadCloud, X } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { hackerOneApi } from "../api";
import type { ConfigProfile } from "../../configuration/types";
import type { HackerOneDetail, HackerOneEvent, HackerOneProgram } from "../types";
import type { Project } from "../../workspaces/types";
import { useI18n } from "../../../i18n";

const props=defineProps<{projects:Project[];profiles:ConfigProfile[]}>();
const emit=defineEmits<{notify:[type:"success"|"error"|"info",text:string]}>();
const {tr}=useI18n(); const programs=ref<HackerOneProgram[]>([]); const detail=ref<HackerOneDetail>();
const changes=ref<HackerOneEvent[]>([]);
const search=ref(""); const busy=ref(false); const selectedProject=ref<number>();
const profile=()=>props.profiles.find(p=>p.isDefault)||props.profiles[0];
async function load(){[programs.value,changes.value]=await Promise.all([hackerOneApi.listHackerOnePrograms(search.value),hackerOneApi.listHackerOneEvents(50)])}
async function sync(handle?:string){const p=profile();if(!p){emit("notify","error",tr("请先创建配置","Create a profile"));return}busy.value=true;try{await hackerOneApi.syncHackerOne(p.id,handle);await load();if(handle)detail.value=await hackerOneApi.getHackerOneDetail(handle);emit("notify","success",tr("HackerOne 信息已更新","HackerOne data updated"))}catch(e){emit("notify","error",String(e))}finally{busy.value=false}}
async function show(program:HackerOneProgram){busy.value=true;try{const active=profile();if(!active){emit("notify","error","请先在配置中心创建配置方案");return}if(!program.scopeCount)await hackerOneApi.syncHackerOne(active.id,program.handle);detail.value=await hackerOneApi.getHackerOneDetail(program.handle);selectedProject.value=props.projects[0]?.id}catch(e){emit("notify","error",String(e))}finally{busy.value=false}}
async function bookmark(program:HackerOneProgram){await hackerOneApi.setHackerOneBookmark(program.handle,!program.bookmarked);program.bookmarked=!program.bookmarked}
async function send(){if(!detail.value||!selectedProject.value)return;try{const count=await hackerOneApi.addHackerOneScopesToProject(detail.value.program.handle,selectedProject.value);emit("notify","success",tr(`已发送 ${count} 个可提交网络资产到项目`,`${count} network scopes added to project`))}catch(e){emit("notify","error",String(e))}}
onMounted(load);
</script>

<template>
  <section class="h1-board">
    <div class="section-toolbar"><div><span class="eyebrow">HACKERONE</span><h2>{{tr('厂商资讯与 Scope','Programs and scopes')}}</h2><p>{{tr('仅使用官方 Hacker API；私有项目数据只保存在本机。','Official Hacker API only; private program data stays local.')}}</p></div><button class="button primary" :disabled="busy" @click="sync()"><RefreshCw :size="16" :class="{spinning:busy}"/> {{tr('同步项目','Sync programs')}}</button></div>
    <div class="h1-search panel"><Search :size="16"/><input v-model="search" :placeholder="tr('搜索厂商或 handle','Search name or handle')" @keyup.enter="load"/><button class="button ghost compact" @click="load">{{tr('查询','Search')}}</button></div>
    <section v-if="changes.length" class="panel h1-changes"><strong>{{tr('最近 Scope / Policy 变化','Recent scope / policy changes')}}</strong><div v-for="item in changes.slice(0,8)" :key="item.id"><em>{{item.eventType}}</em><span>@{{item.programHandle}} · {{item.summary}}</span><time>{{item.createdAt}}</time></div></section>
    <div v-if="programs.length" class="h1-grid"><article v-for="program in programs" :key="program.handle" class="h1-card panel">
      <header><img v-if="program.iconUrl" :src="program.iconUrl"/><span v-else>{{program.name.slice(0,1).toUpperCase()}}</span><button class="icon-button subtle" @click="bookmark(program)"><Star :size="16" :fill="program.bookmarked?'currentColor':'none'"/></button></header>
      <h3>{{program.name}}</h3><small>@{{program.handle}} · {{program.programState}}</small>
      <div class="h1-tags"><em v-if="program.submissionState==='open'">{{tr('接受提交','Open')}}</em><em v-if="program.offersBounties">{{tr('有赏金','Bounty')}}</em><em v-if="program.openScope">Open Scope</em><em v-if="program.safeHarbor">Safe Harbor</em><em v-if="program.fastPayments">Fast Pay</em></div>
      <footer><span>{{program.scopeCount}} Scope</span><button class="text-button" @click="show(program)">{{tr('查看详情','Details')}}</button></footer>
    </article></div>
    <div v-else class="empty-state panel">{{tr('尚未同步。请先在配置中心填写 HackerOne API identifier 和 token，然后点击同步。','No data yet. Configure the HackerOne API identifier and token, then sync.')}}</div>

    <div v-if="detail" class="h1-detail-overlay"><section class="h1-detail panel"><button class="h1-close icon-button subtle" @click="detail=undefined"><X :size="18"/></button>
      <div class="h1-detail-head"><img v-if="detail.program.iconUrl" :src="detail.program.iconUrl"/><div><h2>{{detail.program.name}}</h2><span>@{{detail.program.handle}} · {{detail.program.lastSyncedAt}}</span></div><button class="button ghost compact" @click="sync(detail.program.handle)"><RefreshCw :size="14"/>{{tr('实时刷新','Refresh')}}</button><button class="button ghost compact" @click="openUrl(`https://hackerone.com/${detail.program.handle}`)"><ExternalLink :size="14"/>{{tr('官网','Official')}}</button></div>
      <div class="h1-send"><select v-model="selectedProject"><option v-for="p in projects" :key="p.id" :value="p.id">{{p.name}}</option></select><button class="button secondary" @click="send"><UploadCloud :size="15"/>{{tr('发送可提交网络 Scope 到项目','Send network scopes to project')}}</button></div>
      <h3>Scope</h3><div class="h1-scope-table"><div class="h1-scope-head"><span>Type</span><span>Asset</span><span>{{tr('提交','Submit')}}</span><span>{{tr('赏金','Bounty')}}</span><span>Max</span></div><div v-for="scope in detail.scopes" :key="scope.id"><span>{{scope.assetType}}</span><code>{{scope.assetIdentifier}}</code><span>{{scope.eligibleForSubmission?'✓':'—'}}</span><span>{{scope.eligibleForBounty?'✓':'—'}}</span><span>{{scope.maxSeverity||'—'}}</span><small v-if="scope.instruction">{{scope.instruction}}</small></div></div>
      <template v-if="detail.exclusions.length"><h3>Scope Exclusions</h3><div class="h1-exclusions"><article v-for="item in detail.exclusions" :key="item.id"><strong>{{item.category}}</strong><p>{{item.details}}</p></article></div></template>
      <h3>Policy</h3><pre class="h1-policy">{{detail.program.policy||tr('暂无 Policy','No policy')}}</pre>
    </section></div>
  </section>
</template>
