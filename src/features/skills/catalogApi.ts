import { invoke } from "@tauri-apps/api/core";
import type { AgentSkill, AgentKnowledgeEntry, AgentLearningCandidate, SecurityRulePack } from "./catalogTypes";

// The instruction/knowledge catalog is shared by Chat and Scanning; it owns its IPC contracts.
export const skillCatalogApi = {
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
};
