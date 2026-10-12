/// Original completed invoices or proven no-dispatch/unsent exits permit
/// financial settlement after C expiry. This proves no live authority and cannot publish a result or dispatch.
pub(crate) fn settle_original_web_receipt(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    child: &str,
) -> Result<
    (
        UsageDelta,
        Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
    ),
    String,
> {
    let mode = crate::agent_runtime::web_mode::root::read(tx, &lease.root_run_id)?
        .ok_or("web_model_settlement_original_mode_missing")?;
    if mode.new_evidence_supervision().is_none() {
        return Err("web_model_settlement_original_protocol_missing".into());
    }
    super::root::RootOwner::load_original(tx, &lease.root_run_id)?
        .require_original_coordinator(tx, lease)?;
    let original = super::receipts::original_owner(tx, child, lease, assignment)?;
    original.verify(tx)?;
    let worker = super::super::attempts::current(tx, lease, assignment)?;
    // Child-only inventory must not let a foreign-scope journal disappear from
    // the original Root usage query and turn a paid call into a zero bill.
    let foreign_journal: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?1
         AND (root_run_id<>?2 OR assignment_id<>?3 OR lease_epoch<>?4 OR fencing_token<>?5))",
            params![
                child,
                lease.root_run_id,
                assignment,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if foreign_journal {
        return Err("web_model_lifetime_original_call_conflict".into());
    }
    let (dispatches, journal_rows): (i64, i64) = tx
        .query_row(
            "SELECT count(*) FILTER(WHERE phase='dispatch'),count(*) FROM agent_web_model_journal
         WHERE child_run_id=?1",
            [child],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if dispatches == 0 {
        let effects: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')
            OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)", [child], |r|r.get(0))
            .map_err(|e|e.to_string())?;
        if journal_rows != 0 || effects {
            return Err("web_model_settlement_original_receipt_required".into());
        }
        // A prior SDK admission has a durable ownership inode even if its
        // journal was lost. Absence of DB rows alone never proves no transport.
        let path = tx
            .path()
            .filter(|p| !p.is_empty())
            .ok_or("web_model_lifetime_database_missing")?;
        let admitted = crate::agent_runtime::execution_owner::probe_native_invocation(
            std::path::Path::new(path),
            &lease.scan_id,
            lease.attempt_number,
            super::web_lifetime::WEB_MODEL_INVOCATION_KIND,
            child,
        )
        .map_err(|_| "web_model_settlement_original_receipt_required")?;
        if admitted.is_some() {
            return Err("web_model_settlement_original_receipt_required".into());
        }
    }
    let web: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
        AND role='web_executor' AND backend='native' AND orchestration_policy='multi')",
            [child],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if worker.child_run_id != child || !web {
        return Err("web_model_settlement_original_receipt_required".into());
    }
    let mut idle = super::web_lifetime::require_idle_original(tx, lease)?;
    let path = tx
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("web_model_lifetime_database_missing")?;
    let execution = crate::agent_runtime::execution_owner::probe_native_invocation(
        std::path::Path::new(path),
        &lease.scan_id,
        lease.attempt_number,
        "web-executor-original-execution",
        child,
    )
    .map_err(|e| format!("web_executor_original_not_idle:{e}"))?
    .ok_or("web_executor_original_exit_proof_missing")?;
    idle.push(execution);
    let running: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')",
            [child],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if running {
        return Err("web_executor_original_tools_not_idle".into());
    }
    let unknown_http: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1 AND received_at='')",
        [child], |r|r.get(0)).map_err(|e|e.to_string())?;
    if unknown_http {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    let usage = super::web_lifetime::child_usage_original(tx, lease, assignment, child)?;
    // This adapter is private to this receipt-verified function. Release only
    // settles unspent original reservations; it does not mint replacement work.
    let write: BudgetWrite = |tx, lease, assignment, dimension, kind, amount, key, source| {
        super::append_inner(
            tx,
            lease,
            assignment,
            dimension,
            kind,
            amount,
            key,
            source,
            super::Authority::BoundReceipt,
        )
    };
    settle_with(
        tx,
        lease,
        assignment,
        &usage,
        write,
        super::Authority::BoundReceipt,
    )?;
    original.verify(tx)?;
    Ok((usage, idle))
}
