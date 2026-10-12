fn native_financial_continuation_finish(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt: &NativeCoordinatorTickReceipt,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::{clock, root::model::tick::Tick};
    let db = db::open(&context.db_path)?;
    clock::financial_assessment_write(&db, actor, |tx| {
        let fact = verify_financial_continuation_birth_on(tx, &actor.root_run_id)?;
        native_financial_continuation_require_authority_in(tx, &fact)?;
        if receipt.tick.published(tx, &receipt.saved)? != Some(receipt.event_sequence) {
            return Err("financial_continuation_publication_unverified".into());
        }
        let projection = Tick::project_publication(tx, &actor.root_run_id, receipt.event_sequence)?;
        if projection.scan != context.scan_id || projection.attempt != context.attempt_number {
            return Err("financial_continuation_publication_scope_changed".into());
        }
        let outcome = AgentTargetOutcome::Incomplete(AgentStop::new(
            "financial_continuation_assessment_only",
            "The explicitly approved fresh assessment was published. Remaining target work was not executed."));
        finish_coordinator_run_in_transaction(tx, actor, &outcome)?;
        let changed = tx.execute("UPDATE sentinel_scans SET status='partial',current_checkpoint='financial_assessment_published',
            updated_at=datetime('now','localtime') WHERE id=?1 AND attempt_count=?2 AND status='scanning'",
            params![context.scan_id,context.attempt_number]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("financial_continuation_closure_scope_invalid".into());
        }
        let changed = tx.execute("UPDATE sentinel_scan_attempts SET status='partial',stage='agent_execution',
            checkpoint='financial_assessment_published',stop_reason='financial_continuation_assessment_only',
            finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime')
            WHERE scan_id=?1 AND attempt_number=?2 AND status='scanning'",
            params![context.scan_id,context.attempt_number]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("financial_continuation_closure_scope_invalid".into());
        }
        verify_financial_continuation_birth_on(tx, &actor.root_run_id)?;
        Ok(())
    })
}
