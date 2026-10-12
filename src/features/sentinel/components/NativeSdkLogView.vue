<script setup lang="ts">
import {computed} from "vue";
import type {SdkLogRow} from "../execution/nativeSdkLogContract";
import {useNativeSdkLog} from "../execution/useNativeSdkLog";
const props=defineProps<{scanId:string;attempt?:number;active?:boolean}>();
const {replay,loading,readFailed,connectionUnavailable,refresh}=useNativeSdkLog(
 ()=>props.active!==false&&props.scanId?{scanId:props.scanId,attempt:props.attempt??0}:undefined);
const stageLabel=(value?:string)=>( ({prepared:"原调用已准备",sent:"已发起请求",response_received:"HTTP 响应已读完",cost_saved:"原费用记录已保存",validated:"SDK 检查已通过",terminal:"SDK 入口已终结"} as Record<string,string>)[value??""]??"阶段需核对");
const domainLabel=(value?:string)=>( ({root:"Root 模型调用",specialist:"独立子任务调用",source_round:"源码工具轮 SDK 调用",mapper_loop:"Mapper 受控工具轮 SDK 调用"} as Record<string,string>)[value??""]??"调用归属需核对");
const terminalLabel=(value?:string)=>( ({returned:"原结果已返回",withheld:"结果未交付后续工作",uncertain:"结果未知 · 原占用保留",unsent:"尚未发出",failed:"入口状态需核对"} as Record<string,string>)[value??""]??"尚无终结记录");
const costLabel=(value?:string)=>( ({received:"已保存收到响应的原费用事实",uncertain:"已保存未知结果的原费用事实",unsent:"已保存未发出的原费用事实"} as Record<string,string>)[value??""]??"");
const calls=computed(()=>{
 const grouped=new Map<string,{ownerId:string;rows:SdkLogRow[];last:SdkLogRow}>();
 for(const row of replay.value.rows){
  const call=grouped.get(row.ownerId);
  if(call){call.rows.push(row);call.last=row;}
  else grouped.set(row.ownerId,{ownerId:row.ownerId,rows:[row],last:row});
 }
 return [...grouped.values()].reverse();
});
</script>
<template>
 <section class="native-sdk-log" aria-label="原生 SDK 调用阶段">
  <header><div><span class="eyebrow">MODEL CALL STAGES</span><strong>SDK 调用诊断 · #{{replay.attempt??props.attempt??0}}</strong></div><button class="button ghost compact" type="button" :disabled="loading" @click="refresh">刷新诊断</button></header>
  <p class="sdk-boundary">这里只显示已提交的安全阶段，不保存模型内容。请求已发起不等于服务端已接受；SDK 终结不代表智能体、工具、证据审核或任务已完成。</p>
  <p v-if="connectionUnavailable" role="status">实时订阅暂不可用，正在补读已提交记录并重连。</p>
  <p v-if="readFailed" role="alert">此轮 SDK 记录无法核验，保留最后一次提交内容；不能据此判断调用成功。</p>
  <p v-if="replay.hasGap" role="status">SDK 阶段采集存在缺口；原调用结果和费用以独立业务回执为准。</p>
  <p v-if="replay.incompleteOwners.length" role="status">{{replay.incompleteOwners.length}} 个原调用尚无完整诊断终结记录；可能仍在运行或采集已中断，不能视为已完成。</p>
  <p v-if="replay.metadataTruncated" role="status">诊断归属或缺口列表已达到显示上限，未展示项目不能视为已完成。</p>
  <p v-if="replay.evicted" role="status">当前显示最近 300 条阶段，{{replay.evicted}} 条已移出视图，完整记录仍可按游标读取。</p>
  <p v-if="replay.more||loading" role="status">正在恢复已提交的 SDK 阶段…</p>
  <div v-if="replay.rows.length" class="sdk-rows" role="region" aria-label="已提交 SDK 阶段" tabindex="0">
   <article v-for="call in calls" :key="call.ownerId" :data-state="call.last.terminalState||'open'">
    <header class="sdk-call-heading">
     <div><b>{{domainLabel(call.last.domain)}}</b><span>模型轮次 {{call.last.round}}</span></div>
     <span class="sdk-call-state">{{call.last.stage==='terminal'?terminalLabel(call.last.terminalState):stageLabel(call.last.stage)}}</span>
    </header>
    <ol class="sdk-stage-trail" aria-label="此调用已提交的阶段">
     <li v-for="row in call.rows" :key="row.sequence"><span class="sdk-stage-dot" aria-hidden="true"></span><div><b>{{stageLabel(row.stage)}}</b><time>{{row.time}}</time></div></li>
    </ol>
    <small v-if="call.last.costPhase" class="sdk-cost">{{costLabel(call.last.costPhase)}} · 不等于最终账单金额</small>
    <details><summary>原归属详情</summary><code>{{call.last.domain}} · {{call.last.stage}} · {{call.last.sequence}} / {{call.last.ordinal}}<br>{{call.ownerId}}<br>{{call.last.dispatchKey}}<br>run {{call.last.runId}}<br>worker {{call.last.workerId??'无子工作者'}}<br>{{call.last.costPhase}} · {{call.last.terminalState}}</code></details>
   </article>
  </div>
  <details v-if="replay.gaps.length"><summary>查看已保存的采集缺口</summary><p v-for="gap in replay.gaps" :key="gap.ownerId">{{stageLabel(gap.failedStage)}} · 采集写入失败（{{gap.code}}）<br><code>{{gap.ownerId}} · 已保存 {{gap.afterOrdinal}} 个阶段 · {{gap.time}}</code></p></details>
  <p v-if="!loading&&!readFailed&&!replay.available">此轮尚无可核验的 SDK 诊断归属，不能推断是否已经调用模型。</p>
 </section>
</template>
<style scoped>
.native-sdk-log{display:grid;gap:12px;min-width:0;color:var(--app-ink,#25282d)}
.native-sdk-log header{display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap}
.native-sdk-log header>div{display:grid;gap:5px}.native-sdk-log strong{font-size:14px}
.native-sdk-log p{margin:0;font-size:12px;line-height:1.65;color:var(--app-muted,#717780);overflow-wrap:anywhere}
.native-sdk-log [role="alert"]{color:#d16666}.sdk-boundary{padding:12px 14px;border:1px solid var(--line,#e3e5e9);border-radius:10px;background:var(--app-surface,#fff)}
.sdk-rows{display:grid;gap:12px;max-height:440px;overflow:auto;scrollbar-width:thin;padding:2px}
.sdk-rows article{min-width:0;padding:16px;border:1px solid var(--line,#e3e5e9);border-radius:12px;background:var(--app-surface,#fff);font-size:12px}
.sdk-call-heading>div{display:flex!important;align-items:baseline;gap:10px!important}.sdk-call-heading b{font-weight:650}
.sdk-call-heading span,.sdk-cost{color:var(--app-muted,#717780);font-size:11px}
.sdk-call-state{padding:4px 9px;border-radius:6px;background:color-mix(in srgb,var(--app-muted,#717780) 10%,transparent)}
[data-state="returned"] .sdk-call-state{color:var(--app-ink,#25282d)}
[data-state="uncertain"] .sdk-call-state,[data-state="withheld"] .sdk-call-state{color:#b38140;background:color-mix(in srgb,#b38140 12%,transparent)}
.sdk-stage-trail{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px 18px;margin:16px 0;padding:0;list-style:none}
.sdk-stage-trail li{display:flex;gap:9px;align-items:flex-start;min-width:0}.sdk-stage-trail li>div{display:grid;gap:3px;min-width:0}
.sdk-stage-trail b{font-size:11px;font-weight:550}.sdk-stage-dot{width:6px;height:6px;flex:0 0 6px;margin-top:5px;border-radius:50%;background:var(--app-muted,#717780)}
.sdk-stage-trail time{color:var(--app-muted,#717780);font-size:10px;overflow-wrap:anywhere}.sdk-cost{display:block;margin:0 0 10px;line-height:1.6}
.native-sdk-log details{color:var(--app-muted,#717780);font-size:11px;line-height:1.6}.native-sdk-log summary{cursor:pointer}
.native-sdk-log code{display:block;overflow-wrap:anywhere;white-space:pre-wrap;font-size:10px;margin-top:8px}
.sdk-rows:focus-visible,.native-sdk-log summary:focus-visible{outline:2px solid var(--blue,#2878ff);outline-offset:3px}
@media(max-width:600px){.sdk-stage-trail{grid-template-columns:1fr}.sdk-rows article{padding:13px}.sdk-call-heading{align-items:flex-start!important}}
</style>
