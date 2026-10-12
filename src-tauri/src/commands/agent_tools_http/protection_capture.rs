// Included only inside the actual HTTP transport module. This writes bounded
// original bytes for audit; a fresh WAF response may issue its original seal.
struct ProtectionCapturedResponse<'a> {
    url: &'a str,
    headers: &'a [(String, String)],
    body: &'a [u8],
    status: u16,
    content_type: &'a str,
    truncated: bool,
}

fn capture_protected_http_response(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    request: &AgentHttpRequest,
    claim: Option<&AgentHttpClaim>,
    pre_send: Option<&ProtectionPreSendWitness>,
    response: ProtectionCapturedResponse<'_>,
    stop: AgentStop,
) -> JsonValue {
    let Some(claim) = claim else {
        // Unregistered diagnostics have no durable headers claim to bind.
        return json!({"code":"protected","stop":stop});
    };
    if agent_persist_protection_stop(context, claim, response.status, &stop).is_err() {
        return json!({"code":"protected","stop":stop,"protectionPersistence":"failed"});
    }
    if pre_send.is_some_and(|witness| witness.require_same_claim(context, claim).is_err()) {
        return json!({"code":"protected","stop":stop,"protectionCapture":"original_authorization_binding_changed"});
    }
    let mut record = json!({
        "schemaVersion":1,
        "status":response.status,
        "contentType":response.content_type.chars().take(256).collect::<String>(),
        "truncated":response.truncated,
        "bodySha256":format!("{:x}",Sha256::digest(response.body)),
        "protectionCode":stop.code,
        "provenance":"transport_observed_unsealed",
        "httpClaim":{"runId":claim.run_id,
            "invocationId":claim.invocation_id,"requestIndex":claim.request_index,
            "budgetAttempt":claim.budget_attempt,"ordinal":claim.ordinal,
            "requestHash":claim.request_hash},
    });
    if let Some(witness) = pre_send {
        // This hash records the first send authorization. It is unsealed audit
        // evidence, not a paid source or a renewable execution grant.
        record["preSendAuthorizationHash"] = witness.canonical_authorization_sha256().into();
    }
    let written = agent_write_http_record_fresh(
        context,
        runtime.target_requests,
        &json!({
            "identity":request.identity.key,"method":request.method,"url":response.url,
            "headers":response.headers.iter().map(|(name,value)|json!({"name":name,"value":value})).collect::<Vec<_>>(),
            "body":request.body,"contractKey":request.contract_key,
        }),
        &record,
        response.body,
    );
    let written = match written {
        Ok(written) => written,
        Err(_) => {
            // The already committed stop and known headers cost are retained.
            // No GET replay, graph publication or read-time repair follows.
            return json!({"code":"protected","stop":stop,"protectionCapture":"failed"});
        }
    };
    runtime.note_artifact(written.artifact_id());
    runtime.attach_artifact(written.artifact_id(), &written.body_sha256()[..12]);
    let artifact = written.artifact_id().to_string();
    let mut result = json!({"code":"protected","stop":stop,"artifactId":artifact,
        "protectionArtifactProvenance":"transport_observed_unsealed"});
    if let Some(witness) = pre_send.filter(|_| stop.code == AGENT_STOP_WAF) {
        let issued = (|| -> Result<String,String> {
            let connection = db::open(&context.db_path)?;
            let tx = rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate)
                .map_err(|_|"protection_issuance_lock_failed")?;
            let fresh = fresh_protection_response_on(&tx,context,claim,witness,written,&response,&stop)?;
            let actor = &witness.actor;
            let proof = private_historical_web_mode_in(&tx,&actor.scan_id,actor.attempt_number,&actor.root_run_id,None)?;
            let issuer = crate::agent_runtime::web_mode::protection_issuer::ProtectionIssuer::from_verified_birth_on(
                &tx,&actor.root_run_id,proof)?;
            issuer.issue_new_on(&tx,&fresh)?;
            let fact:JsonValue = serde_json::from_str(&fresh.raw).map_err(|_|"protection_issuance_fact_invalid")?;
            let receipt = fact["receiptId"].as_str().ok_or("protection_issuance_receipt_missing")?.to_string();
            // Strict first-response source verification is a production
            // issuance postcondition, not a paid frame or an execution grant.
            let verified = native_original_protection_response_on(&tx,actor,&receipt)?;
            let bounded = verified.bounded_fact();
            if bounded["receiptId"] != receipt || bounded["executionGrant"] != false
                || bounded["targetRequestsGranted"] != 0 || bounded["closedAuthorVerified"] != false {
                return Err("protection_issuance_source_postcondition_failed".into());
            }
            for row in verified.verification_rows() { row.verify(&tx)?; }
            tx.commit().map_err(|_|"protection_issuance_commit_failed")?;
            Ok(receipt)
        })();
        match issued {
            Ok(receipt) => {
                result["protectionArtifactProvenance"] = "original_signed_protection_response_receipt".into();
                result["protectionReceiptId"] = receipt.into();
            }
            Err(_) => { result["protectionIssuance"] = "failed".into(); }
        }
    }
    result
}
