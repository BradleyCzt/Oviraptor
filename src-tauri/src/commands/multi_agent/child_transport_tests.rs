#[cfg(test)]
thread_local! {
    static EXPIRE_RECEIVED_REVIEWER_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EXPIRE_RECEIVED_DEEP_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EXPIRE_RECEIVED_IDENTITY_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EXPIRE_RECEIVED_MAPPER_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static REAL_CHILD_TRANSPORT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn multi_agent_child_round(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
) -> Result<(String, AgentTokenUsage), String> {
    if REAL_CHILD_TRANSPORT.with(std::cell::Cell::get) {
        let received = multi_agent_child_round_transport(context, lease, child, system, input)?;
        let expire = match child.role {
            crate::agent_runtime::contract::AgentRole::SpaApiMapper => EXPIRE_RECEIVED_MAPPER_ONCE.with(|flag|flag.replace(false)),
            crate::agent_runtime::contract::AgentRole::IdentitySession => EXPIRE_RECEIVED_IDENTITY_ONCE.with(|flag|flag.replace(false)),
            crate::agent_runtime::contract::AgentRole::DeepInvestigator => EXPIRE_RECEIVED_DEEP_ONCE.with(|flag|flag.replace(false)),
            crate::agent_runtime::contract::AgentRole::EvidenceReviewer => EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag|flag.replace(false)),
            _ => false,
        };
        if expire {
            let db = db::open(&context.db_path)?;
            let tx = db.unchecked_transaction().map_err(|e|e.to_string())?;
            let original = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(&tx,lease,child)?;
            if original.text != received.0 || original.usage != received.1 || !original.rejection.is_empty() {
                return Err("test_original_mapper_received_mismatch".into());
            }
            tx.commit().map_err(|e|e.to_string())?;
            let count = db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1 AND assignment_id=?2", params![child.run_id,child.assignment_id]).map_err(|e|e.to_string())?;
            if count != 1 { return Err("test_original_mapper_worker_missing".into()); }
            let deadline = Instant::now()+Duration::from_secs(5);
            while crate::agent_runtime::multi_agent::attempts::current(&db,lease,&child.assignment_id)?.state != "expired" {
                context.supervision.as_ref().ok_or("test_original_supervisor_missing")?.check()?;
                if Instant::now()>=deadline {return Err("test_original_mapper_expiry_timeout".into());}
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        return Ok(received);
    }
    use crate::agent_runtime::contract::AgentRole;
    let text = match child.role {
        AgentRole::SpaApiMapper => serde_json::json!({
            "summary": "test mapper inspected frozen evidence",
            "priorityContracts": [],
            "risks": []
        }),
        AgentRole::IdentitySession => serde_json::json!({
            "summary": "test identity specialist reviewed scoped identity metadata",
            "observedAuthentication": [],
            "evidenceGaps": [],
            "authorizationProven": false
        }),
        AgentRole::ExternalSurface => serde_json::json!({
            "summary": "test public entry assessment", "observations": [],
            "coverageGaps": ["single anonymous entry only"], "confirmedFindings": false
        }),
        AgentRole::DeepInvestigator => {
            assert!(input.get("frozenEvidence").is_none());
            assert!(input["frozenCandidate"]["evidence"]
                .get("volatileOnly")
                .is_none());
            serde_json::json!({
            "summary": "test investigator assessed the reviewer's frozen evidence gap",
            "nextStep": "request_new_contract",
            "gapCode":"missing_owner_control", "supportingFactRefs":[],
            "missingEvidence":input["reviewerMissingEvidence"], "prerequisites":["operator control group"],
            "proposedContracts":["new_attempt_control_group_request"],
            "expectedInformationGain":0.5, "impactCeiling":"medium",
            "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only", "overlapKeys":["owner-control"],
            "falsificationCondition":"owner access absent", "stopCondition":"scope rejection"
            })
        }
        AgentRole::EvidenceReviewer => {
            if input.pointer("/evidence/forceReviewerInsufficientForTest")
                == Some(&JsonValue::Bool(true))
            {
                serde_json::json!({
                    "verdict":"insufficient_evidence",
                    "reasonCodes":["fixture_missing_control"],
                    "missingEvidence":["ownership control"],
                    "confidence":0.0,
                    "summary":"test reviewer requested a verified control"
                })
            } else if input.pointer("/evidence/forceReviewerRejectedForTest")
                == Some(&JsonValue::Bool(true))
            {
                serde_json::json!({
                    "verdict":"rejected",
                    "reasonCodes":["fixture_counterevidence"],
                    "missingEvidence":[],
                    "confidence":0.9,
                    "summary":"test reviewer rejected the frozen candidate bundle"
                })
            } else {
                serde_json::json!({
                    "verdict": "confirmed",
                    "reasonCodes": ["test_fixture_confirmed"],
                    "missingEvidence": [],
                    "confidence": 1.0,
                    "summary": "test reviewer confirmed the frozen candidate bundle"
                })
            }
        }
        _ => return Err("unsupported_test_child_role".into()),
    };
    let usage = AgentTokenUsage::default();
    record_child_model_round(context, child, &usage, &text.to_string())?;
    Ok((text.to_string(), usage))
}

#[cfg(test)]
fn record_child_model_round(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    summary: &str,
) -> Result<(), String> {
    let ledger = AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: child.run_id.clone(),
    };
    ledger.model_round(1, usage, &[])?;
    let connection = db::open(&context.db_path)?;
    let mut snapshot = crate::agent_runtime::checkpoint::RunState::new(&child.run_id, Vec::new());
    snapshot.turns = 1;
    snapshot.model_requests = usage.model_requests.max(0);
    snapshot.target_requests =
        crate::agent_runtime::target_requests::child_received(&connection, &child.run_id)?;
    snapshot.input_tokens = usage.input_tokens.max(0);
    snapshot.cached_input_tokens = usage.cached_input_tokens.max(0);
    snapshot.output_tokens = usage.output_tokens.max(0);
    snapshot.used_tokens = usage.total_tokens.max(0);
    snapshot.progress_signature = crate::agent_runtime::store::stable_hash(summary);
    crate::agent_runtime::checkpoint::write_checkpoint(&connection, &snapshot)
}
