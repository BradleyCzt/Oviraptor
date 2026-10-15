//! This gate authorizes fresh work only. Receipt settlement never calls it:
//! known late costs must still be recorded while other costs are unresolved.
use super::{balance, DIMENSIONS};
use rusqlite::Connection;

pub(crate) fn require_determinate(db: &Connection, root: &str) -> Result<(), String> {
    require_dimensions(db, root)?;
    if let Some(fact) = super::clock::elapsed_fact::read(db, root)? {
        return Err(if fact.exhausted {
            "budget_wall_time_exhausted"
        } else {
            "budget_elapsed_finalized_no_new_work"
        }
        .into());
    }
    require_cost_determinate(db, root)
}

/// Financial history only. A finalized non-exhausted elapsed observation is
/// not a fresh execution grant. Continuation separately verifies its original
/// physical proof, all reservations and the still-valid absolute deadline.
pub(crate) fn require_financially_determinate(db: &Connection, root: &str) -> Result<(), String> {
    require_dimensions(db, root)?;
    if super::clock::elapsed_fact::read(db, root)?.is_some_and(|f| f.exhausted) {
        return Err("budget_wall_time_exhausted".into());
    }
    require_cost_determinate(db, root)
}

fn require_dimensions(db: &Connection, root: &str) -> Result<(), String> {
    let configured: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if configured != DIMENSIONS.len() as i64 {
        return Err("budget_history_requires_reconciliation".into());
    }
    Ok(())
}

pub(crate) fn require_determinate_for_fresh_public_surface_on(
    db:&Connection,
    proof:&crate::scanning::external_surface_response_receipt::ClaimedPublicSurfaceWitness,
)->Result<(),String> {
    let actor=proof.verify_unsent_on(db)?;
    require_dimensions(db,&actor.root_run_id)?;
    if let Some(fact)=super::clock::elapsed_fact::read(db,&actor.root_run_id)? {
        return Err(if fact.exhausted {"budget_wall_time_exhausted"} else {"budget_elapsed_finalized_no_new_work"}.into());
    }
    require_cost_determinate_inner(db,&actor.root_run_id,true)
}

fn require_cost_determinate(db:&Connection,root:&str)->Result<(),String> {
    require_cost_determinate_inner(db,root,false)
}

fn require_cost_determinate_inner(db: &Connection, root: &str, fresh_surface:bool) -> Result<(), String> {
    let uncertain:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1
          AND (state='uncertain' OR (state='received' AND json_extract(response_json,'$.usageReported')=0)))
        OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1
          AND (state='uncertain' OR (state='received' AND json_extract(response_json,'$.usageReported')=0)))
        OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'
          AND json_extract(receipt_json,'$.usageReported')=0)
        OR EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE root_run_id=?1
          AND (phase='uncertain' OR (phase='received' AND (
            json_extract(fact_json,'$.usageReported') IS NOT 1
            OR json_extract(fact_json,'$.usage.inputTokens')+json_extract(fact_json,'$.usage.outputTokens')
               IS NOT json_extract(fact_json,'$.usage.totalTokens')))))
        OR EXISTS(SELECT 1 FROM agent_web_model_journal d WHERE d.root_run_id=?1 AND d.phase='dispatch'
          AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal t WHERE t.call_id=d.call_id AND t.phase IN ('received','unsent') AND {}))",super::WEB_RECEIPT_BINDING),
        [root],|r|r.get(0)).map_err(|e|e.to_string())?;
    // A stopped dispatch is unknown even before its late callback can append
    // an uncertain fact. Keep reservations; this gate never manufactures fees.
    let stopped: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls c
        LEFT JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id AND x.assignment_id=c.assignment_id
        WHERE c.root_run_id=?1 AND c.state='executing'
          AND (x.id IS NULL OR x.state='expired' OR COALESCE(datetime(x.expires_at)>datetime('now','localtime'),0)=0)
          AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE f.family='specialist'
            AND f.root_run_id=c.root_run_id AND f.assignment_id=c.assignment_id AND f.child_run_id=c.child_run_id AND f.lease_attempt_id=x.id
            AND f.round_number=1 AND f.request_hash=c.request_hash AND f.lease_epoch=c.lease_epoch
            AND f.fencing_token=c.fencing_token AND (f.phase='received' OR (f.phase='unsent'
              AND f.fact_json=json_object('code','model_cancelled_before_transport')))))
        OR EXISTS(SELECT 1 FROM agent_source_model_rounds c
        LEFT JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id AND x.assignment_id=c.assignment_id
        WHERE c.root_run_id=?1 AND c.state='executing'
          AND (x.id IS NULL OR x.state='expired' OR COALESCE(datetime(x.expires_at)>datetime('now','localtime'),0)=0)
          AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE f.family='source-round'
            AND f.root_run_id=c.root_run_id AND f.assignment_id=c.assignment_id AND f.child_run_id=c.child_run_id AND f.lease_attempt_id=x.id
            AND f.round_number=c.round_number AND f.request_hash=c.request_hash AND f.lease_epoch=c.lease_epoch
            AND f.fencing_token=c.fencing_token AND (f.phase='received' OR (f.phase='unsent'
              AND f.fact_json=json_object('code','model_cancelled_before_transport')))))",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    let root_unclosed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal d WHERE d.root_run_id=?1 AND d.phase='dispatch'
        AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal t WHERE t.call_id=d.call_id AND t.root_run_id=d.root_run_id
          AND t.lease_attempt_id=d.lease_attempt_id AND t.round=d.round AND t.request_hash=d.request_hash AND t.phase IN ('received','unsent')))",
        [root],|r|r.get(0)).map_err(|e|e.to_string())?;
    // A v3 received invoice commits before additional cost settlement. It
    // remains an obligation until the original terminal proof is committed.
    let root_unsettled: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal j JOIN agent_root_tick_receipts p
        ON p.call_id=j.call_id AND p.root_run_id=j.root_run_id AND p.lease_attempt_id=j.lease_attempt_id
        AND p.round=j.round AND p.request_hash=j.request_hash AND p.phase='request'
        WHERE j.root_run_id=?1 AND j.phase='received' AND json_extract(p.fact_json,'$.request.version')=3
        AND NOT EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs t WHERE t.call_id=j.call_id AND t.root_run_id=j.root_run_id
          AND t.lease_attempt_id=j.lease_attempt_id AND t.round=j.round AND t.request_hash=j.request_hash
          AND t.basis_hash=p.basis_hash AND t.phase='received'))",[root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if uncertain || stopped || root_unclosed || root_unsettled {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    for dimension in DIMENSIONS {
        let unknown=balance(db, root, None, dimension)?.indeterminate;
        let permitted=if fresh_surface && dimension=="target_requests" {1} else {0};
        if unknown!=permitted {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
    }
    Ok(())
}

pub(crate) fn require_settled_for_completion(db: &Connection, root: &str) -> Result<(), String> {
    let configured: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !configured {
        let work:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
            OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')",
            [root],|r|r.get(0)).map_err(|e|e.to_string())?;
        return if work {
            Err("budget_history_requires_reconciliation".into())
        } else {
            Ok(())
        };
    }
    require_determinate(db, root)?;
    for dimension in DIMENSIONS {
        if balance(db, root, None, dimension)?.reserved != 0 {
            return Err("budget_reservations_require_settlement".into());
        }
    }
    Ok(())
}
