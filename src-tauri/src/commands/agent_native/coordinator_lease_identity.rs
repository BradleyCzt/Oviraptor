// Frozen readonly identity metadata is not a target authorization.
fn native_lease_identity_closed_payload(db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    worker: &crate::agent_runtime::multi_agent::attempts::AssignmentAttempt) -> Result<JsonValue,String> {
    let task = native_coordinator_identity_task(db,actor,child)?;
    let receipt = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(db,actor,child)?;
    native_coordinator_identity_semantic(&receipt.text)?;
    for (dimension,amount) in [("model_input_tokens",4000),("model_cached_tokens",4000),("model_output_tokens",4000),("model_requests",1)] {
        let logical = format!("reserve:{}:{dimension}",child.assignment_id);
        let key = if worker.lease_epoch==1 {logical} else {format!("worker:{}:{logical}",worker.id)};
        let count:i64 = db.query_row("SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 AND kind='reserve' AND amount=?5 AND idempotency_key=?6 AND source_id=?7",
            params![actor.root_run_id,child.assignment_id,worker.id,dimension,amount,key,format!("assignment:{}",child.assignment_id)],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count!=1 {return Err("root_lease_identity_original_grant_conflict".into());}
    }
    Ok(json!({"summary":receipt.text,"identityMode":task["identityMode"]}))
}
