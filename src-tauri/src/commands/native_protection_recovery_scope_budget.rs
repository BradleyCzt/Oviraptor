// A signed pending scope total, never a per-Root allocation or execution grant.
fn native_protection_recovery_scope_budget_fact(
    scan: &str,
    draft: &str,
    approval: &JsonValue,
) -> Result<JsonValue, String> {
    let scope: NativeProtectionRecoveryScope =
        serde_json::from_value(approval["proposal"]["newScope"].clone())
            .map_err(|_| "protection_recovery_scope_budget_scope_invalid")?;
    let basis = json!({"purpose":"shared_recovery_scope_budget","scanId":scan,"draftId":draft,
        "approvalId":approval["approvalId"],"requestId":approval["requestId"],
        "scope":scope});
    Ok(
        json!({"schemaVersion":1,"purpose":"shared_recovery_scope_budget",
        "ownerId":crate::agent_runtime::store::stable_hash(&basis.to_string()),
        "scanId":scan,"draftId":draft,"approvalId":approval["approvalId"],"requestId":approval["requestId"],
        "scopeHash":crate::agent_runtime::store::stable_hash(&json!(scope).to_string()),
        "urls":scope.urls,"dimensions":crate::agent_runtime::multi_agent::budget::DIMENSIONS,
        "limits":scope.limits,"maxBudgetUsd":scope.max_budget_usd,"deadlineUtc":scope.deadline_utc,
        "allocationPolicy":"one_shared_scan_total_across_all_roots",
        "wallTimePolicy":"scan_birth_clock_clipped_by_original_deadline",
        "status":"pending_dedicated_fresh_birth","executionAllowed":false,"targetRequestsGranted":0}),
    )
}

fn native_protection_recovery_scope_budget_schema_on(
    db: &rusqlite::Connection,
) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .map_err(|e| e.to_string())?;
    let read =
        |db: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q = db.prepare("SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master WHERE tbl_name='native_protection_recovery_scope_budgets' ORDER BY type,name").map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='native_protection_recovery_scope_budgets' OR tbl_name='native_protection_recovery_scope_budgets')",[],|r| r.get(0)).map_err(|e| e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_scope_budget_schema_changed".into());
    }
    Ok(())
}

fn native_protection_recovery_scope_budget_insert_on(
    db: &rusqlite::Connection,
    value: &JsonValue,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_scope_budget_schema_on(db)?;
    let fact = &value["scopeBudget"];
    let text = serde_json::to_string(fact).map_err(|e| e.to_string())?;
    let count = db.execute("INSERT INTO main.native_protection_recovery_scope_budgets(owner_id,scan_id,draft_id,contract_json,contract_hash) VALUES(?1,?2,?3,?4,?5)",
        params![fact["ownerId"].as_str(),value["scanId"].as_str(),value["draftId"].as_str(),text,crate::agent_runtime::store::stable_hash(&text)]).map_err(|e| e.to_string())?;
    if count != 1 {
        return Err("protection_recovery_scope_budget_not_saved".into());
    }
    Ok(())
}

type NativeProtectionRecoveryScopeBudgetRow = (String, String, String, String);
fn native_protection_recovery_scope_budget_verify_on(
    db: &rusqlite::Connection,
    value: &JsonValue,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    // Historical drafts keep their original signature and shape. No read-time
    // upgrade, clock start, allocation or new grant is supplied for them.
    let Some(fact) = value.get("scopeBudget") else {
        return Ok(());
    };
    native_protection_recovery_scope_budget_schema_on(db)?;
    let scan = value["scanId"]
        .as_str()
        .ok_or("protection_recovery_draft_corrupt")?;
    let draft = value["draftId"]
        .as_str()
        .ok_or("protection_recovery_draft_corrupt")?;
    let expected = native_protection_recovery_scope_budget_fact(scan, draft, &value["approval"])?;
    let row: Option<NativeProtectionRecoveryScopeBudgetRow> = db.query_row(
        "SELECT owner_id,draft_id,contract_json,contract_hash FROM main.native_protection_recovery_scope_budgets WHERE scan_id=?1",
        [scan], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|e| e.to_string())?;
    let (owner, stored_draft, text, hash) =
        row.ok_or("protection_recovery_scope_budget_missing")?;
    let canonical = serde_json::to_string(&expected).map_err(|e| e.to_string())?;
    if fact != &expected
        || fact["ownerId"] != owner
        || stored_draft != draft
        || text != canonical
        || hash != crate::agent_runtime::store::stable_hash(&canonical)
    {
        return Err("protection_recovery_scope_budget_binding_changed".into());
    }
    Ok(())
}
