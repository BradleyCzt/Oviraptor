// Historical signed Mapper-loop delivery is not a fresh executor permission.
impl NativeCoordinatorFrame {
    fn lease_closed_mapper_loop_original_on(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::multi_agent::specialist::mapper_loop::{
            expired_saved, expired_seal,
        };
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::lease_closed_mapper_loop_original_on(&tx, actor, child);
        }
        if child.role != crate::agent_runtime::contract::AgentRole::SpaApiMapper {
            return Err("root_mapper_expired_role_invalid".into());
        }
        let seal = expired_seal::read_original_on(db, actor, child)?
            .ok_or("root_mapper_expired_original_receipt_missing")?;
        let closed = expired_saved::read_closed_original_on(db, actor, child)?;
        if seal.fact()["financial"] != *closed.fact()
            || closed.fact()["workerAttemptId"] != seal.worker_attempt_id()
        {
            return Err("root_mapper_expired_original_financial_changed".into());
        }
        let expiry = closed.fact()["expiryReceiptId"]
            .as_str()
            .ok_or("root_mapper_expired_original_expiry_missing")?;
        // Neither legacy single-call rows nor a normal-final seal may substitute
        // for the original expired delivery. Their exact absence is frozen too.
        let conflict: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_specialist_calls WHERE child_run_id=?1) OR EXISTS(SELECT 1 FROM main.agent_mapper_loop_final_receipts WHERE child_run_id=?1)",
            [&child.run_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if conflict {
            return Err("root_mapper_expired_original_source_conflict".into());
        }
        let mut rows = Vec::new();
        for (sql, keys) in [
            ("SELECT rowid,* FROM main.agent_assignment_attempts WHERE id=?1", vec![seal.worker_attempt_id()]),
            ("SELECT rowid,* FROM main.agent_assignments WHERE id=?1", vec![child.assignment_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_runs WHERE id=?1", vec![child.run_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_mapper_loop_journal WHERE child_run_id=?1 ORDER BY round_number,phase", vec![child.run_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid", vec![seal.worker_attempt_id()]),
            ("SELECT * FROM main.agent_mapper_loop_expired_receipts WHERE receipt_id=?1", vec![seal.receipt_id()]),
            ("SELECT rowid,* FROM main.agent_worker_expiry_receipts WHERE receipt_id=?1", vec![expiry]),
            ("SELECT rowid,* FROM main.agent_worker_expiry_issuance_seals WHERE receipt_id=?1 AND root_run_id=?2", vec![expiry, actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1", vec![actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_root_mode_definitions WHERE root_run_id=?1", vec![actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid", vec![child.assignment_id.as_str()]),
            ("SELECT rowid,* FROM main.agent_messages WHERE id=?1", vec![seal.message_id()]),
            ("SELECT rowid,* FROM main.agent_collaboration_events WHERE entity_id IN (?1,?2,?3) ORDER BY sequence", vec![child.assignment_id.as_str(),child.run_id.as_str(),seal.message_id()]),
        ] { rows.push(NativeCoordinatorFrozenRows::capture(db, sql, &keys)?); }
        for sql in [
            "SELECT rowid,* FROM main.agent_specialist_calls WHERE child_run_id=?1 ORDER BY rowid",
            "SELECT * FROM main.agent_mapper_loop_final_receipts WHERE child_run_id=?1",
            "SELECT rowid,* FROM main.agent_model_cost_facts WHERE child_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_snapshots WHERE run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_events WHERE run_id=?1 ORDER BY sequence",
        ] {
            rows.push(NativeCoordinatorFrozenRows::capture_optional(
                db,
                sql,
                &[&child.run_id],
            )?);
        }
        Ok(Self {
            kind: "lease-expired",
            semantic: json!({"rootRunId":actor.root_run_id,"role":"spa_api_mapper",
                "source":"original_mapper_loop_expired_signed_delivery_v1",
                "workerAttemptId":seal.worker_attempt_id(),"receiptId":expiry,
                "expiredMapperReceiptId":seal.receipt_id(),"assignmentId":child.assignment_id,
                "childRunId":child.run_id,"messageId":seal.message_id(),
                "originalInvoice":closed.output().usage().as_json(),
                "savedOutput":{"summary":closed.output().text()},
                "originalExpiredOutput":seal.fact(),"dispatchGranted":false,"targetRequestsGranted":0}),
            rows,
            review_refs: None,
            review_meta: None,
        })
    }
}

fn native_coordinator_mapper_expired_sdk_exit(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<crate::agent_runtime::execution_owner::NativeInvocationOwner, String> {
    if child.role != crate::agent_runtime::contract::AgentRole::SpaApiMapper {
        return Err("root_mapper_expired_role_invalid".into());
    }
    crate::agent_runtime::execution_owner::probe_native_invocation(
        &context.db_path,
        &actor.scan_id,
        actor.attempt_number,
        "mapper-loop-sdk",
        &child.run_id,
    )
    .map_err(|e| format!("root_mapper_expired_sdk_not_idle:{e}"))?
    .ok_or_else(|| "root_mapper_expired_original_sdk_exit_missing".into())
}
