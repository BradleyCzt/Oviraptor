// U177: a paid original-Root choice of `dispatch:input_parser`, bound to the
// frozen U175 frame it was paid for, mints exactly one real InputParser child run
// (clears `scheduled_child_mint` and `reviewer_floor_grant_wire`). The grant is
// fixed at 4000 model tokens / 1 model request / 0 target requests. Before the
// reservation the Root must still hold Reviewer floor 15000/1 + Root supervision
// floor 16000/1 + the grant, and after it both floors, otherwise
// `input_parser_budget_unavailable` rolls the whole issuance back (zero writes).
// The paid decision, the frame and the fencing are re-verified inside the same
// IMMEDIATE issuance transaction, before and after the reservation. No SDK call
// is made here (U178) and the dispatch gate stays not_ready until U181.
const NATIVE_INPUT_PARSER_GRANT_TOKENS: i64 = 4_000;
const NATIVE_INPUT_PARSER_GRANT_REQUESTS: i64 = 1;
const NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_TOKENS: i64 = 16_000;
const NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_REQUESTS: i64 = 1;
const NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE: &str = "input_parser_budget_unavailable";
const NATIVE_INPUT_PARSER_CHILD_TRIGGER: &str = "input_parser_trigger";

/// Retained capacity no InputParser grant may touch: Reviewer + Root supervision.
fn native_input_parser_retained_floor() -> (i64, i64) {
    use crate::agent_runtime::multi_agent::client_side::{
        REVIEWER_FLOOR_REQUESTS, REVIEWER_FLOOR_TOKENS,
    };
    (
        REVIEWER_FLOOR_TOKENS + NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_TOKENS,
        REVIEWER_FLOOR_REQUESTS + NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_REQUESTS,
    )
}

/// Current remaining child capacity must cover both floors plus `tokens/requests`.
fn native_input_parser_require_headroom(
    db: &rusqlite::Connection,
    root_run_id: &str,
    tokens: i64,
    requests: i64,
) -> Result<(), String> {
    let (floor_tokens, floor_requests) = native_input_parser_retained_floor();
    let (available_tokens, available_requests) =
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(db, root_run_id)?;
    if available_tokens.is_some_and(|n| n < floor_tokens + tokens)
        || available_requests.is_some_and(|n| n < floor_requests + requests)
    {
        return Err(NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE.into());
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_mint_input_parser_child(
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
        || !native_root_paid_suggestion_selected(decision, NATIVE_INPUT_PARSER_DISPATCH_STEP)
    {
        return Err("root_did_not_select_input_parser".into());
    }
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let basis = decision.tick.basis();
    let verify = |db: &rusqlite::Connection, frame: &NativeCoordinatorFrame| -> Result<(), String> {
        crate::collaboration_events::dispatch_schema::verify(db)?;
        native_coordinator_tick_authority(db, &root, actor)?;
        frame.verify(db, &root, actor)?;
        // The paid decision must have been made on exactly this frozen frame.
        if basis["phase"] != frame.kind
            || basis["permittedSuggestion"] != frame.step()
            || basis["changedFact"] != frame.fact()
        {
            return Err("root_input_parser_decision_frame_mismatch".into());
        }
        decision.tick.require_executable(db)?;
        budget::admission::require_determinate(db, &actor.root_run_id)?;
        if decision.tick.published(db, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_decision_publication_changed".into());
        }
        Ok(())
    };
    db.authorizer(Some(native_coordinator_bootstrap_authorize))
        .map_err(|e| e.to_string())?;
    let result =
        (|| -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
            let tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| format!("input_parser_mint_lock:{e}"))?;
            let frame = NativeCoordinatorFrame::input_parser_trigger(&tx, actor, &root.target_dir)?;
            verify(&tx, &frame)?;
            let dedup = format!(
                "{}:{NATIVE_INPUT_PARSER_CHILD_TRIGGER}:{}:1",
                AgentRole::InputParser.as_str(),
                actor.target_key
            );
            let id = format!(
                "asg-{}",
                &store::stable_hash(&format!("{}:{dedup}", actor.root_run_id))[..24]
            );
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1)",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                native_input_parser_require_headroom(
                    &tx,
                    &actor.root_run_id,
                    NATIVE_INPUT_PARSER_GRANT_TOKENS,
                    NATIVE_INPUT_PARSER_GRANT_REQUESTS,
                )?;
            }
            let (floor_tokens, floor_requests) = native_input_parser_retained_floor();
            let task = json!({"schemaVersion":1,"role":AgentRole::InputParser.as_str(),
                "frame":{"kind":frame.kind,"decisionHash":frame.semantic["decisionHash"],
                    "evidenceRefs":frame.semantic["trigger"]["evidenceRefs"],
                    "factBindings":frame.semantic["factBindings"]},
                "rootDecision":{"step":NATIVE_INPUT_PARSER_DISPATCH_STEP,"eventSequence":decision.event_sequence,
                    "summaryHash":store::stable_hash(&decision.summary.as_json().to_string()),
                    "basisHash":store::stable_hash(&basis.to_string())},
                "grant":{"reservedModelTokens":NATIVE_INPUT_PARSER_GRANT_TOKENS,
                    "reservedModelRequests":NATIVE_INPUT_PARSER_GRANT_REQUESTS,"targetRequestsGranted":0,
                    "retainedFloor":{"modelTokens":floor_tokens,"modelRequests":floor_requests}}});
            let (child, fresh) = scheduler::prepare_readonly_child_in_transaction(
                &tx,
                actor,
                AgentRole::InputParser,
                NATIVE_INPUT_PARSER_CHILD_TRIGGER,
                &task,
                NATIVE_INPUT_PARSER_GRANT_TOKENS,
                |db| verify(db, &frame),
            )?;
            if fresh == exists || child.assignment_id != id {
                return Err("input_parser_original_grant_conflict".into());
            }
            // The reservation actually written is the fixed grant on the readonly lane.
            let grant: (i64, i64, String) = tx
                .query_row(
                    "SELECT reserved_tokens,reserved_requests,lane FROM agent_assignments WHERE id=?1",
                    [&child.assignment_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|e| e.to_string())?;
            let closed = !fresh && input_parser_completed_original_replay(&tx, actor, &child, &root.target_dir)?;
            let expected = if closed { (0, 0, "read_only_analysis".to_string()) } else {
                (NATIVE_INPUT_PARSER_GRANT_TOKENS, NATIVE_INPUT_PARSER_GRANT_REQUESTS, "read_only_analysis".to_string())
            };
            if grant != expected {
                return Err("input_parser_original_grant_conflict".into());
            }
            if fresh {
                native_input_parser_require_headroom(&tx, &actor.root_run_id, 0, 0)?;
            }
            verify(&tx, &frame)?;
            if fresh {
                tx.commit()
                    .map_err(|e| format!("input_parser_mint_commit:{e}"))?;
            }
            Ok(child)
        })();
    use rusqlite::hooks::{AuthContext, Authorization};
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let child = result?;
    clear?;
    Ok(child)
}
