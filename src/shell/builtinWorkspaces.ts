import { defineAsyncComponent } from "vue";
import type { BuiltinPluginId } from "../plugins";

// Trusted, bundled entrypoints only. Registry metadata never supplies executable
// paths or remote code. Switching UI availability does not touch background jobs.
export const BUILTIN_WORKSPACES = {
  asset: defineAsyncComponent(() => import("../features/assets/workspace/AssetsPluginWorkspace.vue")),
  sentinel: defineAsyncComponent(() => import("../features/sentinel/workspace/ScanningPluginWorkspace.vue")),
  hackerone: defineAsyncComponent(() => import("../features/hackerone/workspace/HackerOnePluginWorkspace.vue")),
} satisfies Record<BuiltinPluginId, ReturnType<typeof defineAsyncComponent>>;
