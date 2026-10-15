<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { FileText, Play, Plus, Trash2, UploadCloud } from "@lucide/vue";
import { assetApi } from "../api";
import type { ConfigProfile } from "../../configuration/types";
import type { Project } from "../../workspaces/types";
import type { Target } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ projects: Project[]; profiles: ConfigProfile[]; selectedProjectId?: number }>();
const emit = defineEmits<{ started: [runId: number]; projectChange: [id: number]; createProject: []; notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const activeProjects = computed(() => props.projects.filter(project => project.status !== "archived"));
const projectId = ref(activeProjects.value.find(project => project.id === props.selectedProjectId)?.id ?? activeProjects.value[0]?.id);
const profileId = ref(props.profiles.find(p => p.isDefault)?.id ?? props.profiles[0]?.id);
const targetType = ref("auto"); const input = ref(""); const pipeline = ref("full");
const selectedProfile = computed(() => props.profiles.find(profile => profile.id === profileId.value));
// Match the backend boolean defaults; a legacy non-boolean does not disable probing.
const probeEnabled = computed(() => selectedProfile.value?.settings.runProbe !== false);
const refineEnabled = computed(() => selectedProfile.value?.settings.runRefine !== false || probeEnabled.value);
const executionPlan = computed(() => [tr("采集", "Collect"), tr("候选入库", "Import candidates"),
  ...(refineEnabled.value ? [tr("分层", "Refine")] : []), ...(probeEnabled.value ? [tr("探测", "Probe")] : [])].join(" → "));
const jobName = ref(`资产采集 ${new Date().toLocaleDateString()}`); const targets = ref<Target[]>([]); const busy = ref(false);
const lines = computed(() => input.value.split(/\r?\n/).map(v => v.trim()).filter(v => v && !v.startsWith("#")));

async function refreshTargets() { targets.value = projectId.value ? await assetApi.listTargets(projectId.value) : []; }
watch(() => props.selectedProjectId, value => { projectId.value = activeProjects.value.find(project=>project.id===value)?.id ?? activeProjects.value[0]?.id; });
watch(() => props.projects, values => {
  const active=values.filter(project=>project.status!=="archived");
  if (!active.some(project => project.id === projectId.value)) projectId.value = active.find(project=>project.id===props.selectedProjectId)?.id ?? active[0]?.id;
}, { deep: true });
watch(() => props.profiles, values => {
  if (!values.some(profile => profile.id === profileId.value)) profileId.value = values.find(profile => profile.isDefault)?.id ?? values[0]?.id;
}, { deep: true });
watch(projectId, async value => {
  if (value) emit("projectChange", value);
  await refreshTargets();
});
onMounted(refreshTargets);

async function addTargets() {
  if (!projectId.value || !lines.value.length) return;
  try {
    const count = await assetApi.importTargets(projectId.value, targetType.value, lines.value);
    input.value = ""; await refreshTargets(); emit("notify", "success", tr(`已新增 ${count} 个目标`, `${count} targets added`));
  } catch (error) { emit("notify", "error", String(error)); }
}
async function removeTarget(id: number) { await assetApi.removeTarget(id); await refreshTargets(); }
async function loadFile(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]; if (!file) return;
  input.value = await file.text(); (event.target as HTMLInputElement).value = "";
}
async function start() {
  if (!projectId.value || !profileId.value) { emit("notify", "error", tr("请选择项目和配置方案", "Select a project and profile")); return; }
  if (lines.value.length) await addTargets();
  if (!targets.value.length) { emit("notify", "error", tr("请先添加查询目标", "Add at least one target")); return; }
  busy.value = true;
  try {
    const runId = await assetApi.startJob(projectId.value, profileId.value, jobName.value, pipeline.value);
    emit("started", runId); emit("notify", "success", tr(`任务 #${runId} 已启动`, `Job #${runId} started`));
  } catch (error) { emit("notify", "error", String(error)); }
  finally { busy.value = false; }
}
</script>

<template>
  <div class="query-layout">
    <section class="panel query-builder-panel">
      <div class="panel-heading"><div><span class="eyebrow">COLLECTION</span><h2>{{tr('创建资产查询','Create asset query')}}</h2><p>{{tr('每行一个公司名、域名、IP或CIDR。公司与域名命中的同一入口合并发现证据；是否进行存活探测取决于所选配置。来源标签和评分不是归属确认，人工结论和已看记录保持。','One company, domain, IP or CIDR per line. Shared endpoints merge discovery evidence; the selected profile determines whether to probe. Discovery labels and scores are not ownership confirmation. Manual decisions and opened-entry records are preserved.')}}</p></div></div>
      <div class="form-grid two">
        <div class="project-field-with-action"><label class="field"><span>{{tr('归属工作空间','Workspace')}}</span><select v-model="projectId"><option v-for="p in projects.filter(item=>item.status!=='archived')" :key="p.id" :value="p.id">{{ p.name }}</option></select></label><button class="button ghost" type="button" @click="$emit('createProject')"><Plus :size="14" />{{tr('新建','New')}}</button></div>
        <label class="field"><span>{{tr('配置方案','Profile')}}</span><select v-model="profileId"><option v-for="p in profiles" :key="p.id" :value="p.id">{{ p.name }}</option></select></label>
        <label class="field"><span>{{tr('任务名称','Job name')}}</span><input v-model="jobName" /></label>
        <label class="field"><span>{{tr('任务范围','Pipeline')}}</span><select v-model="pipeline"><option value="full">{{tr('采集与处理（默认）','Collect and process (default)')}}</option><option value="collect">{{tr('兼容采集流程','Compatible collection workflow')}}</option></select></label>
      </div>
      <div class="query-execution-plan" :data-query-profile="profileId" :data-probe-plan="probeEnabled ? 'enabled' : 'disabled'" role="status">
        <strong>{{tr('按所选配置执行','Selected profile plan')}}</strong>
        <span>{{selectedProfile ? executionPlan : tr('请选择配置方案','Select a profile')}}</span>
        <small v-if="selectedProfile && !probeEnabled">{{tr('本轮不进行存活探测：入库的是来源候选，不代表当前可访问。','This run does not probe availability. Imported source candidates do not prove current accessibility.')}}</small>
      </div>
      <div class="input-toolbar">
        <div class="segmented">
          <button v-for="item in [['auto',tr('自动识别','Auto')],['company',tr('公司','Company')],['domain',tr('域名','Domain')],['ip','IP'],['cidr','CIDR']]" :key="item[0]" :class="{ active: targetType===item[0] }" @click="targetType=item[0]">{{ item[1] }}</button>
        </div>
        <label class="button ghost compact file-button"><UploadCloud :size="15" /> {{tr('读取 TXT','Import TXT')}}<input type="file" accept=".txt,.csv,text/plain" @change="loadFile" /></label>
      </div>
      <textarea v-model="input" class="target-input" rows="9" placeholder="中国移动通信有限公司&#10;example.com&#10;203.0.113.0/24"></textarea>
      <div class="input-summary"><span><FileText :size="15" /> {{tr(`待添加 ${lines.length} 条`,`${lines.length} pending`)}}</span><span>{{tr('空行与 # 注释自动忽略','Blank lines and # comments are ignored')}}</span></div>
      <div class="query-actions">
        <button class="button secondary" :disabled="!lines.length" @click="addTargets"><Plus :size="16" /> {{tr('添加到项目','Add to project')}}</button>
        <button class="button primary large" :disabled="busy" @click="start"><Play :size="17" /> {{ busy ? tr('启动中…','Starting…') : tr('启动任务','Start job') }}</button>
      </div>
    </section>
    <aside class="panel target-list-panel">
      <div class="panel-heading compact"><div><span class="eyebrow">SCOPE</span><h3>{{tr('项目目标','Project targets')}}</h3></div><span class="count-pill">{{ targets.length }}</span></div>
      <div v-if="targets.length" class="target-list">
        <div v-for="target in targets" :key="target.id" class="target-row">
          <span class="target-type">{{ target.targetType }}</span><span class="target-value">{{ target.value }}</span>
          <button class="icon-button subtle" @click="removeTarget(target.id)"><Trash2 :size="14" /></button>
        </div>
      </div>
      <div v-else class="empty-state small">{{tr('还没有目标','No targets yet')}}<br />{{tr('从左侧输入或导入 TXT','Enter or import TXT on the left')}}</div>
    </aside>
  </div>
</template>

<style scoped>
.query-execution-plan { margin:12px 19px; display:grid; gap:4px; padding:10px 12px; border:1px solid var(--line); border-radius:8px; font-size:11px; line-height:1.6; }
.query-execution-plan strong { font-size:10px; }.query-execution-plan span,.query-execution-plan small { color:var(--muted); overflow-wrap:anywhere; }
</style>
