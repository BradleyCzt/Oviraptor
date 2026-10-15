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

export interface AgentSkillInput {
  id?: number;
  name: string;
  description: string;
  instructions: string;
  enabled: boolean;
}

export type { AgentKnowledgeEntry, AgentLearningCandidate } from "./knowledgeTypes";
