//! Immutable financial issuance binding, never a live execution grant.
use crate::agent_runtime::store;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub(crate) const SYSTEM: &str = "You are the Root Coordinator. Perform one fresh assessment of the original frozen Web evidence for this explicitly approved financial continuation. Return one strict DecisionSummary JSON. No target actions, commands, tools, credentials, reasoning or assistant drafts. Suggestions are advisory only; no children or target work will be dispatched by this assessment. Every list <=16; every string <=512 characters; schemaVersion=1.";

pub(crate) const PURPOSE: &str = "coordinator_financial_continuation_v1";

pub(crate) fn binding_on(db: &Connection, root: &str) -> Result<Option<Value>, String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return binding_on(&snapshot, root);
    }
    let table: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table'
        AND name='agent_financial_continuations')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !table {
        return Ok(None);
    }
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_financial_continuations
        WHERE new_root_run_id=?1)",
            params![root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Ok(None);
    }
    let birth = crate::commands::verify_financial_continuation_birth_on(db, root)?;
    Ok(Some(json!({"version":1,"approvalId":birth["approvalId"],
        "proposalHash":birth["proposalHash"],"birthHash":store::stable_hash(&birth.to_string()),
        "sourceRootRunId":birth["sourceRootRunId"],"sourceAttemptNumber":birth["sourceAttemptNumber"],
        "sourceReceiptId":birth["sourceReceiptId"],"newRootRunId":birth["newRootRunId"],
        "newAttemptNumber":birth["newAttemptNumber"],"taskDescription":birth["taskDescription"],
        "originalDeadline":birth["originalDeadline"]})))
}

pub(super) fn verify_on(
    db: &Connection,
    root: &str,
    round: i64,
    basis: &Value,
    request: &Value,
) -> Result<(), String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return verify_on(&snapshot, root, round, basis, request);
    }
    let Some(binding) = binding_on(db, root)? else {
        if request["purpose"] == PURPOSE
            || basis.get("financialContinuation").is_some()
            || request.get("financialContinuation").is_some()
        {
            return Err("financial_continuation_request_not_issued".into());
        }
        return Ok(());
    };
    if round != 1
        || request["purpose"] != PURPOSE
        || basis["phase"] != "financial-continuation"
        || basis["financialContinuation"] != binding
        || request["financialContinuation"] != binding
        || basis["taskDescription"] != binding["taskDescription"]
        || basis["attempt"] != binding["newAttemptNumber"]
        || basis["root"] != root
        || request["root"] != root
        || request["basisHash"] != store::stable_hash(&basis.to_string())
        || request["tools"] != json!([])
        || [
            "localDeliberation",
            "localParents",
            "localHistory",
            "localRound",
            "bootstrapDispatch",
            "permittedSuggestion",
            "changedFact",
            "liveBudgetObservation",
            "newEvidenceSupervision",
            "budgetSnapshot",
        ]
        .iter()
        .any(|key| basis.get(*key).is_some())
    {
        return Err("financial_continuation_request_binding_changed".into());
    }
    let messages = request["messages"]
        .as_array()
        .ok_or("financial_continuation_messages_invalid")?;
    if messages.len() != 2
        || messages[0]["role"] != "system"
        || messages[0]["content"] != SYSTEM
        || messages[1]["role"] != "user"
    {
        return Err("financial_continuation_messages_invalid".into());
    }
    let input: Value = serde_json::from_str(
        messages[1]["content"]
            .as_str()
            .ok_or("financial_continuation_messages_invalid")?,
    )
    .map_err(|_| "financial_continuation_messages_invalid")?;
    if input.as_object().is_none_or(|o| o.len() != 2)
        || messages
            .iter()
            .any(|m| m.as_object().is_none_or(|o| o.len() != 2))
        || input["basis"] != *basis
        || input["requiredOutput"] != super::decision::DecisionSummary::schema()
    {
        return Err("financial_continuation_messages_invalid".into());
    }
    let birth = crate::commands::verify_financial_continuation_birth_on(db, root)?;
    if basis["scan"] != birth["scanId"]
        || basis["target"] != birth["financialSource"]["targetKey"]
        || basis["planHash"] != birth["newPlanHash"]
        || basis["evidenceFileHash"] != birth["frontendEvidenceHash"]
    {
        return Err("financial_continuation_basis_changed".into());
    }
    crate::commands::native_coordinator_require_original_model_on(
        db,
        birth["sourceRootRunId"]
            .as_str()
            .ok_or("financial_continuation_birth_invalid")?,
        request,
    )
}
