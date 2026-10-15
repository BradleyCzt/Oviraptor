import { onUnmounted, ref, shallowRef, watch } from "vue";
import type { SentinelScan } from "../types";
import { validTaskPage } from "./taskPageContract";

type ScanCursor = Pick<SentinelScan, "updatedAt" | "id">;
interface ScanPagesOptions {
  scope: () => number | undefined;
  isRefreshing: () => boolean;
  read: (project: number | undefined, limit: number, cursor: ScanCursor) => Promise<SentinelScan[]>;
  onError: (message: string) => void;
}

// Own the task list's read-only paging state, separately from task execution.
export function useScanPages(options: ScanPagesOptions) {
  const scans = shallowRef<SentinelScan[]>([]);
  const scanPageSize = 300;
  const scanOlderCursor = shallowRef<ScanCursor>();
  const scanHasMore = ref(false);
  const scanLoadingMore = ref(false);
  let generation = 0;
  let disposed = false;
  const cursorFor = (scan?: SentinelScan): ScanCursor | undefined => scan
    ? { updatedAt: scan.updatedAt, id: scan.id } : undefined;

  function invalidateScanPagination() {
    ++generation;
    scanLoadingMore.value = false;
  }

  function assertPage(page: SentinelScan[]) {
    if (!validTaskPage(page, scanPageSize, options.scope()))
      throw new Error("任务列表数据无效，请重试");
  }

  function replaceScanHead(page: SentinelScan[]) {
    if (disposed) return;
    assertPage(page);
    invalidateScanPagination();
    scans.value = page;
    scanOlderCursor.value = cursorFor(page[page.length - 1]);
    scanHasMore.value = page.length === scanPageSize;
  }

  function merge(page: SentinelScan[], history: boolean) {
    if (disposed) return;
    assertPage(page);
    const merged = new Map(scans.value.map(scan => [scan.id, scan]));
    for (const scan of page) {
      const existing = merged.get(scan.id);
      // A task may have advanced while an older DB page was in flight.
      if (!existing || scan.updatedAt > existing.updatedAt
        || (!history && scan.updatedAt === existing.updatedAt)) merged.set(scan.id, scan);
    }
    scans.value = [...merged.values()].sort((left, right) =>
      right.updatedAt.localeCompare(left.updatedAt) || right.id.localeCompare(left.id));
  }

  const mergeScanPage = (page: SentinelScan[]) => merge(page, false);

  async function loadMoreScanHistory() {
    if (disposed || options.isRefreshing() || !scanHasMore.value || scanLoadingMore.value) return;
    const cursor = scanOlderCursor.value;
    if (!cursor) return;
    const project = options.scope();
    const request = ++generation;
    const current = () => !disposed && generation === request && project === options.scope();
    scanLoadingMore.value = true;
    try {
      const page = await options.read(project, scanPageSize, { ...cursor });
      if (!current()) return;
      // Merge before committing the cursor, so a failed merge remains retryable.
      merge(page, true);
      if (page.length) scanOlderCursor.value = cursorFor(page[page.length - 1]);
      scanHasMore.value = page.length === scanPageSize;
    } catch {
      if (current()) options.onError("历史任务读取失败，请重试");
    } finally {
      if (current()) scanLoadingMore.value = false;
    }
  }

  watch(options.scope, () => {
    invalidateScanPagination();
    scans.value = [];
    scanOlderCursor.value = undefined;
    scanHasMore.value = false;
  }, { flush: "sync" });

  onUnmounted(() => { disposed = true; invalidateScanPagination(); });
  return { scans, scanPageSize, scanOlderCursor, scanHasMore, scanLoadingMore,
    mergeScanPage, replaceScanHead, invalidateScanPagination, loadMoreScanHistory };
}
