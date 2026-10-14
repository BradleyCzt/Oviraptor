import { computed, reactive, ref, type Ref } from "vue";
import type { SentinelScan } from "../types";
import { scanTokenTotal, uncachedInput } from "../presentation";

export type TokenUsageScope = "all" | "cloud" | "local";

// Read-only presentation of the rows already loaded by the Board. This module
// neither fetches all historical tasks nor computes a cost/yield ratio from
// the separately fetched, project-wide Reviewer count.
export function useTokenUsage(options: {
  scans: Readonly<Ref<SentinelScan[]>>;
  vulnerabilityScanIds: Readonly<Ref<string[]>>;
}) {
  const { scans, vulnerabilityScanIds } = options;
  const tokenScope = ref<TokenUsageScope>("all");
  const tokenScans = computed(() => tokenScope.value === "all"
    ? scans.value : scans.value.filter((scan) => scan.llmDeployment === tokenScope.value));
  const totalInputTokenUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + scan.inputTokens, 0));
  const totalOutputTokenUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + scan.outputTokens, 0));
  const totalTokenUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + scanTokenTotal(scan), 0));
  const totalCachedTokenUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + scan.cachedTokens, 0));
  const totalUncachedInputUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + uncachedInput(scan), 0));
  const cacheHitRate = computed(() => totalInputTokenUsage.value
    ? Math.round((totalCachedTokenUsage.value / totalInputTokenUsage.value) * 100) : 0);
  // Legacy counter names describe absence from the loaded association index,
  // not verified zero findings or a completed review.
  const zeroYieldScans = computed(() => tokenScans.value.filter((scan) =>
    scanTokenTotal(scan) > 0 && ["completed", "partial", "recon_only", "failed"].includes(scan.status)
    && !vulnerabilityScanIds.value.includes(scan.id)));
  const zeroYieldTokenUsage = computed(() =>
    zeroYieldScans.value.reduce((sum, scan) => sum + scanTokenTotal(scan), 0));
  const highestCostScan = computed(() =>
    [...tokenScans.value].sort((left, right) => scanTokenTotal(right) - scanTokenTotal(left))[0]);
  const totalRequestUsage = computed(() =>
    tokenScans.value.reduce((sum, scan) => sum + scan.llmRequests, 0));
  const tokenTypeRows = computed(() => ["web", "code", "greybox", "cicd"].map((type) => {
    const rows = tokenScans.value.filter((scan) => (scan.scanType || "web") === type);
    return {
      type,
      input: rows.reduce((sum, scan) => sum + scan.inputTokens, 0),
      cached: rows.reduce((sum, scan) => sum + scan.cachedTokens, 0),
      uncachedInput: rows.reduce((sum, scan) => sum + uncachedInput(scan), 0),
      output: rows.reduce((sum, scan) => sum + scan.outputTokens, 0),
      requests: rows.reduce((sum, scan) => sum + scan.llmRequests, 0),
      total: rows.reduce((sum, scan) => sum + scanTokenTotal(scan), 0),
    };
  }));
  // One typed view model for the panel; the existing shared counters remain
  // refs for the Board's summary and task-center consumers.
  const usage = reactive({ totalInputTokenUsage, totalOutputTokenUsage, totalTokenUsage,
    totalCachedTokenUsage, totalUncachedInputUsage, totalRequestUsage, cacheHitRate,
    zeroYieldScans, zeroYieldTokenUsage,
    highestCostScan, tokenTypeRows });
  return { tokenScope, usage, totalTokenUsage, totalRequestUsage,
    zeroYieldScans, zeroYieldTokenUsage, cacheHitRate };
}

export type TokenUsageSummary = ReturnType<typeof useTokenUsage>["usage"];
