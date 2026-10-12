#[allow(clippy::too_many_arguments)]
fn validate_child_contract(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), String> {
    super::source::validate_surface_role(connection, lease, role)?;
    if role == AgentRole::Coordinator {
        return Err("child_role_must_not_be_coordinator".into());
    }
    // A caller cannot grant target tools to a review/read-only child simply
    // by choosing another lane or passing an arbitrary capability string.
    if (role == AgentRole::EvidenceReviewer) != (lane == AgentLane::Review) {
        return Err("reviewer_requires_exclusive_review_lane".into());
    }
    if role.requires_read_only_analysis_lane() && lane != AgentLane::ReadOnlyAnalysis {
        return Err("analysis_specialist_requires_read_only_lane".into());
    }
    if role == AgentRole::DeepInvestigator && lane != AgentLane::ReadOnlyAnalysis {
        return Err("deep_investigator_requires_read_only_lane".into());
    }
    if role == AgentRole::WebExecutor && lane != AgentLane::TargetTouching {
        return Err("web_executor_requires_target_lane".into());
    }
    if role == AgentRole::Authorization && lane != AgentLane::TargetTouching {
        return Err("authorization_requires_target_lane".into());
    }
    if role == AgentRole::ExternalSurface && lane != AgentLane::TargetTouching {
        return Err("external_surface_requires_target_lane".into());
    }
    let allowed: Option<&[&str]> = match role {
        AgentRole::EvidenceReviewer => Some(&["evidence.read", "review.write"][..]),
        AgentRole::SpaApiMapper => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        AgentRole::IdentitySession => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        AgentRole::ClientSide => Some(&["evidence.read", "mailbox.write"][..]),
        AgentRole::InputParser => Some(&["evidence.read", "mailbox.write"][..]),
        AgentRole::BusinessLogic => Some(&["evidence.read", "mailbox.write"][..]),
        // Source validates the complete frozen phase/capability contract below.
        AgentRole::RepoMapper | AgentRole::SourceAnalyst => None,
        AgentRole::DeepInvestigator => {
            Some(&["evidence.read", "mailbox.read", "mailbox.write"][..])
        }
        AgentRole::WebExecutor => {
            // The executor's tool set is additionally checked per invocation by
            // the Broker against the exact lease and current fencing token.
            None
        }
        AgentRole::Authorization => Some(&["authorization_probe"][..]),
        AgentRole::ExternalSurface => {
            Some(&["public_surface_get", "evidence.read", "mailbox.write"][..])
        }
        AgentRole::Upload => return Err("upload_role_not_implemented".into()),
        AgentRole::Concurrency => return Err("concurrency_role_not_implemented".into()),
        _ => return Err("specialist_role_not_implemented".into()),
    };
    if let Some(allowed) = allowed {
        if capabilities
            .iter()
            .any(|capability| !allowed.contains(&capability.as_str()))
        {
            return Err("role_capability_not_allowed".into());
        }
    }
    if super::source::is_source_role(role) {
        super::source::validate_scheduled_slice(
            connection,
            lease,
            role,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(connection, lease, role)?
    {
        super::source_review_subject::validate_slice(
            connection,
            lease,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::ClientSide {
        super::client_side::validate_scheduled(task_slice, lease, evidence_revision, capabilities)?;
    }
    if role == AgentRole::IdentitySession {
        let (mode, _) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        if mode == crate::auth_session::ScanIdentityMode::AnonymousOnly {
            return Err("identity_session_requires_bound_identity".into());
        }
    }
    if role == AgentRole::Authorization {
        if capabilities != ["authorization_probe"] {
            return Err("authorization_capability_invalid".into());
        }
        let (mode, identities) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        let (count, invalid): (i64, i64) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(CASE WHEN method!='GET' OR owner_identity=tester_identity THEN 1 ELSE 0 END),0) \
             FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3",
            params![lease.scan_id, lease.attempt_number, lease.target_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| "authorization_control_lookup_failed".to_string())?;
        if mode != crate::auth_session::ScanIdentityMode::IdentitySet
            || count == 0
            || count > 4
            || invalid != 0
            || reserved_tokens != 0
            // No model invocation: target requests are claimed by their own
            // broker and never reserved from the model-request ledger.
            || reserved_requests != 0
        {
            return Err("authorization_control_or_budget_invalid".into());
        }
        let unbound: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 \
             AND (owner_identity NOT IN (SELECT value FROM json_each(?4)) OR tester_identity NOT IN (SELECT value FROM json_each(?4))))",
            params![lease.scan_id, lease.attempt_number, lease.target_key, serde_json::json!(identities).to_string()],
            |row| row.get(0),
        ).map_err(|_| "authorization_control_identity_lookup_failed".to_string())?;
        if unbound {
            return Err("authorization_control_identity_unbound".into());
        }
    }
    Ok(())
}
