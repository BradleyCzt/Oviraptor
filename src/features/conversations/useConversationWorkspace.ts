import { computed, onUnmounted, ref, watch } from "vue";
import type { Project } from "../../types";
import type { ConversationApi } from "./api";
import type { Conversation, ConversationLinksInput, ConversationScope, ConversationSendInput, ConversationSnapshot } from "./types";

export function useConversationWorkspace(deps: { project: () => Project | undefined; api: ConversationApi; operationId?: () => string }) {
  const conversations = ref<Conversation[]>([]);
  const total = ref(0);
  const includeArchived = ref(false);
  const query = ref("");
  const selectedId = ref("");
  const snapshot = ref<ConversationSnapshot>();
  const draft = ref("");
  const loadingList = ref(false);
  const loading = ref(false);
  const loadingEarlier = ref(false);
  const busy = ref(false);
  const cancelling = ref(false);
  const listError = ref("");
  const readError = ref("");
  const actionError = ref("");
  const hasListSnapshot = ref(false);
  const titleDraft = ref("");
  const pendingSend = ref<ConversationSendInput>();
  const pendingDelete = ref<(ConversationScope & { expectedRevision: number })>();
  let projectEpoch = 0, selectionEpoch = 0, listRequest = 0, readRequest = 0, actionRequest = 0;
  let disposed = false;
  let poll: ReturnType<typeof setTimeout> | undefined;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingCreate: { projectId: number; operationId: string } | undefined;
  const newOperationId = deps.operationId ?? (() => crypto.randomUUID());
  const project = computed(deps.project);
  const writable = computed(() => project.value?.status === "active" && !snapshot.value?.conversation.archived);
  const replyActive = computed(() => Boolean(snapshot.value?.latestReply && ["queued", "running", "cancel_requested"].includes(snapshot.value.latestReply.status)));
  const messageBytes = computed(() => new TextEncoder().encode(draft.value.trim()).length);
  const messageTooLong = computed(() => messageBytes.value > 16_384);
  const canSend = computed(() => !messageTooLong.value && writable.value && !!snapshot.value && !busy.value && !replyActive.value && !readError.value && !!draft.value.trim());
  const canDelete = computed(() => !!snapshot.value && project.value?.status === "active" && !busy.value && !replyActive.value && !readError.value);
  const currentProject = (epoch: number, projectId: number) => !disposed && epoch === projectEpoch && project.value?.id === projectId;
  const currentSelection = (epoch: number, projectId: number, id: string) => !disposed && epoch === selectionEpoch && project.value?.id === projectId && selectedId.value === id;
  function stopPoll() { if (poll) clearTimeout(poll); poll = undefined; }
  function schedulePoll() {
    stopPoll();
    if (!disposed && (replyActive.value || (busy.value && pendingSend.value))) poll = setTimeout(() => { void readSelected(false); }, 1500);
  }
  function validateSnapshot(value: ConversationSnapshot, projectId: number, id: string) {
    if (value.conversation.projectId !== projectId || value.conversation.id !== id || value.messages.some(message => message.conversationId !== id)) throw Error("Conversation response does not match the selected workspace");
  }
  async function loadList(append = false) {
    const projectId = project.value?.id;
    if (!projectId) return;
    const epoch = projectEpoch, request = ++listRequest;
    loadingList.value = true;
    listError.value = "";
    try {
      const value = await deps.api.listConversations({ projectId, query: query.value.trim(), includeArchived: includeArchived.value, limit: 50, offset: append ? conversations.value.length : 0 });
      if (!currentProject(epoch, projectId) || request !== listRequest) return;
      if (value.items.some(item => item.projectId !== projectId)) throw Error("Conversation list does not match the selected workspace");
      conversations.value = append ? [...conversations.value, ...value.items.filter(item => !conversations.value.some(old => old.id === item.id))] : value.items;
      total.value = value.total;
      hasListSnapshot.value = true;
    } catch (error) { if (currentProject(epoch, projectId) && request === listRequest) listError.value = String(error); }
    finally { if (currentProject(epoch, projectId) && request === listRequest) loadingList.value = false; }
  }
  async function readSelected(showLoading = true) {
    const projectId = project.value?.id, id = selectedId.value;
    if (!projectId || !id) return;
    const epoch = selectionEpoch, request = ++readRequest;
    if (showLoading) loading.value = true;
    try {
      const value = await deps.api.readConversation({ projectId, conversationId: id, limit: 100 });
      if (!currentSelection(epoch, projectId, id) || request !== readRequest) return;
      validateSnapshot(value, projectId, id);
      // Preserve older pages already loaded while polling the latest tail.
      const firstSequence = value.messages[0]?.sequence;
      const previous = snapshot.value?.messages ?? [];
      const overlaps = firstSequence !== undefined && previous.length > 0 && previous[previous.length - 1].sequence >= firstSequence - 1;
      const older = overlaps ? previous.filter(message => message.sequence < firstSequence!) : [];
      snapshot.value = { ...value, messages: [...older, ...value.messages], hasMore: older.length ? snapshot.value!.hasMore : value.hasMore };
      titleDraft.value = value.conversation.title;
      readError.value = "";
      const index = conversations.value.findIndex(item => item.id === id);
      if (index >= 0) conversations.value[index] = value.conversation;
    } catch (error) { if (currentSelection(epoch, projectId, id) && request === readRequest) readError.value = String(error); }
    finally {
      if (currentSelection(epoch, projectId, id) && request === readRequest) { loading.value = false; schedulePoll(); }
    }
  }
  function selectConversation(id: string) {
    if (id === selectedId.value) return;
    selectionEpoch++; actionRequest++; readRequest++;
    stopPoll(); selectedId.value = id; snapshot.value = undefined;
    draft.value = ""; titleDraft.value = ""; pendingSend.value = undefined; pendingDelete.value = undefined;
    readError.value = ""; actionError.value = ""; busy.value = false; cancelling.value = false; loading.value = false; loadingEarlier.value = false;
    void readSelected();
  }
  async function createConversation() {
    const projectId = project.value?.id;
    if (!projectId || project.value?.status !== "active" || busy.value) return;
    const epoch = selectionEpoch, request = ++actionRequest;
    busy.value = true; actionError.value = "";
    pendingCreate ??= { projectId, operationId: newOperationId() };
    try {
      const value = await deps.api.createConversation(pendingCreate);
      if (!currentSelection(epoch, projectId, selectedId.value) || request !== actionRequest) return;
      if (value.projectId !== projectId) throw Error("Created conversation does not match the selected workspace");
      pendingCreate = undefined;
      await loadList();
      if (!currentSelection(epoch, projectId, selectedId.value) || request !== actionRequest) return;
      selectConversation(value.id);
    } catch (error) { if (!disposed && epoch === selectionEpoch && request === actionRequest) actionError.value = String(error); }
    finally { if (!disposed && epoch === selectionEpoch && request === actionRequest) busy.value = false; }
  }
  async function send() {
    const current = snapshot.value;
    if (messageTooLong.value) { actionError.value = "Message exceeds 16,384 UTF-8 bytes"; return; }
    if (!current || !canSend.value) return;
    const projectId = current.conversation.projectId, id = current.conversation.id;
    const epoch = selectionEpoch, request = ++actionRequest;
    // A failed/uncertain request keeps its exact payload and idempotency key.
    // Resolve the original request before changing its payload; reads never dispatch a reply.
    const content = draft.value.trim();
    if (pendingSend.value && pendingSend.value.content !== content) {
      actionError.value = "Check the saved conversation and retry the original message before sending edited text";
      return;
    }
    pendingSend.value ??= { projectId, conversationId: id, expectedRevision: current.conversation.revision, operationId: newOperationId(), content, requestReply: current.replyAvailability.available };
    const input = { ...pendingSend.value };
    busy.value = true; actionError.value = ""; schedulePoll();
    try {
      const value = await deps.api.sendConversationMessage(input);
      if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
      if (value.conversation.projectId !== projectId || value.conversation.id !== id || value.userMessage.conversationId !== id) throw Error("Sent message does not match the selected workspace");
      draft.value = ""; pendingSend.value = undefined;
      await readSelected(false);
      if (currentSelection(epoch, projectId, id) && request === actionRequest) void loadList();
    } catch (error) {
      if (currentSelection(epoch, projectId, id) && request === actionRequest) {
        actionError.value = String(error);
        // Backend guarantees this code is emitted before any message is saved.
        // Preserve the draft but allow a new explicit send against the new revision.
        if (["conversation_revision_conflict", "conversation_invalid_message"].includes(String(error).replace(/^Error:\s*/, ""))) pendingSend.value = undefined;
        await readSelected(false);
      }
    } finally { if (currentSelection(epoch, projectId, id) && request === actionRequest) { busy.value = false; schedulePoll(); } }
  }
  async function updateConversation(change: { title?: string; archived?: boolean }) {
    const current = snapshot.value;
    if (!current || !project.value || project.value.status !== "active" || busy.value || replyActive.value || readError.value) return;
    const { projectId, id } = current.conversation;
    const epoch = selectionEpoch, request = ++actionRequest;
    busy.value = true; actionError.value = "";
    try {
      await deps.api.updateConversation({ projectId, conversationId: id, expectedRevision: current.conversation.revision, ...change });
      if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
      await readSelected(false);
      if (currentSelection(epoch, projectId, id) && request === actionRequest) void loadList();
    } catch (error) { if (currentSelection(epoch, projectId, id) && request === actionRequest) { actionError.value = String(error); await readSelected(false); } }
    finally { if (currentSelection(epoch, projectId, id) && request === actionRequest) busy.value = false; }
  }
  async function setLinks(input: ConversationLinksInput): Promise<boolean> {
    const current = snapshot.value;
    if (!current || !canDelete.value || !writable.value || pendingSend.value) return false;
    const { projectId, id, revision } = current.conversation;
    if (input.projectId !== projectId || input.conversationId !== id || project.value?.id !== projectId || input.expectedRevision !== revision) {
      actionError.value = "Conversation changed. Refresh and review linked records before saving again";
      return false;
    }
    const epoch = selectionEpoch, request = ++actionRequest;
    busy.value = true; actionError.value = "";
    try {
      const value = await deps.api.setConversationLinks({ ...input, links: input.links.map(link => ({ ...link })) });
      if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return false;
      if (value.projectId !== projectId || value.id !== id || value.revision !== revision + 1) throw Error("Linked-record response does not match this conversation");
      await readSelected(false);
      if (!currentSelection(epoch, projectId, id) || request !== actionRequest || readError.value) return false;
      void loadList();
      return true;
    } catch (error) {
      if (currentSelection(epoch, projectId, id) && request === actionRequest) { actionError.value = String(error); await readSelected(false); }
      return false;
    } finally { if (currentSelection(epoch, projectId, id) && request === actionRequest) busy.value = false; }
  }
  async function cancelReply() {
    const current = snapshot.value;
    if (cancelling.value || !current?.latestReply || !replyActive.value || current.latestReply.status === "cancel_requested" || project.value?.status !== "active") return;
    const { projectId, id } = current.conversation, epoch = selectionEpoch;
    cancelling.value = true;
    try {
      await deps.api.cancelConversationReply({ projectId, conversationId: id, replyId: current.latestReply.id });
      if (currentSelection(epoch, projectId, id)) await readSelected(false);
    } catch (error) { if (currentSelection(epoch, projectId, id)) actionError.value = String(error); }
    finally { if (currentSelection(epoch, projectId, id)) cancelling.value = false; }
  }
  function requestDelete() {
    if (!canDelete.value || !snapshot.value) return;
    const { id, projectId, revision } = snapshot.value.conversation;
    pendingDelete.value = { projectId, conversationId: id, expectedRevision: revision };
  }
  function cancelDelete() { if (!busy.value) pendingDelete.value = undefined; }
  async function confirmDelete() {
    const input = pendingDelete.value;
    if (!input || !canDelete.value || !snapshot.value) return;
    const { projectId, conversationId: id } = input;
    if (snapshot.value.conversation.id !== id || snapshot.value.conversation.projectId !== projectId || snapshot.value.conversation.revision !== input.expectedRevision) {
      actionError.value = "Conversation changed. Review its history before confirming deletion again";
      pendingDelete.value = undefined;
      return;
    }
    const epoch = selectionEpoch, request = ++actionRequest;
    busy.value = true; actionError.value = "";
    try {
      await deps.api.deleteConversation({ ...input });
      if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
      // A read issued before this commit must not resurrect its deleted row.
      listRequest++;
      const wasListed = conversations.value.some(item => item.id === id);
      conversations.value = conversations.value.filter(item => item.id !== id);
      if (wasListed) total.value = Math.max(conversations.value.length, total.value - 1);
      selectConversation("");
      void loadList();
    } catch (error) {
      if (currentSelection(epoch, projectId, id) && request === actionRequest) {
        actionError.value = String(error);
        pendingDelete.value = undefined;
        await readSelected(false);
      }
    } finally { if (currentSelection(epoch, projectId, id) && request === actionRequest) busy.value = false; }
  }
  async function loadEarlier() {
    const current = snapshot.value, projectId = project.value?.id, id = selectedId.value;
    if (!current || !projectId || !current.hasMore || loadingEarlier.value || !current.messages.length) return;
    const epoch = selectionEpoch, beforeSequence = current.messages[0].sequence;
    loadingEarlier.value = true;
    try {
      const value = await deps.api.readConversation({ projectId, conversationId: id, beforeSequence, limit: 100 });
      if (!currentSelection(epoch, projectId, id) || !snapshot.value || snapshot.value.messages[0]?.sequence !== beforeSequence) return;
      validateSnapshot(value, projectId, id);
      snapshot.value = { ...snapshot.value, messages: [...value.messages.filter(message => !snapshot.value!.messages.some(existing => existing.id === message.id)), ...snapshot.value.messages], hasMore: value.hasMore };
    } catch (error) { if (currentSelection(epoch, projectId, id)) readError.value = String(error); }
    finally { if (currentSelection(epoch, projectId, id)) loadingEarlier.value = false; }
  }
  watch(() => project.value?.id, () => {
    projectEpoch++; selectionEpoch++; listRequest++; readRequest++; actionRequest++;
    stopPoll(); if (searchTimer) clearTimeout(searchTimer); searchTimer = undefined; conversations.value = []; total.value = 0; selectedId.value = ""; snapshot.value = undefined;
    draft.value = ""; titleDraft.value = ""; pendingCreate = undefined; pendingSend.value = undefined; pendingDelete.value = undefined;
    listError.value = ""; readError.value = ""; actionError.value = "";
    loadingList.value = false; loading.value = false; loadingEarlier.value = false; busy.value = false; cancelling.value = false; hasListSnapshot.value = false;
    void loadList();
  }, { immediate: true, flush: "sync" });
  watch(includeArchived, () => { void loadList(); });
  watch(query, () => {
    // Invalidate immediately; an old search cannot land during the debounce.
    listRequest++;
    if (searchTimer) clearTimeout(searchTimer);
    conversations.value = []; total.value = 0; hasListSnapshot.value = false; listError.value = "";
    loadingList.value = Boolean(project.value);
    searchTimer = setTimeout(() => { searchTimer = undefined; void loadList(); }, 250);
  }, { flush: "sync" });
  onUnmounted(() => { disposed = true; projectEpoch++; selectionEpoch++; actionRequest++; stopPoll(); if (searchTimer) clearTimeout(searchTimer); });
  return { conversations, total, includeArchived, query, selectedId, snapshot, draft, titleDraft, loadingList, loading, loadingEarlier, busy, cancelling, listError, readError, actionError, hasListSnapshot, pendingSend, pendingDelete, project, writable, replyActive, messageBytes, messageTooLong, canSend, canDelete, requestDelete, cancelDelete, confirmDelete, loadList, readSelected, selectConversation, createConversation, send, updateConversation, setLinks, cancelReply, loadEarlier };
}
