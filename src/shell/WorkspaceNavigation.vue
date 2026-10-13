<script setup lang="ts">
import { computed } from "vue";
import { Activity, BrainCircuit, Braces, Database, FileClock, Globe2, LayoutDashboard, MessageSquare, Plus, ShieldAlert, ShieldCheck } from "@lucide/vue";
import { useI18n } from "../i18n";
import type { PluginEnabledState } from "../plugins/types";
import type { ViewKey } from "../types";
import { visibleModules, workspaceNavigation, type ModuleKey, type ShellIcon, type WorkspaceNavItem } from "./moduleCatalog";
const props = defineProps<{ module: ModuleKey; view: ViewKey; sentinelMenu: string; hackerOneEnabled: boolean; enabledPlugins?: PluginEnabledState; runningJobs?: number; blockedCount?: number; fuse?: number }>();
const emit = defineEmits<{ moduleChange: [module: ModuleKey]; navigate: [item: WorkspaceNavItem] }>();
const { tr } = useI18n();
const modules = computed(() => visibleModules(props.hackerOneEnabled,props.enabledPlugins));
const navigation = computed(() => workspaceNavigation(props.module));
const icons = { chat: MessageSquare, assets: Database, scan: ShieldCheck, hackerone: ShieldCheck, dashboard: LayoutDashboard, tasks: Activity, ownership: ShieldCheck, exposure: Globe2, changes: FileClock, quarantine: ShieldAlert, query: Plus, dialog: BrainCircuit, results: Globe2, skills: Braces } satisfies Record<ShellIcon, unknown>;
</script>
<template>
  <nav class="workspace-modules" :aria-label="tr('工作区模块','Product workspaces')">
    <button v-for="item in modules" :key="item.id" :class="{active:module===item.id}" :aria-current="module===item.id?'page':undefined" @click="emit('moduleChange',item.id)">
      <component :is="icons[item.icon]" :size="18"/><span>{{tr(...item.label)}}</span>
    </button>
  </nav>
  <nav v-if="navigation.length" class="nav-list workspace-subnav" :aria-label="tr('当前模块导航','Workspace navigation')">
    <button v-for="item in navigation" :key="item.key" :class="{active:view===item.view&&(module!=='sentinel'||sentinelMenu===item.key),accent:item.cta}" @click="emit('navigate',item)">
      <component :is="icons[item.icon]" :size="17"/><span>{{tr(...item.label)}}</span><em v-if="item.badge&&props[item.badge]">{{props[item.badge]}}</em>
    </button>
  </nav>
</template>
<style scoped>
.workspace-modules{display:flex;flex-direction:column;gap:4px;margin:2px 0 16px}
.workspace-modules button{display:flex;align-items:center;gap:11px;width:100%;height:42px;padding:0 12px;border:0;border-radius:9px;background:transparent;color:var(--app-muted,var(--muted));text-align:left;font-size:13px;font-weight:600}
.workspace-modules button:hover{background:color-mix(in srgb,var(--app-ink,#172033) 7%,transparent);color:var(--app-ink,#172033)}
.workspace-modules button.active{background:color-mix(in srgb,var(--blue) 12%,transparent);color:var(--blue)}
.workspace-subnav{border-top:1px solid color-mix(in srgb,var(--app-ink,#172033) 12%,transparent);padding-top:12px}
</style>
