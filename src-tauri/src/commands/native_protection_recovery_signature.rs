//! Opaque explicit approval envelope. Cold JSON cannot enter the signing path.
use super::{
    JsonValue, NativePricedRecoveryProposal, NativeProtectionRecoveryProposal,
    NativeProtectionRecoveryScope, NativeProtectionRecoverySelection,
};
use crate::agent_runtime::web_mode::{ModeFact, VerifiedMode};
use rusqlite::Connection;
use std::path::Path;

pub(crate) struct FreshProtectionRecoveryApproval {
    value: JsonValue,
    original: VerifiedMode,
}
impl FreshProtectionRecoveryApproval {
    pub(super) fn approve_priced_in(
        db: &Connection,
        db_path: &Path,
        proof: &NativePricedRecoveryProposal,
        selected: &NativeProtectionRecoverySelection,
        scope: &NativeProtectionRecoveryScope,
        request: &str,
        consent: &str,
    ) -> Result<Self, String> {
        if db.is_autocommit()
            || uuid::Uuid::parse_str(request).is_err()
            || consent != super::PRICED_RECOVERY_CONSENT
            || proof.fact["selection"] != serde_json::json!(selected)
            || proof.fact["newScope"] != serde_json::json!(scope)
        {
            return Err("protection_recovery_priced_explicit_approval_required".into());
        }
        proof.verify_on(db, db_path, selected, scope)?;
        let original = original_mode_on(db, selected)?;
        let hash = proof.fact["proposalHash"]
            .as_str()
            .ok_or("protection_recovery_priced_proposal_changed")?;
        let value = serde_json::json!({"schemaVersion":2,"purpose":"explicit_priced_new_target_work_approval",
            "approvalId":super::native_priced_recovery_approval_id(hash),"requestId":request,
            "proposalHash":hash,"consent":consent,"proposal":proof.fact,
            "originalModeReceiptId":original.fact().receipt_id,
            "status":"approved_pending_priced_fresh_birth","executionAllowed":false,
            "targetRequestsGranted":0,"oldScopeRenewed":false});
        Ok(Self { value, original })
    }

    pub(super) fn approve_in(
        db: &Connection,
        db_path: &Path,
        proof: &NativeProtectionRecoveryProposal,
        selected: &NativeProtectionRecoverySelection,
        scope: &NativeProtectionRecoveryScope,
        request: &str,
        consent: &str,
    ) -> Result<Self, String> {
        if db.is_autocommit()
            || uuid::Uuid::parse_str(request).is_err()
            || consent != "approve_exact_new_scope_preserve_old_stop_and_costs"
            || proof.fact["selection"] != serde_json::json!(selected)
            || proof.fact["newScope"] != serde_json::json!(scope)
        {
            return Err("protection_recovery_explicit_approval_required".into());
        }
        proof.verify_on(db, db_path, selected, scope)?;
        let original = original_mode_on(db, selected)?;
        let hash = proof.fact["proposalHash"]
            .as_str()
            .ok_or("protection_recovery_proposal_changed")?;
        let value = serde_json::json!({"schemaVersion":1,"purpose":"explicit_new_target_work_approval",
            "approvalId":super::native_protection_recovery_approval_id(hash),"requestId":request,
            "proposalHash":hash,"consent":consent,"proposal":proof.fact,
            "originalModeReceiptId":original.fact().receipt_id,
            "status":"approved_pending_fresh_birth","executionAllowed":false,
            "targetRequestsGranted":0,"oldScopeRenewed":false});
        Ok(Self { value, original })
    }
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact {
            return Err("protection_recovery_original_mode_changed".into());
        }
        message(&self.value)
    }
    pub(super) fn sealed(&self) -> Result<(JsonValue, Vec<u8>), String> {
        Ok((
            self.value.clone(),
            self.original.sign_new_recovery_approval(self)?,
        ))
    }
}

fn message(value: &JsonValue) -> Result<Vec<u8>, String> {
    let domain = match value["schemaVersion"].as_u64() {
        Some(1) => "oviraptor.explicit-protection-recovery-approval.v1",
        Some(2) => "oviraptor.explicit-protection-recovery-approval.v2",
        _ => return Err("protection_recovery_approval_protocol_unsupported".into()),
    };
    serde_json::to_vec(&serde_json::json!({"domain":domain,"approval":value}))
        .map_err(|e| e.to_string())
}
fn original_mode_on(
    db: &Connection,
    selection: &NativeProtectionRecoverySelection,
) -> Result<VerifiedMode, String> {
    let original = super::original_root_capture_mode(db, &selection.root_run_id)?;
    if original.fact().scan_id != selection.scan_id || original.fact().attempt_number != selection.attempt_number {
        return Err("protection_recovery_original_mode_changed".into());
    }
    Ok(original)
}
pub(super) fn verify_saved_on(
    db: &Connection,
    value: &JsonValue,
    signature: &[u8],
) -> Result<(), String> {
    let selection: NativeProtectionRecoverySelection =
        serde_json::from_value(value["proposal"]["selection"].clone())
            .map_err(|_| "protection_recovery_approval_corrupt")?;
    let original = original_mode_on(db, &selection)?;
    if value["originalModeReceiptId"] != original.fact().receipt_id {
        return Err("protection_recovery_original_mode_changed".into());
    }
    original.verify_saved_recovery_approval(&message(value)?, signature)
}

// Only a newly registered mode draft in the caller's creation transaction can
// enter this signer. Persisted JSON has a verification path, never a resealer.
pub(crate) struct FreshProtectionRecoveryDraft {
    value: JsonValue,
    original: VerifiedMode,
}
impl FreshProtectionRecoveryDraft {
    pub(super) fn create_in(
        fresh: &crate::agent_runtime::web_mode::draft::FreshModeDraft<'_, '_>,
        db_path: &Path,
        proof: &NativeProtectionRecoveryProposal,
        approved: &JsonValue,
    ) -> Result<Self, String> {
        let db = fresh.transaction();
        let selected: NativeProtectionRecoverySelection =
            serde_json::from_value(approved["proposal"]["selection"].clone())
                .map_err(|_| "protection_recovery_approval_corrupt")?;
        let scope: NativeProtectionRecoveryScope =
            serde_json::from_value(approved["proposal"]["newScope"].clone())
                .map_err(|_| "protection_recovery_approval_corrupt")?;
        proof.verify_on(db, db_path, &selected, &scope)?;
        let request = approved["requestId"]
            .as_str()
            .ok_or("protection_recovery_approval_corrupt")?;
        if super::native_protection_recovery_saved_on(db, request)?.as_ref() != Some(approved)
            || approved["proposal"] != proof.fact
            || fresh.scan_id() == selected.scan_id
        {
            return Err("protection_recovery_draft_approval_changed".into());
        }
        let policy: String = db
            .query_row(
                "SELECT policy_json FROM main.sentinel_scan_contexts WHERE scan_id=?1",
                [fresh.scan_id()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let value = serde_json::json!({"schemaVersion":1,"purpose":"explicit_protection_recovery_new_draft",
            "scopeBudget":super::native_protection_recovery_scope_budget_fact(fresh.scan_id(),fresh.draft_id(),approved)?,
            "approval":approved,"scanId":fresh.scan_id(),"draftId":fresh.draft_id(),
            "policyJson":policy,"status":"draft_pending_dedicated_fresh_birth",
            "identitiesBound":false,"executionAllowed":false,"targetRequestsGranted":0,"oldScopeRenewed":false});
        super::native_protection_recovery_draft_pristine_scope_on(db, &value)?;
        Ok(Self {
            value,
            original: original_mode_on(db, &selected)?,
        })
    }
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact {
            return Err("protection_recovery_original_mode_changed".into());
        }
        draft_message(&self.value)
    }
    pub(super) fn sealed(&self) -> Result<(JsonValue, Vec<u8>), String> {
        Ok((
            self.value.clone(),
            self.original.sign_new_recovery_draft(self)?,
        ))
    }
}
fn draft_message(value: &JsonValue) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&serde_json::json!({"domain":"oviraptor.explicit-protection-recovery-draft.v1","draft":value}))
        .map_err(|e|e.to_string())
}
pub(super) fn verify_saved_draft_on(
    db: &Connection,
    value: &JsonValue,
    signature: &[u8],
) -> Result<(), String> {
    let selected: NativeProtectionRecoverySelection =
        serde_json::from_value(value["approval"]["proposal"]["selection"].clone())
            .map_err(|_| "protection_recovery_draft_corrupt")?;
    original_mode_on(db, &selected)?
        .verify_saved_recovery_approval(&draft_message(value)?, signature)
        .map_err(|_| "protection_recovery_draft_signature_invalid".into())
}

pub(crate) struct FreshProtectionRecoveryCancellation {
    value: JsonValue,
    original: VerifiedMode,
}
impl FreshProtectionRecoveryCancellation {
    pub(super) fn cancel_in(db: &Connection, draft: &JsonValue) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("protection_recovery_snapshot_required".into());
        }
        super::native_protection_recovery_draft_pristine_scope_on(db, draft)?;
        let request = draft["approval"]["requestId"]
            .as_str()
            .ok_or("protection_recovery_draft_corrupt")?;
        if super::native_protection_recovery_draft_saved_on(db, request)?.as_ref() != Some(draft) {
            return Err("protection_recovery_cancel_draft_changed".into());
        }
        let selected: NativeProtectionRecoverySelection =
            serde_json::from_value(draft["approval"]["proposal"]["selection"].clone())
                .map_err(|_| "protection_recovery_draft_corrupt")?;
        let value = serde_json::json!({"schemaVersion":1,"purpose":"cancel_unconsumed_protection_recovery_draft",
            "cancellationId":uuid::Uuid::new_v4().to_string(),"scanId":draft["scanId"],"draftId":draft["draftId"],
            "approvalId":draft["approval"]["approvalId"],"requestId":request,
            "draftHash":crate::agent_runtime::store::stable_hash(&draft.to_string()),
            "reason":"user_cancelled_before_fresh_birth","authorityRevoked":true,
            "executionAllowed":false,"targetRequestsGranted":0,"oldStopRetained":true,"oldCostsRetained":true});
        Ok(Self {
            value,
            original: original_mode_on(db, &selected)?,
        })
    }
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact {
            return Err("protection_recovery_original_mode_changed".into());
        }
        cancellation_message(&self.value)
    }
    pub(super) fn sealed(&self) -> Result<(JsonValue, Vec<u8>), String> {
        Ok((
            self.value.clone(),
            self.original.sign_new_recovery_cancellation(self)?,
        ))
    }
}
fn cancellation_message(value: &JsonValue) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&serde_json::json!({"domain":"oviraptor.explicit-protection-recovery-cancellation.v1","cancellation":value}))
        .map_err(|e|e.to_string())
}
pub(super) fn verify_saved_cancellation_on(
    db: &Connection,
    draft: &JsonValue,
    value: &JsonValue,
    signature: &[u8],
) -> Result<(), String> {
    let selected: NativeProtectionRecoverySelection =
        serde_json::from_value(draft["approval"]["proposal"]["selection"].clone())
            .map_err(|_| "protection_recovery_draft_corrupt")?;
    original_mode_on(db, &selected)?
        .verify_saved_recovery_approval(&cancellation_message(value)?, signature)
        .map_err(|_| "protection_recovery_cancellation_signature_invalid".into())
}

// Only a verified stored v2 approval may produce this separate-domain revoke.
pub(crate) struct FreshPricedRecoveryCancellation {
    value: JsonValue,
    original: VerifiedMode,
}
impl FreshPricedRecoveryCancellation {
    pub(super) fn cancel_in(db: &Connection, approval: &JsonValue) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("protection_recovery_snapshot_required".into());
        }
        let request = approval["requestId"]
            .as_str()
            .ok_or("protection_recovery_priced_approval_corrupt")?;
        if super::native_priced_recovery_saved_on(db, request)?.as_ref() != Some(approval) {
            return Err("protection_recovery_priced_cancel_approval_changed".into());
        }
        let selected: NativeProtectionRecoverySelection =
            serde_json::from_value(approval["proposal"]["selection"].clone())
                .map_err(|_| "protection_recovery_priced_approval_corrupt")?;
        let original = original_mode_on(db, &selected)?;
        let value = super::native_priced_recovery_cancellation_value(approval);
        Ok(Self { value, original })
    }
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact {
            return Err("protection_recovery_original_mode_changed".into());
        }
        priced_cancellation_message(&self.value)
    }
    pub(super) fn sealed(&self) -> Result<(JsonValue, Vec<u8>), String> {
        Ok((
            self.value.clone(),
            self.original.sign_new_priced_recovery_cancellation(self)?,
        ))
    }
}
fn priced_cancellation_message(value: &JsonValue) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&serde_json::json!({"domain":"oviraptor.priced-protection-recovery-cancellation.v1","cancellation":value}))
        .map_err(|e|e.to_string())
}
pub(super) fn verify_priced_cancellation_on(
    db: &Connection,
    approval: &JsonValue,
    value: &JsonValue,
    signature: &[u8],
) -> Result<(), String> {
    let selected: NativeProtectionRecoverySelection =
        serde_json::from_value(approval["proposal"]["selection"].clone())
            .map_err(|_| "protection_recovery_priced_approval_corrupt")?;
    let original = original_mode_on(db, &selected)?;
    if value["originalModeReceiptId"] != original.fact().receipt_id {
        return Err("protection_recovery_original_mode_changed".into());
    }
    original
        .verify_saved_recovery_approval(&priced_cancellation_message(value)?, signature)
        .map_err(|_| "protection_recovery_priced_cancellation_signature_invalid".into())
}
