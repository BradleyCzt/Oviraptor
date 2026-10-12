// P1 UI 闭环 — 覆盖矩阵钻取。选一行（来源）后按时间窗口、单位读取工作项明细，键集分页。
// 读不到时如实报错，不显示成“没有缺口”；换来源/窗口/单位/项目后，迟到的旧页被丢弃。

import { ref } from "vue";
import { exposureErrorText, isUnregisteredCommand } from "./contracts";
import { coverageDrillView, type CoverageDrillView } from "./incidents";

export interface ExposureCoverageApi {
  exposureCoverageDrilldown?: (
    projectId: number,
    sourceKey: string,
    windowDays: number,
    unitEntityId: number | null,
    gapsOnly: boolean,
    pageSize: number,
    cursor: string,
  ) => Promise<unknown>;
}

export interface ExposureCoverageDeps {
  projectId: () => number | undefined;
  api: ExposureCoverageApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
  pageSize?: number;
}

export function useExposureCoverage(deps: ExposureCoverageDeps) {
  const pageSize = deps.pageSize ?? 50;
  const sourceKey = ref("");
  const windowDays = ref(0);
  const unitEntityId = ref<number | null>(null);
  const gapsOnly = ref(false);
  const view = ref<CoverageDrillView | null>(null);
  const items = ref<CoverageDrillView["items"]>([]);
  const loading = ref(false);
  const error = ref("");
  const available = ref(true);
  let generation = 0;

  const filterKey = () => JSON.stringify([deps.projectId(), sourceKey.value, windowDays.value, unitEntityId.value, gapsOnly.value]);

  async function read(cursor: string): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId || !sourceKey.value) return;
    if (!deps.api.exposureCoverageDrilldown) {
      available.value = false;
      return;
    }
    const claimed = cursor ? generation : ++generation;
    const key = filterKey();
    loading.value = true;
    try {
      const page = coverageDrillView(
        await deps.api.exposureCoverageDrilldown(projectId, sourceKey.value, windowDays.value, unitEntityId.value, gapsOnly.value, pageSize, cursor),
      );
      if (claimed !== generation || key !== filterKey()) return;
      if (cursor) {
        const seen = new Set(items.value.map((row) => row.id));
        items.value = [...items.value, ...page.items.filter((row) => !seen.has(row.id))];
      } else {
        items.value = page.items;
      }
      view.value = page;
      error.value = "";
    } catch (reason) {
      if (claimed !== generation || key !== filterKey()) return;
      if (isUnregisteredCommand(reason)) available.value = false;
      error.value = exposureErrorText(reason);
      if (!cursor) {
        view.value = null;
        items.value = [];
      }
    } finally {
      if (claimed === generation) loading.value = false;
    }
  }

  async function drill(source: string): Promise<void> {
    sourceKey.value = source;
    unitEntityId.value = null;
    await read("");
  }

  async function setWindow(days: number): Promise<void> {
    windowDays.value = days;
    await read("");
  }

  async function setUnit(unit: number | null): Promise<void> {
    unitEntityId.value = unit;
    await read("");
  }

  async function setGapsOnly(value: boolean): Promise<void> {
    gapsOnly.value = value;
    await read("");
  }

  async function loadMore(): Promise<void> {
    if (!view.value?.hasMore || !view.value.nextCursor || loading.value) return;
    await read(view.value.nextCursor);
  }

  function close(): void {
    generation += 1;
    sourceKey.value = "";
    view.value = null;
    items.value = [];
    error.value = "";
    loading.value = false;
  }

  return { sourceKey, windowDays, unitEntityId, gapsOnly, view, items, loading, error, available, drill, setWindow, setUnit, setGapsOnly, loadMore, close };
}
