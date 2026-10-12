//! Sealed issuance of exact remaining financial ceilings. A schema-three
//! declaration is never accepted by the ordinary fresh-budget validator.
use super::super::root_definition::NewRootBudgetDeclaration;
use crate::agent_runtime::store;
use crate::commands::FinancialContinuationAuthorization;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Definition {
    declaration: NewRootBudgetDeclaration,
    approval_id: String,
    source_root_run_id: String,
    source_receipt_id: String,
    source_financial_hash: String,
    shared_remaining_tokens: Option<i64>,
    shared_remaining_requests: Option<i64>,
    original_deadline: String,
    new_root_created_at: String,
    new_plan_physical_hash: String,
}

fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "migration_original_source_invalid".into())
}
fn cap(v: &Value) -> Result<Option<i64>, String> {
    match v {
        Value::Null => Ok(None),
        v => v
            .as_i64()
            .filter(|n| *n >= 0)
            .map(Some)
            .ok_or_else(|| "migration_remaining_limit_invalid".into()),
    }
}
fn remaining(facts: &Value) -> Result<[Option<i64>; 10], String> {
    let values = facts["remaining"]
        .as_array()
        .filter(|v| v.len() == 10)
        .ok_or("migration_remaining_dimensions_invalid")?;
    let mut result = [None; 10];
    for (index, value) in values.iter().enumerate() {
        result[index] = cap(value)?;
    }
    Ok(result)
}

fn wall_at_birth(db: &Connection, facts: &Value, birth: &str) -> Result<i64, String> {
    let left: i64 = db
        .query_row(
            "SELECT CAST((julianday(?1)-julianday(?2))*86400000 AS INTEGER)",
            params![string(facts, "originalDeadline")?, birth],
            |r| r.get(0),
        )
        .map_err(|_| "migration_original_deadline_invalid")?;
    let financial = remaining(facts)?[9].ok_or("migration_original_deadline_invalid")?;
    let wall = left.min(financial);
    if wall <= 0 {
        return Err("migration_original_deadline_expired".into());
    }
    Ok(wall)
}

struct NativeRoot {
    scan: String,
    attempt: i64,
    target: String,
    hash: String,
    text: String,
    birth: String,
    tokens: i64,
    requests: i64,
}
fn native(db: &Connection, root: &str) -> Result<NativeRoot, String> {
    db.query_row(
        "SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,created_at,
        hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1 AND backend='native'
        AND role='coordinator' AND orchestration_policy='multi' AND root_run_id=id
        AND assignment_id='' AND parent_run_id IS NULL",
        [root],
        |r| {
            Ok(NativeRoot {
                scan: r.get(0)?,
                attempt: r.get(1)?,
                target: r.get(2)?,
                hash: r.get(3)?,
                text: r.get(4)?,
                birth: r.get(5)?,
                tokens: r.get(6)?,
                requests: r.get(7)?,
            })
        },
    )
    .map_err(|_| "migration_new_root_scope_invalid".into())
}

fn validate(db: &Connection, root: &str, def: &Definition, facts: &Value) -> Result<(), String> {
    let row = native(db, root)?;
    let d = &def.declaration;
    let old_attempt = facts["attemptNumber"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("migration_original_source_invalid")?;
    let tokens = cap(&facts["sharedRemainingTokens"])?;
    let requests = cap(&facts["sharedRemainingRequests"])?;
    let mut expected = remaining(facts)?;
    expected[9] = Some(wall_at_birth(db, facts, &row.birth)?);
    if d.schema_version != 3
        || uuid::Uuid::parse_str(&d.declaration_id).is_err()
        || uuid::Uuid::parse_str(&def.approval_id).is_err()
        || d.execution_slot_capacity != 3
        || def.source_root_run_id == root
        || def.source_root_run_id != string(facts, "rootRunId")?
        || def.source_receipt_id != string(facts, "receiptId")?
        || def.source_financial_hash != store::stable_hash(&facts.to_string())
        || def.original_deadline != string(facts, "originalDeadline")?
        || def.new_root_created_at != row.birth
        || def.new_plan_physical_hash != store::stable_hash(&row.text)
        || def.shared_remaining_tokens != tokens
        || def.shared_remaining_requests != requests
        || d.scan_id != row.scan
        || d.scan_id != string(facts, "scanId")?
        || d.attempt_number != row.attempt
        || row.attempt
            != old_attempt
                .checked_add(1)
                .ok_or("migration_attempt_invalid")?
        || d.target_url != row.target
        || row.target != string(facts, "targetKey")?
        || d.plan_hash != row.hash
        || d.limits != expected
        || row.tokens != tokens.unwrap_or(0)
        || row.requests != requests.unwrap_or(0)
    {
        return Err("migration_frozen_contract_conflict".into());
    }
    let plan: Value = serde_json::from_str(&row.text).map_err(|_| "migration_plan_invalid")?;
    if plan["owner"] != "oviraptor"
        || plan["backend"] != "native"
        || plan["schemaVersion"] != 2
        || plan["executionSurface"] != "web_only"
        || plan["surface"] == "source"
        || plan["targetUrl"] != row.target
        || plan["attemptNumber"] != row.attempt
        || plan["budgets"]["hardTotalTokens"].as_i64() != Some(row.tokens)
        || plan["budgets"]["hardModelRequests"].as_i64() != Some(row.requests)
    {
        return Err("migration_plan_scope_invalid".into());
    }
    let mut frozen = plan;
    let fields = frozen.as_object_mut().ok_or("migration_plan_invalid")?;
    fields.remove("attemptNumber");
    fields.remove("targetUrl");
    if store::stable_hash(&frozen.to_string()) != row.hash {
        return Err("migration_plan_hash_invalid".into());
    }
    Ok(())
}

/// Only the private approval and the store's newly inserted Root can issue.
/// The caller publishes its immutable birth receipt before initializing owners.
pub(crate) fn freeze_new(
    fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    authorization: &FinancialContinuationAuthorization,
) -> Result<NewRootBudgetDeclaration, String> {
    let db = fresh.transaction();
    authorization.verify_new_root_on(fresh)?;
    let facts = authorization.verified_financial_source_on(db)?.as_json();
    if facts != authorization.source_facts() || fresh.id() != authorization.new_root_id() {
        return Err("migration_original_source_changed".into());
    }
    let root = fresh.id();
    let row = native(db, root)?;
    let history: bool = db
        .query_row(
            "SELECT
        EXISTS(SELECT 1 FROM agent_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if history {
        return Err("migration_requires_new_root".into());
    }
    let mut limits = remaining(&facts)?;
    limits[9] = Some(wall_at_birth(db, &facts, &row.birth)?);
    let declaration = NewRootBudgetDeclaration {
        schema_version: 3,
        declaration_id: uuid::Uuid::new_v4().to_string(),
        scan_id: row.scan,
        attempt_number: row.attempt,
        target_url: row.target,
        plan_hash: row.hash,
        execution_slot_capacity: 3,
        limits,
    };
    let def = Definition {
        declaration,
        approval_id: authorization.approval_id().into(),
        source_root_run_id: string(&facts, "rootRunId")?.into(),
        source_receipt_id: string(&facts, "receiptId")?.into(),
        source_financial_hash: store::stable_hash(&facts.to_string()),
        shared_remaining_tokens: cap(&facts["sharedRemainingTokens"])?,
        shared_remaining_requests: cap(&facts["sharedRemainingRequests"])?,
        original_deadline: string(&facts, "originalDeadline")?.into(),
        new_root_created_at: row.birth,
        new_plan_physical_hash: store::stable_hash(&row.text),
    };
    validate(db, root, &def, &facts)?;
    let raw = serde_json::to_string(&def).map_err(|e| e.to_string())?;
    let before = db.total_changes();
    let changed = db
        .execute(
            "INSERT INTO agent_financial_root_budget_definitions
        (root_run_id,declaration_id,definition_json,native_plan_text_hash) VALUES(?1,?2,?3,?4)",
            params![
                root,
                def.declaration.declaration_id,
                raw,
                def.new_plan_physical_hash
            ],
        )
        .map_err(|e| e.to_string())?;
    let exact: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions
        WHERE root_run_id=?1 AND declaration_id=?2 AND definition_json=?3 AND native_plan_text_hash=?4)",
        params![root,def.declaration.declaration_id,raw,def.new_plan_physical_hash], |r|r.get(0)).map_err(|e|e.to_string())?;
    if changed != 1 || !exact || db.total_changes().checked_sub(before) != Some(1) {
        return Err("migration_definition_write_conflict".into());
    }
    authorization.verify_new_root_on(fresh)?;
    authorization.verified_financial_source_on(db)?;
    Ok(def.declaration)
}

fn read_definition(db: &Connection, root: &str) -> Result<Option<Definition>, String> {
    let saved: Option<(String, String, String)> = db
        .query_row(
            "SELECT declaration_id,definition_json,
        native_plan_text_hash FROM agent_financial_root_budget_definitions WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, raw, hash)) = saved else {
        let issued: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_financial_continuations WHERE new_root_run_id=?1)",
            [root], |r|r.get(0)).map_err(|e|e.to_string())?;
        if issued {
            return Err("migration_original_definition_missing".into());
        }
        return Ok(None);
    };
    if db.is_autocommit() {
        return Err("migration_budget_snapshot_required".into());
    }
    let def: Definition = serde_json::from_str(&raw).map_err(|_| "migration_definition_invalid")?;
    let canonical = serde_json::to_string(&def).map_err(|e| e.to_string())?;
    // Refuse self/cyclic provenance before traversing any prior Root contract.
    let source_attempt: i64 = db
        .query_row(
            "SELECT attempt_number FROM agent_runs WHERE id=?1",
            [&def.source_root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "migration_original_source_missing")?;
    let fresh: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_definitions WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if canonical != raw
        || fresh
        || def.declaration.declaration_id != id
        || def.new_plan_physical_hash != hash
        || def.source_root_run_id == root
        || source_attempt <= 0
        || source_attempt >= def.declaration.attempt_number
    {
        return Err("migration_definition_binding_conflict".into());
    }
    let birth = crate::commands::verify_financial_continuation_birth_on(db, root)?;
    if birth["approvalId"] != def.approval_id
        || birth["sourceRootRunId"] != def.source_root_run_id
        || birth["sourceReceiptId"] != def.source_receipt_id
        || birth["originalDeadline"] != def.original_deadline
        || birth["newPlanPhysicalHash"] != def.new_plan_physical_hash
        || birth["newPlanHash"] != def.declaration.plan_hash
    {
        return Err("migration_birth_binding_conflict".into());
    }
    validate(db, root, &def, &birth["financialSource"])?;
    Ok(Some(def))
}

pub(crate) fn read(
    db: &Connection,
    root: &str,
) -> Result<Option<NewRootBudgetDeclaration>, String> {
    Ok(read_definition(db, root)?.map(|d| d.declaration))
}
/// Independent gross ceilings never collapse a finite zero into unlimited.
type SharedFinancialCaps = (Option<i64>, Option<i64>);

pub(crate) fn shared_caps_on(
    db: &Connection,
    root: &str,
) -> Result<Option<SharedFinancialCaps>, String> {
    Ok(
        read_definition(db, root)?
            .map(|d| (d.shared_remaining_tokens, d.shared_remaining_requests)),
    )
}

/// This issuance authorizes one plain Root assessment only. The marker is a
/// denial boundary even when its provenance is damaged; it never grants work.
/// Original receive/settlement paths deliberately do not call this guard.
pub(crate) fn require_plain_root_boundary(db: &Connection, root: &str) -> Result<(), String> {
    let issued: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_financial_continuations WHERE new_root_run_id=?1)",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if issued {
        return Err("financial_continuation_plain_root_only".into());
    }
    Ok(())
}
