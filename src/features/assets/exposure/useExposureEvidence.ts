// §Q4/§Q5 — the pending index queue and the per-finding evidence package. Same rules as the
// rest of the page: every read is generation-guarded, a late answer from another project can
// never overwrite the screen, and an unregistered command is reported as missing, not as empty.

import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  exposureErrorText,
  isPendingRow,
  isUnregisteredCommand,
  readEvidence,
  readExposurePage,
  resumeRefusalText,
  evidenceHeadline,
  type ExposureEvidence,
  type ExposurePage,
  type ExposurePending,
} from "./contracts";

export interface ExposureEvidenceApi {
  pageExposurePendingDiscoveries?: (
    projectId: number,
    status: string,
    pageSize: number,
    cursor: string,
  ) => Promise<unknown>;
  readExposureEvidence?: (findingId: number, expectedProjectId: number) => Promise<unknown>;
  resumeExposureScan?: (runId: number, expectedProjectId: number) => Promise<number>;
}

export interface ExposureEvidenceDeps {
  projectId: () => number | undefined;
  active: () => boolean;
  api: ExposureEvidenceApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
}

const PENDING_PAGE_SIZE = 50;

export function useExposureEvidence(deps: ExposureEvidenceDeps) {
  const pending = ref<ExposurePage<ExposurePending> | null>(null);
  const pendingLoading = ref(false);
  const evidence = ref<ExposureEvidence | null>(null);
  const evidenceFor = ref<number | null>(null);
  const evidenceLoading = ref(false);
  const readError = ref("");
  const pagedPending = ref(true);

  let generation = 0;
  /// FIX3-R1 证据详情专用票号：快速切换线索时，迟到的旧证据不能覆盖新选中的线索。
  let evidenceTicket = 0;
  let mounted = false;

  function discarded(claimed: number, projectId: number): boolean {
    return !mounted || claimed !== generation || deps.projectId() !== projectId;
  }

  const pendingNotice = computed(() => {
    if (!deps.projectId()) return "请先选择工作空间";
    if (!pagedPending.value) return "待归并队列接口尚未注册，读取被跳过而不是当作空队列";
    const current = pending.value;
    if (!current) return "尚未读取待归并队列";
    return `待归并 ${current.total} 条，已显示 ${current.items.length} 条${current.hasMore ? "，仍有更多" : "，已到底"}`;
  });
  const headline = computed(() => evidenceHeadline(evidence.value));
  const reviewNeeded = computed(() => evidence.value?.reviewNeeded ?? null);

  async function loadPending(status = "unreviewed"): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) {
      pending.value = null;
      return;
    }
    if (!deps.api.pageExposurePendingDiscoveries) {
      pagedPending.value = false;
      pending.value = null;
      return;
    }
    const claimed = ++generation;
    pendingLoading.value = true;
    try {
      const raw = await deps.api.pageExposurePendingDiscoveries(
        projectId,
        status,
        PENDING_PAGE_SIZE,
        "",
      );
      if (discarded(claimed, projectId)) return;
      pending.value = readExposurePage(raw, isPendingRow, "待归并队列");
      readError.value = "";
    } catch (error) {
      if (discarded(claimed, projectId)) return;
      // A missing command is a registration fact, never an empty queue.
      if (isUnregisteredCommand(error)) pagedPending.value = false;
      else readError.value = exposureErrorText(error);
    } finally {
      if (!discarded(claimed, projectId)) pendingLoading.value = false;
    }
  }

  async function loadMorePending(): Promise<void> {
    const projectId = deps.projectId();
    const current = pending.value;
    if (!projectId || !current?.hasMore || !current.nextCursor) return;
    if (!deps.api.pageExposurePendingDiscoveries) return;
    const claimed = generation;
    try {
      const raw = await deps.api.pageExposurePendingDiscoveries(
        projectId,
        "unreviewed",
        PENDING_PAGE_SIZE,
        current.nextCursor,
      );
      if (discarded(claimed, projectId)) return;
      const next = readExposurePage(raw, isPendingRow, "待归并队列");
      const seen = new Set(current.items.map((row) => row.id));
      pending.value = {
        ...next,
        items: [...current.items, ...next.items.filter((row) => !seen.has(row.id))],
      };
    } catch (error) {
      if (discarded(claimed, projectId)) return;
      readError.value = exposureErrorText(error);
    }
  }

  async function openEvidence(findingId: number): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) return;
    if (!deps.api.readExposureEvidence) {
      readError.value = "证据包接口尚未注册，无法读取来源与版本";
      return;
    }
    const claimed = generation;
    const ticket = ++evidenceTicket;
    evidenceLoading.value = true;
    try {
      const raw = await deps.api.readExposureEvidence(findingId, projectId);
      if (discarded(claimed, projectId) || ticket !== evidenceTicket) return;
      evidence.value = readEvidence(raw);
      evidenceFor.value = findingId;
      readError.value = "";
    } catch (error) {
      if (discarded(claimed, projectId) || ticket !== evidenceTicket) return;
      readError.value = exposureErrorText(error);
    } finally {
      if (!discarded(claimed, projectId) && ticket === evidenceTicket) evidenceLoading.value = false;
    }
  }

  function closeEvidence(): void {
    evidence.value = null;
    evidenceFor.value = null;
  }

  /// Resume only ever continues a round whose frozen plan still matches. A refusal is shown
  /// as a refusal; the page never rephrases it as "started".
  async function resumeRun(runId: number): Promise<boolean> {
    const projectId = deps.projectId();
    if (!projectId) return false;
    if (!deps.api.resumeExposureScan) {
      deps.notify("info", "恢复采集接口尚未注册");
      return false;
    }
    try {
      await deps.api.resumeExposureScan(runId, projectId);
      deps.notify("success", `任务 #${runId} 已按冻结计划继续`);
      return true;
    } catch (error) {
      deps.notify("error", resumeRefusalText(error));
      return false;
    }
  }

  onMounted(() => {
    mounted = true;
  });
  onUnmounted(() => {
    mounted = false;
    generation += 1;
  });

  return {
    pending,
    pendingLoading,
    pendingNotice,
    evidence,
    evidenceFor,
    evidenceLoading,
    headline,
    reviewNeeded,
    readError,
    pagedPending,
    loadPending,
    loadMorePending,
    openEvidence,
    closeEvidence,
    resumeRun,
  };
}
