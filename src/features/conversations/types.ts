export interface Conversation {
  id: string; projectId: number; title: string; titleSource?: "pending" | "ai" | "manual" | "legacy"; archived: boolean; revision: number;
  messageCount: number; lastMessagePreview: string; createdAt: string; updatedAt: string;
}
export interface ConversationMessage {
  id: string; conversationId: string; sequence: number; role: "user" | "assistant";
  content: string; replyToMessageId: string | null; createdAt: string;
}
export interface ConversationLink { kind: "scan" | "asset"; targetId: string; label: string; available: boolean }
export interface ReplyAvailability { available: boolean; code: string; message: string; model: string | null }
export interface ConversationReply {
  id: string; userMessageId: string;
  status: "queued" | "running" | "completed" | "unavailable" | "failed" | "cancel_requested" | "cancelled" | "interrupted";
  code: string; message: string; assistantMessageId: string | null; outcomeUnknown: boolean;
  createdAt: string; updatedAt: string;
}
export interface ConversationSnapshot {
  conversation: Conversation; messages: ConversationMessage[]; hasMore: boolean;
  links: ConversationLink[]; replyAvailability: ReplyAvailability; latestReply: ConversationReply | null;
  toolActivity?: ConversationToolActivity[];
}
export interface ConversationScope { projectId: number; conversationId: string }
export interface ConversationListInput { projectId: number; query?: string; includeArchived?: boolean; limit?: number; offset?: number }
export interface ConversationCreateInput { projectId: number; operationId: string; title?: string }
export interface ConversationReadInput extends ConversationScope { beforeSequence?: number; limit?: number }
export interface ConversationUpdateInput extends ConversationScope { expectedRevision: number; title?: string; archived?: boolean }
export interface ConversationSendInput extends ConversationScope { expectedRevision: number; operationId: string; content: string; requestReply: boolean }
export interface ConversationSendResult { conversation: Conversation; userMessage: ConversationMessage; reply: ConversationReply | null }

export interface ConversationLinksInput extends ConversationScope { expectedRevision: number; links: { kind: "scan" | "asset"; targetId: string }[] }

export interface ConversationToolActivity {
  id: string; replyId: string; userMessageId: string; round: number; callId: string;
  name: string; status: "completed" | "rejected"; summary: string; createdAt: string;
}
export interface ConversationAccess extends ConversationScope {
  directorySupported: boolean; agentEnabled: boolean; version: 1; revision: number; readAssets: boolean; readScans: boolean;
  selectedSkillIds: number[]; directories: { id: string; label: string; path: string }[];
  providerDisclosureAccepted: boolean; providerIdentity: string | null; providerLabel: string | null;
  skillsRequireReview: boolean; skillRevision: string;
}
export interface ConversationAccessSkill { id: number; name: string; description: string; enabled: boolean }
export interface ConversationAccessInput extends ConversationScope {
  expectedRevision: number; agentEnabled: boolean; readAssets: boolean; readScans: boolean;
  selectedSkillIds: number[]; providerDisclosureAccepted: boolean; providerIdentity: string | null; expectedSkillRevision: string;
}
export interface ConversationDirectoryGrantInput extends ConversationScope {
  expectedRevision: number; path: string; providerDisclosureAccepted: true; providerIdentity: string | null; expectedSkillRevision: string;
}
export interface ConversationDirectoryRevokeInput extends ConversationScope { expectedRevision: number; grantId: string }
