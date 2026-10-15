import { onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { HistoricalImportPreview, HistoricalImportRun } from "../types";
import { sentinelApi } from "../api";
import { validateHistoricalPreviews } from "./historicalPreviewContract";

// Global historical ledger only; importing never grants native task authority.
export function useHistoricalImportLedger(view: Ref<"attention" | "history" | "all">) {
  const importedRuns = ref<HistoricalImportRun[]>([]);
  const importedSelected = ref<HistoricalImportRun>();
  const importedPreviews = ref<HistoricalImportPreview[]>([]);
  const importedLoading = ref(false);
  const importedPreviewLoading = ref(false);
  const importedError = ref("");
  const importedHasMore = ref(false);
  const importedLoaded = ref(false);
  let importedSerial = 0;
  let importedPreviewSerial = 0;
  let disposed = false;
  onBeforeUnmount(() => {
    disposed = true;
    ++importedSerial; ++importedPreviewSerial;
  });
  async function loadImportedRuns(older = false, force = false) {
    if (disposed || (importedLoading.value && !force) || (older && !importedHasMore.value)) return;
    if (force) {
      ++importedPreviewSerial;
      importedSelected.value = undefined;
      importedPreviews.value = [];
      importedPreviewLoading.value = false;
      importedRuns.value = [];
      importedHasMore.value = false;
      importedLoaded.value = false;
    }
    const serial = ++importedSerial;
    importedLoading.value = true;
    importedError.value = "";
    try {
      const page = await sentinelApi.listHistoricalImportRuns(older ? importedRuns.value[importedRuns.value.length - 1]?.rowId : undefined);
      if (serial !== importedSerial) return;
      importedHasMore.value = page.length > 50;
      importedRuns.value = older ? [...importedRuns.value, ...page.slice(0, 50)] : page.slice(0, 50);
      importedLoaded.value = true;
    } catch {
      if (serial === importedSerial) importedError.value = "历史导入记录暂时不可读取，请重试";
    } finally {
      if (serial === importedSerial) importedLoading.value = false;
    }
  }
  watch(view, (current) => {
    if (current !== "attention" && !importedLoaded.value && !importedLoading.value) void loadImportedRuns();
  });
  async function selectImportedRun(run: HistoricalImportRun) {
    if (disposed) return;
    if (importedSelected.value?.rowId === run.rowId) {
      importedSelected.value = undefined;
      importedPreviews.value = [];
      importedPreviewLoading.value = false;
      ++importedPreviewSerial;
      return;
    }
    const serial = ++importedPreviewSerial;
    importedSelected.value = run;
    importedPreviews.value = [];
    importedPreviewLoading.value = true;
    importedError.value = "";
    try {
      const rows = await sentinelApi.listHistoricalBundlePreviews(run.bundleId, run.rowId);
      if (serial !== importedPreviewSerial) return;
      importedPreviews.value = validateHistoricalPreviews(rows, run.scanId);
    } catch {
      if (serial === importedPreviewSerial) importedError.value = "历史产物预览暂时不可读取，请重试";
    } finally {
      if (serial === importedPreviewSerial) importedPreviewLoading.value = false;
    }
  }
  return {
    importedRuns, importedSelected, importedPreviews, importedLoading, importedPreviewLoading,
    importedError, importedHasMore, importedLoaded, loadImportedRuns, selectImportedRun,
  };
}
