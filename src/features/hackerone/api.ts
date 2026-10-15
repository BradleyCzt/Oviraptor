import { invoke } from "@tauri-apps/api/core";
import type { HackerOneDetail, HackerOneEvent, HackerOneProgram } from "./types";

export const hackerOneApi = {
  listHackerOnePrograms: (search = "") =>
    invoke<HackerOneProgram[]>("list_hackerone_programs", { search }),
  getHackerOneDetail: (handle: string) =>
    invoke<HackerOneDetail>("get_hackerone_detail", { handle }),
  syncHackerOne: (profileId: number, handle?: string) =>
    invoke<string>("sync_hackerone", { profileId, handle }),
  setHackerOneBookmark: (handle: string, bookmarked: boolean) =>
    invoke<void>("set_hackerone_bookmark", { handle, bookmarked }),
  listHackerOneEvents: (limit = 100) =>
    invoke<HackerOneEvent[]>("list_hackerone_events", { limit }),
  addHackerOneScopesToProject: (handle: string, projectId: number) =>
    invoke<number>("add_hackerone_scopes_to_project", { handle, projectId }),
};
