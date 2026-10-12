//! Original local-exit inventory only; never terminal completion or a refund.
use super::*;
pub(crate) struct VerifiedMapperIdle {
    actor: CoordinatorLease,
    child: ScheduledChild,
    fact: Value,
    has_transport: bool,
}
impl VerifiedMapperIdle {
    pub(crate) fn has_transport(&self) -> bool {
        self.has_transport
    }
    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if actor != &self.actor
            || child != &self.child
            || verify_for_idle_on(db, actor, child)?.fact != self.fact
        {
            return Err("mapper_loop_original_idle_changed".into());
        }
        Ok(())
    }
}
pub(crate) fn verify_for_idle_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<VerifiedMapperIdle, String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    require_schema(db)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let owner = budget::receipts::original_owner(db, &child.run_id, actor, &child.assignment_id)?;
    owner.verify(db)?;
    if child.role != AgentRole::SpaApiMapper || owner.attempt_id() != grant.worker_attempt_id() {
        return Err("mapper_loop_original_idle_scope_changed".into());
    }
    let mut query=db.prepare("SELECT call_id,phase,round_number,fact_json FROM main.agent_mapper_loop_journal WHERE child_run_id=?1 ORDER BY round_number,phase").map_err(|e|e.to_string())?;
    let rows = query
        .query_map([&child.run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let dispatches = rows
        .iter()
        .filter(|r| r.1 == "dispatch")
        .collect::<Vec<_>>();
    if dispatches.len() > 2 || (!rows.is_empty() && dispatches.is_empty()) {
        return Err("mapper_loop_original_idle_rounds_invalid".into());
    }
    let mut facts = Vec::new();
    for (index, row) in dispatches.iter().enumerate() {
        if row.2
            != i64::try_from(index + 1).map_err(|_| "mapper_loop_original_idle_rounds_invalid")?
        {
            return Err("mapper_loop_original_idle_rounds_invalid".into());
        }
        verify_dispatch_original_on(db, actor, child, &row.0)?;
        let phase = cost_phase_original_on(db, actor, child, &row.0)?;
        for physical in rows.iter().filter(|r| r.0 == row.0) {
            let canonical = read(db, &physical.0, &physical.1)?
                .ok_or("mapper_loop_original_idle_row_missing")?;
            let encoded = canonical.to_string();
            if encoded != physical.3 {
                return Err("mapper_loop_original_idle_row_changed".into());
            }
            if physical.1 == "lookup" {
                let first = read_received_on(db, actor, child, &row.0)?;
                lookup::verify_lookup_original_on(db, actor, child, &first)?;
            }
            facts.push(
                json!({"id":physical.0,"phase":physical.1,"round":physical.2,"fact":canonical}),
            );
        }
        if phase.as_deref() == Some("unsent")
            && rows.iter().any(|r| r.0 == row.0 && r.1 == "lookup")
        {
            return Err("mapper_loop_original_idle_phase_conflict".into());
        }
    }
    if rows.iter().any(|r| !dispatches.iter().any(|d| d.0 == r.0)) {
        return Err("mapper_loop_original_idle_orphan_phase".into());
    }
    let mut fees=db.prepare("SELECT dimension,kind,amount,idempotency_key,source_id,lease_attempt_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') AND kind IN ('consume','forfeit') ORDER BY idempotency_key").map_err(|e|e.to_string())?;
    let fee_rows = fees
        .query_map(params![actor.root_run_id, child.assignment_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if fee_rows.iter().any(|r| {
        r.5 != grant.worker_attempt_id()
            || !dispatches.iter().any(|d| {
                r.4 == source(&d.0) || r.4.starts_with(&format!("{}:receipt:", source(&d.0)))
            })
    }) {
        return Err("mapper_loop_original_idle_foreign_fee".into());
    }
    let settled: String = db
        .query_row(
            "SELECT budget_settled_at FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut counters = Vec::new();
    for dimension in &budget::DIMENSIONS[..4] {
        let balance = budget::balance(
            db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )?;
        if settled.is_empty() {
            let initial = if *dimension == "model_requests" {
                2
            } else {
                grant.tokens()
            };
            let remaining = initial
                .checked_sub(balance.consumed)
                .and_then(|n| n.checked_sub(balance.indeterminate))
                .filter(|n| *n >= 0)
                .ok_or("mapper_loop_original_idle_balance_changed")?;
            if balance.reserved != remaining {
                return Err("mapper_loop_original_idle_balance_changed".into());
            }
        }
        counters.push(json!({"dimension":dimension,"consumed":balance.consumed,"indeterminate":balance.indeterminate,"reserved":balance.reserved}));
    }
    for dimension in &budget::DIMENSIONS[4..8] {
        let balance = budget::balance(
            db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )?;
        if balance.consumed != 0 || balance.indeterminate != 0 || balance.reserved != 0 {
            return Err("mapper_loop_original_idle_business_fee".into());
        }
    }
    let clock = budget::clock::original_worker_samples::original_worker_consumed_on(
        db,
        actor,
        child,
        grant.worker_attempt_id(),
    )?;
    let fact = json!({"inputHash":grant.input().hash(),"worker":grant.worker_attempt_id(),"journal":facts,"fees":fee_rows,"counters":counters,"originalChildClock":clock});
    Ok(VerifiedMapperIdle {
        actor: actor.clone(),
        child: child.clone(),
        has_transport: !dispatches.is_empty(),
        fact,
    })
}
