import { computed, ref, type Ref } from "vue";
import type { SentinelFinding } from "../types";
import { endpointUrl, json, text } from "../presentation";

// API exploration owns its display state. The board only supplies the active
// target and its already-scoped findings; this module never fetches or mutates
// task data.
export function useApiResults(
  selectedUrl: Ref<string>,
  rows: (...kinds: string[]) => SentinelFinding[],
) {
  const apiRows = computed(() => rows("api"));
  const expandedApiRows = ref<number[]>([]);

  function toggleApiRow(id: number) {
    expandedApiRows.value = expandedApiRows.value.includes(id)
      ? expandedApiRows.value.filter((value) => value !== id)
      : [...expandedApiRows.value, id];
  }

  function apiRecord(item: SentinelFinding): Record<string, any> {
    return json(item.recordJson);
  }

  function apiUrl(item: SentinelFinding) {
    const data = apiRecord(item);
    return data.url || endpointUrl(selectedUrl.value, data.path);
  }

  function apiPath(item: SentinelFinding) {
    const value = apiUrl(item);
    try { return new URL(value, selectedUrl.value || "http://localhost").pathname || "/"; }
    catch { return value.split("?")[0].split("#")[0] || value; }
  }

  function apiQuery(item: SentinelFinding) {
    const data = apiRecord(item);
    const params = data.parameters || data.queryKeys || data.bodyKeys || [];
    return Array.isArray(params)
      ? params.map((value: any) => String(value?.name || value)).filter(Boolean)
      : Object.keys(params || {});
  }

  function apiResponseSummary(item: SentinelFinding) {
    const data = apiRecord(item);
    return text(data.responseKeys || data.responseSchema?.keys || data.responseBody || "") || "未记录响应字段";
  }

  function apiMethod(item: SentinelFinding) {
    return String(apiRecord(item).method || "GET").toUpperCase();
  }

  function apiSourceSummary(item: SentinelFinding) {
    const data = apiRecord(item);
    return [data.source || data.discoveredFrom || data.extractionEngine || "unknown", data.initiator || "unknown", data.observedCount ? `${data.observedCount} 次观察` : "观察次数未记录"].join(" · ");
  }

  function apiDescription(item: SentinelFinding) {
    const data = apiRecord(item);
    return text(data.description || data.summary || data.title || data.notes || "") || "未提供接口说明；以下内容来自运行时/静态证据。";
  }

  function apiRequestPayload(item: SentinelFinding) {
    const data = apiRecord(item);
    return data.requestBody ?? data.payload ?? data.body ?? data.requestSchema ?? {};
  }

  function apiResponseHeaders(item: SentinelFinding) {
    const data = apiRecord(item);
    return data.responseHeaders || data.headers || {};
  }

  function apiIdentitySummary(item: SentinelFinding) {
    const data = apiRecord(item);
    const values = data.identityKeys || data.identities || data.identityContext || [];
    return Array.isArray(values) ? values.map(String).join("、") : text(values) || "未标注身份";
  }

  return {
    apiRows, expandedApiRows, toggleApiRow, apiRecord, apiUrl, apiPath,
    apiQuery, apiResponseSummary, apiMethod, apiSourceSummary, apiDescription,
    apiRequestPayload, apiResponseHeaders, apiIdentitySummary,
  };
}
