//! Birth-bound first protection response signature. Historical reads never mint
//! a receipt, renew C, settle a model bill or grant executable recovery.
use super::{root, VerifiedMode, WebMode};
use crate::agent_runtime::store;
use rusqlite::{params, Connection};
use serde_json::Value;

pub(crate) struct ProtectionIssuer {
    root: String,
    birth_hash: String,
    proof: VerifiedMode,
}

impl ProtectionIssuer {
    pub(crate) fn from_verified_birth_on(
        db: &Connection,
        root_id: &str,
        proof: VerifiedMode,
    ) -> Result<Self, String> {
        require_snapshot(db)?;
        verify_schema_on(db)?;
        root::verify_birth(db, root_id, &proof)?;
        let d = root::read(db, root_id)?.ok_or("protection_issuer_root_missing")?;
        if d.mode() != WebMode::Multi
            || d.new_evidence_supervision().is_none()
            || d.protection_supervision().is_none()
        {
            return Err("protection_issuer_original_protocol_required".into());
        }
        let birth: String = db
            .query_row(
                "SELECT binding_json FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1",
                [root_id],
                |r| r.get(0),
            )
            .map_err(|_| "protection_issuer_original_birth_missing")?;
        Ok(Self {
            root: root_id.into(),
            birth_hash: store::stable_hash(&birth),
            proof,
        })
    }

    fn validate_fact(&self, db: &Connection, raw: &str) -> Result<Value, String> {
        require_snapshot(db)?;
        verify_schema_on(db)?;
        root::verify_birth(db, &self.root, &self.proof)?;
        let d = root::read(db, &self.root)?.ok_or("protection_issuer_root_missing")?;
        let birth: String = db
            .query_row(
                "SELECT binding_json FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1",
                [&self.root],
                |r| r.get(0),
            )
            .map_err(|_| "protection_issuer_original_birth_missing")?;
        let f: Value = serde_json::from_str(raw).map_err(|_| "protection_receipt_fact_invalid")?;
        let canonical = f.to_string();
        if f.as_object().is_none()
            || canonical != raw
            || f["schemaVersion"] != 1
            || store::stable_hash(&birth) != self.birth_hash
            || d.mode() != WebMode::Multi
            || d.new_evidence_supervision().is_none()
            || d.protection_supervision().is_none()
            || text(&f, "rootRunId")? != self.root
            || text(&f, "scanId")? != self.proof.fact().scan_id
            || positive(&f, "attemptNumber")? != self.proof.fact().attempt_number
            || text(&f, "targetUrl")? != d.target_url
            || text(&f, "protectionCode")? != "confirmed_waf_or_challenge"
        {
            return Err("protection_receipt_original_scope_changed".into());
        }
        for k in [
            "createdAt",
            "runId",
            "workerAttemptId",
            "assignmentId",
            "artifactId",
            "coordinatorFence",
        ] {
            text(&f, k)?;
        }
        for k in ["storageId", "claimRowid", "coordinatorEpoch"] {
            positive(&f, k)?;
        }
        if uuid::Uuid::parse_str(text(&f, "receiptId")?).is_err()
            || !(100..=599).contains(&positive(&f, "status")?)
            || f["status"] == 404
            || text(&f, "classifierVersion")? != "native-directory-block-signal.v1"
            || f["contentType"].as_str().is_none()
            || f["truncated"].as_bool().is_none()
            || f["originalFacts"].as_array().is_none_or(|a| a.is_empty())
            || f["preSendAuthorization"]
                .as_object()
                .is_none_or(|o| o.is_empty())
        {
            return Err("protection_receipt_fact_invalid".into());
        }
        let writer = &f["firstWriter"];
        if writer.as_object().is_none_or(|o| o.len() != 6)
            || text(writer, "artifactId")? != text(&f, "artifactId")?
            || writer["metadataBytes"].as_u64().is_none_or(|n| n == 0)
            || writer["bodyBytes"].as_u64().is_none()
        {
            return Err("protection_receipt_first_writer_invalid".into());
        }
        text(writer, "directory")?;
        for field in ["metadataSha256", "bodySha256"] {
            let hash = text(writer, field)?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("protection_receipt_first_writer_invalid".into());
            }
        }
        Ok(f)
    }

    fn message(&self, raw: &str) -> String {
        serde_json::json!({"domain":"oviraptor.protection-response-receipt.v1",
            "rootRunId":self.root,"birthBindingHash":self.birth_hash,"canonicalFact":raw})
        .to_string()
    }

    pub(crate) fn issue_new_on(
        &self,
        db: &Connection,
        fresh: &crate::scanning::http_exchange::FreshProtectionResponse,
    ) -> Result<(), String> {
        require_snapshot(db)?;
        let (actor, raw) = fresh.require_on(db)?;
        let f = self.validate_fact(db, raw)?;
        if actor.root_run_id != self.root
            || actor.scan_id != text(&f, "scanId")?
            || actor.attempt_number != positive(&f, "attemptNumber")?
            || actor.target_key != text(&f, "targetUrl")?
            || actor.lease_epoch != positive(&f, "coordinatorEpoch")?
            || actor.fencing_token != text(&f, "coordinatorFence")?
        {
            return Err("protection_issuer_original_actor_changed".into());
        }
        let tag = self.proof.sign_private(self.message(raw).as_bytes());
        let hash = store::stable_hash(raw);
        // The original receipt INSERT may not mutate any financial, grant or
        // unrelated source row through a trigger. This Connection is the
        // private transport writer, never an external shared reader.
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        db.authorizer(Some(|a: AuthContext<'_>| match a.action {
            AuthAction::Insert { table_name }
                if a.database_name == Some("main")
                    && table_name == "agent_protection_response_receipts"
                    && a.accessor.is_none() =>
            {
                Authorization::Allow
            }
            AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } => {
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }))
        .map_err(|_| "protection_issuer_write_guard_unavailable")?;
        let _guard = ProtectionReceiptWriteGuard(db);
        let n = db.execute(
            "INSERT INTO main.agent_protection_response_receipts(storage_id,receipt_id,root_run_id,scan_id,
             attempt_number,target_url,run_id,claim_rowid,worker_attempt_id,assignment_id,artifact_id,
             response_status,protection_code,fact_json,fact_hash,signature,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![positive(&f,"storageId")?,text(&f,"receiptId")?,self.root,text(&f,"scanId")?,
                positive(&f,"attemptNumber")?,text(&f,"targetUrl")?,text(&f,"runId")?,positive(&f,"claimRowid")?,
                text(&f,"workerAttemptId")?,text(&f,"assignmentId")?,text(&f,"artifactId")?,
                positive(&f,"status")?,text(&f,"protectionCode")?,raw,hash,tag,text(&f,"createdAt")?],
        ).map_err(|_| "protection_issuer_write_failed")?;
        if n != 1 || self.verify_issued(db, text(&f, "receiptId")?)? != raw {
            return Err("protection_issuer_write_unconfirmed".into());
        }
        let (after_actor, after_raw) = fresh.require_on(db)?;
        if after_actor != actor || after_raw != raw {
            return Err("protection_issuer_fresh_fact_changed".into());
        }
        self.validate_fact(db, after_raw)?;
        Ok(())
    }

    pub(crate) fn verify_issued(
        &self,
        db: &Connection,
        receipt_id: &str,
    ) -> Result<String, String> {
        require_snapshot(db)?;
        verify_schema_on(db)?;
        let (raw,hash,tag):(String,String,Vec<u8>) = db.query_row(
            "SELECT fact_json,fact_hash,signature FROM main.agent_protection_response_receipts WHERE receipt_id=?1",
            [receipt_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).map_err(|_| "protection_receipt_original_missing")?;
        let f = self.validate_fact(db, &raw)?;
        if text(&f, "receiptId")? != receipt_id
            || hash != store::stable_hash(&raw)
            || tag.len() != 32
        {
            return Err("protection_receipt_original_binding_changed".into());
        }
        self.proof
            .verify_private(self.message(&raw).as_bytes(), &tag)
            .map_err(|_| "protection_receipt_original_signature_invalid")?;
        let exact: bool = db.query_row(
            "SELECT count(*)=1 FROM main.agent_protection_response_receipts WHERE storage_id=?1 AND rowid=?1
             AND receipt_id=?2 AND root_run_id=?3 AND scan_id=?4 AND attempt_number=?5 AND target_url=?6
             AND run_id=?7 AND claim_rowid=?8 AND worker_attempt_id=?9 AND assignment_id=?10 AND artifact_id=?11
             AND response_status=?12 AND protection_code=?13 AND fact_json=?14 AND fact_hash=?15 AND signature=?16 AND created_at=?17",
            params![positive(&f,"storageId")?,receipt_id,self.root,text(&f,"scanId")?,positive(&f,"attemptNumber")?,
                text(&f,"targetUrl")?,text(&f,"runId")?,positive(&f,"claimRowid")?,text(&f,"workerAttemptId")?,
                text(&f,"assignmentId")?,text(&f,"artifactId")?,positive(&f,"status")?,text(&f,"protectionCode")?,
                raw,hash,tag,text(&f,"createdAt")?],|r|r.get(0),
        ).map_err(|_| "protection_receipt_original_columns_unavailable")?;
        if !exact {
            return Err("protection_receipt_original_columns_changed".into());
        }
        Ok(raw)
    }
}

fn require_snapshot(db: &Connection) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("protection_receipt_original_snapshot_required".into());
    }
    Ok(())
}
fn text<'a>(f: &'a Value, k: &str) -> Result<&'a str, String> {
    f.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "protection_receipt_fact_invalid".into())
}
fn positive(f: &Value, k: &str) -> Result<i64, String> {
    f.get(k)
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| "protection_receipt_fact_invalid".into())
}

fn schema_rows(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut q = db.prepare("SELECT name,sql FROM main.sqlite_master WHERE name IN
        ('agent_protection_response_receipts','protection_response_receipt_no_update',
         'protection_response_receipt_no_delete','protection_response_receipt_no_replace') ORDER BY name")
        .map_err(|_| "protection_receipt_schema_unavailable")?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| "protection_receipt_schema_unavailable")?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| "protection_receipt_schema_unavailable")?;
    Ok(rows)
}
pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected =
        Connection::open_in_memory().map_err(|_| "protection_receipt_schema_unavailable")?;
    expected
        .execute_batch(include_str!("protection_receipt.sql"))
        .map_err(|_| "protection_receipt_schema_unavailable")?;
    let shadow: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN
        ('agent_protection_response_receipts','protection_response_receipt_no_update',
         'protection_response_receipt_no_delete','protection_response_receipt_no_replace'))",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "protection_receipt_schema_unavailable")?;
    if shadow || schema_rows(db)? != schema_rows(&expected)? {
        return Err("protection_receipt_schema_conflict".into());
    }
    Ok(())
}

struct ProtectionReceiptWriteGuard<'a>(&'a Connection);
impl Drop for ProtectionReceiptWriteGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
    }
}
