// A03: capture one real Broker observation without modifying original frontend evidence.
// Admission stays not_ready until the production hook and runtime acceptance land.
const NATIVE_NEW_EVIDENCE_FACT_SQL: &str =
    "SELECT rowid,* FROM agent_evidence_nodes WHERE id=?1 AND root_run_id=?2";

impl NativeCoordinatorFrame {
    #[cfg_attr(not(test), allow(dead_code))]
    fn new_http_evidence(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        target_dir: &Path,
        evidence_id: &str,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::new_http_evidence(&tx, actor, target_dir, evidence_id);
        }
        use crate::agent_runtime::evidence_graph::{
            contract::{EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, load_evidence_node},
        };
        let node = load_evidence_node(db, evidence_id)?.ok_or("root_new_evidence_missing")?;
        if node.root_run_id != actor.root_run_id
            || node.kind != EvidenceNodeKind::RequestRecord
            || node.provenance != EvidenceProvenance::Observed
            || node.revision <= 0
            || node.created_at.is_empty()
            || !node.supersedes_id.is_empty()
            || !verified_web_http_review_fact(target_dir, &node.artifact_refs, &node.payload)
        {
            return Err("root_new_evidence_source_invalid".into());
        }
        let artifact = node.artifact_refs[0]
            .strip_prefix("agent-http/")
            .ok_or("root_new_evidence_source_invalid")?;
        if node.natural_key_hash.len() != 64
            || !node.natural_key_hash.bytes().all(|b| b.is_ascii_hexdigit())
            || node.id != format!("ev-{}", &node.natural_key_hash[..32])
            || node.natural_key_hash
                != evidence_natural_key(
                    &actor.root_run_id,
                    EvidenceNodeKind::RequestRecord,
                    &format!("{}:{artifact}", node.created_by_run_id),
                )
        {
            return Err("root_new_evidence_identity_invalid".into());
        }
        // Evidence remains attributable after its producer closes; it never grants
        // the old child a new lease. Root authority is checked separately by Tick.
        let author: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a
             ON a.id=r.assignment_id AND a.child_run_id=r.id
             WHERE r.id=?1 AND r.root_run_id=?2 AND r.parent_run_id=?2
             AND r.scan_id=?3 AND r.attempt_number=?4 AND r.target_url=?5
             AND r.role='web_executor' AND r.lane='target_touching'
             AND r.orchestration_policy='multi' AND a.coordinator_run_id=?2
             AND a.role=r.role AND a.lane=r.lane AND a.target_key=?5
             AND a.evidence_revision=?6 AND a.lease_epoch=?7 AND a.fencing_token=?8)",
                params![
                    node.created_by_run_id,
                    actor.root_run_id,
                    actor.scan_id,
                    actor.attempt_number,
                    actor.target_key,
                    node.revision,
                    actor.lease_epoch,
                    actor.fencing_token
                ],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !author {
            return Err("root_new_evidence_author_invalid".into());
        }
        let mut claim_rows = native_new_evidence_claim_rows(db, actor, target_dir, &node)?;
        claim_rows.push(NativeCoordinatorFrozenRows::capture(
            db,
            NATIVE_NEW_EVIDENCE_FACT_SQL,
            &[evidence_id, &actor.root_run_id],
        )?);
        let binding = json!({"evidenceId":node.id,"revision":node.revision,
            "authorRunId":node.created_by_run_id,"naturalKeyHash":node.natural_key_hash,
            "artifactRefs":node.artifact_refs,"payload":node.payload});
        Ok(Self {
            kind: "new-evidence",
            semantic: json!({"rootRunId":actor.root_run_id,"scanId":actor.scan_id,
                "attemptNumber":actor.attempt_number,"targetKey":actor.target_key,
                "bindingHash":crate::agent_runtime::store::stable_hash(&binding.to_string()),
                "binding":binding,"dispatchGranted":false,"targetRequestsGranted":0}),
            rows: claim_rows,
            review_refs: None,
            review_meta: None,
        })
    }

    fn verify_new_evidence_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        let id = self.semantic["binding"]["evidenceId"]
            .as_str()
            .ok_or("root_new_evidence_binding_invalid")?;
        let current = if self.semantic.get("closedAuthor").is_some() {
            Self::new_closed_http_evidence(db, context, actor, id)?
        } else {
            Self::new_http_evidence(db, actor, &context.target_dir, id)?
        };
        if self.fact() != current.fact() || self.review_refs.is_some() || self.review_meta.is_some()
        {
            return Err("root_new_evidence_binding_changed".into());
        }
        Ok(())
    }
}
