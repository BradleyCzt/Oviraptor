import { reactive, type Ref } from "vue";
import type { SentinelFinding, SentinelFuseEntry, SentinelTarget, SentinelValidation } from "../types";

export type FuseDetailTab = "summary" | "fingerprint" | "assets" | "endpoints" | "proof";
type FuseDetailState = {
  open: boolean;
  loading: boolean;
  loaded: boolean;
  tab: FuseDetailTab;
  findings: SentinelFinding[];
  validations: SentinelValidation[];
};

export const fuseDetailTabs: [FuseDetailTab, string][] = [
  ["summary", "概要"],
  ["fingerprint", "指纹配置"],
  ["assets", "JS / API"],
  ["endpoints", "端点验证"],
  ["proof", "漏洞证明"],
];

export function sameTargetUrl(left: string, right: string) {
  const normalize = (value: string) => {
    try {
      const url = new URL(value);
      return `${url.protocol}//${url.host}${url.pathname.replace(/\/$/, "")}${url.search}`;
    } catch {
      return value.replace(/\/$/, "");
    }
  };
  return normalize(left) === normalize(right);
}

export function useFuseDetails(options: {
  targets: Ref<SentinelTarget[]>;
  loadFindings: (scanId: string) => Promise<SentinelFinding[]>;
  loadValidations: (scanId: string) => Promise<SentinelValidation[]>;
  notifyError: (message: string) => void;
}) {
  const states = reactive<Record<number, FuseDetailState>>({});
  const empty: FuseDetailState = {
    open: false, loading: false, loaded: false, tab: "summary", findings: [], validations: [],
  };
  function fuseState(item: SentinelFuseEntry) {
    return states[item.id] || empty;
  }
  function ensureState(item: SentinelFuseEntry) {
    return states[item.id] || (states[item.id] = {
      open: false, loading: false, loaded: false, tab: "summary", findings: [], validations: [],
    });
  }
  function fuseRows(item: SentinelFuseEntry, ...kinds: string[]) {
    return fuseState(item).findings.filter(
      (row) => sameTargetUrl(row.targetUrl, item.url) && kinds.includes(row.kind),
    );
  }
  function fuseTarget(item: SentinelFuseEntry) {
    return options.targets.value.find(
      (target) => target.scanId === item.sourceScanId && sameTargetUrl(target.url, item.url),
    );
  }
  function fuseValidationRows(item: SentinelFuseEntry) {
    return fuseState(item).validations.filter((row) => sameTargetUrl(row.url, item.url));
  }
  async function toggleFuseDetail(item: SentinelFuseEntry) {
    const state = ensureState(item);
    state.open = !state.open;
    if (!state.open || state.loaded || state.loading) return;
    state.loading = true;
    try {
      [state.findings, state.validations] = await Promise.all([
        options.loadFindings(item.sourceScanId),
        options.loadValidations(item.sourceScanId),
      ]);
      state.loaded = true;
    } catch (error) {
      state.open = false;
      options.notifyError(`完整情报加载失败：${String(error)}`);
    } finally {
      state.loading = false;
    }
  }
  return { fuseState, fuseRows, fuseTarget, fuseValidationRows, toggleFuseDetail };
}
