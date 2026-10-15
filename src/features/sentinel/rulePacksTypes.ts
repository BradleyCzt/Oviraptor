// Scanning owns rule repositories, synchronization and their change reports.
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

export interface SecurityRulePackInput {
  key: string;
  name: string;
  engine: string;
  repository: string;
  reference?: string;
  enabled: boolean;
}
