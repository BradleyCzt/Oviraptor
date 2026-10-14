<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, watch } from "vue";
import { Archive, ArrowDown, ArrowUp, Ellipsis, MessageSquare, Plus, RefreshCw, Square, Trash2 } from "@lucide/vue";
import type { Project } from "../../types";
import type { BuiltinPluginHost } from "../../plugins";
const ConversationPluginPanel = defineAsyncComponent(() => import("./ConversationPluginPanel.vue"));
import ConversationMessage from "./ConversationMessage.vue";
import { useI18n } from "../../i18n";
import { conversationApi } from "./api";
import { useConversationWorkspace } from "./useConversationWorkspace";
const props = defineProps<{ projects: Project[]; projectId?: number; pluginHost?: BuiltinPluginHost; historyTarget?: string }>();
const emit = defineEmits<{ selectProject: [id: number]; createProject: []; historySelected: [] }>();
const { tr } = useI18n();
const selectedProject = computed(() => props.projects.find(project => project.id === props.projectId));
const state = useConversationWorkspace({ project: () => selectedProject.value, api: conversationApi });
const { conversations, total, includeArchived, query, selectedId, snapshot, composerDraft, composerBytes, composerTooLong, queuedSteer, steering, steerSupported, steerSaving, steerNotice, canSteer, canSendSteer, titleDraft, loadingList, loading, loadingEarlier, busy, cancelling, listError, readError, actionError, hasListSnapshot, pendingSend, pendingDelete, writable, replyActive, canSend, canDelete } = state;
const transcript = ref<HTMLElement>();
const composerInput = ref<HTMLTextAreaElement>();
const actionsMenu = ref<HTMLDetailsElement>();
const renameOpen = ref(false);
const archiveOpen = ref(false);
const followingTail = ref(true);
const unreadBelow = ref(false);
let forceFollow = true;
let selectionGeneration = 0;
let prependGeneration: number | undefined;
const activeProjects = computed(() => props.projects.filter(project => project.status === "active"));
const sendLabel = computed(() => busy.value ? tr("发送中…", "Sending…") : pendingSend.value ? tr("重试发送", "Retry send") : snapshot.value?.replyAvailability.available ? tr("发送", "Send") : tr("保存消息", "Save message"));
const replyStatus = computed(() => {
  const status = snapshot.value?.latestReply?.status;
  return status ? ({ queued: tr("等待回复", "Reply queued"), running: tr("正在回复…", "Replying…"), completed: tr("回复已保存", "Reply saved"), unavailable: tr("模型暂不可用", "Model unavailable"), failed: tr("回复失败", "Reply failed"), cancel_requested: tr("正在取消…", "Cancelling…"), cancelled: tr("回复已取消", "Reply cancelled"), interrupted: tr("回复已中断", "Reply interrupted") })[status] : "";
});
function closeActions(restoreFocus = false) { if (actionsMenu.value) { actionsMenu.value.open = false; if (restoreFocus) actionsMenu.value.querySelector<HTMLElement>("summary")?.focus(); } }
watch([selectedId, () => props.projectId], () => { selectionGeneration++; prependGeneration = undefined; renameOpen.value = false; archiveOpen.value = false; closeActions(); forceFollow = true; followingTail.value = true; unreadBelow.value = false; });
function observeScroll() {
  const element = transcript.value;
  if (!element || prependGeneration === selectionGeneration) return;
  followingTail.value = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
  if (followingTail.value) unreadBelow.value = false;
}
function jumpToLatest() {
  if (transcript.value) transcript.value.scrollTop = transcript.value.scrollHeight;
  followingTail.value = true;
  unreadBelow.value = false;
}
watch(() => snapshot.value?.messages[snapshot.value.messages.length - 1]?.id, async (id, previous) => {
  const generation = selectionGeneration;
  const follow = forceFollow || followingTail.value;
  await nextTick();
  if (generation !== selectionGeneration || prependGeneration === generation) return;
  if (follow) jumpToLatest();
  else if (id && previous && id !== previous) unreadBelow.value = true;
  if (id) forceFollow = false;
});
watch(composerDraft, async () => {
  await nextTick();
  if (composerInput.value) {
    composerInput.value.style.height = "auto";
    composerInput.value.style.height = `${Math.min(composerInput.value.scrollHeight, 160)}px`;
  }
});
async function loadEarlier() {
  const element = transcript.value;
  const id = selectedId.value;
  const generation = selectionGeneration;
  const oldHeight = element?.scrollHeight ?? 0;
  const oldTop = element?.scrollTop ?? 0;
  prependGeneration = generation;
  try {
    await state.loadEarlier();
    await nextTick();
    if (generation === selectionGeneration && element && element === transcript.value && id === selectedId.value) element.scrollTop = oldTop + element.scrollHeight - oldHeight;
  } finally { if (prependGeneration === generation) { prependGeneration = undefined; observeScroll(); } }
}
function chooseConversation(id: string) { state.selectConversation(id); emit("historySelected"); }
async function newConversation() { await state.createConversation(); if (!actionError.value) emit("historySelected"); }
async function rename() {
  if (!titleDraft.value.trim()) return;
  await state.updateConversation({ title: titleDraft.value.trim() });
  if (!actionError.value) renameOpen.value = false;
}
async function archive() {
  if (!snapshot.value) return;
  await state.updateConversation({ archived: !snapshot.value.conversation.archived });
  if (!actionError.value) archiveOpen.value = false;
}
const steerReason = computed(() => {
  const reasons: Record<string, string> = {
    next_reply: tr("已排队用于下一次回复，当前回复不会改变。", "Queued for the next reply. The current reply won’t change."),
    reopened: tr("重新打开后已暂停，请确认后发送。", "Paused after reopening. Review and send when ready."),
    original_send_unknown: tr("原消息的发送结果尚不确定。请先核实或重试原消息。", "The original message delivery is uncertain. Resolve its retry before sending this direction."),
    dispatch_unknown: tr("发送结果尚不确定。重试会使用相同请求，不会自动重发。", "Delivery is uncertain. Retry reuses the original request; it will not send automatically."),
    dispatching: tr("正在发送已排队的指引…", "Sending your queued direction…"),
    navigation: tr("离开对话后已暂停，请确认后发送。", "Paused when you left this conversation. Review before sending."),
    changed: tr("对话已变化，请确认后发送。", "The conversation changed. Review before sending."),
    reply_stopped: tr("回复已停止，排队的指引已保留。", "The reply stopped. Your queued direction is kept."),
    model_unavailable: tr("没有可用模型，可仅保存此消息。", "No model is available. You can save this message without a reply."),
    storage_error: tr("本地队列未能更新，文本已保留。", "The local queue could not be updated. Your text is kept."),
  };
  return reasons[queuedSteer.value?.reason ?? ""] || "";
});
const steerReceipt = computed(() => steerNotice.value === "reply_saved" ? tr("指引的回复已保存", "Steer reply saved") : steerNotice.value === "saved_only" ? tr("指引已保存，未请求模型回复", "Steer saved without a model reply") : steerNotice.value === "message_saved" ? tr("指引消息已保存", "Steer message saved") : "");
async function submitComposer() { if (replyActive.value) await state.queueSteer(); else await state.send(); }
function submit(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); void submitComposer(); }
}
</script>
<template>
  <section class="conversation-workspace" :class="{'has-local-history':!historyTarget&&selectedProject}" :aria-label="tr('独立对话工作区','Independent chat workspace')">
    <Teleport :to="historyTarget||'body'" :disabled="!historyTarget">
      <aside v-if="selectedProject" class="conversation-list" :aria-label="tr('对话','Conversations')">
        <button class="conversation-new-chat" :disabled="busy||steerSaving||selectedProject.status!=='active'" @click="newConversation"><Plus :size="18"/>{{tr('新对话','New chat')}}</button>
        <input v-model="query" class="conversation-search" type="search" :aria-label="tr('搜索当前工作空间的对话','Search conversations in this workspace')" :placeholder="tr('搜索对话…','Search conversations…')" maxlength="200"/>
        <div class="conversation-history-heading"><span>{{tr('最近对话','Recent chats')}}</span><details class="history-filters"><summary :aria-label="tr('历史记录选项','History options')"><Ellipsis :size="16"/></summary><label class="conversation-archive-filter"><input v-model="includeArchived" type="checkbox"/>{{tr('显示已归档','Show archived')}}</label></details></div>
        <p v-if="listError" class="conversation-error" role="alert">{{listError}} <button class="text-button" @click="state.loadList()">{{tr('重试','Retry')}}</button></p>
        <p v-if="loadingList" class="conversation-muted" role="status">{{tr('正在读取对话…','Loading conversations…')}}</p>
        <p v-else-if="hasListSnapshot&&!conversations.length&&!listError" class="conversation-muted">{{query.trim()?tr('没有匹配的对话','No matching conversations'):tr('新想法，从这里开始','Your next idea starts here')}}</p>
        <div class="conversation-history-items">
          <button v-for="conversation in conversations" :key="conversation.id" class="conversation-row" :class="{selected:selectedId===conversation.id}" :aria-current="selectedId===conversation.id?'page':undefined" :title="conversation.title" @click="chooseConversation(conversation.id)"><MessageSquare :size="15"/><span><strong>{{conversation.title}}</strong><small v-if="conversation.archived">{{tr('已归档','Archived')}}</small></span></button>
          <button v-if="conversations.length<total" class="button ghost compact" :disabled="loadingList" @click="state.loadList(true)">{{tr('更多对话','More conversations')}}</button>
        </div>
      </aside>
    </Teleport>
    <div v-if="!selectedProject" class="conversation-welcome">
      <div class="conversation-welcome-mark"><MessageSquare :size="30"/></div>
      <p class="conversation-eyebrow">OVIRAPTOR</p>
      <h2>{{tr('从一个好问题开始','Start with a good question')}}</h2>
      <p>{{tr('一个安静思考、讨论和保留想法的地方。选择工作空间，即可开始对话。','A little space to think, discuss, and keep your ideas. Choose a workspace to begin.')}}</p>
      <template v-if="projects.length">
        <p>{{tr('选择已有工作空间继续','Choose an existing workspace to continue')}}</p>
        <div class="conversation-projects"><button v-for="project in projects" :key="project.id" class="button ghost" @click="emit('selectProject',project.id)">{{project.name}}<small v-if="project.status==='archived'">{{tr('已归档 · 只读','Archived · read only')}}</small></button></div>
        <button v-if="!activeProjects.length" class="button ghost" @click="emit('createProject')"><Plus :size="15"/>{{tr('新建工作空间','New workspace')}}</button>
      </template>
      <template v-else><button class="button primary" @click="emit('createProject')"><Plus :size="16"/>{{tr('创建工作空间','Create workspace')}}</button><small class="conversation-muted">{{tr('对话、资产和扫描共享同一个工作空间。','One workspace for conversations, assets, and scans.')}}</small></template>
    </div>
    <div v-else class="conversation-main">
      <header class="conversation-header">
        <div class="conversation-heading"><h2>{{snapshot?.conversation.title||tr('新想法，新对话','A fresh conversation')}}</h2><span>{{snapshot?.replyAvailability.model||tr('本地对话','Local conversations')}}</span></div>
        <div class="conversation-header-actions">
          <button class="chat-icon-button" :disabled="busy||steerSaving||selectedProject.status!=='active'" :aria-label="tr('新对话','New chat')" :title="tr('新对话','New chat')" @click="newConversation"><Plus :size="18"/></button>
          <details v-if="snapshot" ref="actionsMenu" class="conversation-actions" @keydown.esc.stop="closeActions(true)">
            <summary class="chat-icon-button" :aria-label="tr('对话选项','Conversation options')" :title="tr('对话选项','Conversation options')"><Ellipsis :size="20"/></summary>
            <div class="conversation-action-menu">
              <button :disabled="loading" :title="tr('刷新历史','Refresh history')" @click="state.readSelected();closeActions()"><RefreshCw :size="15"/>{{tr('刷新历史','Refresh history')}}</button>
              <button :disabled="busy||steerSaving||replyActive||!!queuedSteer||!writable||!!readError" @click="renameOpen=!renameOpen;closeActions()">{{tr('重命名','Rename')}}</button>
              <button :disabled="busy||steerSaving||replyActive||!!queuedSteer||selectedProject.status!=='active'||!!readError" @click="archiveOpen=true;closeActions()"><Archive :size="15"/>{{snapshot.conversation.archived?tr('恢复','Restore'):tr('归档','Archive')}}</button>
              <button :disabled="!canDelete" class="destructive" @click="state.requestDelete();closeActions()"><Trash2 :size="15"/>{{tr('删除','Delete')}}</button>
            </div>
          </details>
        </div>
      </header>
      <div class="conversation-alerts">
        <p v-if="selectedProject.status!=='active'" class="conversation-notice">{{tr('该工作空间已归档，对话历史保持只读。可在项目管理中恢复工作空间。','This workspace is archived. Conversation history is read-only; restore the workspace from Projects to continue.')}}</p>
        <p v-if="actionError" class="conversation-error" role="alert">{{actionError}}</p>
        <p v-if="readError" class="conversation-error" role="alert">{{tr('读取失败，当前内容可能不是最新状态。','Read failed; the displayed history may be stale.')}} {{readError}} <button class="text-button" @click="state.readSelected()">{{tr('重试读取','Retry read')}}</button></p>
        <p v-if="loading" class="conversation-muted" role="status">{{tr('正在读取历史…','Loading history…')}}</p>
        <div v-if="pendingDelete" class="conversation-delete-confirm" role="alert"><div><strong>{{tr('删除这个对话？','Delete this conversation?')}}</strong><p>{{tr('将永久删除此对话和消息，无法恢复。项目、资产、扫描和证据保持不变。','This permanently deletes the conversation and its messages. It cannot be undone. Projects, assets, scans, and evidence are kept.')}}</p></div><button class="button ghost compact" :disabled="busy" @click="state.cancelDelete()">{{tr('取消','Cancel')}}</button><button class="button danger compact" :disabled="!canDelete" @click="state.confirmDelete()">{{busy?tr('删除中…','Deleting…'):tr('永久删除对话','Delete conversation permanently')}}</button></div>
        <div v-if="archiveOpen&&snapshot" class="conversation-delete-confirm"><div><strong>{{snapshot.conversation.archived?tr('恢复这个对话？','Restore this conversation?'):tr('归档这个对话？','Archive this conversation?')}}</strong><p>{{tr('消息会保留，可随时从历史选项中查看已归档的对话。','Messages are kept. Archived conversations are available from History options.')}}</p></div><button class="button ghost compact" :disabled="busy" @click="archiveOpen=false">{{tr('取消','Cancel')}}</button><button class="button primary compact" :disabled="busy||steerSaving||replyActive||!!queuedSteer||selectedProject.status!=='active'||!!readError" @click="archive">{{snapshot.conversation.archived?tr('恢复','Restore'):tr('归档','Archive')}}</button></div>
        <form v-if="renameOpen" class="conversation-rename" @submit.prevent="rename"><input v-model="titleDraft" :aria-label="tr('对话标题','Conversation title')" maxlength="160"/><button class="button secondary compact" :disabled="busy||steerSaving||!titleDraft.trim()">{{tr('保存','Save')}}</button><button type="button" class="button ghost compact" @click="renameOpen=false">{{tr('取消','Cancel')}}</button></form>
      </div>
      <template v-if="snapshot">
        <div class="conversation-reading-area" :class="{'has-queued-steer':queuedSteer}">
          <div ref="transcript" class="conversation-transcript" role="log" :aria-label="tr('对话历史','Conversation history')" aria-live="polite" aria-relevant="additions" tabindex="0" @scroll="observeScroll">
            <div class="conversation-message-column">
              <button v-if="snapshot.hasMore" class="button ghost compact conversation-earlier" :disabled="loadingEarlier" @click="loadEarlier">{{loadingEarlier?tr('读取中…','Loading…'):tr('更早的消息','Earlier messages')}}</button>
              <div v-if="!snapshot.messages.length" class="conversation-empty"><div class="conversation-welcome-mark"><MessageSquare :size="30"/></div><h3>{{tr('有什么想讨论的？','What’s on your mind?')}}</h3><p>{{tr('提一个问题，探索一个想法，或者一起理清思路。','Ask a question, explore an idea, or work through a thought.')}}</p></div>
              <ConversationMessage v-for="message in snapshot.messages" :key="message.id" :message="message"/>
              <div v-if="replyActive" class="conversation-pending" role="status"><span class="reply-pulse" aria-hidden="true"></span><span>{{replyStatus}}<small>{{tr('完成后会显示完整回复','The complete reply will appear when ready')}}</small></span></div>
              <ConversationPluginPanel v-if="pluginHost" :host="pluginHost" :project-id="selectedProject.id" :project-active="selectedProject.status==='active'" :snapshot="snapshot" :blocked="busy||steerSaving||replyActive||!!queuedSteer||!!readError||!!pendingSend" :save-links="state.setLinks"/>
            </div>
          </div>
          <button v-if="!followingTail&&snapshot.messages.length" class="conversation-jump" @click="jumpToLatest"><ArrowDown :size="15"/>{{unreadBelow?tr('查看新消息','New messages'):tr('回到最新','Back to latest')}}</button>
          <div v-if="queuedSteer" class="conversation-steer-queue" role="status">
            <header><strong>{{queuedSteer.status==='uncertain'?tr('指引发送待核实','Steer needs review'):queuedSteer.status==='paused'?tr('指引已暂停','Steer paused'):tr('下一条指引','Next direction')}}</strong><span>{{queuedSteer.status==='queued'?tr('已排队','Queued'):queuedSteer.status==='sending'?tr('发送中','Sending'):''}}</span></header>
            <p class="steer-content">{{queuedSteer.content}}</p><p class="steer-reason">{{steerReason}}</p>
            <div class="steer-queue-actions"><button v-if="!queuedSteer.input&&['queued','paused'].includes(queuedSteer.status)" type="button" class="text-button" :disabled="steering||steerSaving||!steerSupported" @click="state.discardQueuedSteer()">{{tr('移回输入框编辑','Move back to composer')}}</button><button v-if="['paused','uncertain'].includes(queuedSteer.status)" type="button" class="button ghost compact" :disabled="!canSendSteer" @click="state.sendQueuedSteer()">{{queuedSteer.status==='uncertain'?tr('重试指引','Retry steer'):snapshot.replyAvailability.available?tr('发送指引','Send steer'):tr('保存指引','Save steer')}}</button></div>
          </div>
        </div>
        <div class="conversation-compose-area">
          <p v-if="!queuedSteer&&steerReceipt" class="conversation-reply-status" role="status">{{steerReceipt}}</p>
          <div v-if="snapshot.latestReply&&!replyActive" class="conversation-reply-status" role="status"><span>{{replyStatus}}</span><small v-if="snapshot.latestReply.message">{{snapshot.latestReply.message}}</small><small v-if="snapshot.latestReply.outcomeUnknown">{{tr('提供商可能已处理此次请求；不会自动重发。','The provider may have processed this request. It will not be retried automatically.')}}</small></div>
          <p v-if="snapshot.conversation.archived" class="conversation-notice">{{tr('该对话已归档，恢复后可以继续。','This conversation is archived. Restore it to continue.')}}</p>
          <form class="conversation-composer" @submit.prevent="submitComposer">
            <textarea ref="composerInput" v-model="composerDraft" :disabled="!writable||(busy&&!replyActive)||steering" :placeholder="snapshot.conversation.archived?tr('已归档的对话','Archived conversation'):replyActive?tr('补充下一次回复的方向…','Add direction for the next reply…'):tr('输入消息…','Message Oviraptor…')" :aria-label="tr('消息','Message')" :aria-describedby="!steerSupported&&(replyActive||queuedSteer)?'composer-guidance steer-support-notice':'composer-guidance'" rows="2" maxlength="16384" @keydown="submit"/>
            <footer>
              <span class="composer-mode"><MessageSquare :size="14"/>{{snapshot.replyAvailability.available?tr('对话','Chat'):tr('仅保存','Save only')}}</span>
              <small id="composer-guidance" v-if="composerTooLong" class="conversation-error">{{tr(`消息为 ${composerBytes.toLocaleString()} 字节，最多 16,384 字节，请缩短后发送。`,`Message is ${composerBytes.toLocaleString()} bytes; shorten it to 16,384 bytes or less.`)}}</small>
              <small id="composer-guidance" v-else>{{replyActive?tr('Enter 排队下一条 · Shift+Enter 换行','Enter to queue next reply · Shift+Enter for a new line'):pendingSend?tr('重试使用相同请求，不会自动重复发送','Retry reuses the same request'):tr('Enter 发送 · Shift+Enter 换行','Enter to send · Shift+Enter for a new line')}}</small>
              <button v-if="replyActive" type="submit" class="composer-steer" :disabled="!canSteer" :title="tr('排队在当前回复完成后发送，不改变正在进行的回复','Queue for after the current reply; does not change the reply in progress')">{{tr('Steer · 下一轮补充','Steer · Next reply')}}</button>
              <button v-if="replyActive" type="button" class="composer-submit is-stop" :disabled="cancelling||snapshot.latestReply?.status==='cancel_requested'||selectedProject.status!=='active'" :aria-label="tr('停止回复','Stop reply')" :title="tr('停止回复','Stop reply')" @click="state.cancelReply()"><Square :size="15" fill="currentColor"/></button>
              <button v-else type="submit" class="composer-submit" :disabled="!canSend" :aria-label="sendLabel" :title="sendLabel"><ArrowUp :size="20"/></button>
            </footer>
          </form>
          <p v-if="!steerSupported&&(replyActive||queuedSteer)" id="steer-support-notice" class="conversation-model-notice" role="status">{{queuedSteer?tr('请在支持安全本地队列的窗口中恢复此指引，再继续对话。','Reopen in a window with safe local queue support to review this saved direction before continuing.'):tr('Steer 暂不可用：此环境不支持安全本地队列；普通聊天仍可使用。','Steer needs safe local queue support. Ordinary chat still works.')}}</p>
          <p v-else-if="steerSaving" class="conversation-model-notice" role="status">{{tr('正在安全保存指引…','Saving direction safely…')}}</p>
          <p v-if="!snapshot.replyAvailability.available" class="conversation-model-notice">{{snapshot.replyAvailability.message||tr('尚未配置可用模型；消息仍可保存，助手不会生成回复。','No model is available. Messages can still be saved without an assistant reply.')}}</p>
          <p class="conversation-privacy"><span>{{tr('普通对话 · 不执行工具或扫描','Conversation · no tool or scan execution')}}</span> · {{tr('消息保存在本地 · 请求回复时发送给已配置的模型提供商','Stored locally · Replies send conversation text to your configured model provider')}}</p>
        </div>
      </template>
      <div v-else-if="!selectedId&&!loading" class="conversation-welcome">
        <div class="conversation-welcome-mark"><MessageSquare :size="30"/></div><p class="conversation-eyebrow">OVIRAPTOR</p><h2>{{tr('今天想探索什么？','What would you like to explore?')}}</h2><p>{{tr('从一个问题或想法开始。对话会保存在当前工作空间，无需创建扫描任务。','Start with a question or an idea. Your conversations stay in this workspace.')}}</p><button class="button primary" :disabled="busy||steerSaving||selectedProject.status!=='active'" @click="newConversation"><Plus :size="17"/>{{tr('新对话','New chat')}}</button>
        <p class="conversation-welcome-note">{{tr('使用侧边栏查找历史对话','Find your past conversations in the sidebar')}}</p>
      </div>
    </div>
  </section>
</template>
<style scoped>
.conversation-workspace{display:flex;flex:1;min-height:0;min-width:0;height:100%;color:var(--app-ink,#172033)}
.conversation-workspace.has-local-history{gap:20px}.has-local-history>.conversation-list{width:250px;flex-shrink:0}
.conversation-welcome{display:flex;flex:1;min-height:0;overflow:auto;flex-direction:column;align-items:center;justify-content:center;gap:16px;padding:40px 24px;text-align:center;max-width:760px;margin:0 auto}
.conversation-welcome-mark{width:58px;height:58px;display:grid;place-items:center;background:color-mix(in srgb,var(--app-ink,#172033) 5%,transparent);border:1px solid var(--line);border-radius:18px;color:var(--app-ink,#172033)}
.conversation-welcome h2{font-size:clamp(24px,3vw,34px);font-weight:550;letter-spacing:-1px;margin:0}.conversation-welcome p{max-width:480px;color:var(--app-muted,var(--muted));font-size:14px;line-height:1.8;margin:0}.conversation-welcome p.conversation-eyebrow{font-size:10px;letter-spacing:2px}.conversation-welcome p.conversation-welcome-note{font-size:12px;margin-top:12px}.conversation-welcome .button{margin-top:8px}
.conversation-projects{display:flex;flex-wrap:wrap;gap:10px;justify-content:center}.conversation-projects small{font-size:10px}
.conversation-list{display:flex;flex-direction:column;min-height:0;height:100%;color:var(--app-ink,#172033)}
.conversation-new-chat{display:flex;align-items:center;gap:9px;flex-shrink:0;width:100%;padding:11px 12px;border:1px solid var(--line);border-radius:10px;background:var(--app-surface,#fff);color:inherit;font-size:13px;font-weight:550;margin-bottom:12px}
.conversation-search{flex-shrink:0;width:100%;height:36px;border:1px solid transparent;border-radius:8px;padding:0 11px;background:color-mix(in srgb,var(--app-ink,#172033) 4%,transparent);color:inherit;font-size:12px}
.conversation-history-heading{display:flex;align-items:center;justify-content:space-between;margin:22px 8px 8px;color:var(--app-muted,var(--muted));font-size:11px}.history-filters{position:relative}.history-filters summary{display:flex;padding:2px;cursor:pointer;list-style:none}.history-filters summary::-webkit-details-marker{display:none}.conversation-archive-filter{position:absolute;right:-5px;top:25px;z-index:3;display:flex;align-items:center;gap:6px;white-space:nowrap;font-size:12px;padding:12px;background:var(--app-surface,#fff);border:1px solid var(--line);border-radius:9px;box-shadow:0 8px 24px #0002}
.conversation-history-items{min-height:0;overflow:auto;flex:1;scrollbar-width:thin;padding-bottom:8px}.conversation-row{display:flex;align-items:center;gap:10px;padding:11px 10px;width:100%;text-align:left;border:0;border-radius:8px;background:transparent;color:inherit;margin-bottom:2px}.conversation-row>svg{flex-shrink:0;color:var(--app-muted,var(--muted))}.conversation-row>span{min-width:0;display:flex;flex-direction:column;gap:4px}.conversation-row:hover,.conversation-row.selected{background:color-mix(in srgb,var(--app-ink,#172033) 7%,transparent)}.conversation-row strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;font-weight:450}.conversation-row small{font-size:10px;color:var(--app-muted,var(--muted))}
.conversation-main{flex:1;min-width:0;min-height:0;display:flex;flex-direction:column;width:100%;position:relative}
.conversation-header{display:flex;justify-content:space-between;align-items:center;flex-shrink:0;gap:16px;padding:14px 28px;min-height:66px}.conversation-heading{min-width:0}.conversation-heading h2{margin:0;font-weight:550;font-size:14px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.conversation-heading>span{display:block;margin-top:4px;color:var(--app-muted,var(--muted));font-size:11px}.conversation-header-actions{display:flex;gap:4px;flex-shrink:0}.chat-icon-button{width:34px;height:34px;padding:0;display:grid;place-items:center;border:0;border-radius:8px;color:var(--app-muted,var(--muted));background:transparent;cursor:pointer}.chat-icon-button:hover{background:color-mix(in srgb,var(--app-ink,#172033) 6%,transparent)}.chat-icon-button::-webkit-details-marker{display:none}
.conversation-actions{position:relative}.conversation-actions summary{list-style:none}.conversation-action-menu{position:absolute;top:40px;right:0;z-index:10;display:flex;flex-direction:column;min-width:190px;padding:6px;background:var(--app-surface,#fff);border:1px solid var(--line);border-radius:12px;box-shadow:0 12px 36px #0002}.conversation-action-menu button{display:flex;align-items:center;gap:9px;text-align:left;border:0;border-radius:7px;padding:10px 12px;background:transparent;color:inherit;font-size:12px}.conversation-action-menu button:hover{background:color-mix(in srgb,var(--app-ink,#172033) 6%,transparent)}.conversation-action-menu button.destructive{color:var(--red)}
.conversation-alerts{flex-shrink:0;max-height:32%;overflow:auto;padding:0 28px}.conversation-alerts:empty{display:none}.conversation-rename{display:flex;gap:8px;margin:12px 0}.conversation-rename input{min-width:0;flex:1;padding:8px;border:1px solid var(--line);border-radius:7px;color:inherit;background:var(--app-surface,#fff)}
.conversation-reading-area{position:relative;display:flex;flex-direction:column;flex:1;min-height:0}.conversation-transcript{flex:1;min-height:0;min-width:0;overflow:auto;overscroll-behavior:contain;scrollbar-gutter:stable;padding:24px 28px 32px;scrollbar-width:thin;overflow-anchor:none}.conversation-message-column{width:100%;max-width:780px;margin:0 auto}.conversation-earlier{display:flex;margin:0 auto 24px}.conversation-empty{display:flex;min-height:200px;flex-direction:column;align-items:center;justify-content:center;text-align:center;padding:clamp(20px,6vh,70px) 20px;color:var(--app-muted,var(--muted))}.conversation-empty h3{font-size:28px;font-weight:500;letter-spacing:-.7px;color:var(--app-ink,#172033);margin:22px 0 12px}.conversation-empty p{font-size:14px;line-height:1.8;max-width:470px;margin:0 auto}.conversation-pending{display:flex;align-items:flex-start;gap:10px;margin:24px 0;color:var(--app-muted,var(--muted));font-size:13px}.conversation-pending small{display:block;font-size:11px;margin-top:6px}.reply-pulse{width:8px;height:8px;border-radius:50%;background:currentColor;margin-top:5px;animation:reply-pulse 1.5s ease-in-out infinite}@keyframes reply-pulse{50%{opacity:.35}}.conversation-jump{position:absolute;bottom:14px;left:50%;transform:translateX(-50%);border:1px solid var(--line);border-radius:22px;background:var(--app-surface,#fff);color:var(--app-ink,#172033);padding:9px 14px;display:flex;align-items:center;gap:7px;font-size:11px;box-shadow:0 3px 12px #0001;white-space:nowrap}
.conversation-compose-area{flex-shrink:0;width:100%;max-width:836px;align-self:center;padding:0 28px 16px}.conversation-composer{border:1px solid var(--line);border-radius:22px;background:var(--app-surface,#fff);padding:15px 17px 12px;box-shadow:0 4px 24px #00000006}.conversation-composer:focus-within{border-color:color-mix(in srgb,var(--app-ink,#172033) 28%,var(--line));box-shadow:0 4px 24px #00000009}.conversation-composer textarea{display:block;width:100%;resize:none;min-height:58px;max-height:160px;overflow:auto;background:transparent;color:inherit;border:0;outline:none;box-shadow:none!important;font-size:14px;line-height:1.7;padding:0 1px 8px}.conversation-composer footer{display:flex;align-items:center;gap:12px}.conversation-composer small{font-size:10px;color:var(--app-muted,var(--muted));line-height:1.5;margin-left:auto}.composer-mode{display:flex;align-items:center;gap:6px;color:var(--app-muted,var(--muted));font-size:11px;white-space:nowrap}.composer-steer{flex-shrink:0;white-space:nowrap;border:1px solid var(--line);border-radius:18px;padding:8px 12px;background:transparent;color:var(--app-ink,#172033);font-size:11px;font-weight:600}.conversation-steer-queue{flex:0 1 auto;min-height:0;max-height:40%;overflow:auto;width:calc(100% - 56px);max-width:780px;align-self:center;margin:0 0 10px;padding:12px 14px;border:1px solid var(--line);border-radius:12px;background:color-mix(in srgb,var(--app-ink,#172033) 3%,transparent)}.conversation-steer-queue header{display:flex;align-items:center;justify-content:space-between;gap:10px;font-size:11px}.conversation-steer-queue header span{color:var(--app-muted,var(--muted));font-size:10px}.steer-content{max-height:68px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;line-height:1.6;margin:8px 0}.steer-reason{font-size:10px;line-height:1.6;color:var(--app-muted,var(--muted));margin:6px 0}.steer-queue-actions{display:flex;align-items:center;justify-content:space-between;gap:8px}.steer-queue-actions:empty{display:none}.steer-queue-actions .text-button{padding:3px 0}.has-queued-steer .conversation-jump{bottom:auto;top:12px}.composer-submit{display:grid;place-items:center;flex-shrink:0;width:36px;height:36px;padding:0;border:0;border-radius:50%;background:var(--app-ink,#172033);color:var(--app-bg,#fff)}.composer-submit:disabled{opacity:.3}.composer-submit.is-stop{opacity:1}.composer-submit.is-stop:disabled{opacity:.45}
.conversation-delete-confirm{display:flex;align-items:center;gap:10px;margin:12px 0;padding:12px;border:1px solid var(--line);background:var(--app-surface,#fff);border-radius:10px;font-size:12px;flex-wrap:wrap}.conversation-delete-confirm>div{flex:1;min-width:200px}.conversation-delete-confirm p{margin:5px 0;font-size:11px;line-height:1.6}.conversation-reply-status{display:flex;align-items:center;flex-wrap:wrap;gap:8px;font-size:11px;padding:8px 4px;color:var(--app-muted,var(--muted))}.conversation-reply-status small{font-size:11px}.conversation-notice{padding:9px 12px;border-radius:8px;background:color-mix(in srgb,var(--blue) 7%,transparent);font-size:12px;line-height:1.6;margin:8px 0}.conversation-error{font-size:12px;line-height:1.6;color:var(--red,#c44848)!important;overflow-wrap:anywhere}.conversation-muted{font-size:12px;color:var(--app-muted,var(--muted))}.conversation-model-notice,.conversation-privacy{font-size:10px;line-height:1.6;color:var(--app-muted,var(--muted));text-align:center;margin:9px 0 0}.conversation-model-notice{font-size:11px;text-align:left;padding:0 4px}
.conversation-workspace :deep(.chat-plugin-panel){margin:26px 0 0;background:transparent;border:0;border-top:1px solid var(--line);border-radius:0;padding:16px 0 0}
@media(max-width:760px){.conversation-header{padding:12px 16px;min-height:60px}.conversation-alerts{padding:0 16px}.conversation-transcript{padding:18px 16px 24px}.conversation-compose-area{padding:0 12px max(10px,env(safe-area-inset-bottom))}.conversation-composer{padding:12px;border-radius:18px}.conversation-composer footer{gap:7px}.conversation-composer footer:has(.composer-steer) .composer-mode{display:none}.conversation-composer footer>small{max-width:180px;font-size:9px}.composer-mode{font-size:10px;gap:4px}.conversation-welcome{padding:24px 18px}.conversation-welcome h2{font-size:27px}.conversation-privacy{font-size:9px}.conversation-empty h3{font-size:25px}.has-local-history{flex-direction:column}.has-local-history>.conversation-list{width:100%;max-height:220px}.conversation-workspace.has-local-history{height:auto}.conversation-welcome-mark{width:50px;height:50px}}
@media(max-height:580px){.conversation-header{min-height:48px;padding-block:7px}.conversation-composer textarea{min-height:36px;max-height:80px}.conversation-composer{padding-block:10px}.conversation-transcript{padding-block:12px}.conversation-privacy{margin-top:5px;line-height:1.3}.conversation-compose-area{padding-bottom:8px}.conversation-steer-queue{max-height:35%;padding-block:8px}}
@media(prefers-reduced-motion:reduce){.reply-pulse{animation:none}}
</style>
