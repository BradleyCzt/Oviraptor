export interface PackagedSkill {
  id: number;
  name: string;
  description: string;
  entryPath: string;
  enabled: boolean;
}
export interface SkillPackage {
  id: string;
  name: string;
  source: string;
  version: string;
  license: string;
  builtin: boolean;
  digest: string;
  skillCount: number;
  fileCount: number;
  totalBytes: number;
  createdAt: string;
  skills: PackagedSkill[];
  missingCapabilities: string[];
}
export interface SkillPackagePreview {
  digest: string;
  name: string;
  skillCount: number;
  fileCount: number;
  totalBytes: number;
  skills: Omit<PackagedSkill, "id" | "enabled">[];
  missingCapabilities: string[];
  warnings: string[];
}
export interface SkillResource {
  path: string;
  sizeBytes: number;
  sha256: string;
  text: boolean;
  kind: "instructions" | "reference" | "script" | "resource";
}
export interface SkillResourcePage {
  skillId: number;
  path: string;
  content: string | null;
  encoding: "utf8" | "binary";
  offset: number;
  nextOffset: number | null;
  hasMore: boolean;
  totalBytes: number;
}
