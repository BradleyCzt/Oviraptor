import { invoke } from "@tauri-apps/api/core";
import type { ConfigProfile, ModelProfileTestResult, FofaApiTestResult } from "./types";

export const configurationApi = {
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
  testModelProfile: (input: {
    llm: string; deployment: "cloud" | "local"; apiBase: string; apiKey: string;
  }) => invoke<ModelProfileTestResult>("test_model_profile", { input }),
  testFofaApi: (input: { key: string; proxyUrl: string }) =>
    invoke<FofaApiTestResult>("test_fofa_api", { input }),
};
