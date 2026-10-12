use super::*;

fn defer(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    id: &str,
    reason: &str,
) -> Result<(), String> {
    transition_directive_in_transaction(transaction, lease, id, "accepted", "deferred")?;
    transaction
        .execute(
            "UPDATE agent_user_directives SET rejection_code=?1 WHERE id=?2",
            params![reason, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// At most one job per round. Reuse the *stored* snapshot after restart, never
/// silently replace it with evidence from a later round. No target tools exist.
#[cfg(test)]
pub fn prepare_next(
    connection: &Connection,
    lease: &CoordinatorLease,
    evidence: &Value,
) -> Result<Option<ProposalJob>, String> {
    prepare_next_authorized(connection, lease, evidence, |_| Ok(()))
}

pub(crate) fn prepare_next_authorized(
    connection: &Connection, lease: &CoordinatorLease, evidence: &Value,
    authorize: impl Fn(&Connection) -> Result<(), String>,
) -> Result<Option<ProposalJob>, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    authorize(&transaction)?;
    let items = prepare_model_context_in_transaction(&transaction, lease)?;
    let existing: Option<String> = transaction.query_row(
        "SELECT p.directive_id FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id \
         WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.target_key=?3 AND d.root_run_id=?4 \
         AND d.claim_lease_epoch=?5 AND d.claim_fencing_token=?6 AND d.status='assigned' \
         AND p.state IN ('prepared','received') ORDER BY d.rowid LIMIT 1",
        params![lease.scan_id,lease.attempt_number,lease.target_key,lease.root_run_id,lease.lease_epoch,lease.fencing_token],
        |row|row.get(0),
    ).optional().map_err(|e|e.to_string())?;
    if let Some(id) = existing {
        let job =
            load_job(&transaction, lease, &id)?.ok_or("directive_proposal_binding_invalid")?;
        authorize(&transaction)?;
        transaction.commit().map_err(|e| e.to_string())?;
        return Ok(Some(job));
    }
    for item in items {
        let source: String = transaction
            .query_row(
                "SELECT source_draft_id FROM agent_user_directives WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let draft = load_draft(&transaction, &source)?.ok_or("directive_draft_missing")?;
        if draft.intent != "agent_proposal_request" {
            continue;
        }
        if let Some(plan) = &draft.readonly_assessment_plan {
            if plan.schema_version == 3 { continue; }
            defer(&transaction, lease, &item.id, super::super::ordered_plan::NOT_CONNECTED)?;
            continue;
        }
        let roles: Vec<_> = draft
            .requested_roles
            .iter()
            .filter(|role| role.as_str() != "coordinator")
            .collect();
        let role = match roles.as_slice() {
            [role] if role.as_str() == "spa_api_mapper" => AgentRole::SpaApiMapper,
            [role] if role.as_str() == "deep_investigator" => AgentRole::DeepInvestigator,
            [role] if role.as_str() == "evidence_reviewer" => {
                defer(
                    &transaction,
                    lease,
                    &item.id,
                    "proposal_reviewer_requires_frozen_candidate",
                )?;
                continue;
            }
            _ => {
                defer(
                    &transaction,
                    lease,
                    &item.id,
                    "proposal_role_decomposition_required",
                )?;
                continue;
            }
        };
        if draft.side_effect_class != "read_only"
            || !draft.requested_contracts.is_empty()
            || draft.proposed_scope_change.is_some()
        {
            defer(
                &transaction,
                lease,
                &item.id,
                "proposal_contract_not_read_only",
            )?;
            continue;
        }
        // Old confirmed drafts did not expose this child reservation. They
        // cannot acquire a larger action by reinterpretation after upgrading.
        if draft.estimated_tokens != PROPOSAL_TOKENS || draft.estimated_requests != 1 {
            defer(
                &transaction,
                lease,
                &item.id,
                "proposal_reconfirmation_required",
            )?;
            continue;
        }
        let mut input = redact_json(&json!({
            "kind":"human_requested_assessment", "directiveId":item.id,
            "target":lease.target_key,"request":item.text,"frozenEvidence":evidence,
            "referencedFactIds":draft.referenced_fact_ids,
            "requiredOutput":{"summary":"string","suggestions":["string"],"limitations":["string"]},
            "constraints":{"webOnly":true,"targetRequests":0,"tools":[],"advisoryOnly":true},
        }));
        // Freeze a clearly labelled excerpt when the evidence bundle is large.
        // Do not stop the whole task, pretend to have sent the full bundle, or
        // feed a broken JSON fragment as if it were structured verified facts.
        if input.to_string().len() > 2_600 {
            let full = input["frozenEvidence"].to_string();
            let mut excerpt = full.chars().take(1_024).collect::<String>();
            input["frozenEvidence"] = json!({"format":"json_text_excerpt","truncated":true,
                "sourceBytes":full.len(),"sourceSha256":crate::agent_runtime::store::stable_hash(&full),
                "excerpt":""});
            loop {
                input["frozenEvidence"]["excerpt"] = excerpt.clone().into();
                if input.to_string().len() <= 2_600 || excerpt.pop().is_none() {
                    break;
                }
            }
        }
        // Leave room for system/output. An oversized instruction itself must
        // be decomposed/reconfirmed; never silently rewrite the human request.
        if input.to_string().len() > 2_600 {
            defer(
                &transaction,
                lease,
                &item.id,
                "proposal_context_requires_compaction",
            )?;
            continue;
        }
        let reviewer_room: bool = transaction.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM agent_assignments a WHERE a.coordinator_run_id=?1 \
               AND a.role='web_executor' AND a.state IN ('leased','running','waiting_review')) \
             OR EXISTS(SELECT 1 FROM agent_budget_ledger b WHERE b.root_run_id=?1 \
               AND (b.total_tokens=0 OR b.total_tokens-b.spent_tokens-b.reserved_tokens>=?2+?3) \
               AND (b.total_requests=0 OR b.total_requests-b.spent_requests-b.reserved_requests>=2))",
            params![lease.root_run_id,PROPOSAL_TOKENS,crate::agent_runtime::multi_agent::client_side::REVIEWER_FLOOR_TOKENS],|r|r.get(0),
        ).map_err(|e|e.to_string())?;
        if !reviewer_room {
            defer(
                &transaction,
                lease,
                &item.id,
                "proposal_budget_reserved_for_review",
            )?;
            continue;
        }
        // The savepoint rolls back a partial schedule when capacity is absent;
        // the outer transaction still persists the truthful deferred receipt.
        transaction
            .execute_batch("SAVEPOINT proposal_schedule")
            .map_err(|e| e.to_string())?;
        let scheduled = scheduler::schedule_child_in_transaction(
            &transaction,
            lease,
            role,
            AgentLane::ReadOnlyAnalysis,
            &format!("human_directive:{}", item.id),
            &input,
            draft.revision,
            &[
                "evidence.read".into(),
                "mailbox.read".into(),
                "mailbox.write".into(),
            ],
            PROPOSAL_TOKENS,
            1,
        );
        let child = match scheduled {
            Ok(child) => child,
            Err(error)
                if error == "agent_lane_occupied"
                    || error == "child_budget_reservation_exceeded_or_stale" =>
            {
                transaction
                    .execute_batch("ROLLBACK TO proposal_schedule; RELEASE proposal_schedule")
                    .map_err(|e| e.to_string())?;
                defer(
                    &transaction,
                    lease,
                    &item.id,
                    if error == "agent_lane_occupied" {
                        "proposal_lane_unavailable"
                    } else {
                        "proposal_budget_unavailable"
                    },
                )?;
                continue;
            }
            Err(error) => return Err(error),
        };
        transaction
            .execute_batch("RELEASE proposal_schedule")
            .map_err(|e| e.to_string())?;
        let message = mailbox::send(
            &transaction,
            lease,
            &lease.root_run_id,
            &child.run_id,
            "coordinator",
            role.as_str(),
            "human_assessment_request",
            &item.id,
            &child.assignment_id,
            draft.revision,
            &input,
        )?;
        transaction.execute("INSERT INTO agent_directive_proposals(directive_id,assignment_id,child_run_id,request_message_id) VALUES(?1,?2,?3,?4)",
            params![item.id,child.assignment_id,child.run_id,message]).map_err(|e|e.to_string())?;
        transition_directive_in_transaction(&transaction, lease, &item.id, "accepted", "assigned")?;
        let job =
            load_job(&transaction, lease, &item.id)?.ok_or("directive_proposal_binding_invalid")?;
        authorize(&transaction)?;
        transaction.commit().map_err(|e| e.to_string())?;
        return Ok(Some(job));
    }
    authorize(&transaction)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(None)
}
