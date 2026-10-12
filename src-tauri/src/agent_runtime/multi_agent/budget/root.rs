//! Root control accounting; no child assignment or Coordinator is fabricated.
use super::{entries, Kind, DENIED_BY_CONTRACT_DIMENSIONS, DIMENSIONS};
use crate::agent_runtime::multi_agent::lease::{self, CoordinatorLease};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

mod clock;
mod load;
pub(crate) mod model;
mod original;
mod shared;
mod single_exit;
pub(crate) mod target;
mod transaction;

pub(crate) use shared::{require_child_capacity,remaining_child_capacity};

#[derive(Clone)]
pub(crate) struct RootOwner {
    pub(super) root: String,
    pub(super) id: String,
    contract: Value,
    coordinator: Option<CoordinatorLease>,
}

fn contract(db: &Connection, root: &str) -> Result<Value, String> {
    let mut value = db.query_row("SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,created_at,
        hard_token_budget,hard_request_budget,orchestration_policy,root_run_id FROM agent_runs
        WHERE id=?1 AND backend='native' AND role='coordinator' AND assignment_id=''
          AND parent_run_id IS NULL AND (root_run_id=id OR (root_run_id='' AND orchestration_policy='single'))
          AND orchestration_policy IN ('single','multi')", [root], |r|Ok(json!({
        "scan":r.get::<_,String>(0)?,"attempt":r.get::<_,i64>(1)?,"target":r.get::<_,String>(2)?,
        "planHash":r.get::<_,String>(3)?,"plan":r.get::<_,String>(4)?,"origin":r.get::<_,String>(5)?,
        "tokens":r.get::<_,i64>(6)?,"requests":r.get::<_,i64>(7)?,"policy":r.get::<_,String>(8)?,
        "declaredRoot":r.get::<_,String>(9)?,
    }))).map_err(|_|"budget_root_binding_conflict".to_string())?;
    if let Some(mode) = crate::agent_runtime::web_mode::root::read(db, root)? {
        if let Some(protocol) = mode.new_evidence_supervision() { value["newEvidenceSupervision"] = protocol; }
        if let Some(local) = mode.local_deliberation() { value["localDeliberation"] = local; }
        if let Some(bootstrap) = mode.bootstrap_dispatch() { value["bootstrapDispatch"] = bootstrap; }
        if let Some(observation)=mode.live_budget_observation() {value["liveBudgetObservation"]=observation;}
    }
    Ok(value)
}

impl RootOwner {
    pub(crate) fn initialize(tx: &Transaction<'_>, root: &str) -> Result<Self, String> {
        let owner=Self::load_original(tx,root)?;
        owner.require_live(tx)?;
        Ok(owner)
    }
    pub(crate) fn initialize_new_source_root(fresh:&crate::agent_runtime::store::NewlyInsertedSourceRoot<'_, '_>)->Result<Self,String> {
        fresh.verify()?;let tx=fresh.transaction();let root=fresh.id();
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_root_mode_definitions WHERE root_run_id=?1)",[root],|r|r.get(0)).map_err(|e|e.to_string())?;
        if exists {return Err("source_finance_birth_proof_reused".into());}
        let actor=tx.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
            FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(CoordinatorLease{scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|_|"source_finance_first_coordinator_missing")?;
        if actor.lease_epoch!=1 || uuid::Uuid::parse_str(&actor.fencing_token).is_err() {
            return Err("source_finance_first_coordinator_conflict".into());
        }
        lease::validate_coordinator_lease(tx,&actor)?;
        let owner=Self::initialize_on(tx,root,true)?;
        owner.require_original_coordinator(tx,&actor)?;fresh.verify()?;
        Ok(owner)
    }
    // Deliberate lower-level financial fixture issuer, excluded from production.
    // Live SDK/control/claim code never calls this or contains a test fallback.
    #[cfg(test)]
    pub(crate) fn initialize_financial_fixture_for_test(tx:&Transaction<'_>,root:&str)->Result<Self,String> {
        transaction::protect(tx, || Self::initialize_on(tx,root,true))
    }
    pub(crate) fn initialize_new_native_root(fresh:&crate::agent_runtime::store::NewlyInsertedNativeRoot<'_, '_>)->Result<Self,String> {
        crate::agent_runtime::web_mode::root::read(fresh.transaction(),fresh.id())?
            .ok_or("web_finance_private_mode_missing")?;
        let existed:bool=fresh.transaction().query_row("SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)",
            [fresh.id()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if existed {return Err("web_finance_birth_proof_reused".into());}
        Self::initialize_on(fresh.transaction(),fresh.id(),true)
    }
    fn initialize_on(tx:&Transaction<'_>,root:&str,born_mode_root:bool)->Result<Self,String> {
        let frozen = contract(tx, root)?;
        let coordinator = if frozen["policy"] == "multi" {
            Some(tx.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
                FROM agent_coordinator_leases WHERE root_run_id=?1", [root], |r|Ok(CoordinatorLease {
                scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
            })).map_err(|_|"budget_root_coordinator_missing")?)
        } else {
            None
        };
        let saved: Option<(String, String)> = tx
            .query_row(
                "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mode_root:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_mode_definitions WHERE root_run_id=?1)",
            [root],|r|r.get(0)).map_err(|e|e.to_string())?;
        if saved.is_none() && mode_root && !born_mode_root {return Err("web_root_original_finance_missing".into());}
        let evidence = json!({"root":frozen,"coordinator":coordinator.as_ref().map(|c|json!({
            "scan":c.scan_id,"attempt":c.attempt_number,"target":c.target_key,"epoch":c.lease_epoch,"fence":c.fencing_token
        }))});
        let evidence_text = evidence.to_string();
        let owner = Self {
            root: root.into(),
            id: saved
                .as_ref()
                .map(|s| s.0.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            contract: evidence,
            coordinator,
        };
        // Live checks deliberately precede initialization. Missing history is
        // rejected, never upgraded into permission or reconstructed charges.
        owner.require_live_run(tx)?;
        if let Some((_, text)) = saved {
            if text != evidence_text {
                return Err("budget_root_original_owner_conflict".into());
            }
        } else {
            let fresh: bool = tx
                .query_row(
                    "SELECT used_tokens=0 AND used_cached_tokens=0 AND used_requests=0
                AND reserved_tokens=0 AND reserved_requests=0 FROM agent_runs WHERE id=?1",
                    [root],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let historical:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')
                OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
                OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
                OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='')
                OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)
                OR EXISTS(SELECT 1 FROM sentinel_checkpoints p JOIN agent_runs r
                  ON r.scan_id=p.scan_id AND r.target_url=p.url WHERE r.id=?1
                  AND r.orchestration_policy='single' AND p.stage='native_agent_state'
                  AND (NOT json_valid(p.raw_json) OR json_extract(p.raw_json,'$.attemptNumber') IS NULL
                    OR json_extract(p.raw_json,'$.attemptNumber')=r.attempt_number))", [root], |r|r.get(0)).map_err(|e|e.to_string())?;
            if !fresh || historical {
                return Err("budget_history_requires_reconciliation".into());
            }
            let changed=tx.execute("INSERT INTO agent_root_budget_attempts(id,root_run_id,contract_json) VALUES(?1,?2,?3)",params![owner.id,root,evidence_text]).map_err(|e|e.to_string())?;
            if changed != 1 {
                return Err("budget_root_owner_persistence_conflict".into());
            }
        }
        super::limits::initialize_root(tx, root)?;
        owner.verify(tx)?;
        owner.require_live_run(tx)?;
        super::clock::remaining(tx, root)?;
        Ok(owner)
    }

    pub(crate) fn verify(&self, db: &Connection) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.id).is_err()
            || contract(db, &self.root)? != self.contract["root"]
        {
            return Err("budget_root_original_owner_conflict".into());
        }
        let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE id=?1 AND root_run_id=?2 AND contract_json=?3)",
            params![self.id,self.root,self.contract.to_string()], |r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("budget_root_original_owner_conflict".into());
        }
        Ok(())
    }

    fn require_live_run(&self, db: &Connection) -> Result<(), String> {
        let scope = &self.contract["root"];
        lease::require_active_attempt(
            db,
            scope["scan"].as_str().ok_or("budget_root_scope_invalid")?,
            scope["attempt"]
                .as_i64()
                .ok_or("budget_root_scope_invalid")?,
        )?;
        lease::require_open_coordinator(
            db,
            scope["scan"].as_str().ok_or("budget_root_scope_invalid")?,
            scope["attempt"]
                .as_i64()
                .ok_or("budget_root_scope_invalid")?,
            scope["target"]
                .as_str()
                .ok_or("budget_root_scope_invalid")?,
            &self.root,
        )?;
        if let Some(actor) = &self.coordinator {
            lease::validate_coordinator_lease(db, actor)?;
        }
        Ok(())
    }

    pub(crate) fn require_live(&self, db: &Connection) -> Result<(), String> {
        self.require_executable(db)?;
        super::admission::require_determinate(db, &self.root)
    }

    pub(crate) fn require_executable(&self, db: &Connection) -> Result<(), String> {
        self.verify(db)?;
        single_exit::require_no_exit(db, &self.root)?;
        let closed: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_multi_exit_receipts WHERE root_run_id=?1)",
            [&self.root], |r| r.get(0)).map_err(|e| e.to_string())?;
        if closed { return Err("budget_multi_continuation_requires_explicit_contract".into()); }
        self.require_live_run(db)?;
        super::clock::remaining(db, &self.root).map(|_| ())
    }

    pub(super) fn append(
        &self,
        tx: &Transaction<'_>,
        dimension: &str,
        kind: Kind,
        amount: i64,
        key: &str,
        source: &str,
    ) -> Result<(), String> {
        self.verify(tx)?;
        if !DIMENSIONS.contains(&dimension)
            || amount <= 0
            || key.trim().is_empty()
            || source.trim().is_empty()
        {
            return Err("budget_entry_invalid".into());
        }
        if DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
            return Err("budget_dimension_denied_by_contract".into());
        }
        entries::persist(
            tx,
            &self.root,
            "",
            self.id.clone(),
            dimension,
            kind,
            amount,
            key,
            source,
        )?;
        self.verify(tx)
    }

    pub(super) fn append_known_cost(
        &self,
        tx: &Transaction<'_>,
        dimension: &str,
        amount: i64,
        key: &str,
        source: &str,
    ) -> Result<(), String> {
        self.verify(tx)?;
        if amount <= 0 || key.trim().is_empty() || source.trim().is_empty() {
            return Err("budget_entry_invalid".into());
        }
        if DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
            return Err("budget_dimension_denied_by_contract".into());
        }
        entries::persist_known_cost(
            tx,
            &self.root,
            "",
            self.id.clone(),
            dimension,
            Kind::Reserve,
            amount,
            key,
            source,
        )?;
        self.verify(tx)
    }
}

pub(super) fn stored_key(
    db: &Connection,
    root: &str,
    id: &str,
    key: &str,
) -> Result<String, String> {
    let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE id=?1 AND root_run_id=?2)",params![id,root],|r|r.get(0)).map_err(|e|e.to_string())?;
    let prefix = format!("root:{id}:");
    if !bound
        || uuid::Uuid::parse_str(id).is_err()
        || key.trim().is_empty()
        || key.starts_with("worker:")
        || key.starts_with("root:") && !key.starts_with(&prefix)
    {
        return Err("budget_root_attempt_scope_conflict".into());
    }
    Ok(if key.starts_with(&prefix) {
        key.into()
    } else {
        format!("{prefix}{key}")
    })
}
