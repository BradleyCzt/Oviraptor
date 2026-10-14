import { invoke } from "@tauri-apps/api/core";
import type { AppSettings } from "../../shell/types";
import type { ConfigProfile } from "../configuration/types";
import type { DashboardStats } from "../assets/types";
import type { Project, ProjectImpact, StartupStatus } from "./types";

export const workspaceApi = {
  dashboardStats: (projectId?: number) =>
    invoke<DashboardStats>("dashboard_stats", { projectId }),
  listProjects: () => invoke<Project[]>("list_projects"),
  saveProject: (input: { id?: number; name: string; description: string }) =>
    invoke<number>("save_project", { input }),
  projectImpact: (projectId: number) =>
    invoke<ProjectImpact>("project_impact", { projectId }),
  archiveProject: (projectId: number, archived: boolean) =>
    invoke<void>("archive_project", { projectId, archived }),
  deleteProject: (projectId: number) =>
    invoke<void>("delete_project", { projectId }),
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  getAppIconDataUrl: () => invoke<string>("get_app_icon_data_url"),
  saveAppSettings: (reminderDays: number) =>
    invoke<void>("save_app_settings", { input: { reminderDays } }),
  saveAppIcon: (bytes: number[]) => invoke<void>("save_app_icon", { bytes }),
  resetAppIcon: () => invoke<void>("reset_app_icon"),
  startupStatus: () => invoke<StartupStatus>("startup_status"),
  acknowledgeInterruptedRun: (runId: number) =>
    invoke<void>("acknowledge_interrupted_run", { runId }),
  listProfiles: () => invoke<ConfigProfile[]>("list_config_profiles"),
  saveProfile: (input: {
    id?: number;
    name: string;
    description: string;
    isDefault: boolean;
    settings: Record<string, any>;
  }) => invoke<number>("save_config_profile", { input }),
  deleteProfile: (profileId: number) =>
    invoke<void>("delete_config_profile", { profileId }),
};
