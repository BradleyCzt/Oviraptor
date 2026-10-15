import { computed, ref } from "vue";
import { sentinelApi } from "../api";
import { validTaskPage } from "../results/taskPageContract";
import type { AgentDialogSelection, SentinelScan } from "../types";

type DialogProps = Readonly<{ projectId?: number; initialScanId?: string }>;

// Task navigation owns its own project/read generations. Status and conversation
// lifecycle remain in AgentDialog; changing the selected task invalidates them there.
export function useAgentDialogTasks(props: DialogProps, tr: (zh: string, en: string) => string,
  isDisposed: () => boolean) {
  const scans = ref<SentinelScan[]>([]);
  const scanId = ref("");
  const scanSearch = ref("");
  const listError = ref("");
  const pageError = ref("");
  const olderCursor = ref<Pick<SentinelScan, "updatedAt" | "id">>();
  const hasMoreScans = ref(false);
  const loadingMoreScans = ref(false);
  const taskSelection = ref<AgentDialogSelection>();
  const selectionError = ref("");
  const selectionNotice = ref("");
  const selectionBusy = ref(false);
  const selected = computed(() => scans.value.find((scan) => scan.id === scanId.value));
  const visibleScans = computed(() => {
    const query = scanSearch.value.trim().toLocaleLowerCase();
    return query ? scans.value.filter((scan) => [scan.id, scan.taskName, scan.projectName]
      .some((value) => String(value || "").toLocaleLowerCase().includes(query))) : scans.value;
  });
  const scanPageSize = 300;
  let selectionGeneration = 0;
  let selectionReadSerial = 0;
  let taskChoiceSerial = 0;
  let pendingSelectionId: string | undefined;
  let scanListRequestSerial = 0;
  let pageRequestSerial = 0;
  let projectGeneration = 0;

  function validSelection(result: AgentDialogSelection, projectId?: number) {
    return !!result && result.projectId === (projectId ?? null)
      && Number.isSafeInteger(result.revision) && result.revision >= 0
      && (result.revision !== 0 || (result.selectedScanId === null
        && result.selectedScan === null && !result.selectionUnavailable))
      && typeof result.selectionUnavailable === "boolean"
      && (result.selectedScanId === null || (typeof result.selectedScanId === "string"
        && !!result.selectedScanId && result.selectedScanId.length <= 256))
      && (result.selectedScan === null ? (result.revision === 0 || result.selectionUnavailable)
        : !!result.selectedScan && typeof result.selectedScanId === "string"
          && typeof result.selectedScan.updatedAt === "string" && typeof result.selectedScan.status === "string"
          && !result.selectionUnavailable && result.selectedScan.id === result.selectedScanId
          && (projectId === undefined || result.selectedScan.projectId === projectId));
  }

  async function loadTaskSelection() {
    const projectId = props.projectId;
    const generation = selectionGeneration;
    const serial = ++selectionReadSerial;
    const current = () => !isDisposed() && generation === selectionGeneration
      && serial === selectionReadSerial && props.projectId === projectId;
    try {
      const result = await sentinelApi.getAgentDialogSelection(projectId);
      if (!current()) return;
      if (!validSelection(result, projectId)) throw new Error("dialog_selection_invalid_response");
      taskSelection.value = result;
      selectionError.value = "";
      selectionNotice.value = result.selectionUnavailable
        ? tr("上次查看的任务已删除或不再属于当前项目，未自动恢复。", "The previous task was deleted or moved; it was not restored.") : "";
      void persistTaskSelection();
    } catch {
      if (current()) {
        pendingSelectionId = undefined;
        selectionError.value = tr("任务选择读取失败，请重新读取。", "Unable to read task selection. Please reload.");
      }
    }
  }

  async function persistTaskSelection() {
    if (selectionBusy.value || !taskSelection.value || selectionError.value) return;
    const projectId = props.projectId;
    const generation = selectionGeneration;
    const current = () => !isDisposed() && generation === selectionGeneration && props.projectId === projectId;
    selectionBusy.value = true;
    try {
      // Coalesce this window's clicks; an external CAS conflict requires reload.
      while (current() && pendingSelectionId && taskSelection.value) {
        const requested = pendingSelectionId;
        pendingSelectionId = undefined;
        const saved = taskSelection.value;
        if (saved.selectedScanId === requested && !saved.selectionUnavailable) continue;
        const result = await sentinelApi.saveAgentDialogSelection({ projectId,
          expectedRevision: saved.revision, scanId: requested });
        if (!current()) return;
        if (!validSelection(result, projectId) || result.revision !== saved.revision + 1
          || result.selectedScanId !== requested || result.selectionUnavailable)
          throw new Error("dialog_selection_invalid_response");
        taskSelection.value = result;
        selectionNotice.value = "";
      }
    } catch {
      if (current()) {
        pendingSelectionId = undefined;
        selectionError.value = tr("任务选择保存失败，请重新读取后再选择。", "Unable to save task selection. Reload before selecting again.");
      }
    } finally {
      if (current()) selectionBusy.value = false;
    }
  }

  async function selectScan(id: string) {
    const projectId = props.projectId;
    const generation = projectGeneration;
    const serial = ++taskChoiceSerial;
    const current = () => !isDisposed() && projectId === props.projectId
      && generation === projectGeneration && serial === taskChoiceSerial;
    try {
      if (!scans.value.some((scan) => scan.id === id)) {
        const task = await sentinelApi.getAgentDialogTask(id, projectId);
        if (!current()) return;
        if (!task || task.id !== id || (projectId !== undefined && task.projectId !== projectId))
          throw new Error("dialog_selection_scan_unavailable");
        mergeScanRows([task]);
      }
      if (!current()) return;
      scanId.value = id;
      if (!selectionError.value) pendingSelectionId = id;
      await persistTaskSelection();
    } catch {
      if (current()) selectionError.value = tr("无法读取所选任务，请重新读取任务选择。", "Unable to read the selected task. Please reload task selection.");
    }
  }

  async function loadScans() {
    const projectId = props.projectId;
    const generation = projectGeneration;
    const serial = ++scanListRequestSerial;
    const isCurrent = () => !isDisposed() && generation === projectGeneration
      && serial === scanListRequestSerial && props.projectId === projectId;
    try {
      const firstPage = await sentinelApi.listSentinelScans(projectId, scanPageSize);
      if (!isCurrent()) return;
      if (!validTaskPage(firstPage, scanPageSize, projectId)) throw new Error("dialog_task_page_invalid_response");
      // Refresh recent rows without hiding older pages already opened.
      mergeScanRows(firstPage);
      if (!olderCursor.value) {
        if (firstPage.length) olderCursor.value = firstPage[firstPage.length - 1];
        hasMoreScans.value = firstPage.length === scanPageSize;
      }
      listError.value = "";
      if (!taskSelection.value && !selectionError.value) await loadTaskSelection();
      if (!isCurrent()) return;
      if (!scanId.value) {
        const restored = taskSelection.value?.selectedScan;
        if (restored) mergeScanRows([restored]);
        if (props.initialScanId) {
          await selectScan(props.initialScanId);
          if (!isCurrent()) return;
        } else if (restored) {
          scanId.value = restored.id;
        }
      }
      const running = scans.value.find((scan) => scan.status === "scanning" || scan.status === "pausing");
      if (!scanId.value && props.initialScanId && scans.value.some((scan) => scan.id === props.initialScanId)) {
        scanId.value = props.initialScanId;
      } else if (!scanId.value || !scans.value.some((scan) => scan.id === scanId.value)) {
        scanId.value = running?.id || scans.value[0]?.id || "";
      }
    } catch {
      if (isCurrent()) listError.value = tr("任务列表读取失败，请重试。", "Unable to read the task list. Please retry.");
    }
  }

  function mergeScanRows(rows: SentinelScan[]) {
    const merged = new Map(scans.value.map((scan) => [scan.id, scan]));
    for (const scan of rows) {
      const previous = merged.get(scan.id);
      if (!previous || scan.updatedAt >= previous.updatedAt) merged.set(scan.id, scan);
    }
    scans.value = [...merged.values()].sort((left, right) =>
      right.updatedAt.localeCompare(left.updatedAt) || right.id.localeCompare(left.id));
  }

  async function loadMoreScans() {
    if (isDisposed() || !hasMoreScans.value || loadingMoreScans.value) return;
    loadingMoreScans.value = true;
    const projectId = props.projectId;
    const generation = projectGeneration;
    const serial = ++pageRequestSerial;
    const isCurrent = () => !isDisposed() && generation === projectGeneration
      && serial === pageRequestSerial && props.projectId === projectId;
    try {
      const cursor = olderCursor.value;
      const page = await sentinelApi.listSentinelScans(projectId, scanPageSize, cursor);
      if (!isCurrent()) return;
      if (!validTaskPage(page, scanPageSize, projectId)) throw new Error("dialog_task_page_invalid_response");
      if (page.length) olderCursor.value = page[page.length - 1];
      hasMoreScans.value = page.length === scanPageSize;
      mergeScanRows(page);
      pageError.value = "";
    } catch {
      if (isCurrent()) pageError.value = tr("更早任务读取失败，请重试。", "Unable to read older tasks. Please retry.");
    } finally {
      if (isCurrent()) loadingMoreScans.value = false;
    }
  }

  function resetForProject() {
    projectGeneration++;
    selectionGeneration++;
    selectionReadSerial++;
    taskChoiceSerial++;
    taskSelection.value = undefined;
    selectionError.value = "";
    selectionNotice.value = "";
    selectionBusy.value = false;
    pendingSelectionId = undefined;
    scanListRequestSerial++;
    pageRequestSerial++;
    loadingMoreScans.value = false;
    listError.value = "";
    pageError.value = "";
    scans.value = [];
    scanId.value = "";
    olderCursor.value = undefined;
    hasMoreScans.value = false;
  }

  return { scans, scanId, scanSearch, listError, pageError, olderCursor, hasMoreScans,
    loadingMoreScans, taskSelection, selectionError, selectionNotice, selectionBusy,
    selected, visibleScans, loadTaskSelection, selectScan, loadScans, loadMoreScans, resetForProject };
}
