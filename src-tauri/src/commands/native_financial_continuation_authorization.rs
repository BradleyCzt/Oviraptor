// A private approval is created only from a newly verified server proposal.
// Neither a JSON invoice nor a UI remaining-balance object is a capability.
pub(crate) struct FinancialContinuationAuthorization {
    approval_id: String,
    proposal: JsonValue,
    source: crate::agent_runtime::multi_agent::budget::continuation::VerifiedFinancialSource,
    new_root: String,
    plan: AgentExecutionPlan,
}

impl FinancialContinuationAuthorization {
    fn approve_in(
        db: &rusqlite::Connection,
        scan: &str,
        attempt: i64,
        root: &str,
        receipt: &str,
        proposal_hash: &str,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{multi_agent::budget::continuation, store};
        let proposal = native_financial_continuation_preview_in(db, scan, attempt, root, receipt)?;
        if proposal["proposalHash"].as_str() != Some(proposal_hash) {
            return Err("financial_continuation_proposal_changed".into());
        }
        // Separate historical Roots are not evidence of one grant. A proven
        // descendant chain can be added without silently combining budgets.
        if proposal["historicalRootsChecked"] != 1 {
            return Err("financial_continuation_grant_chain_unproven".into());
        }
        let source = continuation::settled_source_on(db, root, receipt)?;
        let facts = source.as_json();
        let original_mode = private_web_mode_on(db, scan, attempt)?;
        let policy: String = db
            .query_row(
                "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
                [scan],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let policy: JsonValue = serde_json::from_str(&policy)
            .map_err(|_| "financial_continuation_original_policy_invalid")?;
        if store::stable_hash(&policy.to_string()) != original_mode.fact().policy_hash {
            return Err("financial_continuation_original_policy_changed".into());
        }
        let raw: String = db
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let value: JsonValue =
            serde_json::from_str(&raw).map_err(|_| "financial_continuation_plan_invalid")?;
        let mut plan =
            AgentExecutionPlan::from_json(&value).ok_or("financial_continuation_plan_invalid")?;
        plan.attempt_number = attempt
            .checked_add(1)
            .ok_or("financial_continuation_attempt_invalid")?;
        let ceiling = |key: &str| -> Result<i64, String> {
            match &facts[key] {
                JsonValue::Null => Ok(0),
                v => v
                    .as_i64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| "financial_continuation_budget_exhausted".into()),
            }
        };
        plan.hard_total_tokens = ceiling("sharedRemainingTokens")?;
        plan.hard_model_requests = ceiling("sharedRemainingRequests")?;
        if plan.hard_total_tokens > 0 {
            plan.soft_uncached_tokens = plan.soft_uncached_tokens.min(plan.hard_total_tokens);
        }
        if plan.hard_model_requests > 0 {
            plan.soft_model_requests = plan.soft_model_requests.min(plan.hard_model_requests);
        }
        for index in [0, 2, 3, 9] {
            if facts["remaining"][index].as_i64() == Some(0) {
                return Err("financial_continuation_budget_exhausted".into());
            }
        }
        let new_root = format!(
            "plan-{scan}-{}-{}",
            plan.attempt_number,
            &store::stable_hash(&plan.target_url)[..12]
        );
        Ok(Self {
            approval_id: Uuid::new_v4().to_string(),
            proposal,
            source,
            new_root,
            plan,
        })
    }

    pub(crate) fn source_facts(&self) -> JsonValue {
        self.source.as_json()
    }
    pub(crate) fn approval_id(&self) -> &str {
        &self.approval_id
    }
    pub(crate) fn new_root_id(&self) -> &str {
        &self.new_root
    }
    pub(crate) fn new_plan(&self) -> JsonValue {
        self.plan.as_json()
    }
    pub(crate) fn new_plan_hash(&self) -> String {
        self.plan.hash()
    }
    pub(crate) fn verified_financial_source_on(
        &self,
        db: &rusqlite::Connection,
    ) -> Result<
        crate::agent_runtime::multi_agent::budget::continuation::VerifiedFinancialSource,
        String,
    > {
        self.source.verify_on(db)?;
        crate::agent_runtime::multi_agent::budget::continuation::settled_source_on(
            db,
            self.proposal["rootRunId"]
                .as_str()
                .ok_or("financial_continuation_source_invalid")?,
            self.proposal["receiptId"]
                .as_str()
                .ok_or("financial_continuation_source_invalid")?,
        )
    }

    pub(crate) fn verify_new_root_on(
        &self,
        fresh: &crate::agent_runtime::store::NewlyInsertedNativeRoot<'_, '_>,
    ) -> Result<(), String> {
        if fresh.id() != self.new_root || fresh.transaction().is_autocommit() {
            return Err("financial_continuation_birth_scope_invalid".into());
        }
        let valid: bool = fresh.transaction().query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3
             AND target_url=?4 AND backend='native' AND role='coordinator' AND orchestration_policy='multi'
             AND root_run_id=id AND parent_run_id IS NULL AND assignment_id='' AND status='prepared'
             AND started_at='' AND finished_at='' AND cancel_requested_at='' AND used_tokens=0
             AND used_cached_tokens=0 AND used_requests=0 AND reserved_tokens=0 AND reserved_requests=0
             AND plan_json=?5 AND plan_hash=?6)",
            params![self.new_root, self.proposal["scanId"].as_str(), self.plan.attempt_number,
                self.plan.target_url, self.plan.as_json().to_string(), self.plan.hash()], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if !valid {
            return Err("financial_continuation_birth_scope_invalid".into());
        }
        Ok(())
    }

    fn publish_birth_in(
        &self,
        db: &rusqlite::Connection,
        files: &FinancialContinuationFiles,
    ) -> Result<JsonValue, String> {
        use crate::agent_runtime::store;
        self.source.verify_on(db)?;
        let project: i64 = db
            .query_row(
                "SELECT project_id FROM sentinel_scans WHERE id=?1",
                [self.proposal["scanId"].as_str()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let fact = json!({"schema":1,"approvalId":self.approval_id,"proposalHash":self.proposal["proposalHash"],
            "scanId":self.proposal["scanId"],"projectId":project,"sourceAttemptNumber":self.proposal["attemptNumber"],
            "newAttemptNumber":self.plan.attempt_number,"sourceRootRunId":self.proposal["rootRunId"],
            "newRootRunId":self.new_root,"sourceReceiptId":self.proposal["receiptId"],
            "financialSource":self.source_facts(),"newPlanHash":self.plan.hash(),
            "newPlanPhysicalHash":store::stable_hash(&self.plan.as_json().to_string()),
            "originalDeadline":self.source_facts()["originalDeadline"],"proposal":self.proposal,
            "taskDescription":self.proposal["taskDescription"],"sourceTargetDir":files.source_target,
            "newTargetDir":files.target,"frontendEvidenceHash":files.evidence_hash});
        let raw = fact.to_string();
        let count = db.execute("INSERT INTO agent_financial_continuations(approval_id,proposal_hash,scan_id,
            source_attempt_number,new_attempt_number,source_root_run_id,new_root_run_id,source_receipt_id,fact_json,fact_hash)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![self.approval_id, self.proposal["proposalHash"].as_str(), self.proposal["scanId"].as_str(),
                self.proposal["attemptNumber"].as_i64(), self.plan.attempt_number, self.proposal["rootRunId"].as_str(),
                self.new_root, self.proposal["receiptId"].as_str(), raw, store::stable_hash(&raw)])
            .map_err(|e| e.to_string())?;
        if count != 1 || verify_financial_continuation_birth_on(db, &self.new_root)? != fact {
            return Err("financial_continuation_birth_unconfirmed".into());
        }
        Ok(fact)
    }
}

/// Pure provenance, including expired source history. Never initializes an
/// owner, clock or lease, and never upgrades an existing Root.
pub(crate) fn verify_financial_continuation_birth_on(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{multi_agent::budget::continuation, store};
    if db.is_autocommit() {
        return Err("financial_continuation_requires_snapshot".into());
    }
    let (approval, proposal, scan, old_attempt, new_attempt, source, receipt, raw, hash):
        (String,String,String,i64,i64,String,String,String,String) = db.query_row(
        "SELECT approval_id,proposal_hash,scan_id,source_attempt_number,new_attempt_number,
         source_root_run_id,source_receipt_id,fact_json,fact_hash FROM agent_financial_continuations WHERE new_root_run_id=?1",
        [root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))
        .map_err(|_| "financial_continuation_birth_missing")?;
    let fact: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "financial_continuation_birth_invalid")?;
    let canonical = fact.to_string();
    let mut frozen_proposal = fact["proposal"].clone();
    frozen_proposal
        .as_object_mut()
        .ok_or("financial_continuation_birth_invalid")?
        .remove("proposalHash");
    if canonical != raw
        || store::stable_hash(&raw) != hash
        || Uuid::parse_str(&approval).is_err()
        || fact["schema"] != 1
        || fact["approvalId"] != approval
        || fact["proposalHash"] != proposal
        || fact["scanId"] != scan
        || fact["sourceAttemptNumber"] != old_attempt
        || fact["newAttemptNumber"] != new_attempt
        || fact["sourceRootRunId"] != source
        || fact["newRootRunId"] != root
        || fact["sourceReceiptId"] != receipt
        || store::stable_hash(&frozen_proposal.to_string()) != proposal
        || fact["proposal"]["proposalHash"] != proposal
        || fact["proposal"]["scanId"] != scan
        || fact["proposal"]["attemptNumber"] != old_attempt
        || fact["proposal"]["rootRunId"] != source
        || fact["proposal"]["receiptId"] != receipt
        || old_attempt <= 0
        || new_attempt
            != old_attempt
                .checked_add(1)
                .ok_or("financial_continuation_attempt_invalid")?
    {
        return Err("financial_continuation_birth_invalid".into());
    }
    let original = continuation::settled_source_history_on(db, &source, &receipt)?;
    if fact["financialSource"] != original.as_json()
        || fact["originalDeadline"] != original.as_json()["originalDeadline"]
    {
        return Err("financial_continuation_original_finance_changed".into());
    }
    if fact["proposal"]["financialSource"] != original.as_json()
        || fact["taskDescription"] != fact["proposal"]["taskDescription"]
    {
        return Err("financial_continuation_birth_invalid".into());
    }
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2
        AND attempt_number=?3 AND target_url=?4 AND backend='native' AND role='coordinator'
        AND parent_run_id IS NULL AND assignment_id='' AND root_run_id=id AND orchestration_policy='multi'
        AND plan_hash=?5)", params![root,scan,new_attempt,original.as_json()["targetKey"].as_str(),
            fact["newPlanHash"].as_str()], |r|r.get(0)).map_err(|e|e.to_string())?;
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !valid || fact["newPlanPhysicalHash"] != store::stable_hash(&plan) {
        return Err("financial_continuation_birth_scope_invalid".into());
    }
    Ok(fact)
}

fn native_financial_continuation_require_authority_in(
    db: &rusqlite::Connection,
    birth: &JsonValue,
) -> Result<(), String> {
    let p = &birth["proposal"];
    let scan = p["scanId"]
        .as_str()
        .ok_or("financial_continuation_birth_invalid")?;
    let old_attempt = p["attemptNumber"]
        .as_i64()
        .ok_or("financial_continuation_birth_invalid")?;
    let root = p["rootRunId"]
        .as_str()
        .ok_or("financial_continuation_birth_invalid")?;
    let original_live: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases
        WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND lease_epoch=?4 AND fencing_token=?5
        AND lease_expires_at>datetime('now','localtime'))",params![root,scan,old_attempt,
        birth["financialSource"]["coordinatorEpoch"].as_i64(),birth["financialSource"]["coordinatorFence"].as_str()],|r|r.get(0))
        .map_err(|e|e.to_string())?;
    if !original_live {
        return Err("financial_continuation_original_coordinator_expired_or_changed".into());
    }
    let mode = private_web_mode_on(db, scan, old_attempt)?;
    if mode.fact().receipt_id != p["originalModeReceiptId"] {
        return Err("financial_continuation_original_mode_changed".into());
    }
    if native_financial_continuation_model_on(db, root)? != p["modelBindingHash"] {
        return Err("financial_continuation_original_model_changed".into());
    }
    let target = p["targetKey"]
        .as_str()
        .ok_or("financial_continuation_birth_invalid")?;
    crate::auth_session::validated_scan_identities(db, scan, target)?;
    for (name, sql) in [
        (
            "context",
            "SELECT rowid,* FROM sentinel_scan_contexts WHERE scan_id=?1",
        ),
        (
            "sessions",
            "SELECT rowid,* FROM browser_auth_sessions WHERE owner_scan_id=?1 ORDER BY rowid",
        ),
    ] {
        if p["evidenceHashes"][name]
            != NativeCoordinatorFrozenRows::capture_optional(db, sql, &[scan])?.hash
        {
            return Err("financial_continuation_original_authority_changed".into());
        }
    }
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN projects p ON p.id=s.project_id
        WHERE s.id=?1 AND s.scan_type='web' AND s.status='scanning' AND s.attempt_count=?2 AND p.status='active' AND s.project_id=?3)
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan,birth["newAttemptNumber"].as_i64(),birth["projectId"].as_i64()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("financial_continuation_current_scope_invalid".into());
    }
    Ok(())
}
