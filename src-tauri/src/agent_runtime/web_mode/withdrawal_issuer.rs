//! Original private birth-bound issuer. It grants no model or target authority.
use super::{root, VerifiedMode, WebMode};
use crate::agent_runtime::{
    multi_agent::{
        attempts::withdrawal_receipt::{self, FreshWithdrawalFact},
        lease::CoordinatorLease,
    },
    store,
};
use rusqlite::{params, Connection};

pub(crate) struct WithdrawalIssuer {
    root: String,
    birth_hash: String,
    proof: VerifiedMode,
}
impl WithdrawalIssuer {
    pub(crate) fn sign_fresh_homogeneous_execution_fact(&self,fresh:&crate::commands::homogeneous_execution::FreshFact)->Result<Vec<u8>,String>{
        let encoded=fresh.encoded_for_original(self.proof.fact())?;
        let value:serde_json::Value=serde_json::from_slice(&encoded).map_err(|_|"homogeneous_execution_fact_invalid")?;
        if value["rootRunId"]!=self.root {return Err("homogeneous_execution_fact_root_changed".into());}Ok(self.proof.sign_private(&encoded))
    }
    pub(crate) fn verify_homogeneous_execution_fact(&self,encoded:&[u8],signature:&[u8])->Result<(),String>{
        let value:serde_json::Value=serde_json::from_slice(encoded).map_err(|_|"homogeneous_execution_fact_invalid")?;
        if value["domain"]!="oviraptor.original-homogeneous-execution.v1" || value["rootRunId"]!=self.root {return Err("homogeneous_execution_fact_root_changed".into());}
        self.proof.verify_private(encoded,signature).map_err(|_|"homogeneous_execution_fact_signature_invalid".into())
    }

    pub(crate) fn sign_fresh_homogeneous_parser_fact(&self,fresh:&crate::agent_runtime::multi_agent::homogeneous::parser_execution::FreshParserFact<'_>)->Result<Vec<u8>,String>{
        let encoded=fresh.encoded_for_original(self.proof.fact())?;
        let value:serde_json::Value=serde_json::from_slice(&encoded).map_err(|_|"homogeneous_parser_fact_invalid")?;
        if value["rootRunId"]!=self.root {return Err("homogeneous_parser_fact_root_changed".into());}
        Ok(self.proof.sign_private(&encoded))
    }
    pub(crate) fn verify_homogeneous_parser_fact(&self,encoded:&[u8],signature:&[u8])->Result<(),String>{
        let value:serde_json::Value=serde_json::from_slice(encoded).map_err(|_|"homogeneous_parser_fact_invalid")?;
        if value["domain"]!="oviraptor.original-homogeneous-parser.v1" || value["rootRunId"]!=self.root {
            return Err("homogeneous_parser_fact_root_changed".into());
        }
        self.proof.verify_private(encoded,signature).map_err(|_|"homogeneous_parser_fact_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_frozen_tool_plan(
        &self, fresh: &crate::tool_supply::frozen_plan::FreshFrozenToolPlan<'_, '_>,
    ) -> Result<Vec<u8>, String> {
        fresh.require_original_root(&self.root)?;
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_frozen_tool_plan(
        &self, encoded: &[u8], signature: &[u8],
    ) -> Result<(), String> {
        self.proof.verify_private(encoded, signature)
            .map_err(|_| "tool_supply_frozen_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_mapper_loop_expired_receipt(
        &self,
        fresh: &crate::agent_runtime::multi_agent::specialist::mapper_loop::expired_seal::FreshExpiredMapperReceipt<'_, '_, '_>,
    ) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_mapper_loop_expired_receipt(
        &self, encoded: &[u8], signature: &[u8],
    ) -> Result<(), String> {
        self.proof.verify_private(encoded, signature)
            .map_err(|_| "mapper_loop_expired_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_mapper_loop_final_receipt(
        &self,
        fresh: &crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::FreshMapperLoopFinalReceipt,
    ) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_mapper_loop_final_receipt(
        &self, encoded: &[u8], signature: &[u8],
    ) -> Result<(), String> {
        self.proof.verify_private(encoded, signature)
            .map_err(|_| "mapper_loop_final_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_business_logic_dispatch_receipt(&self, fresh: &crate::commands::business_logic_dispatch_receipt::FreshBusinessLogicDispatchReceipt) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_business_logic_dispatch_receipt(&self, encoded: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_private(encoded, signature).map_err(|_| "business_logic_dispatch_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_business_logic_output_receipt(&self, fresh: &crate::commands::business_logic_output_receipt::FreshBusinessLogicOutputReceipt) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_business_logic_output_receipt(&self, encoded: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_private(encoded, signature).map_err(|_| "business_logic_output_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_external_surface_output_receipt(&self, fresh: &crate::commands::external_surface_output_receipt::FreshExternalSurfaceOutputReceipt) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_external_surface_output_receipt(&self, encoded: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_private(encoded, signature).map_err(|_| "external_surface_output_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_public_surface_response_receipt(&self, fresh: &crate::commands::external_surface_response_receipt::FreshPublicSurfaceResponse) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_public_surface_response_receipt(&self, encoded: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_private(encoded, signature).map_err(|_| "public_surface_response_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_authorization_output_receipt(&self, fresh: &crate::commands::authorization_output_receipt::FreshAuthorizationOutputReceipt) -> Result<Vec<u8>, String> {
        Ok(self.proof.sign_private(&fresh.encoded_for_original(self.proof.fact())?))
    }
    pub(crate) fn verify_authorization_output_receipt(&self, encoded: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_private(encoded, signature).map_err(|_| "authorization_output_signature_invalid".into())
    }
    pub(crate) fn sign_fresh_web_expiry_delivery(&self, fresh: &super::web_expiry_delivery::FreshWebExpiryDelivery) -> Result<Vec<u8>, String> {
        self.proof.sign_fresh_web_expiry_delivery(fresh)
    }
    pub(crate) fn verify_web_expiry_delivery(&self, message: &[u8], signature: &[u8]) -> Result<(), String> {
        self.proof.verify_web_expiry_delivery(message, signature)
    }

    pub(crate) fn from_verified_birth_on(
        db: &Connection,
        root_id: &str,
        proof: VerifiedMode,
    ) -> Result<Self, String> {
        root::verify_birth(db, root_id, &proof)?;
        let d = root::read(db, root_id)?.ok_or("withdrawal_issuer_root_missing")?;
        // A02's signed birth has no ordinary new-evidence protocol. Even a
        // cold DB deletion of its startup label cannot make it ordinary.
        if d.mode() != WebMode::Multi || d.new_evidence_supervision().is_none() {
            return Err("withdrawal_issuer_ordinary_birth_required".into());
        }
        let raw: String = db
            .query_row(
                "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
                [root_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            root: root_id.into(),
            birth_hash: store::stable_hash(&raw),
            proof,
        })
    }
    pub(crate) fn require_actor(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
    ) -> Result<(), String> {
        root::verify_birth(db, &self.root, &self.proof)?;
        let d = root::read(db, &self.root)?.ok_or("withdrawal_issuer_root_missing")?;
        let raw: String = db
            .query_row(
                "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
                [&self.root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if actor.root_run_id != self.root
            || actor.scan_id != self.proof.fact().scan_id
            || actor.attempt_number != self.proof.fact().attempt_number
            || actor.target_key != d.target_url
            || store::stable_hash(&raw) != self.birth_hash
        {
            return Err("withdrawal_issuer_original_scope_changed".into());
        }
        Ok(())
    }
    fn message(&self, raw: &str) -> String {
        serde_json::json!({"domain":"oviraptor.worker-expiry-issuance.v1","rootRunId":self.root,"birthBindingHash":self.birth_hash,"canonicalFact":raw}).to_string()
    }
    pub(crate) fn issue_new_on(
        &self,
        db: &Connection,
        fresh: &FreshWithdrawalFact<'_, '_>,
    ) -> Result<(), String> {
        let (actor, raw) = fresh.require(db)?;
        self.require_actor(db, actor)?;
        let id: String = db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE root_run_id=?1 AND fact_json=?2", params![self.root,raw], |r|r.get(0)).map_err(|_|"withdrawal_issuer_fresh_receipt_missing")?;
        let message = self.message(raw);
        let tag = self.proof.sign_private(message.as_bytes());
        let n = db.execute("INSERT INTO agent_worker_expiry_issuance_seals(receipt_id,root_run_id,binding_json,signature) VALUES(?1,?2,?3,?4)", params![id,self.root,message,tag]).map_err(|e|e.to_string())?;
        if n != 1 {
            return Err("withdrawal_issuer_write_unconfirmed".into());
        }
        Ok(())
    }
    pub(crate) fn verify_issued(&self, db: &Connection, worker: &str) -> Result<(), String> {
        withdrawal_receipt::verify_original(db, worker)?;
        root::verify_birth(db, &self.root, &self.proof)?;
        let (root_id, raw, message, tag): (String,String,String,Vec<u8>) = db.query_row("SELECT r.root_run_id,r.fact_json,s.binding_json,s.signature FROM agent_worker_expiry_receipts r JOIN agent_worker_expiry_issuance_seals s ON s.receipt_id=r.receipt_id AND s.root_run_id=r.root_run_id WHERE r.worker_attempt_id=?1", [worker], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"withdrawal_issuer_original_seal_missing")?;
        if root_id != self.root || message != self.message(&raw) || tag.len() != 32 {
            return Err("withdrawal_issuer_original_binding_changed".into());
        }
        self.proof
            .verify_private(message.as_bytes(), &tag)
            .map_err(|_| "withdrawal_issuer_signature_invalid".into())
    }
    fn authorization_side_message(&self, raw: &str) -> String {
        serde_json::json!({"domain":"oviraptor.authorization-side-receipt.v1",
            "rootRunId":self.root,"birthBindingHash":self.birth_hash,"canonicalFact":raw})
        .to_string()
    }

    /// Only the actual broker's opaque first-response envelope may issue this
    /// domain. Existing claims/files and arbitrary JSON cannot construct it.
    pub(crate) fn issue_authorization_side_on(
        &self,
        db: &Connection,
        fresh: &crate::commands::authorization_side::FreshAuthorizationSideReceipt,
    ) -> Result<(), String> {
        let (actor, raw) = fresh.require_on(db)?;
        self.require_actor(db, actor)?;
        let fact = self.authorization_side_fact(db, raw)?;
        let id = authorization_side_text(&fact, "receiptId")?;
        let message = self.authorization_side_message(raw);
        let tag = self.proof.sign_private(message.as_bytes());
        let hash = store::stable_hash(raw);
        let n = db.execute("INSERT INTO main.agent_authorization_side_receipts(storage_id,receipt_id,root_run_id,scan_id,attempt_number,target_url,contract_key,side,claim_rowid,assignment_id,child_run_id,worker_attempt_id,artifact_id,fact_json,fact_hash,signature,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![authorization_side_positive(&fact,"storageId")?,id,self.root,
                authorization_side_text(&fact,"scanId")?,authorization_side_positive(&fact,"attemptNumber")?,
                authorization_side_text(&fact,"targetUrl")?,authorization_side_text(&fact,"contractKey")?,
                authorization_side_text(&fact,"side")?,authorization_side_positive(&fact,"claimRowid")?,
                authorization_side_text(&fact,"assignmentId")?,authorization_side_text(&fact,"childRunId")?,
                authorization_side_text(&fact,"workerAttemptId")?,authorization_side_text(&fact,"artifactId")?,
                raw,hash,tag,authorization_side_text(&fact,"createdAt")?]).map_err(|_|"authorization_side_issuance_write_failed")?;
        if n != 1 {
            return Err("authorization_side_issuance_write_unconfirmed".into());
        }
        let original = self.verify_authorization_side_issued(db, id)?;
        if original != raw {
            return Err("authorization_side_issuance_postcondition_failed".into());
        }
        // A later trigger cannot rewrite the fresh original claim/fee/control
        // after the signature INSERT and silently leave a valid-looking seal.
        let (after_actor, after_raw) = fresh.require_on(db)?;
        if after_actor != actor || after_raw != raw {
            return Err("authorization_side_issuance_fresh_fact_changed".into());
        }
        self.require_actor(db, after_actor)
    }

    /// Historical financial proof only. This does not renew C, reactivate a
    /// worker, repair a signature or require an old execution clock to be live.
    pub(crate) fn verify_authorization_side_issued(
        &self,
        db: &Connection,
        receipt_id: &str,
    ) -> Result<String, String> {
        if db.is_autocommit() {
            return Err("authorization_side_original_snapshot_required".into());
        }
        let (raw,hash,tag):(String,String,Vec<u8>)=db.query_row(
            "SELECT fact_json,fact_hash,signature FROM main.agent_authorization_side_receipts WHERE receipt_id=?1",
            [receipt_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"authorization_side_original_receipt_missing")?;
        let fact = self.authorization_side_fact(db, &raw)?;
        if authorization_side_text(&fact, "receiptId")? != receipt_id
            || hash != store::stable_hash(&raw)
            || tag.len() != 32
        {
            return Err("authorization_side_original_binding_changed".into());
        }
        let message = self.authorization_side_message(&raw);
        self.proof
            .verify_private(message.as_bytes(), &tag)
            .map_err(|_| "authorization_side_original_signature_invalid")?;
        let exact:bool=db.query_row("SELECT count(*)=1 FROM main.agent_authorization_side_receipts WHERE receipt_id=?1 AND storage_id=?2 AND rowid=?2 AND root_run_id=?3 AND scan_id=?4 AND attempt_number=?5 AND target_url=?6 AND contract_key=?7 AND side=?8 AND claim_rowid=?9 AND assignment_id=?10 AND child_run_id=?11 AND worker_attempt_id=?12 AND artifact_id=?13 AND fact_json=?14 AND fact_hash=?15 AND signature=?16 AND created_at=?17",
            params![receipt_id,authorization_side_positive(&fact,"storageId")?,self.root,
                authorization_side_text(&fact,"scanId")?,authorization_side_positive(&fact,"attemptNumber")?,
                authorization_side_text(&fact,"targetUrl")?,authorization_side_text(&fact,"contractKey")?,
                authorization_side_text(&fact,"side")?,authorization_side_positive(&fact,"claimRowid")?,
                authorization_side_text(&fact,"assignmentId")?,authorization_side_text(&fact,"childRunId")?,
                authorization_side_text(&fact,"workerAttemptId")?,authorization_side_text(&fact,"artifactId")?,
                raw,hash,tag,authorization_side_text(&fact,"createdAt")?],|r|r.get(0)).map_err(|_|"authorization_side_original_binding_unavailable")?;
        if !exact {
            return Err("authorization_side_original_columns_changed".into());
        }
        Ok(raw)
    }

    fn authorization_side_fact(
        &self,
        db: &Connection,
        raw: &str,
    ) -> Result<serde_json::Value, String> {
        if db.is_autocommit() {
            return Err("authorization_side_original_snapshot_required".into());
        }
        root::verify_birth(db, &self.root, &self.proof)?;
        let birth: String = db
            .query_row(
                "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
                [&self.root],
                |r| r.get(0),
            )
            .map_err(|_| "authorization_side_original_birth_missing")?;
        let d = root::read(db, &self.root)?.ok_or("authorization_side_original_root_missing")?;
        let fact: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "authorization_side_original_fact_invalid")?;
        let canonical = fact.to_string();
        if fact.as_object().is_none()
            || canonical != raw
            || store::stable_hash(&birth) != self.birth_hash
            || d.mode() != WebMode::Multi
            || d.new_evidence_supervision().is_none()
            || authorization_side_text(&fact, "rootRunId")? != self.root
            || authorization_side_text(&fact, "scanId")? != self.proof.fact().scan_id
            || authorization_side_positive(&fact, "attemptNumber")?
                != self.proof.fact().attempt_number
            || authorization_side_text(&fact, "targetUrl")? != d.target_url
            || !matches!(
                authorization_side_text(&fact, "side")?,
                "owner" | "cross" | "tester"
            )
        {
            return Err("authorization_side_original_scope_changed".into());
        }
        for key in [
            "receiptId",
            "createdAt",
            "contractKey",
            "assignmentId",
            "childRunId",
            "workerAttemptId",
            "artifactId",
        ] {
            authorization_side_text(&fact, key)?;
        }
        for key in ["storageId", "claimRowid"] {
            authorization_side_positive(&fact, key)?;
        }
        Ok(fact)
    }
}

fn authorization_side_text<'a>(fact: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    fact.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "authorization_side_original_fact_invalid".into())
}
fn authorization_side_positive(fact: &serde_json::Value, key: &str) -> Result<i64, String> {
    fact.get(key)
        .and_then(serde_json::Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| "authorization_side_original_fact_invalid".into())
}
