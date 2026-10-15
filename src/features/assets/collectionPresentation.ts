import type { CollectionRetentionSummary, CollectionSummary } from "./types";

const counts = ["queries", "completedQueries", "failedQueries", "truncatedQueries", "partialQueries", "invalidRecords", "candidates", "reusedPages"] as const;
const isCount = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;

export function collectionRetention(value: unknown): CollectionRetentionSummary | undefined {
  if (!value || typeof value !== "object") return;
  const row = value as Record<string, unknown>;
  if (row.scope !== "owned_collection_files_and_control_metadata_only" || typeof row.historyTruncated !== "boolean") return;
  if (!["runBytes", "projectBytes", "globalBytes", "expiredFiles", "expiredBytes", "unmanagedRuns", "unknownFiles",
    "maxRunBytes", "maxProjectBytes", "maxGlobalBytes", "maxPages", "maxAgeSeconds"].every(key => isCount(row[key]))) return;
  return value as CollectionRetentionSummary;
}

/** Old or malformed evidence stays unknown; an error message is never evidence. */
export function collectionSummary(value: unknown): CollectionSummary | undefined {
  if (!value || typeof value !== "object") return;
  const row = value as Record<string, unknown>;
  if (row.schemaVersion !== 1 || typeof row.executionComplete !== "boolean" || typeof row.coverageComplete !== "boolean") return;
  if (!counts.every(key => isCount(row[key]))) return;
  if (!["importedCandidates", "probedAssets"].every(key => row[key] === null || isCount(row[key]))) return;
  if (!["reasons", "errorCategories"].every(key => Array.isArray(row[key]) && (row[key] as unknown[]).every(item => typeof item === "string"))) return;
  if ((row.completedQueries as number) > (row.queries as number)) return;
  if (row.coverageComplete && (!row.executionComplete || row.completedQueries !== row.queries ||
    ["failedQueries", "truncatedQueries", "partialQueries", "invalidRecords"].some(key => (row[key] as number) > 0))) return;
  return value as CollectionSummary;
}

const collectionFailureLabels: Record<string, [string, string]> = {
  resource_limited: ["采集存储达到资源上限", "Collection storage resource limit reached"],
  retention_limit: ["达到页面或存储保留上限", "Page or storage retention limit reached"],
  redacted_locator: ["目标地址被脱敏的记录已拒收", "Records with redacted target addresses were rejected"],
  duplicate_record: ["来源返回重复记录", "Source returned duplicate records"],
  duplicate_page: ["来源返回重复页", "Source returned a repeated page"],
  field_downgrade: ["来源回退字段，证据不完整", "Source fields were reduced; evidence is incomplete"],
  total_changed: ["来源总数在分页间变化", "Source total changed between pages"],
  total_mismatch: ["返回记录数超过来源声明总数", "Returned records exceed the reported total"],
  checkpoint_io: ["采集断点读写失败", "Collection checkpoint read/write failed"],
  transport_config: ["采集连接配置无效", "Collection transport configuration is invalid"],
  collection_storage_unavailable: ["采集存储不可用", "Collection storage is unavailable"],
};
const reasonLabels: Record<string, [string, string]> = {
  ...collectionFailureLabels,
  page_limit: ["达到分页上限", "Page limit reached"],
  max_pages: ["达到分页上限", "Page limit reached"],
  partial_query: ["查询结果不完整", "Incomplete query results"],
  partial_queries: ["查询结果不完整", "Incomplete query results"],
  short_page: ["返回页未覆盖声明总数", "Page does not cover the reported total"],
  unknown_total: ["来源未提供可验证的总数", "Source total is unknown"],
  invalid_total: ["来源总数无法验证", "Source total could not be verified"],
  invalid_records: ["存在无效来源记录", "Invalid source records"],
  query_failure: ["部分查询未完成", "Some queries did not finish"],
  query_failed: ["部分查询未完成", "Some queries did not finish"],
  collection_failed: ["采集查询未完成", "Collection queries did not finish"],
  coverage_unproven: ["覆盖范围尚无完整证明", "Complete coverage has not been proven"],
  provider_precheck_failed: ["来源服务预检失败，采集未开始", "Provider precheck failed; collection did not start"],
  collection_pending: ["采集尚未完成", "Collection has not finished"],
  cancelled: ["采集已取消", "Collection cancelled"],
  mixed_observation_times: ["复用了不同时间的页面", "Pages from different times were reused"],
};
const errorLabels: Record<string, [string, string]> = {
  ...collectionFailureLabels,
  provider_quota: ["服务额度不足或限流", "Provider quota or rate limit"],
  provider_auth: ["来源鉴权失败", "Provider authentication failed"],
  provider_error: ["来源查询错误", "Provider query error"],
  timeout: ["请求超时", "Request timeout"],
  connect: ["连接失败", "Connection failed"],
  tls: ["TLS 校验失败", "TLS validation failed"],
  http_rate_limit: ["服务限流", "Provider rate limit"],
  response_body: ["响应读取失败", "Response read failed"],
  request: ["请求失败", "Request failed"],
};

export function collectionNotes(summary: CollectionSummary, tr: (zh: string, en: string) => string): string[] {
  const reasons = summary.reasons.map(code => reasonLabels[code] || ["其他覆盖缺口", "Other coverage gap"]);
  const errors = summary.errorCategories.map(code => errorLabels[code] || ["其他查询错误", "Other query error"]);
  return [...new Set([...reasons, ...errors].map(([zh, en]) => tr(zh, en)))];
}
