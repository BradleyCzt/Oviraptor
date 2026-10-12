//! Attempt/root-scoped numbers, without source IDs, identities or lease tokens.
//! Coverage names describe the implemented writer boundary, not completeness.
use super::{balance, DIMENSIONS};
use rusqlite::{params, Connection};

pub(crate) fn unclosed_web_calls(db: &Connection, root: &str) -> Result<i64, String> {
    db.query_row(
        &format!("SELECT COUNT(*) FROM agent_web_model_journal d WHERE d.root_run_id=?1 AND d.phase='dispatch'
         AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal t WHERE t.call_id=d.call_id
           AND t.phase IN ('received','unsent') AND {})", super::WEB_RECEIPT_BINDING),
        [root], |r|r.get(0),
    ).map_err(|_| "budget_diagnostic_web_calls_unavailable".into())
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionDiagnostic {
    dimension: &'static str,
    hard_limit: Option<i64>,
    reserved: i64,
    consumed: i64,
    indeterminate: i64,
    coverage: &'static str,
}

pub(crate) fn coverage_for(dimension: &str, explicit_root_definition: bool) -> &'static str {
    match dimension {
        "target_requests" => "multi_agent_broker",
        "concurrency_batches" if explicit_root_definition => "race_batch_dispatch_unimplemented",
        "concurrency_batches" => "multi_agent_lane",
        "wall_time_ms" => "admission_samples",
        d if super::DENIED_BY_CONTRACT_DIMENSIONS.contains(&d) => "denied_by_contract",
        _ => "assignment_settlement",
    }
}

pub(crate) fn read(
    db: &Connection,
    root: &str,
) -> Result<Option<Vec<DimensionDiagnostic>>, String> {
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count == 0 {
        let entries: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        return if entries {
            Err("budget_diagnostic_limits_missing".into())
        } else {
            Ok(None)
        };
    }
    if count != DIMENSIONS.len() as i64 {
        return Err("budget_diagnostic_contract_incomplete".into());
    }
    let explicit = super::root_definition::read(db, root)?.is_some();
    let mut rows = Vec::with_capacity(DIMENSIONS.len());
    for dimension in DIMENSIONS {
        let hard_limit = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root, dimension],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(|_| "budget_diagnostic_contract_incomplete")?;
        let b = balance(db, root, None, dimension)?;
        let coverage = coverage_for(dimension, explicit);
        rows.push(DimensionDiagnostic {
            dimension,
            hard_limit,
            reserved: b.reserved,
            consumed: b.consumed,
            indeterminate: b.indeterminate,
            coverage,
        });
    }
    Ok(Some(rows))
}
