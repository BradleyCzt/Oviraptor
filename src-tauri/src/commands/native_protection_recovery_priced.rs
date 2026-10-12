// Versioned explicit pricing approval, still pending a dedicated birth and
// verified financial consumers. Nothing here grants or resumes execution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeRecoveryPricingInput {
    pub schema_version: u8,
    pub max_budget_usd: String,
    pub price_revision: String,
    pub uncached_input: crate::recovery_finance::Money,
    pub cached_input: crate::recovery_finance::Money,
    pub output: crate::recovery_finance::Money,
    pub per_request: crate::recovery_finance::Money,
}
impl NativeRecoveryPricingInput {
    fn freeze(
        &self,
        model: &JsonValue,
        scope: &NativeProtectionRecoveryScope,
    ) -> Result<crate::recovery_finance::FinancialTerms, String> {
        use crate::recovery_finance::{FinancialTerms, Money, RateCard};
        if self.schema_version != 1 || scope.urls.len() != 1 {
            return Err("protection_recovery_pricing_protocol_unsupported".into());
        }
        let limit = Money::parse_usd(&self.max_budget_usd).map_err(str::to_owned)?;
        // The legacy scope number is a display consistency check only. The
        // priced protocol freezes and later consumes the exact integer limit.
        let display: f64 = limit
            .usd()
            .parse()
            .map_err(|_| "protection_recovery_pricing_limit_invalid")?;
        if scope.max_budget_usd != display {
            return Err("protection_recovery_pricing_scope_limit_conflict".into());
        }
        let card = RateCard::new(
            crate::agent_runtime::store::stable_hash(&model.to_string()),
            self.price_revision.clone(),
            self.uncached_input,
            self.cached_input,
            self.output,
            self.per_request,
        )
        .map_err(str::to_owned)?;
        FinancialTerms::new(limit, card).map_err(str::to_owned)
    }

    fn freeze_for_new_proposal(
        &self,
        model: &JsonValue,
        scope: &NativeProtectionRecoveryScope,
    ) -> Result<crate::recovery_finance::FinancialTerms, String> {
        let terms = self.freeze(model, scope)?;
        // Zero token bounds price only the fixed lower bound, not a complete
        // request reservation. Historical receipt reads keep using freeze.
        let fixed_charge = terms
            .pricing()
            .quote_upper_bound(terms.pricing().model_binding_hash(), 0, 0)
            .map_err(str::to_owned)?;
        if fixed_charge > terms.limit() {
            return Err("protection_recovery_fixed_call_exceeds_budget".into());
        }
        Ok(terms)
    }
}

struct NativePricedRecoveryProposal {
    original: NativeProtectionRecoveryProposal,
    fact: JsonValue,
    pricing: NativeRecoveryPricingInput,
}
impl NativePricedRecoveryProposal {
    fn verify_on(
        &self,
        db: &rusqlite::Connection,
        path: &Path,
        selected: &NativeProtectionRecoverySelection,
        scope: &NativeProtectionRecoveryScope,
    ) -> Result<(), String> {
        self.original.verify_on(db, path, selected, scope)?;
        if self.fact["financialTerms"]
            != json!(self
                .pricing
                .freeze_for_new_proposal(&self.original.fact["modelBinding"], scope)?)
            || self.fact["pricingInput"] != json!(self.pricing)
        {
            return Err("protection_recovery_pricing_contract_changed".into());
        }
        Ok(())
    }
}
fn native_priced_recovery_proposal_in(
    db: &rusqlite::Connection,
    path: &Path,
    selected: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
    pricing: &NativeRecoveryPricingInput,
) -> Result<NativePricedRecoveryProposal, String> {
    let original = native_protection_recovery_proposal_in(db, path, selected, scope)?;
    let terms = pricing.freeze_for_new_proposal(&original.fact["modelBinding"], scope)?;
    let mut fact = original.fact.clone();
    fact.as_object_mut()
        .ok_or("protection_recovery_proposal_changed")?
        .remove("proposalHash");
    fact["schemaVersion"] = json!(2);
    fact["purpose"] = json!("explicit_priced_new_target_work_after_original_protection_stop");
    fact["financialTerms"] = json!(terms);
    fact["pricingInput"] = json!(pricing);
    fact["legacyScopeBudgetUsdIsDisplayOnly"] = json!(true);
    fact["pricingBasis"] = json!("approved_rate_card_not_provider_dollar_invoice");
    fact["proposalHash"] = json!(crate::agent_runtime::store::stable_hash(&fact.to_string()));
    Ok(NativePricedRecoveryProposal {
        original,
        fact,
        pricing: pricing.clone(),
    })
}
fn native_priced_recovery_approval_id(hash: &str) -> String {
    crate::agent_runtime::store::stable_hash(&format!(
        "oviraptor.explicit-protection-recovery-approval.v2:{hash}"
    ))
}
const PRICED_RECOVERY_CONSENT: &str = "approve_exact_priced_new_scope_preserve_old_stop_and_costs";

fn native_priced_recovery_schema_on(db: &rusqlite::Connection) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_priced_schema.sql"))
        .map_err(|e| e.to_string())?;
    let read =
        |db: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q = db
                .prepare(
                    "SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master
            WHERE tbl_name='native_protection_recovery_priced_approvals' ORDER BY type,name",
                )
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master
        WHERE name='native_protection_recovery_priced_approvals' OR tbl_name='native_protection_recovery_priced_approvals')",[],|r|r.get(0))
        .map_err(|e|e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_priced_schema_changed".into());
    }
    Ok(())
}

fn native_priced_recovery_saved_on(
    db: &rusqlite::Connection,
    request: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_priced_recovery_schema_on(db)?;
    let row: Option<NativeRecoveryApprovalStoredRow> = db.query_row("SELECT approval_id,request_id,
        source_scan_id,source_attempt_number,source_root_run_id,protection_receipt_id,proposal_hash,
        approval_json,signature FROM main.native_protection_recovery_priced_approvals WHERE request_id=?1",[request],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))
        .optional().map_err(|e|e.to_string())?;
    let Some((id, request, scan, attempt, root, receipt, hash, text, signature)) = row else {
        return Ok(None);
    };
    let fact: JsonValue =
        serde_json::from_str(&text).map_err(|_| "protection_recovery_priced_approval_corrupt")?;
    let mut proposal = fact["proposal"].clone();
    let saved_hash = proposal
        .as_object_mut()
        .ok_or("protection_recovery_priced_approval_corrupt")?
        .remove("proposalHash")
        .ok_or("protection_recovery_priced_approval_corrupt")?;
    let scope: NativeProtectionRecoveryScope = serde_json::from_value(proposal["newScope"].clone())
        .map_err(|_| "protection_recovery_priced_approval_corrupt")?;
    let pricing: NativeRecoveryPricingInput =
        serde_json::from_value(proposal["pricingInput"].clone())
            .map_err(|_| "protection_recovery_priced_approval_corrupt")?;
    let terms: crate::recovery_finance::FinancialTerms =
        serde_json::from_value(proposal["financialTerms"].clone())
            .map_err(|_| "protection_recovery_priced_approval_corrupt")?;
    let canonical = fact.to_string();
    if canonical != text
        || fact["schemaVersion"] != 2
        || fact["purpose"] != "explicit_priced_new_target_work_approval"
        || fact["approvalId"] != id
        || fact["requestId"] != request
        || fact["proposalHash"] != hash
        || fact["consent"] != PRICED_RECOVERY_CONSENT
        || fact["status"] != "approved_pending_priced_fresh_birth"
        || fact["executionAllowed"] != false
        || fact["targetRequestsGranted"] != 0
        || fact["oldScopeRenewed"] != false
        || proposal["schemaVersion"] != 2
        || proposal["purpose"] != "explicit_priced_new_target_work_after_original_protection_stop"
        || proposal["selection"]["scanId"] != scan
        || proposal["selection"]["attemptNumber"] != attempt
        || proposal["selection"]["rootRunId"] != root
        || proposal["selection"]["protectionReceiptId"] != receipt
        || saved_hash != hash
        || hash != crate::agent_runtime::store::stable_hash(&proposal.to_string())
        || id != native_priced_recovery_approval_id(&hash)
        || terms != pricing.freeze(&proposal["modelBinding"], &scope)?
    {
        return Err("protection_recovery_priced_approval_corrupt".into());
    }
    recovery_approval_signature::verify_saved_on(db, &fact, &signature)?;
    Ok(Some(fact))
}

struct NativePricedRecoveryWriter<'a>(&'a rusqlite::Connection);
impl<'a> NativePricedRecoveryWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        native_priced_recovery_schema_on(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "native_protection_recovery_priced_approvals",
            } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
            AuthAction::Pragma {
                pragma_name: "database_list",
                ..
            } => Authorization::Allow,
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }
}
impl Drop for NativePricedRecoveryWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

fn native_priced_recovery_preview_on(
    db: &mut rusqlite::Connection,
    path: &Path,
    selected: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
    pricing: &NativeRecoveryPricingInput,
) -> Result<JsonValue, String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let proof = native_priced_recovery_proposal_in(&tx, path, selected, scope, pricing)?;
    let fact = proof.fact.clone();
    proof.verify_on(&tx, path, selected, scope)?;
    tx.commit().map_err(|e| e.to_string())?;
    drop(proof);
    Ok(fact)
}

fn native_priced_recovery_approve_on(
    path: &Path,
    selected: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
    pricing: &NativeRecoveryPricingInput,
    request: &str,
    hash: &str,
    consent: &str,
) -> Result<JsonValue, String> {
    if uuid::Uuid::parse_str(request).is_err()
        || hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || consent != PRICED_RECOVERY_CONSENT
    {
        return Err("protection_recovery_priced_explicit_approval_required".into());
    }
    let _approval_queue = crate::agent_runtime::execution_owner::queue_recovery_approval(path)?;
    let mut db = native_protection_recovery_open(path, true)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_priced_recovery_schema_on(&tx)?;
    native_priced_recovery_cancel_schema_on(&tx)?;
    if let Some(saved) = native_priced_recovery_saved_on(&tx, request)? {
        if native_priced_recovery_cancel_saved_on(&tx, request)?.is_some() {
            return Err("protection_recovery_priced_approval_revoked".into());
        }
        if saved["proposalHash"] != hash
            || saved["proposal"]["selection"] != json!(selected)
            || saved["proposal"]["newScope"] != json!(scope)
            || saved["proposal"]["pricingInput"] != json!(pricing)
        {
            return Err("protection_recovery_priced_approval_conflict".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(saved);
    }
    // Another window may have approved this source under a different request
    // UUID. Read its verified immutable receipt while holding the same write
    // transaction; never replace it or mint a second approval.
    if let Some(saved) = native_priced_recovery_for_source_on(&tx, selected)? {
        let original_request = saved["requestId"].as_str()
            .ok_or("protection_recovery_priced_approval_corrupt")?;
        if native_priced_recovery_cancel_saved_on(&tx, original_request)?.is_some() {
            return Err("protection_recovery_priced_approval_revoked".into());
        }
        return Err("protection_recovery_priced_source_already_approved".into());
    }
    let proof = native_priced_recovery_proposal_in(&tx, path, selected, scope, pricing)?;
    if proof.fact["proposalHash"] != hash {
        return Err("protection_recovery_priced_proposal_changed".into());
    }
    let approved = recovery_approval_signature::FreshProtectionRecoveryApproval::approve_priced_in(
        &tx, path, &proof, selected, scope, request, consent,
    )?;
    let (value, signature) = approved.sealed()?;
    let writer = NativePricedRecoveryWriter::install(&tx)?;
    let n = tx.execute("INSERT INTO main.native_protection_recovery_priced_approvals
        (approval_id,request_id,source_scan_id,source_attempt_number,source_root_run_id,protection_receipt_id,
        proposal_hash,approval_json,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![native_priced_recovery_approval_id(hash),request,selected.scan_id,selected.attempt_number,
            selected.root_run_id,selected.protection_receipt_id,hash,value.to_string(),signature]).map_err(|e|e.to_string())?;
    if n != 1 || native_priced_recovery_saved_on(&tx, request)?.as_ref() != Some(&value) {
        return Err("protection_recovery_priced_approval_not_saved".into());
    }
    proof.verify_on(&tx, path, selected, scope)?;
    drop(writer);
    tx.commit().map_err(|e| e.to_string())?;
    drop(proof);
    Ok(value)
}

#[tauri::command]
pub fn preview_native_priced_protection_recovery(
    state: State<AppState>,
    selection: NativeProtectionRecoverySelection,
    new_scope: NativeProtectionRecoveryScope,
    pricing: NativeRecoveryPricingInput,
) -> Result<JsonValue, String> {
    let mut db = native_protection_recovery_open(&state.db_path, false)?;
    native_priced_recovery_preview_on(&mut db, &state.db_path, &selection, &new_scope, &pricing)
}
#[tauri::command]
pub fn approve_native_priced_protection_recovery(
    state: State<AppState>,
    selection: NativeProtectionRecoverySelection,
    new_scope: NativeProtectionRecoveryScope,
    pricing: NativeRecoveryPricingInput,
    request_id: String,
    proposal_hash: String,
    consent: String,
) -> Result<JsonValue, String> {
    native_priced_recovery_approve_on(
        &state.db_path,
        &selection,
        &new_scope,
        &pricing,
        &request_id,
        &proposal_hash,
        &consent,
    )
}
#[tauri::command]
pub fn get_native_priced_protection_recovery_approval(
    state: State<AppState>,
    request_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = native_priced_recovery_saved_on(&tx, &request_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}


// Historical lookup does not require a browser cache, a currently enabled
// provider, or a still-live new scope. It verifies the original saved signature.
fn native_priced_recovery_for_source_on(
    db: &rusqlite::Connection,
    selection: &NativeProtectionRecoverySelection,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_priced_recovery_schema_on(db)?;
    let request: Option<String> = db.query_row(
        "SELECT request_id FROM main.native_protection_recovery_priced_approvals
         WHERE source_scan_id=?1 AND source_attempt_number=?2 AND source_root_run_id=?3
          AND protection_receipt_id=?4",
        params![selection.scan_id, selection.attempt_number, selection.root_run_id, selection.protection_receipt_id],
        |row| row.get(0),
    ).optional().map_err(|e| e.to_string())?;
    request.map(|request| native_priced_recovery_saved_on(db, &request))
        .transpose().map(Option::flatten)
}

#[tauri::command]
pub fn find_native_priced_protection_recovery_approval(
    state: State<AppState>,
    selection: NativeProtectionRecoverySelection,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = native_priced_recovery_for_source_on(&tx, &selection)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
