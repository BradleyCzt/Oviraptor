// Only the same actual transport response and first writer can construct this.
// The private constructor is never exposed to a file reader or IPC caller.
pub(crate) struct FreshProtectionResponse {
    connection: *const rusqlite::Connection,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    raw: String,
    originals: Vec<NativeCoordinatorFrozenRows>,
    written: FreshHttpArtifactWritten,
}

impl FreshProtectionResponse {
    pub(crate) fn require_on<'a>(
        &'a self,
        db: &rusqlite::Connection,
    ) -> Result<
        (
            &'a crate::agent_runtime::multi_agent::lease::CoordinatorLease,
            &'a str,
        ),
        String,
    > {
        if db.is_autocommit() || !std::ptr::eq(self.connection, db) {
            return Err("protection_fresh_writer_required".into());
        }
        for row in &self.originals {
            row.verify(db)?;
        }
        let directory = open_agent_artifact_directory(self.written.directory())
            .ok_or("protection_original_directory_unavailable")?;
        for (name, hash, size) in [
            (
                self.written.artifact_id().to_string(),
                self.written.record_sha256(),
                self.written.record_bytes(),
            ),
            (
                self.written.artifact_id().replace(".json", ".body"),
                self.written.body_sha256(),
                self.written.body_bytes(),
            ),
        ] {
            let (bytes, length) = read_agent_artifact_file(&directory, &name, size as u64)
                .ok_or("protection_original_bytes_unavailable")?;
            if length != size as u64
                || bytes.len() != size
                || format!("{:x}", Sha256::digest(&bytes)) != hash
            {
                return Err("protection_original_bytes_changed".into());
            }
        }
        Ok((&self.actor, &self.raw))
    }
}

fn fresh_protection_response_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    witness: &ProtectionPreSendWitness,
    written: FreshHttpArtifactWritten,
    response: &ProtectionCapturedResponse<'_>,
    stop: &AgentStop,
) -> Result<FreshProtectionResponse, String> {
    if db.is_autocommit() || stop.code != AGENT_STOP_WAF {
        return Err("protection_fresh_original_waf_required".into());
    }
    witness.require_same_claim(context, claim)?;
    witness.require_original_on(db)?;
    if written.scan_id() != context.scan_id
        || written.attempt_number() != context.attempt_number
        || written.target_url() != context.target_url
        || written.run_id() != Some(claim.run_id.as_str())
        || written.directory() != context.target_dir.join(AGENT_HTTP_DIRECTORY)
        || written.body_bytes() != response.body.len()
        || written.body_sha256() != format!("{:x}", Sha256::digest(response.body))
    {
        return Err("protection_fresh_original_writer_conflict".into());
    }
    let actor = witness.actor.clone();
    let source = agent_http_budget_source(claim);
    let (assignment, worker) =
        crate::agent_runtime::multi_agent::budget::target::original_target_receipt(
            db,
            &actor,
            &claim.run_id,
            &source,
        )?;
    let header_raw = protection_stop_original_claim_on(db, context, claim, response.status)?;
    let claim_rowid: i64 = db.query_row(
        "SELECT rowid FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3 AND ordinal=?4",
        params![context.scan_id,context.target_url,claim.budget_attempt,claim.ordinal],|r|r.get(0),
    ).map_err(|_|"protection_fresh_headers_missing")?;
    let stop_original: bool = db.query_row(
        "SELECT count(*)=1 FROM agent_protection_stops WHERE scan_id=?1 AND target_url=?2 AND source_attempt_number=?3 AND source_run_id=?4 AND stop_code=?5 AND claim_json=?6 AND claim_hash=?7",
        params![context.scan_id,context.target_url,context.attempt_number,claim.run_id,stop.code,
            header_raw,crate::agent_runtime::store::stable_hash(&header_raw)],|r|r.get(0),
    ).map_err(|_|"protection_fresh_stop_missing")?;
    if !stop_original {
        return Err("protection_fresh_stop_changed".into());
    }
    let mut originals = witness.historical.clone();
    let budget = claim.budget_attempt.to_string();
    let ordinal = claim.ordinal.to_string();
    for (sql, keys) in [
        ("SELECT rowid,* FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",vec![context.scan_id.as_str(),context.target_url.as_str(),budget.as_str(),ordinal.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 ORDER BY rowid",vec![actor.root_run_id.as_str(),assignment.as_str(),worker.as_str(),source.as_str()]),
        ("SELECT rowid,* FROM agent_protection_stops WHERE scan_id=?1 AND target_url=?2",vec![context.scan_id.as_str(),context.target_url.as_str()]),
    ] { originals.push(NativeCoordinatorFrozenRows::capture(db,sql,&keys)?); }
    let storage: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(storage_id),0)+1 FROM agent_protection_response_receipts",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "protection_fresh_storage_unavailable")?;
    let created: String = db
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let raw = json!({"schemaVersion":1,"storageId":storage,"receiptId":Uuid::new_v4().to_string(),
        "createdAt":created,"rootRunId":actor.root_run_id,"scanId":context.scan_id,
        "attemptNumber":context.attempt_number,"targetUrl":context.target_url,"runId":claim.run_id,
        "claimRowid":claim_rowid,"workerAttemptId":worker,"assignmentId":assignment,
        "artifactId":written.artifact_id(),"status":response.status,"protectionCode":stop.code,
        "coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,
        "classifierVersion":"native-directory-block-signal.v1","contentType":response.content_type,
        "truncated":response.truncated,"preSendAuthorization":serde_json::from_str::<JsonValue>(&witness.canonical_authorization).map_err(|_|"protection_fresh_authorization_invalid")?,
        "originalFacts":originals.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>(),
        "firstWriter":{"directory":written.directory(),"artifactId":written.artifact_id(),
            "metadataSha256":written.record_sha256(),"metadataBytes":written.record_bytes(),
            "bodySha256":written.body_sha256(),"bodyBytes":written.body_bytes()}}).to_string();
    let fresh = FreshProtectionResponse {
        connection: db,
        actor,
        raw,
        originals,
        written,
    };
    fresh.require_on(db)?;
    Ok(fresh)
}
