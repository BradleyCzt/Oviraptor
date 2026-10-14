// Types owned by this module; cross-module consumers import this contract directly.
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
