import { shellApi } from "./shell/api";
import { configurationApi } from "./features/configuration/api";
import { environmentApi } from "./features/environment/api";
import { remoteScanningApi } from "./features/sentinel/remoteWorkerApi";
import { conversationApi } from "./features/conversations/api";
import { assetApi } from "./features/assets/api";
import { ownershipAuthorizationApi } from "./features/assets/ownershipDirectoryApi";
import { hackerOneApi } from "./features/hackerone/api";
import { runtimeApi } from "./features/runtime/api";
import { sentinelApi } from "./features/sentinel/api";
import { workspaceApi } from "./features/workspaces/api";

// Stable facade for existing components. Business-specific commands live with
// their feature so adding an Agent command no longer expands the asset/runtime API.
export const api = {
  ...conversationApi,
  ...shellApi,
  ...configurationApi,
  ...environmentApi,
  ...remoteScanningApi,
  ...workspaceApi,
  ...assetApi,
  ...ownershipAuthorizationApi,
  ...hackerOneApi,
  ...sentinelApi,
  ...runtimeApi,
};

export type Api = typeof api;
