import type { AssetPage, AssetQuery, Project, SentinelScan, ViewKey } from "../types";

/** Trusted, bundled application extensions. Chat is part of the host. */
export type BuiltinPluginId = "asset" | "sentinel" | "hackerone";
export type BilingualText = readonly [string, string];
export type PluginEnabledState = Readonly<Record<BuiltinPluginId, boolean>>;
export interface PluginPageContribution {
  id: string;
  view: ViewKey;
  label: BilingualText;
  /** Only navigation metadata; components are literal imports owned by the host. */
  section?: "dialog" | "results" | "queue" | "workbench";
}
export interface PluginCapabilityDeclaration {
  id: string;
  label: BilingualText;
  requiresModel: boolean;
  requiresNetwork: boolean;
  requiredProviders: readonly string[];
  authority: "existing-domain-gates";
  /** Existing pages own these operations; declarations never grant execution. */
  hostCallable: false;
}
export type PluginCommandId = "assets.list-link-candidates" | "assets.open-workspace"
  | "scanning.list-link-candidates" | "scanning.open-workspace";
export interface PluginCommandDeclaration {
  id: PluginCommandId;
  kind: "read" | "navigate";
  scope: "selected-project";
  requiresModel: false;
  requiresNetwork: false;
  permission: "existing-project-read" | "host-navigation";
  implementation: "built-in";
}
export interface BuiltinPluginManifest {
  id: BuiltinPluginId;
  label: BilingualText;
  description: BilingualText;
  defaultView: ViewKey;
  views: readonly ViewKey[];
  hasLogs: boolean;
  requirements: readonly ("hackerone-profile")[];
  pages: readonly PluginPageContribution[];
  commands: readonly PluginCommandDeclaration[];
  capabilities: readonly PluginCapabilityDeclaration[];
}

/** Deliberately workspace-only: current pages do not support entity deep links. */
export type PluginNavigationIntent =
  | { pluginId: "asset"; projectId: number; page: "assets" }
  | { pluginId: "sentinel"; projectId: number; page: "sentinel"; section: "dialog" | "results" | "queue" | "workbench" };
export interface PluginLinkCandidate {
  kind: "asset" | "scan";
  targetId: string;
  projectId: number;
  label: string;
  detail: string;
}
/** Treat as opaque in a picker; reset it whenever plugin/project/search changes. */
export type PluginLinkCursor =
  | { pluginId: "asset"; projectId: number; search: string; page: number }
  | { pluginId: "sentinel"; projectId: number; search: string; before: { updatedAt: string; id: string } };
export interface PluginLinkInput {
  pluginId: "asset" | "sentinel";
  projectId: number;
  search?: string;
  cursor?: PluginLinkCursor;
}
export interface PluginLinkPage { items: PluginLinkCandidate[]; nextCursor?: PluginLinkCursor }
export interface PluginReadApi {
  listAssets(query: AssetQuery): Promise<AssetPage>;
  searchSentinelScanPage(projectId: number | undefined, search: string, view: "attention" | "history" | "all", limit?: number, before?: Pick<SentinelScan, "updatedAt" | "id">): Promise<SentinelScan[]>;
}
export interface BuiltinPluginHost {
  isEnabled(pluginId: BuiltinPluginId): boolean;
  listLinkCandidates(input: PluginLinkInput): Promise<PluginLinkPage>;
  openWorkspace(intent: PluginNavigationIntent): void;
  /** Call synchronously on project or enabled-state changes, even A -> B -> A. */
  invalidate(): void;
}
export interface BuiltinPluginHostOptions {
  api: PluginReadApi;
  enabled: () => PluginEnabledState;
  projectId: () => number | undefined;
  projects: () => readonly Pick<Project, "id" | "status">[];
  navigate: (intent: PluginNavigationIntent) => void;
}
