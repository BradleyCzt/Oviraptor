//! Frozen first-worker Mapper invoices pin allocation across live-balance changes.
use super::mapper_loop_contract::FrozenMapperLoopInput;
use crate::agent_runtime::multi_agent::{attempts, lease::CoordinatorLease};
use rusqlite::{params, Connection};

pub(crate) struct VerifiedMapperLoopGrant {
    input: FrozenMapperLoopInput,
    worker_id: String,
}
impl VerifiedMapperLoopGrant {
    pub(crate) fn input(&self) -> &FrozenMapperLoopInput {
        &self.input
    }
    pub(crate) fn worker_attempt_id(&self) -> &str {
        &self.worker_id
    }
    pub(crate) fn tokens(&self) -> i64 {
        self.input.reserved_tokens()
    }
    pub(crate) fn requests(&self) -> i64 {
        2
    }
}

/// Pure original grant proof, not current worker execution authority. Settlement
/// must be verified by the dedicated loop receipt before any result delivery.
pub(crate) fn original_mapper_loop_grant_on(
    db: &Connection,
    actor: &CoordinatorLease,
    assignment: &str,
) -> Result<VerifiedMapperLoopGrant, String> {
    require_original_invoice_guards(db)?;
    let worker = attempts::current(db, actor, assignment)?;
    if worker.lease_epoch != 1 {
        return Err("mapper_loop_original_worker_conflict".into());
    }
    let (raw, held_tokens, held_requests, settled, state): (String,i64,i64,String,String) = db.query_row(
        "SELECT a.task_slice_json,a.reserved_tokens,a.reserved_requests,a.budget_settled_at,a.state
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.role='spa_api_mapper' AND a.lane='read_only_analysis'
           AND a.target_key=?3 AND a.trigger_code='frontend_evidence_ready' AND a.evidence_revision=1
           AND r.root_run_id=?2 AND r.parent_run_id=?2 AND r.assignment_id=a.id AND r.role=a.role AND r.lane=a.lane
           AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?3 AND r.backend='native'
           AND r.orchestration_policy='multi' AND a.lease_epoch=?6 AND a.fencing_token=?7",
        params![assignment,actor.root_run_id,actor.target_key,actor.scan_id,actor.attempt_number,actor.lease_epoch,actor.fencing_token],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"mapper_loop_original_grant_conflict")?;
    let task: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "mapper_loop_original_grant_conflict")?;
    let canonical = task.to_string();
    if canonical != raw {
        return Err("mapper_loop_original_grant_conflict".into());
    }
    let input = FrozenMapperLoopInput::from_frozen_wire_on(db, actor, &task["mapperLoop"])?;
    let tokens = input.reserved_tokens();
    let policy = &task["rootDecision"]["rustPolicy"];
    if task["mapperLoopHash"] != input.hash()
        || policy["schemaVersion"] != 3
        || policy["reservedModelRequests"] != 2
        || policy["reservedModelTokens"] != tokens
        || policy["mapperLoopRule"] != super::mapper_loop_contract::RULE
        || policy["targetRequestsGranted"] != 0
        || policy["reviewerFloor"] != serde_json::json!({"modelTokens":15000,"modelRequests":1})
    {
        return Err("mapper_loop_original_grant_conflict".into());
    }
    let event = task["rootDecision"]["eventSequence"]
        .as_i64()
        .ok_or("mapper_loop_original_paid_root_missing")?;
    let projection =
        super::super::root::model::tick::Tick::project_publication(db, &actor.root_run_id, event)?;
    if task["rootDecision"]["step"] != "dispatch:spa_api_mapper"
        || projection.record.get("localTools").is_some()
        || task["rootDecision"]["summaryHash"]
            != crate::agent_runtime::store::stable_hash(&projection.record["summary"].to_string())
        || !projection.record["summary"]["suggestions"]
            .as_array()
            .is_some_and(|xs| xs.iter().any(|x| x == "dispatch:spa_api_mapper"))
    {
        return Err("mapper_loop_original_paid_root_missing".into());
    }
    let request_raw: String = db.query_row("SELECT q.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p
        ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication'
        WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(p.fact_json,'$.eventSequence')=?2",
        params![actor.root_run_id,event],|r|r.get(0)).map_err(|_|"mapper_loop_original_paid_root_missing")?;
    let request: serde_json::Value =
        serde_json::from_str(&request_raw).map_err(|_| "mapper_loop_original_paid_root_missing")?;
    let mut basis = request["request"]["basis"].clone();
    let object = basis
        .as_object_mut()
        .ok_or("mapper_loop_original_paid_root_missing")?;
    for key in [
        "localParents",
        "localHistory",
        "localRound",
        "localCallsUsed",
        "budgetSnapshot",
    ] {
        object.remove(key);
    }
    if basis["phase"] != "bootstrap"
        || basis["permittedSuggestion"] != "dispatch:spa_api_mapper"
        || task["rootDecision"]["basisHash"]
            != crate::agent_runtime::store::stable_hash(&basis.to_string())
    {
        return Err("mapper_loop_original_paid_root_missing".into());
    }
    let held = held_tokens == tokens && held_requests == 2 && settled.is_empty();
    let closed = held_tokens == 0 && held_requests == 0 && !settled.is_empty();
    if !(matches!(state.as_str(), "running" | "paused") && held
        || matches!(state.as_str(), "completed" | "paused") && closed)
    {
        return Err("mapper_loop_original_grant_conflict".into());
    }
    let mut query = db.prepare("SELECT dimension,amount,idempotency_key,source_id FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve'
          AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension")
        .map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![actor.root_run_id, assignment, worker.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if rows.len() != 4
        || rows.iter().any(|(dimension, amount, key, source)| {
            *amount
                != if dimension == "model_requests" {
                    2
                } else {
                    tokens
                }
                || key != &format!("reserve:{assignment}:{dimension}")
                || source != &format!("assignment:{assignment}")
        })
    {
        return Err("mapper_loop_original_grant_conflict".into());
    }
    Ok(VerifiedMapperLoopGrant {
        input,
        worker_id: worker.id,
    })
}

pub(crate) fn original_mapper_grant(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<i64, String> {
    require_original_invoice_guards(db)?;
    let worker = attempts::current(db, lease, assignment)?;
    if worker.lease_epoch != 1 {
        return Err("mapper_allocation_original_worker_conflict".into());
    }
    let (held_tokens, held_requests, state, settled, failure): (i64, i64, String, String, String) = db.query_row("SELECT reserved_tokens,reserved_requests,state,budget_settled_at,failure_class FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='spa_api_mapper'", params![assignment,lease.root_run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_| "mapper_allocation_original_grant_conflict")?;
    // A worker paused after its one paid call was received but before local
    // delivery still holds the original grant unsettled; the saved-receipt
    // recovery (readonly_assessment) settles it without another model call.
    // Uncertain/executing calls never qualify: unknown debt cannot be adopted.
    let saved_receipt_only: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND root_run_id=?2 AND state='received')
             AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND state<>'received')",
            params![assignment, lease.root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "mapper_allocation_original_grant_conflict")?;
    let mut q = db.prepare("SELECT dimension,amount,idempotency_key,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![lease.root_run_id, assignment, worker.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let tokens = rows
        .iter()
        .find(|row| row.0 == "model_input_tokens")
        .map(|row| row.1)
        .ok_or("mapper_allocation_original_grant_conflict")?;
    let held_original = held_tokens == tokens && held_requests == 1 && settled.is_empty();
    let intact_projection = state == "running" && held_original
        || state == "paused"
            && held_original
            && saved_receipt_only
            && matches!(
                failure.as_str(),
                "child_usage_reconciliation_required" | "worker_lease_expired"
            )
        || matches!(state.as_str(), "completed" | "paused")
            && held_tokens == 0
            && held_requests == 0
            && !settled.is_empty();
    if !intact_projection
        || !(1..=8_000).contains(&tokens)
        || rows.len() != 4
        || rows.iter().any(|(dimension, amount, key, source)| {
            *amount
                != if dimension == "model_requests" {
                    1
                } else {
                    tokens
                }
                || key != &format!("reserve:{assignment}:{dimension}")
                || source != &format!("assignment:{assignment}")
        })
    {
        return Err("mapper_allocation_original_grant_conflict".into());
    }
    Ok(tokens)
}

pub(super) fn require_original_invoice_guards(db: &Connection) -> Result<(), String> {
    // Do not accept mutable/injected invoices as original allocation proof.
    for (name, suffix) in [
        ("budget_entry_no_update", "BEFORE UPDATE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
        ("budget_entry_no_delete", "BEFORE DELETE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
        ("budget_entry_no_replace", "BEFORE INSERT ON agent_budget_entries WHEN EXISTS(SELECT 1 FROM agent_budget_entries WHERE entry_id=NEW.entry_id OR (root_run_id=NEW.root_run_id AND idempotency_key=NEW.idempotency_key)) BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
    ] {
        let actual: String = db.query_row("SELECT sql FROM main.sqlite_master WHERE type='trigger' AND name=?1", [name], |r| r.get(0)).map_err(|_| "mapper_allocation_invoice_guard_conflict")?;
        let normalize = |s: &str| s.trim().trim_end_matches(';').replace(" IF NOT EXISTS", "").split_whitespace().collect::<Vec<_>>().join(" ");
        if normalize(&actual) != normalize(&format!("CREATE TRIGGER {name} {suffix}")) {
            return Err("mapper_allocation_invoice_guard_conflict".into());
        }
    }
    Ok(())
}
