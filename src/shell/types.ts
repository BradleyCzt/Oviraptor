import type { InterruptedJob } from "../features/assets/types";
// Types owned by this module; cross-module consumers import this contract directly.
export type ViewKey =
  | "chat"
  | "skills"
  | "dashboard"
  | "projects"
  | "query"
  | "assets"
  | "ownership"
  | "exposure"
  | "quarantine"
  | "hackerone"
  | "sentinel"
  | "changes"
  | "tasks"
  | "logs"
  | "settings";

export interface AppSettings {
  reminderDays: number;
  customIcon: boolean;
  deduplicatedAssets: number;
}

export interface ToastMessage {
  id: number;
  type: "success" | "error" | "info";
  text: string;
}

export interface DashboardStats {
  projectCount: number;
  assetCount: number;
  aliveCount: number;
  pendingCount: number;
  newCount: number;
  changedCount: number;
  blockedCount: number;
  runningJobs: number;
}

export interface StaleProject {
  projectId: number;
  projectName: string;
  daysSinceUpdate?: number;
  lastRunAt?: string;
}

export interface StartupStatus {
  reminderDays: number;
  staleProjects: StaleProject[];
  interruptedJobs: InterruptedJob[];
}
