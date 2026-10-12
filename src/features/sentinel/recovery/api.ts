import { invoke } from "@tauri-apps/api/core";
import type { RecoveryOperation, RecoveryScope, RecoverySelection } from "./contract";
import type { PricedRecoveryOperation, RecoveryPricingInput } from "./priced";
export const protectionRecoveryApi = {
  pricedApprovalForSource: (selection: RecoverySelection) => invoke<unknown>("find_native_priced_protection_recovery_approval", { selection }),
  pricedCancel: (requestId: string, approvalId: string) => invoke<unknown>("cancel_native_priced_protection_recovery_approval", {
    requestId, approvalId, consent: "revoke_exact_pending_priced_approval_preserve_original_history",
  }),
  pricedCancellation: (requestId: string) => invoke<unknown>("get_native_priced_protection_recovery_cancellation", { requestId }),
  pricedPreview: (selection: RecoverySelection, newScope: RecoveryScope, pricing: RecoveryPricingInput) =>
    invoke<unknown>("preview_native_priced_protection_recovery", { selection, newScope, pricing }),
  pricedApprove: (op: PricedRecoveryOperation) => invoke<unknown>("approve_native_priced_protection_recovery", {
    selection: op.selection, newScope: op.newScope, pricing: op.pricing, requestId: op.requestId,
    proposalHash: op.proposalHash, consent: "approve_exact_priced_new_scope_preserve_old_stop_and_costs",
  }),
  pricedApproval: (requestId: string) => invoke<unknown>("get_native_priced_protection_recovery_approval", { requestId }),
  sources: (scanId: string) => invoke<unknown>("discover_native_protection_recovery_sources", { scanId }),
  preview: (selection: RecoverySelection, newScope: RecoveryScope) => invoke<unknown>("preview_native_protection_recovery_proposal", { selection, newScope }),
  approve: (op: RecoveryOperation) => invoke<unknown>("approve_native_protection_recovery", {
    selection: op.selection, newScope: op.newScope, requestId: op.requestId, proposalHash: op.proposalHash,
    consent: "approve_exact_new_scope_preserve_old_stop_and_costs",
  }),
  approval: (requestId: string) => invoke<unknown>("get_native_protection_recovery_approval", { requestId }),
  createDraft: (requestId: string) => invoke<unknown>("create_native_protection_recovery_draft", { requestId }),
  draft: (requestId: string) => invoke<unknown>("get_native_protection_recovery_draft", { requestId }),
  cancellation: (requestId: string) => invoke<unknown>("get_native_protection_recovery_cancellation_by_request", { requestId }),
  cancel: (scanId: string) => invoke<void>("cancel_sentinel_scan", { scanId }),
};
