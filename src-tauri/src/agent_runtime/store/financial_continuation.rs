//! The caller's IMMEDIATE transaction owns this sealed, genuinely new insert.
use super::*;

pub(crate) fn insert_financial_continuation_root_in<T>(
    tx: &Transaction<'_>,
    approval: &crate::scanning::financial_continuation::FinancialContinuationAuthorization,
    publish: impl FnOnce(&NewlyInsertedNativeRoot<'_, '_>) -> Result<T, String>,
) -> Result<T, String> {
    approval.verified_financial_source_on(tx)?;
    let plan = approval.new_plan();
    let source = approval.source_facts();
    let scan = source["scanId"]
        .as_str()
        .ok_or("financial_continuation_source_invalid")?;
    let attempt = plan["attemptNumber"]
        .as_i64()
        .ok_or("financial_continuation_plan_invalid")?;
    let target = plan["targetUrl"]
        .as_str()
        .ok_or("financial_continuation_plan_invalid")?;
    let ceilings = [
        "softUncachedTokens",
        "hardTotalTokens",
        "softModelRequests",
        "hardModelRequests",
    ]
    .map(|key| {
        plan["budgets"][key]
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or("financial_continuation_plan_invalid")
    })
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let count = tx.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,
        plan_hash,plan_json,orchestration_policy,root_run_id,soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget)
        VALUES(?1,?2,?3,?4,'native','coordinator',?5,?6,'multi',?1,?7,?8,?9,?10)",
        params![approval.new_root_id(),scan,attempt,target,approval.new_plan_hash(),plan.to_string(),
            ceilings[0],ceilings[1],ceilings[2],ceilings[3]]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("financial_continuation_new_root_unconfirmed".into());
    }
    let fresh = NewlyInsertedNativeRoot {
        transaction: tx,
        id: approval.new_root_id(),
    };
    approval.verify_new_root_on(&fresh)?;
    publish(&fresh)
}
