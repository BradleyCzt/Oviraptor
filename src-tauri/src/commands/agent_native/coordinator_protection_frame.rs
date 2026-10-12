// Only this producer returns a paid protection frame together with the original
// SDK/whole-execution ownership guards. Cloning a frame does not clone a grant.
impl NativeCoordinatorFrame {
    fn protection_closed_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        receipt_id: &str,
    ) -> Result<(Self, NativeClosedProtectionResponse), String> {
        if db.is_autocommit() {
            return Err("root_protection_frame_snapshot_required".into());
        }
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        native_protection_supervision_protocol_on(db, actor)?;
        let closed = native_closed_protection_response_on(db, &root, actor, receipt_id)?;
        let frame = Self {
            kind: "protection-changed",
            semantic: closed.bounded_fact(),
            rows: closed.verification_rows().to_vec(),
            review_refs: None,
            review_meta: None,
        };
        frame.verify_protection_source(db, &root, actor)?;
        Ok((frame, closed))
    }

    // The foreground caller already owns both original guards. Re-probing here
    // would reject that legitimate owner, so only the original historical audit
    // runs again before SDK and around actual semantic publication.
    fn verify_protection_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_protection_source(&snapshot, context, actor);
        }
        if self.kind != "protection-changed"
            || self.review_refs.is_some()
            || self.review_meta.is_some()
            || self.semantic["closedAuthorVerified"] != true
            || self.semantic["executionGrant"] != false
            || self.semantic["targetRequestsGranted"] != 0
            || self.semantic.get("closedAuthor").is_none()
        {
            return Err("root_protection_frame_original_source_required".into());
        }
        native_protection_supervision_protocol_on(db, actor)?;
        let receipt_id = native_protection_text(&self.semantic, "receiptId")?;
        let audited = native_audit_closed_protection_response_on(db, context, actor, receipt_id)?;
        let current = Self {
            kind: "protection-changed",
            semantic: audited.bounded,
            rows: audited.rows,
            review_refs: None,
            review_meta: None,
        };
        if self.fact() != current.fact() {
            return Err("root_protection_frame_original_fact_changed".into());
        }
        for row in &self.rows {
            row.verify(db)?;
        }
        Ok(())
    }
}

// A creation contract is necessary, but it never substitutes for the signed
// response, closed author, original costs or current Coordinator authority.
fn native_protection_supervision_protocol_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<JsonValue, String> {
    if db.is_autocommit() {
        return Err("root_protection_frame_snapshot_required".into());
    }
    let declaration = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("root_protection_original_birth_missing")?;
    let protocol = declaration
        .protection_supervision()
        .ok_or("root_protection_original_protocol_missing")?;
    if declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi
        || protocol
            != json!({"schemaVersion":1,
            "source":"original_signed_protection_response_receipt",
            "closure":"original_cost_settled_saved_and_acknowledged",
            "deduplication":"original_response_physical_fact",
            "permittedStep":"assess:protection_changed","executionGrant":false})
    {
        return Err("root_protection_original_protocol_invalid".into());
    }
    Ok(protocol)
}
