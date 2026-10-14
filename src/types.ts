export interface NativeSourceFinding {
  id: string;
  sourceDecisionId: string;
  scanId: string;
  attemptNumber: number;
  rootRunId: string;
  reviewerRunId: string;
  materialDigest: string;
  title: string;
  path: string;
  line: number | null;
  cwe: string;
  severity: string;
  reviewState: "confirmed";
  rationale: string;
  confidence: number;
  evidenceRefs: string[];
}

export interface NativeSourceCoverageDecision {
  schemaVersion: 1;
  id: string;
  scanId: string;
  attemptNumber: number;
  rootRunId: string;
  reviewerRunId: string;
  assignmentId: string;
  messageId: string;
  modelRequestHash: string;
  modelResponseHash: string;
  materialDigest: string;
  decision: {
    schemaVersion: 1;
    subject: "source_coverage";
    materialDigest: string;
    coverageSufficient: boolean;
    rationale: string;
    reasonCodes: string[];
    evidenceRefs: string[];
    outstandingGaps: string[];
  };
}

export interface NativeSourceCoverageSummary {
  schemaVersion: 1;
  status: "prepared_not_reviewed" | "reviewed";
  scanId: string;
  attemptNumber: number;
  rootRunId: string;
  materialDigest: string;
  executionEligible: false;
  independentReviewCompleted: boolean;
  coverageSufficient?: boolean;
  sourceCoverageDecision?: NativeSourceCoverageDecision;
  requestedScope: "auto" | "full" | "diff";
  effectiveScope: "full" | "diff" | "unavailable";
  selectedFileCount: number;
  changedPathsWithoutContentCount: number;
  outstandingGaps: string[];
}

export interface NativeSourceFindingsPage {
  schemaVersion: 1;
  scanId: string;
  attemptNumber: number;
  status: "audited" | "unverified" | "not_available";
  rootRunId: string | null;
  materialDigest: string | null;
  counts: { decisions: number; confirmed: number; rejected: number; insufficient: number } | null;
  independentCandidateReviewCompleted: boolean;
  independentReviewCompleted: boolean;
  candidateReviewStatus?: "completed" | "not_applicable_no_candidates";
  materialSubject?: "source_candidates" | "source_coverage";
  coverage?: NativeSourceCoverageSummary | null;
  offset: number;
  nextOffset: number | null;
  findings: NativeSourceFinding[];
}

export type ViewKey =
  | "chat"
  | "skills"
  | "dashboard"
  | "projects"
  | "query"
  | "assets"
  | "ownership"
  | "exposure"
  | "quarantine"
  | "hackerone"
  | "sentinel"
  | "changes"
  | "tasks"
  | "logs"
  | "settings";

export interface Project {
  id: number;
  name: string;
  description: string;
  status: string;
  assetCount: number;
  pendingCount: number;
  targetCount: number;
  assetRunCount: number;
  scanCount: number;
  vulnerabilityCount: number;
  validationCount: number;
  activeFuseCount: number;
  lastRunAt?: string;
  lastScanAt?: string;
  createdAt: string;
  updatedAt: string;
}
export interface ProjectImpact {
  assetCount: number;
  assetEventCount: number;
  targetCount: number;
  assetRunCount: number;
  savedViewCount: number;
  sentinelScanCount: number;
  sentinelTargetCount: number;
  findingCount: number;
  validationCount: number;
  opportunityCount: number;
  fuseCount: number;
  appsecVulnerabilityCount: number;
  knowledgeCount: number;
  learningCandidateCount: number;
  browserAuthSessionCount: number;
  otherRecordCount: number;
  totalRecords: number;
}

export interface BrowserAuthSession {
  id: string;
  projectId: number;
  ownerScanId: string;
  draftScopeId: string;
  name: string;
  entryUrl: string;
  finalUrl: string;
  status: "capturing" | "valid" | "needs_check" | "invalid" | "expired" | string;
  scopeHosts: string[];
  cookieCount: number;
  headerCount: number;
  storageCount: number;
  capturedRequestCount: number;
  lastValidatedAt: string;
  expiresAt: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
}

export interface ScanIdentityMaterial {
  session: BrowserAuthSession;
  document: Record<string, unknown>;
}

export interface ConfigProfile {
  id: number;
  name: string;
  description: string;
  isDefault: boolean;
  settings: Record<string, any>;
  createdAt: string;
  updatedAt: string;
}

export interface Target {
  id: number;
  projectId: number;
  targetType: string;
  value: string;
  enabled: boolean;
  createdAt: string;
}

export interface DashboardStats {
  projectCount: number;
  assetCount: number;
  aliveCount: number;
  pendingCount: number;
  newCount: number;
  changedCount: number;
  blockedCount: number;
  runningJobs: number;
}

export interface Asset {
  id: number;
  projectId: number;
  assetKey: string;
  company: string;
  host: string;
  link: string;
  ip: string;
  port: string;
  protocol: string;
  domain: string;
  title: string;
  statusCode: string;
  probeOutcome: string;
  probeEntryState: string;
  reviewTier: string;
  contentCategory: string;
  score: string;
  decision: string;
  note: string;
  ownershipStatus: string;
  ownershipConfidence: number;
  authorizationStatus: string;
  exposureEligible: boolean;
  ownershipSource: string;
  ownershipReason: string;
  isDeleted: boolean;
  firstSeen: string;
  lastSeen: string;
  lastAlive?: string;
  extra: Record<string, string>;
  sentinelStatus: string;
  sentinelScanCount: number;
  sentinelSentAt?: string;
  projectFirstSeen: string;
  projectLastSeen: string;
  lastRunId?: number;
  deletedAt?: string;
  projectName: string;
  hasLogin?: number;
  hasRegister?: number;
  entryOpenedAt?: string | null;
}

export interface ExposureFinding {
  id: number; projectId: number; assetId?: number; category: string; title: string;
  sourceType: string; sourceUrl: string; evidenceExcerpt: string; severity: string;
  confidence: number; status: string; note: string; assetLabel: string;
  exposureEligible: boolean; firstSeenAt: string; lastSeenAt: string;
}
export interface ExposureSummary {
  eligibleAssets: number; total: number; newCount: number; reviewing: number;
  confirmed: number; dismissed: number; highRisk: number;
}
export interface ExposureRun {
  id:number; projectId:number; status:string; eligibleAssets:number; scannedAssets:number;
  fetchedResources:number; findings:number; stage:string; currentSource:string; error:string; createdAt:string; startedAt:string; completedAt:string;
}
export interface ExposureSourceResult { runId:number; sourceKey:string; status:string; itemCount:number; error:string; completedAt:string }

export interface FilterCondition {
  field: string;
  operator: string;
  value: string;
  join: "and" | "or";
}

export interface AssetQuery {
  projectId?: number;
  search: string;
  conditions: FilterCondition[];
  page: number;
  pageSize: number;
  includeDeleted: boolean;
  deletedView?: "active" | "trash" | "all" | string;
  probeView?: string;
  probeOutcomeView?: string;
  sentinelView?: string;
  decisionView?: string;
  ownershipView?: string;
  hasLoginView?: string;
  hasRegisterView?: string;
  entrySeenView?: string;
  sortBy?: string;
  sortDirection?: "asc" | "desc" | string;
}

export interface AssetSummary {
  all: number;
  pending: number;
  uncertain: number;
  confirmed: number;
  rejected: number;
  notApplicable: number;
  sentToAgent: number;
}

export interface AssetPage {
  items: Asset[];
  total: number;
  page: number;
  pageSize: number;
  summary: AssetSummary;
}

export interface AssetSelection {
  projectId: number;
  assetId: number;
}

export interface AssetOwnershipProfile {
  projectId: number;
  legalName: string;
  jurisdiction: string;
  jurisdictions: string[];
  excludedJurisdictions: string[];
  aliases: string[];
  approvedDomains: string[];
  sharedDomains: string[];
  excludedNames: string[];
  excludedDomains: string[];
  notes: string;
  policyVersion: number;
  updatedAt: string;
}

export interface AssetOwnershipSummary {
  all: number;
  unreviewed: number;
  attributed: number;
  confirmed: number;
  related: number;
  thirdParty: number;
  excluded: number;
  exposureEligible: number;
}

export interface AssetOwnershipAssessmentResult {
  assessed: number;
  summary: AssetOwnershipSummary;
}

export interface ContentRuleApplyResult {
  ruleId: number;
  keyword: string;
  matchedAssets: number;
  matchedProjectAssets: number;
}

export interface JobRun {
  id: number;
  projectId: number;
  profileId?: number;
  projectName: string;
  name: string;
  pipeline: string;
  status: string;
  stage: string;
  progress: number;
  processed: number;
  total: number;
  outputDir: string;
  error: string;
  startedAt?: string;
  finishedAt?: string;
  createdAt: string;
}

export interface AppSettings {
  reminderDays: number;
  customIcon: boolean;
  deduplicatedAssets: number;
}

export interface StaleProject {
  projectId: number;
  projectName: string;
  daysSinceUpdate?: number;
  lastRunAt?: string;
}

export interface InterruptedJob {
  runId: number;
  projectId: number;
  projectName: string;
  profileId?: number;
  name: string;
  pipeline: string;
  createdAt: string;
}

export interface StartupStatus {
  reminderDays: number;
  staleProjects: StaleProject[];
  interruptedJobs: InterruptedJob[];
}

export interface LogEntry {
  id: number;
  runId?: number;
  level: string;
  stage: string;
  message: string;
  createdAt: string;
}

export interface AssetEvent {
  id: number;
  projectId: number;
  assetId: number;
  assetKey: string;
  company: string;
  host: string;
  eventType: string;
  summary: string;
  runId?: number;
  createdAt: string;
}

export interface JobProgressEvent {
  runId: number;
  status: string;
  stage: string;
  progress: number;
  message: string;
}

export interface ToastMessage {
  id: number;
  type: "success" | "error" | "info";
  text: string;
}

export interface HackerOneProgram {
  id: string;
  handle: string;
  name: string;
  iconUrl: string;
  policy: string;
  submissionState: string;
  programState: string;
  offersBounties: boolean;
  openScope: boolean;
  fastPayments: boolean;
  safeHarbor: boolean;
  collaboration: boolean;
  lastSyncedAt: string;
  bookmarked: boolean;
  scopeCount: number;
}
export interface HackerOneScope {
  id: string;
  assetType: string;
  assetIdentifier: string;
  eligibleForSubmission: boolean;
  eligibleForBounty: boolean;
  maxSeverity: string;
  instruction: string;
  updatedAt?: string;
}
export interface HackerOneExclusion {
  id: string;
  category: string;
  details: string;
  updatedAt?: string;
}
export interface HackerOneDetail {
  program: HackerOneProgram;
  scopes: HackerOneScope[];
  exclusions: HackerOneExclusion[];
}
export interface HackerOneEvent {
  id: number;
  programHandle: string;
  eventType: string;
  summary: string;
  createdAt: string;
}
export interface SentinelScan {
  administrativeClosureRecorded?: boolean;
  closureHandoffRecorded?: boolean;
  id: string;
  projectId?: number;
  projectName: string;
  status: string;
  currentCheckpoint: string;
  taskPath: string;
  previousScanId: string;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  scanType: string;
  taskName: string;
  sourcePath: string;
  skillNames: string;
  attemptCount: number;
  archivedAt: string;
  createdAt: string;
  updatedAt: string;
  requestedScanMode: string;
  llmModel: string;
  llmDeployment: "cloud" | "local" | "unknown" | string;
  llmFullPower: boolean;
  latestAttemptNumber: number;
  latestAttemptStatus: string;
  latestAttemptCheckpoint: string;
  latestAttemptStopReason: string;
}
export interface SentinelScanAttempt {
  scanId: string;
  attemptNumber: number;
  executionMode: "initial" | "fresh" | "resume" | string;
  status: string;
  stage: string;
  checkpoint: string;
  stopReason: string;
  workDir: string;
  backendPlanJson?: string;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  startedAt: string;
  finishedAt: string;
  updatedAt: string;
}
export interface SentinelRunnerLogView {
  scanId: string;
  attempt: number;
  attemptStatus: string;
  stage: string;
  source: "attempt_work_dir" | "task_root" | string;
  workDir: string;
  updatedAt: string;
  status: "ready" | "not_created" | "empty" | "read_failed" | string;
  message: string;
  lines: string[];
}
export interface ModelProfileTestResult {
  ok: boolean;
  status: string;
  message: string;
  model: string;
  deployment: "cloud" | "local" | string;
}
export interface FofaApiTestResult {
  ok: boolean;
  status: string;
  message: string;
  account: string;
  plan: string;
}
export interface AgentSkill {
  id: number;
  name: string;
  description: string;
  instructions: string;
  builtin: boolean;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface SecurityRuleChange {
  status: "added" | "modified" | "deleted";
  path: string;
}
export interface SecurityRulePack {
  id: number;
  key: string;
  name: string;
  engine: string;
  repository: string;
  reference: string;
  localPath: string;
  previousVersion: string;
  version: string;
  enabled: boolean;
  builtin: boolean;
  status: string;
  lastSyncAt: string;
  error: string;
  addedCount: number;
  modifiedCount: number;
  deletedCount: number;
  changeSummary: SecurityRuleChange[];
  progress: number;
  progressStage: string;
  progressMessage: string;
}
export interface AgentTraceToolStat {
  name: string;
  calls: number;
  results: number;
}
export interface AgentTraceSummary {
  sourceAuthority: "native_ledger" | "historical_external";
  scanId: string;
  taskName: string;
  projectName: string;
  status: string;
  scanType: string;
  model: string;
  runCount: number;
  agentCount: number;
  messageCount: number;
  reasoningCount: number;
  toolCallCount: number;
  toolResultCount: number;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  hookedRequestCount: number;
  exactRequestCapture: boolean;
  usageEntryCount: number;
  usageAgentCount: number;
  tokenUsageEstimated: boolean;
  instructionHash: string;
  tools: AgentTraceToolStat[];
  knowledgeId?: number;
  createdAt: string;
  updatedAt: string;
}
export interface AgentTraceEvent {
  id: string;
  sessionId: string;
  callId: string;
  targetUrl: string;
  eventType: string;
  role: string;
  name: string;
  status: string;
  detail: string;
  detailSize: number;
  detailTruncated: boolean;
  createdAt: string;
}
export interface AgentTraceDetail {
  summary: AgentTraceSummary;
  events: AgentTraceEvent[];
  promptAudit?: ModelPromptAudit;
}
export interface ModelPromptAudit {
  captureMode: "metadata" | "full" | string;
  source: string;
  captureLevel: string;
  exactModelRequest: boolean;
  model: string;
  deployment: "cloud" | "local" | string;
  fullPower: boolean;
  recordedAt: string;
  instructionSha256: string;
  instructionChars: number;
  instruction?: string;
  notice: string;
}
export interface AgentKnowledgeEntry {
  id: number;
  scanId: string;
  projectId?: number;
  title: string;
  summary: string;
  patterns: Record<string, unknown>;
  sourceHash: string;
  skillId?: number;
  createdAt: string;
  updatedAt: string;
}
export interface AgentLearningCandidate {
  id: number;
  scanId: string;
  projectId?: number;
  scanType: string;
  title: string;
  summary: string;
  candidate: Record<string, any>;
  status: "pending" | "accepted" | "rejected" | "applied" | string;
  targetSkillId?: number;
  sourceHash: string;
  createdAt: string;
  reviewedAt: string;
  updatedAt: string;
}
export interface WorkbenchScanInput {
  projectId: number;
  taskName: string;
  scanType: "web" | "code" | "greybox" | "cicd";
  urls: string[];
  sourcePath: string;
  skillIds: number[];
  instruction: string;
  scanMode: "quick" | "standard" | "deep";
  scopeMode: "auto" | "diff" | "full";
  diffBase: string;
  maxBudgetUsd?: number;
  environment: string;
  authProfileName: string;
  authType: "none" | "cookie" | "bearer" | "header";
  authHeaderName: string;
  authValue: string;
  authSessionId: string;
  authSessionIds: string[];
  authSessionScopeId: string;
  ciProvider: string;
  repositoryUrl: string;
  branch: string;
  commitSha: string;
  buildId: string;
  maxCritical: number;
  maxHigh: number;
  blockRelease: boolean;
}
export interface AppSecVulnerability {
  id: number;
  projectId: number;
  fingerprint: string;
  title: string;
  vulnerabilityType: string;
  severity: string;
  status: string;
  confidence: string;
  asset: string;
  environment: string;
  url: string;
  httpMethod: string;
  parameter: string;
  file: string;
  symbol: string;
  startLine: number;
  correlationScore: number;
  correlation: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
  owner: string;
}
export interface AppSecVulnerabilitySource {
  id: number;
  vulnerabilityId: number;
  scanId: string;
  findingId?: number;
  sourceType: string;
  sourceKey: string;
  engine: string;
  evidence: Record<string, any>;
  createdAt: string;
}
export interface AppSecScanContext {
  scanId: string;
  environment: string;
  authProfileName: string;
  authType: string;
  authenticated: boolean;
  ciProvider: string;
  repositoryUrl: string;
  branch: string;
  commitSha: string;
  buildId: string;
  policy: Record<string, any>;
  gateStatus: string;
  gateReason: string;
  createdAt: string;
  updatedAt: string;
}
export interface AppSecScanResult {
  vulnerabilities: AppSecVulnerability[];
  sources: AppSecVulnerabilitySource[];
  context?: AppSecScanContext;
}
export interface SentinelTarget {
  id: number;
  projectId: number;
  scanId?: string;
  company: string;
  url: string;
  status: string;
  valueScore: number;
  scanMode: string;
  routingReason: string;
  lastAttemptNumber: number;
  createdAt: string;
  updatedAt: string;
  scanCount: number;
}
export interface SentinelFuseEntry {
  id: number;
  projectId: number;
  assetId?: number;
  company: string;
  url: string;
  sourceScanId: string;
  reason: string;
  verdict: string;
  note: string;
  evidence: string;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface SentinelCheckpoint {
  scanId: string;
  url: string;
  stage: string;
  rawJson: string;
  updatedAt: string;
}

/** Oviraptor-owned execution plan for one target, from either agent backend. */
export type AgentRequestAccounting = {
  available: true;
  scope: "agent_budget_lineage";
  attemptNumber: number;
  includedAttempts: number[];
  executorRecordedRequests: number;
  /** Subset of executorRecordedRequests; omitted in historical snapshots. */
  executorUnresolvedClaims?: number;
  externalSurfaceReceivedRequests: number;
  externalSurfaceUnresolvedClaims: number;
  authorizationReceivedRequests: number;
  authorizationUnresolvedClaims: number;
  recordedRequests: number;
  budgetCommittedRequests: number;
  includesDeterministicRecon: false;
  automaticReplayAllowed: false;
} | {
  available: false;
  scope: "agent_budget_lineage";
  attemptNumber: number;
  reasonCode: string;
  recordedRequests: null;
  budgetCommittedRequests: null;
  automaticReplayAllowed: false;
};

export type AgentRequestDisposition = "effect_observed" | "not_sent_attested" | "still_unknown";
export interface AgentRequestReview {
  id: string;
  previousReviewId: string;
  snapshotHash: string;
  snapshot: Record<string, unknown>;
  disposition: AgentRequestDisposition;
  note: string;
  actor: "local_operator";
  createdAt: string;
  reviewedAttempt: number;
}
export interface AgentRequestReviewItem {
  requestKey: string;
  snapshotHash: string;
  snapshot: Record<string, unknown> & { source: string; sourceAttempt: number; runId: string };
  receiptState: string;
  sourceChangedSinceReview: boolean;
  reviews: AgentRequestReview[];
}
export interface AgentRequestReviewPage {
  scanId: string;
  attemptNumber: number;
  targetUrl: string;
  items: AgentRequestReviewItem[];
  accounting: AgentRequestAccounting;
  unitemizedExecutorRequests: number;
  automaticReplayAllowed: false;
  executionUnlocked: false;
}
export interface AgentRequestReviewInput {
  scanId: string;
  attemptNumber: number;
  targetUrl: string;
  requestKey: string;
  snapshotHash: string;
  previousReviewId: string;
  operationId: string;
  disposition: AgentRequestDisposition;
  note: string;
  operatorConfirmed: boolean;
}

export interface AgentTargetExecution {
  backend: "native" | "legacy_backend_removed";
  mode: string;
  surface?: string;
  timeoutSeconds?: number;
  budgets?: {
    softUncachedTokens?: number;
    hardTotalTokens?: number;
    softModelRequests?: number;
    hardModelRequests?: number;
    maxTurns?: number;
  };
  hardLimits?: {
    hardTotalTokens?: number;
    hardModelRequests?: number;
    maxTurns?: number;
    noProgressWindow?: number;
  };
  coverage?: {
    required: string[];
    requiredLabels: string[];
    covered: string[];
    completedRatio: number;
    ledgerReported: boolean;
    confirmedFindings: number;
    ledger?: {
      summary?: string;
      uncoveredFamilies?: {
        family: string;
        label: string;
        status: string;
        reason: string;
        reasonCode?: string;
      }[];
      exclusions?: string[];
      manualDeepDiveSuggestions?: string[];
    };
  };
  runtime?: {
    turns: number;
    noProgressStreak: number;
    currentAction?: string | null;
    progressSignature: string;
    lastExpansionReason: string;
    terminalReason: string;
    requestAccounting?: AgentRequestAccounting;
    budgetUsage?: { modelRequests?: number; targetRequests?: number | null; discoveryRounds?: number };
    tokenUsage?: {
      inputTokens?: number;
      cachedInputTokens?: number;
      outputTokens?: number;
      totalTokens?: number;
      modelRequests?: number;
    };
  };
  targetStatus: string;
  targetStatusText: string;
}
export interface SentinelFinding {
  id: number;
  scanId: string;
  targetUrl: string;
  stage: string;
  kind: string;
  recordKey: string;
  title: string;
  severity: string;
  recordJson: string;
  updatedAt: string;
}
export interface HistoricalImportPreview {
  membershipId: number;
  scanId: string;
  attemptNumber: number;
  kind: "finding_candidate" | "coverage" | "run_state" | "evidence_note";
  title: string;
  severity: string;
  target: string;
  producer: string;
  reviewState: "unreviewed";
  readOnly: true;
  executionEligible: false;
}
export interface HistoricalImportRun {
  rowId: number;
  bundleId: string;
  scanId: string;
  attemptNumber: number;
  title: string;
  status: string;
  findingCandidates: number;
  coverageRecords: number;
  importedAt: string;
}
export interface SentinelOpportunity {
  id: number;
  projectId?: number;
  scanId: string;
  targetUrl: string;
  opportunityKey: string;
  category: string;
  title: string;
  score: number;
  status: "queued" | "ready" | "in_progress" | "validated" | "dismissed" | "exhausted" | "needs_more_evidence" | "blocked_by_authorization" | "closed" | string;
  confidence: string;
  why: string[];
  evidence: Array<Record<string, any>>;
  recommendedAction: Record<string, any>;
  source: string;
  record: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
}
export interface InvestigationNode {
  id: number;
  scanId: string;
  targetUrl: string;
  nodeKey: string;
  nodeType: "target" | "identity" | "page_state" | "action" | "api" | "parameter" | "hypothesis" | string;
  label: string;
  confidence: string;
  valueScore: number;
  status: string;
  payload: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
}
export interface InvestigationEdge {
  id: number;
  scanId: string;
  targetUrl: string;
  sourceKey: string;
  relation: string;
  targetKey: string;
  confidence: string;
  evidence: Record<string, any> | any[];
  createdAt: string;
}
export interface InvestigationAction {
  id: number;
  scanId: string;
  targetUrl: string;
  actionKey: string;
  stateKey: string;
  actionType: string;
  label: string;
  outcome: string;
  valueScore: number;
  protocol: Record<string, any>;
  createdAt: string;
  updatedAt: string;
}
export interface InvestigationApiModel {
  id: number;
  scanId: string;
  targetUrl: string;
  apiKey: string;
  method: string;
  url: string;
  normalizedPath: string;
  source: string;
  confidence: string;
  authScope: string;
  parameters: string[];
  requestSchema: Record<string, any>;
  responseSchema: Record<string, any>;
  stateKeys: string[];
  actionKeys: string[];
  identityKeys: string[];
  observedCount: number;
  baselineStatus: "new" | "changed" | "unchanged" | string;
  payload: Record<string, any>;
  updatedAt: string;
  captureStatus?: string;
  responseBody?: string;
  responseHeaders?: Record<string, string>;
  requestHeaders?: Record<string, string>;
  decodedBody?: string;
}
export interface InvestigationHypothesis {
  id: number;
  projectId?: number;
  scanId: string;
  targetUrl: string;
  hypothesisKey: string;
  category: string;
  title: string;
  status: string;
  score: number;
  confidence: string;
  contract: Record<string, any>;
  evidence: any[] | Record<string, any>;
  decision: Record<string, any>;
  mutationApproval: {
    approved?: boolean;
    active?: boolean;
    scope?: Record<string, any>;
    maxAttempts?: number;
    note?: string;
    expiresAt?: string;
    updatedAt?: string;
  };
  sourceOpportunityKey: string;
  createdAt: string;
  updatedAt: string;
}
export interface InvestigationIdentityDiff {
  id: number;
  scanId: string;
  targetUrl: string;
  apiKey: string;
  leftIdentityKey: string;
  rightIdentityKey: string;
  differenceType: string;
  riskScore: number;
  status: string;
  matrix: Record<string, any>;
  createdAt: string;
}
export interface InvestigationRelatedService {
  host: string;
  classification: "monitoring_telemetry" | "device_fingerprint" | "page_bootstrap" | "background_service" | string;
  relation: "same_party" | "third_party" | string;
  requestCount: number;
  methods: string[];
  paths: string[];
  queryKeys: string[];
  identityKeys: string[];
  resourceTypes: string[];
  sources: string[];
  statuses: number[];
  firstUrl: string;
  evidenceSource: string;
}
export interface InvestigationMetrics {
  scanId: string;
  targetUrl: string;
  nodeCount: number;
  edgeCount: number;
  stateCount: number;
  actionCount: number;
  apiCount: number;
  parameterCount: number;
  hypothesisCount: number;
  addedCount: number;
  changedCount: number;
  removedCount: number;
  duplicateCount: number;
  informationGain: number;
  tokenWorthy: boolean;
  stopReason: string;
  decision: Record<string, any>;
  updatedAt: string;
}
export interface InvestigationGraph {
  scanId: string;
  targetUrl: string;
  nodes: InvestigationNode[];
  edges: InvestigationEdge[];
  actions: InvestigationAction[];
  apis: InvestigationApiModel[];
  relatedServices: InvestigationRelatedService[];
  hypotheses: InvestigationHypothesis[];
  identityDiffs: InvestigationIdentityDiff[];
  metrics?: InvestigationMetrics;
}
export interface InvestigationOverview {
  targetCount: number;
  nodeCount: number;
  edgeCount: number;
  apiCount: number;
  parameterCount: number;
  hypothesisCount: number;
  readyHypothesisCount: number;
  identityDiffCount: number;
  tokenWorthyCount: number;
  averageInformationGain: number;
  factCount: number;
  promotedStrategyCount: number;
}
export interface SentinelOverviewStats {
  taskCount: number;
  urlCount: number;
  fingerprintCount: number;
  apiCount: number;
  endpointCount: number;
  vulnerabilityCount: number;
  highRiskCount: number;
  reviewerConfirmedCount: number;
  reviewerHighRiskCount: number;
  sourceReviewerConfirmedCount: number;
  sourceReviewAuditedTaskCount: number;
  sourceReviewUnavailableTaskCount: number;
  sourceReviewUnverifiedTaskCount: number;
  otherVulnerabilityCount: number;
  validatedCount: number;
  pendingVulnerabilityCount: number;
  vulnerableUrlCount: number;
  activeFuseCount: number;
  opportunityCount: number;
  readyOpportunityCount: number;
}
export interface InvestigationValidation {
  id: number;
  scanId: string;
  targetUrl: string;
  opportunityId?: number;
  hypothesisId?: number;
  apiKey?: string;
  identityId?: string;
  method: string;
  requestUrl: string;
  requestHeaders: Record<string, string>;
  requestBody: string;
  responseStatus: number;
  responseStatusText: string;
  responseHeaders: Record<string, string>;
  responseBody: string;
  decodedBody: string;
  verdict: string;
  severity: string;
  confidence: string;
  aiAssessment: string;
  note: string;
  nextAction: string;
  evidenceRefs: string[];
  createdAt: string;
  updatedAt: string;
}
export interface SentinelValidation {
  id: number;
  scanId: string;
  url: string;
  findingKey: string;
  findingKind: string;
  verdict: string;
  severity: string;
  note: string;
  evidence: string;
  createdAt: string;
  updatedAt: string;
}
export interface SentinelValidationWorkItem {
  findingId: number;
  scanId: string;
  projectId?: number;
  projectName: string;
  taskName: string;
  url: string;
  findingKey: string;
  findingKind: string;
  title: string;
  originalSeverity: string;
  recordJson: string;
  validationId?: number;
  verdict: string;
  confirmedSeverity: string;
  note: string;
  evidence: string;
  updatedAt: string;
}
export interface EnvironmentDependency {
  name: string;
  command: string;
  version: string;
  available: boolean;
  detail: string;
}
export interface EnvironmentReport {
  os: string;
  arch: string;
  node: string;
  dependencies: EnvironmentDependency[];
  checkedAt: string;
}

export interface EnvironmentPreparationStatus {
  state: "idle" | "installing" | "requires_manual_recovery";
  owner: string | null;
  createdAt: string | null;
}

export interface LocalWorkerSettings {
  enabled: boolean;
  port: number;
  accessToken: string;
  tailscaleIp: string;
  endpoint: string;
  running: boolean;
  status: string;
}

export interface RemoteWorkerNode {
  id: number;
  name: string;
  endpoint: string;
  accessToken: string;
  enabled: boolean;
  lastSeenAt?: string;
  lastSyncAt?: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
}

export interface WorkerHealth {
  service: string;
  version: string;
  hostname: string;
  os: string;
  arch: string;
  tailscaleIp: string;
  runningScans: number;
  completedScans: number;
  checkedAt: string;
}

/** §8 role configuration draft — never an execution instance or collab message. */
export interface CapabilityBundle {
  id: string;
  displayName: string;
  description: string;
  sandboxDefault: string;
  sideEffectClass: string;
  compatibleRoles: string[];
  tools: string[];
}

export interface RoleConfigDraft {
  id: string;
  displayName: string;
  description: string;
  role: string;
  objective: string;
  capabilityBundleIds: string[];
  status: string;
  statusLabel: string;
  createdAt: string;
  updatedAt: string;
}
export interface GapFollowupReview {
  status: "pending_review" | "resolved" | "insufficient_evidence" | "unverified";
  gapResolved: boolean;
  requestId?: string;
  verdict?: "confirmed" | "rejected" | "insufficient_evidence";
  hypothesisVerdict?: "confirmed" | "rejected" | "insufficient_evidence";
  summary?: string;
  reasonCode?: string;
  candidateId?: string;
  candidateRevision?: number;
  assessment?: { sourceHash: string; hypothesisVerdict: "confirmed" | "rejected" | "insufficient_evidence"; items: Array<{ index: number; missingEvidence: string; status: "addressed" | "insufficient"; factRefs: string[]; reason: string }> };
}
export interface AdministrativeClosureReceipt {
  scanId: string;
  attemptNumber: number;
  closureId: string;
  closedAt: string;
  previousStatus: string;
  snapshotHash: string;
  actor: "local_operator";
  executionState: "administratively_closed_unsettled";
  executionSettled: false;
  automaticReplayAllowed: false;
  requiresIndependentTask: true;
}

export interface AdministrativeClosurePreview {
  receipt: AdministrativeClosureReceipt | null;
  snapshotHash?: string;
  snapshot?: { schemaVersion: 1; scanId: string; attemptNumber: number; previousStatus: string;
    records: Array<{ table: string; rows: number; digest: string }> };
}

export interface ClosureHandoffReceipt {
  requestId: string; sourceScanId: string; closureId: string; sourceHash: string;
  scanId: string; createdAt: string; executionGranted: false; sourceExecutionSettled: false;
  scan: SentinelScan;
}
export interface ClosureHandoffPreview {
  sourceScanId: string; closureId: string; attemptNumber: number; snapshotHash: string;
  projectId: number; targetUrls: string[]; sourceHash: string;
  executionSettled: false; targetRequestsGranted: 0; savedHandoff: ClosureHandoffReceipt | null;
}
export interface ClosureHandoffInput {
  requestId: string; sourceScanId: string; closureId: string; sourceHash: string;
  taskName: string; urls: string[]; scanMode: string; maxBudgetUsd: number;
  authSessionIds: string[]; authSessionScopeId: string; skillIds: number[];
  instruction: string; closure: string; operatorConfirmed: boolean;
}

export interface NativeBudgetDimension {
  dimension: string;
  hardLimit: number | null;
  reserved: number; consumed: number; indeterminate: number;
  coverage: "assignment_settlement" | "multi_agent_broker" | "multi_agent_lane" | "race_batch_dispatch_unimplemented" | "admission_samples" | "denied_by_contract";
}

export interface NativeBudgetDiagnostics {
  schema: "summary_gap_v1"; authoritative: false; totalRoots: number; truncated: boolean;
  roots: Array<{ rootRunId: string; singleOriginalHistory?: {
    schemaVersion: 1; rootRunId: string;
    state: "verified_closed_history" | "not_verified";
    authoritative: false; executionAllowed: false; targetRequestsGranted: 0;
    basis: "original_runtime_records_not_provider_dollar_invoice";
  } | null; gaps: {
    ledgerExists: boolean; reservedTokens: number; spentTokens: number;
    reservedRequests: number; spentRequests: number;
    unsettledAssignmentTokens: number; unsettledAssignmentRequests: number;
    reservationTokenDelta: number | null; reservationRequestDelta: number | null;
    indeterminateInvocations: number; unkeyedInvocations: number;
    unclosedWebModelCalls?: number;
    coordinatorLeaseFound: boolean; fencingMatchesCoordinator: boolean;
    missingDimensions: string[];
    appendJournal?: NativeBudgetDimension[] | null;
    supervisionRequestCapacity?: {
      schemaVersion: 1; authoritative: false; executionAllowed: false;
      pendingPhysicalRequests: number; minimumFollowupModelRequests: number;
      nextHttpRequiredModelRequests: number;
      remainingSharedModelRequests: number | null;
      remainingSharedModelTokens: number | null;
      nextHttpRequestBalanceSufficient: boolean;
      freshWorkBlocked: "cost_reconciliation_required" | "wall_time_exhausted" | "elapsed_finalized" | null;
      futureTokenCapacityGuaranteed: false;
      usdCapacityCoverage: "not_enforced_by_this_ledger";
    } | null;
  } }>;
}

export interface RootHumanAssessmentObligation {
  rootRunId: string; callId: string; round: number; targetKey: string;
  humanDirective: HumanDirectiveDecisionContext;
  state: "cost_unconfirmed" | "assessment_unpublished" | "dispatch_unconfirmed" | "not_sent" | "awaiting_receipt";
  reportedUsage: { inputTokens: number; cachedInputTokens: number; outputTokens: number; totalTokens: number; modelRequests: number } | null;
  createdAt: string;
}
export interface RootHumanAssessmentSnapshot {
  schemaVersion: 1; executionAllowed: false; automaticResumeAllowed: false;
  items: RootHumanAssessmentObligation[]; truncated: boolean;
}

export interface NativeScanStatus {
  /** Original unpublished Human calls; readonly snapshot, separate from the timeline cursor. */
  humanAssessmentObligations?: RootHumanAssessmentSnapshot;
  directiveRecipients?: Array<{ rootRunId: string; targetKey: string; threadKey?: string }>;
  closureHandoff?: { source: ClosureHandoffReceipt | null; successor: ClosureHandoffReceipt | null };
  administrativeClosure?: AdministrativeClosureReceipt | null;
  manualAdministrativeClosureAvailable?: boolean;
  followup?: {
    source: GapFollowupPreview | null;
    tasks: Array<{ scanId: string; taskName: string; status: string; assessmentMessageId: string; review?: GapFollowupReview }>;
    gapResolved: boolean;
    review?: GapFollowupReview | null;
  };
  scanId: string;
  attemptNumber: number;
  status: string;
  /** Read-only count of earlier attempts with saved, unsettled assessment responses. */
  historicalPendingReceipts: number;
  /** Identity-only, unverified saved receipts from older attempts (latest 50). */
  historicalReceiptItems?: Array<{ directiveId: string; attemptNumber: number; createdAt: string; verified: false }>;
  orchestration: "native_pipeline_branches";
  schedulerActive: boolean;
  coordinatorLeaseValid: boolean;
  childRunsStarted: boolean;
  assignmentsRunning: boolean;
  mailboxConsumerActive: boolean;
  independentReviewerRunId: string;
  findingCandidateCount: number;
  reviewGateSatisfied: boolean;
  reviewStatus: "not_applicable" | "pending" | "running" | "confirmed" | "rejected" | "insufficient_evidence" | "failed" | "superseded" | "needs_evidence";
  multiAgentReady: boolean;
  branches: Array<{ branch: "source" | "web"; status: string; checkpoint: string; report: unknown; updatedAt: string;
    dispatch?: { state: "never_claimed" | "claimed" | "legacy_unknown" | "invalid_receipt"; claimedAt: string | null; automaticReplayAllowed: false; manualRecoveryAvailable?: boolean; manualClosureAvailable?: boolean } }>;
  sourceGaps: string[];
  unresolvedContainers: Array<{ receiptId: string; purpose: string; containerName: string; status: string; detail: string }>;
  targets?: Array<{ url: string; status: string; detail: string }>;
  llmRequests?: number;
  totalTokens?: number;
  stopDiagnostic: {
    category: "active" | "soft" | "hard" | "capability" | "natural";
    code: string;
    stage: "cleanup" | "branch" | "target" | "source" | "scan";
    nextAction: string;
    continuationConstraint: string;
    automaticResumeAllowed: false;
    obligations: Array<{ kind: string; reference: string; status: string }>;
    obligationsTruncated: boolean;
  };
  latestSequence: number;
  isIncremental: boolean;
  /** Bounded loaded window; older messages remain available through a read-only page. */
  timelineBeforeSequence?: number;
  hasEarlierTimeline?: boolean;
  timelineHasMore?: boolean;
  directiveDrafts: HumanDirectiveDraft[];
  timeline: AgentTimelineItem[];
}

export interface AgentTimelinePage {
  scanId: string;
  attemptNumber: number;
  beforeSequence: number;
  timelineBeforeSequence: number;
  hasEarlierTimeline: boolean;
  timeline: AgentTimelineItem[];
}

export interface GapFollowupPreview {
  sourceScanId: string;
  assessmentMessageId: string;
  projectId: number;
  targetUrl: string;
  rootRunId: string;
  assignmentId: string;
  candidateId: string;
  candidateRevision: number;
  missingEvidence: string[];
  proposal: { summary: string; prerequisites: string[] };
  sourceHash: string;
  targetRequestsGranted: 0;
}

export interface GapFollowupDraftInput {
  requestId: string;
  sourceScanId: string;
  assessmentMessageId: string;
  sourceHash: string;
  taskName: string;
  scanMode?: "quick" | "standard" | "deep";
  maxBudgetUsd?: number;
  authSessionIds: string[];
  authSessionScopeId: string;
  skillIds: number[];
  instruction: string;
  closure?: "breadth" | "proof";
}

export interface GapFollowupSubmission {
    input: GapFollowupDraftInput;
    scan: SentinelScan | null;
    createdScanId: string;
}

/** Local UI state, never an execution grant or a mailbox acknowledgement. */
export interface AgentDialogSelection {
  projectId: number | null;
  revision: number;
  selectedScanId: string | null;
  selectedScan: SentinelScan | null;
  selectionUnavailable: boolean;
}

export interface AgentDialogView {
  scanId: string;
  attemptNumber: number;
  revision: number;
  selectedThread: string;
  allReadSequence: number;
  threadReadSequences: Record<string, number>;
}

export interface AgentDialogViewInput {
  scanId: string;
  attemptNumber: number;
  expectedRevision: number;
  selectedThread: string;
  markReadThrough?: number;
  selectedThreadSequence?: number;
}

// Read-only original paid Root publication projection; no execution permission.
export interface HumanDirectiveDecisionContext {
  schemaVersion: 1;
  directiveId: string;
  draftId: string;
  revision: number;
  draftHash: string;
  confirmationReceiptId: string;
  threadKey: string;
  targetKey: string;
}

export interface RootDecisionRecord {
  humanDirective?: HumanDirectiveDecisionContext;
  schemaVersion: 1;
  advisoryOnly: true;
  callId: string;
  round: number;
  modelEventSequence: number;
  summary: { schemaVersion: 1; observed: string[]; missing: string[];
    suggestions: string[]; costNotes: string[]; risks: string[] };
  usage: { inputTokens: number; cachedInputTokens: number; outputTokens: number;
    totalTokens: number; modelRequests: 1 };
}

export interface AgentTimelineItem {
  humanReview?: HumanDirectiveReviewReceipt | null;
  orderedAssessmentExecution?: OrderedAssessmentExecution | null;
  decisionRecord?: RootDecisionRecord;
  id: string;
  sequence: number;
  timestamp: string;
  eventType: "directive_draft" | "user_directive" | "mailbox_message" | "agent_run" | "assignment" | "review_gate" | "host_boundary_candidate" | "request_review" | "administrative_closure" | "closure_handoff" | "root_decision" | "worker_report" | "homogeneous_guidance" | "homogeneous_decision";
  workerId?: string;
  sourceUserDirectiveId?: string;
  sliceId?: string;
  modelRequests?: number;
  totalTokens?: number;
  zeroIntent?: { reason: "covered_by_inflight" | "blocked_on_inflight_output"; evidence_assignment_ids: string[]; note_zh: string } | null;
  producedIntent?: boolean;
  fromRole: string;
  fromRunId: string;
  toRole: string;
  toRunId: string;
  messageKind: string;
  correlationId: string;
  threadKey?: string;
  assignmentId: string;
  evidenceRevision: number;
  deliveryState: string;
  ackState: string;
  status: string;
  summary: string;
  targetKey?: string;
  intent?: string;
  decision?: string;
  validationResult?: string;
  reasonCodes?: string[];
  confirmationRequired?: boolean;
  revision?: number;
  draftHash?: string;
  sideEffectClass?: string;
  requiredApprovals?: string[];
  sourceGuidance?: {
    phase: string;
    assignmentId: string;
    childRunId: string;
    eventSequence: number;
    inputHash: string;
    state: "model_received";
    advisoryOnly: true;
  } | null;
  taskClosure?: {
    fromStatus: string;
    disposition: "not_started" | "not_applied" | "outcome_unknown" | "receipt_pending" | "reconciliation_required" | "analysis_guidance_delivered";
    rootTerminalCode: string;
    requiresReconciliation: boolean;
    automaticRetry: false;
    closedAt: string;
  } | null;
  proposalAction?: {
    state: "prepared" | "executing" | "uncertain" | "received" | "completed" | "failed";
    assignmentId: string;
    childRunId: string;
    summary: string | null;
    errorCode: string;
    advisoryOnly: true;
    coverageVerified: false;
  } | null;
  localReconciliation?: {
    schemaVersion: 1;
    kind: "saved_assessment_receipt";
    status: "completed" | "failed";
    resultMessageId: string;
    assignmentId: string;
    childRunId: string;
    responseHash: string;
    usageHash: string;
    taskClosureHash: string;
    completedAt: string;
    modelRequests: 0;
    targetRequests: 0;
    advisoryOnly: true;
    coverageVerified: false;
  } | null;
  queueAction?: {
    kind: "prioritize_family";
    family: string;
    matchedItems: number;
    changedOrder: boolean;
    beforeQueueHash: string;
    afterQueueHash: string;
    targetRequests: 0;
    modelRequests: 0;
    coverageVerified: false;
  } | null;
  gapDetail?: {
    gapCode: string;
    nextStep: "observe_existing_evidence" | "request_new_contract" | "manual_review";
    sideEffectClass: "read_only";
    missingEvidence: string[];
    prerequisites: string[];
    supportingFactRefs: string[];
    proposedContracts: string[];
    estimatedCost: { modelTokens: number; modelRequests: number; targetRequests: number };
  };
  gapAssessment?: {
    decision: "deferred_requires_new_evidence_revision";
    reasonCode: "human_review_required" | "no_new_verified_fact_in_revision" | "proposal_is_not_a_verified_execution_contract" | "operator_approval_new_attempt_required";
    newAttemptRequired?: boolean;
    targetRequestsGranted?: 0;
  };
}

export interface AgentAttemptMailboxMessage {
  id: string;
  createdAt: string;
  fromRole: string;
  fromRunId: string;
  toRole: string;
  toRunId: string;
  kind: string;
  correlationId: string;
  assignmentId: string;
  evidenceRevision: number;
  deliveryState: string;
  ackState: string;
  summary: string;
}

export interface AgentAttemptMailboxPage {
  scanId: string;
  attemptNumber: number;
  messages: AgentAttemptMailboxMessage[];
  hasOlder: boolean;
  olderCursor: { createdAt: string; id: string } | null;
}

export interface AgentAttemptToolInvocation {
  id: string;
  origin: "web_invocation" | "source_receipt";
  recordedAt: string;
  timeBasis: "tool_started" | "model_response_received";
  receiptIntegrity: "not_checked" | "matched" | "pending" | "unverified";
  resultKind?: "control" | "data";
  invocationId: string;
  runId: string;
  role: string;
  toolName: string;
  status: string;
  policyDecision: string;
  startedAt: string;
  finishedAt: string;
  hasRequestArtifact: boolean;
  hasResponseArtifact: boolean;
  errorClass: string;
}

export interface AgentAttemptToolPage {
  schemaVersion: 2;
  scanId: string;
  attemptNumber: number;
  invocations: AgentAttemptToolInvocation[];
  hasOlder: boolean;
  olderCursor: string | null;
}

export interface OrderedAssessmentAction {
  actionId: string; order: number; role: "spa_api_mapper" | "deep_investigator";
  tokenCeiling: 4000; modelRequests: 1; maxOutputTokens: 512; targetRequests: 0;
  previousActionId: string | null; requiredPreviousState: "frozen_original_evidence" | "valid_advisory_receipt";
  advisoryOnly: true; executionState: "not_started";
}
export interface OrderedAssessmentPlan {
  schemaVersion: 2 | 3; purpose: "human_readonly_assessment"; bindingHash: string; planHash: string;
  dispatchState: "not_connected" | "requires_dispatch_checks"; totalTokenCeiling: 8000; totalModelRequests: 2; targetRequests: 0;
  actions: OrderedAssessmentAction[];
}

export interface OrderedAssessmentExecution {
  schemaVersion: 1; directiveId: string; sourceDraftId: string; draftHash: string; planHash: string;
  scanId: string; attemptNumber: number; rootRunId: string; targetKey: string; threadKey: string;
  state: string; completedAssessments: number; plannedAssessments: 2; advisoryOnly: true;
  coverageVerified: false; independentReviewApproved: false;
  actions: Array<{actionId: string; order: number; role: string; state: string; assignmentId: string | null;
    childRunId: string | null; receipt: {assessment: {summary: string; suggestions: string[]; limitations: string[]; valid: boolean};
      usage: {inputTokens: number; cachedInputTokens: number; outputTokens: number; totalTokens: number; modelRequests: 1};
      [key: string]: unknown} | null}>;
}

export interface HumanDirectiveDraft {
  id: string;
  sourceMessageId: string;
  scanId: string;
  attemptNumber: number;
  rootRunId: string;
  targetKey: string;
  recipientRole: string;
  threadKey: string;
  text: string;
  intent: string;
  requestedRoles: string[];
  referencedFactIds: string[];
  requestedContracts: string[];
  priorityChanges: string[];
  proposedScopeChange?: string;
  readonlyAssessmentPlan?: OrderedAssessmentPlan;
  estimatedTokens: number;
  estimatedRequests: number;
  sideEffectClass: "read_only" | "controlled_write" | "irreversible_blocked";
  requiredApprovals: string[];
  validationResult: "valid" | "confirmation_required" | "rejected";
  reasonCodes: string[];
  coordinatorDecision: "accept" | "partially_accept" | "defer" | "reject" | "need_confirmation";
  confirmationRequired: boolean;
  safeExecutionText: string;
  revision: number;
  draftHash: string;
  status: "drafted" | "need_confirmation" | "rejected" | "confirmed" | "cancelled" | "expired";
  confirmedDirectiveId: string;
}

export interface PostScanDirectiveResult {
  id: string;
  status: "pending" | "rejected";
  accepted: boolean;
  message: string;
}

export interface HumanDirectiveReviewReceipt {
  receiptId: string; sequence: number; createdAt: string;
  kind: "approve" | "revise" | "reject"; draftId: string; revision: number; draftHash: string;
  scanId: string; attemptNumber: number; rootRunId: string; targetKey: string; threadKey: string;
  argumentHash: string; reason: string; directiveId: string;
  successorDraftId: string | null; successorRevision: number | null; successorHash: string | null;
  requiresConfirmation: boolean; terminal: true; executionCompleted: false;
  actions: Array<{ order: number; role: string; intent: string;
    reviewDisposition: "coordinator_queued" | "not_queued"; capabilityState: "existing_policy_required" | "blocked";
    reasonCode: string; executionState: "not_started"; directiveId: string; executionReceipt: null }>;
}
export interface HumanDirectiveReviewResult {
  receipt: HumanDirectiveReviewReceipt;
  draft: HumanDirectiveDraft | null;
}
