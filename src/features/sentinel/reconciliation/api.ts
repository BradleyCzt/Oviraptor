import { invoke } from "@tauri-apps/api/core";
import type { RootCostInventory, RootCostReceipt } from "./contract";
import type { RootContinuationPreview } from "./continuation";
import type { RootContinuationIssuance } from "./issuance";
import type { RootContinuationDiscovery } from "./discovery";

export const rootCostReconciliationApi = {
  discoverNativeRootFinancialContinuation: (scanId: string, attemptNumber: number) =>
    invoke<RootContinuationDiscovery>("discover_native_root_financial_continuation", { scanId, attemptNumber }),
  approveNativeRootFinancialContinuation: (scanId: string, attemptNumber: number, rootRunId: string,
    receiptId: string, proposalHash: string) => invoke<RootContinuationIssuance>("approve_native_root_financial_continuation",
      { scanId, attemptNumber, rootRunId, receiptId, proposalHash }),
  getNativeRootFinancialContinuation: (scanId: string, attemptNumber: number, rootRunId: string,
    receiptId: string, proposalHash: string) => invoke<RootContinuationIssuance>("get_native_root_financial_continuation",
      { scanId, attemptNumber, rootRunId, receiptId, proposalHash }),
  previewNativeRootFinancialContinuation: (scanId: string, attemptNumber: number, rootRunId: string, receiptId: string) =>
    invoke<RootContinuationPreview>("preview_native_root_financial_continuation", { scanId, attemptNumber, rootRunId, receiptId }),
  getNativeRootCostReconciliations: (scanId: string, attemptNumber: number) =>
    invoke<RootCostInventory>("get_native_root_cost_reconciliations", { scanId, attemptNumber }),
  confirmNativeRootCostReconciliation: (scanId: string, attemptNumber: number,
    rootRunId: string, callId: string, confirmationHash: string) =>
    invoke<RootCostReceipt>("confirm_native_root_cost_reconciliation",
      { scanId, attemptNumber, rootRunId, callId, confirmationHash }),
};
