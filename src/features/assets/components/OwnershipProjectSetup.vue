<script lang="ts">
// O1 项目预填闭环（设计 §4）：首屏只展示项目类型、集团/单位卡、范围摘要和“查看建议”；
// 接受建议只进入按项目保留的草稿，保存时以草稿创建时看到的目录版本/绑定版本做 CAS。
// 目录只记录组织事实；绑定与目录都不授予任何扫描或暴露面权限。
interface DraftEntity { draftKey: string; legalName: string; entityKind: string; parentKey: string; regionCode: string; source: string; sourceRef: string; origin: string }
interface DraftIdentifier { targetKey: string; kind: string; value: string; source: string; sourceRef: string; origin: string }
interface DraftBinding { projectKind: "group_ledger" | "unit"; groupKey: string; unitKey: string }
interface DirectoryDraft { baseDirectoryVersion: number; baseBindingVersion: number; entities: DraftEntity[]; identifiers: DraftIdentifier[]; binding: DraftBinding | null }
const directoryDrafts = new Map<number, DirectoryDraft>();
let draftSeq = 0;
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import {
  ORG_ENTITY_KIND_LABELS, SUGGESTION_STATE_LABELS, ownershipDirectoryApi,
  type OrgEntityRef, type OwnershipSuggestion, type OwnershipSuggestionPreview, type ProjectOwnershipBootstrap,
} from "../ownershipDirectoryApi";

// refreshToken：门禁页保存档案或写入归属结论后递增，只刷新本卡片的档案/授权摘要，不动目录草稿。
const props = defineProps<{ projectId: number; projectName: string; refreshToken?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; directorySaved: [projectId: number] }>();

const bootstrap = ref<ProjectOwnershipBootstrap | null>(null);
const preview = ref<OwnershipSuggestionPreview | null>(null);
const draft = ref<DirectoryDraft | null>(null);
const showSuggestions = ref(false);
const loadingBootstrap = ref(false);
const loadingPreview = ref(false);
const savingDirectory = ref(false);
const conflict = ref("");
// 项目代次：切换项目或卸载时递增；响应回来先核对 projectId + 代次，过期结果直接丢弃。
let generation = 0;
const isCurrent = (projectId: number, gen: number) => gen === generation && projectId === props.projectId;

const READINESS_LABELS: Record<string, string> = { rules_only: "仅确定规则", model_not_ready: "模型未接入", directory_unbound: "未绑定组织目录", profile_unsaved: "档案未保存" };
const FIELD_LABELS: Record<string, string> = { legal_name: "主体正式名称", group: "关联集团", member_entity: "成员单位", alias: "简称/品牌", official_domain: "官方域名（精确主机）", network_clue: "网络线索", domain_clue: "域名线索" };
const FIELD_ORDER = ["legal_name", "group", "member_entity", "alias", "official_domain", "domain_clue", "network_clue"];
const IDENTIFIER_KINDS: Record<string, string> = { brand: "品牌", short_name: "简称", english_name: "英文名", historical_name: "历史名称", official_domain: "官方域名", shared_domain: "共享域名" };

const ready = computed(() => !!bootstrap.value && bootstrap.value.projectId === props.projectId);
const entityName = (key: string) => {
  if (!key) return "未选择";
  if (key.startsWith("id:")) return bootstrap.value?.entities.find(item => item.id === Number(key.slice(3)))?.legalName ?? `目录实体 #${key.slice(3)}`;
  const found = draft.value?.entities.find(item => item.draftKey === key.slice(6));
  return found ? `${found.legalName}（草稿）` : "草稿实体已移除";
};
const effectiveBinding = computed<DraftBinding | null>(() => {
  if (draft.value?.binding) return draft.value.binding;
  const saved = bootstrap.value?.binding;
  return saved ? { projectKind: saved.projectKind, groupKey: `id:${saved.groupEntityId}`, unitKey: saved.unitEntityId ? `id:${saved.unitEntityId}` : "" } : null;
});
const groupOptions = computed(() => [
  ...(bootstrap.value?.entities.filter(item => item.entityKind === "group").map(item => ({ key: `id:${item.id}`, label: item.legalName })) ?? []),
  ...(draft.value?.entities.filter(item => item.entityKind === "group").map(item => ({ key: `draft:${item.draftKey}`, label: `${item.legalName}（草稿）` })) ?? []),
]);
const groupIdOf = (key: string) => key.startsWith("id:") ? Number(key.slice(3)) : null;
const entityOptions = computed(() => {
  const groupKey = effectiveBinding.value?.groupKey ?? "";
  const groupId = groupIdOf(groupKey);
  return [
    ...(bootstrap.value?.entities.filter(item => groupId !== null && (item.id === groupId || item.groupId === groupId)).map(item => ({ key: `id:${item.id}`, label: `${item.legalName} · ${ORG_ENTITY_KIND_LABELS[item.entityKind] ?? item.entityKind}` })) ?? []),
    ...(draft.value?.entities.map(item => ({ key: `draft:${item.draftKey}`, label: `${item.legalName}（草稿）· ${ORG_ENTITY_KIND_LABELS[item.entityKind] ?? item.entityKind}` })) ?? []),
  ];
});
const unitOptions = computed(() => entityOptions.value.filter(item => item.key !== effectiveBinding.value?.groupKey && !groupOptions.value.some(group => group.key === item.key)));
const accepted = computed(() => new Set([...(draft.value?.entities.map(item => item.origin) ?? []), ...(draft.value?.identifiers.map(item => item.origin) ?? [])]));
const draftCount = computed(() => (draft.value?.entities.length ?? 0) + (draft.value?.identifiers.length ?? 0) + (draft.value?.binding ? 1 : 0));
// 批量接受（§4“批量接受建议”）只对成员单位、简称/品牌、精确主机域名开放；集团需逐个输入正式名称。
const BATCH_FIELDS = ["member_entity", "alias", "official_domain"];
const scopeDomainCount = computed(() => {
  const current = bootstrap.value; if (!current) return 0;
  const scope = new Set(current.scopeEntityIds);
  return current.entities.filter(item => scope.has(item.id)).reduce((sum, item) => sum + item.identifiers.filter(identifier => identifier.kind === "official_domain" || identifier.kind === "shared_domain").length, 0);
});
const groupedSuggestions = computed(() => FIELD_ORDER.map(field => ({ field, items: preview.value?.suggestions.filter(item => item.field === field) ?? [] })).filter(group => group.items.length));

function enterProject() {
  generation++;
  bootstrap.value = null; preview.value = null; showSuggestions.value = false; conflict.value = "";
  draft.value = directoryDrafts.get(props.projectId) ?? null;
  void loadBootstrap();
}
async function loadBootstrap() {
  const projectId = props.projectId; const gen = generation;
  loadingBootstrap.value = true;
  try {
    const next = await ownershipDirectoryApi.getProjectOwnershipBootstrap(projectId);
    if (isCurrent(projectId, gen)) bootstrap.value = next;
  } catch (error) { if (isCurrent(projectId, gen)) emit("notify", "error", String(error)); }
  finally { if (gen === generation) loadingBootstrap.value = false; }
}
async function loadPreview() {
  const projectId = props.projectId; const gen = generation;
  loadingPreview.value = true;
  try {
    const next = await ownershipDirectoryApi.previewProjectOwnershipSuggestions(projectId);
    if (isCurrent(projectId, gen)) preview.value = next;
  } catch (error) { if (isCurrent(projectId, gen)) emit("notify", "error", String(error)); }
  finally { if (gen === generation) loadingPreview.value = false; }
}
function toggleSuggestions() {
  showSuggestions.value = !showSuggestions.value;
  if (showSuggestions.value && !preview.value) void loadPreview();
}
function ensureDraft(): DirectoryDraft | null {
  if (!ready.value || !bootstrap.value) return null;
  if (!draft.value) {
    draft.value = { baseDirectoryVersion: bootstrap.value.directoryVersion, baseBindingVersion: bootstrap.value.binding?.bindingVersion ?? 0, entities: [], identifiers: [], binding: null };
    directoryDrafts.set(props.projectId, draft.value);
  }
  return draft.value;
}
function pruneDraft() {
  if (draft.value && !draftCount.value) { directoryDrafts.delete(props.projectId); draft.value = null; }
}
function setBinding(patch: Partial<DraftBinding>) {
  const current = ensureDraft(); if (!current) return;
  const base = current.binding ?? effectiveBinding.value ?? { projectKind: "group_ledger" as const, groupKey: "", unitKey: "" };
  const next = { ...base, ...patch };
  if (next.projectKind === "group_ledger") next.unitKey = "";
  current.binding = next;
}
const sourceRefOf = (item: OwnershipSuggestion) => `project:${props.projectId}; ${item.source}; rows=${item.supportRows}; conflicts=${item.conflictRows}; examples=${item.examplesRef.join(",")}; dataRevision=${item.dataRevision}`;
function acceptBlockedReason(item: OwnershipSuggestion): string {
  if (accepted.value.has(item.key)) return "已在草稿中";
  if (item.field === "legal_name") return item.state === "saved" ? "" : "项目标题只供参考，不能直接作为法人名称";
  if (item.field === "network_clue") return "网络线索不能生成法人";
  if (item.field === "domain_clue") return "域名线索不能生成法人";
  if (item.state === "conflict") return "冲突需人工核对，不能一键接受";
  if (item.field === "group") return item.state === "saved" ? "" : effectiveBinding.value?.groupKey && item.entityId && effectiveBinding.value.groupKey === `id:${item.entityId}` ? "已选为本项目集团" : "";
  if (item.state !== "pending") return "";
  if (item.field === "member_entity" && !item.parentEntityId && !effectiveBinding.value?.groupKey) return "请先确认集团";
  if (item.field === "official_domain" && !item.entityId) return "缺少目录实体";
  return "";
}
function canAccept(item: OwnershipSuggestion) {
  if (acceptBlockedReason(item)) return false;
  if (item.field === "group") return item.state !== "saved";
  return item.state === "pending";
}
function accept(item: OwnershipSuggestion) {
  if (!canAccept(item)) return;
  const current = ensureDraft(); if (!current) return;
  if (item.field === "group") {
    if (item.entityId) { setBinding({ groupKey: `id:${item.entityId}` }); return; }
    const name = window.prompt("请输入集团正式法定名称（company 标签前缀不一定是法定名称）", item.value)?.trim();
    if (!name) { pruneDraft(); return; }
    const draftKey = `g${++draftSeq}`;
    current.entities.push({ draftKey, legalName: name, entityKind: "group", parentKey: "", regionCode: "", source: "company_label", sourceRef: sourceRefOf(item), origin: item.key });
    if (!effectiveBinding.value?.groupKey) setBinding({ groupKey: `draft:${draftKey}` });
    return;
  }
  if (item.field === "member_entity") {
    const parentKey = item.parentEntityId ? `id:${item.parentEntityId}` : effectiveBinding.value?.groupKey ?? "";
    current.entities.push({ draftKey: `e${++draftSeq}`, legalName: item.value, entityKind: item.entityKind || "unknown", parentKey, regionCode: item.regionClue, source: "company_label", sourceRef: sourceRefOf(item), origin: item.key });
    return;
  }
  if (item.field === "alias") {
    current.identifiers.push({ targetKey: "", kind: "brand", value: item.value, source: "company_label", sourceRef: sourceRefOf(item), origin: item.key });
    return;
  }
  if (item.field === "official_domain" && item.entityId) {
    current.identifiers.push({ targetKey: `id:${item.entityId}`, kind: "official_domain", value: item.value, source: "asset_host", sourceRef: sourceRefOf(item), origin: item.key });
  }
}
// 逐条复用 accept 的全部限制：标题、网络/域名线索、冲突、缺集团的条目仍被拒绝；只进入草稿，不写库。
function acceptAll(field: string) {
  if (!BATCH_FIELDS.includes(field)) return;
  for (const item of preview.value?.suggestions.filter(row => row.field === field) ?? []) if (canAccept(item)) accept(item);
}
function removeEntity(index: number) {
  const current = draft.value; if (!current) return;
  const [removed] = current.entities.splice(index, 1);
  const key = `draft:${removed.draftKey}`;
  for (const entity of current.entities) if (entity.parentKey === key) entity.parentKey = "";
  for (const identifier of current.identifiers) if (identifier.targetKey === key) identifier.targetKey = "";
  if (current.binding?.groupKey === key) current.binding.groupKey = "";
  if (current.binding?.unitKey === key) current.binding.unitKey = "";
  pruneDraft();
}
function removeIdentifier(index: number) { draft.value?.identifiers.splice(index, 1); pruneDraft(); }
function clearBindingDraft() { if (draft.value) { draft.value.binding = null; pruneDraft(); } }
function discardDraft() {
  if (!draft.value || !window.confirm("放弃本项目所有未保存的目录与绑定草稿？")) return;
  directoryDrafts.delete(props.projectId); draft.value = null; conflict.value = "";
}
function rebaseDraft() {
  const current = draft.value; const latest = bootstrap.value;
  if (!current || !latest || !ready.value) return;
  if (!window.confirm(`保留草稿，并改为基于最新目录 v${latest.directoryVersion}、绑定 v${latest.binding?.bindingVersion ?? 0}？\n\n保存时草稿中的绑定会替换他人刚保存的绑定；新增实体若已被他人加入，会因同名被拒绝。请先核对最新目录。`)) return;
  current.baseDirectoryVersion = latest.directoryVersion; current.baseBindingVersion = latest.binding?.bindingVersion ?? 0; conflict.value = "";
}
const toRef = (key: string): OrgEntityRef => key.startsWith("id:") ? { entityId: Number(key.slice(3)) } : { draftKey: key.slice(6) };
function validationError(current: DirectoryDraft): string {
  for (const entity of current.entities) {
    if (!entity.legalName.trim()) return "草稿实体缺少正式名称";
    if (entity.entityKind !== "group" && !entity.parentKey) return `「${entity.legalName}」未选择上级`;
  }
  for (const identifier of current.identifiers) if (!identifier.targetKey) return `「${identifier.value}」未选择所属单位`;
  if (current.binding) {
    if (!current.binding.groupKey) return "绑定缺少集团";
    if (current.binding.projectKind === "unit" && !current.binding.unitKey) return "单位项目必须选择具体单位";
  }
  return "";
}
function orderedEntities(current: DirectoryDraft): DraftEntity[] {
  const result: DraftEntity[] = []; const done = new Set<string>(); let pending = [...current.entities];
  while (pending.length) {
    const ready = pending.filter(entity => !entity.parentKey.startsWith("draft:") || done.has(entity.parentKey.slice(6)));
    if (!ready.length) { result.push(...pending); break; }
    for (const entity of ready) { result.push(entity); done.add(entity.draftKey); }
    pending = pending.filter(entity => !done.has(entity.draftKey));
  }
  return result;
}
async function saveDirectory() {
  const current = draft.value; const projectId = props.projectId; const projectName = props.projectName; const gen = generation;
  if (!current || savingDirectory.value || !ready.value) return;
  const invalid = validationError(current);
  if (invalid) { emit("notify", "error", `${invalid}；未保存`); return; }
  const snapshot = JSON.stringify(current);
  savingDirectory.value = true; conflict.value = "";
  try {
    const result = await ownershipDirectoryApi.saveProjectOwnershipDirectory({
      projectId,
      expectedDirectoryVersion: current.baseDirectoryVersion,
      expectedBindingVersion: current.baseBindingVersion,
      entities: orderedEntities(current).map(entity => ({ draftKey: entity.draftKey, legalName: entity.legalName.trim(), entityKind: entity.entityKind, parent: entity.entityKind === "group" ? undefined : toRef(entity.parentKey), regionCode: entity.regionCode, source: entity.source, sourceRef: entity.sourceRef })),
      identifiers: current.identifiers.map(identifier => ({ entity: toRef(identifier.targetKey), kind: identifier.kind, value: identifier.value.trim(), source: identifier.source, sourceRef: identifier.sourceRef })),
      binding: current.binding ? { projectKind: current.binding.projectKind, group: toRef(current.binding.groupKey), unit: current.binding.projectKind === "unit" ? toRef(current.binding.unitKey) : undefined } : undefined,
    });
    // 只清掉“与提交内容一致”的那份草稿；保存期间又改过的草稿保留，下一次保存会因版本已变而提示。
    const stored = directoryDrafts.get(projectId);
    if (stored && JSON.stringify(stored) === snapshot) directoryDrafts.delete(projectId);
    emit("notify", "success", `「${projectName}」组织目录 v${result.directoryVersion}、项目绑定 v${result.bindingVersion} 已保存；目录与绑定不授予任何扫描或暴露面权限`);
    emit("directorySaved", projectId);
    if (isCurrent(projectId, gen)) {
      draft.value = directoryDrafts.get(projectId) ?? null;
      void loadBootstrap();
      if (showSuggestions.value) void loadPreview();
    }
  } catch (error) {
    const text = String(error);
    if (isCurrent(projectId, gen) && text.includes("VERSION_CONFLICT")) { conflict.value = text; void loadBootstrap(); }
    emit("notify", "error", isCurrent(projectId, gen) ? text : `「${projectName}」${text}`);
  } finally { savingDirectory.value = false; }
}
watch(() => props.projectId, enterProject, { immediate: true });
watch(() => props.refreshToken, () => { if (ready.value) void loadBootstrap(); });
onBeforeUnmount(() => { generation++; });
</script>

<template>
  <section class="panel ownership-setup">
    <header>
      <div><strong>项目归属设置</strong><small>组织目录在项目间共享、确认一次后复用；目录与绑定只说明“属于谁”，不授予任何扫描或暴露面权限。</small></div>
      <span v-if="!ready">{{ loadingBootstrap ? "加载中…" : "未加载" }}</span>
      <span v-else>目录 v{{ bootstrap?.directoryVersion }} · <template v-for="item in bootstrap?.readiness" :key="item"><em class="ownership-setup-tag">{{ READINESS_LABELS[item] ?? item }}</em></template></span>
    </header>
    <div v-if="ready && bootstrap" class="ownership-setup-cards">
      <div class="ownership-setup-card"><small>项目</small><strong>{{ bootstrap.projectName }}</strong><span>#{{ bootstrap.projectId }}（项目标题不作为法人名称）</span></div>
      <div class="ownership-setup-card"><small>项目类型</small>
        <select :value="effectiveBinding?.projectKind ?? ''" @change="setBinding({ projectKind: ($event.target as HTMLSelectElement).value as 'group_ledger' | 'unit' })">
          <option value="" disabled>未选择</option><option value="group_ledger">集团总账</option><option value="unit">单位项目</option>
        </select>
      </div>
      <div class="ownership-setup-card"><small>关联集团</small>
        <select :value="effectiveBinding?.groupKey ?? ''" @change="setBinding({ groupKey: ($event.target as HTMLSelectElement).value, unitKey: '' })">
          <option value="" disabled>{{ groupOptions.length ? "选择已确认集团" : "目录暂无集团，可从建议中确认" }}</option>
          <option v-for="item in groupOptions" :key="item.key" :value="item.key">{{ item.label }}</option>
        </select>
      </div>
      <div v-if="effectiveBinding?.projectKind === 'unit'" class="ownership-setup-card"><small>本项目单位</small>
        <select :value="effectiveBinding?.unitKey ?? ''" @change="setBinding({ unitKey: ($event.target as HTMLSelectElement).value })">
          <option value="" disabled>选择集团下的单位</option>
          <option v-for="item in unitOptions" :key="item.key" :value="item.key">{{ item.label }}</option>
        </select>
      </div>
      <div class="ownership-setup-card"><small>范围摘要</small>
        <strong>{{ bootstrap.scopeEntityIds.length }} 个目录单位</strong>
        <span>本项目已允许 {{ bootstrap.authorization.allowed.toLocaleString() }} 条 · 暴露面可准入 {{ bootstrap.authorization.exposureEligible.toLocaleString() }} 条（仅本项目授权）</span>
        <span v-if="scopeDomainCount">目录域名 {{ scopeDomainCount }} 个：评估时只按精确主机作为证据，不复制为根域名规则</span>
        <span v-if="bootstrap.entitiesTruncated">目录较大，仅列出集团与本项目范围内的单位</span>
      </div>
      <button class="button ghost" :disabled="loadingPreview" @click="toggleSuggestions">{{ showSuggestions ? "收起建议" : loadingPreview ? "生成建议中…" : "查看建议" }}</button>
    </div>
    <div v-if="conflict" class="ownership-bulk"><strong>{{ conflict }}</strong><button @click="rebaseDraft">保留草稿，改为基于最新版本</button><button @click="discardDraft">放弃草稿</button></div>

    <div v-if="showSuggestions && preview" class="ownership-setup-suggestions">
      <p>当前门禁范围 {{ preview.scopeRows.toLocaleString() }} 条，公司标签 {{ preview.labelCount.toLocaleString() }} 类<template v-if="preview.truncatedLabels">（另有 {{ preview.truncatedLabels.toLocaleString() }} 类未展开）</template>，无公司标签 {{ preview.unlabeledRows.toLocaleString() }} 条。标签类别只是导航候选，不是可整体确认的归属组。数据版本 {{ preview.dataRevision }}</p>
      <table>
        <thead><tr><th>类别</th><th>建议值</th><th>状态</th><th>支持 / 冲突行</th><th>依据与线索</th><th></th></tr></thead>
        <tbody>
          <template v-for="group in groupedSuggestions" :key="group.field">
            <tr class="ownership-setup-group"><th colspan="5">{{ FIELD_LABELS[group.field] ?? group.field }} · {{ group.items.length }} 项</th><th><button v-if="BATCH_FIELDS.includes(group.field) && group.items.some(canAccept)" @click="acceptAll(group.field)">全部接受到草稿（{{ group.items.filter(canAccept).length }}）</button></th></tr>
            <tr v-for="item in group.items" :key="item.key">
              <td>{{ FIELD_LABELS[item.field] ?? item.field }}<small v-if="item.entityKind"> · {{ ORG_ENTITY_KIND_LABELS[item.entityKind] ?? item.entityKind }}</small></td>
              <td><strong>{{ item.value }}</strong></td>
              <td><span class="ownership-setup-state" :class="`state-${item.state}`">{{ SUGGESTION_STATE_LABELS[item.state] ?? item.state }}</span></td>
              <td>{{ item.supportRows.toLocaleString() }} / {{ item.conflictRows.toLocaleString() }}</td>
              <td>{{ item.reason }}<small v-if="item.parentHint"> · 候选上级：{{ item.parentHint }}</small><small v-if="item.regionClue"> · 管理地域线索：{{ item.regionClue }}</small><small v-if="item.locationClue"> · 所在地线索：{{ item.locationClue }}</small><small v-if="item.examplesRef.length"> · 示例资产 #{{ item.examplesRef.join(", #") }}</small></td>
              <td><button v-if="canAccept(item)" @click="accept(item)">{{ item.field === "group" && item.entityId ? "选为本项目集团" : "接受到草稿" }}</button><small v-else>{{ acceptBlockedReason(item) }}</small></td>
            </tr>
          </template>
        </tbody>
      </table>
    </div>

    <div v-if="draft && draftCount" class="ownership-setup-draft">
      <header><strong>未保存的目录草稿（{{ draftCount }} 项，基于目录 v{{ draft.baseDirectoryVersion }} / 绑定 v{{ draft.baseBindingVersion }}）</strong>
        <span><button class="button ghost" @click="discardDraft">放弃草稿</button><button class="button primary" :disabled="savingDirectory" @click="saveDirectory">{{ savingDirectory ? "保存中" : "保存目录与绑定" }}</button></span>
      </header>
      <div v-for="(entity, index) in draft.entities" :key="entity.draftKey" class="ownership-setup-row">
        <input v-model="entity.legalName" placeholder="正式法定名称"/>
        <select v-model="entity.entityKind"><option v-for="(label, kind) in ORG_ENTITY_KIND_LABELS" :key="kind" :value="kind">{{ label }}</option></select>
        <select v-if="entity.entityKind !== 'group'" v-model="entity.parentKey"><option value="" disabled>选择上级</option><option v-for="item in entityOptions.filter(option => option.key !== `draft:${entity.draftKey}`)" :key="item.key" :value="item.key">{{ item.label }}</option></select>
        <input v-model="entity.regionCode" placeholder="管理单位所在省/市（可空）"/>
        <button @click="removeEntity(index)">移除</button>
      </div>
      <div v-for="(identifier, index) in draft.identifiers" :key="`${identifier.origin}:${index}`" class="ownership-setup-row">
        <input v-model="identifier.value"/>
        <select v-model="identifier.kind"><option v-for="(label, kind) in IDENTIFIER_KINDS" :key="kind" :value="kind">{{ label }}</option></select>
        <select v-model="identifier.targetKey"><option value="" disabled>选择所属单位</option><option v-for="item in entityOptions" :key="item.key" :value="item.key">{{ item.label }}</option></select>
        <button @click="removeIdentifier(index)">移除</button>
      </div>
      <div v-if="draft.binding" class="ownership-setup-row">
        <span>绑定：{{ draft.binding.projectKind === "unit" ? "单位项目" : "集团总账" }} · 集团 {{ entityName(draft.binding.groupKey) }}<template v-if="draft.binding.projectKind === 'unit'"> · 单位 {{ entityName(draft.binding.unitKey) }}</template>（不复制其他项目授权）</span>
        <button @click="clearBindingDraft">撤销绑定修改</button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.ownership-setup { overflow: hidden; }
.ownership-setup > header, .ownership-setup-draft > header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 15px 18px; border-bottom: 1px solid #e8edf3; background: #fafbfd; }
.ownership-setup > header > div { display: grid; gap: 3px; }
.ownership-setup header strong { font-size: 12px; }
.ownership-setup header small { color: #8491a3; font-size: 9px; }
.ownership-setup > header > span { font: 9px "IBM Plex Mono", monospace; color: #4774ad; }
.ownership-setup-tag { font-style: normal; margin-left: 4px; padding: 1px 6px; border-radius: 6px; background: #eef3fb; }
.ownership-setup-cards { display: flex; flex-wrap: wrap; align-items: stretch; gap: 10px; padding: 14px 18px; }
.ownership-setup-card { display: grid; gap: 4px; min-width: 170px; padding: 10px 12px; border: 1px solid #e3e9f1; border-radius: 9px; background: #fbfcfe; font-size: 11px; }
.ownership-setup-card small { color: #8491a3; font-size: 9px; }
.ownership-setup-card span { color: #627189; font-size: 10px; }
.ownership-setup-card select, .ownership-setup-row select, .ownership-setup-row input { height: 30px; border: 1px solid #dfe6ee; border-radius: 7px; background: #fff; font-size: 11px; padding: 0 8px; }
.ownership-setup-suggestions { padding: 0 18px 14px; overflow-x: auto; }
.ownership-setup-suggestions p { color: #627189; font-size: 10px; margin: 0 0 8px; }
.ownership-setup-suggestions table { width: 100%; border-collapse: collapse; font-size: 11px; }
.ownership-setup-suggestions th, .ownership-setup-suggestions td { padding: 6px 8px; border-bottom: 1px solid #eef2f7; text-align: left; vertical-align: top; }
.ownership-setup-suggestions td small { color: #7b889b; }
.ownership-setup-group th { background: #f6f8fb; color: #4b5a70; font-weight: 600; }
.ownership-setup-state { padding: 1px 6px; border-radius: 6px; background: #eef3fb; white-space: nowrap; }
.ownership-setup-state.state-conflict { background: #fdecec; color: #b03a3a; }
.ownership-setup-state.state-unknown { background: #f1f3f6; color: #66727f; }
.ownership-setup-state.state-directory_confirmed, .ownership-setup-state.state-saved { background: #e9f7ef; color: #23784a; }
.ownership-setup-draft { border-top: 1px solid #e8edf3; }
.ownership-setup-draft > header span { display: flex; gap: 8px; }
.ownership-setup-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px 18px; border-bottom: 1px solid #f1f4f8; font-size: 11px; }
.ownership-setup-row input { min-width: 220px; }
</style>
