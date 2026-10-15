// Types owned by this module; cross-module consumers import this contract directly.
export interface ConfigProfile {
  id: number;
  name: string;
  description: string;
  isDefault: boolean;
  settings: Record<string, any>;
  createdAt: string;
  updatedAt: string;
}

export interface ModelProfileTestResult {
  ok: boolean;
  status: string;
  message: string;
  model: string;
  deployment: "cloud" | "local" | string;
}

export interface FofaApiTestResult {
  ok: boolean;
  status: string;
  message: string;
  account: string;
  plan: string;
}
