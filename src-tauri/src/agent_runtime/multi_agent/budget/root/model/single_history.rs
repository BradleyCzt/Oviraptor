//! Read original Single invoices through their own closed financial interval.
//! This audit never creates authority, settles unknown usage or renews a clock.
use super::*;

pub(crate) fn verify_settled_calls(db: &Connection, root: &str) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    let owner = RootOwner::load_single(db, root)?;
    let _idle = owner.require_single_deletion_idle(db)?;
    let exit = owner.read_single_exit(db)?;
    if exit.unsettled_ms != 0 {
        return Err("continuation_single_interval_unsettled".into());
    }
    let foreign_protocol: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.agent_root_tick_receipts WHERE root_run_id=?1)
         OR EXISTS(SELECT 1 FROM main.agent_root_tick_terminal_proofs WHERE root_run_id=?1)",
        [root], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if foreign_protocol {
        return Err("continuation_single_tick_scope_conflict".into());
    }
    let mut query = db.prepare("SELECT call_id,round,request_hash,receipt_json
        FROM main.agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch' ORDER BY round")
        .map_err(|e| e.to_string())?;
    let calls = query.query_map([root], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?,
        r.get::<_, String>(2)?, r.get::<_, String>(3)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    let finite: bool = db.query_row("SELECT hard_limit IS NOT NULL FROM main.agent_budget_limits
        WHERE root_run_id=?1 AND dimension='model_input_tokens'", [root], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    for dispatch in calls {
        verify_call(db, &owner, finite, dispatch)?;
    }
    owner.read_single_exit(db)?;
    Ok(())
}

/// Read one original Single bill, without declaring its whole Root closed.
/// No exit, clock renewal, cost transition or transport grant is created here.
/// A prior known bill can remain known while another call is still uncertain.
pub(crate) fn verify_settled_bill_on(db: &Connection, root: &str, call: &str) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    let owner = RootOwner::load_single(db, root)?;
    let foreign: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_root_tick_receipts WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM main.agent_root_tick_terminal_proofs WHERE root_run_id=?1)",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("continuation_single_tick_scope_conflict".into());
    }
    let dispatch = db.query_row("SELECT call_id,round,request_hash,receipt_json
        FROM main.agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='dispatch'",
        params![root,call], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .map_err(|_| "continuation_single_dispatch_required")?;
    let finite: bool = db.query_row("SELECT hard_limit IS NOT NULL FROM main.agent_budget_limits
        WHERE root_run_id=?1 AND dimension='model_input_tokens'", [root], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    verify_call(db, &owner, finite, dispatch)
}

fn verify_call(db: &Connection, owner: &RootOwner, finite: bool,
    (id, round, request, raw): (String, i64, String, String)) -> Result<(), String> {
    let root = owner.root.as_str();
    // A single-bill read does not borrow the whole Root's exit/inode proof.
    // Validate its own canonical original identity before matching exact fees.
    let dispatch: Value = serde_json::from_str(&raw).map_err(|_| "continuation_single_dispatch_invalid")?;
    let canonical_dispatch = dispatch.to_string();
    if canonical_dispatch != raw || dispatch.as_object().is_none_or(|v| v.len() != 2)
        || dispatch["reservedRequests"] != 1 || round < 1
        || request.len() != 64 || !request.bytes().all(|b| b.is_ascii_hexdigit())
        || id != crate::agent_runtime::store::stable_hash(&json!({"owner":owner.id,
            "root":root,"round":round,"request":request}).to_string()) {
        return Err("continuation_single_dispatch_invalid".into());
    }
    let tokens = dispatch["reservedTokens"].as_i64().filter(|n| *n > 0)
        .ok_or("continuation_single_dispatch_invalid")?;
    let call = RootModelCall {owner: owner.clone(), id, round, request, tokens, remaining: Duration::ZERO};
    call.verify(db, "dispatch", &call.dispatch())?;
    let terminals: Vec<(String, String)> = db.prepare("SELECT phase,receipt_json
        FROM main.agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase<>'dispatch' ORDER BY phase")
        .map_err(|e| e.to_string())?.query_map(params![root, call.id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?;
    let [(phase, raw)] = terminals.as_slice() else {
        return Err("continuation_single_terminal_required".into());
    };
    let fact: Value = serde_json::from_str(raw).map_err(|_| "continuation_single_terminal_invalid")?;
    let canonical = fact.to_string();
    if canonical != *raw {
        return Err("continuation_single_terminal_invalid".into());
    }
    call.verify(db, phase, &fact)?;
    let values = match phase.as_str() {
        "unsent" => {
            // The original Single gateway only issues BeforeTransport for
            // cancellation. Network/provider failures have unknown outcome
            // and must never masquerade as a refundable no-send receipt.
            if fact.as_object().is_none_or(|v| v.len() != 1)
                || fact["code"] != crate::agent_runtime::model::gateway::ModelError::Cancelled.code() {
                return Err("continuation_single_unsent_proof_invalid".into());
            }
            [0; 4]
        }
        "received" => {
            let hash = fact["responseHash"].as_str().ok_or("continuation_single_terminal_invalid")?;
            if fact.as_object().is_none_or(|v| v.len() != 3) || fact["usageReported"] != true
                || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err("continuation_single_known_usage_required".into());
            }
            let read = |key: &str| fact["usage"][key].as_i64().filter(|n| *n >= 0)
                .ok_or("continuation_single_usage_invalid");
            let usage = crate::agent_runtime::store::UsageDelta {
                input_tokens: read("inputTokens")?, cached_input_tokens: read("cachedInputTokens")?,
                output_tokens: read("outputTokens")?, total_tokens: read("totalTokens")?,
                model_requests: read("modelRequests")?,
            };
            if usage.as_json() != fact["usage"] || usage.model_requests != 1
                || usage.cached_input_tokens > usage.input_tokens
                || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
                || finite && usage.total_tokens > tokens {
                return Err("continuation_single_usage_invalid".into());
            }
            [usage.input_tokens, usage.cached_input_tokens, usage.output_tokens, 1]
        }
        _ => return Err("continuation_single_known_usage_required".into()),
    };
    let mut expected = Vec::new();
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        let mut held = if index == 3 { 1 } else { tokens };
        let mut add = |kind: &str, amount: i64, suffix: &str| -> Result<(), String> {
            if amount > 0 {
                let key = budget::root::stored_key(db, root, &owner.id,
                    &format!("root-model:{}:{dimension}:{suffix}", call.id))?;
                expected.push((String::new(), owner.id.clone(), (*dimension).to_owned(), kind.to_owned(), amount, key));
            }
            Ok(())
        };
        add("reserve", held, "reserve")?;
        if phase == "unsent" {
            add("release", held, "terminal")?;
        } else {
            if values[index] > held {
                add("reserve", values[index] - held, "unlimited")?;
                held = values[index];
            }
            add("consume", values[index], "terminal")?;
            add("release", held - values[index], "release")?;
        }
    }
    let actual = db.prepare("SELECT assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key
        FROM main.agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 ORDER BY assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key")
        .map_err(|e| e.to_string())?.query_map(params![root, call.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?,
            r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, String>(5)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    expected.sort();
    if actual != expected {
        return Err("continuation_single_original_cost_conflict".into());
    }
    Ok(())
}
