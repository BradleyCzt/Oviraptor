import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, DashboardStats, StartupStatus } from "./types";

export const shellApi = {
  dashboardStats: (projectId?: number) =>
    invoke<DashboardStats>("dashboard_stats", { projectId }),
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  getAppIconDataUrl: () => invoke<string>("get_app_icon_data_url"),
  saveAppSettings: (reminderDays: number) =>
    invoke<void>("save_app_settings", { input: { reminderDays } }),
  saveAppIcon: (bytes: number[]) => invoke<void>("save_app_icon", { bytes }),
  resetAppIcon: () => invoke<void>("reset_app_icon"),
  startupStatus: () => invoke<StartupStatus>("startup_status"),
};
