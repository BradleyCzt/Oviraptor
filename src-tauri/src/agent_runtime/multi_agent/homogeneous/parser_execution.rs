//! One original paid response owns one scoped, frozen, offline execution.
//! An interrupted invocation is retained for reconciliation, never replayed.
use super::{execution_policy, model_worker, store};
use crate::{
    agent_runtime::{
        multi_agent::{attempts, lease, scheduler::ScheduledChild, specialist},
        store::stable_hash,
        web_mode::{withdrawal_issuer::WithdrawalIssuer, ModeFact, VerifiedMode},
    },
    tool_supply::{
        approved_bundle, container_parser, execution_readiness, ToolAdministratorConfiguration,
    },
};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
const DOMAIN: &str = "oviraptor.original-homogeneous-parser.v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolRequest {
    pub name: String,
    pub approval_id: String,
    pub program: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaidPlan {
    output: Value,
    evidence_refs: Vec<String>,
    terminal_hint: String,
    tool_request: ToolRequest,
}

pub(crate) struct FreshParserFact<'a> {
    db: &'a Connection,
    actor: &'a lease::CoordinatorLease,
    child: &'a ScheduledChild,
    fact: Value,
}
impl FreshParserFact<'_> {
    pub(crate) fn encoded_for_original(&self, mode: &ModeFact) -> Result<Vec<u8>, String> {
        require_worker(self.db, self.actor, self.child)?;
        if self.fact["domain"] != DOMAIN
            || self.fact["rootRunId"] != self.actor.root_run_id
            || mode.scan_id != self.actor.scan_id
            || mode.attempt_number != self.actor.attempt_number
        {
            return Err("homogeneous_parser_original_binding_changed".into());
        }
        serde_json::to_vec(&self.fact).map_err(|e| e.to_string())
    }
}
fn issuer(db: &Connection, actor: &lease::CoordinatorLease) -> Result<WithdrawalIssuer, String> {
    let mode: VerifiedMode = crate::commands::original_root_capture_mode(db, &actor.root_run_id)?;
    WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, mode)
}

fn require_worker(
    db: &Connection,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    if child.role != crate::agent_runtime::contract::AgentRole::DeepInvestigator {
        return Err("homogeneous_parser_readonly_carrier_required".into());
    }
    execution_policy::require_on(db, actor)?;
    attempts::require_live_for_run(db, &child.run_id)?;
    let allowed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id JOIN agent_task_slices s ON s.slice_id=a.slice_id JOIN agent_homogeneous_tasks t ON t.task_id=s.parent_task_id JOIN agent_lane_slots slot ON slot.assignment_id=a.id AND slot.root_run_id=t.root_run_id AND slot.released_at='' JOIN agent_capability_leases cap ON cap.assignment_id=a.id AND cap.child_run_id=r.id AND cap.capability='source_analysis' WHERE r.id=?1 AND a.id=?2 AND t.root_run_id=?3 AND r.root_run_id=t.root_run_id AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?6 AND r.status='running' AND r.cancel_requested_at='' AND a.state='running' AND a.scheduling_model='homogeneous_workers' AND a.worker_profile_id='homogeneous_v1' AND a.lane='read_only_analysis' AND r.lane=a.lane AND s.lane=a.lane AND slot.lane=a.lane AND a.lease_epoch=?7 AND a.fencing_token=?8 AND cap.root_run_id=t.root_run_id AND cap.lease_epoch=a.lease_epoch AND cap.fencing_token=a.fencing_token AND cap.revoked_at='' AND cap.lease_expires_at>datetime('now','localtime'))",params![child.run_id,child.assignment_id,actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !allowed {
        return Err("homogeneous_parser_worker_capability_denied".into());
    }
    crate::agent_runtime::multi_agent::budget::clock::remaining(db, &actor.root_run_id)?;
    Ok(())
}

fn request_input(
    db: &Connection,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
    requested: &ToolRequest,
) -> Result<Value, String> {
    if requested.name != "source_analysis"
        || requested.program.is_empty()
        || requested.program.len() > 16 * 1024
        || requested.evidence_refs.is_empty()
        || requested.evidence_refs.len() > 16
    {
        return Err("homogeneous_parser_request_invalid".into());
    }
    let assignment = store::load_assignment(db, &child.assignment_id)
        .map_err(|e| e.detail)?
        .ok_or("homogeneous_parser_assignment_missing")?;
    let task = store::load_task_by_root_run(db, &actor.root_run_id)
        .map_err(|e| e.detail)?
        .ok_or("homogeneous_parser_task_missing")?;
    let slice = store::load_slice(db, &assignment.slice_id)
        .map_err(|e| e.detail)?
        .ok_or("homogeneous_parser_slice_missing")?;
    let input = model_worker::input(db, &task, &slice)?;
    let mut selected = Vec::new();
    for id in &requested.evidence_refs {
        if selected.iter().any(|v: &Value| v["evidence_ref"] == *id) {
            return Err("homogeneous_parser_duplicate_input".into());
        }
        let value = input["evidence"]
            .as_array()
            .and_then(|rows| rows.iter().find(|v| v["evidence_ref"] == *id))
            .ok_or("homogeneous_parser_input_scope_escape")?;
        selected.push(value.clone());
    }
    Ok(
        json!({"program":requested.program,"input":{"goal":input["goal"],"slice_id":slice.slice_id,"scope_kind":slice.scope_kind,"scope_value":slice.scope_value,"evidence":selected}}),
    )
}

struct FrozenCall {
    id: String,
    fact: Value,
    request: container_parser::Request,
    original_request: ToolRequest,
}
fn freeze(
    tx: &Transaction<'_>,
    cfg: &ToolAdministratorConfiguration,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
    requested: &ToolRequest,
) -> Result<FrozenCall, String> {
    require_worker(tx, actor, child)?;
    let paid = specialist::received_for_local_delivery(tx, actor, child)?;
    if !paid.rejection.is_empty() {
        return Err("homogeneous_parser_original_model_receipt_rejected".into());
    }
    let parsed: PaidPlan = serde_json::from_str(&paid.text)
        .map_err(|_| "homogeneous_parser_original_model_output_invalid")?;
    if !super::machine::output_shape_valid(&parsed.output)
        || parsed.evidence_refs.len() > 16
        || !super::types::is_member(&parsed.terminal_hint, &["completed", "partial", "failed"])
        || !requested
            .evidence_refs
            .iter()
            .all(|id| parsed.evidence_refs.contains(id))
    {
        return Err("homogeneous_parser_original_model_output_invalid".into());
    }
    let original = parsed.tool_request;
    if &original != requested {
        return Err("homogeneous_parser_original_model_request_changed".into());
    }
    let attempt: i64 = tx
        .query_row(
            "SELECT attempt FROM agent_assignments WHERE id=?1 AND child_run_id=?2",
            params![child.assignment_id, child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let existing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_homogeneous_parser_calls WHERE assignment_id=?1 AND assignment_attempt=?2)",params![child.assignment_id,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if existing {
        return Err("homogeneous_parser_original_invocation_reconciliation_required".into());
    }
    let ready = execution_readiness::require_current_on(
        tx,
        cfg,
        &actor.root_run_id,
        &requested.approval_id,
    )?;
    let runtime = ready
        .container()
        .ok_or("homogeneous_parser_runtime_unsupported")?;
    let bundle = approved_bundle::open_on(tx, cfg, &requested.approval_id)?;
    let manifest = ready.original_approval().manifest();
    if bundle.metadata().entries().len() != 1
        || !manifest
            .broker_capabilities
            .iter()
            .any(|c| c == "source_analysis")
    {
        return Err("homogeneous_parser_dependencies_or_capability_unsupported".into());
    }
    let entry = &bundle.metadata().entries()[0];
    let start = usize::try_from(entry.content_offset())
        .map_err(|_| "homogeneous_parser_package_size_invalid")?;
    let size = usize::try_from(entry.content_bytes())
        .map_err(|_| "homogeneous_parser_package_size_invalid")?;
    let end = start
        .checked_add(size)
        .ok_or("homogeneous_parser_package_size_invalid")?;
    let code = std::str::from_utf8(
        bundle
            .original_bytes()
            .get(start..end)
            .ok_or("homogeneous_parser_package_size_invalid")?,
    )
    .map_err(|_| "homogeneous_parser_entrypoint_not_python")?
    .to_string();
    let request = container_parser::Request {
        code,
        input: request_input(tx, actor, child, requested)?,
        runtime_digest: runtime.digest().into(),
        limits: container_parser::Limits {
            memory_mib: manifest.memory_mib,
            timeout_seconds: manifest.timeout_seconds,
            output_bytes: manifest.output_bytes,
        },
    };
    let (sdk_request_hash,sdk_response_hash):(String,String)=tx.query_row("SELECT request_hash,response_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",params![child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let fact = json!({"domain":DOMAIN,"kind":"request","invocationId":id,"rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"assignmentId":child.assignment_id,"childRunId":child.run_id,"assignmentAttempt":attempt,"leaseEpoch":actor.lease_epoch,"fencingToken":actor.fencing_token,"sdkRequestHash":sdk_request_hash,"sdkResponseHash":sdk_response_hash,"approvalId":requested.approval_id,"approvalHash":ready.original_approval().envelope_hash(),"packageDigest":manifest.digest,"requested":requested,"runtimeRequest":request});
    let fresh = FreshParserFact {
        db: tx,
        actor,
        child,
        fact: fact.clone(),
    };
    let signature = issuer(tx, actor)?.sign_fresh_homogeneous_parser_fact(&fresh)?;
    let text = fact.to_string();
    tx.execute("INSERT INTO agent_homogeneous_parser_calls(invocation_id,root_run_id,assignment_id,child_run_id,assignment_attempt,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,actor.root_run_id,child.assignment_id,child.run_id,attempt,text,stable_hash(&text),signature]).map_err(|e|e.to_string())?;
    require_worker(tx, actor, child)?;
    Ok(FrozenCall {
        id,
        fact,
        request,
        original_request: requested.clone(),
    })
}

fn verify_frozen(
    db: &Connection,
    cfg: &ToolAdministratorConfiguration,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
    call: &FrozenCall,
) -> Result<(), String> {
    let tx =
        Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|e| e.to_string())?;
    verify_frozen_on(&tx, cfg, actor, child, call)?;
    tx.commit().map_err(|e| e.to_string())
}

fn verify_frozen_on(
    tx: &Transaction<'_>,
    cfg: &ToolAdministratorConfiguration,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
    call: &FrozenCall,
) -> Result<(), String> {
    let db: &Connection = tx;
    require_worker(db, actor, child)?;
    let paid = specialist::received_for_local_delivery(db, actor, child)?;
    if !paid.rejection.is_empty() {
        return Err("homogeneous_parser_original_model_receipt_rejected".into());
    }
    let original: PaidPlan = serde_json::from_str(&paid.text)
        .map_err(|_| "homogeneous_parser_original_model_output_invalid")?;
    if original.tool_request != call.original_request {
        return Err("homogeneous_parser_original_model_request_changed".into());
    }
    let sdk:(String,String)=db.query_row("SELECT request_hash,response_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",params![child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if call.fact["sdkRequestHash"] != sdk.0 || call.fact["sdkResponseHash"] != sdk.1 {
        return Err("homogeneous_parser_original_sdk_receipt_changed".into());
    }
    let (text,hash,signature):(String,String,Vec<u8>)=db.query_row("SELECT fact_json,fact_hash,signature FROM agent_homogeneous_parser_calls WHERE invocation_id=?1 AND root_run_id=?2 AND assignment_id=?3 AND child_run_id=?4",params![call.id,actor.root_run_id,child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if text != call.fact || hash != stable_hash(&text) {
        return Err("homogeneous_parser_frozen_request_changed".into());
    }
    issuer(db, actor)?.verify_homogeneous_parser_fact(text.as_bytes(), &signature)?;
    let ready = execution_readiness::require_current_on(
        tx,
        cfg,
        &actor.root_run_id,
        &call.original_request.approval_id,
    )?;
    if ready.original_approval().envelope_hash()
        != call.fact["approvalHash"]
            .as_str()
            .ok_or("homogeneous_parser_approval_changed")?
    {
        return Err("homogeneous_parser_approval_changed".into());
    }
    if request_input(tx, actor, child, &call.original_request)? != call.request.input {
        return Err("homogeneous_parser_input_changed".into());
    }
    let bundle = approved_bundle::open_on(tx, cfg, &call.original_request.approval_id)?;
    if bundle.manifest().digest != call.fact["packageDigest"] {
        return Err("homogeneous_parser_package_changed".into());
    }
    Ok(())
}

pub(crate) fn execute(
    db: &Connection,
    cfg: &ToolAdministratorConfiguration,
    actor: &lease::CoordinatorLease,
    child: &ScheduledChild,
    request: &ToolRequest,
    cancelled: &dyn Fn() -> bool,
) -> Result<Value, String> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let call = freeze(&tx, cfg, actor, child, request)?;
    tx.commit().map_err(|e| e.to_string())?;
    verify_frozen(db, cfg, actor, child, &call)?;
    let check = || cancelled() || verify_frozen(db, cfg, actor, child, &call).is_err();
    #[cfg(not(test))]
    let receipt = container_parser::execute_guarded(&call.request, &check)?;
    // The test harness executable has its own main. Exercise the same real
    // fixed runtime here; cross-process guardian/SIGKILL proof uses the actual
    // built App binary in tools/test_nest_sandbox_guardian.py.
    #[cfg(test)]
    let receipt = container_parser::Runtime::inspect()?.execute(&call.request, &check)?;
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    require_worker(&tx, actor, child)?;
    verify_frozen_on(&tx, cfg, actor, child, &call)?;
    let fact = json!({"domain":DOMAIN,"kind":"result","invocationId":call.id,"rootRunId":actor.root_run_id,"requestFactHash":stable_hash(&call.fact.to_string()),"receipt":receipt,"analysisOnly":true,"vulnerabilityConfirmed":false});
    let fresh = FreshParserFact {
        db: &tx,
        actor,
        child,
        fact: fact.clone(),
    };
    let signature = issuer(&tx, actor)?.sign_fresh_homogeneous_parser_fact(&fresh)?;
    let text = fact.to_string();
    tx.execute("INSERT INTO agent_homogeneous_parser_results(invocation_id,root_run_id,assignment_id,child_run_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![call.id,actor.root_run_id,child.assignment_id,child.run_id,text,stable_hash(&text),signature]).map_err(|e|e.to_string())?;
    require_worker(&tx, actor, child)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(fact)
}
