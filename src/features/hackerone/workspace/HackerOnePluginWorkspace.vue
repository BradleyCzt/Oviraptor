<script setup lang="ts">
import { defineAsyncComponent, onUnmounted, ref, watch } from "vue";
import { RefreshCw, ShieldCheck } from "@lucide/vue";
import { hackerOneApi } from "../api";
import { useI18n } from "../../../i18n";
import type { HackerOneEvent } from "../types";
import type { PluginWorkspaceProps, PluginWorkspaceEmits } from "../../../shell/workspaceContract";
const HackerOneBoard=defineAsyncComponent(()=>import("../../../components/HackerOneBoard.vue"));
const props=defineProps<PluginWorkspaceProps>();
const emit=defineEmits<PluginWorkspaceEmits>();
const {t,tr}=useI18n();
const hackerOneLogEvents=ref<HackerOneEvent[]>([]), readError=ref("");
let disposed=false,generation=0;
const notify=(kind:"success"|"error"|"info",text:string)=>{if(!disposed)emit("notify",kind,text);};
async function refresh(){
 const request=++generation;if(props.view!=="logs")return;readError.value="";
 try{const value=await hackerOneApi.listHackerOneEvents(300);if(!disposed&&request===generation&&props.view==="logs")hackerOneLogEvents.value=value;}
 catch(error){if(!disposed&&request===generation)readError.value=String(error);}
}
watch(()=>[props.view,props.refreshToken],()=>{void refresh();},{immediate:true});
onUnmounted(()=>{disposed=true;generation++;});
</script>
<template>
 <section class="hackerone-plugin-workspace">
  <HackerOneBoard v-if="view==='hackerone'" :projects="projects" :profiles="profiles" @notify="notify"/>
  <template v-else-if="view==='logs'">
    <div class="section-toolbar"><div><span class="eyebrow">HACKERONE ACTIVITY</span><h2>{{tr('HackerOne 同步记录','HackerOne sync activity')}}</h2></div><button class="button ghost" @click="refresh"><RefreshCw :size="16"/>{{t.refresh}}</button></div>
    <p v-if="readError" role="alert">{{readError}}</p>
          <section class="panel module-activity-list"><article v-for="event in hackerOneLogEvents" :key="event.id"><span class="run-status-mark success"><ShieldCheck :size="18"/></span><div><strong>{{event.programHandle}}</strong><small>{{event.eventType}}</small><p>{{event.summary}}</p></div><time>{{event.createdAt}}</time></article><div v-if="!hackerOneLogEvents.length" class="empty-state">{{tr('暂无 HackerOne 同步记录','No HackerOne sync activity')}}</div></section>
  </template>
 </section>
</template>
