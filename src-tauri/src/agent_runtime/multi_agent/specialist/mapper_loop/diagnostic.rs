//! Pure original phases; this does not grant execution or repair missing rows.
use super::*;
pub(crate) fn cost_phase_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    id: &str,
) -> Result<Option<String>, String> {
    verify_dispatch_original_on(db, actor, child, id)?;
    let received = read(db, id, "received")?;
    let uncertain = read(db, id, "uncertain")?;
    let unsent = read(db, id, "unsent")?;
    if usize::from(received.is_some())
        + usize::from(uncertain.is_some())
        + usize::from(unsent.is_some())
        > 1
    {
        return Err("mapper_loop_terminal_phase_conflict".into());
    }
    if received.is_some() {
        let original = read_received_on(db, actor, child, id)?;
        let source = source(id);
        let receipt_source = format!(
            "{source}:receipt:{}",
            store::stable_hash(&original.fact.to_string())
        );
        let bad: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND (source_id=?2 OR substr(source_id,1,length(?2)+9)=?2||':receipt:') AND (assignment_id<>?3 OR lease_attempt_id<>?4 OR source_id<>?5 OR dimension NOT IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') OR kind NOT IN ('consume','forfeit') OR idempotency_key<>'received:'||?2||':'||dimension))",params![actor.root_run_id,source,child.assignment_id,original.worker,receipt_source],|r|r.get(0)).map_err(|e|e.to_string())?;
        if bad {
            return Err("mapper_loop_original_received_fee_shape_changed".into());
        }
        return Ok(Some("received".into()));
    }
    let invoice = read(db, id, "invoice")?;
    if invoice.is_some() && (uncertain.is_some() || unsent.is_some()) {
        return Err("mapper_loop_terminal_invoice_conflict".into());
    }
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let dispatch = read(db, id, "dispatch")?.ok_or("mapper_loop_original_dispatch_missing")?;
    let source = source(id);
    let estimate = dispatch["estimate"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("mapper_loop_original_estimate_invalid")?;
    let mut query=db.prepare("SELECT dimension,kind,amount,idempotency_key,assignment_id,lease_attempt_id FROM agent_budget_entries WHERE root_run_id=?1 AND (source_id=?2 OR substr(source_id,1,length(?2)+9)=?2||':receipt:') ORDER BY dimension,idempotency_key").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![actor.root_run_id, source], |r| {
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
    if let Some(fact) = uncertain {
        if fact != json!({"code":"model_outcome_unknown"}) || rows.len() != 4 {
            return Err("mapper_loop_original_uncertain_changed".into());
        }
        for dimension in budget::DIMENSIONS.iter().take(4) {
            let matching = rows
                .iter()
                .filter(|r| r.0 == *dimension)
                .collect::<Vec<_>>();
            let [row] = matching.as_slice() else {
                return Err("mapper_loop_original_unknown_fee_missing".into());
            };
            let amount = if *dimension == "model_requests" {
                1
            } else {
                estimate
            };
            if row.1 != "forfeit"
                || row.2 != amount
                || row.3 != format!("unknown:{source}:{dimension}")
                || row.4 != child.assignment_id
                || row.5 != grant.worker_attempt_id()
            {
                return Err("mapper_loop_original_unknown_fee_changed".into());
            }
        }
        Ok(Some("uncertain".into()))
    } else {
        if !rows.is_empty() {
            return Err("mapper_loop_original_unreceived_fee_conflict".into());
        }
        if let Some(fact) = unsent {
            let object = fact
                .as_object()
                .ok_or("mapper_loop_original_unsent_changed")?;
            if object.len() != 1 || fact["code"].as_str().is_none_or(|s| s.is_empty()) {
                return Err("mapper_loop_original_unsent_changed".into());
            }
            Ok(Some("unsent".into()))
        } else if invoice.is_some() {
            Ok(Some("invoice".into()))
        } else {
            Ok(None)
        }
    }
}
