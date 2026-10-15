import { invoke } from "@tauri-apps/api/core";
import type { SentinelScan } from "./types";

// Remote scan history/control belongs to Scanning. Worker identity and node CRUD stay in Runtime.

export const remoteScanningApi = {
  listRemoteWorkerScans: (nodeId: number) =>
    invoke<SentinelScan[]>("list_remote_worker_scans", { nodeId }),
  controlRemoteWorkerScan: (input: {
    nodeId: number;
    scanId: string;
    action: "pause" | "resume" | "cancel";
  }) => invoke<unknown>("control_remote_worker_scan", { input }),
  syncWorkerNode: (nodeId: number) => invoke<number>("sync_worker_node", { nodeId }),
};
