use super::*;
pub(crate) fn prepare_next(
    db: &Connection,
    scope: &CoordinatorLease,
    evidence: &Value,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<Option<ActionJob>, String> {
    let private = writer::private_connection(db, Mode::Schedule)?;
    let db = &private;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    current(&tx, scope)?;
    check(&tx)?;
    let ids: Vec<String> = {
        let mut q=tx.prepare("SELECT id FROM agent_user_directives WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3
      AND target_key=?4 AND claim_run_id=?1 AND claim_lease_epoch=?5 AND claim_fencing_token=?6 AND status IN ('accepted','assigned')
      AND json_extract(payload_json,'$.readonlyAssessmentPlan.schemaVersion')=3 ORDER BY rowid").map_err(|e|e.to_string())?;
        let result = q
            .query_map(
                params![
                    scope.root_run_id,
                    scope.scan_id,
                    scope.attempt_number,
                    scope.target_key,
                    scope.lease_epoch,
                    scope.fencing_token
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        result
    };
    if let Some(id) = ids.into_iter().next() {
        let (draft, original, state) = source(&tx, &id)?;
        if original != *scope_with_empty_deadline(scope) {
            return Err("ordered_original_c_changed".into());
        }
        validate_thread_key(
            &tx,
            &scope.root_run_id,
            &scope.target_key,
            &draft.thread_key,
        )?;
        if !fact_refs_current(&tx, &scope.root_run_id, &draft.referenced_fact_ids)? {
            return Err("ordered_fact_reference_not_current".into());
        }
        let previous = load(&tx, &id, 1)?;
        let order = if let Some(previous) = &previous {
            if !matches!(previous.state.as_str(), "completed" | "failed") {
                let job = previous.clone();
                check(&tx)?;
                current(&tx, scope)?;
                tx.commit().map_err(|e| e.to_string())?;
                return Ok(Some(job));
            }
            if previous.state == "failed" {
                return Err("ordered_failed_predecessor_cannot_advance".into());
            }
            receipts::verified(&tx, previous)?.ok_or("ordered_predecessor_receipt_missing")?;
            if let Some(job) = load(&tx, &id, 2)? {
                check(&tx)?;
                current(&tx, scope)?;
                tx.commit().map_err(|e| e.to_string())?;
                return Ok(Some(job));
            }
            2
        } else {
            if state != "accepted" {
                return Err("ordered_first_checkpoint_missing".into());
            }
            1
        };
        let plan = draft
            .readonly_assessment_plan
            .as_ref()
            .ok_or("ordered_plan_missing")?;
        let action = &plan.actions[(order - 1) as usize];
        let input = input(&tx, &id, &draft, order, evidence, previous.as_ref())?;
        // The original total is a ceiling, not new root capacity. Preserve the
        // existing Web Reviewer's floor and check each fresh dispatch again.
        let need = if order == 1 { 8000 } else { 4000 };
        let requests = if order == 1 { 2 } else { 1 };
        let capacity = match budget::root::require_child_capacity(&tx, &scope.root_run_id, need, requests) {
            Ok(()) => true,
            Err(error) if error == "child_budget_reservation_exceeded_or_stale" => false,
            Err(error) => return Err(error),
        };
        let room:bool=tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND role='web_executor'
          AND state IN ('leased','running','waiting_review')) OR EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1
          AND (total_tokens=0 OR total_tokens-spent_tokens-reserved_tokens>=?2+?4)
          AND (total_requests=0 OR total_requests-spent_requests-reserved_requests>=?3+1))",
          params![scope.root_run_id,need,requests,crate::agent_runtime::multi_agent::client_side::REVIEWER_FLOOR_TOKENS],|r|r.get(0)).map_err(|e|e.to_string())?;
        let role = match action.role.as_str() {
            "spa_api_mapper" => AgentRole::SpaApiMapper,
            "deep_investigator" => AgentRole::DeepInvestigator,
            _ => return Err("ordered_role_unimplemented".into()),
        };
        let trigger = format!("human_ordered:{id}:{order}");
        let child = predicted(scope, role, &trigger, draft.revision);
        if !capacity || !room {
            if order != 1 {
                return Err(if !capacity { "child_budget_reservation_exceeded_or_stale" }
                    else { "ordered_budget_reserved_for_review" }.into());
            }
            // No action/worker was created. Defer only this confirmed fresh
            // plan atomically, without ending the original executor or pool.
            let guard = Writer::install(&private, scope, &id, &child, Mode::Defer)?;
            let changed = tx.execute("UPDATE agent_user_directives SET status='deferred',rejection_code='ordered_budget_unavailable',
                finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status='accepted'", [&id])
                .map_err(|e| e.to_string())?;
            if changed != 1 { return Err("ordered_defer_not_persisted".into()); }
            guard.verify()?;
            check(&tx)?;
            current(&tx, scope)?;
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(None);
        }
        let guard = Writer::install(&private, scope, &id, &child, Mode::Schedule)?;
        let scheduled = scheduler::schedule_child_in_transaction(
            &tx,
            scope,
            role,
            AgentLane::ReadOnlyAnalysis,
            &trigger,
            &input,
            draft.revision,
            &[
                "evidence.read".into(),
                "mailbox.read".into(),
                "mailbox.write".into(),
            ],
            4000,
            1,
        )?;
        if scheduled != child {
            return Err("ordered_schedule_identity_changed".into());
        }
        let message = mailbox::send(
            &tx,
            scope,
            &scope.root_run_id,
            &child.run_id,
            "coordinator",
            role.as_str(),
            "human_ordered_assessment_request",
            &action.action_id,
            &child.assignment_id,
            draft.revision,
            &input,
        )?;
        let changed=tx.execute("INSERT INTO agent_directive_ordered_actions(action_id,directive_id,action_order,plan_hash,previous_action_id,assignment_id,child_run_id,request_message_id,input_json)
          VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![action.action_id,id,order,plan.plan_hash,action.previous_action_id.clone().unwrap_or_default(),child.assignment_id,child.run_id,message,input.to_string()]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("ordered_schedule_not_persisted".into());
        }
        if order == 1 {
            transition_directive_in_transaction(&tx, scope, &id, "accepted", "assigned")?
        }
        let job = load(&tx, &id, order)?.ok_or("ordered_schedule_not_persisted")?;
        if job.child != child || job.input != input || job.state != "prepared" {
            return Err("ordered_schedule_not_persisted".into());
        }
        event_exact(&tx, &job, "orderedState", &json!("prepared"))?;
        guard.verify()?;
        check(&tx)?;
        current(&tx, scope)?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(Some(job));
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(None)
}
fn scope_with_empty_deadline(scope: &CoordinatorLease) -> Box<CoordinatorLease> {
    let mut scope = scope.clone();
    scope.lease_expires_at.clear();
    Box::new(scope)
}
fn predicted(
    scope: &CoordinatorLease,
    role: AgentRole,
    trigger: &str,
    revision: i64,
) -> scheduler::ScheduledChild {
    let dedup = format!(
        "{}:{}:{}:{}",
        role.as_str(),
        trigger,
        scope.target_key,
        revision
    );
    let assignment_id = format!(
        "asg-{}",
        &stable_hash(&format!("{}:{dedup}", scope.root_run_id))[..24]
    );
    scheduler::ScheduledChild {
        run_id: format!("run-{assignment_id}"),
        assignment_id,
        role,
    }
}
fn excerpt(value: &Value, max: usize) -> Value {
    let raw = value.to_string();
    if raw.len() <= max {
        return value.clone();
    }
    json!({"format":"json_text_excerpt","truncated":true,"sourceBytes":raw.len(),"sourceSha256":stable_hash(&raw),
      "excerpt":raw.chars().take(max/2).collect::<String>()})
}
fn input(
    db: &Connection,
    id: &str,
    draft: &HumanDirectiveDraft,
    order: i64,
    evidence: &Value,
    previous: Option<&ActionJob>,
) -> Result<Value, String> {
    let plan = draft
        .readonly_assessment_plan
        .as_ref()
        .ok_or("ordered_plan_missing")?;
    let action = &plan.actions[(order - 1) as usize];
    let mut input = json!({"kind":"human_ordered_readonly_assessment","directiveId":id,"actionId":action.action_id,"order":order,
      "planHash":plan.plan_hash,"target":draft.target_key,"request":draft.safe_execution_text,"referencedFactIds":draft.referenced_fact_ids,
      "frozenEvidence":if let Some(previous)=previous {previous.input["frozenEvidence"].clone()} else {excerpt(&crate::agent_runtime::secrets::redact_json(evidence),512)},
      "previousAssessment":Value::Null,"requiredOutput":{"summary":"string","suggestions":["string"],"limitations":["string"]},
      "constraints":{"webOnly":true,"targetRequests":0,"tools":[],"advisoryOnly":true,"independentReviewApproved":false}});
    if let Some(previous) = previous {
        let receipt =
            receipts::verified(db, previous)?.ok_or("ordered_predecessor_receipt_missing")?;
        if receipt["outcome"] != "valid_advisory" {
            return Err("ordered_predecessor_not_valid".into());
        }
        input["previousAssessment"] = json!({"receiptId":receipt["receiptId"],"receiptHash":stable_hash(&receipt.to_string()),
          "actionId":previous.action.action_id,"advisoryOnly":true,"reviewApproved":false,"assessment":excerpt(&previous.response,512)});
    }
    if input.to_string().len() > 2600 {
        return Err("ordered_context_requires_reconfirmation".into());
    }
    Ok(input)
}
