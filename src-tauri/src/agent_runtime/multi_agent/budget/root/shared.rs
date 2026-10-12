//! One gross ceiling for Root calls and child grants. Class reservations cover
//! the same child allowance; input/output cannot be charged twice on admission.
use super::super::{balance, entries, Balance};
use rusqlite::{params, Connection, OptionalExtension};

fn add(a: i64, b: i64) -> Result<i64, String> {
    a.checked_add(b)
        .ok_or_else(|| "budget_amount_overflow".into())
}

fn held(b: &Balance) -> Result<i64, String> {
    add(b.reserved, b.indeterminate)
}

fn child_gross(db: &Connection, root: &str) -> Result<(i64, i64), String> {
    let orphan: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries e WHERE e.root_run_id=?1 AND e.assignment_id<>''
        AND NOT EXISTS(SELECT 1 FROM agent_assignment_attempts x WHERE x.id=e.lease_attempt_id AND x.root_run_id=e.root_run_id AND x.assignment_id=e.assignment_id))",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if orphan {
        return Err("budget_attempt_scope_conflict".into());
    }
    let mut statement = db.prepare("SELECT DISTINCT assignment_id,lease_attempt_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>''").map_err(|e|e.to_string())?;
    let workers = statement
        .query_map([root], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let (mut tokens, mut requests) = (0, 0);
    for worker in workers {
        let (assignment, attempt) = worker.map_err(|e| e.to_string())?;
        let input =
            entries::balance_for_attempt(db, root, &assignment, &attempt, "model_input_tokens")?;
        let output =
            entries::balance_for_attempt(db, root, &assignment, &attempt, "model_output_tokens")?;
        let model =
            entries::balance_for_attempt(db, root, &assignment, &attempt, "model_requests")?;
        let paid = add(input.consumed, output.consumed)?;
        // A child owns one whole grant across all rounds. A paid prefix uses
        // that grant, whereas terminal settlement releases its unused classes.
        let gross = paid
            .max(add(input.consumed, held(&input)?)?)
            .max(add(output.consumed, held(&output)?)?);
        tokens = add(tokens, gross)?;
        requests = add(requests, add(model.consumed, held(&model)?)?)?;
    }
    Ok((tokens, requests))
}

pub(super) fn available(db: &Connection, root: &str) -> Result<(Option<i64>, Option<i64>), String> {
    let orphan: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries e WHERE e.root_run_id=?1 AND e.assignment_id=''
        AND NOT EXISTS(SELECT 1 FROM agent_root_budget_attempts a WHERE a.id=e.lease_attempt_id AND a.root_run_id=e.root_run_id))",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if orphan {
        return Err("budget_root_attempt_scope_conflict".into());
    }
    let (token_limit, request_limit, policy): (i64, i64, String) = db.query_row("SELECT hard_token_budget,hard_request_budget,orchestration_policy FROM agent_runs
        WHERE id=?1 AND backend='native' AND role='coordinator' AND assignment_id='' AND parent_run_id IS NULL", [root], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if token_limit < 0 || request_limit < 0 || !matches!(policy.as_str(), "single" | "multi") {
        return Err("budget_limit_invalid".into());
    }
    let migrated = super::super::continuation::migration::read(db, root)?;
    let (token_cap, request_cap) = if let Some(declaration) = &migrated {
        let caps = super::super::continuation::migration::shared_caps_on(db, root)?
            .ok_or("migration_shared_contract_missing")?;
        for (dimension, expected) in super::super::DIMENSIONS.iter().zip(declaration.limits) {
            let saved: Option<i64> = db.query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root,dimension],|r|r.get(0)).map_err(|e|e.to_string())?;
            if saved != expected {
                return Err("budget_frozen_limit_changed".into());
            }
        }
        caps
    } else {
        for (dimension, expected) in [
            ("model_input_tokens", token_limit),
            ("model_output_tokens", token_limit),
            ("model_requests", request_limit),
        ] {
            let saved: Option<i64> = db.query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root,dimension], |r|r.get(0)).map_err(|e|e.to_string())?;
            if saved != (expected > 0).then_some(expected) {
                return Err("budget_frozen_limit_changed".into());
            }
        }
        (
            (token_limit > 0).then_some(token_limit),
            (request_limit > 0).then_some(request_limit),
        )
    };
    let coarse: Option<[i64; 6]> = db.query_row("SELECT total_tokens,total_requests,spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1", [root], |r|Ok([r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?])).optional().map_err(|e|e.to_string())?;
    let fine = child_gross(db, root)?;
    let child = if let Some(coarse) = coarse {
        if policy != "multi"
            || coarse[0] != token_limit
            || coarse[1] != request_limit
            || coarse.iter().any(|v| *v < 0)
        {
            return Err("budget_projection_contract_conflict".into());
        }
        let total = (add(coarse[2], coarse[4])?, add(coarse[3], coarse[5])?);
        // Unlimited invoices can top up their class receipts before the legacy
        // projection is published. A finite original grant cannot grow.
        if token_cap.is_some() && total.0 != fine.0 || request_cap.is_some() && total.1 != fine.1 {
            return Err("budget_projection_balance_conflict".into());
        }
        (total.0.max(fine.0), total.1.max(fine.1))
    } else {
        if fine != (0, 0) {
            return Err("budget_projection_contract_conflict".into());
        }
        (0, 0)
    };
    let input = balance(db, root, Some(""), "model_input_tokens")?;
    let output = balance(db, root, Some(""), "model_output_tokens")?;
    let model = balance(db, root, Some(""), "model_requests")?;
    let root_tokens = add(
        add(input.consumed, output.consumed)?,
        held(&input)?.max(held(&output)?),
    )?;
    let root_requests = add(model.consumed, held(&model)?)?;
    let occupied = (add(child.0, root_tokens)?, add(child.1, root_requests)?);
    let remaining = |limit: Option<i64>, used: i64| -> Result<Option<i64>, String> {
        let Some(limit) = limit else {
            return Ok(None);
        };
        limit
            .checked_sub(used)
            .filter(|v| *v >= 0)
            .map(Some)
            .ok_or_else(|| "budget_hard_limit_exceeded".into())
    };
    Ok((
        remaining(token_cap, occupied.0)?,
        remaining(request_cap, occupied.1)?,
    ))
}

pub(crate) fn remaining_child_capacity(
    db: &Connection,
    root: &str,
) -> Result<(Option<i64>, Option<i64>), String> {
    available(db, root)
}

pub(crate) fn require_child_capacity(
    db: &Connection,
    root: &str,
    tokens: i64,
    requests: i64,
) -> Result<(), String> {
    if tokens < 0 || requests < 0 {
        return Err("budget_limit_invalid".into());
    }
    let (available_tokens, available_requests) = available(db, root)?;
    if available_tokens.is_some_and(|v| v < tokens)
        || available_requests.is_some_and(|v| v < requests)
    {
        return Err("child_budget_reservation_exceeded_or_stale".into());
    }
    Ok(())
}
