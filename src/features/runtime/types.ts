// Types owned by this module; cross-module consumers import this contract directly.
export interface LocalWorkerSettings {
  enabled: boolean;
  port: number;
  accessToken: string;
  tailscaleIp: string;
  endpoint: string;
  running: boolean;
  status: string;
}

export interface RemoteWorkerNode {
  id: number;
  name: string;
  endpoint: string;
  accessToken: string;
  enabled: boolean;
  lastSeenAt?: string;
  lastSyncAt?: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
}

export interface WorkerHealth {
  service: string;
  version: string;
  hostname: string;
  os: string;
  arch: string;
  tailscaleIp: string;
  runningScans: number;
  completedScans: number;
  checkedAt: string;
}
