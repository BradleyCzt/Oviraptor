//! ClientSide analyzes frozen local observations. No target/browser/tool grant.
use super::{lease::CoordinatorLease, scheduler::ScheduledChild};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Task {
    pub(crate) schema_version: u8,
    pub(crate) phase: String,
    pub(crate) root_run_id: String,
    pub(crate) scan_id: String,
    pub(crate) attempt_number: i64,
    pub(crate) target: String,
    pub(crate) coordinator_epoch: i64,
    pub(crate) coordinator_fence: String,
    pub(crate) evidence_revision: i64,
    pub(crate) target_requests_granted: u8,
    pub(crate) browser_actions_granted: u8,
    pub(crate) tools_granted: Vec<String>,
    pub(crate) observations: Vec<Observation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Observation {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) classification: String,
    pub(crate) artifact: Artifact,
    pub(crate) value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_proof: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Artifact {
    pub(crate) path: String,
    pub(crate) content_hash: String,
}
fn bounded(s: &str, n: usize) -> bool {
    !s.is_empty() && s.len() <= n && !s.chars().any(|c| c.is_control())
}
fn artifact_path(path: &str) -> bool {
    let leaf = path.strip_prefix("agent-http/").unwrap_or(path);
    !leaf.is_empty()
        && !matches!(leaf, "." | "..")
        && leaf.len() <= 100
        && leaf
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        && (leaf == path || leaf.ends_with(".json") || leaf.ends_with(".body"))
}
impl Task {
    pub(crate) fn from_value(
        value: &Value,
        lease: &CoordinatorLease,
        revision: i64,
    ) -> Result<Self, String> {
        let task: Self =
            serde_json::from_value(value.clone()).map_err(|_| "client_side_task_invalid")?;
        if task.schema_version != 1
            || task.phase != "client_side_readonly"
            || task.root_run_id != lease.root_run_id
            || task.scan_id != lease.scan_id
            || task.attempt_number != lease.attempt_number
            || task.target != lease.target_key
            || task.coordinator_epoch != lease.lease_epoch
            || task.coordinator_fence != lease.fencing_token
            || task.evidence_revision != revision
            || revision != 1
            || task.target_requests_granted != 0
            || task.browser_actions_granted != 0
            || !task.tools_granted.is_empty()
            || task.observations.is_empty()
            || task.observations.len() > 4
        {
            return Err("client_side_task_scope_invalid".into());
        }
        let mut ids = BTreeSet::new();
        for o in &task.observations {
            if !bounded(&o.id, 128)
                || !ids.insert(o.id.clone())
                || !matches!(
                    o.kind.as_str(),
                    "meta_csp" | "csp_header" | "http_csp_absent"
                )
                || o.classification != "source-derived"
                || !artifact_path(&o.artifact.path)
                || o.artifact.content_hash.len() != 64
                || !o
                    .artifact
                    .content_hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                || !bounded(&o.value, 1024)
                || o.source_proof
                    .as_ref()
                    .is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
                || (o.artifact.path.starts_with("agent-http/") && o.source_proof.is_none())
            {
                return Err("client_side_observation_invalid".into());
            }
        }
        Ok(task)
    }
}
pub(crate) fn validate_scheduled(
    value: &Value,
    lease: &CoordinatorLease,
    revision: i64,
    caps: &[String],
) -> Result<(), String> {
    if caps != ["evidence.read", "mailbox.write"] {
        return Err("client_side_capability_invalid".into());
    }
    Task::from_value(value, lease, revision).map(|_| ())
}
pub(crate) fn assignment(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Task, String> {
    let (text,revision):(String,i64)=db.query_row(
        "SELECT task_slice_json,evidence_revision FROM agent_assignments WHERE id=?1 AND child_run_id=?2
          AND coordinator_run_id=?3 AND role='client_side' AND lane='read_only_analysis' AND target_key=?4
          AND lease_epoch=?5 AND fencing_token=?6",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.target_key,lease.lease_epoch,lease.fencing_token],
        |r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"client_side_assignment_invalid")?;
    let value: Value = serde_json::from_str(&text).map_err(|_| "client_side_task_invalid")?;
    Task::from_value(&value, lease, revision)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Assessment {
    summary: String,
    observation_refs: Vec<String>,
    gaps: Vec<String>,
    candidates: Vec<Value>,
}
pub(crate) fn validate_assessment(text: &str, task: &Task) -> Result<(), String> {
    if text.len() > 4096 {
        return Err("client_side_response_oversized".into());
    }
    let a: Assessment = serde_json::from_str(text).map_err(|_| "client_side_response_invalid")?;
    let ids: BTreeSet<_> = task.observations.iter().map(|o| o.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    if !bounded(&a.summary, 1024)
        || a.observation_refs.is_empty()
        || a.observation_refs.len() > 4
        || a.observation_refs
            .iter()
            .any(|s| !ids.contains(s.as_str()) || !seen.insert(s.as_str()))
        || !a.candidates.is_empty()
        || a.gaps.is_empty()
        || a.gaps.len() > 8
        || !a.gaps.iter().any(|g| g == "missing_browser_validation")
        || a.gaps.iter().any(|g| !bounded(g, 128))
    {
        return Err("client_side_readonly_assessment_insufficient".into());
    }
    // This first role slice deliberately publishes configuration facts only.
    // An impact/candidate/Reviewer stage needs its own measured behavior proof.
    Ok(())
}

// Caller holds the actual scheduling IMMEDIATE lock, before and after reserve.
/// EvidenceReviewer retained capacity shared by Mapper/Executor/proposal/ordered/surface gates.
pub(crate) const REVIEWER_FLOOR_TOKENS: i64 = 15_000;
pub(crate) const REVIEWER_FLOOR_REQUESTS: i64 = 1;

pub(crate) fn require_reviewer_floor(
    db: &Connection,
    lease: &CoordinatorLease,
    new_tokens: i64,
    new_requests: i64,
) -> Result<(), String> {
    use super::budget::{balance, root::RootOwner};
    RootOwner::load_original(db, &lease.root_run_id)?.require_original_coordinator(db, lease)?;
    if new_tokens < 0 || new_requests < 0 {
        return Err("client_side_budget_invalid".into());
    }
    use rusqlite::OptionalExtension;
    let ledger: Option<(i64,i64,i64,i64,i64,i64)> = db.query_row(
        "SELECT total_tokens,spent_tokens,reserved_tokens,total_requests,spent_requests,reserved_requests FROM agent_budget_ledger
         WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3",params![lease.root_run_id,lease.lease_epoch,lease.fencing_token],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .optional().map_err(|_|"client_side_budget_ledger_missing")?;
    let (tokens,spent,reserved,requests,used,held) = match ledger {
        Some(row) => row,
        None => first_child_capacity(db, lease, new_tokens, new_requests)?,
    };
    // Same EvidenceReviewer floor as Mapper/Executor retained capacity.
    let tf = new_tokens
        .checked_add(REVIEWER_FLOOR_TOKENS)
        .ok_or("client_side_budget_invalid")?;
    let rf = new_requests
        .checked_add(REVIEWER_FLOOR_REQUESTS)
        .ok_or("client_side_budget_invalid")?;
    if (tokens > 0
        && !tokens
            .checked_sub(spent)
            .and_then(|n| n.checked_sub(reserved))
            .is_some_and(|n| n >= tf))
        || (requests > 0
            && !requests
                .checked_sub(used)
                .and_then(|n| n.checked_sub(held))
                .is_some_and(|n| n >= rf))
    {
        return Err("client_side_budget_headroom_missing_reviewer_floor_preserved".into());
    }
    for (dimension, needed) in [
        ("model_input_tokens", tf),
        ("model_output_tokens", tf),
        ("model_requests", rf),
    ] {
        let b = balance(db, &lease.root_run_id, None, dimension)?;
        let hard: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![lease.root_run_id, dimension],
                |r| r.get(0),
            )
            .map_err(|_| "client_side_budget_limits_missing")?;
        if b.indeterminate != 0
            || hard.is_some_and(|h| {
                !h.checked_sub(b.reserved)
                    .and_then(|n| n.checked_sub(b.consumed))
                    .is_some_and(|n| n >= needed)
            })
        {
            return Err("client_side_budget_headroom_missing_reviewer_floor_preserved".into());
        }
    }
    Ok(())
}

// The first child has no coarse projection yet: its existing scheduler creates
// that projection atomically with the reservation. Read original born capacity
// only, then require the real ledger after reserve; never repair missing history.
fn first_child_capacity(
    db: &Connection, lease: &CoordinatorLease, tokens: i64, requests: i64,
) -> Result<(i64,i64,i64,i64,i64,i64), String> {
    if tokens <= 0 || requests != 1
        || crate::agent_runtime::web_mode::root::read(db, &lease.root_run_id)?.is_none()
    {
        return Err("client_side_budget_ledger_missing".into());
    }
    let history: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='legacy_role_pipeline')
        OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_runs child JOIN agent_runs root ON root.id=?1
          WHERE child.scan_id=root.scan_id AND child.attempt_number=root.attempt_number
            AND child.target_url=root.target_url AND child.id<>root.id)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM sentinel_checkpoints p JOIN agent_runs r
          ON p.scan_id=r.scan_id AND p.url=r.target_url WHERE r.id=?1
            AND p.stage='native_agent_state')",
        [&lease.root_run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if history { return Err("client_side_budget_ledger_missing_history".into()); }
    db.query_row("SELECT hard_token_budget,0,0,hard_request_budget,0,0 FROM agent_runs
        WHERE id=?1 AND role='coordinator' AND backend='native' AND status IN ('prepared','running')
        AND finished_at='' AND used_tokens=0 AND used_cached_tokens=0
        AND used_requests=0 AND reserved_tokens=0 AND reserved_requests=0",
        [&lease.root_run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .map_err(|_| "client_side_budget_ledger_missing".into())
}
