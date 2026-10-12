//! Typed original Root target cost, bound to the existing immutable HTTP claim.
use super::RootOwner;
use crate::agent_runtime::multi_agent::budget::{scope, Kind};
use rusqlite::{params, types::Value, Connection, Transaction};
pub(crate) mod lifetime;

#[derive(Clone)]
pub(crate) struct RootTargetRequest {
    pub scan: String,
    pub attempt: i64,
    pub target: String,
    pub budget_attempt: i64,
    pub ordinal: i64,
    pub invocation: String,
    pub request_index: i64,
    pub request_hash: String,
}

#[derive(Clone)]
pub(crate) struct RootTargetCall {
    owner: RootOwner,
    request: RootTargetRequest,
    source: String,
    original: Vec<Value>,
}

impl std::fmt::Debug for RootTargetCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootTargetCall")
            .field("root", &self.owner.root)
            .field("attempt", &self.owner.id)
            .field("source", &self.source)
            .finish()
    }
}

impl RootTargetCall {
    pub(crate) fn claim(
        tx: &Transaction<'_>,
        owner: RootOwner,
        request: RootTargetRequest,
        controlled_write: bool,
        upload_bytes: usize,
    ) -> Result<
        (
            Self,
            crate::agent_runtime::execution_owner::NativeInvocationOwner,
        ),
        String,
    > {
        crate::agent_runtime::target_protection::require_clear_on(tx, &request.scan, &request.target)?;
        super::super::continuation::migration::require_plain_root_boundary(tx, &owner.root)?;
        if owner.contract["root"]["policy"] != "single" || owner.coordinator.is_some() {
            return Err("budget_root_original_owner_conflict".into());
        }
        owner.require_live(tx)?;
        // Current frozen contracts grant zero write/upload budget. A human
        // approval row alone cannot manufacture a grant. These dimensions need
        // their own canonical effect/byte receipts before they may be enabled.
        if controlled_write || upload_bytes > 0 {
            return Err("budget_operation_not_granted".into());
        }
        if request.request_hash.len() != 64
            || !request.request_hash.bytes().all(|b| b.is_ascii_hexdigit())
            || request.attempt < 1
            || request.budget_attempt < 1
            || request.ordinal < 1
            || request.request_index < 1
            || request.invocation.is_empty()
        {
            return Err("budget_target_claim_invalid".into());
        }
        super::clock::sample(tx, &owner)?;
        let source = format!(
            "http:{}:{}:{}:{}",
            owner.root, request.invocation, request.request_index, request.request_hash
        );
        let original = http_proof(tx, &owner.root, &request, None)?;
        let call = Self {
            owner,
            request,
            source,
            original,
        };
        for (kind, suffix) in [(Kind::Reserve, "reserve"), (Kind::Forfeit, "dispatch")] {
            call.owner.append(
                tx,
                "target_requests",
                kind,
                1,
                &format!("target:{}:{suffix}", call.source),
                &call.source,
            )?;
        }
        call.verify_dispatch(tx)?;
        call.owner.require_executable(tx)?;
        crate::agent_runtime::target_protection::require_clear_on(tx, &call.request.scan, &call.request.target)?;
        let path = tx
            .path()
            .filter(|p| !p.is_empty())
            .ok_or("single_target_lifetime_database_missing")?;
        let guard = crate::agent_runtime::execution_owner::claim_native_invocation(
            std::path::Path::new(path),
            &call.request.scan,
            call.request.attempt,
            lifetime::INVOCATION_KIND,
            &call.source,
        )?;
        Ok((call, guard))
    }

    fn verify_dispatch(&self, db: &Connection) -> Result<(), String> {
        self.owner.verify(db)?;
        let exact: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1
             AND assignment_id='' AND lease_attempt_id=?2 AND dimension='target_requests'
             AND amount=1 AND source_id=?3 AND ((kind='reserve' AND idempotency_key=?4)
             OR (kind='forfeit' AND idempotency_key=?5))",
                params![
                    self.owner.root,
                    self.owner.id,
                    self.source,
                    scope::stored_key(
                        db,
                        &self.owner.root,
                        "",
                        &self.owner.id,
                        &format!("target:{}:reserve", self.source)
                    )?,
                    scope::stored_key(
                        db,
                        &self.owner.root,
                        "",
                        &self.owner.id,
                        &format!("target:{}:dispatch", self.source)
                    )?
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exact != 2 {
            return Err("budget_target_claim_missing".into());
        }
        Ok(())
    }

    pub(crate) fn require_executable(&self, db: &Connection) -> Result<(), String> {
        self.verify_dispatch(db)?;
        if http_proof(db, &self.owner.root, &self.request, None)? != self.original {
            return Err("budget_target_http_binding_conflict".into());
        }
        // Own dispatched target debt is expected while this request is in flight.
        // Fresh work uses RootOwner::require_live and still rejects that debt.
        self.owner.require_executable(db)
    }

    pub(crate) fn transport_timeout(
        &self,
        db: &Connection,
        ceiling: std::time::Duration,
    ) -> Result<std::time::Duration, String> {
        self.require_executable(db)?;
        Ok(super::super::clock::remaining(db, &self.owner.root)?.min(ceiling))
    }

    pub(crate) fn receive(&self, tx: &Transaction<'_>, status: u16) -> Result<(), String> {
        if !(100..=599).contains(&status) {
            return Err("budget_target_receipt_invalid".into());
        }
        self.verify_dispatch(tx)?;
        if http_proof(tx, &self.owner.root, &self.request, Some(status))? != self.original {
            return Err("budget_target_receipt_invalid".into());
        }
        let key = format!("target:{}:receipt", self.source);
        let prior: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND idempotency_key=?2)",
            params![self.owner.root, scope::stored_key(tx, &self.owner.root, "", &self.owner.id, &key)?],
            |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if prior {
            return Err("budget_target_receipt_replay_denied".into());
        }
        self.owner.append(
            tx,
            "target_requests",
            Kind::Reconcile,
            1,
            &key,
            &self.source,
        )?;
        self.verify_dispatch(tx)?;
        if http_proof(tx, &self.owner.root, &self.request, Some(status))? != self.original {
            return Err("budget_target_receipt_invalid".into());
        }
        // Saving known original cost never authorizes output or future work.
        Ok(())
    }
}

fn http_proof(
    db: &Connection,
    root: &str,
    request: &RootTargetRequest,
    status: Option<u16>,
) -> Result<Vec<Value>, String> {
    let mut statement = db.prepare(
        "SELECT c.scan_id,c.target_url,c.budget_attempt,c.ordinal,c.attempt_number,
         c.run_id,c.invocation_id,c.request_index,c.tool_name,c.identity_handle,c.request_hash,c.created_at
         FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
         JOIN tool_invocations i ON i.run_id=c.run_id AND i.invocation_id=c.invocation_id
           AND i.tool_name=c.tool_name AND i.policy_decision='allow'
         WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_url=?3 AND c.budget_attempt=?4
           AND c.ordinal=?5 AND c.run_id=?6 AND c.invocation_id=?7 AND c.request_index=?8 AND c.request_hash=?9
           AND r.scan_id=c.scan_id AND r.attempt_number=c.attempt_number AND r.target_url=c.target_url
           AND ((?10=0 AND c.response_status=0 AND c.received_at='' AND i.status='running')
             OR (?10>0 AND c.response_status=?10 AND c.received_at<>''))",
    ).map_err(|e| e.to_string())?;
    let columns = statement.column_count();
    statement
        .query_row(
            params![
                request.scan,
                request.attempt,
                request.target,
                request.budget_attempt,
                request.ordinal,
                root,
                request.invocation,
                request.request_index,
                request.request_hash,
                status.unwrap_or(0)
            ],
            |row| (0..columns).map(|i| row.get(i)).collect(),
        )
        .map_err(|_| "budget_target_http_binding_conflict".into())
}
