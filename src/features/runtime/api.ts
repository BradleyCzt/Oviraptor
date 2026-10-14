import { invoke } from "@tauri-apps/api/core";
import type { EnvironmentReport, EnvironmentPreparationStatus } from "../environment/types";
import type { LocalWorkerSettings, RemoteWorkerNode, WorkerHealth } from "./types";
import type { SentinelScan } from "../sentinel/types";

export type InstallLogRow = { id: number; stage: string; stream: string; message: string; time: string };
export type InstallLogPage = { rows: InstallLogRow[]; more: boolean; olderRows: number; earliestId: number };

export const runtimeApi = {
  checkEnvironment: (profileId?: number) =>
    invoke<EnvironmentReport>("check_environment", { profileId }),
  installEnvironmentDependencies: (profileId?: number) =>
    invoke<string>("install_environment_dependencies", { profileId }),
  listEnvironmentInstallLogs: (afterId?: number, limit = 300) =>
    invoke<InstallLogPage>("list_environment_install_logs", { afterId, limit }),
  getEnvironmentPreparationStatus: () =>
    invoke<EnvironmentPreparationStatus>("get_environment_preparation_status"),
  recoverEnvironmentPreparation: (expectedOwner: string, acknowledgement: string) =>
    invoke<void>("recover_environment_preparation", { expectedOwner, acknowledgement }),
  getLocalWorkerSettings: () =>
    invoke<LocalWorkerSettings>("get_local_worker_settings"),
  saveLocalWorkerSettings: (input: {
    enabled: boolean;
    port: number;
    accessToken: string;
  }) => invoke<LocalWorkerSettings>("save_local_worker_settings", { input }),
  listWorkerNodes: () => invoke<RemoteWorkerNode[]>("list_worker_nodes"),
  saveWorkerNode: (input: {
    id?: number;
    name: string;
    endpoint: string;
    accessToken: string;
    enabled: boolean;
  }) => invoke<number>("save_worker_node", { input }),
  deleteWorkerNode: (nodeId: number) => invoke<void>("delete_worker_node", { nodeId }),
  testWorkerNode: (nodeId: number) => invoke<WorkerHealth>("test_worker_node", { nodeId }),
  listRemoteWorkerScans: (nodeId: number) =>
    invoke<SentinelScan[]>("list_remote_worker_scans", { nodeId }),
  getRemoteWorkerEnvironment: (nodeId: number) =>
    invoke<EnvironmentReport>("get_remote_worker_environment", { nodeId }),
  controlRemoteWorkerScan: (input: {
    nodeId: number;
    scanId: string;
    action: "pause" | "resume" | "cancel";
  }) => invoke<unknown>("control_remote_worker_scan", { input }),
  syncWorkerNode: (nodeId: number) => invoke<number>("sync_worker_node", { nodeId }),
};
