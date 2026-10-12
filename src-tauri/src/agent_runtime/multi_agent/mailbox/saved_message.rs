use super::*;
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts::AssignmentAttempt, scheduler::ScheduledChild},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn verify(
    db: &Connection,
    lease: &CoordinatorLease,
    worker: &AssignmentAttempt,
    from: &str,
    to: &str,
    from_role: &str,
    to_role: &str,
    kind: &str,
    correlation: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    if !matches!(
        worker.state.as_str(),
        "running" | "paused" | "completed" | "failed" | "expired"
    ) {
        return Err("mailbox_saved_worker_state_conflict".into());
    }
    let role = AgentRole::try_parse(if from == lease.root_run_id {
        to_role
    } else {
        from_role
    })
    .ok_or("mailbox_saved_role_invalid")?;
    let child = ScheduledChild {
        assignment_id: worker.assignment_id.clone(),
        run_id: worker.child_run_id.clone(),
        role,
    };
    if kind == "human_ordered_assessment_result" {
        let (job, expected) = super::super::directive::ordered_execution::result_for_mailbox(db, lease, correlation)?;
        if job.child != child || from != child.run_id || to != lease.root_run_id || *payload != expected {
            return Err("mailbox_saved_ordered_mismatch".into());
        }
        return Ok(());
    }
    if kind == "human_assessment_result" {
        let job = super::super::directive::proposals::result_for_mailbox(db, lease, correlation)?;
        let expected = serde_json::json!({"directiveId":job.directive_id,"summary":job.response["summary"],
            "assessment":job.response,"advisoryOnly":true,"coverageVerified":false,"targetRequests":0});
        if job.child != child
            || from != child.run_id
            || to != lease.root_run_id
            || *payload != expected
        {
            return Err("mailbox_saved_proposal_mismatch".into());
        }
        return Ok(());
    }
    if role == AgentRole::SpaApiMapper
        && crate::agent_runtime::web_mode::root::read(db, &lease.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15)
    {
        let output = super::super::specialist::mapper_loop::verified_original_output_on(db, lease, &child)?;
        if worker.state == "expired" {
            super::super::specialist::mapper_loop::expired_saved::read_closed_original_on(db, lease, &child)?;
        } else {
            super::super::specialist::mapper_loop::require_settled_output_on(db, lease, &child, &output)?;
        }
        if from != child.run_id || to != lease.root_run_id || to_role != "coordinator"
            || kind != "evidence_summary" || correlation != format!("mapper:{}", child.assignment_id)
            || *payload != serde_json::json!({"summary":output.text()})
        {
            return Err("mailbox_saved_mapper_loop_original_mismatch".into());
        }
        return Ok(());
    }
    let received = super::super::specialist::received_for_local_delivery(db, lease, &child)?;
    if !received.rejection.is_empty() {
        return Err("mailbox_saved_response_rejected".into());
    }
    let parsed = serde_json::from_str::<JsonValue>(&received.text).ok();
    if role == AgentRole::EvidenceReviewer && kind == "review_decision" {
        let bound: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_review_requests WHERE id=?1
            AND root_run_id=?2 AND assignment_id=?3 AND reviewer_run_id=?4 AND candidate_id=?5
            AND candidate_revision=?6 AND lease_epoch=?7 AND fencing_token=?8)",
                params![
                    correlation,
                    lease.root_run_id,
                    child.assignment_id,
                    child.run_id,
                    payload["candidateId"].as_str(),
                    payload["candidateRevision"].as_i64(),
                    lease.lease_epoch,
                    lease.fencing_token
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !bound {
            return Err("mailbox_saved_review_scope_conflict".into());
        }
    }
    if role == AgentRole::DeepInvestigator && matches!(kind, "gap_proposed" | "proposal_assessed") {
        let raw: String = db
            .query_row(
                "SELECT task_slice_json FROM agent_assignments WHERE id=?1
            AND child_run_id=?2 AND trigger_code='reviewer_insufficient_evidence'",
                params![child.assignment_id, child.run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let task: JsonValue = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let candidate = task["candidateId"]
            .as_str()
            .ok_or("mailbox_saved_gap_scope_invalid")?;
        let revision = task["revision"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or("mailbox_saved_gap_scope_invalid")?;
        if payload["candidateId"] != task["candidateId"]
            || payload["evidenceRevision"] != revision
            || correlation != format!("review-gap:{candidate}:{revision}")
        {
            return Err("mailbox_saved_gap_scope_conflict".into());
        }
    }
    let valid = match (role, kind) {
        (
            AgentRole::SpaApiMapper
            | AgentRole::RepoMapper
            | AgentRole::SourceAnalyst
            | AgentRole::ExternalSurface,
            "evidence_summary",
        )
        | (AgentRole::IdentitySession, "identity_assessment") => {
            from == child.run_id && payload["summary"].as_str() == Some(received.text.as_str())
        }
        (AgentRole::BusinessLogic, "evidence_summary") => {
            super::super::budget::root::RootOwner::load_original(db, &lease.root_run_id)?
                .require_original_coordinator(db, lease)?;
            let directory = crate::commands::resolve_agent_evidence_location(db, &lease.root_run_id)?;
            let collection = crate::commands::native_business_logic_output_collection_on(
                db, lease, &child, &directory,
            )?;
            from == child.run_id
                && to == lease.root_run_id
                && to_role == "coordinator"
                && correlation == format!("business-logic:{}", child.assignment_id)
                && *payload == collection.fact()["payload"]
        }
        (AgentRole::ClientSide, "evidence_summary") => {
            let task = super::super::client_side::assignment(db, lease, &child)?;
            super::super::client_side::validate_assessment(&received.text, &task)?;
            let frozen = serde_json::to_value(&task).map_err(|_| "client_side_task_invalid")?;
            from == child.run_id
                && to == lease.root_run_id
                && to_role == "coordinator"
                && correlation == format!("client-side:{}", child.assignment_id)
                && payload.as_object().is_some_and(|v| v.len() == 2)
                && payload["summary"].as_str() == Some(received.text.as_str())
                && payload["clientSideTask"] == frozen
        }
        (AgentRole::InputParser, "input_assessment") => {
            // U178: the saved InputParser result stays inside the readonly surface
            // it was minted for. Re-bind the surface to this lease and the paid
            // frame decision hash, then re-check the strict parameter schema.
            let slice_text: String = db
                .query_row(
                    "SELECT task_slice_json FROM agent_assignments WHERE id=?1 AND child_run_id=?2",
                    params![child.assignment_id, child.run_id],
                    |r| r.get(0),
                )
                .map_err(|_| "input_parser_assignment_invalid")?;
            let slice: JsonValue =
                serde_json::from_str(&slice_text).map_err(|_| "input_parser_task_invalid")?;
            let decision_hash = slice["frame"]["decisionHash"]
                .as_str()
                .ok_or("input_parser_task_scope_invalid")?;
            let task = super::super::input_parser::Task::from_value(
                &payload["inputParserTask"],
                lease,
                decision_hash,
            )?;
            super::super::input_parser::validate_assessment(&received.text, &task)?;
            from == child.run_id
                && to == lease.root_run_id
                && to_role == "coordinator"
                && correlation == format!("input-parser:{}", child.assignment_id)
                && payload.as_object().is_some_and(|v| v.len() == 2)
                && payload["summary"].as_str() == Some(received.text.as_str())
        }
        (AgentRole::EvidenceReviewer, "source_review_result") => {
            from == child.run_id
                && super::super::source_reviewer::receipt_payload(db, lease, &child)?.0 == *payload
        }
        (AgentRole::EvidenceReviewer, "source_coverage_review_result") => {
            from == child.run_id
                && super::super::source_coverage_reviewer::receipt_payload(db, lease, &child)?.0
                    == *payload
        }
        (AgentRole::EvidenceReviewer, "review_decision") => {
            from == child.run_id
                && parsed.as_ref().is_some_and(|r| {
                    r["summary"] == payload["summary"] && r["verdict"] == payload["verdict"]
                })
        }
        (AgentRole::DeepInvestigator, "gap_proposed") => {
            from == child.run_id
                && parsed
                    .as_ref()
                    .is_some_and(|r| r["summary"] == payload["summary"])
        }
        (AgentRole::DeepInvestigator, "proposal_assessed") => {
            from == lease.root_run_id
                && payload["targetRequestsGranted"] == 0
                && payload["decision"] == "deferred_requires_new_evidence_revision"
                && payload["schemaVersion"] == 3
        }
        _ => false,
    };
    if !valid {
        return Err("mailbox_saved_result_mismatch".into());
    }
    Ok(())
}
