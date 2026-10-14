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
