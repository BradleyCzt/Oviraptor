<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { Archive, ArrowUp, MessageSquare, Plus, RefreshCw, Square, Trash2 } from "@lucide/vue";
import type { Project } from "../../types";
import { useI18n } from "../../i18n";
import { conversationApi } from "./api";
import { useConversationWorkspace } from "./useConversationWorkspace";
const props = defineProps<{ projects: Project[]; projectId?: number }>();
const emit = defineEmits<{ selectProject: [id: number]; createProject: [] }>();
const { tr } = useI18n();
const selectedProject = computed(() => props.projects.find(project => project.id === props.projectId));
const state = useConversationWorkspace({ project: () => selectedProject.value, api: conversationApi });
const { conversations, total, includeArchived, query, selectedId, snapshot, draft, titleDraft, loadingList, loading, loadingEarlier, busy, cancelling, listError, readError, actionError, hasListSnapshot, pendingSend, pendingDelete, writable, replyActive, messageBytes, messageTooLong, canSend, canDelete } = state;
const transcript = ref<HTMLElement>();
const renameOpen = ref(false);
const activeProjects = computed(() => props.projects.filter(project => project.status === "active"));
const replyStatus = computed(() => {
  const status = snapshot.value?.latestReply?.status;
  return status ? ({ queued: tr("等待回复", "Reply queued"), running: tr("正在回复…", "Replying…"), completed: tr("回复已保存", "Reply saved"), unavailable: tr("模型暂不可用", "Model unavailable"), failed: tr("回复失败", "Reply failed"), cancel_requested: tr("正在取消…", "Cancelling…"), cancelled: tr("回复已取消", "Reply cancelled"), interrupted: tr("回复已中断", "Reply interrupted") })[status] : "";
});
watch(selectedId, () => { renameOpen.value = false; });
watch(() => snapshot.value?.messages[snapshot.value.messages.length - 1]?.id, async () => {
  const follow = !transcript.value || transcript.value.scrollHeight - transcript.value.scrollTop - transcript.value.clientHeight < 100;
  await nextTick();
  if (follow && transcript.value) transcript.value.scrollTop = transcript.value.scrollHeight;
});
async function rename() {
  if (!titleDraft.value.trim()) return;
  await state.updateConversation({ title: titleDraft.value.trim() });
  if (!actionError.value) renameOpen.value = false;
}
function submit(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void state.send(); }
}
</script>
<template>
  <section class="conversation-workspace" :aria-label="tr('独立对话工作区','Independent chat workspace')">
    <div v-if="!selectedProject" class="conversation-welcome">
      <MessageSquare :size="34"/>
      <h2>{{tr('从对话开始','Start with a conversation')}}</h2>
      <p>{{tr('在共享工作空间内保留对话，可独立使用，无需先创建扫描。','Keep conversations in a shared workspace. A scan is not required.')}}</p>
      <template v-if="projects.length">
        <p>{{tr('选择已有工作空间继续','Choose an existing workspace to continue')}}</p>
        <div class="conversation-projects"><button v-for="project in projects" :key="project.id" class="button secondary" @click="emit('selectProject',project.id)">{{project.name}}<small v-if="project.status==='archived'">{{tr('已归档 · 只读','Archived · read only')}}</small></button></div>
        <button v-if="!activeProjects.length" class="button ghost" @click="emit('createProject')"><Plus :size="15"/>{{tr('新建工作空间','New workspace')}}</button>
      </template>
      <template v-else><p>{{tr('先创建一个共享工作空间，对话、资产和扫描将共用这个空间。','Create a shared workspace for your conversations, assets, and scans.')}}</p><button class="button primary" @click="emit('createProject')"><Plus :size="16"/>{{tr('创建工作空间','Create workspace')}}</button></template>
    </div>
    <template v-else>
      <aside class="conversation-list">
        <header><strong>{{tr('对话','Conversations')}}</strong><button class="button primary compact" :disabled="busy||selectedProject.status!=='active'" @click="state.createConversation()"><Plus :size="14"/>{{tr('新对话','New chat')}}</button></header>
        <input v-model="query" class="conversation-search" type="search" :aria-label="tr('搜索当前工作空间的对话','Search conversations in this workspace')" :placeholder="tr('搜索历史对话…','Find past conversations…')" maxlength="200"/>
        <label class="conversation-archive-filter"><input v-model="includeArchived" type="checkbox"/>{{tr('显示已归档','Show archived')}}</label>
        <p v-if="listError" class="conversation-error" role="alert">{{listError}}<button class="text-button" @click="state.loadList()">{{tr('重试','Retry')}}</button></p>
        <p v-if="loadingList" role="status">{{tr('正在读取对话…','Loading conversations…')}}</p>
        <p v-else-if="hasListSnapshot&&!conversations.length&&!listError" class="conversation-muted">{{query.trim()?tr('没有匹配的对话','No matching conversations'):tr('还没有对话','No conversations yet')}}</p>
        <button v-for="conversation in conversations" :key="conversation.id" class="conversation-row" :class="{selected:selectedId===conversation.id}" :aria-current="selectedId===conversation.id?'page':undefined" @click="state.selectConversation(conversation.id)"><strong>{{conversation.title}}</strong><span>{{conversation.lastMessagePreview||tr('空对话','Empty conversation')}}</span><small>{{conversation.archived?tr('已归档','Archived'):''}} {{conversation.updatedAt.slice(0,10)}}</small></button>
        <button v-if="conversations.length<total" class="button ghost compact" :disabled="loadingList" @click="state.loadList(true)">{{tr('更多对话','More conversations')}}</button>
      </aside>
      <main class="conversation-main">
        <p v-if="selectedProject.status!=='active'" class="conversation-notice">{{tr('该工作空间已归档，对话历史保持只读。可在项目管理中恢复工作空间。','This workspace is archived. Conversation history is read-only; restore the workspace from Projects to continue.')}}</p>
        <p v-if="actionError" class="conversation-error" role="alert">{{actionError}}</p>
        <p v-if="readError" class="conversation-error" role="alert">{{tr('读取失败，当前内容可能不是最新状态。','Read failed; the displayed history may be stale.')}} {{readError}} <button class="text-button" @click="state.readSelected()">{{tr('重试读取','Retry read')}}</button></p>
        <p v-if="loading" role="status">{{tr('正在读取历史…','Loading history…')}}</p>
        <template v-if="snapshot">
          <header class="conversation-header"><div><h2>{{snapshot.conversation.title}}</h2><small>{{tr('普通对话 · 不执行工具或扫描','Conversation · no tool or scan execution')}}<span v-if="snapshot.replyAvailability.model"> · {{snapshot.replyAvailability.model}}</span></small></div><div class="conversation-actions"><button class="button ghost compact" :disabled="loading" :title="tr('刷新历史','Refresh history')" @click="state.readSelected()"><RefreshCw :size="14"/></button><button class="button ghost compact" :disabled="busy||replyActive||!writable||!!readError" @click="renameOpen=!renameOpen">{{tr('重命名','Rename')}}</button><button class="button ghost compact" :disabled="busy||replyActive||selectedProject.status!=='active'||!!readError" @click="state.updateConversation({archived:!snapshot.conversation.archived})"><Archive :size="14"/>{{snapshot.conversation.archived?tr('恢复','Restore'):tr('归档','Archive')}}</button><button class="button ghost compact" :disabled="!canDelete" @click="state.requestDelete()"><Trash2 :size="14"/>{{tr('删除','Delete')}}</button></div></header>
          <div v-if="pendingDelete" class="conversation-delete-confirm" role="alert"><div><strong>{{tr('删除这个对话？','Delete this conversation?')}}</strong><p>{{tr('将永久删除此对话和消息，无法恢复。项目、资产、扫描和证据保持不变。','This permanently deletes the conversation and its messages. It cannot be undone. Projects, assets, scans, and evidence are kept.')}}</p></div><button class="button ghost compact" :disabled="busy" @click="state.cancelDelete()">{{tr('取消','Cancel')}}</button><button class="button danger compact" :disabled="!canDelete" @click="state.confirmDelete()">{{busy?tr('删除中…','Deleting…'):tr('永久删除对话','Delete conversation permanently')}}</button></div>
          <form v-if="renameOpen" class="conversation-rename" @submit.prevent="rename"><input v-model="titleDraft" :aria-label="tr('对话标题','Conversation title')" maxlength="160"/><button class="button secondary compact" :disabled="busy||!titleDraft.trim()">{{tr('保存','Save')}}</button><button type="button" class="button ghost compact" @click="renameOpen=false">{{tr('取消','Cancel')}}</button></form>
          <div ref="transcript" class="conversation-transcript" role="log" :aria-label="tr('对话历史','Conversation history')">
            <button v-if="snapshot.hasMore" class="button ghost compact" :disabled="loadingEarlier" @click="state.loadEarlier()">{{loadingEarlier?tr('读取中…','Loading…'):tr('更早的消息','Earlier messages')}}</button>
            <div v-if="!snapshot.messages.length" class="conversation-empty"><MessageSquare :size="28"/><h3>{{tr('有什么想讨论的？','What would you like to discuss?')}}</h3><p>{{tr('消息保存在本地。请求回复时，对话文本会发送给已配置的模型提供商。','Messages are stored locally. Requesting a reply sends conversation text to your configured model provider.')}}</p></div>
            <article v-for="message in snapshot.messages" :key="message.id" class="conversation-message" :class="message.role"><span>{{message.role==='user'?tr('你','You'):tr('助手','Assistant')}}</span><p>{{message.content}}</p></article>
          </div>
          <div v-if="snapshot.latestReply" class="conversation-reply-status" role="status"><span>{{replyStatus}}</span><small v-if="snapshot.latestReply.message">{{snapshot.latestReply.message}}</small><small v-if="snapshot.latestReply.outcomeUnknown">{{tr('提供商可能已处理此次请求；不会自动重发。','The provider may have processed this request. It will not be retried automatically.')}}</small><button v-if="replyActive&&selectedProject.status==='active'" class="button ghost compact" :disabled="cancelling||snapshot.latestReply.status==='cancel_requested'" @click="state.cancelReply()"><Square :size="12"/>{{tr('停止回复','Stop reply')}}</button></div>
          <p v-if="!snapshot.replyAvailability.available" class="conversation-notice">{{snapshot.replyAvailability.message||tr('尚未配置可用模型；消息仍可保存，助手不会生成回复。','No model is available. Messages can still be saved without an assistant reply.')}}</p>
          <p v-if="snapshot.conversation.archived" class="conversation-notice">{{tr('该对话已归档，恢复后可以继续。','This conversation is archived. Restore it to continue.')}}</p>
          <form class="conversation-composer" @submit.prevent="state.send()"><textarea v-model="draft" :disabled="!writable||busy||replyActive" :placeholder="tr('输入消息…','Write a message…')" :aria-label="tr('消息','Message')" rows="3" maxlength="16384" @keydown="submit"/><footer><small v-if="messageTooLong" class="conversation-error">{{tr(`消息为 ${messageBytes.toLocaleString()} 字节，最多 16,384 字节，请缩短后发送。`,`Message is ${messageBytes.toLocaleString()} bytes; shorten it to 16,384 bytes or less.`)}}</small><small v-else>{{pendingSend?tr('重试使用相同请求，不会自动重复发送','Retry reuses the same request'):tr('Enter 发送 · Shift+Enter 换行','Enter to send · Shift+Enter for a new line')}}</small><button class="button primary" :disabled="!canSend"><ArrowUp :size="16"/>{{busy?tr('发送中…','Sending…'):pendingSend?tr('重试发送','Retry send'):snapshot.replyAvailability.available?tr('发送','Send'):tr('保存消息','Save message')}}</button></footer></form>
        </template>
        <div v-else-if="!selectedId&&!loading" class="conversation-welcome"><MessageSquare :size="32"/><h2>{{tr('从一个新对话开始','Start a new conversation')}}</h2><p>{{tr('可以提问和讨论，不需要扫描任务。','Ask questions and discuss ideas without creating a scan.')}}</p><button class="button primary" :disabled="busy||selectedProject.status!=='active'" @click="state.createConversation()"><Plus :size="16"/>{{tr('新对话','New chat')}}</button></div>
      </main>
    </template>
  </section>
</template>
<style scoped>
.conversation-workspace{display:flex;min-height:calc(100vh - 150px);gap:22px;color:var(--app-ink,#172033)}
.conversation-welcome{display:flex;flex:1;flex-direction:column;align-items:center;justify-content:center;gap:12px;padding:48px 24px;text-align:center;max-width:760px;margin:0 auto}
.conversation-welcome>svg,.conversation-empty>svg{color:var(--blue)}
.conversation-welcome h2{font-size:27px;letter-spacing:-.6px;margin:5px 0}
.conversation-welcome p{max-width:510px;color:var(--app-muted,var(--muted));font-size:13px;line-height:1.7;margin:0}
.conversation-projects{display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin-top:12px}
.conversation-projects small{font-size:10px}
.conversation-list{width:230px;flex-shrink:0;border-right:1px solid var(--line);padding-right:16px}
.conversation-list header{display:flex;align-items:center;justify-content:space-between;margin-bottom:14px;gap:8px;font-size:13px}
.conversation-search{width:100%;height:34px;margin-bottom:10px;border:1px solid var(--line);border-radius:8px;padding:0 10px;background:var(--app-surface,#fff);color:inherit;font-size:12px}
.conversation-archive-filter{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--app-muted,var(--muted));margin-bottom:12px}
.conversation-row{display:flex;flex-direction:column;gap:5px;padding:12px;width:100%;text-align:left;border:0;border-radius:9px;background:transparent;color:inherit;margin-bottom:4px}
.conversation-row:hover,.conversation-row.selected{background:color-mix(in srgb,var(--blue) 8%,transparent)}
.conversation-row strong,.conversation-row span{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.conversation-row strong{font-size:12px}.conversation-row span{font-size:11px}.conversation-row small{font-size:10px;color:var(--app-muted,var(--muted))}
.conversation-main{flex:1;min-width:0;display:flex;flex-direction:column;max-width:1000px;margin:0 auto;width:100%}
.conversation-header{display:flex;justify-content:space-between;align-items:center;gap:16px;padding-bottom:16px;border-bottom:1px solid var(--line)}
.conversation-header h2{margin:0 0 5px;font-size:18px;overflow-wrap:anywhere}.conversation-header small{color:var(--app-muted,var(--muted));font-size:11px}
.conversation-actions{display:flex;gap:6px;flex-wrap:wrap;justify-content:flex-end}
.conversation-rename{display:flex;gap:8px;margin:12px 0}.conversation-rename input{min-width:0;flex:1;padding:8px;border:1px solid var(--line);border-radius:7px;color:inherit;background:var(--app-surface,#fff)}
.conversation-transcript{flex:1;overflow:auto;max-height:calc(100vh - 410px);min-height:220px;padding:20px 4px;scrollbar-gutter:stable}
.conversation-empty{text-align:center;padding:40px 20px;color:var(--app-muted,var(--muted))}.conversation-empty h3{font-weight:500;color:var(--app-ink,#172033)}.conversation-empty p{font-size:12px;line-height:1.7;max-width:470px;margin:0 auto}
.conversation-message{max-width:92%;margin:0 0 25px;padding:12px 16px;border-radius:14px}.conversation-message.user{margin-left:auto;background:color-mix(in srgb,var(--blue) 8%,transparent)}.conversation-message>span{font-size:10px;font-weight:600;color:var(--app-muted,var(--muted))}.conversation-message p{white-space:pre-wrap;overflow-wrap:anywhere;font-size:14px;line-height:1.8;margin:7px 0 0}
.conversation-composer{border:1px solid var(--line);border-radius:16px;background:var(--app-surface,#fff);padding:13px;margin-top:12px}.conversation-composer textarea{width:100%;resize:vertical;min-height:64px;max-height:250px;background:transparent;color:inherit;border:0;outline:none;font-size:14px;line-height:1.7}.conversation-composer footer{display:flex;justify-content:space-between;align-items:center;gap:10px}.conversation-composer small{font-size:10px;color:var(--app-muted,var(--muted))}
.conversation-delete-confirm{display:flex;align-items:center;gap:10px;margin:12px 0;padding:12px;border:1px solid color-mix(in srgb,var(--red,#c44848) 35%,transparent);background:color-mix(in srgb,var(--red,#c44848) 5%,transparent);border-radius:10px;font-size:12px;flex-wrap:wrap}.conversation-delete-confirm>div{flex:1;min-width:200px}.conversation-delete-confirm p{margin:5px 0;font-size:11px;line-height:1.6}
.conversation-reply-status{display:flex;align-items:center;flex-wrap:wrap;gap:8px;font-size:12px;padding:8px 0;color:var(--app-muted,var(--muted))}.conversation-reply-status small{font-size:11px}.conversation-notice{padding:10px 12px;border-radius:8px;background:color-mix(in srgb,var(--blue) 7%,transparent);font-size:12px;line-height:1.6}.conversation-error{font-size:12px;line-height:1.6;color:var(--red,#c44848);overflow-wrap:anywhere}.conversation-muted{font-size:12px;color:var(--app-muted,var(--muted))}
@media(max-width:1050px){.conversation-workspace{gap:14px}.conversation-list{width:190px}.conversation-header{align-items:flex-start;flex-direction:column}.conversation-actions{justify-content:flex-start}}
@media(max-width:760px){.conversation-workspace{flex-direction:column}.conversation-list{width:100%;border-right:0;border-bottom:1px solid var(--line);padding:0 0 12px;max-height:230px;overflow:auto}.conversation-row{padding:8px}.conversation-transcript{max-height:55vh}.conversation-welcome{padding:28px 12px}}
</style>
