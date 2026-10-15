<script setup lang="ts">
import { computed } from "vue";
import { Check, ChevronDown, ShieldAlert, Wrench } from "@lucide/vue";
import { useI18n } from "../../i18n";
import type { ConversationToolActivity } from "./types";
const props = defineProps<{ activity: ConversationToolActivity[] }>();
const { tr } = useI18n();
const completed = computed(() => props.activity.filter(item => item.status === "completed").length);
const labels: Record<string, readonly [string, string]> = {
  project_get_context: ["读取项目概况", "Read project context"],
  assets_search: ["查询项目资产", "Search project assets"], assets_read: ["读取资产详情", "Read asset details"],
  scans_search: ["查询已有扫描", "Search saved scans"], scans_read_summary: ["读取扫描摘要", "Read scan summary"],
  skills_list: ["查看已授权 Skills", "List permitted skills"], skills_read: ["读取 Skill", "Read a skill"],
  directory_list: ["列出授权目录", "List permitted directory"], directory_read_file: ["读取授权文件", "Read permitted file"],
  conversation_set_title: ["生成对话标题", "Generate conversation title"],
};
function label(name: string) { const value = labels[name]; return value ? tr(value[0], value[1]) : name; }
</script>
<template>
  <details v-if="activity.length" class="conversation-tool-activity">
    <summary><Wrench :size="13"/><span>{{tr('工具记录','Tool activity')}}</span><small>{{completed}} / {{activity.length}} {{tr('已完成','completed')}}</small><ChevronDown :size="13"/></summary>
    <ol><li v-for="item in activity" :key="item.id" :class="{'is-rejected':item.status==='rejected'}">
      <div class="tool-activity-heading"><Check v-if="item.status==='completed'" :size="14"/><ShieldAlert v-else :size="14"/><strong>{{label(item.name)}}</strong><span>{{item.status==='completed'?tr('已完成','Completed'):tr('未完成','Not completed')}}</span></div>
      <p>{{item.summary}}</p><small>{{tr(`第 ${item.round} 轮`,`Round ${item.round}`)}}</small>
    </li></ol>
    <p class="tool-activity-note">{{tr('来自后端保存的调用记录。这里只显示经过限制的摘要，不包含完整文件或凭据。','Saved backend call records. These are bounded summaries, not complete files or credentials.')}}</p>
  </details>
</template>
<style scoped>
.conversation-tool-activity{border:1px solid var(--line,#e1e3e7);border-radius:12px;margin:-12px 0 26px;background:color-mix(in srgb,var(--app-ink,#172033) 2%,transparent);font-size:12px;overflow:hidden}.conversation-tool-activity>summary{cursor:pointer;display:flex;align-items:center;gap:8px;padding:10px 13px;list-style:none}.conversation-tool-activity>summary::-webkit-details-marker{display:none}.conversation-tool-activity>summary small{margin-left:auto;font-size:10px;color:var(--app-muted,#687078)}.conversation-tool-activity>summary>svg:last-child{transition:transform .15s}.conversation-tool-activity[open]>summary>svg:last-child{transform:rotate(180deg)}.conversation-tool-activity ol{list-style:none;margin:0;padding:0 13px}.conversation-tool-activity li{padding:12px 0;border-top:1px solid var(--line,#e1e3e7)}.tool-activity-heading{display:flex;align-items:center;gap:7px}.tool-activity-heading strong{font-weight:550}.tool-activity-heading>span{margin-left:auto;font-size:10px;color:var(--app-muted,#687078)}.tool-activity-heading>svg{color:var(--green,#48775b)}.is-rejected .tool-activity-heading>svg{color:var(--orange,#a56b36)}.conversation-tool-activity li p{white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.7;margin:8px 0 4px}.conversation-tool-activity li>small,.tool-activity-note{font-size:10px;color:var(--app-muted,#687078)}.tool-activity-note{padding:0 13px 12px;margin:0;line-height:1.6}
</style>
