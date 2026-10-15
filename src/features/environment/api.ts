import { invoke } from "@tauri-apps/api/core";
import type { EnvironmentReport, EnvironmentPreparationStatus, InstallLogPage } from "./types";

export const environmentApi = {
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
};
