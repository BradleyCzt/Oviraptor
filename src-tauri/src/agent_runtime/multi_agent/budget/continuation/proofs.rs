//! Audit the original received calls, then capture their physical rows. A
//! ledger balance or a JSON phase label alone cannot establish paid history.
use super::super::{balance_for_attempt, historical::CostOwner, receipts, root::model::tick::Tick};
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{
        attempts::audit_rows::Rows, lease::CoordinatorLease, scheduler::ScheduledChild,
        source_rounds, specialist,
    },
    store::{stable_hash, UsageDelta},
};
use rusqlite::{params, Connection};
use serde_json::Value;

fn strings(db: &Connection, sql: &str, root: &str) -> Result<Vec<String>, String> {
    db.prepare(sql)
        .map_err(|e| e.to_string())?
        .query_map([root], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub(super) fn verify_calls(
    db: &Connection,
    root: &str,
    actor: Option<&CoordinatorLease>,
) -> Result<(), String> {
    if actor.is_none() {
        super::super::root::model::single_history::verify_settled_calls(db, root)?;
    } else {
        let single_exit: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_single_exit_receipts WHERE root_run_id=?1)",
            [root], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if single_exit {
            return Err("continuation_multi_single_exit_scope_conflict".into());
        }
    }
    let model_calls = if actor.is_some() {
        strings(db, "SELECT DISTINCT call_id FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY call_id", root)?
    } else {
        Vec::new()
    };
    for call in model_calls {
        let audit=Tick::audit_cost_for_continuation(db,root,&call)?;
        let paid_unpublished = audit["phase"]=="received" && audit["originalSemanticAvailable"]!=true;
        if paid_unpublished && Tick::read_original_cost_reconciliation(db,root,&call)?.is_none() {
            return Err("continuation_unpublished_paid_confirmation_required".into());
        }
    }
    // Reject extra control/terminal rows or scalar charges with no original
    // provider call. Tick verifies the opposite direction and exact bindings.
    let orphan: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_root_tick_receipts p WHERE p.root_run_id=?1
          AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal j WHERE j.root_run_id=p.root_run_id AND j.call_id=p.call_id))
        OR EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs p WHERE p.root_run_id=?1
          AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal j WHERE j.root_run_id=p.root_run_id AND j.call_id=p.call_id))
        OR EXISTS(SELECT 1 FROM agent_budget_entries e WHERE e.root_run_id=?1 AND e.assignment_id=''
          AND e.dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests')
          AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal j WHERE j.root_run_id=e.root_run_id AND j.call_id=e.source_id))
        OR EXISTS(SELECT 1 FROM agent_web_model_journal t WHERE t.root_run_id=?1 AND t.phase IN ('received','unsent')
          AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal d WHERE d.root_run_id=t.root_run_id
            AND d.call_id=t.call_id AND d.phase='dispatch' AND d.assignment_id=t.assignment_id
            AND d.child_run_id=t.child_run_id AND d.lease_epoch=t.lease_epoch AND d.fencing_token=t.fencing_token
            AND d.round=t.round AND d.request_hash=t.request_hash))",
        [root],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if orphan {
        return Err("continuation_original_call_proof_missing".into());
    }
    if let Some(actor) = actor {
        verify_children(db, root, actor)?;
    } else {
        let children:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE root_run_id=?1 AND id<>?1)
            OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>'')",
            [root],|r|r.get(0)).map_err(|e|e.to_string())?;
        if children {
            return Err("continuation_single_child_scope_conflict".into());
        }
    }
    super::targets::verify(db, root)?;
    Ok(())
}

fn usage(text: &str) -> Result<UsageDelta, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "continuation_original_usage_invalid")?;
    let number = |key: &str| {
        v[key]
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or_else(|| "continuation_original_usage_invalid".to_string())
    };
    let input_tokens = number("inputTokens")?;
    let cached_input_tokens = number("cachedInputTokens")?;
    let output_tokens = number("outputTokens")?;
    let model_requests = number("modelRequests")?;
    if cached_input_tokens > input_tokens
        || input_tokens.checked_add(output_tokens) != Some(number("totalTokens")?)
    {
        return Err("continuation_original_usage_invalid".into());
    }
    Ok(UsageDelta {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        model_requests,
    })
}

fn verify_children(db: &Connection, root: &str, actor: &CoordinatorLease) -> Result<(), String> {
    let mut q=db.prepare("SELECT DISTINCT child_run_id FROM agent_specialist_calls WHERE root_run_id=?1
        UNION SELECT child_run_id FROM agent_source_model_rounds WHERE root_run_id=?1
        UNION SELECT child_run_id FROM agent_web_model_journal WHERE root_run_id=?1
        UNION SELECT child_run_id FROM agent_assignment_attempts x WHERE x.root_run_id=?1 AND EXISTS(
          SELECT 1 FROM agent_budget_entries e WHERE e.root_run_id=x.root_run_id AND e.lease_attempt_id=x.id)
        ORDER BY 1").map_err(|e|e.to_string())?;
    let children = q
        .query_map([root], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for child in children {
        let owner =
            CostOwner::for_run(db, &child)?.ok_or("continuation_original_child_required")?;
        if owner.lease.root_run_id != root
            || owner.lease.scan_id != actor.scan_id
            || owner.lease.attempt_number != actor.attempt_number
            || owner.lease.target_key != actor.target_key
            || owner.lease.lease_epoch != actor.lease_epoch
            || owner.lease.fencing_token != actor.fencing_token
        {
            return Err("continuation_original_child_conflict".into());
        }
        owner.verify(db)?;
        receipts::original_owner(db, &child, actor, &owner.assignment)?.verify(db)?;
        let (role,specialist_count,source_count,web_count):(String,i64,i64,i64)=db.query_row(
            "SELECT r.role,(SELECT count(*) FROM agent_specialist_calls c WHERE c.child_run_id=r.id),
              (SELECT count(*) FROM agent_source_model_rounds c WHERE c.child_run_id=r.id),
              (SELECT count(*) FROM agent_web_model_journal c WHERE c.child_run_id=r.id)
             FROM agent_runs r WHERE r.id=?1",[&child],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).map_err(|e|e.to_string())?;
        let child_role =
            AgentRole::try_parse(&role).ok_or("continuation_original_child_role_invalid")?;
        let expected = if specialist_count == 1 && source_count == 0 && web_count == 0 {
            specialist::verify_received_original(db, actor, &owner.assignment)?;
            let raw: String = db
                .query_row(
                    "SELECT usage_json FROM agent_specialist_calls
                WHERE root_run_id=?1 AND child_run_id=?2 AND state='received'",
                    params![root, child],
                    |r| r.get(0),
                )
                .map_err(|_| "continuation_original_specialist_receipt_required")?;
            usage(&raw)?
        } else if specialist_count == 0 && source_count > 0 && web_count == 0 {
            let scheduled = ScheduledChild {
                assignment_id: owner.assignment.clone(),
                run_id: child.clone(),
                role: child_role,
            };
            if source_rounds::audit_unsent_first(db, actor, &scheduled)? {
                UsageDelta::default()
            } else if owner.attempt.state == "completed" {
                source_rounds::audit_completion(db, actor, &scheduled)?.usage
            } else if owner.attempt.state == "failed" {
                source_rounds::audit_exhausted(db, actor, &scheduled)?
            } else {
                return Err("continuation_original_source_receipt_required".into());
            }
        } else if specialist_count == 0 && source_count == 0 && web_count > 0 {
            super::super::web_lifetime::child_usage_original(db, actor, &owner.assignment, &child)?
        } else if specialist_count == 0 && source_count == 0 && web_count == 0 {
            UsageDelta::default()
        } else {
            return Err("continuation_original_child_call_family_conflict".into());
        };
        for (dimension, amount) in super::super::DIMENSIONS[..4].iter().zip([
            expected.input_tokens,
            expected.cached_input_tokens,
            expected.output_tokens,
            expected.model_requests,
        ]) {
            let paid =
                balance_for_attempt(db, root, &owner.assignment, &owner.attempt.id, dimension)?;
            if paid.reserved != 0 || paid.indeterminate != 0 || paid.consumed != amount {
                return Err("continuation_original_child_cost_conflict".into());
            }
        }
    }
    // Received fee-only facts must be the exact original received call. Unknown
    // and orphan facts cannot be laundered through a zero aggregate balance.
    let bad: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE f.root_run_id=?1 AND
      (f.phase NOT IN ('received','unsent') OR NOT EXISTS(SELECT 1 FROM agent_assignment_attempts x
        WHERE x.id=f.lease_attempt_id AND x.root_run_id=f.root_run_id AND x.assignment_id=f.assignment_id
        AND x.child_run_id=f.child_run_id AND x.coordinator_epoch=f.lease_epoch AND x.coordinator_fencing_token=f.fencing_token)
       OR NOT ((f.phase='received' AND f.family='specialist' AND f.round_number=1 AND EXISTS(SELECT 1 FROM agent_specialist_calls c
         WHERE c.root_run_id=f.root_run_id AND c.assignment_id=f.assignment_id AND c.child_run_id=f.child_run_id
           AND c.lease_epoch=f.lease_epoch AND c.fencing_token=f.fencing_token AND c.request_hash=f.request_hash
           AND c.state='received' AND c.response_hash=json_extract(f.fact_json,'$.responseHash') AND c.usage_json=json_extract(f.fact_json,'$.usage')))
         OR (f.phase='received' AND f.family='source-round' AND EXISTS(SELECT 1 FROM agent_source_model_rounds c
         WHERE c.root_run_id=f.root_run_id AND c.assignment_id=f.assignment_id AND c.child_run_id=f.child_run_id
           AND c.lease_epoch=f.lease_epoch AND c.fencing_token=f.fencing_token AND c.round_number=f.round_number AND c.request_hash=f.request_hash
           AND c.state='received' AND c.response_hash=json_extract(f.fact_json,'$.responseHash') AND c.usage_json=json_extract(f.fact_json,'$.usage')))
         OR (f.phase='unsent' AND f.family='source-round' AND f.round_number=1
           AND f.fact_json=json_object('code','model_cancelled_before_transport') AND EXISTS(SELECT 1 FROM agent_source_model_rounds c
             WHERE c.root_run_id=f.root_run_id AND c.assignment_id=f.assignment_id AND c.child_run_id=f.child_run_id
               AND c.round_number=1 AND c.lease_epoch=f.lease_epoch AND c.fencing_token=f.fencing_token AND c.request_hash=f.request_hash
               AND c.state='executing' AND c.response_json='{}' AND c.usage_json='{}' AND c.response_hash=''
               AND c.event_sequence=0 AND c.failure_code='' AND c.finished_at='')))))",
      [root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if bad {
        return Err("continuation_original_model_cost_proof_conflict".into());
    }
    Ok(())
}

fn row_hash(rows: &Rows) -> String {
    // Type-tagged rusqlite Values preserve physical distinctions; raw plans,
    // invoices and request payloads remain private and are never returned.
    stable_hash(&format!("{:?}:{:?}", rows.columns, rows.values))
}

pub(super) fn physical_hashes(db: &Connection, root: &str, single: bool) -> Result<[String; 3], String> {
    let root_rows = Rows::read(db, "SELECT rowid,* FROM agent_runs WHERE id=?1", [root])?;
    let owner_rows = Rows::read(
        db,
        "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1",
        [root],
    )?;
    let mut hashes = vec![row_hash(&root_rows), row_hash(&owner_rows)];
    for table in [
        "agent_budget_limits",
        "agent_budget_entries",
        "agent_budget_clock_origins",
        "agent_root_elapsed_facts",
        "agent_budget_ledger",
        "agent_root_model_journal",
        "agent_root_tick_receipts",
        "agent_root_tick_terminal_proofs",
        "agent_root_cost_reconciliation_receipts",
        "agent_root_mode_definitions",
        "agent_root_budget_definitions",
        "agent_specialist_calls",
        "agent_source_model_rounds",
        "agent_web_model_journal",
        "agent_model_cost_facts",
        "agent_assignment_attempts",
    ] {
        hashes.push(row_hash(&Rows::read(
            db,
            &format!("SELECT rowid,* FROM {table} WHERE root_run_id=?1 ORDER BY rowid"),
            [root],
        )?));
    }
    for sql in ["SELECT rowid,* FROM agent_assignments WHERE coordinator_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_runs WHERE root_run_id=?1 AND id<>?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_http_request_claims WHERE run_id IN(SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_source_tool_receipts WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_external_surface_captures WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_authorization_probe_claims WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_evidence_nodes WHERE root_run_id=?1 ORDER BY rowid"] {
        hashes.push(row_hash(&Rows::read(db,sql,[root])?));
    }
    // Single history is anchored by its original exit, not Multi Tick. Keep
    // the established Multi fingerprint bytes unchanged for signed histories.
    if single {
        hashes.push(row_hash(&Rows::read(db,
            "SELECT rowid,* FROM main.agent_single_exit_receipts WHERE root_run_id=?1 ORDER BY rowid",
            [root])?));
    }
    Ok([
        hashes[0].clone(),
        hashes[1].clone(),
        stable_hash(&hashes.join(":")),
    ])
}
