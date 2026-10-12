// §E3 — the cluster view. Clusters and members are read page by page: the panel never pulls
// every cluster or every asset into the front end, and an unregistered cluster command is
// reported as missing rather than as "nothing grouped yet".

import { computed, ref } from "vue";
import { exposureErrorText, isUnregisteredCommand, type ExposurePage, type JsonLike } from "./contracts";
import type { ExposureCluster, ExposureClusterMember } from "./api";

export interface ExposureClustersApi {
  pageExposureClusters?: (
    projectId: number,
    kind: string,
    search: string,
    cursor: string,
    pageSize: number,
  ) => Promise<unknown>;
  pageExposureClusterMembers?: (
    projectId: number,
    clusterId: number,
    cursor: string,
    pageSize: number,
  ) => Promise<unknown>;
}

export interface ExposureClustersDeps {
  projectId: () => number | undefined;
  api: ExposureClustersApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
  pageSize?: number;
}

interface MemberPage {
  items: ExposureClusterMember[];
  total: number;
  hasMore: boolean;
  cursor: string;
  loading: boolean;
  error: string;
}

const emptyMembers = (): MemberPage => ({
  items: [],
  total: 0,
  hasMore: false,
  cursor: "",
  loading: false,
  error: "",
});

function readPage<T>(value: unknown): ExposurePage<T> {
  if (typeof value !== "object" || value === null) throw new Error("分页返回结构无效");
  const row = value as Record<string, unknown>;
  if (!Array.isArray(row.items)) throw new Error("分页缺少 items 列表");
  if (typeof row.total !== "number") throw new Error("分页缺少过滤总数");
  if (typeof row.hasMore !== "boolean") throw new Error("分页缺少 hasMore");
  return {
    items: row.items as T[],
    total: row.total,
    hasMore: row.hasMore,
    nextCursor: String(row.nextCursor ?? ""),
    pageSize: Number(row.pageSize ?? 0),
  };
}

export function useExposureClusters(deps: ExposureClustersDeps) {
  const pageSize = deps.pageSize ?? 20;
  const clusters = ref<ExposureCluster[]>([]);
  const total = ref(0);
  const hasMore = ref(false);
  const cursor = ref("");
  const kindFilter = ref("");
  const searchFilter = ref("");
  const loading = ref(false);
  const loadingMore = ref(false);
  const loaded = ref(false);
  const stale = ref(false);
  const available = ref(true);
  const readError = ref("");
  const expanded = ref<number[]>([]);
  const members = ref<Record<number, MemberPage>>({});

  let generation = 0;
  let readProject = 0;

  function discarded(claimed: number, projectId: number): boolean {
    return claimed !== generation || deps.projectId() !== projectId;
  }

  function clearRows(): void {
    clusters.value = [];
    members.value = {};
    expanded.value = [];
    total.value = 0;
    hasMore.value = false;
    cursor.value = "";
    loaded.value = false;
    stale.value = false;
    readError.value = "";
  }

  function clearForNoProject(): void {
    clearRows();
    readProject = 0;
  }

  async function load(): Promise<boolean> {
    const projectId = deps.projectId();
    if (!projectId) {
      clearForNoProject();
      return false;
    }
    if (!deps.api.pageExposureClusters) {
      available.value = false;
      readError.value = "";
      clearForNoProject();
      return false;
    }
    const claimed = ++generation;
    if (projectId !== readProject) {
      // Rows grouped under another workspace must not stay on screen while this one is being
      // read, not even for the duration of one request.
      clearRows();
      readProject = projectId;
    }
    const kind = kindFilter.value;
    const search = searchFilter.value;
    loading.value = true;
    try {
      const page = readPage<ExposureCluster>(
        await deps.api.pageExposureClusters(projectId, kind, search, "", pageSize),
      );
      if (discarded(claimed, projectId)) return false;
      clusters.value = page.items;
      total.value = page.total;
      hasMore.value = page.hasMore;
      cursor.value = page.nextCursor;
      // A new filter or reload invalidates whatever members were expanded: keeping them
      // would show rows read under a different predicate.
      members.value = {};
      expanded.value = [];
      loaded.value = true;
      stale.value = false;
      available.value = true;
      readError.value = "";
      return true;
    } catch (reason) {
      if (discarded(claimed, projectId)) return false;
      const text = exposureErrorText(reason);
      if (isUnregisteredCommand(reason)) available.value = false;
      readError.value = text;
      if (!loaded.value) {
        // Nothing was ever read for this project: the panel has to say it failed, not "0 clusters".
        clusters.value = [];
        total.value = 0;
        hasMore.value = false;
      } else {
        // A refresh failure keeps the last trustworthy snapshot and labels it stale.
        stale.value = true;
        deps.notify("error", `簇列表刷新失败，显示的是上一次的快照：${text}`);
      }
      return false;
    } finally {
      loading.value = false;
    }
  }

  async function loadMore(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !hasMore.value || !cursor.value || loadingMore.value) return;
    if (!deps.api.pageExposureClusters) return;
    const claimed = generation;
    const kind = kindFilter.value;
    const search = searchFilter.value;
    loadingMore.value = true;
    try {
      const page = readPage<ExposureCluster>(
        await deps.api.pageExposureClusters(projectId, kind, search, cursor.value, pageSize),
      );
      if (discarded(claimed, projectId) || kind !== kindFilter.value || search !== searchFilter.value) return;
      const seen = new Set(clusters.value.map((row) => row.id));
      clusters.value = [...clusters.value, ...page.items.filter((row) => !seen.has(row.id))];
      total.value = page.total;
      hasMore.value = page.hasMore;
      cursor.value = page.nextCursor;
    } catch (reason) {
      deps.notify("error", `加载更多簇失败：${exposureErrorText(reason)}`);
    } finally {
      loadingMore.value = false;
    }
  }

  async function loadMembers(clusterId: number, append = false): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !deps.api.pageExposureClusterMembers) return;
    const claimed = generation;
    const previous = members.value[clusterId] ?? emptyMembers();
    if (append && (!previous.cursor || previous.loading)) return;
    members.value = {
      ...members.value,
      [clusterId]: { ...previous, loading: true, error: "" },
    };
    try {
      const page = readPage<ExposureClusterMember>(
        await deps.api.pageExposureClusterMembers(
          projectId,
          clusterId,
          append ? previous.cursor : "",
          pageSize,
        ),
      );
      if (discarded(claimed, projectId)) return;
      const items = append ? [...previous.items, ...page.items] : page.items;
      members.value = {
        ...members.value,
        [clusterId]: {
          items,
          total: page.total,
          hasMore: page.hasMore,
          cursor: page.nextCursor,
          loading: false,
          error: "",
        },
      };
    } catch (reason) {
      if (discarded(claimed, projectId)) return;
      const text = exposureErrorText(reason);
      members.value = {
        ...members.value,
        [clusterId]: { ...previous, loading: false, error: text },
      };
      // A refused cluster read is a real error, never an empty member list.
      deps.notify("error", text);
    }
  }

  async function toggle(clusterId: number): Promise<void> {
    const at = expanded.value.indexOf(clusterId);
    if (at >= 0) {
      expanded.value = expanded.value.filter((row) => row !== clusterId);
      return;
    }
    expanded.value = [...expanded.value, clusterId];
    if (!members.value[clusterId]) await loadMembers(clusterId);
  }

  const summary = computed(() => {
    const direct = clusters.value.reduce((sum, row) => sum + (row.directCount || 0), 0);
    const inherited = clusters.value.reduce((sum, row) => sum + (row.inheritedCount || 0), 0);
    const gaps = clusters.value.filter((row) => row.gapNote).length;
    return { direct, inherited, gaps };
  });

  /** Wording that distinguishes "grouped nothing" from "could not read". */
  const notice = computed(() => {
    if (!deps.projectId()) return "请先选择工作空间";
    if (!available.value) return "簇读取接口尚未注册，这里显示的不是“没有聚类结果”";
    if (readError.value && !loaded.value) return `簇读取失败：${readError.value}`;
    if (stale.value) return `显示的是上一次成功读取的簇快照：${readError.value}`;
    if (!clusters.value.length) {
      return total.value === 0
        ? "还没有满足成簇条件的公开观察（至少两个准入资产共享同一字节或同一注册域）"
        : `当前过滤没有命中，共 ${total.value} 个簇`;
    }
    return `本页 ${clusters.value.length} / 共 ${total.value} 个簇；直接依据 ${summary.value.direct} 项、继承 ${summary.value.inherited} 项、带缺口 ${summary.value.gaps} 个`;
  });

  /// G1-1 which rule drew this cluster's boundary. An old backend that does not report it
  /// is said to be unreported, never assumed to be the public-suffix rule.
  function boundaryText(row: ExposureCluster): string {
    if (row.kind === "content") return "相同响应内容";
    if (typeof row.boundaryBasis !== "string") return "边界依据未报告（旧后端）";
    if (row.boundaryBasis === "") return "旧版末两段分组（G1 之前，未按公共后缀，可能误合并）";
    if (row.boundaryBasis === "psl") return `公共后缀注册域（${row.pslVersion || "版本未记录"}）`;
    if (row.boundaryBasis === "exact_host") return "精确主机（无可注册域）";
    return row.boundaryLabel || `未知边界 ${row.boundaryBasis}`;
  }

  function cacheLabel(row: ExposureCluster): string {
    if (!row.cacheTime) return "尚无缓存结论";
    const expiry = row.cacheExpiresAt ? `，有效期至 ${row.cacheExpiresAt}` : "，未设期限";
    return `缓存于 ${row.cacheTime}${expiry}`;
  }

  function evidenceLabel(member: ExposureClusterMember): string {
    if (!member.admittedNow) return "已失效：不再满足准入";
    if (member.evidenceSource === "direct") return "自带结论";
    if (member.evidenceSource === "stale_representative_revoked") return "代表已撤权，结论待复核";
    return `继承自 ${member.inheritedFromLabel || `资产 ${member.inheritedFromAssetId ?? "?"}`}`;
  }

  function provenance(row: ExposureCluster): JsonLike {
    return {
      clusterKey: row.clusterKey,
      boundaryBasis: row.boundaryBasis ?? null,
      pslVersion: row.pslVersion ?? null,
      ruleVersion: row.ruleVersion,
      settingsVersion: row.settingsVersion,
      representativeRunId: row.representativeRunId,
    };
  }

  return {
    boundaryText,
    clusters,
    members,
    expanded,
    total,
    hasMore,
    loading,
    loadingMore,
    loaded,
    stale,
    available,
    readError,
    kindFilter,
    searchFilter,
    notice,
    summary,
    load,
    loadMore,
    loadMembers,
    toggle,
    cacheLabel,
    evidenceLabel,
    provenance,
  };
}
