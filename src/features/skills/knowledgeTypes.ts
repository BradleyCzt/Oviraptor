// Reusable knowledge and its review lifecycle retain source/project provenance.
// A scanId is provenance or an optional read filter, not scan execution authority.
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
