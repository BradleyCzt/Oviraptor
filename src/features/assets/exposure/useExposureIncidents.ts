// G4 — the incident queue. One page at a time (keyset), one incident detail at a time, and
// human actions that carry the revision they read plus a fresh operation id. The incident
// commands are registered (IR-15 landed 2026-10-09). An "unknown command" answer still turns
// the panel into `integration_pending` instead of "no incidents", and no action is offered then.

import { computed, ref } from "vue";
import { exposureErrorText, isUnregisteredCommand, type JsonLike } from "./contracts";
import { buildIncidentAction, incidentLine, newOperationId, type ActionForm, type IncidentListItem } from "./incidents";

export interface ExposureIncidentsApi {
  pageExposureIncidents?: (projectId: number, status: string, reviewNeededOnly: boolean, pageSize: number, cursor: string) => Promise<unknown>;
  readExposureIncident?: (projectId: number, incidentId: number) => Promise<unknown>;
  pageExposureIncidentLocations?: (projectId: number, incidentId: number, pageSize: number, cursor: string) => Promise<unknown>;
  actOnExposureIncident?: (projectId: number, action: JsonLike) => Promise<unknown>;
  proposeExposureIncidents?: (projectId: number) => Promise<unknown>;
}

export interface ExposureIncidentsDeps {
  projectId: () => number | undefined;
  api: ExposureIncidentsApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
  pageSize?: number;
  operationId?: () => string;
}

type Row = Record<string, unknown>;
const isRow = (value: unknown): value is Row => typeof value === "object" && value !== null && !Array.isArray(value);

export function useExposureIncidents(deps: ExposureIncidentsDeps) {
  const pageSize = deps.pageSize ?? 20;
  const items = ref<IncidentListItem[]>([]);
  const total = ref(0);
  const hasMore = ref(false);
  const cursor = ref("");
  const statusFilter = ref("open");
  const reviewNeededOnly = ref(false);
  const loading = ref(false);
  const loaded = ref(false);
  const available = ref(true);
  const readError = ref("");
  const detail = ref<Row | null>(null);
  const acting = ref(false);
  /// P1 传播位置分页：详情只带第一页（50 条），之后按游标继续读，不截断也不重复。
  const locations = ref<{ rows: Row[]; total: number; hasMore: boolean; cursor: string }>({ rows: [], total: 0, hasMore: false, cursor: "" });
  const locationsLoading = ref(false);
  /// P1 人工拆分：勾选的位置编号（只允许当前详情里读到的有效位置）。
  const selectedLocations = ref<number[]>([]);
  /// P1 人工合并：读到的目标事件（带它的版本），提交时作为 targetExpectedRevision。
  const mergeTarget = ref<{ id: number; revision: number; title: string; status: string } | null>(null);
  /// The operation id of a submit that has not been answered yet: a retry reuses it.
  let pendingOperation: { key: string; id: string } | null = null;
  let generation = 0;
  /// FIX3-R1 详情专用票号：同一项目里快速切换事件时，只有最后一次打开的响应能写入详情。
  let detailTicket = 0;
  let readProject = 0;

  const discarded = (claimed: number, projectId: number) => claimed !== generation || deps.projectId() !== projectId;

  function reset(): void {
    items.value = [];
    total.value = 0;
    hasMore.value = false;
    cursor.value = "";
    loaded.value = false;
    readError.value = "";
    detail.value = null;
    resetLocations();
  }

  function resetLocations(): void {
    locations.value = { rows: [], total: 0, hasMore: false, cursor: "" };
    selectedLocations.value = [];
    mergeTarget.value = null;
  }

  function readLocations(value: unknown): { rows: Row[]; total: number; hasMore: boolean; cursor: string } {
    if (!isRow(value) || !Array.isArray(value.items) || typeof value.total !== "number" || typeof value.hasMore !== "boolean") {
      throw new Error("位置分页结构无效");
    }
    const next = String(value.nextCursor ?? "");
    if (value.hasMore && !next) throw new Error("位置分页声称还有下一页却没有游标");
    return { rows: value.items.filter(isRow), total: value.total, hasMore: value.hasMore, cursor: next };
  }

  function readPage(value: unknown): { rows: IncidentListItem[]; total: number; hasMore: boolean; next: string } {
    if (!isRow(value) || !Array.isArray(value.items) || typeof value.total !== "number" || typeof value.hasMore !== "boolean") {
      throw new Error("事件分页结构无效");
    }
    const rows = value.items.map(incidentLine);
    if (rows.some((row) => row === null)) throw new Error("事件条目结构无效");
    const next = String(value.nextCursor ?? "");
    if (value.hasMore && !next) throw new Error("事件分页声称还有下一页却没有游标");
    return { rows: rows as IncidentListItem[], total: value.total, hasMore: value.hasMore, next };
  }

  async function load(): Promise<boolean> {
    const projectId = deps.projectId();
    if (!projectId) {
      reset();
      readProject = 0;
      return false;
    }
    if (!deps.api.pageExposureIncidents) {
      available.value = false;
      reset();
      return false;
    }
    const claimed = ++generation;
    if (projectId !== readProject) {
      reset();
      readProject = projectId;
    }
    loading.value = true;
    try {
      const page = readPage(await deps.api.pageExposureIncidents(projectId, statusFilter.value, reviewNeededOnly.value, pageSize, ""));
      if (discarded(claimed, projectId)) return false;
      items.value = page.rows;
      total.value = page.total;
      hasMore.value = page.hasMore;
      cursor.value = page.next;
      loaded.value = true;
      available.value = true;
      readError.value = "";
      return true;
    } catch (reason) {
      if (discarded(claimed, projectId)) return false;
      if (isUnregisteredCommand(reason)) {
        available.value = false;
        reset();
        return false;
      }
      readError.value = exposureErrorText(reason);
      if (!loaded.value) items.value = [];
      return false;
    } finally {
      if (claimed === generation) loading.value = false;
    }
  }

  async function loadMore(): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !hasMore.value || !cursor.value || loading.value || !deps.api.pageExposureIncidents) return;
    const claimed = generation;
    const filter = [statusFilter.value, reviewNeededOnly.value] as const;
    loading.value = true;
    try {
      const page = readPage(await deps.api.pageExposureIncidents(projectId, filter[0], filter[1], pageSize, cursor.value));
      if (discarded(claimed, projectId) || filter[0] !== statusFilter.value || filter[1] !== reviewNeededOnly.value) return;
      const seen = new Set(items.value.map((row) => row.id));
      items.value = [...items.value, ...page.rows.filter((row) => !seen.has(row.id))];
      total.value = page.total;
      hasMore.value = page.hasMore;
      cursor.value = page.next;
    } catch (reason) {
      deps.notify("error", `加载更多事件失败：${exposureErrorText(reason)}`);
    } finally {
      loading.value = false;
    }
  }

  async function open(incidentId: number): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !deps.api.readExposureIncident) return;
    const claimed = generation;
    const ticket = ++detailTicket;
    try {
      const value = await deps.api.readExposureIncident(projectId, incidentId);
      if (discarded(claimed, projectId) || ticket !== detailTicket) return;
      if (!isRow(value) || value.id !== incidentId || value.projectId !== projectId) throw new Error("事件详情与请求的项目或编号不一致");
      const switched = detail.value?.id !== incidentId;
      detail.value = value;
      locations.value = isRow(value.locations) ? readLocations(value.locations) : { rows: [], total: 0, hasMore: false, cursor: "" };
      if (switched) mergeTarget.value = null;
      const active = new Set(locations.value.rows.map((row) => Number(row.id)));
      selectedLocations.value = selectedLocations.value.filter((id) => !switched && active.has(id));
    } catch (reason) {
      if (discarded(claimed, projectId) || ticket !== detailTicket) return;
      detail.value = null;
      deps.notify("error", `事件详情读取失败：${exposureErrorText(reason)}`);
    }
  }

  /// P1 第 51 条以后的位置：按游标继续读同一事件，去重追加；换了事件就丢弃迟到的页。
  async function loadMoreLocations(): Promise<void> {
    const projectId = deps.projectId();
    const current = detail.value;
    if (!projectId || !current || locationsLoading.value || !locations.value.hasMore || !locations.value.cursor || !deps.api.pageExposureIncidentLocations) return;
    const incidentId = Number(current.id);
    const claimed = generation;
    locationsLoading.value = true;
    try {
      const page = readLocations(await deps.api.pageExposureIncidentLocations(projectId, incidentId, 50, locations.value.cursor));
      if (discarded(claimed, projectId) || Number(detail.value?.id) !== incidentId) return;
      const seen = new Set(locations.value.rows.map((row) => Number(row.id)));
      locations.value = {
        rows: [...locations.value.rows, ...page.rows.filter((row) => !seen.has(Number(row.id)))],
        total: page.total,
        hasMore: page.hasMore,
        cursor: page.cursor,
      };
    } catch (reason) {
      deps.notify("error", `加载更多位置失败：${exposureErrorText(reason)}`);
    } finally {
      locationsLoading.value = false;
    }
  }

  function toggleLocation(locationId: number): void {
    if (!locations.value.rows.some((row) => Number(row.id) === locationId)) return;
    selectedLocations.value = selectedLocations.value.includes(locationId)
      ? selectedLocations.value.filter((id) => id !== locationId)
      : [...selectedLocations.value, locationId];
  }

  /// P1 合并前先读目标事件：拿到它的版本，确认同一项目、不是自己、也没有已被合并。
  async function readMergeTarget(targetId: number): Promise<boolean> {
    const projectId = deps.projectId();
    const current = detail.value;
    mergeTarget.value = null;
    if (!projectId || !current || !deps.api.readExposureIncident) return false;
    if (!Number.isInteger(targetId) || targetId <= 0 || targetId === Number(current.id)) {
      deps.notify("error", "请填写另一起事件的编号");
      return false;
    }
    try {
      const value = await deps.api.readExposureIncident(projectId, targetId);
      if (deps.projectId() !== projectId || Number(detail.value?.id) !== Number(current.id)) return false;
      if (!isRow(value) || value.id !== targetId || value.projectId !== projectId || typeof value.revision !== "number") throw new Error("目标事件与请求的项目或编号不一致");
      if (value.status === "merged") throw new Error("目标事件已被合并到别处，不能再作为合并目标");
      mergeTarget.value = { id: targetId, revision: value.revision, title: String(value.title ?? ""), status: String(value.status ?? "") };
      return true;
    } catch (reason) {
      deps.notify("error", `目标事件读取失败：${exposureErrorText(reason)}`);
      return false;
    }
  }

  async function merge(actor: string, reason: string): Promise<boolean> {
    const target = mergeTarget.value;
    if (!target) {
      deps.notify("error", "请先读取目标事件（合并要带上目标事件的版本）");
      return false;
    }
    const done = await act({ action: "merge", targetIncidentId: target.id, targetExpectedRevision: target.revision, actor, reason });
    if (done) mergeTarget.value = null;
    return done;
  }

  async function split(actor: string, reason: string): Promise<boolean> {
    if (!selectedLocations.value.length) {
      deps.notify("error", "请先勾选要拆出的位置");
      return false;
    }
    if (selectedLocations.value.length >= locations.value.total) {
      deps.notify("error", "拆分后原事件至少要保留一个位置");
      return false;
    }
    const done = await act({ action: "split", locationIds: [...selectedLocations.value].sort((a, b) => a - b), actor, reason });
    if (done) selectedLocations.value = [];
    return done;
  }

  /// One human action. Offered only when the backend answered reads; carries the revision the
  /// detail showed. A conflict refreshes the detail instead of retrying blindly.
  async function act(form: ActionForm): Promise<boolean> {
    const projectId = deps.projectId();
    const current = detail.value;
    if (!projectId || !current || acting.value || !available.value || !deps.api.actOnExposureIncident) return false;
    const incident = { id: Number(current.id), revision: Number(current.revision) };
    const key = JSON.stringify([projectId, incident, form]);
    const operationId = pendingOperation?.key === key ? pendingOperation.id : (deps.operationId ?? newOperationId)();
    const built = buildIncidentAction(incident, form, operationId);
    if (!built.ok) {
      deps.notify("error", built.error);
      return false;
    }
    pendingOperation = { key, id: operationId };
    acting.value = true;
    try {
      const result = await deps.api.actOnExposureIncident(projectId, built.payload);
      pendingOperation = null;
      if (deps.projectId() !== projectId) return false;
      const replayed = isRow(result) && result.replayed === true;
      deps.notify("success", replayed ? "该操作此前已提交，结果按原记录返回" : "已记录");
      await open(incident.id);
      await load();
      return true;
    } catch (reason) {
      const message = exposureErrorText(reason);
      if (isUnregisteredCommand(reason)) {
        available.value = false;
        pendingOperation = null;
      } else if (/conflict|当前版本/.test(String(reason))) {
        pendingOperation = null;
        await open(incident.id);
      }
      deps.notify("error", `操作未写入：${message}`);
      return false;
    } finally {
      acting.value = false;
    }
  }

  const notice = computed(() => {
    if (!deps.projectId()) return "请先选择工作空间";
    if (!available.value) return "事件台账接口尚未注册（integration_pending，IR-15）：这里显示的不是“没有事件”";
    if (readError.value && !loaded.value) return `事件读取失败：${readError.value}`;
    if (readError.value) return `显示的是上一次读取的事件：${readError.value}`;
    if (!loaded.value) return "尚未读取";
    if (!items.value.length) return "当前筛选没有事件（候选事件只在采集或导入后自动建议，状态只由人工改）";
    return `本页 ${items.value.length} / 共 ${total.value} 起（集团唯一计数，已合并的不重复计）`;
  });

  async function proposeHistory(): Promise<boolean> {
    const projectId = deps.projectId();
    if (!projectId || !available.value || !deps.api.proposeExposureIncidents) return false;
    try {
      await deps.api.proposeExposureIncidents(projectId);
      deps.notify("success", "已为历史轮次补建议（只写台账，不联系任何人）");
      await load();
      return true;
    } catch (reason) {
      if (isUnregisteredCommand(reason)) {
        available.value = false;
        deps.notify("error", "事件台账接口尚未注册");
        return false;
      }
      deps.notify("error", `补建议失败：${exposureErrorText(reason)}`);
      return false;
    }
  }

  return {
    items, total, hasMore, statusFilter, reviewNeededOnly, loading, loaded, available, readError, detail, acting, notice,
    locations, locationsLoading, selectedLocations, mergeTarget,
    load, loadMore, open, act, proposeHistory, loadMoreLocations, toggleLocation, readMergeTarget, merge, split,
  };
}
