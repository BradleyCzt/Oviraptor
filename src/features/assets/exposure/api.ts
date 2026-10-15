// §E1 — the exposure surface's own IPC wrapper. `src/features/assets/api.ts` is no longer in
// this slice's whitelist, so every exposure call is defined here instead; the only foreign
// call the page still needs is the asset ledger read for the bound-asset picker, which the
// page passes in from the shared facade without modifying it.
//
// Read-only vs write is stated per entry because the page may not start a collection as a
// side effect of reading. Command names match the registrations the coordinator landed in
// `src-tauri/src/lib.rs` for Q0–Q6; the pre-Q1 entries stay because the page still falls
// back to them while the paginated commands are being wired end to end.

import { invoke } from "@tauri-apps/api/core";
import type { AssetPage, AssetQuery, ExposureFinding, ExposureRun, ExposureSourceResult, ExposureSummary } from "../types";
import type {
  JsonLike,
  ExposureEvidence,
  ExposureFindingDraft,
  ExposureFindingPageInput,
  ExposurePage,
  ExposurePending,
  ExposureScopeSummary,
} from "./contracts";

/// §E2 what the settings endpoint answers: the current values, whether they were saved by a
/// user at all, and the bounds the backend will accept.
export interface ExposureSettingsView extends JsonLike {
  version: number;
  storedVersion: number;
  saved: boolean;
  schemaVersion: number;
  exposureSources: string[];
  exposureMaxRequests: number;
  exposureMaxResponseBytes: number;
  exposureMaxTotalBytes: number;
  exposureMaxItems: number;
  exposureConcurrency?: number;
  exposureGithubCredentialRef?: string;
  exposureTimeoutSeconds: number;
  exposureCtPageSize: number;
  exposureArchivePageSize: number;
  exposureAllowPrivateDestinations: boolean;
  exposureAuthorizationDays: number;
  bounds: JsonLike;
  /** G1-0 what the executor really applies (serial execution, per-domain caps). */
  effective?: JsonLike;
}

export interface ExposureSettingVersionPage {
  /** `settings` is null and `corrupt` true when the stored bytes cannot be parsed. */
  items: { version: number; settings: JsonLike | null; corrupt?: boolean; createdAt: string }[];
  /** True only when older versions exist beyond this window (backend reads limit + 1). */
  truncated: boolean;
  limit?: number;
}

export interface ExposureRunPlanView {
  runId: number;
  projectId: number;
  status: string;
  stage: string;
  eligibleAssets: number;
  scannedAssets: number;
  fetchedResources: number;
  findings: number;
  error: string;
  createdAt: string;
  startedAt: string;
  completedAt: string;
}

/// §E3 one cluster row: what grouped it, which observation the representative came from,
/// how many members carry their own conclusion, and when those bytes were last cached.
export interface ExposureCluster {
  id: number;
  clusterKey: string;
  kind: string;
  representativeAssetId: number | null;
  representativeLabel: string;
  representativeObservationId: number | null;
  representativeRunId: number;
  representativeObservedAt: string;
  memberCount: number;
  directCount: number;
  inheritedCount: number;
  gapNote: string;
  ruleVersion: string;
  settingsVersion: number;
  updatedAt: string;
  cacheTime: string;
  cacheExpiresAt: string;
  /** G1-1 `psl` / `exact_host` / `content_hash`; "" = pre-G1 last-two-labels row. */
  boundaryBasis?: string;
  pslVersion?: string;
  boundaryLabel?: string;
}

/// §E3 one member of a cluster. `evidenceSource` says whether the asset was itself concluded
/// on, inherited from the representative, or lost admission (stale).
export interface ExposureClusterMember {
  assetId: number;
  label: string;
  membership: string;
  evidenceSource: string;
  inheritedFromAssetId: number | null;
  inheritedFromLabel: string;
  inheritedRunId: number;
  observationId: number | null;
  ruleVersion: string;
  verifiedAt: string;
  admittedNow: boolean;
  origin: string;
  contentHash: string;
}

/// §E5 one review decision as it was recorded: actor, reason, the revision it expected and the
/// evidence it referenced. History is append-only, so this is what the page shows.
export interface ExposureReviewEvent {
  fromStatus: string;
  toStatus: string;
  note: string;
  actor: string;
  expectedRevision: number;
  references: number[];
  createdAt: string;
  /** FIX3-R2 复核事件 id；提交复核时带上读到的最新一条，同版本的并发复核才能被识别为冲突。 */
  eventId?: number;
}

/// §E5 a remediation record bound to the finding, the observation and the round.
export interface ExposureRemediation {
  id: number;
  findingId: number;
  observationId: number | null;
  runId: number;
  actor: string;
  status: string;
  note: string;
  createdAt: string;
  closedAt: string;
  retestRunIds: number[];
  findingTitle: string;
  findingStatus: string;
}

/// §E5 what the newest retest round actually saw. `canClose` is a guard, never a verdict.
export interface ExposureRetestOutcome extends JsonLike {
  basis: string;
  canClose: boolean;
  retestRunId?: number;
  roundStatus?: string;
  url?: string;
  previousHash?: string;
  currentHash?: string;
  retestObservedAt?: string;
  note?: string;
}

/// §E5 one conclusion class. The labels keep "no evidence" apart from "no problem".
export interface ExposureTaxonomyRow {
  code: string;
  label: string;
  count: number;
  meaning: string;
}

export const exposureApi = {
  // ——— read-only ———
  /** Read-only: one page of findings with the real filtered total. */
  pageExposureFindings: (input: ExposureFindingPageInput) =>
    invoke<ExposurePage<ExposureFinding> & { dispositions?: Record<string, unknown> }>("page_exposure_findings", { input }),
  /** Read-only: newest-first run page. */
  pageExposureRuns: (projectId: number, pageSize: number, cursor: string) =>
    invoke<ExposurePage<ExposureRun>>("page_exposure_runs", { projectId, pageSize, cursor }),
  /** Read-only: source outcomes of one run, re-checked against the expected project. */
  listExposureSourceResultsForProject: (runId: number, expectedProjectId: number) =>
    invoke<ExposureSourceResult[]>("list_exposure_source_results_for_project", { runId, expectedProjectId }),
  /** Read-only: one finding, scoped. */
  readExposureFinding: (findingId: number, expectedProjectId: number) =>
    invoke<ExposureFinding>("read_exposure_finding", { findingId, expectedProjectId }),
  /** Read-only: §Q5 evidence package (observations, versions, review trail). */
  readExposureEvidence: (findingId: number, expectedProjectId: number) =>
    invoke<ExposureEvidence>("read_exposure_evidence", { findingId, expectedProjectId }),
  /** Read-only: the summary and the list share one predicate. */
  exposureScopeSummary: (projectId: number) =>
    invoke<ExposureScopeSummary>("exposure_scope_summary", { projectId }),
  /** Read-only: the caps the current code actually applies. */
  exposureKnownLimits: () => invoke<unknown[]>("exposure_known_limits"),
  /** Read-only: §E2 the project's own collection settings, defaults and bounds. */
  getExposureSettings: (projectId: number) =>
    invoke<ExposureSettingsView>("get_exposure_settings", { projectId }),
  /** Read-only: §E2 settings history. */
  listExposureSettingVersions: (projectId: number, limit: number) =>
    invoke<ExposureSettingVersionPage>("list_exposure_setting_versions", { projectId, limit }),
  /** Write: §E2 saves a new settings version; a stale editor is refused. */
  saveExposureSettings: (projectId: number, expectedVersion: number, settings: JsonLike) =>
    invoke<number>("save_exposure_settings", { projectId, expectedVersion, settings }),
  /** Read-only: §Q4 pending index discoveries, paged. */
  pageExposurePendingDiscoveries: (projectId: number, status: string, pageSize: number, cursor: string) =>
    invoke<ExposurePage<ExposurePending>>("page_exposure_pending_discoveries", {
      projectId,
      status,
      pageSize,
      cursor,
    }),
  /** Read-only: §E3 one page of content/domain clusters for this project. */
  pageExposureClusters: (
    projectId: number,
    kind: string,
    search: string,
    cursor: string,
    pageSize: number,
  ) =>
    invoke<ExposurePage<ExposureCluster>>("page_exposure_clusters", {
      projectId,
      kind,
      search,
      cursor,
      pageSize,
    }),
  /** Read-only: §E3 one page of a single cluster's members, admission re-checked per row. */
  pageExposureClusterMembers: (projectId: number, clusterId: number, cursor: string, pageSize: number) =>
    invoke<ExposurePage<ExposureClusterMember>>("page_exposure_cluster_members", {
      projectId,
      clusterId,
      cursor,
      pageSize,
    }),
  /** Read-only: §E5 the review classes, each derived from rows that exist. */
  exposureReviewTaxonomy: (projectId: number) =>
    invoke<ExposureTaxonomyRow[]>("exposure_review_taxonomy", { projectId }),
  /** Read-only: §E5 who decided what, when, against which evidence. */
  listExposureReviewTrail: (projectId: number, findingId: number) =>
    invoke<ExposureReviewEvent[]>("list_exposure_review_trail", { projectId, findingId }),
  /** Read-only: §E5 what the newest retest round saw compared with the conclusion's baseline. */
  readExposureRetestOutcome: (projectId: number, remediationId: number) =>
    invoke<ExposureRetestOutcome>("read_exposure_retest_outcome", { projectId, remediationId }),
  /** Read-only: §E4 the digest and row count of an exported evidence file. */
  verifyExposureExport: (path: string) =>
    invoke<JsonLike>("verify_exposure_export", { path }),
  /** Read-only: §E5 one page of remediation records with their retest rounds. */
  pageExposureRemediations: (projectId: number, status: string, pageSize: number, cursor: string) =>
    invoke<ExposurePage<ExposureRemediation>>("page_exposure_remediations", {
      projectId,
      status,
      pageSize,
      cursor,
    }),

  // ——— §E5 writes, each behind an explicit control ———
  /** Write: a review decision carrying the revision it read; a moved queue is refused. */
  reviewExposureFindingScoped: (
    projectId: number,
    findingId: number,
    expectedRevision: number,
    status: string,
    actor: string,
    reason: string,
    references: number[],
    expectedReviewEvent: number,
  ) =>
    invoke<number>("review_exposure_finding_scoped", {
      projectId,
      findingId,
      expectedRevision,
      status,
      actor,
      reason,
      references,
      expectedReviewEvent,
    }),
  /** Write: bind a remediation to the finding, the observation and the round. */
  saveExposureRemediation: (
    projectId: number,
    findingId: number,
    observationId: number | null,
    runId: number,
    actor: string,
    note: string,
    status: string,
  ) => invoke<number>("save_exposure_remediation", { projectId, findingId, observationId, runId, actor, note, status }),
  /** Write: start a retest as a new round, re-authorized; the old observation stays as it is. */
  startExposureRetest: (projectId: number, remediationId: number) =>
    invoke<number>("start_exposure_retest", { projectId, remediationId }),
  /** Write: close or reject a remediation; closing needs a retest basis that supports it. */
  markExposureRemediation: (projectId: number, remediationId: number, status: string, actor: string, reason: string) =>
    invoke<ExposureRetestOutcome>("mark_exposure_remediation", { projectId, remediationId, status, actor, reason }),
  /** Read-only: the pre-Q1 window, kept so the page can say what it covers. */
  exposureSummary: (projectId: number) => invoke<ExposureSummary>("exposure_summary", { projectId }),
  listExposureFindings: (projectId: number, status = "", search = "") =>
    invoke<ExposureFinding[]>("list_exposure_findings", { projectId, status, search }),
  listExposureRuns: (projectId: number) => invoke<ExposureRun[]>("list_exposure_runs", { projectId }),
  listExposureSourceResults: (runId: number, expectedProjectId: number) =>
    invoke<ExposureSourceResult[]>("list_exposure_source_results", { runId, expectedProjectId }),

  // ——— writes, each only behind an explicit control ———
  /** Write: creates or reuses the active round for this project. Explicit only. */
  startExposureScan: (projectId: number) => invoke<number>("start_exposure_scan", { projectId }),
  /** Write: continues a round whose frozen plan still matches. Explicit only. */
  resumeExposureScan: (runId: number, expectedProjectId: number) =>
    invoke<number>("resume_exposure_scan", { runId, expectedProjectId }),
  /** Write: asks for cancellation; collected results stay. */
  cancelExposureScan: (projectId: number, runId: number) => invoke<void>("cancel_exposure_scan", { projectId, runId }),
  /** Write: merges one manual candidate into the review queue. */
  saveExposureFinding: (input: ExposureFindingDraft) => invoke<number>("save_exposure_finding", { input }),
  // G0/P1-4 / IR-01 landed 2026-10-09: the unversioned review command is retired from
  // lib.rs and from the shared facade. Reviews go through `reviewExposureFindingScoped`.
  /** Write: import files or a user-supplied intel report. The declared URL is stored, never fetched. */
  importExposureDocuments: (projectId: number, request: JsonLike) =>
    invoke<unknown>("import_exposure_documents", { projectId, request }),
  /** Write: one manual intel grade. Never closes the lead and never contacts the source. */
  gradeExposureIntel: (projectId: number, request: JsonLike) =>
    invoke<unknown>("grade_exposure_intel", { projectId, request }),
  /** Write: one manual reclassification. CAS on the classification id the page read. */
  reclassifyExposureFinding: (projectId: number, request: JsonLike) =>
    invoke<unknown>("reclassify_exposure_finding", { projectId, request }),
  /** Write: drops the redacted CSV into the export directory. G4-3 `kind` = findings (default)
   *  or ledger (incident ledger with rowKind rows). The file is named
   *  `<项目名>_暴露面[_事件台账]_<本地时间+时区>.csv`; hash and rows come from verifyExposureExport. */
  exportExposureFindings: (projectId: number, kind?: "findings" | "ledger") =>
    invoke<{ path: string; rows: number }>("export_exposure_findings", kind ? { projectId, kind } : { projectId }),
};

/// G4 incident commands. Registered in lib.rs (IR-15 landed 2026-10-09). An unknown-command
/// answer still means integration_pending, which is not the same as "no incidents".
export const exposureIncidentApi = {
  /** Read-only: one keyset page of deduplicated incidents. */
  pageExposureIncidents: (projectId: number, status: string, reviewNeededOnly: boolean, pageSize: number, cursor: string) =>
    invoke<unknown>("page_exposure_incidents", { projectId, status, reviewNeededOnly, pageSize, cursor }),
  /** Read-only: one incident with locations, unit links and its action trail. */
  readExposureIncident: (projectId: number, incidentId: number) =>
    invoke<unknown>("read_exposure_incident", { projectId, incidentId }),
  /** Read-only: the next page of one incident's locations. */
  pageExposureIncidentLocations: (projectId: number, incidentId: number, pageSize: number, cursor: string) =>
    invoke<unknown>("page_exposure_incident_locations", { projectId, incidentId, pageSize, cursor }),
  /** Write: one human action with expectedRevision and operationId. Handling is recorded only. */
  actOnExposureIncident: (projectId: number, action: JsonLike) =>
    invoke<unknown>("act_on_exposure_incident", { projectId, action }),
  /** Read-only (P1 UI): one coverage row drilled down by time window and unit, items paged by cursor. */
  exposureCoverageDrilldown: (projectId: number, sourceKey: string, windowDays: number, unitEntityId: number | null, gapsOnly: boolean, pageSize: number, cursor: string) =>
    invoke<unknown>("exposure_coverage_drilldown", { projectId, sourceKey, windowDays, unitEntityId, gapsOnly, pageSize, cursor }),
  /** Write: suggest incidents for rounds that finished before the ledger existed. Records only. */
  proposeExposureIncidents: (projectId: number) =>
    invoke<unknown>("propose_exposure_incidents", { projectId }),
};

/// The one non-exposure call the page needs: the asset ledger behind the bound-asset picker.
export type AssetPageReader = (query: AssetQuery) => Promise<AssetPage>;
