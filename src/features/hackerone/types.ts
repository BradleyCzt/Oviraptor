// Types owned by this module; cross-module consumers import this contract directly.
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
