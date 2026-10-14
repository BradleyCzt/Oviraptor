<script setup lang="ts">
import { computed, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronDown, FolderPlus, RefreshCw, ShieldCheck, X } from "@lucide/vue";
import { useI18n } from "../../i18n";
import { conversationApi } from "./api";
import { useConversationAccess } from "./useConversationAccess";
const props = defineProps<{ projectId: number; conversationId: string; writable: boolean; replyActive: boolean; blocked: boolean; model?: string | null }>();
const emit = defineEmits<{ busy: [value: boolean]; mode: [value: boolean | undefined] }>();
const { tr } = useI18n();
const state = useConversationAccess({
  scope: () => ({ projectId: props.projectId, conversationId: props.conversationId }),
  writable: () => props.writable, replyActive: () => props.replyActive, blocked: () => props.blocked, api: conversationApi,
  chooseDirectory: async () => {
    const result = await open({ directory: true, multiple: false, title: tr("选择此对话可读取的目录", "Choose a directory this conversation may read") });
    return typeof result === "string" ? result : null;
  },
});
const { access, skills, loading, saving, choosing, busy, error, skillsError, disclosureAccepted, stagedDirectory, canExpand, canRevoke } = state;
const enabledCount = computed(() => access.value ? Number(access.value.readAssets) + Number(access.value.readScans) + access.value.selectedSkillIds.length + access.value.directories.length : 0);
watch(() => access.value?.agentEnabled, value => emit("mode", value), { immediate: true, flush: "sync" });
watch(busy, value => emit("busy", value), { immediate: true, flush: "sync" });
async function changeSkill(id: number, event: Event) {
  const input = event.target as HTMLInputElement;
  await state.setSkill(id, input.checked);
  input.checked = Boolean(access.value?.selectedSkillIds.includes(id));
}
</script>
<template>
  <details class="conversation-access-panel">
    <summary><ShieldCheck :size="14"/><strong>{{tr('工具与访问权限','Tools & access')}}</strong><span>{{loading?tr('读取中','Loading'):tr(`${enabledCount} 项已授权`,`${enabledCount} permissions`)}}</span><ChevronDown :size="13"/></summary>
    <div class="conversation-access-content">
      <p class="access-intro">{{tr('模型可按问题选择已授权的只读工具。所有查询仅限当前项目；不会启动扫描、执行命令或写入文件。','The model can choose permitted read-only tools for your question. Queries stay in this project; tools do not start scans, run commands, or write files.')}}</p>
      <p v-if="error" class="access-error" role="alert">{{error}} <button type="button" class="text-button" :disabled="busy||loading" @click="state.refresh()"><RefreshCw :size="12"/>{{tr('重新读取权限','Refresh permissions')}}</button></p>
      <p v-if="loading&&!access" role="status">{{tr('正在读取此对话的权限…','Reading this conversation’s permissions…')}}</p>
      <template v-if="access">
        <div class="access-mode" role="group" :aria-label="tr('对话模式','Conversation mode')"><button type="button" :aria-pressed="access.agentEnabled" :class="{selected:access.agentEnabled}" :disabled="access.agentEnabled||!canExpand" @click="state.setTools({agentEnabled:true})">Agent</button><button type="button" :aria-pressed="!access.agentEnabled" :class="{selected:!access.agentEnabled}" :disabled="!access.agentEnabled||!canRevoke" @click="state.setTools({agentEnabled:false})">{{tr('普通对话','Plain chat')}}</button></div>
        <p v-if="!access.agentEnabled" class="access-muted">{{tr('普通对话只提供对话和当前项目概况，不调用工具或生成 AI 标题。已保存的权限保留，但不会被使用。','Plain chat sends conversation and project context without tools or AI title generation. Saved grants are retained but inactive.')}}</p>
        <div class="access-disclosure"><p>{{tr('启用后，所选模型提供商会接收工具读取到的项目记录、选定 Skill 内容和授权目录中的文件内容，用于回答当前对话。','When enabled, your configured model provider receives project records, selected skill contents, and files read from approved directories to answer this conversation.')}} <span v-if="access.providerLabel">{{tr('接收方：','Recipient: ')}}{{access.providerLabel}} · </span><span v-if="model">{{tr('当前模型：','Current model: ')}}{{model}}</span></p>
          <label v-if="!access.providerDisclosureAccepted"><input v-model="disclosureAccepted" type="checkbox" :disabled="!canExpand"/>{{tr('我同意将所选工具读取的内容发送给当前配置的模型提供商','I agree to send results from the selected tools to my configured model provider')}}</label>
          <button v-if="!access.providerDisclosureAccepted && disclosureAccepted && enabledCount" type="button" class="button secondary compact" :disabled="!canExpand || !access.providerIdentity" @click="state.setTools({})">{{tr('确认此接收方与当前 Skill 内容','Confirm recipient and current skill content')}}</button>
          <p v-if="access.skillsRequireReview" class="access-muted">{{tr('所选 Skill 已停用或内容改变。请重新确认内容，或移除失效的选择。','Selected skills changed or were disabled. Review their current content or remove unavailable selections.')}}</p>
          <small v-else-if="access.providerDisclosureAccepted">{{tr('已确认此数据传输。关闭下方权限可阻止后续读取；已发送的数据无法收回。','Data sharing confirmed. Turning permissions off blocks future reads; previously sent data cannot be recalled.')}}</small>
        </div>
        <div class="access-tool-list">
          <div class="access-tool-row"><div><strong>{{tr('项目资产','Project assets')}}</strong><small>{{tr('查询并读取当前项目中已有的资产记录','Search and read existing assets in this project')}}</small></div><button type="button" class="access-switch" role="switch" :aria-label="tr('允许读取项目资产','Allow reading project assets')" :aria-checked="access.readAssets" :disabled="access.readAssets?!canRevoke:!canExpand||!access.agentEnabled||!disclosureAccepted" @click="state.setTools({readAssets:!access.readAssets})"><span/></button></div>
          <div class="access-tool-row"><div><strong>{{tr('已有扫描结果','Saved scan results')}}</strong><small>{{tr('查询并读取已保存的扫描摘要，不发起新扫描','Search saved scan summaries without starting a scan')}}</small></div><button type="button" class="access-switch" role="switch" :aria-label="tr('允许读取扫描结果','Allow reading saved scan results')" :aria-checked="access.readScans" :disabled="access.readScans?!canRevoke:!canExpand||!access.agentEnabled||!disclosureAccepted" @click="state.setTools({readScans:!access.readScans})"><span/></button></div>
        </div>
        <section class="access-skills"><h4>{{tr('Skills · 按需读取','Skills · read on demand')}}</h4><p v-if="skillsError" class="access-error">{{skillsError}} <button type="button" class="text-button" @click="state.loadSkills()">{{tr('重试','Retry')}}</button></p><p v-else-if="!skills.length" class="access-muted">{{tr('没有可用的 Skill','No skills available')}}</p>
          <label v-for="skill in skills" :key="skill.id" class="access-skill-row"><input type="checkbox" :checked="access.selectedSkillIds.includes(skill.id)" :disabled="access.selectedSkillIds.includes(skill.id)?!canRevoke:!canExpand||!access.agentEnabled||!disclosureAccepted||!skill.enabled" @change="changeSkill(skill.id,$event)"/><span><strong>{{skill.name}}</strong><small>{{skill.description}}</small><small v-if="!skill.enabled">{{tr('已停用','Disabled')}}</small></span></label>
        </section>
        <section class="access-directories"><h4>{{tr('可读取的目录','Readable directories')}}</h4><p class="access-muted">{{tr('只允许目录内的受支持文本文件。密钥文件、隐藏目录和越界路径由后端拒绝。','Only supported text files inside an approved directory are readable. The backend rejects secret files, hidden directories, and paths outside the grant.')}}</p>
          <ul v-if="access.directories.length"><li v-for="directory in access.directories" :key="directory.id"><span><strong>{{directory.label}}</strong><small>{{directory.path}}</small></span><button type="button" class="text-button" :disabled="!canRevoke" :aria-label="tr(`撤销目录 ${directory.label}`,`Revoke directory ${directory.label}`)" @click="state.revokeDirectory(directory.id)"><X :size="14"/>{{tr('撤销','Revoke')}}</button></li></ul>
          <p v-if="!access.directorySupported" class="access-muted">{{tr('当前平台尚不支持安全目录读取，其他已授权工具仍可使用。','Safe directory reading is not supported on this platform. Other permitted tools remain available.')}}</p>
          <button v-else type="button" class="button ghost compact" :disabled="!canExpand||!access.agentEnabled" @click="state.chooseDirectory()"><FolderPlus :size="14"/>{{choosing?tr('选择中…','Choosing…'):tr('选择目录','Choose directory')}}</button>
          <div v-if="stagedDirectory" class="access-directory-confirm"><strong>{{tr('仅为此对话授予只读权限？','Grant read-only access to this conversation?')}}</strong><p>{{stagedDirectory}}</p><small>{{tr('模型可能按需读取此目录的文件内容，并将内容发送给当前配置的模型提供商。','The model may read files in this directory as needed and send their contents to the configured model provider.')}}</small><div><button type="button" class="button ghost compact" :disabled="busy" @click="state.clearStagedDirectory()">{{tr('取消','Cancel')}}</button><button type="button" class="button secondary compact" :disabled="!canExpand||!access.agentEnabled||!disclosureAccepted" @click="state.grantDirectory()">{{tr('授权只读访问','Allow read-only access')}}</button></div></div>
        </section>
        <p v-if="replyActive" class="access-muted">{{tr('回复期间可撤销权限；新增权限需等待回复结束，并在下一次发送时生效。','You can revoke access during a reply. Additions require an idle conversation and take effect on the next send.')}}</p>
        <p v-if="saving" role="status">{{tr('正在保存权限…','Saving permissions…')}}</p>
      </template>
    </div>
  </details>
</template>
<style scoped>
.access-mode{display:flex;gap:4px;padding:3px;border:1px solid var(--line,#ddd);border-radius:8px;width:fit-content;margin:12px 0}.access-mode button{font-size:11px;background:transparent;border:0;color:var(--app-muted,#667);padding:6px 13px;border-radius:5px}.access-mode button.selected{background:var(--app-ink,#172033);color:var(--app-surface,#fff);opacity:1}.conversation-access-panel{border:1px solid var(--line,#e1e3e7);border-radius:12px;margin:0 0 12px;font-size:12px;max-width:100%;background:var(--app-surface,#fff)}.conversation-access-panel>summary{display:flex;align-items:center;gap:8px;padding:11px 13px;cursor:pointer;list-style:none}.conversation-access-panel>summary::-webkit-details-marker{display:none}.conversation-access-panel>summary>strong{font-weight:550}.conversation-access-panel>summary>span{margin-left:auto;font-size:10px;color:var(--app-muted,#687078)}.conversation-access-content{padding:0 14px 14px;max-height:min(28vh,300px);overflow:auto;border-top:1px solid var(--line,#e1e3e7)}.access-intro,.access-disclosure p,.access-muted{line-height:1.7;color:var(--app-muted,#687078)}.access-disclosure{padding:10px 12px;border-radius:8px;background:color-mix(in srgb,var(--blue,#567) 6%,transparent);margin:10px 0}.access-disclosure p{margin:0 0 8px;font-size:11px}.access-disclosure label{display:flex;align-items:flex-start;gap:8px;font-size:11px;line-height:1.6}.access-disclosure small{font-size:10px;line-height:1.6;color:var(--app-muted,#687078)}.access-tool-row{display:flex;align-items:center;gap:16px;padding:13px 0;border-bottom:1px solid var(--line,#e1e3e7)}.access-tool-row>div{flex:1}.access-tool-row strong,.access-skill-row strong,.access-directories li strong{font-weight:550;display:block}.access-tool-row small,.access-skill-row small,.access-directories li small{display:block;color:var(--app-muted,#687078);font-size:10px;line-height:1.6;margin-top:4px;overflow-wrap:anywhere}.access-switch{width:34px;height:20px;padding:2px;border:0;border-radius:12px;background:var(--line,#ddd);flex-shrink:0}.access-switch span{display:block;width:16px;height:16px;border-radius:50%;background:white;box-shadow:0 1px 3px #0002;transition:transform .12s}.access-switch[aria-checked="true"]{background:var(--blue,#567)}.access-switch[aria-checked="true"] span{transform:translateX(14px)}.access-switch:disabled{opacity:.45}.access-content input{accent-color:var(--blue,#567)}h4{font-size:12px;font-weight:600;margin:18px 0 9px}.access-skill-row{display:flex;align-items:flex-start;gap:8px;padding:8px 0}.access-skill-row input{margin-top:3px}.access-directories ul{list-style:none;padding:0;margin:9px 0}.access-directories li{display:flex;align-items:flex-start;gap:8px;border-top:1px solid var(--line,#e1e3e7);padding:10px 0}.access-directories li>span{flex:1;min-width:0}.access-directory-confirm{border:1px solid var(--line,#e1e3e7);padding:12px;border-radius:8px;margin-top:10px}.access-directory-confirm>p{overflow-wrap:anywhere}.access-directory-confirm>small{display:block;line-height:1.6;color:var(--app-muted,#687078)}.access-directory-confirm>div{display:flex;gap:8px;justify-content:flex-end;margin-top:10px}.access-error{color:var(--red,#b33);overflow-wrap:anywhere;line-height:1.6}.text-button{display:inline-flex;align-items:center;gap:4px}.access-muted{font-size:10px}
</style>
