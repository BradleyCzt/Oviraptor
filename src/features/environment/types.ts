// Types owned by this module; cross-module consumers import this contract directly.
export interface EnvironmentDependency {
  name: string;
  command: string;
  version: string;
  available: boolean;
  detail: string;
}

export interface EnvironmentReport {
  os: string;
  arch: string;
  node: string;
  dependencies: EnvironmentDependency[];
  checkedAt: string;
}

export interface EnvironmentPreparationStatus {
  state: "idle" | "installing" | "requires_manual_recovery";
  owner: string | null;
  createdAt: string | null;
}

export type InstallLogRow = { id: number; stage: string; stream: string; message: string; time: string };
export type InstallLogPage = { rows: InstallLogRow[]; more: boolean; olderRows: number; earliestId: number };
