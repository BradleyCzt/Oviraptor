//! Exact Root-scoped withdrawal proof. Never reads asset tables or CAS.
use super::*;
use rusqlite::types::Value;
use serde_json::{json, Value as JsonValue};

use super::audit_rows::Rows;

// These facts may not change merely because a worker's execution grant ends.
// Root-scoped accounting and journals include siblings, so collateral release
// or a new dispatch inside a trigger also fails the final comparison.
pub(crate) const FROZEN: &[&str] = &[
    "SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT rowid,* FROM agent_worker_expiry_receipts WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_coordinator_leases WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_budget_ledger WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_budget_limits WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_root_budget_definitions WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_budget_clock_origins WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_contract_owners WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_lane_leases WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE coordinator_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_source_model_rounds WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_review_requests WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_review_decisions WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_finding_candidates WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_evidence_nodes WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_evidence_edges WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_evidence_revisions WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM sentinel_findings WHERE scan_id=(SELECT scan_id FROM agent_runs WHERE id=?1) AND target_url=(SELECT target_url FROM agent_runs WHERE id=?1) ORDER BY rowid",
    "SELECT * FROM sentinel_checkpoints WHERE scan_id=(SELECT scan_id FROM agent_runs WHERE id=?1) AND url=(SELECT target_url FROM agent_runs WHERE id=?1) ORDER BY rowid",
    "SELECT * FROM agent_messages WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_source_review_decisions WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_source_coverage_decisions WHERE root_run_id=?1 ORDER BY rowid",
    "SELECT * FROM agent_source_tool_receipts WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE coordinator_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_events WHERE run_id IN (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_snapshots WHERE run_id IN (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM tool_invocations WHERE run_id IN (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_http_request_claims WHERE run_id IN (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_authorization_probe_claims WHERE child_run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM agent_external_surface_captures WHERE child_run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1) ORDER BY rowid",
    "SELECT * FROM sentinel_scans WHERE id=(SELECT scan_id FROM agent_runs WHERE id=?1) ORDER BY rowid",
    "SELECT * FROM sentinel_deleted_scans WHERE scan_id=(SELECT scan_id FROM agent_runs WHERE id=?1) ORDER BY rowid",
];

pub(super) struct ExpiryProof {
    mutable: [Rows; 4],
    frozen: Vec<Rows>,
    old_events: Rows,
    event_floor: i64,
    new_events: Vec<(String, String, JsonValue)>,
}

impl ExpiryProof {
    pub(super) fn capture(
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
        worker: &AssignmentAttempt,
    ) -> Result<Self, String> {
        let mutable = [
            Rows::read(db, "SELECT * FROM agent_assignment_attempts WHERE root_run_id=?1 OR id=?2 ORDER BY rowid", params![actor.root_run_id,worker.id])?,
            Rows::read(db, "SELECT * FROM agent_assignments WHERE coordinator_run_id=?1 OR id=?2 ORDER BY rowid", params![actor.root_run_id,child.assignment_id])?,
            Rows::read(db, "SELECT * FROM agent_runs WHERE id=?1 OR root_run_id=?1 OR id=?2 ORDER BY rowid", params![actor.root_run_id,child.run_id])?,
            Rows::read(db, "SELECT * FROM agent_capability_leases WHERE root_run_id=?1 OR child_run_id=?2 OR assignment_id=?3 ORDER BY rowid", params![actor.root_run_id,child.run_id,child.assignment_id])?,
        ];
        let frozen = FROZEN
            .iter()
            .map(|sql| Rows::read(db, sql, [&actor.root_run_id]))
            .collect::<Result<_, _>>()?;
        let event_floor = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let old_events = Self::old_events(db, actor, event_floor)?;
        Ok(Self {
            mutable,
            frozen,
            old_events,
            event_floor,
            new_events: Vec::new(),
        })
    }

    pub(super) fn expect_withdrawal(
        &mut self,
        child: &ScheduledChild,
        worker: &AssignmentAttempt,
        now: &str,
    ) -> Result<(), String> {
        let role = child.role.as_str();
        if self.mutable[1].text(&child.assignment_id, "state")? != "paused"
            || self.mutable[1].text(&child.assignment_id, "failure_class")?
                != "worker_lease_expired"
        {
            self.new_events.push((
                "assignment".into(),
                child.assignment_id.clone(),
                json!({"role":role,"state":"paused"}),
            ));
        }
        if self.mutable[2].text(&child.run_id, "status")? != "paused" {
            self.new_events.push((
                "agent_run".into(),
                child.run_id.clone(),
                json!({"role":role,"status":"paused","terminalState":""}),
            ));
        }
        for (column, value) in [
            ("state", "expired"),
            ("finished_at", now),
            ("failure_class", "worker_lease_expired"),
        ] {
            self.mutable[0].set(&worker.id, column, value)?;
        }
        for (column, value) in [
            ("state", "paused"),
            ("failure_class", "worker_lease_expired"),
            ("updated_at", now),
        ] {
            self.mutable[1].set(&child.assignment_id, column, value)?;
        }
        for (column, value) in [("status", "paused"), ("updated_at", now)] {
            self.mutable[2].set(&child.run_id, column, value)?;
        }
        let grants = &mut self.mutable[3];
        let names = [
            "root_run_id",
            "assignment_id",
            "child_run_id",
            "lease_epoch",
            "fencing_token",
            "revoked_at",
        ];
        let columns = names
            .iter()
            .map(|n| grants.column(n))
            .collect::<Result<Vec<_>, _>>()?;
        for row in &mut grants.values {
            let expected = [
                Value::Text(worker.root_run_id.clone()),
                Value::Text(child.assignment_id.clone()),
                Value::Text(child.run_id.clone()),
                Value::Integer(worker.coordinator_epoch),
                Value::Text(worker.coordinator_fencing_token.clone()),
                Value::Text(String::new()),
            ];
            if columns.iter().zip(&expected).all(|(i, v)| &row[*i] == v) {
                row[columns[5]] = Value::Text(now.into());
            }
        }
        Ok(())
    }

    fn old_events(db: &Connection, actor: &CoordinatorLease, floor: i64) -> Result<Rows, String> {
        Rows::read(db,"SELECT * FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND sequence<=?3 ORDER BY sequence",params![actor.scan_id,actor.attempt_number,floor])
    }

    pub(super) fn verify(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
        worker: &AssignmentAttempt,
    ) -> Result<(), String> {
        let actual = Self::capture(db, actor, child, worker)?;
        if actual.mutable != self.mutable
            || actual.frozen != self.frozen
            || Self::old_events(db, actor, self.event_floor)? != self.old_events
        {
            return Err("assignment_expiry_original_proof_changed".into());
        }
        let mut query = db.prepare("SELECT entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND sequence>?3 ORDER BY sequence").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(
                params![actor.scan_id, actor.attempt_number, self.event_floor],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if rows.len() != self.new_events.len()
            || rows.iter().zip(&self.new_events).any(
                |((kind, id, event, payload), (expected_kind, expected_id, expected_payload))| {
                    kind != expected_kind
                        || event != expected_kind
                        || id != expected_id
                        || serde_json::from_str::<JsonValue>(payload).ok().as_ref()
                            != Some(expected_payload)
                },
            )
        {
            return Err("assignment_expiry_event_postcondition".into());
        }
        super::super::lease::validate_coordinator_lease(db, actor)?;
        super::super::lease::require_executable_coordinator(db, actor)
    }
}
