//! Durable withdrawal audit. It grants no SDK, replacement worker or refund.
use super::*;
use serde::{Deserialize, Serialize};
mod sources;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fact {
    version: u8,
    receipt_rowid: i64,
    meaning: String,
    receipt: String,
    worker: String,
    root: String,
    assignment: String,
    child: String,
    scan: String,
    attempt: i64,
    target: String,
    withdrawal_epoch: i64,
    withdrawal_fence: String,
    cutoff: String,
    before_worker: String,
    before_caps: String,
    after_worker: String,
    after_caps: String,
    context: String,
    event_floor: i64,
    event_ceiling: i64,
    events: String,
}
pub(super) struct Witness {
    actor: CoordinatorLease,
    before_worker: String,
    before_caps: String,
    context: String,
    event_floor: i64,
}
impl Witness {
    pub(super) fn capture(
        db: &Connection,
        actor: &CoordinatorLease,
        worker: &AssignmentAttempt,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("worker_expiry_receipt_snapshot_required".into());
        }
        sources::verify_schema(db)?;
        Ok(Self {
            actor: actor.clone(),
            before_worker: sources::worker(db, &worker.id)?,
            before_caps: sources::caps(db, &worker.assignment_id)?,
            context: sources::context(db, &worker.root_run_id, &worker.assignment_id)?,
            event_floor: db
                .query_row(
                    "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?,
        })
    }
    // Called only by the opaque proof after the protected transition succeeded.
    pub(super) fn persist(
        &self,
        db: &Connection,
        worker: &AssignmentAttempt,
        proof: &super::new_withdrawal::NewlyWithdrawnWorker<'_>,
        issuer: Option<&crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>,
    ) -> Result<(), String> {
        sources::verify_schema(db)?;
        if worker.state != "expired"
            || worker.finished_at.is_empty()
            || worker.failure_class != "worker_lease_expired"
            || self.actor.root_run_id != worker.root_run_id
            || sources::context(db, &worker.root_run_id, &worker.assignment_id)? != self.context
        {
            return Err("worker_expiry_receipt_original_source_changed".into());
        }
        let mut fact = Fact {
            version: 1,
            receipt_rowid: db
                .query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM agent_worker_expiry_receipts",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?
                .checked_add(1)
                .filter(|n| *n > 0)
                .ok_or("worker_expiry_receipt_rowid_exhausted")?,
            meaning: "withdrawn_only".into(),
            receipt: uuid::Uuid::new_v4().to_string(),
            worker: worker.id.clone(),
            root: worker.root_run_id.clone(),
            assignment: worker.assignment_id.clone(),
            child: worker.child_run_id.clone(),
            scan: self.actor.scan_id.clone(),
            attempt: self.actor.attempt_number,
            target: self.actor.target_key.clone(),
            withdrawal_epoch: self.actor.lease_epoch,
            withdrawal_fence: self.actor.fencing_token.clone(),
            cutoff: worker.finished_at.clone(),
            before_worker: self.before_worker.clone(),
            before_caps: self.before_caps.clone(),
            after_worker: sources::worker(db, &worker.id)?,
            after_caps: sources::caps(db, &worker.assignment_id)?,
            context: self.context.clone(),
            event_floor: self.event_floor,
            event_ceiling: db
                .query_row(
                    "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?,
            events: String::new(),
        };
        fact.events = sources::events(db, &fact)?;
        let raw = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
        let hash = crate::agent_runtime::store::stable_hash(&raw);
        let n=db.execute("INSERT INTO agent_worker_expiry_receipts(receipt_id,worker_attempt_id,root_run_id,assignment_id,child_run_id,fact_json,fact_hash,rowid,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![fact.receipt,fact.worker,fact.root,fact.assignment,fact.child,raw,hash,fact.receipt_rowid,fact.cutoff]).map_err(|e|e.to_string())?;
        if n != 1 {
            return Err("worker_expiry_receipt_write_unconfirmed".into());
        }
        verify_original(db, &worker.id)?;
        if let Some(issuer) = issuer {
            let fresh = FreshWithdrawalFact {
                proof,
                raw: &raw,
                actor: &self.actor,
            };
            issuer.issue_new_on(db, &fresh)?;
            issuer.verify_issued(db, &worker.id)?;
        }
        Ok(())
    }
}

// Pure snapshot verification. Original C expiry/heartbeat and future legal
// saved-result scalar changes do not rewrite the frozen withdrawal fact.
pub(crate) fn verify_original(db: &Connection, worker: &str) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("worker_expiry_receipt_snapshot_required".into());
    }
    sources::verify_schema(db)?;
    let (id,root,assignment,child,raw,hash):(String,String,String,String,String,String)=db.query_row(
        "SELECT receipt_id,root_run_id,assignment_id,child_run_id,fact_json,fact_hash FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",[worker],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_|"worker_expiry_original_receipt_missing")?;
    let fact: Fact =
        serde_json::from_str(&raw).map_err(|_| "worker_expiry_receipt_fact_invalid")?;
    let physical: bool = db.query_row("SELECT rowid=?2 AND created_at=?3 FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1", params![worker,fact.receipt_rowid,fact.cutoff], |r|r.get(0)).map_err(|e|e.to_string())?;
    let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id JOIN agent_runs r ON r.id=x.root_run_id JOIN agent_runs c ON c.id=x.child_run_id AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_url=r.target_url AND c.root_run_id=r.id AND c.parent_run_id=r.id AND c.assignment_id=a.id AND c.role=a.role AND c.lane=a.lane AND c.backend='native' WHERE x.id=?1 AND x.root_run_id=?2 AND x.assignment_id=?3 AND x.child_run_id=?4 AND x.state='expired' AND x.failure_class='worker_lease_expired' AND x.finished_at=?5 AND datetime(x.expires_at)<=datetime(x.finished_at) AND datetime(x.finished_at)<=datetime('now','localtime') AND a.coordinator_run_id=x.root_run_id AND a.target_key=?6 AND r.scan_id=?7 AND r.attempt_number=?8 AND r.target_url=?6 AND r.root_run_id=r.id AND r.backend='native' AND r.role='coordinator' AND r.orchestration_policy='multi')",params![worker,root,assignment,child,fact.cutoff,fact.target,fact.scan,fact.attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !physical
        || fact.receipt_rowid <= 0
        || !bound
        || fact.version != 1
        || fact.meaning != "withdrawn_only"
        || fact.worker != worker
        || fact.receipt != id
        || fact.root != root
        || fact.assignment != assignment
        || fact.child != child
        || fact.event_floor > fact.event_ceiling
        || fact.withdrawal_epoch <= 0
        || fact.cutoff.is_empty()
        || uuid::Uuid::parse_str(&id).is_err()
        || uuid::Uuid::parse_str(&fact.withdrawal_fence).is_err()
        || serde_json::to_string(&fact).map_err(|e| e.to_string())? != raw
        || crate::agent_runtime::store::stable_hash(&raw) != hash
        || sources::worker(db, worker)? != fact.after_worker
        || sources::caps(db, &assignment)? != fact.after_caps
        || sources::context(db, &root, &assignment)? != fact.context
        || sources::events(db, &fact)? != fact.events
    {
        return Err("worker_expiry_receipt_original_fact_conflict".into());
    }
    Ok(())
}

// Only the actual protected first transition can construct this envelope.
pub(crate) struct FreshWithdrawalFact<'a, 'w> {
    proof: &'a super::new_withdrawal::NewlyWithdrawnWorker<'w>,
    raw: &'a str,
    actor: &'a CoordinatorLease,
}
impl FreshWithdrawalFact<'_, '_> {
    pub(crate) fn require<'a>(
        &'a self,
        db: &Connection,
    ) -> Result<(&'a CoordinatorLease, &'a str), String> {
        let fact: Fact =
            serde_json::from_str(self.raw).map_err(|_| "worker_expiry_receipt_fact_invalid")?;
        self.proof.require_fact_identity(
            db,
            (
                &fact.worker,
                &fact.root,
                &fact.assignment,
                &fact.child,
                &fact.cutoff,
            ),
            self.actor,
        )?;
        if fact.scan != self.actor.scan_id
            || fact.attempt != self.actor.attempt_number
            || fact.target != self.actor.target_key
        {
            return Err("worker_new_withdrawal_fact_identity_changed".into());
        }
        verify_original(db, &fact.worker)?;
        Ok((self.actor, self.raw))
    }
}
