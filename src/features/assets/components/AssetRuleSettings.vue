<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { RefreshCw, ShieldBan, Search } from '@lucide/vue';
import { assetApi } from '../api';
import { useI18n } from '../../../i18n';
import type { AssetRuleCatalog } from '../types';
import type { Project } from '../../workspaces/types';
import PageIsolationDialog from './PageIsolationDialog.vue';

const props=defineProps<{projects:Project[];selectedProjectId?:number}>();
const emit=defineEmits<{notify:[type:'success'|'error'|'info',text:string];changed:[]}>();
const {tr}=useI18n();
const catalog=ref<AssetRuleCatalog>({projectRules:[],legacyRules:[]});
const project=ref<number|undefined>();
const text=ref('');const busy=ref(false);const error=ref('');
const manageProject=ref<number>();
const projectName=(id:number)=>props.projects.find(p=>p.id===id)?.name||`#${id}`;
const matches=(...values:string[])=>values.join(' ').toLowerCase().includes(text.value.trim().toLowerCase());
const projectRules=computed(()=>catalog.value.projectRules.filter(r=>(!project.value||r.projectId===project.value)&&matches(r.value,r.exampleTitle,projectName(r.projectId))));
const legacyRules=computed(()=>catalog.value.legacyRules.filter(r=>matches(r.keyword,r.exampleTitle,r.exampleUrl)));
async function refresh() {
  if(busy.value)return;busy.value=true;error.value='';
  try {catalog.value=await assetApi.getAssetRuleCatalog();}catch(e){error.value=String(e);}finally{busy.value=false;}
}
async function changed(){await refresh();emit('changed');}
onMounted(refresh);
</script>

<template>
  <section class="rule-settings panel">
    <header class="panel-heading"><div><span class="eyebrow">PAGE EXCLUSIONS</span><h3>{{tr('页面与划词规则','Page and text rules')}}</h3><p>{{tr('集中查看所有项目规则及以前划词保存的全局规则。此处打开时不会应用或迁移规则。','All project rules and previously saved global text rules. Opening this area does not apply or migrate rules.')}}</p></div><button class="button ghost compact" :disabled="busy" @click="refresh"><RefreshCw :size="14"/>{{tr('刷新','Refresh')}}</button></header>
    <div class="rule-search"><Search :size="16"/><input v-model="text" :placeholder="tr('搜索规则、示例标题或项目','Search rules, examples or projects')"/><select v-model="project"><option :value="undefined">{{tr('全部项目规则','All project rules')}}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{p.name}}</option></select></div>
    <p v-if="error" role="alert" class="rule-error">{{error}}</p>
    <p v-if="busy" role="status" class="rule-help">{{tr('读取已保存规则…','Loading saved rules…')}}</p>
    <section class="rule-section">
      <div class="rule-section-heading"><div><h4>{{tr('项目页面隔离','Project page exclusions')}} <span>{{catalog.projectRules.length}}</span></h4><p>{{tr('只作用于所属项目；停用时恢复该规则拥有的状态，人工操作和其他规则优先。新规则请从资产示例划词添加。','Applies only within its project. Disabling restores owned state while preserving manual changes and other rules. Create rules from an asset example.')}}</p></div></div>
      <p v-if="!busy&&!projectRules.length" class="rule-empty">{{tr('没有匹配的项目规则','No matching project rules')}}</p>
      <article v-for="rule in projectRules" :key="rule.id" class="saved-rule"><div class="rule-icon"><ShieldBan :size="17"/></div><div class="rule-copy"><strong>{{rule.kind==='title_contains'?rule.value:rule.exampleTitle||tr('完整正文规则','Complete body rule')}}</strong><span>{{projectName(rule.projectId)}} · {{rule.kind==='title_contains'?tr('标题包含','Title contains'):tr('完整正文相同','Same complete body')}}</span><small v-if="rule.kind==='body_sha256'">SHA-256: {{rule.value}}</small></div><span class="rule-state" :class="{enabled:rule.enabled}">{{rule.enabled?tr('已启用','Enabled'):tr('已停用','Disabled')}}</span><button class="button ghost compact" @click="manageProject=rule.projectId">{{tr('查看与管理','View and manage')}}</button></article>
    </section>
    <section class="rule-section">
      <div class="rule-section-heading"><div><h4>{{tr('以前划词添加的全局规则','Previously saved global text rules')}} <span>{{catalog.legacyRules.length}}</span></h4><p>{{tr('历史规则作用于所有项目，也曾写入运行方案的负面关键词。这里完整展示启用与停用记录；历史变更缺少原状态备份，本轮不会自动迁移、撤销或删除。下方项目选择不缩小全局规则范围。','Historical rules affect all projects and were also copied to profile negative keywords. All enabled and disabled rules are listed. Without original-state backups, this view does not migrate, undo or delete historical effects. The project filter does not restrict global rules.')}}</p></div></div>
      <p v-if="!busy&&!legacyRules.length" class="rule-empty">{{tr('没有匹配的历史全局规则','No matching saved global rules')}}</p>
      <article v-for="rule in legacyRules" :key="rule.id" class="saved-rule"><div class="rule-icon legacy"><ShieldBan :size="17"/></div><div class="rule-copy"><strong>{{rule.keyword}}</strong><span>{{tr('全局 · 标题包含','Global · title contains')}} · #{{rule.id}} · {{rule.createdAt}}</span><small v-if="rule.exampleTitle">{{tr('来源示例：','Example: ')}}{{rule.exampleTitle}}</small><small v-if="rule.exampleUrl">{{rule.exampleUrl}}</small></div><span class="rule-state" :class="{enabled:rule.enabled}">{{rule.enabled?tr('已启用','Enabled'):tr('已停用','Disabled')}}</span></article>
    </section>
    <PageIsolationDialog v-if="manageProject" :projects="projects" :project-id="manageProject" @close="manageProject=undefined" @changed="changed" @notify="(type,value)=>emit('notify',type,value)"/>
  </section>
</template>

<style scoped>
.rule-settings {padding:24px;}.panel-heading p,.rule-section-heading p,.rule-help {font-size:12px;line-height:1.7;color:var(--muted);max-width:800px;}
.rule-search {display:flex;align-items:center;gap:10px;border:1px solid var(--line);border-radius:10px;padding:9px 12px;margin:20px 0;color:var(--muted);}
.rule-search input {flex:1;min-width:120px;border:0;background:transparent;color:var(--app-ink);outline:0;}.rule-search select {max-width:230px;border:1px solid var(--line);padding:7px;border-radius:7px;background:var(--app-surface);color:var(--app-ink);}
.rule-section+.rule-section {margin-top:28px;border-top:1px solid var(--line);padding-top:22px;}.rule-section-heading h4 {margin:0;font-size:14px;}.rule-section-heading h4 span {margin-left:8px;color:var(--muted);font-weight:400;}
.saved-rule {display:flex;align-items:center;gap:13px;padding:15px 0;border-bottom:1px solid var(--line);}.rule-icon {display:grid;place-items:center;width:34px;height:34px;flex-shrink:0;border-radius:9px;background:color-mix(in srgb,var(--blue) 8%,var(--app-surface));color:var(--blue);}.rule-icon.legacy {color:var(--muted);background:color-mix(in srgb,var(--muted) 8%,var(--app-surface));}
.rule-copy {display:grid;gap:5px;flex:1;min-width:0;}.rule-copy strong {font-size:13px;overflow-wrap:anywhere;}.rule-copy span,.rule-copy small {font-size:11px;color:var(--muted);overflow-wrap:anywhere;}.rule-state {font-size:11px;color:var(--muted);white-space:nowrap;border:1px solid var(--line);border-radius:20px;padding:4px 9px;}.rule-state.enabled {color:var(--blue);border-color:color-mix(in srgb,var(--blue) 25%,var(--line));}.rule-empty {padding:16px;font-size:12px;color:var(--muted);background:color-mix(in srgb,var(--muted) 4%,var(--app-surface));border-radius:9px;}.rule-error {color:var(--red);font-size:12px;}
@media(max-width:700px){.rule-search {flex-wrap:wrap;}.saved-rule {flex-wrap:wrap;}.rule-copy {min-width:180px;}}
</style>
