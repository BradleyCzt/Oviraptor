import { invoke } from "@tauri-apps/api/core";
import type { Conversation, ConversationAccess, ConversationAccessInput, ConversationAccessSkill, ConversationDirectoryGrantInput, ConversationDirectoryRevokeInput, ConversationCreateInput, ConversationListInput, ConversationReadInput, ConversationReply, ConversationScope, ConversationSendInput, ConversationSendResult, ConversationSnapshot, ConversationUpdateInput } from "./types";

// Standalone chat never calls the scan/directive API. All reads and writes bind
// both IDs so a conversation from another project cannot be selected implicitly.
export const conversationApi = {
  getConversationAccess: (input: ConversationScope) => invoke<ConversationAccess>("get_conversation_access", { input }),
  setConversationAccess: (input: ConversationAccessInput) => invoke<ConversationAccess>("set_conversation_access", { input }),
  listConversationAccessSkills: (input: ConversationScope) => invoke<ConversationAccessSkill[]>("list_conversation_access_skills", { input }),
  grantConversationDirectory: (input: ConversationDirectoryGrantInput) => invoke<ConversationAccess>("grant_conversation_directory", { input }),
  revokeConversationDirectory: (input: ConversationDirectoryRevokeInput) => invoke<ConversationAccess>("revoke_conversation_directory", { input }),
  listConversations: (input: ConversationListInput) => invoke<{ items: Conversation[]; total: number }>("list_conversations", { input }),
  createConversation: (input: ConversationCreateInput) => invoke<Conversation>("create_conversation", { input }),
  readConversation: (input: ConversationReadInput) => invoke<ConversationSnapshot>("read_conversation", { input }),
  updateConversation: (input: ConversationUpdateInput) => invoke<Conversation>("update_conversation", { input }),
  setConversationLinks: (input: ConversationScope & { expectedRevision: number; links: { kind: "scan" | "asset"; targetId: string }[] }) => invoke<Conversation>("set_conversation_links", { input }),
  sendConversationMessage: (input: ConversationSendInput) => invoke<ConversationSendResult>("send_conversation_message", { input }),
  cancelConversationReply: (input: ConversationScope & { replyId: string }) => invoke<ConversationReply>("cancel_conversation_reply", { input }),
  deleteConversation: (input: ConversationScope & { expectedRevision: number }) => invoke<void>("delete_conversation", { input }),
};
export type ConversationApi = typeof conversationApi;
