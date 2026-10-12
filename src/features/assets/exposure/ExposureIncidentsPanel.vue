<script setup lang="ts">
// G4 — the incident ledger panel: deduplicated incidents, their locations, unit links and
// action trail. Handling is recorded only; nothing on this panel contacts anyone.
import { computed, ref, watch } from "vue";
import { exposureIncidentApi } from "./api";
import { useExposureIncidents } from "./useExposureIncidents";
import {
  BASIS_LABELS,
  EVIDENCE_GRADE_LABELS,
  HANDLING_ACTIONS,
  HANDLING_LABELS,
  HANDLING_NOTICE,
  INCIDENT_STATUS_LABELS,
  LINK_LEVEL_LABELS,
  type ActionForm,
} from "./incidents";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [kind: "success" | "error" | "info", text: string] }>();

const incidents = useExposureIncidents({
  projectId: () => props.projectId,
  api: exposureIncidentApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  items, hasMore, statusFilter, reviewNeededOnly, loading, available, detail, acting, notice,
  locations, locationsLoading, selectedLocations, mergeTarget,
  load, loadMore, open, act, proposeHistory, loadMoreLocations, toggleLocation, readMergeTarget, merge, split,
} = incidents;

const actor = ref("");
const reason = ref("");
const reviewStatus = ref("reviewing");
const handling = ref("notification_recorded");
const channel = ref("");
const externalReference = ref("");
const retestRunId = ref<number | undefined>(undefined);
const entityId = ref<number | undefined>(undefined);
const linkLevel = ref("candidate_link");
const evidenceIds = ref("");
const mergeTargetId = ref<number | undefined>(undefined);

watch(() => props.projectId, () => void load(), { immediate: true });

const reviewStatuses = Object.entries(INCIDENT_STATUS_LABELS).filter(([key]) => key !== "merged");
const entities = computed(() => (detail.value?.entities as Record<string, unknown>[] | undefined) ?? []);
const events = computed(() => (detail.value?.events as Record<string, unknown>[] | undefined) ?? []);

async function submit(kind: ActionForm["action"]): Promise<void> {
  const form: ActionForm = { action: kind, actor: actor.value, reason: reason.value };
  if (kind === "review") form.status = reviewStatus.value;
  if (kind === "record_action") {
    form.handling = handling.value;
    form.channel = channel.value;
    form.externalReference = externalReference.value;
    form.retestRunId = retestRunId.value;
  }
  if (kind === "link_entity") {
    form.entityId = entityId.value;
    form.linkLevel = linkLevel.value;
    form.evidenceObservationIds = evidenceIds.value.split(/[,，\s]+/).filter(Boolean).map(Number).filter(Number.isInteger);
  }
  if (await act(form)) reason.value = "";
}

async function submitSplit(): Promise<void> {
  if (await split(actor.value, reason.value)) reason.value = "";
}

async function submitMerge(): Promise<void> {
  if (await merge(actor.value, reason.value)) {
    reason.value = "";
    mergeTargetId.value = undefined;
  }
}
</script>

<template>
  <section class="panel exposure-incidents">
    <header>
      <div><strong>事件台账</strong><small>{{ notice }}</small></div>
      <label v-if="available">
        <span>状态</span>
        <select v-model="statusFilter" @change="load()">
          <option value="open">未关闭（候选/复核中/已核实）</option>
          <option value="">全部（不含已合并）</option>
          <option v-for="(label, key) in INCIDENT_STATUS_LABELS" :key="key" :value="key">{{ label }}</option>
        </select>
      </label>
      <label v-if="available" class="exposure-check"><input v-model="reviewNeededOnly" type="checkbox" @change="load()" />只看需重新复核</label>
      <button v-if="available" type="button" :disabled="loading" @click="proposeHistory">为历史轮次补建议</button>
    </header>
    <p class="exposure-gap">{{ HANDLING_NOTICE }}</p>
    <div v-for="item in items" :key="item.id" :class="['exposure-incident-row', { selected: detail && detail.id === item.id }]">
      <strong>#{{ item.id }} {{ item.title }}</strong>
      <span>{{ item.line }}</span>
      <em v-if="item.reviewNeeded" class="exposure-gap">有新位置或内容变化，需要重新复核</em>
      <button @click="open(item.id)">查看</button>
    </div>
    <div v-if="loading" class="exposure-empty">正在加载…</div>
    <footer v-if="hasMore" class="exposure-more"><button :disabled="loading" @click="loadMore">加载更多事件</button></footer>

    <article v-if="detail" class="exposure-incident-detail">
      <h3>#{{ detail.id }} {{ detail.title }}（v{{ detail.revision }}）</h3>
      <p>
        {{ INCIDENT_STATUS_LABELS[String(detail.status)] ?? detail.status }} · 对应旧线索状态 {{ detail.legacyFindingStatus || "—" }}
        · 依据 {{ BASIS_LABELS[String(detail.basis)] ?? detail.basis }} · {{ EVIDENCE_GRADE_LABELS[String(detail.evidenceGrade)] ?? detail.evidenceGrade }}
        · {{ HANDLING_LABELS[String(detail.handlingState)] ?? detail.handlingState }}
      </p>
      <p v-if="detail.reviewNeeded" class="exposure-gap">{{ detail.reviewNeededReason }}</p>
      <h4>传播位置（已显示 {{ locations.rows.length }} / 共 {{ locations.total }}）</h4>
      <div v-for="row in locations.rows" :key="String(row.id)" class="exposure-incident-location">
        <label v-if="available" class="exposure-check">
          <input
            type="checkbox"
            :data-location="String(row.id)"
            :checked="selectedLocations.includes(Number(row.id))"
            @change="toggleLocation(Number(row.id))"
          />拆出
        </label>
        <span>{{ row.locationKind }} · {{ row.collectionMode }} · {{ row.canonicalUrl }}</span>
        <small>第 {{ row.runId }} 轮 · 观察 {{ row.observationId ?? "—" }} · {{ row.observedAt }}<template v-if="row.indexedAt"> · 收录 {{ row.indexedAt }}</template><template v-if="row.truncated"> · 已截断</template></small>
      </div>
      <footer v-if="locations.hasMore" class="exposure-more">
        <button data-action="more-locations" :disabled="locationsLoading" @click="loadMoreLocations">加载更多位置（还有 {{ locations.total - locations.rows.length }} 条）</button>
      </footer>
      <h4>涉及单位</h4>
      <div v-for="row in entities" :key="String(row.entityId) + String(row.origin)">
        {{ row.legalName }} · {{ LINK_LEVEL_LABELS[String(row.linkLevel)] ?? row.linkLevel }} · {{ row.origin }} · 目录 v{{ row.directoryVersion }}
      </div>
      <div v-if="!entities.length" class="exposure-empty">单位未知（没有权威的逐资产单位归属，不按名称或地区推断）</div>
      <h4>动作记录</h4>
      <div v-for="row in events" :key="String(row.id)"><small>{{ row.createdAt }} · {{ row.actor }} · {{ row.action }} · {{ row.reason }}</small></div>
      <p v-if="detail.eventsTruncated" class="exposure-gap">只显示最近 50 条动作</p>

      <div v-if="available" class="exposure-incident-actions">
        <label><span>操作人</span><input v-model="actor" /></label>
        <label class="wide"><span>理由（必填）</span><input v-model="reason" /></label>
        <div>
          <select v-model="reviewStatus">
            <option v-for="row in reviewStatuses" :key="row[0]" :value="row[0]">{{ row[1] }}</option>
          </select>
          <button :disabled="acting" @click="submit('review')">写入复核结论</button>
        </div>
        <div>
          <select v-model="handling">
            <option v-for="row in HANDLING_ACTIONS" :key="row.value" :value="row.value">{{ row.label }}</option>
          </select>
          <input v-model="channel" placeholder="通知对象与渠道（人工已完成）" />
          <input v-model="externalReference" placeholder="外部工单号（可选）" />
          <input v-model.number="retestRunId" type="number" placeholder="复测轮次号" />
          <button :disabled="acting" @click="submit('record_action')">登记</button>
        </div>
        <div>
          <span>已勾选 {{ selectedLocations.length }} 个位置</span>
          <button data-action="split" :disabled="acting || !selectedLocations.length" @click="submitSplit">拆分为新事件（单位关联随证据带走）</button>
        </div>
        <div>
          <input v-model.number="mergeTargetId" data-field="merge-target" type="number" placeholder="并入的目标事件编号" />
          <button data-action="read-target" :disabled="acting" @click="readMergeTarget(Number(mergeTargetId))">读取目标事件</button>
          <small v-if="mergeTarget">目标 #{{ mergeTarget.id }} {{ mergeTarget.title }}（v{{ mergeTarget.revision }}，{{ INCIDENT_STATUS_LABELS[mergeTarget.status] ?? mergeTarget.status }}）</small>
          <button data-action="merge" :disabled="acting || !mergeTarget" @click="submitMerge">合并到目标事件</button>
        </div>
        <div>
          <input v-model.number="entityId" type="number" placeholder="组织目录实体编号" />
          <select v-model="linkLevel"><option v-for="(label, key) in LINK_LEVEL_LABELS" :key="key" :value="key">{{ label }}</option></select>
          <input v-model="evidenceIds" placeholder="证据观察编号（确认关联必填）" />
          <button :disabled="acting" @click="submit('link_entity')">关联单位</button>
        </div>
      </div>
    </article>
  </section>
</template>
