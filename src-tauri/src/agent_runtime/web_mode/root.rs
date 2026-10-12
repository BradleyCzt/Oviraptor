//! New Root sidecar; no Native plan mutation and no existing Root upgrade.
use super::{ModeFact, VerifiedMode, WebMode};
use crate::agent_runtime::{multi_agent::lease, store};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod birth_seal;
pub(crate) use birth_seal::verify_birth;
mod bootstrap;
use bootstrap::BootstrapDispatch;
mod local;
use local::LocalDeliberation;
mod observation;
use observation::LiveBudgetObservation;
mod new_evidence;
use new_evidence::NewEvidenceSupervision;
mod lease_expiry;
use lease_expiry::LeaseExpirySupervision;
mod protection;
use protection::ProtectionSupervision;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NewRootModeDeclaration {
    pub(super) mode_id: String,
    pub(super) target_url: String,
    pub(super) plan_hash: String,
    pub(super) fact: ModeFact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_deliberation: Option<LocalDeliberation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bootstrap_dispatch: Option<BootstrapDispatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live_budget_observation: Option<LiveBudgetObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new_evidence_supervision: Option<NewEvidenceSupervision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease_expiry_supervision: Option<LeaseExpirySupervision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protection_supervision: Option<ProtectionSupervision>,
    #[serde(skip)]
    authorization: Option<VerifiedMode>,
}
impl NewRootModeDeclaration {
    pub(super) fn verify_fresh_birth(&self, db: &Connection, root: &str) -> Result<(), String> {
        verify_birth(db, root, self.authorization.as_ref().ok_or("web_mode_private_creation_missing")?)
    }
    pub(crate) fn from_verified(
        proof: &VerifiedMode,
        target: &str,
        plan_hash: &str,
    ) -> Result<Self, String> {
        let value = Self {
            mode_id: uuid::Uuid::new_v4().to_string(),
            target_url: target.into(),
            plan_hash: plan_hash.into(),
            fact: proof.fact().clone(),
            local_deliberation: None,
            bootstrap_dispatch: None,
            live_budget_observation: None,
            new_evidence_supervision: None,
            lease_expiry_supervision: None,
            protection_supervision: None,
            authorization: Some(proof.clone()),
        };
        value.validate_scope(
            &value.fact.scan_id,
            value.fact.attempt_number,
            target,
            plan_hash,
        )?;
        Ok(value)
    }
    pub(crate) fn validate_scope(
        &self,
        scan: &str,
        attempt: i64,
        target: &str,
        hash: &str,
    ) -> Result<(), String> {
        self.fact.validate()?;
        if let Some(local) = &self.local_deliberation {
            local.validate()?;
            if self.mode() != WebMode::Multi {
                return Err("root_local_deliberation_mode_invalid".into());
            }
        }
        if let Some(bootstrap) = &self.bootstrap_dispatch {
            bootstrap.validate()?;
            if self.mode() != WebMode::Multi {
                return Err("root_bootstrap_dispatch_mode_invalid".into());
            }
        }
        if let Some(observation) = &self.live_budget_observation {
            observation.validate()?;
            if self.mode() != WebMode::Multi {
                return Err("root_budget_observation_mode_invalid".into());
            }
        }
        if let Some(protocol) = &self.new_evidence_supervision {
            protocol.validate()?;
            if self.mode() != WebMode::Multi {
                return Err("root_new_evidence_supervision_mode_invalid".into());
            }
        }
        if let Some(protocol) = &self.lease_expiry_supervision {
            protocol.validate()?;
            if self.mode() != WebMode::Multi || self.new_evidence_supervision.is_none() {
                return Err("root_lease_expiry_supervision_mode_invalid".into());
            }
        }
        if let Some(protocol) = &self.protection_supervision {
            protocol.validate()?;
            if self.mode() != WebMode::Multi || self.new_evidence_supervision.is_none() {
                return Err("root_protection_supervision_mode_invalid".into());
            }
        }
        if uuid::Uuid::parse_str(&self.mode_id).is_err()
            || self.fact.scan_id != scan
            || self.fact.attempt_number != attempt
            || self.target_url != target
            || self.plan_hash != hash
            || !self.fact.targets.iter().any(|s| s == target)
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("web_mode_root_scope_invalid".into());
        }
        Ok(())
    }
    pub(crate) fn with_local_deliberation(mut self) -> Result<Self, String> {
        if self.authorization.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        if self.mode() == WebMode::Multi {
            self.local_deliberation = Some(LocalDeliberation::creation_contract());
            self.bootstrap_dispatch = Some(BootstrapDispatch::creation_contract());
            self.live_budget_observation = Some(LiveBudgetObservation::creation_contract());
        }
        Ok(self)
    }
    pub(crate) fn with_new_evidence_supervision(mut self) -> Result<Self, String> {
        if self.authorization.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        if self.mode() == WebMode::Multi {
            self.new_evidence_supervision = Some(NewEvidenceSupervision::creation_contract());
        }
        Ok(self)
    }
    // Explicit legacy fixture birth before INSERT. No database writer or
    // existing-root reader calls this; the original v5 byte contract is kept.
    #[cfg(test)]
    pub(crate) fn with_original_v5_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::original_v5_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    // Original v6 regression birth, before INSERT and signing. Existing
    // receipts are read unchanged and are never downgraded or re-signed.
    #[cfg(test)]
    pub(crate) fn with_original_v6_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::original_v6_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_original_v8_http_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::http_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_original_v7_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::original_v7_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_web_expiry_v9_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::web_expiry_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_authorization_v11_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::authorization_output_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_mapper_v15_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::mapper_loop_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_business_logic_v14_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::business_logic_dispatch_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_business_logic_v13_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::business_logic_output_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_external_surface_v12_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::external_surface_output_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_capability_v10_bootstrap_for_test(mut self) -> Result<Self, String> {
        if self.authorization.is_none() || self.mode() != WebMode::Multi || self.local_deliberation.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        self.bootstrap_dispatch = Some(BootstrapDispatch::capability_creation_contract());
        self.bootstrap_dispatch.as_ref().unwrap().validate()?;
        Ok(self)
    }
    pub(crate) fn with_lease_expiry_supervision(mut self) -> Result<Self, String> {
        if self.authorization.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        if self.mode() == WebMode::Multi {
            if self.new_evidence_supervision.is_none() {
                return Err("root_lease_expiry_supervision_mode_invalid".into());
            }
            self.lease_expiry_supervision = Some(LeaseExpirySupervision::creation_contract());
        }
        Ok(self)
    }
    pub(crate) fn with_protection_supervision(mut self) -> Result<Self, String> {
        if self.authorization.is_none() {
            return Err("web_mode_private_creation_missing".into());
        }
        if self.mode() == WebMode::Multi {
            if self.new_evidence_supervision.is_none() {
                return Err("root_protection_supervision_mode_invalid".into());
            }
            self.protection_supervision = Some(ProtectionSupervision::creation_contract());
        }
        Ok(self)
    }
    pub(crate) fn protection_supervision(&self) -> Option<Value> {
        self.protection_supervision.as_ref().map(ProtectionSupervision::as_json)
    }
    pub(crate) fn lease_expiry_supervision(&self) -> Option<Value> {
        self.lease_expiry_supervision.as_ref().map(LeaseExpirySupervision::as_json)
    }
    pub(crate) fn new_evidence_supervision(&self) -> Option<Value> {
        self.new_evidence_supervision
            .as_ref()
            .map(NewEvidenceSupervision::as_json)
    }
    pub(crate) fn local_deliberation(&self) -> Option<Value> {
        self.local_deliberation
            .as_ref()
            .map(LocalDeliberation::as_json)
    }
    pub(crate) fn bootstrap_dispatch(&self) -> Option<Value> {
        self.bootstrap_dispatch
            .as_ref()
            .map(BootstrapDispatch::as_json)
    }
    pub(crate) fn live_budget_observation(&self) -> Option<Value> {
        self.live_budget_observation
            .as_ref()
            .map(LiveBudgetObservation::as_json)
    }
    pub(crate) fn mode(&self) -> WebMode {
        self.fact.mode
    }
    pub(crate) fn fact(&self) -> &ModeFact {
        &self.fact
    }
    pub(crate) fn same_binding(&self, other: &Self) -> bool {
        self.mode_id == other.mode_id
            && self.target_url == other.target_url
            && self.plan_hash == other.plan_hash
            && self.fact == other.fact
            && self.local_deliberation == other.local_deliberation
            && self.bootstrap_dispatch == other.bootstrap_dispatch
            && self.live_budget_observation == other.live_budget_observation
            && self.new_evidence_supervision == other.new_evidence_supervision
            && self.lease_expiry_supervision == other.lease_expiry_supervision
            && self.protection_supervision == other.protection_supervision
    }
}

pub(crate) fn freeze_new(
    fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    declaration: &NewRootModeDeclaration,
) -> Result<(), String> {
    let db = fresh.transaction();
    let root = fresh.id();
    if declaration
        .authorization
        .as_ref()
        .is_none_or(|p| p.fact() != &declaration.fact)
    {
        return Err("web_mode_private_creation_missing".into());
    }
    lease::require_active_attempt(
        db,
        &declaration.fact.scan_id,
        declaration.fact.attempt_number,
    )?;
    validate_root(db, root, declaration)?;
    let history:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_runs p ON p.id=?1
        WHERE r.scan_id=p.scan_id AND r.attempt_number=p.attempt_number AND r.target_url=p.target_url AND r.id<>p.id)
        OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_mode_definitions WHERE root_run_id=?1)",[root],|r|r.get(0)).map_err(|e|e.to_string())?;
    let pristine:bool=db.query_row("SELECT status='prepared' AND started_at='' AND finished_at='' AND cancel_requested_at=''
        AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0 AND reserved_tokens=0 AND reserved_requests=0
        FROM agent_runs WHERE id=?1",[root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if history || !pristine {
        return Err("web_mode_requires_new_root".into());
    }
    let text: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let n=db.execute("INSERT INTO agent_root_mode_definitions(root_run_id,mode_id,definition_json,native_plan_text_hash) VALUES(?1,?2,?3,?4)",
        params![root,declaration.mode_id,serde_json::to_string(declaration).map_err(|e|e.to_string())?,store::stable_hash(&text)]).map_err(|e|e.to_string())?;
    birth_seal::issue(fresh, declaration)?;
    if n != 1
        || read(db, root)?
            .as_ref()
            .is_none_or(|saved| !saved.same_binding(declaration))
    {
        return Err("web_mode_root_receipt_changed".into());
    }
    Ok(())
}

fn validate_root(db: &Connection, root: &str, d: &NewRootModeDeclaration) -> Result<(), String> {
    let (scan,attempt,target,hash,text,policy,declared):(String,i64,String,String,String,String,String)=db.query_row(
        "SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,orchestration_policy,root_run_id FROM agent_runs
        WHERE id=?1 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''",
        [root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|_|"web_mode_root_missing")?;
    d.validate_scope(&scan, attempt, &target, &hash)?;
    let plan: Value = serde_json::from_str(&text).map_err(|_| "web_mode_plan_invalid")?;
    if policy != d.mode().as_str()
        || declared != if d.mode() == WebMode::Multi { root } else { "" }
        || plan["owner"] != "oviraptor"
        || plan["backend"] != "native"
        || plan["schemaVersion"] != 2
        || plan["executionSurface"] != "web_only"
        || plan["surface"] == "source"
        || plan["attemptNumber"] != attempt
        || plan["targetUrl"] != target
    {
        return Err("web_mode_root_binding_changed".into());
    }
    let mut frozen = plan.clone();
    let object = frozen.as_object_mut().ok_or("web_mode_plan_invalid")?;
    object.remove("attemptNumber");
    object.remove("targetUrl");
    if store::stable_hash(&frozen.to_string()) != hash {
        return Err("web_mode_plan_hash_changed".into());
    }
    let (receipt, _) = super::stored_fact(db, &scan, attempt)?;
    if receipt != d.fact {
        return Err("web_mode_startup_fact_changed".into());
    }
    Ok(())
}

pub(crate) fn read(db: &Connection, root: &str) -> Result<Option<NewRootModeDeclaration>, String> {
    let saved:Option<(String,String,String)>=db.query_row("SELECT mode_id,definition_json,native_plan_text_hash FROM agent_root_mode_definitions WHERE root_run_id=?1",
        [root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, text, hash)) = saved else {
        return Ok(None);
    };
    let d: NewRootModeDeclaration =
        serde_json::from_str(&text).map_err(|_| "web_mode_root_receipt_invalid")?;
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if d.mode_id != id
        || serde_json::to_string(&d).map_err(|e| e.to_string())? != text
        || store::stable_hash(&plan) != hash
    {
        return Err("web_mode_root_receipt_changed".into());
    }
    validate_root(db, root, &d)?;
    Ok(Some(d))
}
