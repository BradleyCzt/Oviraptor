import { invoke } from "@tauri-apps/api/core";
import type { AssetOwnershipProfile } from "../../types";
import type { AssetOwnershipDecisionExpectation } from "./api";

// O1 项目预填闭环：共享组织目录（只记录组织事实，不携带授权）+ 只读候选建议 + 项目绑定（CAS）。
export interface OrgIdentifierView { id: number; kind: string; value: string; source: string; sourceRef: string }
export interface OrgEntityView {
  id: number; groupId: number | null; parentId: number | null; entityKind: string; legalName: string;
  regionCode: string; unifiedCode: string; source: string; sourceRef: string; directoryVersion: number;
  identifiers: OrgIdentifierView[];
}
export interface ProjectOrgBindingView { projectKind: "group_ledger" | "unit"; groupEntityId: number; unitEntityId: number | null; bindingVersion: number; updatedAt: string }
export interface ProjectOwnershipBootstrap {
  projectId: number; projectName: string; profile: AssetOwnershipProfile; profileSaved: boolean;
  profilePrefillSources: Record<string, string>; binding: ProjectOrgBindingView | null; directoryVersion: number;
  entities: OrgEntityView[]; entitiesTruncated: boolean; scopeEntityIds: number[];
  authorization: { allowed: number; exposureEligible: number }; dataRevision: string; readiness: string[];
}
export interface OwnershipSuggestion {
  key: string; field: "group" | "member_entity" | "alias" | "official_domain" | "legal_name" | "network_clue" | "domain_clue"; value: string;
  entityId: number | null; entityKind: string; parentEntityId: number | null; parentHint: string; regionClue: string; locationClue: string;
  source: string; supportRows: number; conflictRows: number; examplesRef: number[]; reason: string;
  directoryVersion: number; policyVersion: number; dataRevision: string;
  state: "saved" | "directory_confirmed" | "pending" | "conflict" | "unknown";
}
export interface OwnershipSuggestionPreview {
  projectId: number; directoryVersion: number; policyVersion: number; dataRevision: string; scopeRows: number;
  unlabeledRows: number; labelCount: number; truncatedLabels: number; suggestions: OwnershipSuggestion[]; readiness: string[];
}
export interface OrgEntityRef { entityId?: number; draftKey?: string }
export interface OrgEntityDraft { draftKey: string; legalName: string; entityKind: string; parent?: OrgEntityRef; regionCode?: string; unifiedCode?: string; source?: string; sourceRef?: string }
export interface OrgIdentifierDraft { entity: OrgEntityRef; kind: string; value: string; source?: string; sourceRef?: string }
export interface ProjectOrgBindingDraft { projectKind: "group_ledger" | "unit"; group: OrgEntityRef; unit?: OrgEntityRef }
export interface ProjectOwnershipDirectorySave {
  projectId: number; expectedDirectoryVersion: number; expectedBindingVersion: number;
  entities: OrgEntityDraft[]; identifiers: OrgIdentifierDraft[]; binding?: ProjectOrgBindingDraft;
}
export interface ProjectOwnershipDirectorySaveResult { projectId: number; directoryVersion: number; bindingVersion: number; createdEntities: { draftKey: string; entityId: number }[]; createdIdentifiers: number }

export const ownershipDirectoryApi = {
  getProjectOwnershipBootstrap: (projectId: number) =>
    invoke<ProjectOwnershipBootstrap>("get_project_ownership_bootstrap", { projectId }),
  previewProjectOwnershipSuggestions: (projectId: number) =>
    invoke<OwnershipSuggestionPreview>("preview_project_ownership_suggestions", { projectId }),
  saveProjectOwnershipDirectory: (input: ProjectOwnershipDirectorySave) =>
    invoke<ProjectOwnershipDirectorySaveResult>("save_project_ownership_directory", { input }),
};

export const ORG_ENTITY_KIND_LABELS: Record<string, string> = {
  group: "集团", headquarters: "总部", province_company: "省公司", city_branch: "市分支", county_branch: "区县分支",
  professional_subsidiary: "专业子公司", overseas_entity: "境外单位", supplier: "关联供应商", unknown: "类型未知",
};
export const SUGGESTION_STATE_LABELS: Record<string, string> = {
  saved: "已保存", directory_confirmed: "已确认目录", pending: "待接受建议", conflict: "冲突", unknown: "未知",
};

// O2 后台聚合复核：分析任务、决策组/成员 cursor 分页、组批量预览与确认（提交 groupId，不传资产 ID 列表）。
export interface OwnershipAnalysisVersions { policyVersion: number; directoryVersion: number; bindingVersion: number; scopeVersion: string; algorithmVersion: string; rulesVersion?: string }
export interface OwnershipAnalysisRun {
  id: number; projectId: number; status: "queued" | "running" | "completed" | "cancelled" | "failed"; versions: OwnershipAnalysisVersions;
  dataRevision: string; dataRevisionAtFinish: string; totalAssets: number; processedAssets: number; groupCount: number; memberCount: number;
  cancelRequested: boolean; membersPruned: boolean; error: string; createdAt: string; startedAt: string; finishedAt: string;
  // O3：full / incremental（基于 baseRunId 的数据水位增量）；modeReason 说明为何全量；resumeCount 为崩溃/重启后接管次数。
  runKind?: "full" | "incremental"; baseRunId?: number | null; modeReason?: string; reclassifiedAssets?: number; carriedAssets?: number; resumeCount?: number; watermarkAt?: string;
}
export interface OwnershipReviewQueueCounts {
  pendingAssets: number; pendingGroups: number; conflictGroups: number; counterEvidenceGroups: number; completedGroups: number; manualAssets: number;
  newSinceSnapshot: number; changedSinceSnapshot: number; memberTotal: number; groupMemberSum: number; groupTotal: number;
}
export interface OwnershipAnalysisOverview {
  projectId: number; activeRun: OwnershipAnalysisRun | null; latestRun: OwnershipAnalysisRun | null; snapshot: OwnershipAnalysisRun | null;
  current: OwnershipAnalysisVersions; currentDataRevision: string; staleReasons: string[]; queue: OwnershipReviewQueueCounts | null; readiness: string[];
}
export interface OwnershipGroupMember {
  assetId: number; host: string; registrableDomain: string; suffixSection: string; company: string; link: string; title: string;
  current: { linked: boolean; inScope: boolean; status: string; source: string; authorizationStatus: string; exposureEligible: boolean; reason: string };
  changedSinceSnapshot: boolean;
}
export type OwnershipGroupAction = "confirm_ownership" | "related" | "third_party" | "excluded";
export interface OwnershipReviewGroup {
  id: number; runId: number; lane: string; navBucket: string; candidateEntityId: number | null; candidateLabel: string; candidateSource: string;
  relationKind: string; regionClue: string; boundaryKind: string; boundaryValue: string; supportKind: string; counterKind: string;
  decisionStatus: string; decisionSource: string; authorizationStatus: string; exposureEligible: boolean; memberCount: number; distinctHosts: number;
  samples: OwnershipGroupMember[]; sampleRule: string; reason: string; priority: number; sortRank: number; groupVersion: number; appliedOperations: number;
  allowedActions: OwnershipGroupAction[];
  authorizationRevoked?: number;
}
export interface OwnershipGroupPage { projectId: number; snapshot: OwnershipAnalysisRun; staleReasons: string[]; total: number; items: OwnershipReviewGroup[]; nextCursor: string }
export interface OwnershipMemberPage { projectId: number; runId: number; groupId: number; groupVersion: number; total: number; items: OwnershipGroupMember[]; nextCursor: string }
export interface OwnershipGroupDecisionRequest { projectId: number; runId: number; groupId: number; action: OwnershipGroupAction; note: string }
export interface OwnershipGroupDecisionPreview {
  projectId: number; runId: number; groupId: number; groupVersion: number; policyVersion: number; directoryVersion: number; bindingVersion: number;
  action: OwnershipGroupAction; note: string; expectedActionHash: string; memberCount: number; applicable: number; skipped: number; conflicted: number;
  reasons: Record<string, number>; excludedSamples: { assetId: number; outcome: string; reason: string }[]; currentAuthorization: Record<string, number>;
  proposed: { status: string; source: string; authorization: string; exposureEligible: string; grantsAuthorization: boolean; setsExposureEligible: boolean };
  notes: string[];
}
export interface OwnershipGroupDecisionApply {
  operationId: string; projectId: number; runId: number; groupId: number; groupVersion: number; policyVersion: number; directoryVersion: number;
  bindingVersion: number; action: OwnershipGroupAction; note: string; expectedActionHash: string;
}
export interface OwnershipGroupDecisionResult {
  operationId: string; projectId: number; runId: number; groupId: number; action: OwnershipGroupAction;
  applied: number; skipped: number; conflicted: number; failed: number; reasons: Record<string, number>; groupVersion: number; replayed: boolean;
  // O3：applying = 大组分块写入中（计数为已提交分块的真实进度，后台继续，可续跑）；completed = 全部完成。
  status?: "applying" | "completed" | "failed"; totalItems?: number; doneItems?: number;
}
export interface OwnershipOperationView {
  operationId: string; projectId: number; runId: number; groupId: number; kind: "group_decision" | "group_revert" | "authorization_revoke"; action: string;
  status: "applying" | "blocked" | "completed" | "failed"; targetOperationId: string; totalItems: number; doneItems: number;
  applied: number; skipped: number; conflicted: number; failed: number; reasons: Record<string, number>; resumeCount: number; error: string;
  createdAt: string; updatedAt: string; ownedByLiveWorker: boolean;
}
export interface OwnershipRevertPreview {
  projectId: number; targetOperationId: string; targetAction: string; runId: number; groupId: number; note: string; appliedInTarget: number;
  revertible: number; conflicted: number; reasons: Record<string, number>; authorizationNotRestored: number; expectedRevertHash: string; notes: string[];
}
export interface OwnershipRevertApply { operationId: string; projectId: number; targetOperationId: string; note: string; expectedRevertHash: string }
export interface OwnershipRevokeRequest { operationId: string; projectId: number; runId: number; groupId: number; note: string }
export interface OwnershipRevokeResult {
  operationId: string; projectId: number; runId: number; groupId: number; status: "blocked" | "completed"; revoked: number; alreadyRevoked: number;
  withoutDecision: number; groupVersion: number; replayed: boolean;
}

export const ownershipReviewApi = {
  startOwnershipAnalysis: (projectId: number, mode: "auto" | "full" = "auto") => invoke<OwnershipAnalysisRun>("start_ownership_analysis", { projectId, mode }),
  readOwnershipAnalysis: (projectId: number) => invoke<OwnershipAnalysisOverview>("read_ownership_analysis", { projectId }),
  cancelOwnershipAnalysis: (projectId: number, runId: number) => invoke<OwnershipAnalysisRun>("cancel_ownership_analysis", { projectId, runId }),
  pageOwnershipGroups: (input: { projectId: number; runId: number; lane?: string; navBucket?: string; region?: string; pageSize: number; cursor: string }) =>
    invoke<OwnershipGroupPage>("page_ownership_groups", { input }),
  pageOwnershipGroupMembers: (input: { projectId: number; runId: number; groupId: number; pageSize: number; cursor: string }) =>
    invoke<OwnershipMemberPage>("page_ownership_group_members", { input }),
  previewOwnershipGroupDecision: (input: OwnershipGroupDecisionRequest) =>
    invoke<OwnershipGroupDecisionPreview>("preview_ownership_group_decision", { input }),
  applyOwnershipGroupDecision: (input: OwnershipGroupDecisionApply) =>
    invoke<OwnershipGroupDecisionResult>("apply_ownership_group_decision", { input }),
  readOwnershipOperation: (projectId: number, operationId: string) =>
    invoke<OwnershipOperationView>("read_ownership_operation", { projectId, operationId }),
  previewOwnershipGroupRevert: (input: { projectId: number; targetOperationId: string; note: string }) =>
    invoke<OwnershipRevertPreview>("preview_ownership_group_revert", { input }),
  applyOwnershipGroupRevert: (input: OwnershipRevertApply) =>
    invoke<OwnershipGroupDecisionResult>("apply_ownership_group_revert", { input }),
  revokeOwnershipGroupAuthorization: (input: OwnershipRevokeRequest) =>
    invoke<OwnershipRevokeResult>("revoke_ownership_group_authorization", { input }),
};

// O3 拆分逐条合同：确认归属（只写归属事实，不创建执行授权）与授权（走既有受控授权合同 update_asset_ownership 的 confirmed 分支）。
export type OwnershipRuleScope = "none" | "exact_host" | "subdomains";
export interface OwnershipSelection { projectId: number; assetId: number }
export interface OwnershipConfirmResult { changed: number; expectations: AssetOwnershipDecisionExpectation[]; ruleScope: OwnershipRuleScope; learnedRules: string[] }
export const ownershipAuthorizationApi = {
  confirmAssetOwnership: (input: { selections: OwnershipSelection[]; note: string; domainRuleScope: OwnershipRuleScope }, expectedProjectId: number, expectedDecisions: AssetOwnershipDecisionExpectation[]) =>
    invoke<OwnershipConfirmResult>("confirm_asset_ownership", { input, expectedProjectId, expectedDecisions }),
  authorizeAssetOwnership: (input: { selections: OwnershipSelection[]; note: string }, expectedProjectId: number, expectedDecisions: AssetOwnershipDecisionExpectation[]) =>
    invoke<number>("authorize_asset_ownership", { input, expectedProjectId, expectedDecisions }),
};

export const OWNERSHIP_LANE_LABELS: Record<string, string> = {
  conflict: "证据冲突", counter_evidence: "存在反证", consistent: "规则一致", insufficient: "证据不足", unknown: "未知", manual: "已有人工结论",
};
export const OWNERSHIP_NAV_LABELS: Record<string, string> = {
  headquarters_shared: "总部/共享", province: "省公司", city: "市分支", subsidiary: "专业子公司", overseas: "境外", supplier: "关联供应商", conflict: "冲突", unknown: "未知",
};
