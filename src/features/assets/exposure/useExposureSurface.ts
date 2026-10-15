// §Q0 — the exposure page's read state, in one place so the races are explicit.
// Every read carries a generation token and the project it was issued for: a late answer
// from an older project or an older run can never overwrite what is on screen, a failed
// refresh keeps the last trustworthy snapshot and marks it stale, and polling stops when
// the page is hidden or unmounted. Nothing here starts a collection on its own.

import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { Asset, AssetPage, AssetQuery, ExposureFinding, ExposureRun, ExposureSourceResult } from "../types";
import {
  classificationLine,
  dispositionBadge,
  DISPOSITION_FILTERS,
  readDispositionMap,
  type ExposureDispositionView,
  describeLimits,
  describePassiveScope,
  unitProjectionLines,
  exposureErrorText,
  isFindingRow,
  isUnregisteredCommand,
  isRunRow,
  isSourceRow,
  readExposurePage,
  readScopeSummary,
  legacyWindowNotice,
  type ExposurePage,
  type ExposureFindingDraft,
  type ExposureFindingPageInput,
  type ExposureScopeSummary,
  exportLine,
} from "./contracts";

export interface ExposureSurfaceApi {
  exposureScopeSummary: (projectId: number) => Promise<unknown>;
  pageExposureFindings: (input: ExposureFindingPageInput) => Promise<unknown>;
  pageExposureRuns: (projectId: number, pageSize: number, cursor: string) => Promise<unknown>;
  listExposureSourceResultsForProject: (runId: number, expectedProjectId: number) => Promise<unknown>;
  // The pre-Q1 endpoints, kept so the page still works until the paginated commands are
  // registered in `lib.rs` by the coordinator. When absent, the page just fails normally.
  exposureSummary?: (projectId: number) => Promise<unknown>;
  listExposureFindings?: (projectId: number, status: string, search: string) => Promise<unknown>;
  listExposureRuns?: (projectId: number) => Promise<unknown>;
  listExposureSourceResults?: (runId: number, expectedProjectId: number) => Promise<unknown>;
  listAssets: (query: AssetQuery) => Promise<AssetPage>;
  startExposureScan: (projectId: number) => Promise<number>;
  cancelExposureScan: (projectId: number, runId: number) => Promise<void>;
  saveExposureFinding: (input: ExposureFindingDraft) => Promise<number>;
  exportExposureFindings: (projectId: number, kind?: "findings" | "ledger") => Promise<{ path: string; rows: number }>;
  /** G4-3 reads the landed file back for its digest and row count. */
  verifyExposureExport?: (path: string) => Promise<unknown>;
}

export interface ExposureSurfaceDeps {
  projectId: () => number | undefined;
  /// Whether the page is still mounted and visible to the user.
  active: () => boolean;
  api: ExposureSurfaceApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
}

const FINDING_PAGE_SIZE = 50;
const ASSET_PAGE_SIZE = 100;
/// A picker that walks 100 rows at a time; 20 pages is the honest ceiling of this slice,
/// and reaching it is reported instead of pretending the list was complete.
const ASSET_PAGE_LIMIT = 20;
const POLL_INTERVAL_MS = 2500;

function isRecordLike(value: unknown): value is object {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function useExposureSurface(deps: ExposureSurfaceDeps) {
  const summary = ref<ExposureScopeSummary | null>(null);
  const findings = ref<ExposureFinding[]>([]);
  const runs = ref<ExposureRun[]>([]);
  const sources = ref<ExposureSourceResult[]>([]);
  const boundAssets = ref<Asset[]>([]);
  const selectedRunId = ref<number | null>(null);
  const statusFilter = ref("");
  const searchFilter = ref("");
  const findingsTotal = ref(0);
  const nextCursor = ref("");
  const hasMore = ref(false);
  const loading = ref(false);
  const loadingMore = ref(false);
  const stale = ref(false);
  const readError = ref("");
  /// False until one read has actually landed. Without it a first failed load would render
  /// the "no findings for this filter" empty state, which is exactly the false blank §E1
  /// forbids: the list was never read, it is not empty.
  const hasSnapshot = ref(false);
  const boundAssetsTotal = ref(0);
  const boundAssetsTruncated = ref(false);
  const starting = ref(false);
  /// False once the paginated commands turn out not to be registered yet: the page then
  /// reads the legacy endpoints and says which window they cover.
  const pagedReadsAvailable = ref(true);

  let generation = 0;
  /// The bound-asset picker is its own series: refreshing the finding list must not
  /// abandon a page walk that is already half done.
  let assetSeries = 0;
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let inFlight = false;
  let mounted = false;

  const activeRun = computed(
    () => runs.value.find((row) => ["queued", "running"].includes(row.status)) ?? null,
  );
  const scopeNotice = computed(() => describePassiveScope(summary.value));
  const knownLimits = computed(() => summary.value?.knownLimits ?? []);
  const limitsNotice = computed(() => describeLimits(summary.value));
  const dispositionFilter = ref("");
  /// G1-3 finding id → disposition; null = the backend did not report it.
  const dispositions = ref<Record<number, ExposureDispositionView> | null>(null);
  const classificationNotice = computed(() => classificationLine(summary.value));
  const dispositionFilters = DISPOSITION_FILTERS;
  const badgeFor = (findingId: number) => dispositionBadge(dispositions.value, findingId);
  const unitLines = computed(() =>
    summary.value ? unitProjectionLines(summary.value.unitProjection, summary.value.unitProjectionError) : [],
  );
  /// G4-3 the last export line (rows + digest read back from the landed file).
  const lastExport = ref("");
  const beyondPlanNotice = computed(() => {
    const current = summary.value;
    if (!current) return "";
    if (current.limitsError) return `采集上限不可用：${current.limitsError}`;
    if (current.assetsBeyondPlan > 0) {
      return `准入资产 ${current.eligibleAssetsTotal} 个，本轮计划只取 ${current.eligibleAssetsPlanned} 个，另有 ${current.assetsBeyondPlan} 个未采集`;
    }
    if (!current.projectActive) return "工作空间已归档，本轮没有采集范围";
    return "";
  });

  /// A result is only allowed to touch the screen when it is still the newest read for
  /// the same project and the page is still mounted.
  function discarded(claimed: number, projectId: number): boolean {
    return !mounted || claimed !== generation || deps.projectId() !== projectId;
  }

  function clearForNoProject(): void {
    summary.value = null;
    findings.value = [];
    runs.value = [];
    sources.value = [];
    boundAssets.value = [];
    selectedRunId.value = null;
    findingsTotal.value = 0;
    nextCursor.value = "";
    hasMore.value = false;
    stale.value = false;
    readError.value = "";
    hasSnapshot.value = false;
    boundAssetsTotal.value = 0;
    boundAssetsTruncated.value = false;
  }

  async function showSourcesFor(
    runId: number | null,
    claimed: number,
    projectId: number,
  ): Promise<void> {
    selectedRunId.value = runId;
    if (runId === null) {
      if (!discarded(claimed, projectId)) sources.value = [];
      return;
    }
    try {
      // Whichever way the finding list was read, the sources follow the same mode: the
      // scoped endpoint when it exists, the legacy one when it does not.
      const scoped = await (async () => {
        if (!pagedReadsAvailable.value && deps.api.listExposureSourceResults) {
          return deps.api.listExposureSourceResults(runId, projectId);
        }
        try {
          return await deps.api.listExposureSourceResultsForProject(runId, projectId);
        } catch (error) {
          if (!isUnregisteredCommand(error) || !deps.api.listExposureSourceResults) throw error;
          if (!discarded(claimed, projectId)) pagedReadsAvailable.value = false;
          return deps.api.listExposureSourceResults(runId, projectId);
        }
      })();
      if (discarded(claimed, projectId)) return;
      const rows = scoped;
      if (!Array.isArray(rows) || !rows.every(isSourceRow)) {
        throw new Error("数据源响应结构无效");
      }
      sources.value = rows as ExposureSourceResult[];
    } catch (error) {
      if (discarded(claimed, projectId)) return;
      sources.value = [];
      stale.value = true;
      readError.value = exposureErrorText(error);
    }
  }

  async function readPaged(projectId: number): Promise<{
    summary: ExposureScopeSummary;
    findings: ExposurePage<ExposureFinding>;
    runs: ExposurePage<ExposureRun>;
    dispositions: Record<number, ExposureDispositionView> | null;
  }> {
    const [summaryValue, [findingsValue, dispositionValue], runsValue] = await Promise.all([
      deps.api.exposureScopeSummary(projectId).then(readScopeSummary),
      deps.api
        .pageExposureFindings({
          projectId,
          status: statusFilter.value,
          search: searchFilter.value,
          pageSize: FINDING_PAGE_SIZE,
          cursor: "",
          disposition: dispositionFilter.value,
        })
        .then(
          (value) =>
            [
              readExposurePage(value, isFindingRow, "暴露线索"),
              readDispositionMap((value as { dispositions?: unknown } | null)?.dispositions),
            ] as const,
        ),
      deps.api
        .pageExposureRuns(projectId, FINDING_PAGE_SIZE, "")
        .then((value) => readExposurePage(value, isRunRow, "采集任务")),
    ]);
    return { summary: summaryValue, findings: findingsValue, runs: runsValue, dispositions: dispositionValue };
  }

  /// The pre-Q1 endpoints: no cursor, no membership re-check, and a silent window of the
  /// first 500 findings / last 20 runs. Used only while the paginated commands are not
  /// registered, and always labelled as such on the page.
  async function readLegacy(projectId: number): Promise<{
    summary: ExposureScopeSummary;
    findings: ExposurePage<ExposureFinding>;
    runs: ExposurePage<ExposureRun>;
    dispositions: Record<number, ExposureDispositionView> | null;
  }> {
    const [rawSummary, rawFindings, rawRuns] = await Promise.all([
      deps.api.exposureSummary?.(projectId) ?? Promise.reject(new Error("legacy summary 不可用")),
      deps.api.listExposureFindings?.(projectId, statusFilter.value, searchFilter.value) ?? Promise.resolve([]),
      deps.api.listExposureRuns?.(projectId) ?? Promise.resolve([]),
    ]);
    if (!isRecordLike(rawSummary)) throw new Error("旧范围汇总结构无效");
    const number = (key: string) => Number((rawSummary as Record<string, unknown>)[key]) || 0;
    const eligible = number("eligibleAssets");
    const rows = Array.isArray(rawFindings) ? rawFindings.filter(isFindingRow) : [];
    const runRows = Array.isArray(rawRuns) ? rawRuns.filter(isRunRow) : [];
    return {
      summary: {
        projectId,
        projectActive: true,
        eligibleAssetsTotal: eligible,
        // G1-0: the legacy endpoint knows no project cap; G0 invented 200 here.
        eligibleAssetsPlanned: eligible,
        assetsBeyondPlan: 0,
        findingsTotal: number("total"),
        findingsNew: number("newCount"),
        findingsReviewing: number("reviewing"),
        findingsConfirmed: number("confirmed"),
        findingsDismissed: number("dismissed"),
        highRiskOpen: number("highRisk"),
        knownLimits: [],
        limitsBasis: "legacy_unverified",
        limitsError: "旧汇总接口不报告项目的工作项上限，计划数未核实",
        unitProjection: null,
        unitProjectionError: "",
        findingsExcluded: null,
        findingsClassificationPending: null,
        findingsUnclassified: null,
        classifyPolicyVersion: "",
        liveRun: null,
        liveRunError: "",
        coverage: null,
        coverageError: "旧汇总接口不报告覆盖矩阵",
        unitStatistics: null,
        unitStatisticsError: "旧汇总接口不报告单位统计",
      },
      dispositions: null,
      findings: { items: rows, nextCursor: "", hasMore: false, total: rows.length, pageSize: rows.length },
      runs: { items: runRows, nextCursor: "", hasMore: false, total: runRows.length, pageSize: runRows.length },
    };
  }

  async function load(): Promise<void> {
    const claimed = ++generation;
    const projectId = deps.projectId();
    if (!projectId) {
      clearForNoProject();
      return;
    }
    inFlight = true;
    loading.value = true;
    try {
      let next;
      try {
        next = await readPaged(projectId);
        if (!discarded(claimed, projectId)) pagedReadsAvailable.value = true;
      } catch (error) {
        // Only an unregistered command may step down to the legacy window.
        if (!isUnregisteredCommand(error) || !deps.api.listExposureFindings) throw error;
        next = await readLegacy(projectId);
        if (!discarded(claimed, projectId)) pagedReadsAvailable.value = false;
      }
      if (discarded(claimed, projectId)) return;
      summary.value = next.summary;
      findings.value = next.findings.items;
      dispositions.value = next.dispositions;
      findingsTotal.value = next.findings.total;
      hasMore.value = next.findings.hasMore;
      nextCursor.value = next.findings.hasMore ? next.findings.nextCursor : "";
      runs.value = next.runs.items;
      const kept =
        selectedRunId.value !== null && next.runs.items.some((row) => row.id === selectedRunId.value)
          ? selectedRunId.value
          : (next.runs.items[0]?.id ?? null);
      await showSourcesFor(kept, claimed, projectId);
      if (discarded(claimed, projectId)) return;
      stale.value = false;
      readError.value = "";
      hasSnapshot.value = true;
    } catch (error) {
      // A late or superseded failure must not mark the *current* project stale.
      if (discarded(claimed, projectId)) return;
      stale.value = true;
      readError.value = exposureErrorText(error);
      // The notification is deliberately generic; the panel shows the stripped reason.
      deps.notify("error", "暴露面读取失败，仍显示上次可信快照");
    } finally {
      if (!discarded(claimed, projectId)) loading.value = false;
      inFlight = false;
    }
  }

  /// The next page of findings, appended to what is already on screen.
  async function loadMore(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !hasMore.value || loadingMore.value) return;
    const claimed = generation;
    loadingMore.value = true;
    try {
      const page = await deps.api
        .pageExposureFindings({
          projectId,
          status: statusFilter.value,
          search: searchFilter.value,
          pageSize: FINDING_PAGE_SIZE,
          cursor: nextCursor.value,
          disposition: dispositionFilter.value,
        })
        .then((value) => {
          const more = readDispositionMap((value as { dispositions?: unknown } | null)?.dispositions);
          return { ...readExposurePage(value, isFindingRow, "暴露线索"), more };
        });
      if (discarded(claimed, projectId)) return;
      dispositions.value = page.more === null || dispositions.value === null ? null : { ...dispositions.value, ...page.more };
      const seen = new Set(findings.value.map((row) => row.id));
      findings.value = [...findings.value, ...page.items.filter((row) => !seen.has(row.id))];
      findingsTotal.value = page.total;
      hasMore.value = page.hasMore;
      nextCursor.value = page.hasMore ? page.nextCursor : "";
    } catch (error) {
      if (discarded(claimed, projectId)) return;
      deps.notify("error", `下一页读取失败，已有结果保持不变：${exposureErrorText(error)}`);
    } finally {
      loadingMore.value = false;
    }
  }

  /// The bound-asset picker walks pages: a single request cannot be assumed to be the
  /// whole scope, and a short read is labelled as truncated instead of complete.
  async function loadBoundAssets(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) return;
    const series = ++assetSeries;
    const collected: Asset[] = [];
    let total = 0;
    let pages = 0;
    try {
      while (pages < ASSET_PAGE_LIMIT) {
        pages += 1;
        const page = await deps.api.listAssets({
          projectId,
          search: "",
          conditions: [],
          page: pages,
          pageSize: ASSET_PAGE_SIZE,
          includeDeleted: false,
          deletedView: "active",
          probeView: "all",
          probeOutcomeView: "all",
          sentinelView: "all",
          decisionView: "all",
          ownershipView: "eligible",
          sortBy: "priority",
          sortDirection: "desc",
        } as AssetQuery);
        if (series !== assetSeries || !mounted || deps.projectId() !== projectId) return;
        total = page?.total ?? 0;
        collected.push(...(page?.items ?? []));
        if (collected.length >= total || (page?.items ?? []).length === 0) break;
      }
      if (series !== assetSeries || !mounted || deps.projectId() !== projectId) return;
      boundAssets.value = collected;
      boundAssetsTotal.value = total;
      boundAssetsTruncated.value = collected.length < total;
    } catch (error) {
      if (series !== assetSeries || !mounted || deps.projectId() !== projectId) return;
      deps.notify("error", `准入资产分页读取失败：${exposureErrorText(error)}`);
    }
  }

  async function startScan(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || starting.value || activeRun.value) return;
    starting.value = true;
    try {
      const runId = await deps.api.startExposureScan(projectId);
      deps.notify("success", `被动采集任务 #${runId} 已启动；本轮范围受已知上限约束`);
      await load();
    } catch (error) {
      deps.notify("error", `无法启动被动采集：${exposureErrorText(error)}`);
    } finally {
      starting.value = false;
    }
  }

  async function cancelScan(): Promise<void> {
    const current = activeRun.value;
    const projectId = deps.projectId();
    if (!current || projectId === undefined) return;
    try {
      // FIX2-4：带上当前项目，后端核对任务归属，不属于本项目的取消会被拒绝。
      await deps.api.cancelExposureScan(projectId, current.id);
      deps.notify("info", "已请求取消，已采集结果会保留");
      await load();
    } catch (error) {
      deps.notify("error", `取消失败：${exposureErrorText(error)}`);
    }
  }

  // G0/P1-4：复核写入只走带 projectId/expectedRevision/操作者/依据的 scoped 入口（复核整改面板）。
  // 这里不再提供无版本的 review_exposure_finding 写入；旧入口的退役由主控在注册表中处理。

  async function save(input: ExposureFindingDraft): Promise<boolean> {
    try {
      await deps.api.saveExposureFinding(input);
      await load();
      return true;
    } catch (error) {
      deps.notify("error", `保存失败：${exposureErrorText(error)}`);
      return false;
    }
  }

  async function exportAll(kind: "findings" | "ledger" = "findings"): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) return;
    try {
      const result = kind === "findings"
        ? await deps.api.exportExposureFindings(projectId)
        : await deps.api.exportExposureFindings(projectId, kind);
      // G4-3 the digest and row count come from reading the landed file back.
      let verified: unknown = null;
      try {
        verified = deps.api.verifyExposureExport ? await deps.api.verifyExposureExport(result.path) : null;
      } catch {
        verified = null;
      }
      if (deps.projectId() !== projectId) return;
      lastExport.value = exportLine(result, verified);
      deps.notify("success", lastExport.value);
    } catch (error) {
      deps.notify("error", `导出失败：${exposureErrorText(error)}`);
    }
  }

  /// Only an active run, a visible page and no read already in flight may poll.
  function tick(): void {
    if (inFlight || !mounted || !deps.active() || !activeRun.value) return;
    if (typeof document !== "undefined" && document.hidden) return;
    void load();
  }

  // The project control is the single trigger: switching workspaces re-reads both lists
  // and invalidates whatever the previous project still owes us.
  watch(
    () => deps.projectId(),
    (next, previous) => {
      if (next === previous) return;
      selectedRunId.value = null;
      void load();
      void loadBoundAssets();
    },
  );

  onMounted(() => {
    mounted = true;
    pollTimer = setInterval(tick, POLL_INTERVAL_MS);
    void load();
    void loadBoundAssets();
  });
  onUnmounted(() => {
    mounted = false;
    generation += 1;
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = undefined;
  });

  return {
    summary,
    findings,
    runs,
    sources,
    boundAssets,
    selectedRunId,
    statusFilter,
    searchFilter,
    findingsTotal,
    hasMore,
    nextCursor,
    loading,
    loadingMore,
    stale,
    readError,
    hasSnapshot,
    starting,
    boundAssetsTotal,
    boundAssetsTruncated,
    activeRun,
    scopeNotice,
    beyondPlanNotice,
    limitsNotice,
    unitLines,
    lastExport,
    dispositionFilter,
    dispositions,
    dispositionFilters,
    badgeFor,
    classificationNotice,
    knownLimits,
    pagedReadsAvailable,
    windowNotice: computed(() => (pagedReadsAvailable.value ? "" : legacyWindowNotice())),
    load,
    loadMore,
    loadBoundAssets,
    showSourcesFor: (runId: number | null) => {
      const projectId = deps.projectId();
      if (!projectId) return Promise.resolve();
      return showSourcesFor(runId, ++generation, projectId);
    },
    startScan,
    cancelScan,
    save,
    exportAll,
    tick,
  };
}
