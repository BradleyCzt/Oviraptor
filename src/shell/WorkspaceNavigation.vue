<script setup lang="ts">
import { computed } from "vue";
import { Activity, BrainCircuit, Braces, Database, FileClock, Globe2, LayoutDashboard, MessageSquare, Plus, ShieldAlert, ShieldCheck } from "@lucide/vue";
import { useI18n } from "../i18n";
import type { PluginEnabledState } from "../plugins/types";
import type { ViewKey } from "../types";
import { visibleModules, workspaceNavigation, type ModuleKey, type ShellIcon, type WorkspaceNavItem } from "./moduleCatalog";
const props = defineProps<{ collapsed?: boolean; module: ModuleKey; view: ViewKey; sentinelMenu: string; hackerOneEnabled: boolean; enabledPlugins?: PluginEnabledState; runningJobs?: number; blockedCount?: number; fuse?: number }>();
const emit = defineEmits<{ moduleChange: [module: ModuleKey]; navigate: [item: WorkspaceNavItem]; openSkills: [] }>();
const { tr } = useI18n();
const modules = computed(() => visibleModules(props.hackerOneEnabled,props.enabledPlugins));
const chatModules = computed(() => modules.value.filter(item => item.id === "chat"));
const pluginModules = computed(() => modules.value.filter(item => item.id !== "chat"));
const navigation = computed(() => workspaceNavigation(props.module));
const icons = { chat: MessageSquare, assets: Database, scan: ShieldCheck, hackerone: ShieldCheck, dashboard: LayoutDashboard, tasks: Activity, ownership: ShieldCheck, exposure: Globe2, changes: FileClock, quarantine: ShieldAlert, query: Plus, dialog: BrainCircuit, results: Globe2, skills: Braces } satisfies Record<ShellIcon, unknown>;
</script>
<template>
  <div class="workspace-navigation" :class="{collapsed}">
    <nav class="workspace-modules workspace-chat-module" :class="{collapsed}" :aria-label="tr('对话','Chat')">
      <button v-for="item in chatModules" :key="item.id" :title="tr(...item.label)" :aria-label="tr(...item.label)" :class="{active:module===item.id&&view!=='skills'}" :aria-current="module===item.id&&view!=='skills'?'page':undefined" @click="emit('moduleChange',item.id)">
        <component :is="icons[item.icon]" :size="18"/><span>{{tr(...item.label)}}</span>
      </button>
    </nav>
    <div v-show="!collapsed" class="workspace-chat-history"><slot name="history"/></div>
    <div class="workspace-plugin-group">
      <nav class="workspace-modules workspace-plugin-modules" :class="{collapsed}" :aria-label="tr('插件工作区','Plugin workspaces')">
        <button v-for="item in pluginModules" :key="item.id" :title="tr(...item.label)" :aria-label="tr(...item.label)" :class="{active:module===item.id&&view!=='skills'}" :aria-current="module===item.id&&view!=='skills'?'page':undefined" @click="emit('moduleChange',item.id)">
          <component :is="icons[item.icon]" :size="18"/><span>{{tr(...item.label)}}</span>
        </button>
        <button :class="{active:view==='skills'}" :aria-current="view==='skills'?'page':undefined" :title="tr('Skills 管理','Skills')" :aria-label="tr('Skills 管理','Skills')" @click="emit('openSkills')"><Braces :size="18"/><span>{{tr('Skills 管理','Skills')}}</span></button>
      </nav>
      <nav v-if="navigation.length" v-show="!collapsed" class="nav-list workspace-subnav" :aria-label="tr('当前模块导航','Workspace navigation')">
        <button v-for="item in navigation" :key="item.key" :aria-current="view===item.view&&(module!=='sentinel'||sentinelMenu===item.key)?'page':undefined" :class="{active:view===item.view&&(module!=='sentinel'||sentinelMenu===item.key),accent:item.cta}" @click="emit('navigate',item)">
          <component :is="icons[item.icon]" :size="17"/><span>{{tr(...item.label)}}</span><em v-if="item.badge&&props[item.badge]">{{props[item.badge]}}</em>
        </button>
      </nav>
    </div>
    <div v-show="!collapsed" class="workspace-selector"><slot name="workspace"/></div>
  </div>
</template>
<style scoped>
.workspace-navigation{display:flex;flex-direction:column;flex:1;min-height:0;min-width:0}
.workspace-chat-history{flex:1;min-height:0;overflow:hidden}
.workspace-plugin-group{flex:0 1 auto;min-height:0;max-height:48%;overflow:auto;margin-top:auto;padding-top:12px;border-top:1px solid var(--line);scrollbar-width:thin}
.workspace-selector{flex-shrink:0}
.workspace-navigation.collapsed .workspace-plugin-group{max-height:none;flex-shrink:0;overflow:visible;border-top:0;padding-top:0}

.workspace-modules{display:flex;flex-direction:column;gap:4px;margin:2px 0 16px}
.workspace-modules button{display:flex;align-items:center;gap:11px;width:100%;height:42px;padding:0 12px;border:0;border-radius:9px;background:transparent;color:var(--app-muted,var(--muted));text-align:left;font-size:13px;font-weight:600}
.workspace-modules button:hover{background:color-mix(in srgb,var(--app-ink,#172033) 7%,transparent);color:var(--app-ink,#172033)}
.workspace-modules button.active{background:color-mix(in srgb,var(--blue) 12%,transparent);color:var(--blue)}
.workspace-subnav{border-top:1px solid color-mix(in srgb,var(--app-ink,#172033) 12%,transparent);padding-top:12px}
.workspace-modules.collapsed button{justify-content:center;width:40px;padding:0}.workspace-modules.collapsed button span{display:none}.workspace-modules.collapsed{align-items:center}
</style>
