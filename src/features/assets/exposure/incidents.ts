// G4 — incidents, coverage, live progress and unit statistics: the pure wording layer. Nothing
// here talks to the backend. Every function turns a backend answer into lines that cannot
// overstate it: a missing block reads "未报告", a gap never reads as green or "无风险", the
// unit sum is shown next to the group-unique count, and handling actions say they were only
// recorded (the application never contacts anyone).

import type { JsonLike } from "./contracts";

type Row = Record<string, unknown>;

const isRow = (value: unknown): value is Row => typeof value === "object" && value !== null && !Array.isArray(value);
const num = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) ? value : null);
const text = (value: unknown): string => (typeof value === "string" ? value : "");

export const INCIDENT_STATUS_LABELS: Record<string, string> = {
  candidate: "候选（待复核）",
  reviewing: "复核中",
  verified: "已核实",
  dismissed: "已排除",
  excluded_public: "公开材料已排除",
  merged: "已并入其它事件",
};

export const HANDLING_LABELS: Record<string, string> = {
  none: "未登记处置",
  notified: "已登记通知（人工）",
  takedown_requested: "已登记下架申请（人工）",
  takedown_reported: "已登记下架完成（人工，未经复测）",
  remediating: "已登记整改（人工）",
  retest_recorded: "已关联复测",
  closed: "已登记关闭（人工）",
};

/// Actions a person records after doing them elsewhere. None of them sends anything.
export const HANDLING_ACTIONS: { value: string; label: string; needsChannel: boolean }[] = [
  { value: "notification_recorded", label: "登记：已通知（填写通知了谁、通过什么渠道）", needsChannel: true },
  { value: "takedown_requested_recorded", label: "登记：已提交下架申请", needsChannel: false },
  { value: "takedown_confirmed_recorded", label: "登记：对方称已下架", needsChannel: false },
  { value: "remediation_recorded", label: "登记：已整改", needsChannel: false },
  { value: "retest_linked", label: "关联一轮复测（需要轮次号）", needsChannel: false },
  { value: "closed_recorded", label: "登记关闭（需写明依据）", needsChannel: false },
  { value: "reopened", label: "重新打开", needsChannel: false },
];

export const HANDLING_NOTICE = "这里只登记人工已经做过的通知、下架或整改；应用不会联系任何人或平台，也不会自动关闭事件。";

export const LINK_LEVEL_LABELS: Record<string, string> = {
  confirmed_link: "确认关联",
  candidate_link: "候选关联",
  unknown: "单位未知",
  conflict: "关联冲突",
};

export const BASIS_LABELS: Record<string, string> = {
  identical_normalized_full_text: "规范化全文相同",
  identical_full_response_body: "完整响应体相同",
  single_risk_lead: "单条风险线索",
  manual_split: "人工拆分",
};

export const EVIDENCE_GRADE_LABELS: Record<string, string> = {
  observed_content: "实际读取了内容",
  imported: "用户导入",
  claim: "情报声称（未见样本前不等于已核实）",
  index_only: "仅索引命中",
  snippet_or_metadata: "摘要或元数据",
};

export const COVERAGE_STATE_LABELS: Record<string, string> = {
  unconfigured: "未配置",
  not_implemented: "未实施",
  pending_authorization: "待授权",
  planned: "计划中",
  not_run: "已配置·尚未执行",
  executed: "已执行",
  executed_no_hit: "已执行无命中",
  candidate: "存在候选",
  verified_incident: "存在已核实事件",
  partial: "部分执行",
  failed: "失败",
  cancelled_interrupted: "取消/中断",
  not_applicable: "不适用",
};

export const INFO_CATEGORY_LABELS: Record<string, string> = {
  internal_business: "内部业务",
  technical_network: "技术与网络",
  source_code: "源代码",
  credential: "凭据",
  employee_data: "员工信息",
  customer_data: "客户信息",
  storage_misexposure: "存储误公开",
  sold_intel: "出售情报",
  unclassified: "未分类",
};

export interface CoverageCellView {
  category: string;
  state: string;
  label: string;
  tone: "green" | "amber" | "red" | "grey";
}

/// A cell is green only when the backend said `executed_no_hit` with tone green. Anything the
/// page does not recognise is grey, never green.
export function coverageCellView(cell: unknown): CoverageCellView {
  if (!isRow(cell)) return { category: "", state: "unknown", label: "未报告", tone: "grey" };
  const state = text(cell.state);
  const backendTone = text(cell.tone);
  const tone = state === "executed_no_hit" && backendTone === "green"
    ? "green"
    : (["amber", "red"].includes(backendTone) ? backendTone : "grey") as CoverageCellView["tone"];
  return {
    category: text(cell.infoCategory),
    state,
    label: COVERAGE_STATE_LABELS[state] ?? `未知状态 ${state || "（空）"}`,
    tone,
  };
}

export interface CoverageRowView {
  capability: string;
  sourceKey: string;
  /** The round this row is read from: run, status, start and finish time (P1 时间窗口). */
  window: string;
  label: string;
  state: string;
  reason: string;
  denominator: string;
  integrationPending: boolean;
  cells: CoverageCellView[];
}

const CAPABILITY_LABELS: Record<string, string> = {
  first_party_web: "第一方网页",
  certificate_index: "证书透明度索引",
  archive_index: "历史归档索引",
  repository_metadata: "代码平台仓库元数据",
  code_search: "代码平台代码搜索",
  explicit_document_url: "指定公开文档 URL",
  file_import: "文件导入",
  public_intel_import: "公开情报导入",
  pdf_docx_ocr: "PDF/Office/OCR",
  netdisk_keyword_search: "网盘/文库关键词搜索",
  underground_feed: "地下市场/暗网",
};

const COVERAGE_RUN_STATUS: Record<string, string> = {
  queued: "排队中", running: "进行中", completed: "已完成", partial: "部分完成", failed: "失败", cancelled: "已取消", interrupted: "已中断",
};

/// One matrix row with its denominator spelled out: unit, planned, executed, uncovered.
export function coverageRows(coverage: unknown): CoverageRowView[] {
  if (!isRow(coverage) || !Array.isArray(coverage.rows)) return [];
  return coverage.rows.filter(isRow).map((row) => {
    const d = isRow(row.denominator) ? row.denominator : {};
    const unit = text(d.unit) || "项";
    const planned = num(d.planned) ?? 0;
    const executed = num(d.executed) ?? 0;
    const uncovered = num(d.uncovered) ?? Math.max(planned - executed, 0);
    const outside = num(d.outsidePlan) ?? 0;
    const parts = [`计划 ${planned} ${unit}`, `已执行 ${executed}`, `未覆盖 ${uncovered}`];
    if ((num(d.failed) ?? 0) > 0) parts.push(`失败 ${d.failed}`);
    if ((num(d.blockedQuota) ?? 0) > 0) parts.push(`配额阻塞 ${d.blockedQuota}`);
    if ((num(d.blockedAuthorization) ?? 0) > 0) parts.push(`待授权 ${d.blockedAuthorization}`);
    if (outside > 0) parts.push(`另有 ${outside} 个超出本轮上限未计划`);
    const state = text(row.state);
    const w = isRow(row.window) ? row.window : null;
    return {
      capability: text(row.capability),
      sourceKey: text(row.sourceKey),
      window: w
        ? `第 ${num(w.runId) ?? "?"} 轮（${COVERAGE_RUN_STATUS[text(w.runStatus)] ?? text(w.runStatus)}）· 开始 ${text(w.createdAt) || "—"} · 结束 ${text(w.completedAt) || "未结束"}`
        : "没有计划过这个来源的轮次",
      label: CAPABILITY_LABELS[text(row.capability)] ?? text(row.capability),
      state,
      reason: text(row.reason),
      denominator: parts.join(" · "),
      integrationPending: text(row.integration) === "integration_pending",
      cells: Array.isArray(row.cells) ? row.cells.map(coverageCellView) : [],
    };
  });
}

export function coverageNotice(coverage: unknown, error: string): string {
  if (error) return `覆盖矩阵不可用：${error}（不是“全部覆盖”，也不是“没有风险”）`;
  if (!isRow(coverage)) return "后端未报告覆盖矩阵（integration_pending）";
  const axis = isRow(coverage.unitAxis) ? coverage.unitAxis : {};
  const unitPart = text(axis.state) === "reported" ? "" : "；单位维度未报告";
  return `只有全部工作项完成且无命中的格子才显示为绿色；失败、阻塞、取消和超出上限都留在分母里${unitPart}`;
}

/// G4-2 the live lines. A percent is shown only when the backend gave one, with its basis.
export function liveRunLines(live: unknown, error: string): string[] {
  if (error) return [`实时进度不可用：${error}`];
  if (!isRow(live)) return [];
  const lines: string[] = [];
  const progress = isRow(live.progress) ? live.progress : {};
  const percent = num(progress.percent);
  const head = `第 ${live.runId} 轮 · ${text(live.status)} · 阶段 ${text(live.stage) || "—"}${text(live.currentSource) ? ` · 当前来源 ${text(live.currentSource)}` : ""}`;
  lines.push(head);
  const done = num(progress.done) ?? 0;
  const total = num(progress.total) ?? 0;
  lines.push(
    percent === null
      ? `工作项 ${done} / ${total}（尚无可计算的进度）`
      : `工作项 ${done} / ${total}（${percent}%，${text(progress.note) || "按工作项计"}）`,
  );
  if (Array.isArray(live.sources)) {
    for (const source of live.sources.filter(isRow)) {
      lines.push(`· ${text(source.sourceKey)}：${num(source.completed) ?? 0} / ${num(source.total) ?? 0} 完成`);
    }
  }
  const usage = isRow(live.usage) ? live.usage : {};
  const budget = num(usage.budgetRequests);
  lines.push(`请求 ${num(usage.requests) ?? 0}${budget !== null ? ` / 预算 ${budget}` : ""} · 字节 ${num(usage.bytes) ?? 0}${num(usage.budgetBytes) !== null ? ` / 预算 ${usage.budgetBytes}` : ""}`);
  if (text(live.lastPersistedAt)) lines.push(`最近落盘 ${text(live.lastPersistedAt)}`);
  if (live.cancelRequested === true) lines.push("已请求取消：当前工作项结束后停止，已采集结果保留");
  const gaps = num(live.gapsTotal) ?? 0;
  if (gaps > 0) lines.push(`未闭合工作项 ${gaps} 个${live.gapsTruncated === true ? "（只列出前 20 个）" : ""}`);
  if (live.resumable === true) lines.push("这一轮可以按冻结计划恢复（已完成项不会重跑）");
  return lines;
}

/// G4-1 the statistics lines: group-unique next to the per-unit sum, provinces, professional
/// subsidiaries apart, unknown unit, and "未报告" where the backend had nothing.
export function unitStatisticsLines(stats: unknown, error: string): string[] {
  if (error) return [`单位统计不可用：${error}`];
  if (!isRow(stats)) return ["后端未报告单位统计（integration_pending）"];
  const lines: string[] = [];
  const group = isRow(stats.groupUnique) ? stats.groupUnique : {};
  const open = num(group.open) ?? 0;
  const sum = num(stats.sumOfUnitInvolvement) ?? 0;
  lines.push(`集团唯一事件 ${open} 起（未关闭）；单位涉及数之和 ${sum}${sum > open ? "（一起事件涉及多个单位时各计一次，所以大于集团数）" : ""}`);
  if (Array.isArray(stats.provinces)) {
    for (const row of stats.provinces.filter(isRow)) {
      lines.push(`· ${text(row.legalName)}：唯一事件 ${num(row.uniqueIncidents) ?? 0}（确认 ${num(row.confirmed) ?? 0} / 候选 ${num(row.candidate) ?? 0} / 冲突 ${num(row.conflict) ?? 0}）`);
    }
  }
  if (Array.isArray(stats.professionalSubsidiaries) && stats.professionalSubsidiaries.length) {
    lines.push(`专业公司单列：${stats.professionalSubsidiaries.filter(isRow).map((row) => `${text(row.legalName)} ${(num(row.confirmed) ?? 0) + (num(row.candidate) ?? 0)}`).join("，")}`);
  }
  lines.push(`单位未知的事件 ${num(stats.unknownUnitIncidents) ?? 0} 起`);
  const dedup = isRow(stats.dedup) ? stats.dedup : {};
  lines.push(`风险线索 ${num(dedup.riskLeads) ?? 0} 条 → 去重后事件 ${num(dedup.openIncidents) ?? 0} 起，有效传播位置 ${num(dedup.activeLocations) ?? 0} 处`);
  const assets = isRow(stats.assets) ? stats.assets : {};
  if (text(assets.state) !== "reported") lines.push("资产数：未报告");
  else if (Array.isArray(assets.perUnit) && assets.perUnit.length) {
    lines.push(`资产按单位（归属模块逐资产投影）：${assets.perUnit.filter(isRow).map((row) => `${text(isRow(row.unit) ? row.unit.legalName : "")} ${num(row.assets) ?? 0}`).join("，")}；未分配单位 ${num(assets.unassigned) ?? 0}`);
  }
  return lines;
}

export interface IncidentListItem {
  id: number;
  title: string;
  status: string;
  revision: number;
  line: string;
  reviewNeeded: boolean;
}

export function incidentLine(item: unknown): IncidentListItem | null {
  if (!isRow(item) || num(item.id) === null || num(item.revision) === null) return null;
  const units = isRow(item.units) ? item.units : {};
  const unitPart = units.unknown === true
    ? "单位未知"
    : `单位 确认 ${num(units.confirmed) ?? 0} / 候选 ${num(units.candidate) ?? 0}${(num(units.conflict) ?? 0) > 0 ? ` / 冲突 ${units.conflict}` : ""}`;
  const status = text(item.status);
  return {
    id: num(item.id) as number,
    title: text(item.title),
    status,
    revision: num(item.revision) as number,
    reviewNeeded: item.reviewNeeded === true,
    line: [
      INCIDENT_STATUS_LABELS[status] ?? status,
      `${num(item.activeLocations) ?? 0} 个位置`,
      BASIS_LABELS[text(item.basis)] ?? text(item.basis),
      EVIDENCE_GRADE_LABELS[text(item.evidenceGrade)] ?? text(item.evidenceGrade),
      unitPart,
      HANDLING_LABELS[text(item.handlingState)] ?? text(item.handlingState),
    ].filter(Boolean).join(" · "),
  };
}

export interface ActionForm {
  action: "review" | "record_action" | "link_entity" | "merge" | "split";
  /** merge: the incident this one is merged into, with the revision the page read for it. */
  targetIncidentId?: number;
  targetExpectedRevision?: number;
  /** split: the active locations that move to a new incident. */
  locationIds?: number[];
  status?: string;
  handling?: string;
  channel?: string;
  externalReference?: string;
  retestRunId?: number;
  entityId?: number;
  linkLevel?: string;
  evidenceObservationIds?: number[];
  actor: string;
  reason: string;
}

/// Builds the payload `act_on_exposure_incident` expects, or explains why it cannot.
export function buildIncidentAction(
  incident: { id: number; revision: number },
  form: ActionForm,
  operationId: string,
): { ok: true; payload: JsonLike } | { ok: false; error: string } {
  const reason = form.reason.trim();
  const actor = form.actor.trim();
  if (!/^[A-Za-z0-9_-]{8,80}$/.test(operationId)) return { ok: false, error: "操作编号无效" };
  if (!actor) return { ok: false, error: "请填写操作人" };
  if (reason.length < 4) return { ok: false, error: "每个动作都要写明理由（至少 4 个字）" };
  const base: Record<string, unknown> = {
    operationId,
    action: form.action,
    incidentId: incident.id,
    expectedRevision: incident.revision,
    actor,
    reason,
  };
  if (form.action === "review") {
    if (!form.status || form.status === "merged" || !INCIDENT_STATUS_LABELS[form.status]) return { ok: false, error: "请选择复核结论" };
    base.status = form.status;
  } else if (form.action === "record_action") {
    const spec = HANDLING_ACTIONS.find((row) => row.value === form.handling);
    if (!spec) return { ok: false, error: "请选择要登记的处置" };
    if (spec.needsChannel && !(form.channel ?? "").trim()) return { ok: false, error: "登记通知要写明通知了谁、通过什么渠道（应用不会代为联系）" };
    if (spec.value === "retest_linked" && !Number.isInteger(form.retestRunId)) return { ok: false, error: "关联复测需要轮次号" };
    base.handling = spec.value;
    if ((form.channel ?? "").trim()) base.channel = (form.channel ?? "").trim();
    if ((form.externalReference ?? "").trim()) base.externalReference = (form.externalReference ?? "").trim();
    if (spec.value === "retest_linked") base.retestRunId = form.retestRunId;
  } else if (form.action === "merge") {
    if (!Number.isInteger(form.targetIncidentId) || (form.targetIncidentId as number) <= 0) return { ok: false, error: "请填写要并入的目标事件编号" };
    if (form.targetIncidentId === incident.id) return { ok: false, error: "不能把事件合并到它自己" };
    if (!Number.isInteger(form.targetExpectedRevision)) return { ok: false, error: "目标事件的版本未读取，请先读取目标事件" };
    base.targetIncidentId = form.targetIncidentId;
    base.targetExpectedRevision = form.targetExpectedRevision;
  } else if (form.action === "split") {
    const ids = [...new Set(form.locationIds ?? [])];
    if (!ids.length || ids.length > 500 || !ids.every((id) => Number.isInteger(id) && id > 0)) return { ok: false, error: "请勾选要拆出的位置（1~500 个）" };
    base.locationIds = ids;
  } else {
    if (!Number.isInteger(form.entityId) || !form.linkLevel || !LINK_LEVEL_LABELS[form.linkLevel]) return { ok: false, error: "请选择组织目录实体与关联级别" };
    if (form.linkLevel === "confirmed_link" && !(form.evidenceObservationIds ?? []).length) return { ok: false, error: "确认关联需要本事件里的观察作为证据" };
    base.entityId = form.entityId;
    base.linkLevel = form.linkLevel;
    base.evidenceObservationIds = form.evidenceObservationIds ?? [];
  }
  return { ok: true, payload: base as JsonLike };
}

/// A fresh operation id per submit; a retried submit reuses the id it already sent.
export function newOperationId(random: () => string = () => globalThis.crypto.randomUUID()): string {
  return `ui-${random().replace(/[^A-Za-z0-9-]/g, "").slice(0, 60)}`;
}

// ───────────── P1 覆盖矩阵钻取：时间窗口 × 单位 × 工作项明细 ─────────────

export const COVERAGE_WINDOWS: { days: number; label: string }[] = [
  { days: 0, label: "最近一轮" },
  { days: 7, label: "最近 7 天" },
  { days: 30, label: "最近 30 天" },
  { days: 90, label: "最近 90 天" },
];

const WORK_ITEM_STATUS: Record<string, string> = {
  completed: "已完成", failed: "失败", orphaned: "失联", cancelled: "已取消", skipped_unconfigured: "未配置跳过",
  blocked_quota: "配额阻塞", blocked_authorization: "授权阻塞", pending: "待执行", claimed: "执行中",
};

export interface CoverageDrillView {
  windowLabel: string;
  rounds: string[];
  unitState: "reported" | "not_reported" | "not_asset_scoped" | "unknown";
  unitNote: string;
  units: { entityId: number; line: string; gaps: number }[];
  items: { id: number; line: string; gap: boolean }[];
  total: number;
  hasMore: boolean;
  nextCursor: string;
}

/// Reads one drill-down page. Anything malformed is an error, never an empty (= "no gaps") view.
export function coverageDrillView(value: unknown): CoverageDrillView {
  if (!isRow(value) || !Array.isArray(value.items) || typeof value.total !== "number" || typeof value.hasMore !== "boolean") {
    throw new Error("覆盖钻取返回结构无效");
  }
  const next = text(value.nextCursor);
  if (value.hasMore && !next) throw new Error("覆盖钻取声称还有下一页却没有游标");
  const window = isRow(value.window) ? value.window : {};
  const rounds = (Array.isArray(value.rounds) ? value.rounds : []).filter(isRow).map((round) => {
    const counts = isRow(round.itemCounts) ? round.itemCounts : {};
    const parts = Object.entries(counts).map(([status, count]) => `${WORK_ITEM_STATUS[status] ?? status} ${count}`);
    const source = text(round.sourceResult);
    return `第 ${num(round.runId) ?? "?"} 轮（${COVERAGE_RUN_STATUS[text(round.runStatus)] ?? text(round.runStatus)}）${text(round.createdAt)} → ${text(round.completedAt) || "未结束"}：${parts.join(" · ") || "无工作项"}${source ? ` · 来源结果 ${source}` : ""}`;
  });
  const axis = isRow(value.unitAxis) ? value.unitAxis : {};
  const unitState = (["reported", "not_reported", "not_asset_scoped"].includes(text(axis.state)) ? text(axis.state) : "unknown") as CoverageDrillView["unitState"];
  const units = unitState === "reported" && Array.isArray(axis.rows)
    ? axis.rows.filter(isRow).map((row) => ({
        entityId: num(row.entityId) ?? 0,
        gaps: num(row.gaps) ?? 0,
        line: `${text(row.legalName)}：${num(row.completed) ?? 0} / ${num(row.total) ?? 0} 完成，缺口 ${num(row.gaps) ?? 0}`,
      }))
    : [];
  const unitNote = unitState === "reported"
    ? "单位来自归属模块逐资产投影；未归属的资产单列，不按名称或地区推断"
    : unitState === "not_asset_scoped"
      ? text(axis.label) || "这个来源不按资产计划，不按单位分摊"
      : `单位维度未报告${text(axis.reason) ? `：${text(axis.reason)}` : ""}（不是 0）`;
  return {
    windowLabel: text(window.label) || "最近一轮",
    rounds,
    unitState,
    unitNote,
    units,
    items: value.items.filter(isRow).map((row) => {
      const status = text(row.status);
      const target = text(row.origin) || text(row.entity) || `资产 ${num(row.assetId) ?? "—"}`;
      return {
        id: num(row.id) ?? 0,
        gap: status !== "completed",
        line: `#${num(row.id)} 第 ${num(row.runId)} 轮 · ${target} · ${WORK_ITEM_STATUS[status] ?? status}${text(row.reason) ? ` · ${text(row.reason)}` : ""} · 请求 ${num(row.requests) ?? 0}`,
      };
    }),
    total: value.total,
    hasMore: value.hasMore,
    nextCursor: next,
  };
}
