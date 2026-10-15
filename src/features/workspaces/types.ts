// Types owned by this module; cross-module consumers import this contract directly.

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
