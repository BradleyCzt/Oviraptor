import { defineAsyncComponent } from "vue";

// These feature surfaces are mounted only after their Sentinel tab is selected.
// Keep their imports local to this business boundary so the board's initial
// script does not eagerly load the chat, task center, and investigation tools.
export const AgentDialog = defineAsyncComponent(
  () => import("./AgentDialog.vue"),
);
export const SentinelTaskCenter = defineAsyncComponent(
  () => import("./SentinelTaskCenter.vue"),
);
export const SentinelSourceResults = defineAsyncComponent(
  () => import("./results/SentinelSourceResults.vue"),
);
export const AgentWorkbench = defineAsyncComponent(
  () => import("./AgentWorkbench.vue"),
);
export const AgentTraceHub = defineAsyncComponent(
  () => import("./AgentTraceHub.vue"),
);
