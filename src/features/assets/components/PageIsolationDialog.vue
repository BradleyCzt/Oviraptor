<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { ShieldBan, ScanSearch } from "@lucide/vue";
import { assetApi } from "../api";
import type { Asset, PageIsolationPreview, PageIsolationRule, PageIsolationRuleInput } from "../types";
import type { Project } from "../../workspaces/types";
import { useI18n } from "../../../i18n";
import ModalShell from "../../../shared/components/ModalShell.vue";

const props = defineProps<{ projects: Project[]; asset?: Asset; projectId?: number; keyword?: string }>();
const emit = defineEmits<{ close: []; changed: []; notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const project = ref(props.asset?.projectId ?? props.projectId);
const hasBody = computed(() => props.asset?.extra?.probe_body_complete === "true" && props.asset?.extra?.probe_body_truncated === "false" && typeof props.asset?.extra?.body_sha256 === "string");
const kind = ref<PageIsolationRuleInput["kind"]>(props.keyword || !hasBody.value ? "title_contains" : "body_sha256");
const keyword = ref((props.keyword ?? props.asset?.title ?? "").slice(0,200));
const rules = ref<PageIsolationRule[]>([]);
const preview = ref<PageIsolationPreview>();
const busy = ref(false);
const error = ref("");
let revision = 0;
watch([project, kind, keyword], () => { preview.value = undefined; error.value = ""; revision++; });
async function loadRules() {
  const id = project.value;
  rules.value = [];
  if (!id) return;
  try { const value = await assetApi.listPageRules(id); if (project.value === id) rules.value = value; }
  catch (e) { if (project.value === id) error.value = String(e); }
}
watch(project, loadRules);
onMounted(loadRules);
async function inspect() {
  if (!props.asset || !project.value || busy.value) return;
  busy.value = true; error.value = "";
  const current = ++revision;
  try {
    const result = await assetApi.previewPageRule({ projectId: project.value, sourceAssetId: props.asset.id, kind: kind.value, value: keyword.value });
    if (current === revision) preview.value = result;
  } catch (e) { if (current === revision) error.value = String(e); }
  finally { busy.value = false; }
}
async function apply() {
  const inspected = preview.value;
  if (!inspected || busy.value) return;
  busy.value = true; error.value = "";
  try {
    const result = await assetApi.applyPageRule(inspected.rule, inspected.previewToken);
    preview.value = undefined;
    emit("changed");
    emit("notify", "success", tr(`规则已启用并隔离 ${result.isolated} 条，保护 ${result.protected} 条；原记录已备份`, `Rule enabled; ${result.isolated} isolated and ${result.protected} protected. Original records backed up`));
    await loadRules();
  } catch (e) { preview.value = undefined; error.value = String(e); }
  finally { busy.value = false; }
}
async function disable(rule: PageIsolationRule) {
  if (!project.value || busy.value) return;
  busy.value = true; error.value = ""; preview.value = undefined; revision++;
  try {
    const result = await assetApi.disablePageRule(project.value, rule.id);
    emit("changed");
    emit("notify", "success", tr(`规则已停用，恢复 ${result.restored} 条，保留 ${result.retained} 条（人工状态或其他规则）`, `Rule disabled; ${result.restored} restored, ${result.retained} retained due to manual state or another rule`));
    await loadRules();
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
</script>

<template>
  <ModalShell :title="tr('页面隔离规则','Page isolation rules')" wide @close="!busy && emit('close')">
    <template #eyebrow><span class="eyebrow">PAGE RULES</span></template>
    <div class="page-rules">
      <p class="rule-intro">{{tr('将无用页面从正常队列移出，全部命中都隔离，不保留代表。原资产与 URL 保留，可恢复。','Remove unwanted pages from the normal queue. Isolate every match, including representatives. Original assets and URLs remain recoverable.')}}</p>
      <label class="rule-field"><span>{{tr('仅应用于项目','Apply only to project')}}</span><select v-model="project" :disabled="!!asset || busy"><option :value="undefined" disabled>{{tr('选择项目','Select project')}}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{p.name}}</option></select></label>
      <section v-if="asset" class="rule-create">
        <div class="rule-source"><span>{{tr('示例页面','Example page')}}</span><strong>{{asset.title || tr('无标题','Untitled')}}</strong><small>{{asset.link || asset.host}}</small></div>
        <label class="rule-field"><span>{{tr('匹配方式','Match by')}}</span><select v-model="kind" :disabled="busy"><option value="title_contains">{{tr('标题包含','Title contains')}}</option><option value="body_sha256" :disabled="!hasBody">{{tr('完整正文相同','Same complete body')}}</option></select></label>
        <label v-if="kind==='title_contains'" class="rule-field"><span>{{tr('标题中的特征文字','Distinctive text in title')}}</span><input v-model="keyword" maxlength="200" :disabled="busy" :placeholder="tr('例如 OneNet 或 Welcome to nginx','For example OneNet or Welcome to nginx')" /></label>
        <p class="rule-help">{{kind==='title_contains' ? tr('匹配标题中的这段文字，正文可以不同；建议使用明确的页面特征。','Matches this text in titles even when bodies differ. Use a distinctive page phrase.') : tr('忽略 IP、端口、状态码和响应类型，只匹配完整正文。正文仍有差异时不会命中。','Ignores IP, port, status and response type; requires the same complete body. Different bodies remain separate.')}}</p>
        <p v-if="!hasBody" class="rule-help">{{tr('旧记录缺少完整正文证明：可按标题预览，或先复测后使用正文规则。','This record lacks complete-body proof. Preview a title rule, or reprobe before using a body rule.')}}</p>
        <button class="button ghost compact" :disabled="busy || (kind==='title_contains' && keyword.trim().length<2)" @click="inspect"><ScanSearch :size="15" /> {{busy?tr('处理中…','Working…'):tr('预览全部命中','Preview all matches')}}</button>
        <div v-if="preview" class="rule-preview">
          <div class="rule-counts"><div><span>{{tr('命中','Matches')}}</span><strong>{{preview.matched.toLocaleString()}}</strong></div><div><span>{{tr('将隔离','To isolate')}}</span><strong>{{preview.eligible.toLocaleString()}}</strong></div><div><span>{{tr('受保护','Protected')}}</span><strong>{{preview.protected.toLocaleString()}}</strong></div><div><span>{{tr('已在回收站','Already archived')}}</span><strong>{{preview.alreadyIsolated.toLocaleString()}}</strong></div></div>
          <p class="rule-help">{{tr('人工结论、人工备注、手动恢复、归属授权和已有调查优先保留。已看时间保持。应用前保存原状态备份。','Manual decisions, notes, restorations, ownership authorization and investigations take priority. Opened-entry times are retained. Original state is backed up before applying.')}}</p>
          <ul class="rule-samples"><li v-for="row in preview.sample" :key="row.assetId"><div><strong>{{row.title || tr('无标题','Untitled')}}</strong><small>{{row.link}}</small></div><span>{{row.alreadyIsolated?tr('已隔离','Archived'):row.eligible?tr('将隔离','Isolate'):tr('保留','Keep')}}</span></li></ul>
          <p v-if="preview.matched>preview.sample.length" class="rule-help">{{tr(`展示前 ${preview.sample.length} 条，计数和应用包含全部命中。`,`Showing ${preview.sample.length} examples; counts and application include every match.`)}}</p>
          <button class="button primary" :disabled="busy || !preview.matched" @click="apply"><ShieldBan :size="15" /> {{tr(`启用规则并隔离 ${preview.eligible} 条`,`Enable rule and isolate ${preview.eligible}`)}}</button>
        </div>
      </section>
      <p v-else class="rule-help">{{tr('在资产列表选择一条示例，点击“隔离此类页面”新建规则。','Select an example asset and choose “Isolate this page type” to create a rule.')}}</p>
      <p v-if="error" role="alert" class="rule-error">{{error}}</p>
      <section class="rule-list"><h3>{{tr('当前项目规则','Project rules')}}</h3><p v-if="!rules.length" class="rule-help">{{tr('暂无规则','No rules yet')}}</p><article v-for="rule in rules" :key="rule.id"><div><strong>{{rule.kind==='title_contains'?tr('标题包含','Title contains'):tr('完整正文相同','Same complete body')}}</strong><span>{{rule.kind==='title_contains'?rule.value:(rule.exampleTitle || tr('已选页面正文','Selected page body'))}}</span><small>{{rule.enabled?tr('已启用 · 后续入库自动隔离','Enabled · automatically applied on subsequent imports'):tr('已停用','Disabled')}}</small></div><button v-if="rule.enabled" class="button ghost compact" :disabled="busy" @click="disable(rule)">{{tr('停用并恢复','Disable and restore')}}</button></article></section>
    </div>
    <template #footer><button class="button ghost" :disabled="busy" @click="emit('close')">{{tr('关闭','Close')}}</button></template>
  </ModalShell>
</template>

<style scoped>
.page-rules { display:grid; gap:18px; }
.rule-intro,.rule-help { color:var(--muted); font-size:12px; line-height:1.7; margin:0; }
.rule-field { display:grid; gap:7px; font-size:12px; }
.rule-field input,.rule-field select { width:100%; padding:10px 12px; border:1px solid var(--line); border-radius:9px; background:var(--app-surface); color:var(--app-ink,#172033); }
.rule-create { display:grid; gap:12px; padding:16px; border:1px solid var(--line); border-radius:12px; }
.rule-source,.rule-list article>div { display:grid; gap:5px; min-width:0; }
.rule-source span,.rule-source small,.rule-list small { color:var(--muted); font-size:11px; overflow-wrap:anywhere; }
.rule-source strong { font-size:13px; }
.rule-preview { display:grid; gap:12px; border-top:1px solid var(--line); padding-top:14px; }
.rule-counts { display:grid; grid-template-columns:repeat(4,1fr); gap:8px; }
.rule-counts>div { display:grid; gap:7px; padding:12px; border-radius:9px; background:color-mix(in srgb,var(--blue) 5%,var(--app-surface)); }
.rule-counts span { color:var(--muted); font-size:11px; }.rule-counts strong { font-size:21px; }
.rule-samples { padding:0; margin:0; list-style:none; max-height:230px; overflow:auto; }
.rule-samples li { display:flex; justify-content:space-between; gap:16px; padding:10px 0; border-bottom:1px solid var(--line); font-size:12px; }
.rule-samples li>div { display:grid; gap:3px; min-width:0; }.rule-samples small { color:var(--muted); overflow-wrap:anywhere; }.rule-samples li>span { flex-shrink:0; color:var(--muted); }
.rule-error { color:var(--red,#c43a3a); font-size:12px; overflow-wrap:anywhere; }.rule-list h3 { font-size:13px; margin:0 0 10px; }
.rule-list article { display:flex; align-items:center; gap:16px; justify-content:space-between; border-top:1px solid var(--line); padding:12px 0; font-size:12px; }
.rule-list article>div>span { color:var(--muted); overflow-wrap:anywhere; max-width:450px; }.rule-list button { flex-shrink:0; }
@media(max-width:600px) { .rule-counts {grid-template-columns:repeat(2,1fr);} .rule-list article {align-items:flex-start;flex-direction:column;} }
</style>
