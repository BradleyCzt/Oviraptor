// Advisory paid assessment of one already signed and closed WAF observation.
// No replacement Web child, target request, capability or C renewal follows.
fn native_coordinator_protection_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let (frame, original_closed) =
        NativeCoordinatorFrame::protection_closed_on(&tx, &root, actor, receipt_id)?;
    native_supervision_frame_admission(frame.kind)?;
    let basis = native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
    if frame.step() != "assess:protection_changed"
        || basis["protectionSupervision"] != native_protection_supervision_protocol_on(&tx, actor)?
        || basis["permittedSuggestion"] != "assess:protection_changed"
    {
        return Err("root_protection_original_step_invalid".into());
    }
    frame.verify_protection_source(&tx, &root, actor)?;
    tx.commit().map_err(|e| e.to_string())?;
    // Keep both original guards across every paid SDK round, financial receipt,
    // semantic publication and original saved-decision replay.
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    if !decision.summary.as_json()["suggestions"]
        .as_array()
        .is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str() == Some("assess:protection_changed"))
        })
    {
        return Err("root_protection_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify_protection_source(&tx, &root, actor)?;
    for row in original_closed.verification_rows() {
        row.verify(&tx)?;
    }
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_protection_publication_changed".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    drop(original_closed);
    Ok(decision)
}

// Both first delivered closure and whole execution replay assess only the
// original issued response. An unsealed WAF result stays stopped; absence never
// causes a file-based signature, target retry or a newly created receipt.
fn multi_agent_protection_supervision(
    context: &AgentRunContext,
    session: &MultiAgentSession,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    if !matches!(outcome, AgentTargetOutcome::Limited(stop) if stop.code==AGENT_STOP_WAF) {
        return Ok(());
    }
    let db = db::open(&context.db_path)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let mode = crate::agent_runtime::web_mode::root::read(&snapshot, &session.lease.root_run_id)?;
    if !mode.is_some_and(|mode| mode.protection_supervision().is_some()) {
        return Ok(());
    }
    let mut q = snapshot
        .prepare(
            "SELECT receipt_id,run_id FROM main.agent_protection_response_receipts
         WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4
         ORDER BY storage_id",
        )
        .map_err(|e| e.to_string())?;
    let receipts = q
        .query_map(
            params![
                session.lease.root_run_id,
                session.lease.scan_id,
                session.lease.attempt_number,
                session.lease.target_key
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    drop(snapshot);
    match receipts.as_slice() {
        [] => Ok(()),
        [(receipt, author)] if author == &session.executor.run_id => {
            session.supervisor.check()?;
            native_coordinator_protection_feedback(context, &session.lease, receipt)?;
            Ok(())
        }
        _ => Err("root_protection_original_receipt_scope_conflict".into()),
    }
}
