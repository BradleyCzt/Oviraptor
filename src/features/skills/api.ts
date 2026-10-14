import { invoke } from "@tauri-apps/api/core";
import type { SkillPackage, SkillPackagePreview, SkillResource, SkillResourcePage } from "./types";

export const skillsApi = {
  listPackages: () => invoke<SkillPackage[]>("list_skill_packages"),
  previewPackage: (path: string) => invoke<SkillPackagePreview>("preview_skill_package", { path }),
  importPackage: (input: { path: string; expectedDigest: string }) => invoke<SkillPackage>("import_skill_package", { input }),
  installBundled: () => invoke<SkillPackage>("install_bundled_skill_package"),
  setEnabled: (skillId: number, enabled: boolean) => invoke<void>("set_packaged_skill_enabled", { skillId, enabled }),
  listResources: (skillId: number) => invoke<{ skillId: number; packageId: string; resources: SkillResource[] }>("list_skill_package_resources", { skillId }),
  readResource: (skillId: number, path: string, offset = 0, limit = 65_536) => invoke<SkillResourcePage>("read_skill_package_resource", { skillId, path, offset, limit }),
};
export type SkillsApi = typeof skillsApi;
