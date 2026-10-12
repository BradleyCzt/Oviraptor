//! One original Root provider call, claimed atomically before any transport.
use super::RootOwner;
use crate::agent_runtime::{
    model::gateway::ModelResponse,
    multi_agent::budget::{self, Kind, DIMENSIONS},
};
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};
use std::time::Duration;

mod receipt;
pub(crate) mod cache_provenance;
pub(crate) mod single_lifetime;
pub(crate) mod single_history;
pub(crate) mod tick;

#[derive(Clone)]
pub(crate) struct RootModelCall {
    owner: RootOwner,
    id: String,
    round: i64,
    request: String,
    tokens: i64,
    pub(crate) remaining: Duration,
}

enum LocalHeadroom<'a> {
    Generic,
    VerifiedOneShot(&'a tick::authorization_admission::VerifiedOneShotAssessment),
}

impl RootModelCall {
    pub(crate) fn initialize_coordinator_control(tx:&Transaction<'_>,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<(),String> {
        super::transaction::protect(tx,|| {
            let owner=RootOwner::load_original(tx,&actor.root_run_id)?;
            owner.require_original_coordinator(tx,actor)?;
            owner.require_executable(tx)
        })
    }

    #[cfg(test)]
    pub(crate) fn claim(
        tx: &Transaction<'_>,
        root: &str,
        round: i64,
        request: &str,
        estimate: i64,
    ) -> Result<Self, String> {
        super::transaction::protect(tx, || Self::claim_on(tx, root, round, request, estimate))
    }

    pub(crate) fn claim_single_transport(
        tx: &Transaction<'_>, root: &str, round: i64, request: &str, estimate: i64,
    ) -> Result<(Self, crate::agent_runtime::execution_owner::NativeInvocationOwner), String> {
        super::transaction::protect(tx, || {
            RootOwner::load_single(tx,root)?;
            let (call,guard)=Self::claim_inner(tx,root,round,request,estimate,true,LocalHeadroom::Generic)?;
            Ok((call,guard.ok_or("single_model_transport_owner_missing")?))
        })
    }

    fn claim_on(
        tx: &Transaction<'_>, root: &str, round: i64, request: &str, estimate: i64,
    ) -> Result<Self, String> {
        Self::claim_inner(tx,root,round,request,estimate,false,LocalHeadroom::Generic).map(|(call,_)|call)
    }

    fn claim_one_shot_on(
        tx: &Transaction<'_>, root: &str, round: i64, request: &str, estimate: i64,
        proof: &tick::authorization_admission::VerifiedOneShotAssessment,
    ) -> Result<Self, String> {
        Self::claim_inner(tx,root,round,request,estimate,false,LocalHeadroom::VerifiedOneShot(proof)).map(|(call,_)|call)
    }

    fn claim_inner(
        tx: &Transaction<'_>,
        root: &str,
        round: i64,
        request: &str,
        estimate: i64,
        transport: bool,
        local_headroom: LocalHeadroom<'_>,
    ) -> Result<(Self, Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>), String> {
        if let LocalHeadroom::VerifiedOneShot(proof) = &local_headroom {
            proof.require_on(tx, root, round, request)?;
        }
        if round < 1
            || request.len() != 64
            || !request.bytes().all(|b| b.is_ascii_hexdigit())
            || estimate <= 0
        {
            return Err("budget_root_call_invalid".into());
        }
        let owner = RootOwner::initialize(tx, root)?;
        if owner.contract["root"]["policy"] == "single" {
            RootOwner::load_single(tx, root)?;
        }
        owner.require_live(tx)?;
        let history:(i64,i64,bool)=tx.query_row("SELECT
            (SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'),
            (SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'),
            EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1 AND round=?2)",params![root,round],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if history != (round - 1, round - 1, false) {
            return Err("budget_history_requires_reconciliation".into());
        }
        super::clock::sample(tx, &owner)?;
        let (available_tokens, available_requests) = super::shared::available(tx, root)?;
        if available_tokens.is_some_and(|available| available < estimate) {
            return Err("budget_hard_limit_exceeded".into());
        }
        let tokens = estimate;
        if available_requests.is_some_and(|available| available < 1) {
            return Err("budget_model_requests_exhausted".into());
        }
        let call = Self {
            id: crate::agent_runtime::store::stable_hash(
                &json!({"owner":owner.id,"root":root,"round":round,"request":request}).to_string(),
            ),
            remaining: budget::clock::remaining(tx, root)?,
            owner,
            round,
            request: request.into(),
            tokens,
        };
        // Only the actual Single SDK path acquires local invocation ownership.
        // Existing history must use its original inode; it cannot mint exit proof.
        let guard=if transport {
            match single_lifetime::require_idle_original(tx,&call.owner)? {
                Some(guard)=>Some(guard),
                None=>Some(crate::agent_runtime::execution_owner::claim_native_invocation(
                    std::path::Path::new(tx.path().filter(|p|!p.is_empty()).ok_or("single_model_lifetime_database_missing")?),
                    call.owner.contract["root"]["scan"].as_str().ok_or("budget_root_scope_invalid")?,
                    call.owner.contract["root"]["attempt"].as_i64().ok_or("budget_root_scope_invalid")?,
                    single_lifetime::INVOCATION_KIND,&call.owner.root)?),
            }
        } else {None};
        for (dimension, amount) in DIMENSIONS[..4].iter().zip([tokens, tokens, tokens, 1]) {
            call.owner.append(
                tx,
                dimension,
                Kind::Reserve,
                amount,
                &format!("root-model:{}:{dimension}:reserve", call.id),
                &call.id,
            )?;
        }
        call.insert(tx, "dispatch", &call.dispatch())?;
        call.require_executable(tx)?;
        if matches!(local_headroom, LocalHeadroom::Generic)
            && call.owner.contract["root"].get("localDeliberation").is_some() {
            super::require_child_capacity(tx, root, 15_000, 1)?;
        } else { super::require_child_capacity(tx, root, 0, 0)?; }
        Ok((call,guard))
    }

    fn dispatch(&self) -> Value {
        json!({"reservedTokens":self.tokens,"reservedRequests":1})
    }

    fn verify(&self, db: &Connection, phase: &str, fact: &Value) -> Result<(), String> {
        self.owner.verify(db)?;
        let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3
            AND round=?4 AND request_hash=?5 AND phase=?6 AND receipt_json=?7)",params![self.id,self.owner.root,self.owner.id,self.round,self.request,phase,fact.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("budget_root_call_binding_conflict".into());
        }
        Ok(())
    }

    fn insert(&self, tx: &Transaction<'_>, phase: &str, fact: &Value) -> Result<(), String> {
        let inserted=tx.execute("INSERT INTO agent_root_model_journal(call_id,root_run_id,lease_attempt_id,round,request_hash,phase,receipt_json)
            VALUES(?1,?2,?3,?4,?5,?6,?7)",params![self.id,self.owner.root,self.owner.id,self.round,self.request,phase,fact.to_string()]).map_err(|e|e.to_string())?;
        if inserted != 1 {
            return Err("budget_root_call_persistence_conflict".into());
        }
        self.verify(tx, phase, fact)
    }

    pub(crate) fn require_executable(&self, db: &Connection) -> Result<(), String> {
        self.verify(db, "dispatch", &self.dispatch())?;
        self.owner.require_executable(db)
    }

    pub(crate) fn require_next_work(&self, db: &Connection) -> Result<(), String> {
        self.require_executable(db)?;
        self.owner.require_live(db)
    }

    pub(crate) fn terminal(
        &self,
        tx: &Transaction<'_>,
        phase: &str,
        response: Option<&ModelResponse>,
        code: &str,
    ) -> Result<bool, String> {
        super::transaction::protect(tx, || self.terminal_on(tx, phase, response, code))
    }

    pub(super) fn received_fact(response: &ModelResponse) -> Value {
        json!({"responseHash":crate::agent_runtime::store::stable_hash(&json!({"text":response.text,
                "tools":response.tool_calls.iter().map(|c|json!({"id":c.id,"name":c.name,"arguments":c.arguments})).collect::<Vec<_>>(),
                "usage":response.usage.as_json(),"reported":response.usage_reported,"finish":response.finish_reason}).to_string()),
                "usage":response.usage.as_json(),"usageReported":response.usage_reported})
    }

    fn terminal_on(
        &self,
        tx: &Transaction<'_>,
        phase: &str,
        response: Option<&ModelResponse>,
        code: &str,
    ) -> Result<bool, String> {
        self.verify(tx, "dispatch", &self.dispatch())?;
        if !matches!(
            (phase, response),
            ("received", Some(_)) | ("uncertain", None) | ("unsent", None)
        ) {
            return Err("budget_root_terminal_invalid".into());
        }
        let fact = if let Some(response) = response {
            Self::received_fact(response)
        } else {
            json!({"code":code})
        };
        if phase == "received" && self.actual_cost_policy(tx)? {
            self.verify(tx, phase, &fact)?;
        } else {
            self.insert(tx, phase, &fact)?;
            if let Some(response) = response {
                self.record_cache_provenance(tx, response)?;
            }
        }
        let unresolved = receipt::settle(tx, self, phase, response)?;
        self.verify(tx, phase, &fact)?;
        self.verify(tx, "dispatch", &self.dispatch())?;
        Ok(unresolved)
    }
}
