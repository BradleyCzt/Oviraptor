<script setup lang="ts">
// §E5 — review, remediation and retest for one selected finding. Nothing here rewrites an
// observation: a decision is an event carrying the revision it read, and a retest is a new
// round that has to pass the authorization gate again.
import { onMounted, ref, watch } from "vue";
import { Check, X } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureReview } from "./useExposureReview";
import { DISPOSITION_LABELS, INFO_CATEGORY_LABELS, buildReclassifyRequest } from "./contracts";

const props = defineProps<{ projectId?: number; findingId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const review = useExposureReview({
  projectId: () => props.projectId,
  findingId: () => props.findingId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  trail, remediations, taxonomy, outcome, actor, reason, remediationNote, revision, busy,
  loaded, error, conflict, available, notice, canClose, outcomeLabel, openRemediation,
  latestObservationId, load, review: decide, recordRemediation, retest, mark, remediationBlocked,
  evidence,
} = review;

const reclassifyDisposition = ref("risk_candidate");
const reclassifyCategory = ref("credential");
const reclassifyBusy = ref(false);
const reclassifyNotice = ref("");

function freshOperationId(): string {
  const raw = globalThis.crypto?.randomUUID?.() ?? "";
  const id = `ui${raw.replace(/-/g, "")}`;
  return id.length >= 8 ? id : "ui-reclass-fallback";
}

async function submitReclassify(): Promise<void> {
  const projectId = props.projectId;
  const findingId = props.findingId;
  const current = evidence.value?.disposition;
  if (!projectId || !findingId || !current) return;
  const built = buildReclassifyRequest({
    findingId,
    expectedClassificationId: current.classificationId ?? 0,
    disposition: reclassifyDisposition.value,
    infoCategory: reclassifyCategory.value,
    actor: actor.value,
    reason: reason.value,
  }, freshOperationId());
  if (!built.ok) {
    reclassifyNotice.value = built.error;
    return;
  }
  reclassifyBusy.value = true;
  try {
    await exposureApi.reclassifyExposureFinding(projectId, built.payload);
    reclassifyNotice.value = "已改判（写入审计，不联系任何人）";
    await load();
  } catch (error) {
    reclassifyNotice.value = `改判未写入：${error instanceof Error ? error.message : String(error)}`;
  } finally {
    reclassifyBusy.value = false;
  }
}

onMounted(() => {
  void load();
});
watch(() => [props.projectId, props.findingId], () => void load());
</script>

<template>
  <section class="panel exposure-review">
    <header>
      <div>
        <strong>人工复核与整改复测</strong>
        <small>{{ notice }}</small>
      </div>
      <button class="button ghost" :disabled="!findingId || busy" @click="load">重新读取</button>
    </header>

    <p v-if="!findingId" class="exposure-empty">请先在上方选择要复核的线索（只列出当前已读到的一页）。</p>
    <p v-else-if="!available" class="exposure-error">
      <strong>复核接口尚未注册</strong>
      <span>页面不会把“读不到”显示成“没有复核记录”；等主控注册 review_exposure_finding_scoped 等命令后才能写入结论。</span>
    </p>
    <p v-else-if="error && !loaded" class="exposure-error"><strong>读取失败</strong><span>{{ error }}</span></p>

    <template v-if="findingId">
      <div class="exposure-review-taxonomy">
        <span v-for="row in taxonomy" :key="row.code" :title="row.meaning">
          {{ row.label }} <strong>{{ row.count }}</strong>
        </span>
        <small>每个数字都来自真实行；“证据不足”不等于“没有问题”。</small>
      </div>

      <div class="exposure-review-form" :class="{ pending: !loaded }">
        <label><span>操作者</span><input v-model="actor" placeholder="姓名或工号，写进复核轨迹" /></label>
        <label class="wide"><span>依据（为什么会这么定）</span>
          <textarea v-model="reason" rows="3" placeholder="例：两份响应的字节一致，且路径可公开访问"></textarea>
        </label>
        <p v-if="conflict" class="exposure-error"><strong>队列已前进，本次结论没有写入</strong><span>{{ error }}</span></p>
        <div class="exposure-review-actions">
          <span>按 r{{ revision }} 提交，引用观察 {{ latestObservationId ?? "无" }}</span>
          <button :disabled="busy || !loaded || !available" @click="decide('reviewing')">标记复核中</button>
          <button class="confirm" :disabled="busy || !loaded || !available" @click="decide('confirmed')"><Check :size="12" />确认暴露</button>
          <button :disabled="busy || !loaded || !available" @click="decide('dismissed')"><X :size="12" />排除</button>
        </div>
      </div>

      <div class="exposure-review-remediation">
        <label class="wide"><span>整改说明（绑定这条线索与其观察）</span>
          <textarea v-model="remediationNote" rows="2" placeholder="例：已从镜像删除该文件并轮换凭据"></textarea>
        </label>
        <div class="exposure-review-actions">
          <button v-if="!remediationBlocked" :disabled="busy || !loaded || !available || !remediationNote.trim() || !actor.trim()" @click="recordRemediation">登记整改</button>
          <span v-else class="exposure-gap">{{ remediationBlocked }}</span>
          <form v-if="remediationBlocked" class="exposure-reclassify" @submit.prevent="submitReclassify">
            <strong>人工改判</strong>
            <label>处置
              <select v-model="reclassifyDisposition">
                <option v-for="(label, key) in DISPOSITION_LABELS" :key="key" :value="key">{{ label }}</option>
              </select>
            </label>
            <label>信息类别
              <select v-model="reclassifyCategory">
                <option v-for="(label, key) in INFO_CATEGORY_LABELS" :key="key" :value="key">{{ label }}</option>
              </select>
            </label>
            <button type="submit" :disabled="reclassifyBusy || !loaded">写入改判</button>
            <small v-if="reclassifyNotice">{{ reclassifyNotice }}</small>
          </form>
          <button :disabled="busy || !openRemediation" @click="retest">另起一轮复测</button>
          <button class="confirm" :disabled="busy || !loaded || !openRemediation || !canClose || !actor.trim()" @click="mark('closed')">按依据关闭</button>
          <button :disabled="busy || !openRemediation" @click="mark('rejected')">驳回</button>
        </div>
        <p v-if="openRemediation" class="exposure-review-state">
          整改 #{{ openRemediation.id }} · {{ openRemediation.status }} · 操作者 {{ openRemediation.actor }} ·
          来源轮次 {{ openRemediation.runId }} · 复测轮 {{ openRemediation.retestRunIds.join("/") || "无" }}
        </p>
        <p v-if="!openRemediation" class="exposure-empty">这条线索还没有整改记录</p>
        <p class="exposure-review-outcome">
          复测依据：{{ outcomeLabel }}
          <template v-if="outcome?.note"> · {{ outcome.note }}</template>
          <template v-if="outcome?.previousHash"> · 原 {{ outcome.previousHash.slice(0, 8) }} → 新 {{ (outcome.currentHash || "无").slice(0, 8) }}</template>
        </p>
        <p v-if="outcome && !canClose" class="exposure-gap">关闭必须拿到复测依据（例如内容确实变了）；“本轮没取到”只说明没取到。</p>
      </div>

      <div class="exposure-review-trail">
        <strong>复核轨迹（追加，不覆盖）</strong>
        <div v-for="(row, index) in trail" :key="index">
          <span>{{ row.createdAt }} · {{ row.actor }} · {{ row.fromStatus || "初始" }} → {{ row.toStatus }}（按 r{{ row.expectedRevision }}）</span>
          <small>{{ row.note }}<template v-if="row.references?.length"> · 引用 {{ row.references.join(",") }}</template></small>
        </div>
        <p v-if="!trail.length">还没有人工复核事件</p>
      </div>

      <div v-if="remediations.length" class="exposure-review-list">
        <strong>本项目整改</strong>
        <div v-for="row in remediations" :key="row.id">
          <span>#{{ row.id }} · {{ row.findingTitle }} · {{ row.status }}</span>
          <small>观察 {{ row.observationId ?? "无" }} · 轮次 {{ row.runId }} · {{ row.actor }} · {{ row.createdAt }}{{ row.closedAt ? ` · 关闭于 ${row.closedAt}` : "" }}</small>
        </div>
      </div>
    </template>
  </section>
</template>
