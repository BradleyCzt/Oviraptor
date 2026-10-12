// REM-A04 U175: frozen InputParser trigger frame (clears `freeze_bound_basis`
// for InputParser only). The frame freezes the U174 Rust trigger decision and
// hash-binds it to the exact verified Broker facts it cites. It is a readonly
// basis, never a dispatch: it grants no child, budget, capability or target
// request, it stays on the reserved `input-parser-output` kind, and
// `native_coordinator_prepare_input_parser` stays not_ready while the remaining
// InputParser prepare (a) blockers are open.
const NATIVE_INPUT_PARSER_FRAME_FACT_SQL: &str =
    "SELECT rowid,id,root_run_id,kind,provenance,revision,payload_json,artifact_refs_json
     FROM agent_evidence_nodes WHERE id=?1 AND root_run_id=?2 AND kind='request_record'
     AND provenance='observed'";

/// Per cited fact: the verified Broker record/body hashes the decision rests on.
#[cfg_attr(not(test), allow(dead_code))]
fn native_input_parser_frame_bindings(
    db: &rusqlite::Connection,
    root_run_id: &str,
    target_dir: &Path,
    evidence_refs: &[String],
) -> Result<JsonValue, String> {
    use crate::agent_runtime::evidence_graph::store::load_evidence_node;
    let mut bindings = Vec::with_capacity(evidence_refs.len());
    for id in evidence_refs {
        let node = load_evidence_node(db, id)?.ok_or("root_input_parser_frame_fact_missing")?;
        if node.root_run_id != root_run_id
            || !verified_web_http_review_fact(target_dir, &node.artifact_refs, &node.payload)
        {
            return Err("root_input_parser_frame_fact_invalid".into());
        }
        bindings.push(json!({"evidenceRef":node.id,"artifactRefs":node.artifact_refs,
            "recordSha256":node.payload["recordSha256"],"bodySha256":node.payload["bodySha256"],
            "requestId":node.payload["requestId"]}));
    }
    Ok(JsonValue::Array(bindings))
}

#[cfg_attr(not(test), allow(dead_code))]
fn native_input_parser_frame_decision_hash(trigger: &JsonValue, bindings: &JsonValue) -> String {
    crate::agent_runtime::store::stable_hash(&json!({"trigger":trigger,"factBindings":bindings}).to_string())
}

impl NativeCoordinatorFrame {
    /// Readonly: freezes an eligible trigger for this actor's Root; never writes.
    #[cfg_attr(not(test), allow(dead_code))]
    fn input_parser_trigger(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        target_dir: &Path,
    ) -> Result<Self, String> {
        let decision = native_input_parser_trigger_decision(db, &actor.root_run_id, target_dir)?;
        if !decision.eligible {
            return Err("root_input_parser_trigger_not_eligible".into());
        }
        let trigger = serde_json::to_value(&decision).map_err(|e| e.to_string())?;
        let bindings =
            native_input_parser_frame_bindings(db, &actor.root_run_id, target_dir, &decision.evidence_refs)?;
        let rows = decision
            .evidence_refs
            .iter()
            .map(|id| {
                NativeCoordinatorFrozenRows::capture(
                    db,
                    NATIVE_INPUT_PARSER_FRAME_FACT_SQL,
                    &[id.as_str(), actor.root_run_id.as_str()],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            kind: "input-parser-output",
            semantic: json!({"role":decision.role,"rootRunId":actor.root_run_id,
                "scanId":actor.scan_id,"targetKey":actor.target_key,
                "decisionHash":native_input_parser_frame_decision_hash(&trigger,&bindings),
                "trigger":trigger,"factBindings":bindings,
                "targetRequestsGranted":0,"dispatchGranted":false}),
            rows,
            review_refs: None,
            review_meta: None,
        })
    }

    /// Re-derives the decision from current verified facts; any drift fails closed.
    #[cfg_attr(not(test), allow(dead_code))]
    fn verify_input_parser_trigger_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        let semantic = &self.semantic;
        if semantic["role"] != crate::agent_runtime::contract::AgentRole::InputParser.as_str()
            || semantic["rootRunId"] != actor.root_run_id
            || semantic["scanId"] != actor.scan_id
            || semantic["targetKey"] != actor.target_key
            || semantic["targetRequestsGranted"] != 0
            || semantic["dispatchGranted"] != false
            || self.rows.len() != semantic["trigger"]["evidenceRefs"].as_array().map_or(0, Vec::len)
        {
            return Err("root_input_parser_frame_scope_invalid".into());
        }
        let decision =
            native_input_parser_trigger_decision(db, &actor.root_run_id, &context.target_dir)?;
        let trigger = serde_json::to_value(&decision).map_err(|e| e.to_string())?;
        if !decision.eligible || trigger != semantic["trigger"] {
            return Err("root_input_parser_trigger_changed".into());
        }
        let bindings = native_input_parser_frame_bindings(
            db,
            &actor.root_run_id,
            &context.target_dir,
            &decision.evidence_refs,
        )?;
        if bindings != semantic["factBindings"]
            || semantic["decisionHash"] != native_input_parser_frame_decision_hash(&trigger, &bindings)
        {
            return Err("root_input_parser_frame_binding_changed".into());
        }
        Ok(())
    }
}
