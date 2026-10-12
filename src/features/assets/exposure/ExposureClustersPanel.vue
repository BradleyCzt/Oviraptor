<script setup lang="ts">
// §E3 — the exposure surface's cluster view. It reads clusters page by page, expands one
// cluster's members on demand, and always separates a direct conclusion from an inherited one
// and from a member that lost admission. Nothing here can start, resume or cancel a round.
import { onMounted, ref, watch } from "vue";
import { Search } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureClusters } from "./useExposureClusters";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const clusters = useExposureClusters({
  projectId: () => props.projectId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  clusters: rows, members, expanded, total, hasMore, loading, loadingMore, stale, available,
  readError, kindFilter, searchFilter, notice, summary,
  load, loadMore, loadMembers, toggle, cacheLabel, evidenceLabel, boundaryText,
} = clusters;

const showProvenance = ref<number | null>(null);

onMounted(() => {
  void load();
});
watch(() => props.projectId, () => void load());
</script>

<template>
  <section class="panel exposure-clusters">
    <header>
      <div>
        <strong>内容簇与域名簇</strong>
        <small>{{ notice }}</small>
      </div>
      <div class="exposure-clusters-tools">
        <label><span>类型</span>
          <select v-model="kindFilter" @change="load">
            <option value="">全部</option>
            <option value="content">同字节内容簇</option>
            <option value="domain">同注册域簇</option>
          </select>
        </label>
        <label><Search :size="14" /><input v-model="searchFilter" placeholder="搜索簇键或成员资产" @keyup.enter="load" /></label>
        <button class="button ghost" :disabled="!projectId || loading" @click="load">重新读取</button>
      </div>
    </header>

    <p v-if="!available" class="exposure-error">
      <strong>簇读取接口尚未注册</strong>
      <span>页面不会把“读不到”显示成“没有聚类结果”；等主控注册 page_exposure_clusters 与 page_exposure_cluster_members 后才能给出真实簇。</span>
    </p>
    <p v-else-if="readError && !rows.length" class="exposure-error">
      <strong>簇读取失败</strong><span>{{ readError }}</span>
    </p>
    <p v-else-if="stale" class="exposure-stale"><strong>显示的是上一次成功读取的簇快照</strong><span>{{ readError }}</span></p>

    <p class="exposure-clusters-bounds">簇只是分组：既不授予归属，也不授予授权；继承的成员没有自己的验证时间。</p>

    <p v-if="rows.length" class="exposure-clusters-count">
      本页 {{ rows.length }} / 共 {{ total }} 个簇 · 直接依据 {{ summary.direct }} · 继承 {{ summary.inherited }} · 带缺口 {{ summary.gaps }}
    </p>

    <article v-for="row in rows" :key="row.id" class="exposure-cluster">
      <div class="exposure-cluster-head">
        <span class="exposure-cluster-kind" :class="row.kind">{{ row.kind === "content" ? "同字节" : row.boundaryBasis === "exact_host" ? "同主机" : "同注册域" }}</span>
        <strong>{{ row.representativeLabel || `资产 ${row.representativeAssetId ?? "?"}` }}</strong>
        <em>{{ row.memberCount }} 个成员（直接 {{ row.directCount }} / 继承 {{ row.inheritedCount }}）</em>
      </div>
      <small>
        代表来自第 {{ row.representativeRunId }} 轮 · 观察 #{{ row.representativeObservationId ?? "无" }} ·
        规则 {{ row.ruleVersion || "未记录" }} · 配置 v{{ row.settingsVersion }} · 分组于 {{ row.updatedAt }}
      </small>
      <small class="exposure-cluster-boundary">分组边界：{{ boundaryText(row) }}</small>
      <small class="exposure-cluster-cache">{{ cacheLabel(row) }}</small>
      <p v-if="row.gapNote" class="exposure-gap">{{ row.gapNote }}</p>
      <div class="exposure-cluster-actions">
        <button @click="toggle(row.id)">{{ expanded.includes(row.id) ? "收起成员" : "展开成员" }}</button>
        <button @click="showProvenance = showProvenance === row.id ? null : row.id">依据</button>
      </div>
      <div v-if="showProvenance === row.id" class="exposure-cluster-provenance">
        <span>簇键 {{ row.clusterKey }}</span>
        <span>代表观察必须存在才写 direct；其余成员显示“继承自…”，并可指回被继承的资产与轮次。</span>
      </div>
      <div v-if="expanded.includes(row.id)" class="exposure-cluster-members">
        <p v-if="members[row.id]?.error" class="exposure-error"><strong>成员读取失败</strong><span>{{ members[row.id]?.error }}</span></p>
        <p v-if="members[row.id]?.loading" class="exposure-empty">正在加载成员…</p>
        <ul v-else-if="members[row.id]?.items.length">
          <li v-for="member in members[row.id].items" :key="member.assetId" :class="{ revoked: !member.admittedNow }">
            <strong>{{ member.label || `资产 ${member.assetId}` }}</strong>
            <span>{{ evidenceLabel(member) }}</span>
            <small>
              {{ member.membership === "representative" ? "代表" : "成员" }} ·
              观察 #{{ member.observationId ?? "无" }} ·
              {{ member.verifiedAt ? `自带结论于 ${member.verifiedAt}` : "没有自己的验证时间" }}
              <template v-if="member.inheritedRunId"> · 继承轮次 {{ member.inheritedRunId }}</template>
            </small>
          </li>
        </ul>
        <p v-else-if="members[row.id] && !members[row.id].loading" class="exposure-empty">这个簇还没有可读的成员行</p>
        <button
          v-if="members[row.id]?.hasMore"
          class="button ghost"
          :disabled="members[row.id]?.loading"
          @click="loadMembers(row.id, true)"
        >
          {{ members[row.id]?.loading ? "正在加载…" : `加载更多成员（已读 ${members[row.id]?.items.length ?? 0} / ${members[row.id]?.total ?? 0}）` }}
        </button>
      </div>
    </article>

    <div v-if="loading && !rows.length" class="exposure-empty">正在加载簇…</div>
    <footer v-if="hasMore" class="exposure-more">
      <button class="button ghost" :disabled="loadingMore" @click="loadMore">
        {{ loadingMore ? "正在加载…" : `加载更多簇（已读 ${rows.length} / ${total}）` }}
      </button>
    </footer>
  </section>
</template>
