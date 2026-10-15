import { invoke } from "@tauri-apps/api/core";
import type { AgentKnowledgeEntry, AgentLearningCandidate } from "../skills/knowledgeTypes";

/** Scanning produces reusable candidates from its retained traces and findings.
 * Review and application remain explicit operations of the knowledge catalog.
 */
export const scanningLearningApi = {
  generateAgentLearningCandidate: (scanId: string) =>
    invoke<AgentLearningCandidate>("generate_agent_learning_candidate", { scanId }),
  aggregateAgentKnowledge: (scanType: string) =>
    invoke<AgentKnowledgeEntry>("aggregate_agent_knowledge", { scanType }),
};
