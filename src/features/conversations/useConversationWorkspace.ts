import { computed, onUnmounted, ref, watch } from "vue";
import type { Project } from "../../types";
import type { ConversationApi } from "./api";
import type { Conversation, ConversationLinksInput, ConversationScope, ConversationSendInput, ConversationSnapshot } from "./types";

type SteerLocks = Pick<LockManager, "request">;
type SteerStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
export interface ConversationSteer extends ConversationScope {
  version: 1; operationId: string; replyId: string; content: string;
  status: "queued" | "paused" | "sending" | "uncertain";
  reason: string; input?: ConversationSendInput;
}
const steerKey = (projectId: number, id: string) => `oviraptor.conversation.steer.v1:${projectId}:${id}`;
const activeReplyStatuses = ["queued", "running", "cancel_requested"];
const validSteerText = (content: unknown): content is string => typeof content === "string" && !!content.trim() && !content.includes("\0") && new TextEncoder().encode(content).length <= 16_384;

export function useConversationWorkspace(deps: { project: () => Project | undefined; api: ConversationApi; operationId?: () => string; storage?: SteerStorage | null; locks?: SteerLocks | null }) {
  const conversations = ref<Conversation[]>([]);
  const total = ref(0);
  const includeArchived = ref(false);
  const query = ref("");
  const selectedId = ref("");
  const snapshot = ref<ConversationSnapshot>();
  const draft = ref("");
  // A running send retains its original retry draft. Steering has separate text.
  const steerDraft = ref("");
  const queuedSteer = ref<ConversationSteer>();
  const steering = ref(false);
  const steerSaving = ref(false);
  const steerSupported = computed(() => Boolean(deps.locks === undefined ? globalThis.navigator?.locks : deps.locks));
  const steerNotice = ref("");
  let autoSteerId: string | undefined;
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
  const replyActive = computed(() => Boolean(snapshot.value?.latestReply && activeReplyStatuses.includes(snapshot.value.latestReply.status)));
  const messageBytes = computed(() => new TextEncoder().encode(draft.value.trim()).length);
  const messageTooLong = computed(() => messageBytes.value > 16_384);
  const composerDraft = computed({
    get: () => !replyActive.value && pendingSend.value ? draft.value : replyActive.value || steerDraft.value ? steerDraft.value : draft.value,
    set: (value: string) => { if (!replyActive.value && pendingSend.value) draft.value = value; else if (replyActive.value || steerDraft.value) steerDraft.value = value; else draft.value = value; },
  });
  const composerBytes = computed(() => new TextEncoder().encode(composerDraft.value.trim()).length);
  const composerTooLong = computed(() => composerBytes.value > 16_384);
  const canSteer = computed(() => steerSupported.value && !steerSaving.value && writable.value && replyActive.value && !cancelling.value && snapshot.value?.latestReply?.status !== "cancel_requested" && !steering.value && !queuedSteer.value && !readError.value && validSteerText(steerDraft.value.trim()));
  const canSendSteer = computed(() => steerSupported.value && !steerSaving.value && !!queuedSteer.value && ["paused", "uncertain"].includes(queuedSteer.value.status) && writable.value && !replyActive.value && !busy.value && !pendingSend.value && !readError.value);
  const canSend = computed(() => !steerSaving.value && !composerTooLong.value && (!queuedSteer.value || !!pendingSend.value) && writable.value && !!snapshot.value && !busy.value && !replyActive.value && !readError.value && !!composerDraft.value.trim());
  const canDelete = computed(() => !steerSaving.value && !queuedSteer.value && !!snapshot.value && project.value?.status === "active" && !busy.value && !replyActive.value && !readError.value);
  const currentProject = (epoch: number, projectId: number) => !disposed && epoch === projectEpoch && project.value?.id === projectId;
  const currentSelection = (epoch: number, projectId: number, id: string) => !disposed && epoch === selectionEpoch && project.value?.id === projectId && selectedId.value === id;
  function stopPoll() { if (poll) clearTimeout(poll); poll = undefined; }
  function schedulePoll() {
    stopPoll();
    if (!disposed && (replyActive.value || steering.value || (busy.value && pendingSend.value))) poll = setTimeout(() => { void readSelected(false); }, 1500);
  }
  function storage(): SteerStorage {
    const value = deps.storage === undefined ? globalThis.localStorage : deps.storage;
    if (!value) throw Error("Steer requires local draft storage. Your text has not been queued");
    return value;
  }
  function readSteer(projectId: number, id: string): ConversationSteer | undefined {
    const raw = storage().getItem(steerKey(projectId, id));
    if (raw === null) return;
    if (raw.length > 150_000) throw Error("Stored Steer is invalid; it has not been sent");
    const value = JSON.parse(raw) as ConversationSteer;
    if (!value || value.version !== 1 || value.projectId !== projectId || value.conversationId !== id || !validSteerText(value.content) || typeof value.operationId !== "string" || !value.operationId || value.operationId.length > 200 || typeof value.replyId !== "string" || !value.replyId || !["queued", "paused", "sending", "uncertain"].includes(value.status) || typeof value.reason !== "string") throw Error("Stored Steer does not match this conversation; it has not been sent");
    const input = value.input;
    if (input && (input.projectId !== projectId || input.conversationId !== id || input.operationId !== value.operationId || input.content !== value.content || !Number.isSafeInteger(input.expectedRevision) || input.expectedRevision < 1 || typeof input.requestReply !== "boolean")) throw Error("Stored Steer request is invalid; it has not been sent");
    if ((value.status === "sending" || value.status === "uncertain") && !input) throw Error("Stored Steer request is incomplete; it has not been sent");
    return value;
  }
  function persistSteer(value: ConversationSteer, rejectedBeforeSave = false) {
    const saved = readSteer(value.projectId, value.conversationId);
    if (saved && (saved.operationId !== value.operationId || saved.content !== value.content || saved.input && !rejectedBeforeSave && JSON.stringify(saved.input) !== JSON.stringify(value.input))) throw Error("Steer changed in another window. Reopen the conversation to review it");
    const encoded = JSON.stringify(value), key = steerKey(value.projectId, value.conversationId);
    storage().setItem(key, encoded);
    if (storage().getItem(key) !== encoded) throw Error("Steer could not be saved locally. Your text has not been queued");
  }
  async function withSteerLock<T>(scope: ConversationScope, mutate: () => T): Promise<T> {
    const locks = deps.locks === undefined ? globalThis.navigator?.locks : deps.locks;
    if (!locks) throw Error("Steer requires safe local queue locking. Your text is kept; reopen this conversation in a supported window to review any saved direction");
    return locks.request(steerKey(scope.projectId, scope.conversationId), { mode: "exclusive", ifAvailable: true }, lock => {
      if (!lock) throw Error("This conversation’s Steer queue is busy in another window. Your text has been kept; try again");
      return mutate();
    });
  }
  async function pauseSteer(reason: string) {
    autoSteerId = undefined;
    const value = queuedSteer.value;
    if (!value || value.status !== "queued") return;
    const epoch = selectionEpoch;
    const paused: ConversationSteer = { ...value, status: "paused", reason };
    // Stop automatic permission synchronously, before awaiting storage admission.
    queuedSteer.value = paused;
    try { await withSteerLock(value, () => persistSteer(paused)); }
    catch (error) { if (currentSelection(epoch, value.projectId, value.conversationId)) actionError.value = String(error); }
  }
  async function restoreSteer(projectId: number, id: string) {
    autoSteerId = undefined;
    const epoch = selectionEpoch;
    steerSaving.value = true;
    try {
      // Ordinary chat must work without storage or Web Locks when no queue exists.
      if (deps.storage === undefined && typeof globalThis.localStorage === "undefined") return;
      const existing = readSteer(projectId, id);
      if (!existing) return;
      queuedSteer.value = { ...existing, status: existing.input ? "uncertain" : "paused", reason: existing.input ? "dispatch_unknown" : "reopened" };
      await withSteerLock(existing, () => {
        if (!currentSelection(epoch, projectId, id)) return;
        const value = readSteer(projectId, id);
        if (!value) { queuedSteer.value = undefined; return; }
        const restored: ConversationSteer = { ...value, status: value.input ? "uncertain" : "paused", reason: value.input ? "dispatch_unknown" : "reopened" };
        persistSteer(restored); queuedSteer.value = restored;
      });
    } catch (error) { if (currentSelection(epoch, projectId, id)) actionError.value = String(error); }
    finally { if (currentSelection(epoch, projectId, id)) steerSaving.value = false; }
  }
  async function queueSteer() {
    if (!steerSupported.value) { actionError.value = "Steer requires safe local queue locking. Your text is kept; reopen this conversation in a supported window to review any saved direction"; return; }
    if (!canSteer.value || !snapshot.value?.latestReply) return;
    const { projectId, id } = snapshot.value.conversation, epoch = selectionEpoch;
    const value: ConversationSteer = { version: 1, projectId, conversationId: id, operationId: newOperationId(), replyId: snapshot.value.latestReply.id, content: steerDraft.value.trim(), status: "queued", reason: "next_reply" };
    steerSaving.value = true;
    try {
      await withSteerLock(value, () => {
        if (!currentSelection(epoch, projectId, id)) return;
        if (!writable.value || !replyActive.value || cancelling.value || snapshot.value?.latestReply?.id !== value.replyId || snapshot.value.latestReply.status === "cancel_requested" || queuedSteer.value) throw Error("The reply changed before Steer was queued. Your text has been kept");
        // Read/check/write/verify and acceptance happen under one cross-window lock.
        persistSteer(value); queuedSteer.value = value; autoSteerId = value.operationId;
        if (steerDraft.value.trim() === value.content) steerDraft.value = "";
        steerNotice.value = ""; actionError.value = ""; schedulePoll();
      });
    } catch (error) { if (currentSelection(epoch, projectId, id)) actionError.value = String(error); }
    finally { if (currentSelection(epoch, projectId, id)) steerSaving.value = false; }
  }
  async function discardQueuedSteer() {
    const value = queuedSteer.value;
    if (!value || value.input || steering.value || steerSaving.value || !["queued", "paused"].includes(value.status)) return;
    const epoch = selectionEpoch;
    steerSaving.value = true;
    try {
      await withSteerLock(value, () => {
        if (!currentSelection(epoch, value.projectId, value.conversationId)) return;
        const target = replyActive.value || pendingSend.value || steerDraft.value ? steerDraft : draft;
        if (target.value && target.value !== value.content) throw Error("Finish or clear your current draft before moving Steer back to the composer");
        const saved = readSteer(value.projectId, value.conversationId);
        if (!saved || saved.operationId !== value.operationId || saved.input) throw Error("Steer changed. Reopen the conversation before changing it");
        const key = steerKey(value.projectId, value.conversationId);
        storage().removeItem(key);
        if (storage().getItem(key) !== null) throw Error("Steer could not be removed from local storage");
        queuedSteer.value = undefined; autoSteerId = undefined; steerNotice.value = "";
        target.value = value.content;
      });
    } catch (error) { if (currentSelection(epoch, value.projectId, value.conversationId)) actionError.value = String(error); }
    finally { if (currentSelection(epoch, value.projectId, value.conversationId)) steerSaving.value = false; }
  }
  async function dispatchSteer(automatic: boolean) {
    let value = queuedSteer.value;
    const current = snapshot.value;
    if (!value || !current || busy.value || pendingSend.value || replyActive.value || readError.value || !writable.value || steering.value || steerSaving.value) return;
    if (automatic && (value.status !== "queued" || value.operationId !== autoSteerId)) return;
    if (!automatic && !canSendSteer.value) return;
    const { projectId, conversationId: id } = value, epoch = selectionEpoch;
    if (current.conversation.id !== id || current.conversation.projectId !== projectId) return;
    steerSaving.value = true;
    try {
      const input = await withSteerLock(value, () => {
        if (!currentSelection(epoch, projectId, id) || !writable.value || replyActive.value || readError.value || busy.value || pendingSend.value) return;
        if (automatic && (autoSteerId !== value!.operationId || queuedSteer.value?.status !== "queued" || snapshot.value?.latestReply?.id !== value!.replyId || snapshot.value.latestReply.status !== "completed" || snapshot.value.latestReply.outcomeUnknown || !snapshot.value.replyAvailability.available)) return;
        const saved = readSteer(projectId, id);
        if (!saved || saved.operationId !== value!.operationId || saved.content !== value!.content) throw Error("Steer changed. Reopen the conversation to review it");
        if (automatic && (saved.status !== "queued" || saved.input)) {
          queuedSteer.value = { ...saved, status: saved.input ? "uncertain" : "paused", reason: "changed" }; autoSteerId = undefined; return;
        }
        value = saved;
        const payload = saved.input ?? { projectId, conversationId: id, expectedRevision: snapshot.value!.conversation.revision, operationId: saved.operationId, content: saved.content, requestReply: snapshot.value!.replyAvailability.available };
        const sending: ConversationSteer = { ...saved, status: "sending", reason: "dispatching", input: payload };
        // Exact idempotent payload is durable before any provider-capable IPC.
        persistSteer(sending); queuedSteer.value = sending; autoSteerId = undefined;
        return payload;
      });
      if (!input || !currentSelection(epoch, projectId, id)) return;
      // Lock admission and promise continuation are separate turns: recheck before IPC.
      // Keep the prewritten identity if state changed; another window may have seen it.
      if (!writable.value || replyActive.value || readError.value || busy.value || pendingSend.value || automatic && (snapshot.value?.latestReply?.id !== value.replyId || snapshot.value.latestReply.status !== "completed" || snapshot.value.latestReply.outcomeUnknown || !snapshot.value.replyAvailability.available)) {
        const paused: ConversationSteer = { ...value, input, status: "uncertain", reason: "changed" };
        queuedSteer.value = paused;
        await withSteerLock(value, () => persistSteer(paused));
        return;
      }
      const request = ++actionRequest;
      busy.value = true; steering.value = true; actionError.value = ""; schedulePoll();
      try {
        const result = await deps.api.sendConversationMessage({ ...input });
        if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
        if (result.conversation.projectId !== projectId || result.conversation.id !== id || result.userMessage.conversationId !== id || result.userMessage.content !== input.content || result.reply && result.reply.userMessageId !== result.userMessage.id) throw Error("Steer receipt does not match this conversation");
        await withSteerLock(value, () => {
          const savedNow = readSteer(projectId, id);
          if (!savedNow || savedNow.operationId !== input.operationId) throw Error("Steer receipt was saved, but its local draft changed. Reopen the conversation");
          storage().removeItem(steerKey(projectId, id));
          if (storage().getItem(steerKey(projectId, id)) !== null) throw Error("Steer was saved, but local recovery information could not be cleared");
        });
        if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
        queuedSteer.value = undefined;
        steerNotice.value = result.reply?.status === "completed" ? "reply_saved" : !input.requestReply ? "saved_only" : "message_saved";
        await readSelected(false);
        if (currentSelection(epoch, projectId, id) && request === actionRequest) void loadList();
      } catch (error) {
        if (currentSelection(epoch, projectId, id) && request === actionRequest) {
          const rejected = ["conversation_revision_conflict", "conversation_invalid_message", "conversation_reply_capacity"].includes(String(error).replace(/^Error:\s*/, ""));
          const recovery: ConversationSteer = rejected
            ? { ...value, status: "paused", reason: "changed", input: undefined }
            : { ...value, status: "uncertain", reason: "dispatch_unknown", input };
          queuedSteer.value = recovery;
          try { await withSteerLock(value, () => persistSteer(recovery, rejected)); } catch { /* Pre-dispatch identity remains recoverable. */ }
          if (!currentSelection(epoch, projectId, id) || request !== actionRequest) return;
          actionError.value = String(error); await readSelected(false);
        }
      } finally {
        if (currentSelection(epoch, projectId, id) && request === actionRequest) { busy.value = false; steering.value = false; schedulePoll(); }
      }
    } catch (error) { if (currentSelection(epoch, projectId, id)) { actionError.value = String(error); void pauseSteer("storage_error"); } }
    finally { if (currentSelection(epoch, projectId, id)) steerSaving.value = false; }
  }
  async function maybeDispatchSteer() {
    const value = queuedSteer.value, current = snapshot.value;
    if (!value || value.status !== "queued" || !current || value.operationId !== autoSteerId) return;
    const reply = current.latestReply;
    if (!writable.value || !reply || reply.id !== value.replyId) { pauseSteer("changed"); return; }
    if (reply.status === "cancel_requested") { pauseSteer("reply_stopped"); return; }
    if (!current.replyAvailability.available) { void pauseSteer("model_unavailable"); return; }
    if (replyActive.value) return;
    if (reply.status !== "completed" || reply.outcomeUnknown) { void pauseSteer("reply_stopped"); return; }
    if (busy.value || pendingSend.value || readError.value) return;
    await dispatchSteer(true);
  }
  async function sendQueuedSteer() { await dispatchSteer(false); }
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
      if (currentSelection(epoch, projectId, id) && request === readRequest) { loading.value = false; schedulePoll(); void maybeDispatchSteer(); }
    }
  }
  function selectConversation(id: string) {
    if (id === selectedId.value) return;
    pauseSteer("navigation");
    selectionEpoch++; actionRequest++; readRequest++;
    stopPoll(); selectedId.value = id; snapshot.value = undefined;
    draft.value = ""; steerDraft.value = ""; queuedSteer.value = undefined; steerNotice.value = ""; steering.value = false; steerSaving.value = false; titleDraft.value = ""; pendingSend.value = undefined; pendingDelete.value = undefined;
    readError.value = ""; actionError.value = ""; busy.value = false; cancelling.value = false; loading.value = false; loadingEarlier.value = false;
    if (id && project.value?.id) restoreSteer(project.value.id, id);
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
    if (!replyActive.value && !busy.value && !pendingSend.value && steerDraft.value) { draft.value = steerDraft.value; steerDraft.value = ""; }
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
        pauseSteer("original_send_unknown");
        actionError.value = String(error);
        // Backend guarantees this code is emitted before any message is saved.
        // Preserve the draft but allow a new explicit send against the new revision.
        if (["conversation_revision_conflict", "conversation_invalid_message"].includes(String(error).replace(/^Error:\s*/, ""))) pendingSend.value = undefined;
        await readSelected(false);
      }
    } finally { if (currentSelection(epoch, projectId, id) && request === actionRequest) { busy.value = false; schedulePoll(); void maybeDispatchSteer(); } }
  }
  async function updateConversation(change: { title?: string; archived?: boolean }) {
    const current = snapshot.value;
    if (!current || !project.value || project.value.status !== "active" || busy.value || replyActive.value || queuedSteer.value || readError.value) return;
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
    pauseSteer("reply_stopped");
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
    pauseSteer("navigation");
    projectEpoch++; selectionEpoch++; listRequest++; readRequest++; actionRequest++;
    stopPoll(); if (searchTimer) clearTimeout(searchTimer); searchTimer = undefined; conversations.value = []; total.value = 0; selectedId.value = ""; snapshot.value = undefined;
    draft.value = ""; steerDraft.value = ""; queuedSteer.value = undefined; steerNotice.value = ""; steering.value = false; steerSaving.value = false; titleDraft.value = ""; pendingCreate = undefined; pendingSend.value = undefined; pendingDelete.value = undefined;
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
  onUnmounted(() => { pauseSteer("reopened"); disposed = true; projectEpoch++; selectionEpoch++; actionRequest++; stopPoll(); if (searchTimer) clearTimeout(searchTimer); });
  return { steerSupported, steerSaving, steerDraft, composerDraft, composerBytes, composerTooLong, queuedSteer, steering, steerNotice, canSteer, canSendSteer, queueSteer, sendQueuedSteer, discardQueuedSteer, conversations, total, includeArchived, query, selectedId, snapshot, draft, titleDraft, loadingList, loading, loadingEarlier, busy, cancelling, listError, readError, actionError, hasListSnapshot, pendingSend, pendingDelete, project, writable, replyActive, messageBytes, messageTooLong, canSend, canDelete, requestDelete, cancelDelete, confirmDelete, loadList, readSelected, selectConversation, createConversation, send, updateConversation, setLinks, cancelReply, loadEarlier };
}
