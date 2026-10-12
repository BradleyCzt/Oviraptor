//! A private first-transition proof, never a read-time reconstruction.
use super::super::supervisor::expiry_writer::ExpiryWriteAuthority;
use super::*;

pub(crate) struct NewlyWithdrawnWorker<'writer> {
    authority: &'writer ExpiryWriteAuthority,
    original_scope: CoordinatorLease,
    worker_after: AssignmentAttempt,
    witness: super::withdrawal_receipt::Witness,
}
impl NewlyWithdrawnWorker<'_> {
    pub(crate) fn require_fact_identity(
        &self,
        db: &Connection,
        identity: (&str, &str, &str, &str, &str),
        actor: &CoordinatorLease,
    ) -> Result<(), String> {
        self.require_issuance(db)?;
        let expected = (
            &*self.worker_after.id,
            &*self.worker_after.root_run_id,
            &*self.worker_after.assignment_id,
            &*self.worker_after.child_run_id,
            &*self.worker_after.finished_at,
        );
        if identity != expected
            || actor.root_run_id != self.original_scope.root_run_id
            || actor.scan_id != self.original_scope.scan_id
            || actor.attempt_number != self.original_scope.attempt_number
            || actor.target_key != self.original_scope.target_key
        {
            return Err("worker_new_withdrawal_fact_identity_changed".into());
        }
        Ok(())
    }
    pub(crate) fn require_issuance(&self, db: &Connection) -> Result<(), String> {
        self.verify_current(db, self.authority)
    }
    pub(in crate::agent_runtime::multi_agent) fn persist_receipt(
        &self,
        db: &Connection,
        authority: &ExpiryWriteAuthority,
        issuer: Option<&crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>,
    ) -> Result<(), String> {
        self.verify_current(db, authority)?;
        self.witness.persist(db, &self.worker_after, self, issuer)?;
        self.verify_current(db, authority)
    }
    pub(in crate::agent_runtime::multi_agent) fn verify_current(
        &self,
        db: &Connection,
        authority: &ExpiryWriteAuthority,
    ) -> Result<(), String> {
        if !std::ptr::eq(self.authority, authority) {
            return Err("worker_new_withdrawal_writer_changed".into());
        }
        authority.require_snapshot(db)?;
        let current = current(db, &self.original_scope, &self.worker_after.assignment_id)?;
        if current != self.worker_after
            || current.state != "expired"
            || current.failure_class != "worker_lease_expired"
            || current.finished_at.is_empty()
        {
            return Err("worker_new_withdrawal_proof_changed".into());
        }
        Ok(())
    }
}

// This function requires the supervisor's unforgeable, connection-bound writer
// token. Generic cleanup keeps its original bool API and cannot get this proof.
pub(in crate::agent_runtime::multi_agent) fn withdraw_new_for_supervision<'writer>(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    authority: &'writer ExpiryWriteAuthority,
) -> Result<Option<NewlyWithdrawnWorker<'writer>>, String> {
    authority.require_snapshot(tx)?;
    let mut original = actor.clone();
    (original.lease_epoch, original.fencing_token) = tx.query_row(
        "SELECT lease_epoch,fencing_token FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3",
        params![child.assignment_id,child.run_id,actor.root_run_id], |r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e|format!("worker_new_withdrawal_binding:{e}"))?;
    let before = current(tx, &original, &child.assignment_id)?;
    if !matches!(before.state.as_str(), "leased" | "running" | "paused") {
        // In particular, no proof for old expired rows, even if their audit
        // labels now happen to match a modern source contract.
        return Ok(None);
    }
    let witness = super::withdrawal_receipt::Witness::capture(tx, actor, &before)?;
    if !super::try_expire_original_in_transaction(tx, actor, child)? {
        return Ok(None);
    }
    let proof = NewlyWithdrawnWorker {
        authority,
        witness,
        original_scope: original.clone(),
        worker_after: current(tx, &original, &child.assignment_id)?,
    };
    proof.verify_current(tx, authority)?;
    Ok(Some(proof))
}
