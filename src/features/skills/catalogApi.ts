import { invoke } from "@tauri-apps/api/core";
import type { AgentSkill, AgentSkillInput } from "./catalogTypes";
import type { AgentKnowledgeEntry, AgentLearningCandidate } from "./knowledgeTypes";

/** Saved instructions own their CRUD and portable text imports; no scan starts here. */
export const instructionCatalogApi = {
  listAgentSkills: () => invoke<AgentSkill[]>("list_agent_instructions"),
  saveAgentSkill: (input: AgentSkillInput) => invoke<number>("save_agent_instruction", { input }),
  deleteAgentSkill: (skillId: number) => invoke<void>("delete_agent_instruction", { skillId }),
  exportAgentSkills: () => invoke<string>("export_agent_instructions"),
  importAgentSkills: (path: string) => invoke<number>("import_agent_instructions", { path }),
  importSecSkillKnowledge: (path: string) =>
    invoke<Record<string, unknown>>("import_sec_skill_knowledge", { path }),
};

/** Knowledge retains provenance, review and explicit application to instructions.
 * scanId arguments here only filter stored knowledge/candidates. Producers that
 * read live scan traces belong to Scanning's learningApi contract.
 */
export const knowledgeCatalogApi = {
  ingestAgentKnowledgeSource: (source: string, forceRefresh = false) =>
    invoke<AgentKnowledgeEntry>("ingest_agent_knowledge_source", { source, forceRefresh }),
  listAgentKnowledge: (scanId?: string) => invoke<AgentKnowledgeEntry[]>("list_agent_knowledge", { scanId }),
  listAgentLearningCandidates: (status?: string, scanId?: string) =>
    invoke<AgentLearningCandidate[]>("list_agent_learning_candidates", { status, scanId }),
  reviewAgentLearningCandidate: (candidateId: number, decision: string, targetSkillId?: number) =>
    invoke<AgentLearningCandidate>("review_agent_learning_candidate", { candidateId, decision, targetSkillId }),
  applyAgentLearningCandidate: (candidateId: number) =>
    invoke<number>("apply_agent_learning_candidate", { candidateId }),
  deleteAgentLearningCandidate: (candidateId: number) =>
    invoke<void>("delete_agent_learning_candidate", { candidateId }),
  deleteAgentKnowledge: (knowledgeId: number) =>
    invoke<void>("delete_agent_knowledge", { knowledgeId }),
  convertAgentKnowledgeToInstruction: (knowledgeId: number) =>
    invoke<number>("convert_agent_knowledge_to_instruction", { knowledgeId }),
  refineAgentSkillWithKnowledge: (skillId: number) =>
    invoke<number>("refine_agent_instruction_with_knowledge", { skillId }),
  exportAgentKnowledge: () => invoke<string>("export_agent_knowledge"),
  importAgentKnowledge: (path: string) => invoke<number>("import_agent_knowledge", { path }),
};

// Compatibility for existing instruction/knowledge consumers. New consumers use
// the explicit lifecycle APIs above; Scanning producers and rule packs are absent.
export const skillCatalogApi = { ...instructionCatalogApi, ...knowledgeCatalogApi };
