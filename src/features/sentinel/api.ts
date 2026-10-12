import { invoke } from "@tauri-apps/api/core";
import { rootCostReconciliationApi } from "./reconciliation/api";
import type { NativeSdkLogPage } from "./execution/nativeSdkLogContract";
import type { NativeProcessLogPage } from "./execution/nativeProcessLogContract";
import type {
  HumanDirectiveReviewResult,
  AgentDialogView,
  AgentDialogViewInput,
  AgentDialogSelection,
  AdministrativeClosureReceipt,
  AdministrativeClosurePreview,
  ClosureHandoffPreview,
  ClosureHandoffInput,
  ClosureHandoffReceipt,
  GapFollowupPreview,
  GapFollowupDraftInput,
  GapFollowupSubmission,
  AppSecScanResult,
  BrowserAuthSession,
  ScanIdentityMaterial,
  InvestigationGraph,
  InvestigationHypothesis,
  InvestigationValidation,
  InvestigationOverview,
  FofaApiTestResult,
  SecurityRulePack,
  AgentTargetExecution,
  AgentRequestReview,
  AgentRequestReviewPage,
  AgentRequestReviewInput,
  AgentTimelineItem,
  AgentTimelinePage,
  HumanDirectiveDraft,
  NativeScanStatus,
  NativeBudgetDiagnostics,
  NativeSourceFindingsPage,
  AgentAttemptMailboxPage,
  AgentAttemptToolPage,
  PostScanDirectiveResult,
  SentinelCheckpoint,
  SentinelFinding,
  HistoricalImportPreview,
  HistoricalImportRun,
  SentinelFuseEntry,
  SentinelOpportunity,
  SentinelOverviewStats,
  SentinelRunnerLogView,
  SentinelScan,
  SentinelScanAttempt,
  SentinelTarget,
  SentinelValidation,
  SentinelValidationWorkItem,
  AgentKnowledgeEntry,
  AgentLearningCandidate,
  ModelProfileTestResult,
  AgentSkill,
  AgentTraceDetail,
  AgentTraceSummary,
  WorkbenchScanInput,
  CapabilityBundle,
  RoleConfigDraft,
} from "../../types";

export const sentinelApi = {
  ...rootCostReconciliationApi,
  recoverNativeSourcePauseResult: (scanId: string, attemptNumber: number) =>
    invoke<{ schemaVersion: 1; scanId: string; attemptNumber: number; rootRunId: string; scanStatus: "paused"; branchStatus: "partial"; changed: boolean; executionReplayed: false }>(
      "recover_native_source_pause_result", { scanId, attemptNumber }),
  exportNativeSourceFindings: (scanId: string, attemptNumber: number, format?: "json" | "sarif" | "bundle") =>
    invoke<string>("export_native_source_findings", { scanId, attemptNumber, ...(format ? { format } : {}) }),
  getNativeSourceFindings: (scanId: string, attemptNumber: number, offset = 0, limit = 50) =>
    invoke<NativeSourceFindingsPage>("get_native_source_findings", { scanId, attemptNumber, offset, limit }),
  getAgentRequestReviews: (scanId: string, attemptNumber: number, targetUrl: string) =>
    invoke<AgentRequestReviewPage>("get_agent_request_reviews", { scanId, attemptNumber, targetUrl }),
  recordAgentRequestReview: (input: AgentRequestReviewInput) =>
    invoke<AgentRequestReview>("record_agent_request_review", { input }),
  getAuthorizationControlSetup: (scanId: string, targetUrl: string) =>
    invoke<{ attemptNumber: number; identityIds: string[]; contractKeys: string[] }>("get_authorization_control_setup", { scanId, targetUrl }),
  saveAuthorizationControl: (input: {
    scanId: string; attemptNumber: number; targetUrl: string; contractKey: string;
    ownerObjectUrl: string; testerControlUrl: string; objectQueryKey: string;
    ownerObjectValue: string; testerObjectValue: string; responseObjectPointer: string;
    ownerIdentity: string; testerIdentity: string;
  }) => invoke<void>("save_authorization_control", { input }),
  getNativeScanStatus: (scanId: string, afterSequence?: number, expectedAttemptNumber?: number) =>
    invoke<NativeScanStatus>("get_native_scan_status", { scanId, afterSequence, expectedAttemptNumber }),
  getNativeBudgetDiagnostics: (scanId: string, attemptNumber: number) =>
    invoke<NativeBudgetDiagnostics>("get_native_budget_diagnostics", { scanId, attemptNumber }),
  getNativeScanTimelinePage: (scanId: string, attemptNumber: number, beforeSequence: number) =>
    invoke<AgentTimelinePage>("get_native_scan_timeline_page", { scanId, attemptNumber, beforeSequence }),
  recoverNeverDispatchedWebAttempt: (scanId: string, attemptNumber: number) =>
    invoke<{ scanId: string; attemptNumber: number; dispatchState: "claimed"; executionState: "submitted"; automaticReplayAllowed: false }>(
      "recover_never_dispatched_web_attempt", { scanId, attemptNumber, operatorConfirmed: true }),
  closeNeverDispatchedWebAttempt: (scanId: string, attemptNumber: number) =>
    invoke<{ scanId: string; attemptNumber: number; closureId: string; closedAt: string; executionState: "closed_without_dispatch"; automaticReplayAllowed: false }>(
      "close_never_dispatched_web_attempt", { scanId, attemptNumber, operatorConfirmed: true }),
  previewWebAdministrativeClosure: (scanId: string, attemptNumber: number) =>
    invoke<AdministrativeClosurePreview>("preview_web_administrative_closure", { scanId, attemptNumber }),
  previewWebClosureHandoff: (scanId: string) =>
    invoke<ClosureHandoffPreview>("preview_web_closure_handoff", { scanId }),
  createWebClosureHandoff: (input: ClosureHandoffInput) =>
    invoke<ClosureHandoffReceipt>("create_web_closure_handoff", { input }),
  closeWebTaskAdministratively: (scanId: string, attemptNumber: number, operationId: string, snapshotHash: string) =>
    invoke<AdministrativeClosureReceipt>("close_web_task_administratively", { scanId, attemptNumber, operationId, snapshotHash, operatorConfirmed: true }),
  getAgentDialogView: (scanId: string, attemptNumber: number) =>
    invoke<AgentDialogView>("get_agent_dialog_view", { scanId, attemptNumber }),
  saveAgentDialogView: (input: AgentDialogViewInput) =>
    invoke<AgentDialogView>("save_agent_dialog_view", { input }),
  getAgentDialogSelection: (projectId?: number) =>
    invoke<AgentDialogSelection>("get_agent_dialog_selection", { projectId }),
  saveAgentDialogSelection: (input: { projectId?: number; expectedRevision: number; scanId: string }) =>
    invoke<AgentDialogSelection>("save_agent_dialog_selection", { input }),
  getAgentDialogTask: (scanId: string, projectId?: number) =>
    invoke<SentinelScan | null>("get_agent_dialog_task", { scanId, projectId }),
  getNativeAttemptMailboxHistory: (scanId: string, attemptNumber: number, before?: { createdAt: string; id: string }) =>
    invoke<AgentAttemptMailboxPage>("get_native_attempt_mailbox_history", {
      scanId, attemptNumber, beforeCreatedAt: before?.createdAt, beforeId: before?.id, limit: 50,
    }),
  getNativeAttemptToolHistory: (scanId: string, attemptNumber: number, before?: string) =>
    invoke<AgentAttemptToolPage>("get_native_attempt_execution_history", {
      scanId, attemptNumber, before, limit: 50,
    }),
  draftScanDirective: (scanId: string, text: string, threadKey = "team") => invoke<HumanDirectiveDraft>("draft_scan_directive", { scanId, text, threadKey }),
  confirmScanDirective: (scanId: string, draftId: string, revision: number, draftHash: string) =>
    invoke<PostScanDirectiveResult>("confirm_scan_directive", { scanId, draftId, revision, draftHash }),
  reviseScanDirective: (scanId: string, draftId: string, revision: number, draftHash: string, text: string) =>
    invoke<HumanDirectiveReviewResult>("revise_scan_directive", { scanId, draftId, revision, draftHash, text }),
  rejectScanDirective: (scanId: string, draftId: string, revision: number, draftHash: string, reason: string) =>
    invoke<HumanDirectiveReviewResult>("reject_scan_directive", { scanId, draftId, revision, draftHash, reason }),
  reconcileScanDirectiveReceipt: (scanId: string, attemptNumber: number, directiveId: string) =>
    invoke<NonNullable<AgentTimelineItem["localReconciliation"]>>("reconcile_scan_directive_receipt", { scanId, attemptNumber, directiveId }),
  reconcileHistoricalScanDirectiveReceipt: (scanId: string, attemptNumber: number, directiveId: string) =>
    invoke<NonNullable<AgentTimelineItem["localReconciliation"]>>("reconcile_historical_scan_directive_receipt", { scanId, attemptNumber, directiveId }),
  cancelScanDirective: (scanId: string, draftId: string, revision: number, draftHash: string) =>
    invoke<void>("cancel_scan_directive", { scanId, draftId, revision, draftHash }),
  // Compatibility-only API. New UI code must use draft + confirm.
  postScanDirective: (scanId: string, text: string) => invoke<HumanDirectiveDraft>("post_scan_directive", { scanId, text }),
  createSentinelScan: (
    projectId: number,
    assetIds: number[],
    scanMode: "quick" | "standard" | "deep" = "standard",
  ) => invoke<SentinelScan>("create_sentinel_scan", { projectId, assetIds, scanMode }),
  createSentinelUrlScan: (
    projectId: number,
    taskName: string,
    urls: string[],
    scanMode: "quick" | "standard" | "deep" = "standard",
    maxBudgetUsd?: number,
    authSessionId?: string,
    authSessionIds?: string[],
    authSessionScopeId?: string,
    skillIds?: number[],
    instruction?: string,
    closure?: "breadth" | "proof",
    orchestrationMode?: "single" | "multi",
  ) => invoke<SentinelScan>("create_sentinel_url_scan", {
    projectId, taskName, urls, scanMode, maxBudgetUsd, authSessionId, authSessionIds, authSessionScopeId,
    skillIds, instruction, closure, orchestrationMode,
  }),
  createSentinelUrlScanWithToolSelection: (
    projectId: number,
    taskName: string,
    urls: string[],
    scanMode: "quick" | "standard" | "deep" = "standard",
    maxBudgetUsd?: number,
    authSessionId?: string,
    authSessionIds?: string[],
    authSessionScopeId?: string,
    skillIds?: number[],
    instruction?: string,
    closure?: "breadth" | "proof",
    orchestrationMode?: "single" | "multi",
    approvedIds?: string[],
  ) => invoke<SentinelScan>("create_sentinel_url_scan_with_tool_selection", {
    projectId, taskName, urls, scanMode, maxBudgetUsd, authSessionId, authSessionIds, authSessionScopeId,
    skillIds, instruction, closure, orchestrationMode, approvedIds,
  }),
  createSentinelHomogeneousUrlScan: (
    projectId: number, taskName: string, urls: string[], scanMode: "quick" | "standard" | "deep" = "standard",
    maxBudgetUsd?: number, authSessionId?: string, authSessionIds?: string[], authSessionScopeId?: string,
    skillIds?: number[], instruction?: string, closure?: "breadth" | "proof",
    _orchestrationMode?: "single" | "multi", execution = false, approvedIds?: string[],
  ) => invoke<SentinelScan>("create_sentinel_homogeneous_url_scan", {
    projectId, taskName, urls, scanMode, maxBudgetUsd, authSessionId, authSessionIds,
    authSessionScopeId, skillIds, instruction, closure, execution, approvedIds,
  }),
  previewAgentGapFollowup: (scanId: string, assessmentMessageId: string) =>
    invoke<GapFollowupPreview>("preview_agent_gap_followup", { scanId, assessmentMessageId }),
  createAgentGapFollowup: (input: GapFollowupDraftInput) =>
    invoke<SentinelScan>("create_agent_gap_followup", { input }),
  getAgentGapFollowupSubmission: (sourceScanId: string, assessmentMessageId: string) =>
    invoke<GapFollowupSubmission | null>("get_agent_gap_followup_submission", { sourceScanId, assessmentMessageId }),
  releaseAgentGapFollowupSubmission: (requestId: string, expectedScanId?: string) =>
    invoke<void>("release_agent_gap_followup_submission", { requestId, expectedScanId }),
  listBrowserAuthSessions: (projectId: number, draftScopeId: string) =>
    invoke<BrowserAuthSession[]>("list_browser_auth_sessions", { projectId, draftScopeId }),
  listSentinelScanAuthSessions: (scanId: string) =>
    invoke<BrowserAuthSession[]>("list_sentinel_scan_auth_sessions", { scanId }),
  getSentinelScanIdentityMaterial: (projectId: number, scanId: string) =>
    invoke<ScanIdentityMaterial[]>("get_sentinel_scan_identity_material", { projectId, scanId }),
  openBrowserAuthSession: (input: { id?: string; projectId: number; name: string; entryUrl: string; draftScopeId?: string; scanId?: string }) =>
    invoke<BrowserAuthSession>("open_browser_auth_session", { input }),
  finishBrowserAuthSession: (sessionId: string) =>
    invoke<BrowserAuthSession>("finish_browser_auth_session", { sessionId }),
  validateBrowserAuthSession: (sessionId: string) =>
    invoke<BrowserAuthSession>("validate_browser_auth_session", { sessionId }),
  deleteBrowserAuthSession: (sessionId: string) =>
    invoke<void>("delete_browser_auth_session", { sessionId }),
  testModelProfile: (input: {
    llm: string; deployment: "cloud" | "local"; apiBase: string; apiKey: string;
  }) => invoke<ModelProfileTestResult>("test_model_profile", { input }),
  testFofaApi: (input: { key: string; proxyUrl: string }) =>
    invoke<FofaApiTestResult>("test_fofa_api", { input }),
  listAgentSkills: () => invoke<AgentSkill[]>("list_agent_instructions"),
  saveAgentSkill: (input: {
    id?: number; name: string; description: string; instructions: string; enabled: boolean;
  }) => invoke<number>("save_agent_instruction", { input }),
  deleteAgentSkill: (skillId: number) => invoke<void>("delete_agent_instruction", { skillId }),
  exportAgentSkills: () => invoke<string>("export_agent_instructions"),
  importAgentSkills: (path: string) => invoke<number>("import_agent_instructions", { path }),
  importSecSkillKnowledge: (path: string) =>
    invoke<Record<string, unknown>>("import_sec_skill_knowledge", { path }),
  ingestAgentKnowledgeSource: (source: string, forceRefresh = false) =>
    invoke<AgentKnowledgeEntry>("ingest_agent_knowledge_source", { source, forceRefresh }),
  listAgentTraces: () => invoke<AgentTraceSummary[]>("list_agent_traces"),
  getAgentTrace: (scanId: string) => invoke<AgentTraceDetail>("get_agent_trace", { scanId }),
  listAgentKnowledge: (scanId?: string) => invoke<AgentKnowledgeEntry[]>("list_agent_knowledge", { scanId }),
  listAgentLearningCandidates: (status?: string, scanId?: string) =>
    invoke<AgentLearningCandidate[]>("list_agent_learning_candidates", { status, scanId }),
  generateAgentLearningCandidate: (scanId: string) =>
    invoke<AgentLearningCandidate>("generate_agent_learning_candidate", { scanId }),
  reviewAgentLearningCandidate: (candidateId: number, decision: string, targetSkillId?: number) =>
    invoke<AgentLearningCandidate>("review_agent_learning_candidate", { candidateId, decision, targetSkillId }),
  applyAgentLearningCandidate: (candidateId: number) =>
    invoke<number>("apply_agent_learning_candidate", { candidateId }),
  deleteAgentLearningCandidate: (candidateId: number) =>
    invoke<void>("delete_agent_learning_candidate", { candidateId }),
  analyzeAgentTrace: (scanId: string) =>
    invoke<AgentKnowledgeEntry>("analyze_agent_trace", { scanId }),
  aggregateAgentKnowledge: (scanType: string) =>
    invoke<AgentKnowledgeEntry>("aggregate_agent_knowledge", { scanType }),
  deleteAgentKnowledge: (knowledgeId: number) =>
    invoke<void>("delete_agent_knowledge", { knowledgeId }),
  convertAgentKnowledgeToInstruction: (knowledgeId: number) =>
    invoke<number>("convert_agent_knowledge_to_instruction", { knowledgeId }),
  refineAgentSkillWithKnowledge: (skillId: number) =>
    invoke<number>("refine_agent_instruction_with_knowledge", { skillId }),
  exportAgentKnowledge: () => invoke<string>("export_agent_knowledge"),
  importAgentKnowledge: (path: string) => invoke<number>("import_agent_knowledge", { path }),
  listSecurityRulePacks: () => invoke<SecurityRulePack[]>("list_security_rule_packs"),
  saveSecurityRulePack: (input: {
    key: string; name: string; engine: string; repository: string; reference?: string; enabled: boolean;
  }) => invoke<number>("save_security_rule_pack", { input }),
  deleteSecurityRulePack: (packId: number) =>
    invoke<void>("delete_security_rule_pack", { packId }),
  syncSecurityRulePack: (packId: number) =>
    invoke<SecurityRulePack>("sync_security_rule_pack", { packId }),
  startWorkbenchScan: (input: WorkbenchScanInput) =>
    invoke<SentinelScan>("start_workbench_scan", { input }),
  rescanWorkbenchScan: (scanId: string) =>
    invoke<SentinelScan>("rescan_workbench_scan", { scanId }),
  rescanSentinelScan: (scanId: string) => invoke<SentinelScan>("rescan_sentinel_scan", { scanId }),
  confirmSentinelScan: (scanId: string) => invoke<SentinelScan>("confirm_sentinel_scan", { scanId }),
  pauseSentinelScan: (scanId: string) => invoke<SentinelScan>("pause_sentinel_scan", { scanId }),
  resumeSentinelScan: (scanId: string) => invoke<SentinelScan>("resume_sentinel_scan", { scanId }),
  cancelSentinelScan: (scanId: string) => invoke<void>("cancel_sentinel_scan", { scanId }),
  deleteSentinelScan: (scanId: string) => invoke<void>("delete_sentinel_scan", { scanId }),
  listSentinelScans: (projectId?: number, limit = 300, before?: Pick<SentinelScan, "updatedAt" | "id">) =>
    invoke<SentinelScan[]>("list_sentinel_scans", { projectId, limit, beforeUpdatedAt: before?.updatedAt, beforeId: before?.id }),
  searchSentinelScanPage: (projectId: number | undefined, search: string, view: "attention" | "history" | "all", limit = 100, before?: Pick<SentinelScan, "updatedAt" | "id">) =>
    invoke<SentinelScan[]>("search_sentinel_scan_page", { projectId, search, view, limit, beforeUpdatedAt: before?.updatedAt, beforeId: before?.id }),
  archiveSentinelScan: (scanId: string, projectId: number, archive: boolean) =>
    invoke<SentinelScan>("archive_sentinel_scan", { scanId, projectId, archive }),
  listSentinelScanAttempts: (scanId: string) =>
    invoke<SentinelScanAttempt[]>("list_sentinel_scan_attempts", { scanId }),
  listSentinelVulnerabilityScanIds: (projectId?: number) =>
    invoke<string[]>("list_sentinel_vulnerability_scan_ids", { projectId }),
  getSentinelRunnerLog: (scanId: string, limit = 300) =>
    invoke<string[]>("get_sentinel_runner_log", { scanId, limit }),
  readNativeProcessLog: (scanId: string, attempt: number, cursorAttempt?: number, afterSequence = 0, limit = 300) =>
    invoke<NativeProcessLogPage>("read_native_process_log", { scanId, attempt, cursorAttempt, afterSequence, limit }),
  readNativeSdkLog: (scanId: string, attempt: number, cursorAttempt?: number, afterSequence = 0, limit = 300, ownerId?: string) =>
    invoke<NativeSdkLogPage>("read_native_sdk_log", { scanId, attempt, cursorAttempt, ownerId, afterSequence, limit }),
  readSentinelRunnerLog: (scanId: string, attempt?: number, limit = 300) =>
    invoke<SentinelRunnerLogView>("read_sentinel_runner_log", { scanId, attempt, limit }),
  searchSentinelScanIds: (search: string) =>
    invoke<string[]>("search_sentinel_scan_ids", { search }),
  listSentinelTargets: (projectId?: number, limit = 5000) =>
    invoke<SentinelTarget[]>("list_sentinel_targets", { projectId, limit }),
  listSentinelFuseZone: (projectId?: number) =>
    invoke<SentinelFuseEntry[]>("list_sentinel_fuse_zone", { projectId }),
  saveSentinelFuseReview: (input: {
    id: number; verdict: string; note: string; evidence: string; archived: boolean;
  }) => invoke<void>("save_sentinel_fuse_review", { input }),
  removeSentinelFuseEntry: (entryId: number) =>
    invoke<SentinelScan>("remove_sentinel_fuse_entry", { entryId }),
  listSentinelCheckpoints: (scanId: string) =>
    invoke<SentinelCheckpoint[]>("list_sentinel_checkpoints", { scanId }),
  listSentinelFindings: (scanId: string, kind?: string) =>
    invoke<SentinelFinding[]>("list_sentinel_findings", { scanId, kind }),
  listHistoricalImportPreviews: (scanId: string) =>
    invoke<HistoricalImportPreview[]>("list_historical_import_previews", { scanId }),
  listHistoricalImportRuns: (beforeId?: number) =>
    invoke<HistoricalImportRun[]>("list_historical_import_runs", { beforeId }),
  listHistoricalBundlePreviews: (bundleId: string, projectionId: number) =>
    invoke<HistoricalImportPreview[]>("list_historical_bundle_previews", { bundleId, projectionId }),
  listSentinelOpportunities: (projectId?: number, scanId?: string, status?: string, limit = 500) =>
    invoke<SentinelOpportunity[]>("list_sentinel_opportunities", { projectId, scanId, status, limit }),
  updateSentinelOpportunityStatus: (opportunityId: number, status: string) =>
    invoke<void>("update_sentinel_opportunity_status", { opportunityId, status }),
  getInvestigationGraph: (scanId: string, targetUrl?: string) =>
    invoke<InvestigationGraph>("get_investigation_graph", { scanId, targetUrl }),
  listInvestigationHypotheses: (scanId?: string, status?: string) =>
    invoke<InvestigationHypothesis[]>("list_investigation_hypotheses", { scanId, status }),
  updateInvestigationHypothesis: (hypothesisId: number, status: string) =>
    invoke<void>("update_investigation_hypothesis", { input: { hypothesisId, status } }),
  setInvestigationMutationApproval: (
    hypothesisId: number,
    approved: boolean,
    maxAttempts = 1,
    expiresMinutes = 30,
    note = "",
  ) => invoke<void>("set_investigation_mutation_approval", {
    input: { hypothesisId, approved, maxAttempts, expiresMinutes, note },
  }),
  replayInvestigationRequest: (input: {
    url: string; method: string; headers: Record<string, string>; body?: string;
    timeoutMs?: number; allowMutation?: boolean; identityId?: string;
  }) => invoke<{ status: number; statusText: string; headers: Record<string, string>; body: string; decodedBody: string; contentType: string; contentEncoding: string; bodyIsJson: boolean; elapsedMs: number; identityId: string }>("replay_investigation_request", { input }),
  saveInvestigationValidation: (input: {
    scanId: string; targetUrl: string; opportunityId?: number; hypothesisId?: number; apiKey?: string; identityId?: string;
    method: string; requestUrl: string; requestHeaders: Record<string, string>; requestBody?: string;
    responseStatus: number; responseStatusText?: string; responseHeaders: Record<string, string>; responseBody?: string;
    decodedBody?: string; verdict: string; severity?: string; confidence?: string; aiAssessment?: string; note?: string;
    nextAction?: string; evidenceRefs?: string[];
  }) => invoke<InvestigationValidation>("save_investigation_validation", { input }),
  listInvestigationValidations: (scanId: string, opportunityId?: number) =>
    invoke<InvestigationValidation[]>("list_investigation_validations", { scanId, opportunityId }),
  investigationOverview: (projectId?: number) =>
    invoke<InvestigationOverview>("investigation_overview", { projectId }),
  listAppSecScanResult: (scanId: string) =>
    invoke<AppSecScanResult>("list_appsec_scan_result", { scanId }),
  sentinelOverviewStats: (projectId?: number) =>
    invoke<SentinelOverviewStats>("sentinel_overview_stats", { projectId }),
  listSentinelValidations: (scanId: string) =>
    invoke<SentinelValidation[]>("list_sentinel_validations", { scanId }),
  listAllSentinelValidations: (projectId?: number, limit = 5000) =>
    invoke<SentinelValidation[]>("list_all_sentinel_validations", { projectId, limit }),
  listSentinelValidationWorkItems: (projectId?: number, limit = 5000) =>
    invoke<SentinelValidationWorkItem[]>("list_sentinel_validation_work_items", { projectId, limit }),
  saveSentinelValidation: (input: {
    scanId: string; url: string; findingKey: string; findingKind: string; verdict: string;
    severity: string; note: string; evidence: string;
  }) => invoke<void>("save_sentinel_validation", { input }),
  agentTargetExecution: (scanId: string, url: string) =>
    invoke<AgentTargetExecution>("get_agent_target_execution", { scanId, url }),
  exportSentinelResults: (scanId: string) => invoke<string>("export_sentinel_results", { scanId }),
  importSentinelResults: (content: string) => invoke<number>("import_sentinel_results", { content }),
  exportSentinelProject: (projectId: number) => invoke<string>("export_sentinel_project", { projectId }),
  importSentinelProject: (path: string) => invoke<number>("import_sentinel_project", { path }),
  listCapabilityBundles: () => invoke<CapabilityBundle[]>("list_capability_bundles"),
  listRoleConfigDrafts: () => invoke<RoleConfigDraft[]>("list_role_config_drafts"),
  roleConfigDraftStatusLabel: () => invoke<string>("role_config_draft_status_label"),
  saveRoleConfigDraft: (input: {
    id: string;
    displayName: string;
    description?: string;
    role: string;
    objective?: string;
    capabilityBundleIds: string[];
  }) => invoke<RoleConfigDraft>("save_role_config_draft", { input }),
};
