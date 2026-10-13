import type { ConfigProfile, InterruptedJob, Project, ViewKey } from "../types";
import type { WorkspaceNavItem } from "./moduleCatalog";

/** The host owns shared project/profile context; built-ins own their business state. */
export type AssetWorkspaceIntent = { id: number; projectId: number } & (
  | { kind: "start"; profileId?: number; name?: string; pipeline?: string }
  | { kind: "resume"; job: InterruptedJob }
);
export interface ResumeWorkflowResult { started: boolean; acknowledged: boolean; error?: string }
export interface ResumeWorkflowHost {
  begin(job: InterruptedJob): boolean;
  settle(job: InterruptedJob, result: ResumeWorkflowResult): void;
}
export interface PluginWorkspaceProps {
  projects: Project[];
  profiles: ConfigProfile[];
  projectId?: number;
  view: ViewKey;
  refreshToken: number;
  scanningEnabled?: boolean;
  resumeWorkflow?: ResumeWorkflowHost;
  navigation?: WorkspaceNavItem;
  intent?: AssetWorkspaceIntent;
}
export interface WorkspaceMetadata {
  title?: string;
  navigationKey?: string;
  badges?: { runningJobs?: number; blockedCount?: number; fuse?: number };
}
export interface PluginWorkspaceEmits {
  navigate: [view: ViewKey];
  selectProject: [id?: number];
  createProject: [];
  restoreProject: [project: Project];
  projectsChanged: [];
  notify: [type: "success" | "error" | "info", text: string];
  metadata: [value: WorkspaceMetadata];
  intentConsumed: [id: number];
  interruptedResumed: [runId: number];
}
