<script setup lang="ts">
import { computed } from "vue";
import { ArrowUpRight, Link2, Search, X } from "@lucide/vue";
import { BUILTIN_PLUGINS } from "../../plugins";
import type { BuiltinPluginHost } from "../../plugins";
import { useI18n } from "../../i18n";
import type { ConversationLinksInput, ConversationSnapshot } from "./types";
import { useConversationPlugins } from "./useConversationPlugins";
const props = defineProps<{
  host: BuiltinPluginHost; projectId: number; projectActive: boolean;
  snapshot?: ConversationSnapshot; blocked: boolean;
  saveLinks: (input: ConversationLinksInput) => Promise<boolean>;
}>();
const { tr } = useI18n();
const plugins = BUILTIN_PLUGINS.filter(plugin => plugin.id === "asset" || plugin.id === "sentinel");
const state = useConversationPlugins({ host: () => props.host, snapshot: () => props.snapshot, projectId: () => props.projectId, projectActive: () => props.projectActive, blocked: () => props.blocked, save: input => props.saveLinks(input) });
const { expanded, editing, pluginId, query, candidates, draftLinks, nextCursor, loading, saving, error, stale, hasUnavailable, disabledNewLinks, canEdit, canSave } = state;
const links = computed(() => props.snapshot?.links ?? []);
const pluginForKind = (kind: "asset" | "scan") => kind === "asset" ? "asset" : "sentinel";
const openLabel = (id: string) => id === "asset" ? tr("打开资产工作区", "Open Assets workspace") : tr("打开扫描工作区", "Open Scanning workspace");
</script>
<template>
  <section class="chat-plugin-panel" :aria-label="tr('内置能力插件','Built-in capability plugins')">
    <button type="button" class="chat-plugin-heading" :aria-expanded="expanded" @click="state.togglePanel()"><Link2 :size="14"/><strong>{{tr('插件与关联','Plugins & links')}} · {{links.length}}</strong><span>{{expanded?tr('收起','Hide'):tr('展开','Show')}}</span></button>
    <div v-if="expanded" class="chat-plugin-content">
    <p class="chat-plugin-description">{{tr('资产和扫描是此应用的内置插件。打开工作区或关联已有记录；这些操作不会让模型读取记录，也不会启动扫描。','Assets and Scanning are built-in plugins. Open a workspace or link existing records; these actions do not send records to the model or start scans.')}}</p>
    <div class="chat-plugin-cards">
      <article v-for="plugin in plugins" :key="plugin.id" class="chat-plugin-card">
        <div><strong>{{tr(plugin.label[0],plugin.label[1])}}</strong><small>{{props.host.isEnabled(plugin.id)?tr('已启用','Enabled'):tr('已停用','Disabled')}}</small></div>
        <p>{{tr(plugin.description[0],plugin.description[1])}}</p>
        <div class="chat-plugin-card-actions"><button type="button" class="button ghost compact" :disabled="!props.host.isEnabled(plugin.id)" @click="state.openWorkspace(plugin.id as 'asset'|'sentinel')">{{openLabel(plugin.id)}}<ArrowUpRight :size="13"/></button><button v-if="snapshot" type="button" class="button secondary compact" :disabled="!canEdit||!props.host.isEnabled(plugin.id)" @click="state.openEditor(plugin.id as 'asset'|'sentinel')"><Link2 :size="13"/>{{tr('关联记录','Link records')}}</button></div>
      </article>
    </div>
    <p v-if="error&&!editing" class="chat-plugin-error" role="alert">{{error}}</p>
    <div v-if="snapshot" class="chat-linked-records">
      <div class="chat-links-heading"><strong>{{tr('已关联记录','Linked records')}} <span>{{links.length}} / 50</span></strong><button v-if="links.length" type="button" class="button ghost compact" :disabled="!canEdit" @click="state.openEditor()">{{tr('管理关联','Manage links')}}</button></div>
      <p v-if="!links.length" class="chat-plugin-muted">{{tr('尚未关联资产或扫描。关联是本地导航引用，不是证据导入或执行授权。','No assets or scans linked. A link is a local reference, not imported evidence or execution permission.')}}</p>
      <ul v-else class="chat-link-list"><li v-for="link in links" :key="`${link.kind}:${link.targetId}`"><div><small>{{link.kind==='asset'?tr('资产','Asset'):tr('扫描','Scan')}}</small><strong>{{link.label}}</strong><code>{{link.targetId}}</code><span v-if="!link.available" class="chat-plugin-muted">{{tr('记录已不可用，引用保留','Record unavailable; reference retained')}}</span><span v-else-if="!props.host.isEnabled(pluginForKind(link.kind))" class="chat-plugin-muted">{{tr('插件已停用，引用保留','Plugin disabled; reference retained')}}</span></div><button type="button" class="button ghost compact" :disabled="!link.available||!props.host.isEnabled(pluginForKind(link.kind))" @click="state.openWorkspace(pluginForKind(link.kind))">{{openLabel(pluginForKind(link.kind))}}<ArrowUpRight :size="12"/></button></li></ul>
    </div>
    <section v-if="editing" class="chat-link-editor" :aria-label="tr('选择已有记录','Choose existing records')">
      <header><strong>{{tr('关联当前工作空间中的记录','Link records from this workspace')}}</strong><button type="button" class="button ghost compact" :disabled="saving" :aria-label="tr('关闭关联选择器','Close record picker')" @click="state.closeEditor()"><X :size="15"/></button></header>
      <p class="chat-plugin-muted">{{tr('仅保存引用。删除关联不会删除资产、扫描或证据。','Only references are saved. Removing a link does not delete assets, scans, or evidence.')}}</p>
      <p v-if="stale" class="chat-plugin-warning" role="alert">{{tr('对话已经变化。关闭并重新打开选择器，以最新记录重新确认。','The conversation changed. Close and reopen this picker to review the latest records.')}}</p>
      <p v-if="hasUnavailable" class="chat-plugin-warning">{{tr('部分关联记录已不可用。请明确移除这些引用后再保存，系统不会自动清除。','Some linked records are unavailable. Remove those references explicitly before saving; they are never cleared automatically.')}}</p>
      <p v-if="disabledNewLinks" class="chat-plugin-warning">{{tr('新选记录所属插件已停用。请移除这些新选择，或重新启用插件后再保存。','A plugin for newly selected records was disabled. Remove those new selections or re-enable the plugin before saving.')}}</p>
      <div v-if="draftLinks.length" class="chat-link-selection"><span v-for="link in draftLinks" :key="`${link.kind}:${link.targetId}`"><small>{{link.kind==='asset'?tr('资产','Asset'):tr('扫描','Scan')}}</small>{{link.label}}<button type="button" :disabled="!canEdit||stale" :aria-label="tr(`移除关联 ${link.label}`,`Remove link ${link.label}`)" @click="state.removeLink(link)"><X :size="12"/></button></span></div>
      <div class="chat-link-tabs"><button v-for="plugin in plugins" :key="plugin.id" type="button" :class="{selected:pluginId===plugin.id}" :disabled="saving||!props.host.isEnabled(plugin.id)" @click="state.selectPlugin(plugin.id as 'asset'|'sentinel')">{{tr(plugin.label[0],plugin.label[1])}}</button></div>
      <label class="chat-link-search"><Search :size="14"/><input v-model="query" type="search" maxlength="200" :disabled="saving||!props.host.isEnabled(pluginId)" :aria-label="tr('搜索当前工作空间的记录','Search records in this workspace')" :placeholder="tr('搜索已有记录…','Search existing records…')"/></label>
      <p v-if="!props.host.isEnabled(pluginId)" class="chat-plugin-muted">{{tr('插件已停用。可管理已有引用，但不会查询此插件。','This plugin is disabled. Existing links can be managed; its records are not queried.')}}</p>
      <p v-if="error" class="chat-plugin-error" role="alert">{{error}} <button type="button" class="text-button" :disabled="loading||saving" @click="state.loadCandidates()">{{tr('重新读取','Reload records')}}</button></p>
      <p v-if="loading" class="chat-plugin-muted" role="status">{{tr('正在读取记录…','Loading records…')}}</p>
      <p v-else-if="!error&&!candidates.length&&props.host.isEnabled(pluginId)" class="chat-plugin-muted">{{tr('没有匹配记录。可打开对应工作区查看或创建业务记录。','No matching records. Open the corresponding workspace to inspect or create records.')}}</p>
      <ul class="chat-candidate-list"><li v-for="candidate in candidates" :key="`${candidate.kind}:${candidate.targetId}`"><div><strong>{{candidate.label}}</strong><small>{{candidate.detail}}</small><code>{{candidate.targetId}}</code></div><button type="button" class="button secondary compact" :disabled="!canEdit||stale||state.isSelected(candidate)||draftLinks.length>=50" @click="state.addCandidate(candidate)">{{state.isSelected(candidate)?tr('已选择','Selected'):tr('选择','Select')}}</button></li></ul>
      <button v-if="nextCursor" type="button" class="button ghost compact" :disabled="loading||saving" @click="state.loadCandidates(true)">{{tr('更多记录','More records')}}</button>
      <footer><small>{{tr(`已选择 ${draftLinks.length} / 50 条`,`Selected ${draftLinks.length} / 50`)}}</small><button type="button" class="button ghost compact" :disabled="saving" @click="state.closeEditor()">{{tr('取消','Cancel')}}</button><button type="button" class="button primary compact" :disabled="!canSave" @click="state.save()">{{saving?tr('保存中…','Saving…'):tr('保存关联','Save links')}}</button></footer>
    </section>
    </div>
  </section>
</template>
<style scoped>
.chat-plugin-panel{margin:14px 0;padding:14px;border:1px solid var(--line);border-radius:12px;background:color-mix(in srgb,var(--blue) 2%,var(--app-surface,#fff));font-size:12px}
.chat-plugin-heading{cursor:pointer;font-size:12px;display:flex;align-items:center;gap:7px;width:100%;border:0;padding:0;background:none;color:inherit;text-align:left}.chat-plugin-heading strong{font-size:12px}.chat-plugin-heading span{margin-left:auto;font-size:10px;color:var(--app-muted,var(--muted))}.chat-plugin-content{margin-top:10px}.chat-plugin-description,.chat-plugin-muted{font-size:11px;line-height:1.7;color:var(--app-muted,var(--muted));margin:6px 0 10px}
.chat-plugin-cards{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.chat-plugin-card{padding:12px;border:1px solid var(--line);border-radius:9px;background:var(--app-surface,#fff)}.chat-plugin-card>div:first-child{display:flex;align-items:center;justify-content:space-between;gap:8px}.chat-plugin-card small{font-size:10px;color:var(--app-muted,var(--muted))}.chat-plugin-card p{font-size:11px;line-height:1.6;color:var(--app-muted,var(--muted));margin:8px 0}.chat-plugin-card-actions{display:flex;flex-wrap:wrap;gap:5px}
.chat-linked-records{margin-top:12px}.chat-links-heading{display:flex;align-items:center;justify-content:space-between;gap:8px}.chat-links-heading span{font-size:10px;font-weight:400;color:var(--app-muted,var(--muted))}.chat-link-list,.chat-candidate-list{list-style:none;padding:0;margin:8px 0}.chat-link-list li,.chat-candidate-list li{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:9px 0;border-bottom:1px solid var(--line)}.chat-link-list li>div,.chat-candidate-list li>div{display:flex;flex-direction:column;gap:4px;min-width:0}.chat-link-list strong,.chat-candidate-list strong{font-size:11px;overflow-wrap:anywhere}.chat-link-list small,.chat-candidate-list small{font-size:10px;color:var(--app-muted,var(--muted));overflow-wrap:anywhere}.chat-link-list code,.chat-candidate-list code{font-size:9px;overflow-wrap:anywhere;color:var(--app-muted,var(--muted))}.chat-link-list button{max-width:180px;flex-shrink:0}
.chat-link-editor{padding:13px;margin-top:12px;border:1px solid var(--blue);border-radius:10px;background:var(--app-surface,#fff)}.chat-link-editor>header{display:flex;justify-content:space-between;gap:10px;align-items:center}.chat-link-selection{display:flex;gap:6px;flex-wrap:wrap;margin:9px 0}.chat-link-selection>span{display:flex;gap:6px;align-items:center;padding:5px 7px;background:color-mix(in srgb,var(--blue) 8%,transparent);border-radius:6px;font-size:10px;overflow-wrap:anywhere;max-width:100%}.chat-link-selection button{display:inline-flex;border:0;background:none;color:inherit;padding:2px}.chat-link-selection small{color:var(--app-muted,var(--muted))}.chat-link-tabs{display:flex;gap:6px;margin:12px 0 8px}.chat-link-tabs button{border:1px solid var(--line);background:transparent;color:inherit;padding:6px 10px;border-radius:7px;font-size:11px}.chat-link-tabs button.selected{color:var(--blue);border-color:var(--blue)}.chat-link-search{display:flex;align-items:center;gap:8px;border:1px solid var(--line);border-radius:7px;padding:8px}.chat-link-search input{border:0;outline:none;width:100%;background:transparent;color:inherit;font-size:12px}.chat-candidate-list{max-height:240px;overflow:auto}.chat-link-editor>footer{display:flex;align-items:center;justify-content:flex-end;gap:8px;margin-top:12px}.chat-link-editor>footer small{margin-right:auto;color:var(--app-muted,var(--muted));font-size:10px}.chat-plugin-warning{font-size:11px;line-height:1.7;color:var(--amber,#9a6700)}.chat-plugin-error{font-size:11px;line-height:1.7;color:var(--red,#c44848)}
@media(max-width:800px){.chat-plugin-cards{grid-template-columns:1fr}.chat-link-list li{align-items:flex-start;flex-direction:column}.chat-plugin-panel{padding:10px}}
</style>
