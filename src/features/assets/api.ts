import { invoke } from "@tauri-apps/api/core";
import type { AssetEvent, AssetPage, AssetQuery, AssetSelection, AssetOwnershipProfile, AssetOwnershipSummary, AssetOwnershipAssessmentResult, ExposureFinding, ExposureSummary, ExposureRun, ExposureSourceResult, ContentRuleApplyResult, JobRun, LogEntry, Target } from "./types";

/** O0：页面加载该行时看到的归属结论（与 list_assets 返回的 ownership* 字段一一对应）。 */
export interface AssetOwnershipDecisionExpectation {
  assetId: number;
  status: string;
  source: string;
  confidence: number;
  authorizationStatus: string;
  exposureEligible: boolean;
  reason: string;
}

export const assetApi = {
  importTargets: (projectId: number, targetType: string, values: string[]) =>
    invoke<number>("import_targets", { input: { projectId, targetType, values } }),
  listTargets: (projectId: number) => invoke<Target[]>("list_targets", { projectId }),
  removeTarget: (targetId: number) => invoke<void>("remove_target", { targetId }),
  listAssets: (query: AssetQuery) => invoke<AssetPage>("list_assets", { query }),
  markAssetEntryOpened: (projectId: number, assetId: number) =>
    invoke<string>("mark_asset_entry_opened", { projectId, assetId }),
  addContentRule: (keyword: string, sourceAssetId?: number) =>
    invoke<ContentRuleApplyResult>("add_content_rule", { input: { keyword, sourceAssetId } }),
  updateDecision: (projectId: number, assetIds: number[], decision: string, note = "") =>
    invoke<void>("update_decision", { input: { projectId, assetIds, decision, note } }),
  updateAssetDecisions: (selections: AssetSelection[], decision: string, note = "") =>
    invoke<number>("update_asset_decisions", { input: { selections, decision, note } }),
  getAssetOwnershipProfile: (projectId: number) =>
    invoke<AssetOwnershipProfile>("get_asset_ownership_profile", { projectId }),
  // O0：expectedVersion 为页面加载档案时的 policyVersion；尚未保存过档案（updatedAt 为空）时传 0。
  saveAssetOwnershipProfile: (input: AssetOwnershipProfile, expectedVersion: number) =>
    invoke<AssetOwnershipProfile>("save_asset_ownership_profile", { input, expectedVersion }),
  getAssetOwnershipSummary: (projectId: number) =>
    invoke<AssetOwnershipSummary>("get_asset_ownership_summary", { projectId }),
  assessAssetOwnership: (projectId: number) =>
    invoke<AssetOwnershipAssessmentResult>("assess_asset_ownership", { projectId }),
  // O0：必须带页面当前加载的项目与每条资产在页面上看到的结论；任一前置条件不满足时后端整批拒绝。
  updateAssetOwnership: (selections: AssetSelection[], status: string, note: string, learnDomainRule: boolean, expectedProjectId: number, expectedDecisions: AssetOwnershipDecisionExpectation[]) =>
    invoke<number>("update_asset_ownership", { input: { selections, status, note, learnDomainRule }, expectedProjectId, expectedDecisions }),
  // §Q3/§Q4/§Q5 — the durable round: resume by frozen plan, review the pending index queue,
  // and read the redacted evidence package behind one finding.
  resumeExposureScan: (runId: number, expectedProjectId: number) =>
    invoke<number>("resume_exposure_scan", { runId, expectedProjectId }),
  pageExposurePendingDiscoveries: (projectId: number, status: string, pageSize: number, cursor: string) =>
    invoke<unknown>("page_exposure_pending_discoveries", { projectId, status, pageSize, cursor }),
  readExposureEvidence: (findingId: number, expectedProjectId: number) =>
    invoke<unknown>("read_exposure_evidence", { findingId, expectedProjectId }),
  exposureSummary: (projectId: number) => invoke<ExposureSummary>("exposure_summary", { projectId }),
  listExposureFindings: (projectId: number, status = "", search = "") => invoke<ExposureFinding[]>("list_exposure_findings", { projectId, status, search }),
  saveExposureFinding: (input: { projectId:number; assetId?:number; category:string; title:string; sourceType:string; sourceUrl:string; evidenceExcerpt:string; severity:string; confidence:number }) => invoke<number>("save_exposure_finding", { input }),
  startExposureScan: (projectId:number) => invoke<number>("start_exposure_scan", { projectId }),
  listExposureRuns: (projectId:number) => invoke<ExposureRun[]>("list_exposure_runs", { projectId }),
  listExposureSourceResults: (runId:number, expectedProjectId:number) => invoke<ExposureSourceResult[]>("list_exposure_source_results", { runId, expectedProjectId }),
  cancelExposureScan: (projectId:number, runId:number) => invoke<void>("cancel_exposure_scan", { projectId, runId }),
  exportExposureFindings: (projectId:number) => invoke<{path:string;rows:number}>("export_exposure_findings", { projectId }),
  reclassifyExposureFinding: (projectId:number, request: Record<string, unknown>) => invoke<unknown>("reclassify_exposure_finding", { projectId, request }),
  // §Q1 只读分页接口：错误码在消息里，旧调用与旧返回形状保持不变。
  exposureScopeSummary: (projectId:number) => invoke<unknown>("exposure_scope_summary", { projectId }),
  pageExposureFindings: (input: { projectId:number; status:string; search:string; pageSize:number; cursor:string }) =>
    invoke<unknown>("page_exposure_findings", { input }),
  pageExposureRuns: (projectId:number, pageSize:number, cursor:string) =>
    invoke<unknown>("page_exposure_runs", { projectId, pageSize, cursor }),
  listExposureSourceResultsForProject: (runId:number, expectedProjectId:number) =>
    invoke<unknown>("list_exposure_source_results_for_project", { runId, expectedProjectId }),
  readExposureFinding: (findingId:number, expectedProjectId:number) =>
    invoke<unknown>("read_exposure_finding", { findingId, expectedProjectId }),
  exposureKnownLimits: () => invoke<unknown>("exposure_known_limits"),
  softDeleteAssets: (projectId: number, assetIds: number[], deleted: boolean) =>
    invoke<void>("soft_delete_assets", { projectId, assetIds, deleted }),
  softDeleteAssetSelections: (selections: AssetSelection[], deleted: boolean) =>
    invoke<number>("soft_delete_asset_selections", { input: { selections, deleted } }),
  listRuns: (projectId?: number, limit = 100) =>
    invoke<JobRun[]>("list_runs", { projectId, limit }),
  listLogs: (runId?: number, limit = 500, projectId?: number) =>
    invoke<LogEntry[]>("list_logs", { runId, projectId, limit }),
  listEvents: (projectId?: number, eventType?: string, limit = 500) =>
    invoke<AssetEvent[]>("list_asset_events", { projectId, eventType, limit }),
  startJob: (projectId: number, profileId: number, name: string, pipeline: string) =>
    invoke<number>("start_job", { input: { projectId, profileId, name, pipeline } }),
  resumeJob: (runId: number) => invoke<number>("resume_job", { runId }),
  cancelJob: (runId: number) => invoke<void>("cancel_job", { runId }),
  exportAssets: (query: AssetQuery, fields: string[], chineseHeaders = true) =>
    invoke<{ path: string; rows: number }>("export_assets", {
      request: { query, fields, chineseHeaders },
    }),
};
