import { invoke } from "@tauri-apps/api/core";
import type { EnvironmentReport } from "../environment/types";
import type { LocalWorkerSettings, RemoteWorkerNode, WorkerHealth } from "./types";


export const runtimeApi = {
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
  getRemoteWorkerEnvironment: (nodeId: number) =>
    invoke<EnvironmentReport>("get_remote_worker_environment", { nodeId }),
};
