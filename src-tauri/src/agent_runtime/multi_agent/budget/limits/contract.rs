//! One frozen dimension mapping for initialization and read-only owner proofs.
use super::super::DIMENSIONS;
use rusqlite::{params, Connection};
use serde_json::Value;

pub(super) fn frozen(db: &Connection, root: &str) -> Result<[Option<i64>; 10], String> {
    let (tokens, requests, text): (i64, i64, String) = db
        .query_row(
            "SELECT hard_token_budget,hard_request_budget,plan_json FROM agent_runs
         WHERE id=?1 AND backend='native'",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    if tokens < 0 || requests < 0 {
        return Err("budget_limit_invalid".into());
    }
    let plan: Value = serde_json::from_str(&text).map_err(|_| "budget_frozen_plan_invalid")?;
    let target = if plan["surface"] == "source" {
        0
    } else {
        plan["budgets"]["hardModelRequests"]
            .as_i64()
            .filter(|v| *v >= 0)
            .ok_or("budget_frozen_request_limit_invalid")?
            .max(1)
            .saturating_mul(4)
            .min(400)
    };
    if let Some(declaration) = super::super::root_definition::read(db, root)? {
        let finite = |value| (value > 0).then_some(value);
        if declaration.schema_version != 3
            && (declaration.limits[..3] != [finite(tokens); 3]
                || declaration.limits[3] != finite(requests))
        {
            return Err("budget_declaration_native_ceiling_conflict".into());
        }
        return Ok(declaration.limits);
    }
    let finite = |value| (value > 0).then_some(value);
    Ok([
        finite(tokens),
        finite(tokens),
        finite(tokens),
        finite(requests),
        Some(target),
        Some(0),
        Some(0),
        Some(0),
        Some(3),
        Some(super::frozen_wall_time(&plan)?),
    ])
}

pub(super) fn verify(db: &Connection, root: &str) -> Result<(), String> {
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != 10 {
        return Err("budget_dimension_contract_incomplete".into());
    }
    for (dimension, expected) in DIMENSIONS.iter().zip(frozen(db, root)?) {
        let actual: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root, dimension],
                |r| r.get(0),
            )
            .map_err(|_| "budget_dimension_contract_incomplete")?;
        if actual != expected {
            return Err("budget_frozen_limit_changed".into());
        }
    }
    Ok(())
}
