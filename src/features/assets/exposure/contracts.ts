// §Q0 — the exposure surface's own contracts. The page reads these instead of guessing:
// a page carries its real filtered total and whether more rows exist, an error keeps its
// stable code, and a payload that does not match the shape is rejected so a broken
// response can never overwrite the last trustworthy snapshot.

import type { ExposureFinding, ExposureRun, ExposureSourceResult } from "../types";

export type ExposureReviewStatus = "new" | "reviewing" | "confirmed" | "dismissed";

export const EXPOSURE_REVIEW_STATUSES: readonly ExposureReviewStatus[] = [
  "new",
  "reviewing",
  "confirmed",
  "dismissed",
];

export interface ExposurePage<T> {
  items: T[];
  nextCursor: string;
  hasMore: boolean;
  total: number;
  pageSize: number;
}

/// The manual-entry shape the existing IPC already accepts. Declared here so the page
/// does not depend on a type the shared `types.ts` never exported.
export interface ExposureFindingDraft {
  projectId: number;
  assetId?: number;
  category: string;
  title: string;
  sourceType: string;
  sourceUrl: string;
  evidenceExcerpt: string;
  severity: string;
  confidence: number;
}

export interface ExposureFindingPageInput {
  projectId: number;
  status: string;
  search: string;
  pageSize: number;
  cursor: string;
  /** G1-3 '' = any; otherwise one of DISPOSITION_FILTERS (unknown words are refused). */
  disposition?: string;
}

export interface ExposureLimit {
  key: string;
  label: string;
  cap: number;
  effect: string;
}

export interface ExposureScopeSummary {
  projectId: number;
  projectActive: boolean;
  eligibleAssetsTotal: number;
  eligibleAssetsPlanned: number;
  assetsBeyondPlan: number;
  findingsTotal: number;
  findingsNew: number;
  findingsReviewing: number;
  findingsConfirmed: number;
  findingsDismissed: number;
  highRiskOpen: number;
  knownLimits: ExposureLimit[];
  /** G1-0 `project_settings_v<N>` / `defaults` / `settings_corrupt` / `legacy_unverified`. */
  limitsBasis: string;
  /** G1-0 non-empty when the project's caps could not be read: nothing is planned then. */
  limitsError: string;
  /** G1-2 null when the backend did not report it (or could not read it). */
  unitProjection: ExposureUnitProjection | null;
  unitProjectionError: string;
  /** G1-3 null when the backend did not report classification counts (older build). */
  findingsExcluded: number | null;
  findingsClassificationPending: number | null;
  findingsUnclassified: number | null;
  classifyPolicyVersion: string;
  /** G4-2 the latest round as it runs; null = no round or not reported. */
  liveRun?: JsonLike | null;
  liveRunError?: string;
  /** G4-1 the coverage matrix; null with an error when unreadable, never an empty grid. */
  coverage?: JsonLike | null;
  coverageError?: string;
  /** G4-1 group-unique incidents and per-unit involvement. */
  unitStatistics?: JsonLike | null;
  unitStatisticsError?: string;
}

/// G1-2 one directory entity named by the projection.
export interface ExposureUnitRef {
  entityId: number;
  legalName: string;
  entityKind: string;
}

/// G1-2 the asset→unit projection: counts per unit only from the directory, the project
/// binding and (IR-02/IR-11) the ownership module's read-only per-asset projection.
export interface ExposureUnitAssetCount {
  unit: ExposureUnitRef;
  bucket: string;
  province: ExposureUnitRef | null;
  assets: number;
}

export interface ExposureUnitProjection {
  projectionVersion: string;
  bindingStatus: "unbound" | "group_ledger" | "unit" | "binding_broken";
  group: ExposureUnitRef | null;
  unit: ExposureUnitRef | null;
  unitBucket: string;
  unitProvince: ExposureUnitRef | null;
  directoryVersion: number;
  bindingVersion: number;
  scopeEntities: number;
  counts: {
    members: number;
    candidateLinked: number;
    confirmedGroup: number | null;
    confirmedUnit: number | null;
    unassigned: number;
    pendingAuthorization: number;
    eligible: number;
    planned: number;
    executed: number;
    uncovered: number;
    latestRunId: number | null;
  };
  perAssetEdge: string;
  perUnit: ExposureUnitAssetCount[];
  unassignedReasons: Record<string, number>;
  notes: string[];
}

const UNIT_BINDINGS = ["unbound", "group_ledger", "unit", "binding_broken"] as const;

/// G1-2 strict reader: an unknown binding status or a missing count is refused, never shown as 0.
export function readUnitProjection(value: unknown): ExposureUnitProjection | null {
  if (value === null || value === undefined) return null;
  if (!isRecord(value) || !isRecord(value.counts)) throw new Error("单位投影结构无效");
  if (!(UNIT_BINDINGS as readonly string[]).includes(String(value.bindingStatus))) {
    throw new Error(`单位投影绑定状态未知：${String(value.bindingStatus)}`);
  }
  const counts = value.counts;
  for (const key of ["members", "candidateLinked", "unassigned", "pendingAuthorization", "eligible", "planned", "executed", "uncovered"]) {
    if (!positiveFinite(counts[key])) throw new Error(`单位投影缺少 ${key}`);
  }
  const optional = (key: string) => (counts[key] === null || counts[key] === undefined ? null : Number(counts[key]));
  const ref = (row: unknown): ExposureUnitRef | null =>
    isRecord(row) ? { entityId: Number(row.entityId), legalName: String(row.legalName ?? ""), entityKind: String(row.entityKind ?? "") } : null;
  return {
    projectionVersion: String(value.projectionVersion ?? ""),
    bindingStatus: value.bindingStatus as ExposureUnitProjection["bindingStatus"],
    group: ref(value.group),
    unit: ref(value.unit),
    unitBucket: String(value.unitBucket ?? "unknown"),
    unitProvince: ref(value.unitProvince),
    directoryVersion: Number(value.directoryVersion) || 0,
    bindingVersion: Number(value.bindingVersion) || 0,
    scopeEntities: Number(value.scopeEntities) || 0,
    counts: {
      members: Number(counts.members),
      candidateLinked: Number(counts.candidateLinked),
      confirmedGroup: optional("confirmedGroup"),
      confirmedUnit: optional("confirmedUnit"),
      unassigned: Number(counts.unassigned),
      pendingAuthorization: Number(counts.pendingAuthorization),
      eligible: Number(counts.eligible),
      planned: Number(counts.planned),
      executed: Number(counts.executed),
      uncovered: Number(counts.uncovered),
      latestRunId: optional("latestRunId"),
    },
    perAssetEdge: typeof value.perAssetEdge === "string" ? value.perAssetEdge : "not_reported",
    perUnit: Array.isArray(value.perUnit)
      ? value.perUnit.filter(isRecord).flatMap((row) => {
          const unit = ref(row.unit);
          return unit && positiveFinite(row.assets)
            ? [{ unit, bucket: String(row.bucket ?? "unknown"), province: ref(row.province), assets: Number(row.assets) }]
            : [];
        })
      : [],
    unassignedReasons: isRecord(value.unassignedReasons)
      ? Object.fromEntries(Object.entries(value.unassignedReasons).filter(([, count]) => positiveFinite(count)).map(([key, count]) => [key, Number(count)]))
      : {},
    notes: Array.isArray(value.notes) ? value.notes.map(String) : [],
  };
}

const UNIT_BUCKET_LABELS: Record<string, string> = {
  province: "省公司",
  city: "地市公司",
  county: "区县公司",
  professional_subsidiary: "专业公司",
  headquarters: "总部",
  group: "集团",
  overseas: "境外实体",
  supplier: "供应商",
  unknown: "类型未知",
  none: "未绑定",
};

/// G1-2 the overview lines. A professional subsidiary is said to sit outside every province;
/// an unbound project never shows a group or unit count.
export function unitProjectionLines(projection: ExposureUnitProjection | null, error: string): string[] {
  if (error) return [`单位投影不可用：${error}`];
  if (!projection) return ["后端未报告单位投影（integration_pending）"];
  const c = projection.counts;
  const head = (() => {
    switch (projection.bindingStatus) {
      case "unbound":
        return `项目未绑定组织目录：确认资产 ${c.unassigned} 个全部记为未分配单位`;
      case "binding_broken":
        return `项目绑定已失效（v${projection.bindingVersion}），按未绑定处理：未分配单位 ${c.unassigned}`;
      case "group_ledger":
        return c.confirmedUnit === null
          ? `集团「${projection.group?.legalName ?? "?"}」总账：确认集团资产 ${c.confirmedGroup ?? 0}，其中未分配单位 ${c.unassigned}（逐资产单位归属：后端未报告）`
          : `集团「${projection.group?.legalName ?? "?"}」总账：确认集团资产 ${c.confirmedGroup ?? 0}，其中有单位 ${c.confirmedUnit}、未分配单位 ${c.unassigned}（无权威单位依据，不按名称或地区推断）`;
      case "unit": {
        const bucket = UNIT_BUCKET_LABELS[projection.unitBucket] ?? projection.unitBucket;
        const place = projection.unitProvince
          ? `，所属省公司「${projection.unitProvince.legalName}」`
          : projection.unitBucket === "professional_subsidiary"
            ? "，专业公司不归入任何省公司"
            : "，未判定所属省公司";
        return `单位「${projection.unit?.legalName ?? "?"}」（${bucket}${place}）：确认本单位资产 ${c.confirmedUnit ?? 0}`;
      }
    }
  })();
  const coverage = `候选关联 ${c.candidateLinked} · 待授权 ${c.pendingAuthorization} · 准入 ${c.eligible} · 最近一轮计划 ${c.planned} / 已执行 ${c.executed} · 未覆盖 ${c.uncovered}（目录 v${projection.directoryVersion}）`;
  const perUnit = projection.perUnit.map((row) => {
    const bucket = UNIT_BUCKET_LABELS[row.bucket] ?? row.bucket;
    const place = row.province ? `，属「${row.province.legalName}」` : row.bucket === "professional_subsidiary" ? "，专业公司单列" : "";
    return `· ${row.unit.legalName}（${bucket}${place}）：资产 ${row.assets}`;
  });
  return [head, coverage, ...perUnit, ...projection.notes];
}

/// IR-02/IR-11 why a confirmed asset has no unit, in words.
export const UNASSIGNED_REASON_LABELS: Record<string, string> = {
  project_unbound: "项目未绑定目录",
  binding_broken: "绑定已失效",
  no_unit_decision: "只做了单独确认，没有指明单位",
  operation_in_progress: "组决定尚未完成",
  group_without_directory_entity: "复核组没有目录单位",
  entity_retired: "单位已从目录退役",
  entity_outside_binding: "单位不在绑定集团内",
  entity_not_a_unit: "实体不是单位",
  changed_after_group_decision: "组决定之后人工改过归属",
  decision_event_missing: "缺组决定记录",
  operation_missing: "缺操作记录",
  group_missing: "缺复核组记录",
};

export function asExposureStatus(value: unknown): ExposureReviewStatus | undefined {
  return typeof value === "string" &&
    (EXPOSURE_REVIEW_STATUSES as readonly string[]).includes(value)
    ? (value as ExposureReviewStatus)
    : undefined;
}

/// The backend keeps its stable code in the message: `[exposure_cursor_invalid] …`.
export function exposureErrorCode(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? "");
  const matched = /^\s*\[([a-z0-9_]+)\]/i.exec(text);
  return matched ? matched[1] : text.slice(0, 160);
}

/// A storage or driver message can carry a URL with a credential in it. The page shows a
/// stripped version, and anything that looks like a secret is replaced outright.
const LEAK_PATTERN = /(bearer|token|secret|password|passwd|api[_-]?key|authorization)/i;

export function exposureErrorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? "未知错误");
  const stripped = text.replace(/^\s*\[[a-z0-9_]+\]\s*/i, "").slice(0, 240);
  return LEAK_PATTERN.test(stripped) ? "暴露面接口返回异常" : stripped;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function positiveFinite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

/// Commands the coordinator has not registered yet reject at the IPC boundary. Only that
/// failure may fall back to the legacy endpoints; a storage or validation error must not.
export function isUnregisteredCommand(error: unknown): boolean {
  const text = error instanceof Error ? error.message : String(error ?? "");
  return /(unknown|not found|unregistered)[ _-]*command|command[ _-].*(unknown|not found|unregistered)/i.test(text);
}

/// A page is only accepted when it is self-consistent: unique ids, a real total, and a
/// cursor that agrees with `hasMore`. Anything else throws and the caller keeps its
/// previous snapshot.
export function readExposurePage<T>(
  value: unknown,
  itemIsValid: (item: unknown) => item is T,
  label: string,
): ExposurePage<T> {
  if (!isRecord(value)) throw new Error(`${label} 返回结构无效`);
  if (!Array.isArray(value.items)) throw new Error(`${label} 缺少条目列表`);
  const items: T[] = [];
  for (const row of value.items) {
    if (!itemIsValid(row)) throw new Error(`${label} 含无效条目`);
    items.push(row);
  }
  if (!positiveFinite(value.total)) throw new Error(`${label} 缺少真实总数`);
  if (typeof value.hasMore !== "boolean") throw new Error(`${label} 缺少是否还有更多`);
  if (typeof value.nextCursor !== "string") throw new Error(`${label} 缺少游标`);
  if (!positiveFinite(value.pageSize) || value.pageSize < 1) throw new Error(`${label} 页长无效`);
  if (!value.hasMore && value.nextCursor !== "") throw new Error(`${label} 末尾页不能带游标`);
  const ids = new Set<number>();
  for (const item of items) {
    const id = (item as { id?: unknown }).id;
    if (!positiveFinite(id) || id === 0) throw new Error(`${label} 条目缺少 id`);
    if (ids.has(id)) throw new Error(`${label} 同页出现重复 id`);
    ids.add(id);
  }
  if (items.length > value.total) throw new Error(`${label} 行数超过总数`);
  return {
    items,
    nextCursor: value.nextCursor,
    hasMore: value.hasMore,
    total: value.total,
    pageSize: value.pageSize,
  };
}

export const isFindingRow = (value: unknown): value is ExposureFinding =>
  isRecord(value) && typeof value.id === "number" && typeof value.title === "string";

export const isRunRow = (value: unknown): value is ExposureRun =>
  isRecord(value) && typeof value.id === "number" && typeof value.status === "string";

export const isSourceRow = (value: unknown): value is ExposureSourceResult =>
  isRecord(value) && typeof value.sourceKey === "string" && typeof value.status === "string";

export function readScopeSummary(value: unknown): ExposureScopeSummary {
  if (!isRecord(value)) throw new Error("暴露面范围汇总结构无效");
  const numbers = [
    "eligibleAssetsTotal",
    "eligibleAssetsPlanned",
    "assetsBeyondPlan",
    "findingsTotal",
    "findingsNew",
    "findingsReviewing",
    "findingsConfirmed",
    "findingsDismissed",
    "highRiskOpen",
  ] as const;
  for (const key of numbers) {
    if (!positiveFinite(value[key])) throw new Error(`暴露面汇总缺少 ${key}`);
  }
  if (typeof value.projectActive !== "boolean") throw new Error("暴露面汇总缺少项目状态");
  if (!Array.isArray(value.knownLimits)) throw new Error("暴露面汇总缺少已知限制");
  const knownLimits = value.knownLimits.map((row) => {
    if (!isRecord(row) || typeof row.key !== "string" || typeof row.label !== "string") {
      throw new Error("暴露面已知限制条目无效");
    }
    return { key: row.key, label: row.label, cap: Number(row.cap) || 0, effect: String(row.effect ?? "") };
  });
  return {
    projectId: Number(value.projectId) || 0,
    projectActive: value.projectActive,
    eligibleAssetsTotal: Number(value.eligibleAssetsTotal),
    eligibleAssetsPlanned: Number(value.eligibleAssetsPlanned),
    assetsBeyondPlan: Number(value.assetsBeyondPlan),
    findingsTotal: Number(value.findingsTotal),
    findingsNew: Number(value.findingsNew),
    findingsReviewing: Number(value.findingsReviewing),
    findingsConfirmed: Number(value.findingsConfirmed),
    findingsDismissed: Number(value.findingsDismissed),
    highRiskOpen: Number(value.highRiskOpen),
    knownLimits,
    limitsBasis: typeof value.limitsBasis === "string" ? value.limitsBasis : "unreported",
    limitsError: typeof value.limitsError === "string" ? value.limitsError : "",
    unitProjection: readUnitProjection(value.unitProjection),
    unitProjectionError: typeof value.unitProjectionError === "string" ? value.unitProjectionError : "",
    findingsExcluded: optionalCount(value, "findingsExcluded"),
    findingsClassificationPending: optionalCount(value, "findingsClassificationPending"),
    findingsUnclassified: optionalCount(value, "findingsUnclassified"),
    classifyPolicyVersion: typeof value.classifyPolicyVersion === "string" ? value.classifyPolicyVersion : "",
    liveRun: isRecord(value.liveRun) ? (value.liveRun as JsonLike) : null,
    liveRunError: typeof value.liveRunError === "string" ? value.liveRunError : "",
    coverage: isRecord(value.coverage) ? (value.coverage as JsonLike) : null,
    coverageError: typeof value.coverageError === "string" ? value.coverageError : "",
    unitStatistics: isRecord(value.unitStatistics) ? (value.unitStatistics as JsonLike) : null,
    unitStatisticsError: typeof value.unitStatisticsError === "string" ? value.unitStatisticsError : "",
  };
}

/// G1-3 absent = "not reported" (null); present but not a count = a broken payload.
function optionalCount(value: Record<string, unknown>, key: string): number | null {
  if (value[key] === undefined || value[key] === null) return null;
  if (!positiveFinite(value[key])) throw new Error(`暴露面汇总的 ${key} 不是有效计数`);
  return Number(value[key]);
}

// ——— G1-3 classification and public-procurement exclusion ———

export const DISPOSITION_LABELS: Record<string, string> = {
  risk_candidate: "风险候选",
  classification_pending: "待分类",
  excluded_public_procurement: "已排除：公开招采",
  excluded_public_material: "已排除：公开材料",
};

export const INFO_CATEGORY_LABELS: Record<string, string> = {
  internal_business: "内部业务资料",
  technical_network: "技术与网络信息",
  source_code: "源代码",
  credential: "凭据",
  employee_data: "员工信息",
  customer_data: "客户信息",
  storage_misexposure: "存储错误公开",
  sold_intel: "被售卖情报",
  unclassified: "未分类",
};

export const DISPOSITION_FILTERS: { value: string; label: string }[] = [
  { value: "", label: "全部处置" },
  { value: "not_excluded", label: "隐藏已排除" },
  { value: "risk_candidate", label: "风险候选" },
  { value: "classification_pending", label: "待分类" },
  { value: "excluded", label: "已排除（招采/公开材料）" },
  { value: "unclassified", label: "旧线索（未分类）" },
];

export interface ExposureDispositionView {
  disposition: string;
  infoCategory: string;
  sourceChannel: string;
  reason: string;
  policyVersion: string;
  conflict: boolean;
  conflictNote: string;
  /** Present once the backend reports the CAS id (IR-07). */
  classificationId?: number;
}

export const isExcludedDisposition = (value: string | undefined): boolean =>
  value === "excluded_public_procurement" || value === "excluded_public_material";

function readDispositionView(raw: unknown, where: string): ExposureDispositionView {
  if (
    !isRecord(raw) ||
    typeof raw.disposition !== "string" ||
    !(raw.disposition in DISPOSITION_LABELS) ||
    typeof raw.infoCategory !== "string" ||
    typeof raw.reason !== "string" ||
    typeof raw.conflict !== "boolean"
  ) {
    throw new Error(`${where}的处置记录无效`);
  }
  return {
    disposition: raw.disposition,
    infoCategory: raw.infoCategory,
    sourceChannel: typeof raw.sourceChannel === "string" ? raw.sourceChannel : "",
    reason: raw.reason,
    policyVersion: typeof raw.policyVersion === "string" ? raw.policyVersion : "",
    conflict: raw.conflict,
    conflictNote: typeof raw.conflictNote === "string" ? raw.conflictNote : "",
    classificationId: typeof raw.classificationId === "number" ? raw.classificationId : undefined,
  };
}

/// The page's side map (finding id → disposition). Absent = an older backend: every row is
/// then "not reported", never "risk". A malformed entry fails the read instead of vanishing.
export function readDispositionMap(raw: unknown): Record<number, ExposureDispositionView> | null {
  if (raw === undefined) return null;
  if (!isRecord(raw)) throw new Error("线索处置旁表结构无效");
  const map: Record<number, ExposureDispositionView> = {};
  for (const [key, value] of Object.entries(raw)) {
    const id = Number(key);
    if (!Number.isInteger(id) || id <= 0) throw new Error(`线索处置旁表含无效 id ${key}`);
    map[id] = readDispositionView(value, `线索 ${key} `);
  }
  return map;
}

/// One badge per row. `map === null` means the backend did not report dispositions at all.
export function dispositionBadge(
  map: Record<number, ExposureDispositionView> | null,
  findingId: number,
): { label: string; tone: string; title: string } {
  if (map === null) return { label: "处置未报告", tone: "unknown", title: "当前后端未返回分类结果（integration_pending）" };
  const view = map[findingId];
  if (!view) return { label: "旧线索（未分类）", tone: "legacy", title: "G1-3 之前记录的线索，没有分类行；仍按原规则计入待办" };
  const category = INFO_CATEGORY_LABELS[view.infoCategory] ?? view.infoCategory;
  const conflict = view.conflict ? `；冲突：${view.conflictNote}` : "";
  return {
    label: `${DISPOSITION_LABELS[view.disposition]} · ${category}${view.conflict ? " · 有冲突" : ""}`,
    tone: isExcludedDisposition(view.disposition) ? "excluded" : view.disposition === "risk_candidate" ? "risk" : "pending",
    title: `${view.reason}${conflict}（${view.policyVersion}）`,
  };
}

/// Overview line: the excluded count is stated next to "high risk", so a lower number is
/// never read as "fewer problems found".
export function classificationLine(summary: ExposureScopeSummary | null): string {
  if (!summary) return "";
  if (summary.findingsExcluded === null) return "分类计数未报告（当前后端不支持 G1-3 分类）";
  return `自动排除 ${summary.findingsExcluded}（公开招采/公开材料，仍可在列表查看，不计入高风险）· 待分类 ${summary.findingsClassificationPending ?? 0} · 旧线索未分类 ${summary.findingsUnclassified ?? 0}`;
}

/// Why the remediation button is not offered. Re-classification is IR-07 (not integrated).
export function remediationBlockedReason(
  disposition: ExposureDispositionView | null | undefined,
  findingStatus: string | undefined,
): string {
  if (!disposition || !isExcludedDisposition(disposition.disposition) || findingStatus === "confirmed") return "";
  return `这条线索已自动判为「${DISPOSITION_LABELS[disposition.disposition]}」（${disposition.reason}），不是待整改风险。请在下方人工改判。`;
}

/// G1-0 one line naming where the caps come from, so a default is never read as a project
/// decision and an unreadable configuration is never shown as a set of numbers.
export function describeLimits(summary: ExposureScopeSummary | null): string {
  if (!summary) return "尚未读取采集上限";
  if (summary.limitsError) return `采集上限不可用：${summary.limitsError}`;
  const rows = summary.knownLimits.map((row) => `${row.label} ${row.cap}`).join(" · ");
  if (!rows) return "尚未读取采集上限";
  const basis = summary.limitsBasis.startsWith("project_settings_v")
    ? `项目配置 v${summary.limitsBasis.slice("project_settings_v".length)}`
    : summary.limitsBasis === "defaults"
      ? "代码默认值（项目尚未保存配置）"
      : "来源未报告";
  return `${rows}（${basis}）`;
}

/// The caps the *legacy* endpoints still apply. Until the paginated commands are wired,
/// the page must show these numbers instead of implying full coverage.
export const LEGACY_WINDOWS = { findings: 500, runs: 20, boundAssetsPage: 200 } as const;

export function legacyWindowNotice(): string {
  return `旧接口上限：线索最多前 ${LEGACY_WINDOWS.findings} 条、任务最近 ${LEGACY_WINDOWS.runs} 条、单页资产最多 ${LEGACY_WINDOWS.boundAssetsPage} 条`;
}

/// One honest sentence about what the current passive round can and cannot reach.
export function describePassiveScope(summary: ExposureScopeSummary | null): string {
  if (!summary) return "尚未读取采集范围";
  if (summary.limitsError) return `计划数未核实：${summary.limitsError}`;
  const planned = `本轮计划 ${summary.eligibleAssetsPlanned} 个准入资产`;
  if (summary.assetsBeyondPlan > 0) {
    return `${planned}（全量 ${summary.eligibleAssetsTotal}，另有 ${summary.assetsBeyondPlan} 个本轮未采集）`;
  }
  if (!summary.projectActive) return "工作空间已归档，本轮没有采集范围";
  return `${planned}（全量 ${summary.eligibleAssetsTotal}）`;
}

export type JsonLike = Record<string, unknown>;

/// §Q4 — a name or URL an external index handed back, waiting for a human.
export interface ExposurePending {
  id: number;
  projectId: number;
  runId: number;
  sourceKey: string;
  kind: string;
  entity: string;
  canonicalKey: string;
  evidenceUrl: string;
  status: string;
  createdAt: string;
}

export function isPendingRow(value: unknown): value is ExposurePending {
  return (
    isRecord(value) &&
    positiveFinite(value.id) &&
    typeof value.entity === "string" &&
    typeof value.sourceKey === "string" &&
    typeof value.status === "string"
  );
}

/// §Q5 — the evidence behind one finding: immutable observations, the version trail, and the
/// review-needed marker. Nothing here can carry a raw secret because the backend redacts on
/// write; the reader still refuses rows without a provenance version.
export interface ExposureObservation {
  id: number;
  runId: number;
  sourceKey: string;
  kind: string;
  url: string;
  finalUrl: string;
  statusCode: number;
  contentType: string;
  contentHash: string;
  headersHash: string;
  bodyBytes: number;
  truncated: boolean;
  ruleVersion: string;
  redactionVersion: string;
  boundary: string;
  currentAccessible: boolean | null;
  indexedAt: string;
  queriedAt: string;
  evidence: unknown;
}

export interface ExposureFindingVersion {
  revision: number;
  runId: number;
  contentHash: string;
  ruleVersion: string;
  redactionVersion: string;
  severity: string;
  manualStatus: string;
  reviewNeeded: boolean;
  createdAt: string;
}

export interface ExposureEvidence {
  finding: { id: number; category: string; title: string; confidence: number; status: string };
  observations: ExposureObservation[];
  versions: ExposureFindingVersion[];
  reviewEvents: { fromStatus: string; toStatus: string; note: string; actor: string; createdAt: string }[];
  reviewNeeded: { revision: number; ruleVersion: string } | null;
  /** G1-3 current disposition (null for a pre-G1-3 finding or an older backend). */
  disposition: ExposureDispositionView | null;
  classifications: { observationId: number | null; runId: number; disposition: string; infoCategory: string; reason: string; markers: string[] }[];
  classificationError: string;
  /** G3-2 intel grade of an intel lead (absent for other leads and older backends). */
  intelGrade?: string | null;
  intelGradeError?: string | null;
}

function isObservation(value: unknown): value is ExposureObservation {
  return (
    isRecord(value) &&
    positiveFinite(value.id) &&
    typeof value.url === "string" &&
    typeof value.contentHash === "string" &&
    typeof value.ruleVersion === "string" &&
    value.ruleVersion !== "" &&
    typeof value.redactionVersion === "string" &&
    value.redactionVersion !== ""
  );
}

export function readEvidence(value: unknown): ExposureEvidence {
  if (!isRecord(value)) throw new Error("证据包返回结构无效");
  const finding = value.finding;
  if (!isRecord(finding) || !positiveFinite(finding.id)) throw new Error("证据包缺少线索主体");
  if (!Array.isArray(value.observations) || !Array.isArray(value.versions)) {
    throw new Error("证据包缺少观察或版本列表");
  }
  const observations: ExposureObservation[] = [];
  for (const row of value.observations) {
    if (!isObservation(row)) throw new Error("证据包含无效观察");
    observations.push(row);
  }
  const versions: ExposureFindingVersion[] = [];
  for (const row of value.versions) {
    if (!isRecord(row) || !positiveFinite(row.revision)) throw new Error("证据包含无效版本");
    versions.push(row as unknown as ExposureFindingVersion);
  }
  return {
    finding: finding as unknown as ExposureEvidence["finding"],
    observations,
    versions,
    reviewEvents: Array.isArray(value.reviewEvents)
      ? (value.reviewEvents as ExposureEvidence["reviewEvents"])
      : [],
    reviewNeeded: isRecord(value.reviewNeeded)
      ? (value.reviewNeeded as unknown as ExposureEvidence["reviewNeeded"])
      : null,
    disposition: value.disposition === undefined || value.disposition === null ? null : readDispositionView(value.disposition, "证据包"),
    classifications: Array.isArray(value.classifications)
      ? (value.classifications as ExposureEvidence["classifications"])
      : [],
    classificationError: typeof value.classificationError === "string" ? value.classificationError : "",
    intelGrade: typeof value.intelGrade === "string" ? value.intelGrade : null,
    intelGradeError: typeof value.intelGradeError === "string" ? value.intelGradeError : null,
  };
}

/// One sentence that keeps "inherited" and "directly observed" apart, so the page can never
/// present a cluster conclusion as if it had fetched that asset.
export function evidenceHeadline(evidence: ExposureEvidence | null): string {
  if (!evidence) return "尚未读取证据";
  const rounds = new Set(evidence.observations.map((row) => row.runId)).size;
  const current = evidence.observations.filter((row) => row.currentAccessible === true).length;
  const unknown = evidence.observations.filter((row) => row.currentAccessible === null).length;
  const parts = [`${evidence.observations.length} 条观察`, `跨 ${rounds} 轮`];
  if (current > 0) parts.push(`${current} 条本轮确认可访问`);
  if (unknown > 0) parts.push(`${unknown} 条当前可访问性未知（仅索引命中）`);
  if (evidence.reviewNeeded) parts.push(`内容或规则已变化，需复核（版本 ${evidence.reviewNeeded.revision}）`);
  return parts.join(" · ");
}

/// The refusal text of a frozen plan, kept readable but never turned into a success.
export function resumeRefusalText(error: unknown): string {
  const text = exposureErrorText(error);
  if (text.includes("exposure_plan_frozen_mismatch")) {
    return "冻结的计划与当前授权或配置不一致，需另起新一轮采集，旧结果保留";
  }
  if (text.includes("exposure_plan_missing")) {
    return "这是旧版有限采集留下的任务，没有工作项台账，不能按新合同恢复";
  }
  return text;
}

// ───────────── G2 GitHub: source names, statuses and capabilities ─────────────

export const SOURCE_LABELS: Record<string, string> = {
  passive_web: "被动 Web 采集",
  certificate_transparency: "证书透明度",
  web_archive: "历史归档",
  github_repositories: "GitHub 仓库元数据（匿名）",
  github_code: "GitHub 代码搜索（需凭据）",
  document_link: "指定公开文档 URL",
  document_import: "文件导入（用户提供）",
  public_intel_import: "公开情报导入（用户提供）",
};

export const sourceLabel = (key: string): string => SOURCE_LABELS[key] ?? `未知来源 ${key}`;

export const SOURCE_STATUS_LABELS: Record<string, string> = {
  completed: "已完成（只代表本轮有界结果）",
  completed_no_matches: "已执行，有界结果未命中（不代表没有泄漏）",
  partial: "部分完成",
  failed: "失败",
  rate_limited: "额度受限",
  auth_failed: "身份被拒",
  unconfigured: "未配置",
  unsupported: "未实现",
  parse_error: "响应无法解析",
  cancelled: "已取消",
};

/** Reason codes the GitHub adapter writes at the start of a source error. */
export const GITHUB_REASON_LABELS: Record<string, string> = {
  github_auth_failed_401: "401 身份无效",
  github_forbidden_403: "403 无权访问",
  github_rate_limited_403: "403 额度用尽",
  github_rate_limited_429: "429 请求过多",
  github_rate_limited_remaining_0: "额度剩余 0，停止翻页",
  github_query_rejected_422: "422 查询被拒绝",
  github_unavailable_503: "503 服务暂不可用",
  github_http_error: "其它 HTTP 错误",
  github_redirect_not_followed: "接口跳转未跟随（凭据不随跳转）",
  github_link_rejected: "下一页链接不合规，未访问",
  github_parse_error: "响应无法解析",
  integration_pending: "未配置：凭据引用端口待接入（IR-04）",
};

/** G3: reason codes the document reader and the import path write at the start of an error. */
export const DOCUMENT_REASON_LABELS: Record<string, string> = {
  document_link_unconfigured: "未指定文档 URL",
  document_link_not_in_frozen_list: "地址不在本轮冻结清单内，未访问",
  document_url_invalid: "地址不合规，未访问",
  document_port_denied: "非 80/443 端口，未访问",
  document_private_denied: "目标是内网/本机地址，未访问",
  document_scheme_denied: "协议不允许，未访问",
  document_userinfo_denied: "地址带账号信息，未访问",
  document_budget_exhausted: "本轮请求额度用尽，未访问",
  document_transport_failed: "网络连接失败",
  document_redirect_not_followed: "返回跳转，未跟随（只读指定地址）",
  document_access_denied: "401/403 无权访问",
  document_not_found: "404/410 文档不存在",
  document_rate_limited: "429 请求过多",
  document_http: "其它 HTTP 状态",
  document_unsupported_type: "类型不支持（PDF/Office/压缩包/图片），只记元数据",
  document_charset_unsupported: "非 UTF-8 编码，未解析",
  document_not_utf8: "正文不是合法 UTF-8，未解析",
  document_truncated: "正文超出单响应上限，只分析了前段",
  import_not_resumable: "导入批次中断后不能按网络任务恢复，需重新导入",
  unsupported_format: "文件格式不支持（PDF/Office/图片等）",
  invalid_utf8: "文件不是合法 UTF-8",
  parse_error: "文件内容无法按声明格式解析",
};

const SOURCE_REASON_LABELS: Record<string, string> = { ...GITHUB_REASON_LABELS, ...DOCUMENT_REASON_LABELS };

/** Status-suffixed codes such as `document_http_500` or `document_access_denied_403`. */
const SUFFIXED_REASON = /^(document_(?:http|access_denied|not_found|rate_limited))_\d{3}$/;

function reasonLabel(code: string): string {
  const exact = SOURCE_REASON_LABELS[code];
  if (exact) return exact;
  const suffixed = SUFFIXED_REASON.exec(code);
  return suffixed ? `${SOURCE_REASON_LABELS[suffixed[1]]}（${code.slice(suffixed[1].length + 1)}）` : "";
}

export type SourceTone = "ok" | "empty" | "pending" | "gap";

export interface SourceStatusView {
  label: string;
  status: string;
  reason: string;
  detail: string;
  tone: SourceTone;
}

/** The leading `code:` of a source error, if it is one the page knows. */
export function sourceReasonCode(error: string): string {
  const head = (error ?? "").split(/[:：]/, 1)[0]?.trim() ?? "";
  return reasonLabel(head) ? head : "";
}

export function sourceStatusView(row: { sourceKey: string; status: string; error?: string }): SourceStatusView {
  const error = row.error ?? "";
  const code = sourceReasonCode(error);
  const tone: SourceTone =
    row.status === "completed"
      ? "ok"
      : row.status === "completed_no_matches"
        ? "empty"
        : row.status === "unconfigured" && code === "integration_pending"
          ? "pending"
          : "gap";
  return {
    label: sourceLabel(row.sourceKey),
    status: SOURCE_STATUS_LABELS[row.status] ?? `未知状态 ${row.status}`,
    reason: code ? reasonLabel(code) : "",
    detail: error,
    tone,
  };
}

export interface SourceCapability {
  sourceKey: string;
  capability: string;
  credential: "none" | "required" | string;
  state: string;
  integration: string;
  reason: string;
}

export const CAPABILITY_LABELS: Record<string, string> = {
  repository_metadata: "仓库元数据搜索",
  default_branch_code_search: "默认分支代码搜索",
  controlled_blob_read: "命中文件受控读取",
  explicit_document_url: "指定公开文档 URL 只读",
  file_import: "文件导入（文本/JSON/CSV/已提取文本）",
  public_intel_import: "公开情报导入与分级",
  pdf_docx_ocr: "PDF / Office / 图片 OCR",
  netdisk_keyword_search: "文库/网盘关键词搜索",
  underground_feed: "暗网/地下情报源接入",
};

/** `null` when the backend does not report capabilities (an older build): the page then says
 *  so instead of assuming anything is available. */
export function readSourceCapabilities(raw: unknown): SourceCapability[] | null {
  if (!Array.isArray(raw)) return null;
  return raw
    .filter((row): row is Record<string, unknown> => typeof row === "object" && row !== null)
    .filter(row => typeof row.capability === "string" && typeof row.state === "string")
    .map(row => ({
      sourceKey: String(row.sourceKey ?? ""),
      capability: String(row.capability),
      credential: String(row.credential ?? ""),
      state: String(row.state),
      integration: String(row.integration ?? ""),
      reason: String(row.reason ?? ""),
    }));
}

export type CapabilityTone = "ok" | "pending" | "gap" | "unsupported";

export function capabilityTone(cap: SourceCapability): CapabilityTone {
  if (cap.state === "configured") return "ok";
  if (cap.state === "unsupported") return "unsupported";
  if (cap.state === "integration_pending" || cap.integration === "integration_pending") return "pending";
  return "gap";
}

export function capabilityLine(cap: SourceCapability): string {
  const name = CAPABILITY_LABELS[cap.capability] ?? cap.capability;
  const credential = cap.credential === "none" ? "无需凭据" : cap.credential === "required" ? "需要凭据" : `凭据：${cap.credential}`;
  const state =
    cap.state === "configured"
      ? "可用"
      : cap.state === "integration_pending"
        ? "已实现 · 入口待注册（integration_pending）"
        : cap.integration === "integration_pending"
          ? "未配置（integration_pending）"
          : cap.state === "unconfigured"
            ? "未配置"
            : cap.state === "unsupported"
              ? "未实现"
              : cap.state;
  return `${name} · ${credential} · ${state}${cap.reason ? `：${cap.reason}` : ""}`;
}

// Same rule as the backend: legacy logins such as `rob--w` are real and accepted.
const GITHUB_LOGIN = /^(?=.*[a-z0-9])[a-z0-9-]{1,39}$/;

/** The official open-source owner list as typed by an operator: one login per line or comma.
 *  Invalid entries are reported, never silently dropped. */
export function parseOwnerList(text: string): { owners: string[]; invalid: string[] } {
  const owners: string[] = [];
  const invalid: string[] = [];
  for (const raw of (text ?? "").split(/[\s,，、]+/)) {
    const login = raw.trim().toLowerCase();
    if (!login) continue;
    if (!GITHUB_LOGIN.test(login)) invalid.push(raw.trim());
    else if (!owners.includes(login)) owners.push(login);
  }
  return { owners: owners.sort(), invalid };
}

// ───────────── G3 documents and intel ─────────────

export const INTEL_GRADE_LABELS: Record<string, string> = {
  claim_only: "仅声称（无样本支撑）",
  sample_supported: "样本支撑（样本出现集团名称或凭据形态值）",
  human_verified: "人工核实（有样本或主体锚点）",
};

export const intelGradeLabel = (grade: string): string => INTEL_GRADE_LABELS[grade] ?? `未知等级 ${grade}`;

export interface DocumentUrlEntry {
  url: string;
  publicMaterial: boolean;
  note: string;
}

export const DOCUMENT_URLS_LIMIT = 20;
const DOCUMENT_NOTE_LIMIT = 200;

/** Same canonical form as the backend: http/https, a host, no account part, no fragment. */
export function canonicalDocumentUrl(raw: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(raw.trim());
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  if (!parsed.hostname || parsed.username || parsed.password) return null;
  parsed.hash = "";
  return parsed.toString();
}

/** One entry per line: `URL`, or `URL | 理由` to declare the document public material (the
 *  reason is required by the backend and kept as evidence). Invalid lines are named, never
 *  silently dropped. */
export function parseDocumentUrls(text: string): { entries: DocumentUrlEntry[]; invalid: string[] } {
  const byUrl = new Map<string, DocumentUrlEntry>();
  const invalid: string[] = [];
  for (const line of (text ?? "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const bar = trimmed.indexOf("|");
    const rawUrl = bar >= 0 ? trimmed.slice(0, bar).trim() : trimmed;
    const note = bar >= 0 ? trimmed.slice(bar + 1).trim() : "";
    const url = canonicalDocumentUrl(rawUrl);
    if (!url || (bar >= 0 && !note) || [...note].length > DOCUMENT_NOTE_LIMIT) {
      invalid.push(trimmed);
      continue;
    }
    byUrl.set(url, { url, publicMaterial: bar >= 0, note });
  }
  const entries = [...byUrl.values()].sort((a, b) => (a.url < b.url ? -1 : a.url > b.url ? 1 : 0));
  if (entries.length > DOCUMENT_URLS_LIMIT) invalid.push(`最多 ${DOCUMENT_URLS_LIMIT} 个地址（当前 ${entries.length} 个）`);
  return { entries, invalid };
}

/** Reads the backend's `exposureDocumentUrls` (strings or objects) back into entries. */
export function readDocumentUrls(raw: unknown): DocumentUrlEntry[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map(row =>
      typeof row === "string"
        ? { url: row, publicMaterial: false, note: "" }
        : typeof row === "object" && row !== null && typeof (row as Record<string, unknown>).url === "string"
          ? {
              url: String((row as Record<string, unknown>).url),
              publicMaterial: (row as Record<string, unknown>).publicMaterial === true,
              note: String((row as Record<string, unknown>).note ?? ""),
            }
          : null,
    )
    .filter((row): row is DocumentUrlEntry => row !== null);
}

export function formatDocumentUrls(entries: DocumentUrlEntry[]): string {
  return entries.map(row => (row.publicMaterial ? `${row.url} | ${row.note}` : row.url)).join("\n");
}

/** The intel grade line of an evidence package (only intel leads carry one). A grade that could
 *  not be read is shown as such, never as a lower or higher grade. */
export function intelGradeLine(pkg: unknown): string {
  if (typeof pkg !== "object" || pkg === null) return "";
  const row = pkg as Record<string, unknown>;
  if (typeof row.intelGradeError === "string" && row.intelGradeError) return `情报等级读取失败：${row.intelGradeError}`;
  if (typeof row.intelGrade !== "string" || !row.intelGrade) return "";
  return `情报等级：${intelGradeLabel(row.intelGrade)}（声称与样本均来自用户提供的报告，来源未经应用验证）`;
}

/// G4-3 The export result line: rows and the digest read back from the landed file.
export function exportLine(result: { path: string; rows: number }, verified: unknown): string {
  if (!isRecord(verified)) return `已导出 ${result.rows} 行：${result.path}（未能读回核验）`;
  const sha = (typeof verified.sha256 === "string" ? verified.sha256 : "");
  const format = (typeof verified.format === "string" ? verified.format : "");
  return `已导出 ${(typeof verified.rows === "number" ? verified.rows : result.rows)} 行（${format || "格式未报告"}）· SHA-256 ${sha.slice(0, 16)}… · ${result.path}`;
}

// ——— IR-14 / IR-15 landed 2026-10-09: import and grade payloads ———

const IMPORT_CHANNELS = ["document_import", "public_intel_import"] as const;
const IMPORT_FORMATS = ["text", "json", "csv", "extracted_text"] as const;
const INTEL_FORMATS = ["json", "csv"] as const;
const INTEL_GRADES = ["claim_only", "sample_supported", "human_verified"] as const;

export interface ImportFileInput {
  name: string;
  format: string;
  contentBase64: string;
}

export interface ImportFormInput {
  channel: string;
  operator: string;
  sourceDeclaration: string;
  declaredUrl?: string;
  batchLabel?: string;
  publicMaterial?: boolean;
  publicMaterialNote?: string;
  extractedFrom?: string;
  files: ImportFileInput[];
}

/// Builds the `import_exposure_documents` body, or explains why it must not be sent.
/// A declared URL is stored as a claim only: this function never fetches it.
export function buildImportRequest(
  form: ImportFormInput,
  operationId: string,
): { ok: true; payload: JsonLike } | { ok: false; error: string } {
  if (!/^[A-Za-z0-9_-]{8,80}$/.test(operationId)) return { ok: false, error: "操作编号必须是 8~80 位字母、数字、- 或 _" };
  if (!IMPORT_CHANNELS.includes(form.channel as (typeof IMPORT_CHANNELS)[number])) return { ok: false, error: "请选择导入渠道" };
  const operator = form.operator.trim();
  const sourceDeclaration = form.sourceDeclaration.trim();
  if (!operator || operator.length > 80) return { ok: false, error: "请填写操作人（不超过 80 字）" };
  if (!sourceDeclaration || sourceDeclaration.length > 500) return { ok: false, error: "请填写来源说明（这份材料从哪来）" };
  const note = (form.publicMaterialNote ?? "").trim();
  if (form.publicMaterial && (note.length < 1 || note.length > 200)) return { ok: false, error: "声明为公开材料必须写明理由" };
  if (!form.files.length) return { ok: false, error: "请选择至少一个文件" };
  if (form.files.length > 20) return { ok: false, error: "一批最多 20 个文件" };
  const allowed = form.channel === "public_intel_import" ? INTEL_FORMATS : IMPORT_FORMATS;
  for (const file of form.files) {
    const name = file.name.trim();
    if (!name || name.length > 200 || /[/\\]/.test(name) || name === "." || name === "..") return { ok: false, error: `文件名不合规：${file.name || "（空）"}（不能是路径）` };
    if (!allowed.includes(file.format as "json")) return { ok: false, error: `${name} 的格式 ${file.format} 不在本渠道允许范围内` };
    if (!file.contentBase64) return { ok: false, error: `${name} 没有内容` };
  }
  const payload: Record<string, unknown> = {
    operationId,
    channel: form.channel,
    operator,
    sourceDeclaration,
    files: form.files.map((file) => ({ name: file.name.trim(), format: file.format, contentBase64: file.contentBase64 })),
  };
  if ((form.declaredUrl ?? "").trim()) payload.declaredUrl = form.declaredUrl!.trim();
  if ((form.batchLabel ?? "").trim()) payload.batchLabel = form.batchLabel!.trim();
  if (form.publicMaterial) {
    payload.publicMaterial = true;
    payload.publicMaterialNote = note;
  }
  if ((form.extractedFrom ?? "").trim()) payload.extractedFrom = form.extractedFrom!.trim();
  return { ok: true, payload: payload as JsonLike };
}

export interface GradeFormInput {
  findingId: number;
  expectedGrade: string;
  grade: string;
  actor: string;
  reason: string;
  sampleObservationId?: number;
  entityId?: number;
}

/// Builds the `grade_exposure_intel` body. human_verified needs a sample or a directory
/// entity; the application never contacts the claimed source.
export function buildGradeRequest(
  form: GradeFormInput,
  operationId: string,
): { ok: true; payload: JsonLike } | { ok: false; error: string } {
  if (!/^[A-Za-z0-9_-]{8,80}$/.test(operationId)) return { ok: false, error: "操作编号必须是 8~80 位字母、数字、- 或 _" };
  if (!INTEL_GRADES.includes(form.grade as (typeof INTEL_GRADES)[number])) return { ok: false, error: "请选择等级" };
  if (!form.expectedGrade) return { ok: false, error: "没有读到当前等级，不能改级" };
  const actor = form.actor.trim();
  const reason = form.reason.trim();
  if (!actor || actor.length > 80) return { ok: false, error: "请填写操作人" };
  if (reason.length < 4 || reason.length > 500) return { ok: false, error: "改级必须写明理由（至少 4 个字）" };
  const sample = form.sampleObservationId;
  const entity = form.entityId;
  const sampleOk = typeof sample === "number" && Number.isInteger(sample) && sample > 0;
  const entityOk = typeof entity === "number" && Number.isInteger(entity) && entity > 0;
  if (form.grade === "human_verified" && !sampleOk && !entityOk) {
    return { ok: false, error: "定为已人工核验需要本线索的样本观察，或组织目录里的单位" };
  }
  if (form.grade === "sample_supported" && !sampleOk) {
    return { ok: false, error: "定为样本支持需要本线索里的样本观察编号" };
  }
  const payload: Record<string, unknown> = {
    operationId,
    findingId: form.findingId,
    expectedGrade: form.expectedGrade,
    grade: form.grade,
    actor,
    reason,
  };
  if (sampleOk) payload.sampleObservationId = sample;
  if (entityOk) payload.entityId = entity;
  return { ok: true, payload: payload as JsonLike };
}


export function buildReclassifyRequest(
  form: { findingId: number; expectedClassificationId: number; disposition: string; infoCategory: string; actor: string; reason: string },
  operationId: string,
): { ok: true; payload: JsonLike } | { ok: false; error: string } {
  if (!/^[A-Za-z0-9_-]{8,80}$/.test(operationId)) return { ok: false, error: "操作编号必须是 8~80 位字母、数字、- 或 _" };
  if (!(form.disposition in DISPOSITION_LABELS)) return { ok: false, error: "请选择处置" };
  if (!(form.infoCategory in INFO_CATEGORY_LABELS)) return { ok: false, error: "请选择信息类别" };
  if (!Number.isInteger(form.expectedClassificationId) || form.expectedClassificationId <= 0) {
    return { ok: false, error: "没有读到当前分类编号，不能改判" };
  }
  const actor = form.actor.trim();
  const reason = form.reason.trim();
  if (!actor || actor.length > 80) return { ok: false, error: "请填写操作人" };
  if (reason.length < 4 || reason.length > 500) return { ok: false, error: "改判必须写明理由（至少 4 个字）" };
  return {
    ok: true,
    payload: {
      operationId,
      findingId: form.findingId,
      expectedClassificationId: form.expectedClassificationId,
      disposition: form.disposition,
      infoCategory: form.infoCategory,
      actor,
      reason,
    },
  };
}
