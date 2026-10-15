// Types owned by this module; cross-module consumers import this contract directly.
export interface Target {
  id: number;
  projectId: number;
  targetType: string;
  value: string;
  enabled: boolean;
  createdAt: string;
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

export interface InterruptedJob {
  runId: number;
  projectId: number;
  projectName: string;
  profileId?: number;
  name: string;
  pipeline: string;
  createdAt: string;
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
