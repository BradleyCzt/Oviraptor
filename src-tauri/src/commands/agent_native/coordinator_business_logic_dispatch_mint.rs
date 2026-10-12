// Opaque fresh witness is constructed only after actual paid mint verification.
pub(crate) struct FreshBusinessLogicDispatchMint {
    connection: usize,
    root: String,
    assignment: String,
    child: String,
    worker: String,
    directory: std::path::PathBuf,
    event: i64,
    input: JsonValue,
}
// This API consumes a real paid Root publication, never a model JSON grant.
fn native_coordinator_mint_business_logic_child(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler},
        store,
    };
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(decision, "dispatch:business_logic")
    {
        return Err("root_did_not_select_business_logic".into());
    }
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let frame = NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &root.target_dir)?;
    let input = native_business_logic_input_collection_on(&tx, actor, &root.target_dir)?;
    let verify = |db: &rusqlite::Connection| -> Result<(), String> {
        crate::collaboration_events::dispatch_schema::verify(db)?;
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        native_coordinator_tick_authority(db, &root, actor)?;
        frame.verify_business_logic_dispatch_source(db, &root, actor)?;
        let basis = decision.tick.basis();
        if basis["phase"] != "business-logic-input"
            || basis["permittedSuggestion"] != "dispatch:business_logic"
            || basis["changedFact"] != frame.fact()
        {
            return Err("root_business_logic_dispatch_decision_frame_mismatch".into());
        }
        decision.tick.require_executable(db)?;
        if decision.tick.published(db, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_business_logic_dispatch_publication_changed".into());
        }
        budget::admission::require_determinate(db, &actor.root_run_id)?;
        budget::clock::remaining(db, &actor.root_run_id)?;
        if native_business_logic_input_collection_on(db, actor, &root.target_dir)?.fact()
            != input.fact()
        {
            return Err("root_business_logic_dispatch_original_input_changed".into());
        }
        Ok(())
    };
    verify(&tx)?;
    let trigger = "business_logic_frozen_evidence_ready";
    let dedup = format!("business_logic:{trigger}:{}:1", actor.target_key);
    let expected_id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", actor.root_run_id))[..24]
    );
    let inventory:(i64,i64)=tx.query_row("SELECT count(*),COALESCE(sum(CASE WHEN a.id=?2 AND a.role='business_logic' AND r.role='business_logic' AND a.coordinator_run_id=?1 AND r.root_run_id=?1 THEN 1 ELSE 0 END),0) FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE (a.coordinator_run_id=?1 OR r.root_run_id=?1) AND (a.role='business_logic' OR r.role='business_logic')",params![actor.root_run_id,expected_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if inventory.0 > 1 || inventory.0 != inventory.1 {
        return Err("root_business_logic_dispatch_original_worker_conflict".into());
    }
    if inventory.0 == 1 {
        let run_id: String = tx
            .query_row(
                "SELECT child_run_id FROM agent_assignments WHERE id=?1",
                [&expected_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let existing = scheduler::ScheduledChild {
            assignment_id: expected_id.clone(),
            run_id,
            role: AgentRole::BusinessLogic,
        };
        let source = business_logic_dispatch_receipt::verify_original_on(&tx, actor, &existing)?;
        if source.fact()["dispatchEventSequence"] != decision.event_sequence
            || source.fact()["originalFrame"] != frame.fact()
        {
            return Err("root_business_logic_dispatch_existing_decision_conflict".into());
        }
    }
    if inventory.0 == 0 {
        budget::supervision_capacity::require_business_logic_issue_headroom_on(&tx, actor, 1)?;
    }
    let (child, fresh) = scheduler::prepare_readonly_child_in_transaction(
        &tx,
        actor,
        AgentRole::BusinessLogic,
        trigger,
        input.task(),
        2000,
        verify,
    )?;
    if child.assignment_id != expected_id || fresh != (inventory.0 == 0) {
        return Err("root_business_logic_dispatch_original_grant_conflict".into());
    }
    if fresh {
        let grant:(i64,i64,String,String)=tx.query_row("SELECT reserved_tokens,reserved_requests,lane,capability_lease_json FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
        let mut caps: Vec<String> = serde_json::from_str(&grant.3)
            .map_err(|_| "root_business_logic_dispatch_original_grant_conflict")?;
        caps.sort();
        if grant.0 != 2000
            || grant.1 != 1
            || grant.2 != "read_only_analysis"
            || caps != vec!["evidence.read".to_string(), "mailbox.write".to_string()]
        {
            return Err("root_business_logic_dispatch_original_grant_conflict".into());
        }
        crate::agent_runtime::multi_agent::client_side::require_reviewer_floor(&tx, actor, 0, 0)?;
    }
    verify(&tx)?;
    if fresh {
        let worker =
            crate::agent_runtime::multi_agent::attempts::current(&tx, actor, &child.assignment_id)?;
        let fresh_mint = FreshBusinessLogicDispatchMint {
            connection: &*tx as *const rusqlite::Connection as usize,
            root: actor.root_run_id.clone(),
            assignment: child.assignment_id.clone(),
            child: child.run_id.clone(),
            worker: worker.id,
            directory: root.target_dir.clone(),
            event: decision.event_sequence,
            input: input.fact().clone(),
        };
        business_logic_dispatch_receipt::issue_new_on(&tx, actor, &child, &fresh_mint)?;
        verify(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(child)
}
