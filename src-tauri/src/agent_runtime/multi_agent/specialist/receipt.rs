//! Specialist result verification and atomic, redacted model receipt.
use super::{binding, load, CallRow, PendingCall, StoredResponse};
use crate::agent_runtime::{
    checkpoint::{self, RunState},
    contract::AgentEventKind,
    secrets::redact_text_with,
    store::{self, UsageDelta},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

mod failed_cost;
mod private_writer;

pub(super) fn record_failed_uncertain(db: &Connection, call: &PendingCall) -> Result<(), String> {
    failed_cost::uncertain(db, call)
}


fn read_usage(value: &Value) -> Result<UsageDelta, String> {
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
            .ok_or_else(|| "specialist_usage_invalid".to_string())
    };
    let usage = UsageDelta {
        input_tokens: number("inputTokens")?,
        cached_input_tokens: number("cachedInputTokens")?,
        output_tokens: number("outputTokens")?,
        total_tokens: number("totalTokens")?,
        model_requests: number("modelRequests")?,
    };
    if usage.model_requests != 1
        || usage.cached_input_tokens > usage.input_tokens
        || usage.input_tokens > usage.total_tokens
        || usage.output_tokens > usage.total_tokens
    {
        return Err("specialist_usage_invalid".into());
    }
    Ok(usage)
}

fn receipt_hash(call: &PendingCall, response: &Value, usage: &Value) -> String {
    store::stable_hash(&json!({"assignment":call.child.assignment_id,"child":call.child.run_id,
        "root":call.lease.root_run_id,"epoch":call.lease.lease_epoch,"fence":call.lease.fencing_token,
        "requestHash":call.request_hash,"response":response,"usage":usage}).to_string())
}

fn event_payload(db: &Connection, call: &PendingCall, usage: &UsageDelta) -> Result<Value, String> {
    let ids = super::super::directive::source_guidance::delivered_ids(
        db,
        &call.lease,
        &call.child,
        false,
    )?;
    Ok(
        json!({"turns":1,"modelRequests":usage.model_requests,"totalTokens":usage.total_tokens,
        "uncachedInputTokens":usage.uncached_input(),"toolCalls":[],"deliveredDirectiveIds":ids}),
    )
}

pub(super) fn verify_response(
    connection: &Connection,
    call: &PendingCall,
    row: &CallRow,
) -> Result<StoredResponse, String> {
    if row.state != "received"
        || row.sequence < 1
        || row.response_hash != receipt_hash(call, &row.response, &row.usage)
    {
        return Err("specialist_response_receipt_invalid".into());
    }
    let text = row.response["text"]
        .as_str()
        .ok_or("specialist_response_text_invalid")?
        .to_string();
    let rejection = row.response["rejection"]
        .as_str()
        .ok_or("specialist_response_rejection_invalid")?
        .to_string();
    if call.child.role == crate::agent_runtime::contract::AgentRole::ClientSide {
        let raw = row.response["rawTextHash"]
            .as_str()
            .ok_or("client_side_raw_hash_missing")?;
        if raw.len() != 64
            || !raw.bytes().all(|b| b.is_ascii_hexdigit())
            || (!rejection.is_empty() && !text.is_empty())
        {
            return Err("client_side_semantic_receipt_invalid".into());
        }
        if rejection.is_empty() {
            let task = super::super::client_side::assignment(connection, &call.lease, &call.child)?;
            super::super::client_side::validate_assessment(&text, &task)?;
        }
    }
    let usage = read_usage(&row.usage)?;
    let event: Option<String> = connection.query_row(
        "SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2 AND event_type='model_round_completed'",
        params![call.child.run_id,row.sequence], |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    if event
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        != Some(event_payload(connection, call, &usage)?)
    {
        return Err("specialist_response_event_invalid".into());
    }
    super::super::directive::source_guidance::verify_delivery(
        connection,
        &call.lease,
        &call.child,
        false,
        row.sequence,
    )?;
    let snapshot = store::read_snapshot(connection, &call.child.run_id)?
        .ok_or("specialist_response_checkpoint_missing")?;
    let state = RunState::from_json(&snapshot.snapshot);
    if snapshot.last_sequence < row.sequence
        || state.run_id != call.child.run_id
        || state.turns != 1
        || state.model_requests != usage.model_requests
        || state.target_requests
            != crate::agent_runtime::target_requests::child_received(
                connection,
                &call.child.run_id,
            )?
        || state.input_tokens != usage.input_tokens
        || state.cached_input_tokens != usage.cached_input_tokens
        || state.output_tokens != usage.output_tokens
        || state.used_tokens != usage.total_tokens
        || state.progress_signature != store::stable_hash(&text)
    {
        return Err("specialist_response_checkpoint_invalid".into());
    }
    Ok(StoredResponse {
        text,
        usage,
        rejection,
    })
}

pub(super) fn verify_received(
    db: &Connection,
    call: &PendingCall,
    row: &CallRow,
) -> Result<StoredResponse, String> {
    let saved = verify_response(db, call, row)?;
    super::super::budget::receipts::verify(
        db,
        &call.lease,
        &call.child.assignment_id,
        &format!(
            "specialist:{}:{}",
            call.child.assignment_id, call.request_hash
        ),
        &row.response_hash,
        &saved.usage,
    )?;
    Ok(saved)
}

/// The receipt, usage event and checkpoint commit together before downstream work.
#[cfg(test)]
pub fn record_received(
    connection: &Connection,
    call: &PendingCall,
    text: &str,
    tool_calls_present: bool,
    usage: &UsageDelta,
) -> Result<StoredResponse, String> {
    record_received_with_provenance(connection, call, text, tool_calls_present, usage, true)
}

pub fn record_received_with_provenance(
    connection: &Connection,
    call: &PendingCall,
    text: &str,
    tool_calls_present: bool,
    usage: &UsageDelta,
    usage_reported: bool,
) -> Result<StoredResponse, String> {
    let usage = read_usage(&usage.as_json())?;
    let too_large = text.len() > 1_048_576;
    let raw_hash = store::artifact_id(text.as_bytes());
    let text = if too_large {
        String::new()
    } else {
        if super::super::homogeneous::material_policy::for_root(connection, &call.lease.root_run_id)? {
            text.to_string()
        } else { redact_text_with(text, None) }
    };
    let rejection = if !usage_reported {
        "model_usage_requires_reconciliation"
    } else if too_large {
        "response_too_large"
    } else if tool_calls_present {
        "unexpected_tool_calls"
    } else if text.trim().is_empty() {
        "empty_response"
    } else {
        ""
    };
    let (text, rejection) =
        if call.child.role == crate::agent_runtime::contract::AgentRole::ClientSide {
            if !rejection.is_empty() {
                (String::new(), rejection.to_string())
            } else {
                // A damaged business task rejects semantics, not the already
                // incurred original invoice. Financial binding is verified by
                // the existing private receipt/failure writer under its lock.
                let valid = super::super::client_side::assignment(connection, &call.lease, &call.child)
                    .and_then(|task| super::super::client_side::validate_assessment(&text, &task));
                match valid {
                    Ok(()) => (text, String::new()),
                    Err(code) => (String::new(), code),
                }
            }
        } else {
            (text, rejection.to_string())
        };
    let mut response = json!({"text":text,"rejection":rejection,"usageReported":usage_reported});
    if call.child.role == crate::agent_runtime::contract::AgentRole::ClientSide {
        // Retain the original wire hash and usage, not an invalid semantic body.
        response["rawTextHash"] = json!(raw_hash);
    }
    // Roll back business publication before retaining this original incurred bill.
    let mut cost_saved = false;
    match private_writer::run(connection, |db| record_response(db, call, &response, &usage, usage_reported, &mut cost_saved)) {
        Ok(saved) => Ok(saved),
        Err(primary) if cost_saved => Err(primary),
        Err(primary) => {
            let hash = receipt_hash(call, &response, &usage.as_json());
            match failed_cost::record(connection, call, &hash, &usage, usage_reported) {
                Ok(()) => Err(primary),
                Err(persist) => Err(format!("{primary};specialist_cost_after_publication_failure:{persist}")),
            }
        }
    }
}

fn record_response(
    connection: &Connection,
    call: &PendingCall,
    response: &Value,
    usage: &UsageDelta,
    usage_reported: bool,
    cost_saved: &mut bool,
) -> Result<StoredResponse, String> {
    let text = response["text"].as_str().ok_or("specialist_response_text_invalid")?;
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if let Err(error) =
        super::super::attempts::require_unexpired_response_worker(&tx, &call.lease, &call.child)
    {
        super::super::budget::model_facts::specialist(
            &tx,
            &call.lease,
            &call.child,
            &call.request_hash,
            super::super::budget::model_facts::Fact::Received {
                hash: &receipt_hash(call, response, &usage.as_json()),
                usage,
                reported: usage_reported,
            },
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        *cost_saved = true;
        return Err(error);
    }
    binding(&tx, &call.lease, &call.child)?;
    let row = load(&tx, call)?.ok_or("specialist_dispatch_missing")?;
    if row.state == "received" {
        if &row.response != response || row.usage != usage.as_json() {
            return Err("specialist_response_replay_conflict".into());
        }
        return verify_received(&tx, call, &row);
    }
    if row.state != "executing" || !row.failure_code.is_empty() {
        return Err("specialist_response_state_conflict".into());
    }
    crate::collaboration_events::web_creation_schema::verify_model_delivery(&tx)?;
    let sequence = store::append_event(
        &tx,
        &call.child.run_id,
        AgentEventKind::ModelRoundCompleted,
        &event_payload(&tx, call, usage)?,
        &[],
    )?;
    let mut state = RunState::new(&call.child.run_id, Vec::new());
    state.turns = 1;
    state.model_requests = usage.model_requests;
    state.target_requests =
        crate::agent_runtime::target_requests::child_received(&tx, &call.child.run_id)?;
    state.input_tokens = usage.input_tokens;
    state.cached_input_tokens = usage.cached_input_tokens;
    state.output_tokens = usage.output_tokens;
    state.used_tokens = usage.total_tokens;
    state.progress_signature = store::stable_hash(text);
    checkpoint::write_checkpoint(&tx, &state)?;
    let changed = tx.execute(
        "UPDATE agent_specialist_calls SET state='received',response_json=?1,usage_json=?2,response_hash=?3,event_sequence=?4,finished_at=datetime('now','localtime') \
         WHERE assignment_id=?5 AND child_run_id=?6 AND state='executing' AND failure_code=''",
        params![response.to_string(),usage.as_json().to_string(),receipt_hash(call,response,&usage.as_json()),sequence,call.child.assignment_id,call.child.run_id],
    ).map_err(|e|format!("specialist_response_persist:{e}"))?;
    if changed != 1 {
        return Err("specialist_response_persist_missing".into());
    }
    super::super::directive::source_guidance::record_delivery(
        &tx,
        &call.lease,
        &call.child,
        false,
        sequence,
    )?;
    let saved = verify_response(
        &tx,
        call,
        &load(&tx, call)?.ok_or("specialist_response_missing")?,
    )?;
    let source = format!(
        "specialist:{}:{}",
        call.child.assignment_id, call.request_hash
    );
    let hash = receipt_hash(call, response, &usage.as_json());
    if usage_reported {
        super::super::budget::receipts::record(
            &tx,
            &call.lease,
            &call.child.assignment_id,
            &source,
            &hash,
            usage,
            None,
        )?;
    } else {
        super::super::budget::receipts::record_estimated(
            &tx,
            &call.lease,
            &call.child.assignment_id,
            &source,
            &hash,
            usage,
            None,
        )?;
    }
    verify_received(
        &tx,
        call,
        &load(&tx, call)?.ok_or("specialist_response_missing")?,
    )?;
    tx.commit()
        .map_err(|e| format!("specialist_response_commit:{e}"))?;
    Ok(saved)
}
