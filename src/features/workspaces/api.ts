import { invoke } from "@tauri-apps/api/core";
import type { Project, ProjectImpact } from "./types";

export const workspaceApi = {
  listProjects: () => invoke<Project[]>("list_projects"),
  saveProject: (input: { id?: number; name: string; description: string }) =>
    invoke<number>("save_project", { input }),
  projectImpact: (projectId: number) =>
    invoke<ProjectImpact>("project_impact", { projectId }),
  archiveProject: (projectId: number, archived: boolean) =>
    invoke<void>("archive_project", { projectId, archived }),
  deleteProject: (projectId: number) =>
    invoke<void>("delete_project", { projectId }),
};
