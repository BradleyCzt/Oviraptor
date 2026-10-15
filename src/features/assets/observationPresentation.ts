import type { Asset } from "./types";

const field = (asset: Asset, key: string): string => typeof asset.extra?.[key] === "string" ? asset.extra[key].trim() : "";
function instant(value: string): number | undefined {
  // A provider's unzoned date may be displayed, but cannot prove ordering.
  const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.exec(value);
  if (!parts) return;
  const [, year, month, day, hour, minute, second] = parts.map(Number);
  if (month < 1 || month > 12 || day < 1 || day > new Date(Date.UTC(year, month, 0)).getUTCDate()
    || hour > 23 || minute > 59 || second > 59) return;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

export function assetObservation(asset: Asset) {
  const sourceObservedAt = field(asset, "collection_provider_observed_at") || field(asset, "lastupdatetime");
  const collectionReceivedAt = field(asset, "collection_received_at");
  const probeCheckedAt = field(asset, "probe_checked_at");
  const historyReason = field(asset, "probe_history_reason");
  const probeObservedIp = field(asset, "probe_observed_ip");
  const probeObservedEndpoint = field(asset, "probe_observed_endpoint");
  const probeEndpointBasis = field(asset, "probe_observed_endpoint_basis");
  const received = instant(collectionReceivedAt), checked = instant(probeCheckedAt);
  const hasProbe = !!asset.probeOutcome?.trim();
  const state = !hasProbe ? "candidate"
    : asset.probeOutcome === "skipped" ? "skipped"
    : historyReason === "endpoint_changed" || historyReason === "collection_after_probe" ? "unverified"
    : historyReason === "unknown_probe_time" ? "historical"
    : historyReason ? "unknown"
    : received !== undefined && checked !== undefined && checked < received ? "unverified"
    : !probeCheckedAt ? "historical"
    : checked === undefined ? "unknown"
    : collectionReceivedAt && (received === undefined || checked === undefined) ? "unknown"
    : "recorded";
  return { sourceObservedAt, collectionReceivedAt, probeCheckedAt, historyReason, probeObservedIp, probeObservedEndpoint, probeEndpointBasis, state };
}
