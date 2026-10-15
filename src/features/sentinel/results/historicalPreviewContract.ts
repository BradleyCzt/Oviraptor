import type { HistoricalImportPreview } from "../types";

// Validates the normalized read-only API projection, never the original JSON.
// Multiple historical attempts are valid; they cannot select a native attempt.
export function validateHistoricalPreviews(value: unknown, scanId: string): HistoricalImportPreview[] {
  if (!Array.isArray(value) || value.length > 300 || value.some(row => !row
    || row.scanId !== scanId || row.reviewState !== "unreviewed"
    || row.readOnly !== true || row.executionEligible !== false)) {
    throw new Error("historical_preview_contract_mismatch");
  }
  return value;
}
