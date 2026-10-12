use super::{
    fact_refs_current,
    interpretation::explicit_host_boundary_request,
    source_guidance, HumanDirectiveDraft,
};
use crate::agent_runtime::secrets::redact_text_with;
#[cfg(test)]
use rusqlite::TransactionBehavior;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn create_draft(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    root_run_id: &str,
    target_key: &str,
    recipient_role: &str,
    text: &str,
    bound_lease_epoch: i64,
    bound_fencing_token: &str,
) -> Result<HumanDirectiveDraft, String> {
    create_draft_in_thread(
        connection,
        scan_id,
        attempt_number,
        root_run_id,
        target_key,
        recipient_role,
        text,
        "team",
        bound_lease_epoch,
        bound_fencing_token,
    )
}

pub(super) fn validate_thread_key(
    connection: &Connection,
    root_run_id: &str,
    target_key: &str,
    thread_key: &str,
) -> Result<(), String> {
    if thread_key == "team" {
        return Ok(());
    }
    if thread_key.is_empty() || thread_key.chars().count() > 200 || root_run_id.is_empty() {
        return Err("directive_thread_not_in_current_root".into());
    }
    if thread_key == target_key || thread_key == format!("coordinator:{root_run_id}") {
        return Ok(());
    }
    let owned: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_messages WHERE root_run_id=?1 AND correlation_id=?2 AND correlation_id<>'') \
         OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND id=?2)",
        params![root_run_id, thread_key], |row| row.get(0),
    ).map_err(|error| format!("directive_thread_lookup_failed:{error}"))?;
    if owned {
        Ok(())
    } else {
        Err("directive_thread_not_in_current_root".into())
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn create_draft_in_thread(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    root_run_id: &str,
    target_key: &str,
    recipient_role: &str,
    text: &str,
    thread_key: &str,
    bound_lease_epoch: i64,
    bound_fencing_token: &str,
) -> Result<HumanDirectiveDraft, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令草案：{error}"))?;
    let draft = create_draft_in_transaction(
        &transaction,
        scan_id,
        attempt_number,
        root_run_id,
        target_key,
        recipient_role,
        text,
        thread_key,
        bound_lease_epoch,
        bound_fencing_token,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交用户指令草案：{error}"))?;
    Ok(draft)
}

/// Caller resolves the recipient inside this same write transaction, before
/// drafting. Neither another target nor a new attempt can race the binding.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_draft_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    scan_id: &str,
    attempt_number: i64,
    root_run_id: &str,
    target_key: &str,
    recipient_role: &str,
    text: &str,
    thread_key: &str,
    bound_lease_epoch: i64,
    bound_fencing_token: &str,
) -> Result<HumanDirectiveDraft, String> {
    create_revision_in_transaction(
        transaction,
        scan_id,
        attempt_number,
        root_run_id,
        target_key,
        recipient_role,
        text,
        thread_key,
        bound_lease_epoch,
        bound_fencing_token,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_revision_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    scan_id: &str,
    attempt_number: i64,
    root_run_id: &str,
    target_key: &str,
    recipient_role: &str,
    text: &str,
    thread_key: &str,
    bound_lease_epoch: i64,
    bound_fencing_token: &str,
    revision: i64,
) -> Result<HumanDirectiveDraft, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 2_000 {
        return Err("directive_length_invalid".into());
    }
    validate_thread_key(transaction, root_run_id, target_key, thread_key)?;
    let plaintext = super::super::homogeneous::material_policy::for_root(transaction, root_run_id)?;
    let mut interpreted = super::interpretation::interpret_with_material(text, target_key, plaintext);
    let redacted = if plaintext { text.to_string() } else { redact_text_with(text, None) };
    source_guidance::interpret_focus(
        transaction,
        root_run_id,
        target_key,
        thread_key,
        text,
        &mut interpreted,
    )?;
    let fact_refs_valid =
        fact_refs_current(transaction, root_run_id, &interpreted.referenced_fact_ids)?;
    if !fact_refs_valid {
        interpreted.validation_result = "rejected".into();
        interpreted.coordinator_decision = "reject".into();
        interpreted.status = "rejected".into();
        interpreted.confirmation_required = false;
        interpreted.safe_execution_text.clear();
        interpreted
            .reason_codes
            .push("fact_reference_not_current".into());
    }
    if !root_run_id.is_empty() && !target_key.is_empty() && attempt_number > 0
        && bound_lease_epoch > 0 && !bound_fencing_token.is_empty()
    {
        super::ordered_plan::freeze_fresh_on(
            transaction, root_run_id, target_key, &mut interpreted, text,
        )?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let source_message_id = uuid::Uuid::new_v4().to_string();
    if revision < 1 {
        return Err("directive_revision_invalid".into());
    }
    let mut hash_material = json!({
        "sourceMessageId": source_message_id,
        "scanId": scan_id,
        "attemptNumber": attempt_number,
        "rootRunId": root_run_id,
        "targetKey": target_key,
        "recipientRole": recipient_role,
        "threadKey": thread_key,
        "text": redacted,
        "intent": interpreted.intent,
        "requestedRoles": interpreted.requested_roles,
        "referencedFactIds": interpreted.referenced_fact_ids,
        "requestedContracts": interpreted.requested_contracts,
        "priorityChanges": interpreted.priority_changes,
        "proposedScopeChange": interpreted.proposed_scope_change,
        "estimatedTokens": interpreted.estimated_tokens,
        "estimatedRequests": interpreted.estimated_requests,
        "sideEffectClass": interpreted.side_effect_class,
        "requiredApprovals": interpreted.required_approvals,
        "validationResult": interpreted.validation_result,
        "reasonCodes": interpreted.reason_codes,
        "coordinatorDecision": interpreted.coordinator_decision,
        "safeExecutionText": interpreted.safe_execution_text,
        "revision": revision,
        "boundLeaseEpoch": bound_lease_epoch,
        "boundFencingToken": bound_fencing_token,
    });
    super::ordered_plan::append_hash_material(&mut hash_material)?;
    let draft_hash = crate::agent_runtime::store::stable_hash(&hash_material.to_string());
    // A rejected host request must leave an auditable, non-executable boundary
    // record in the same commit as its chat draft. This is not an approval lane.
    let host_boundary_requested = explicit_host_boundary_request(text);
    transaction
        .execute(
            "INSERT INTO agent_directive_drafts(id,source_message_id,scan_id,attempt_number,root_run_id,target_key,recipient_role,text_redacted,intent,\
             requested_roles_json,referenced_fact_ids_json,requested_contracts_json,priority_changes_json,proposed_scope_change_json,\
             estimated_tokens,estimated_requests,side_effect_class,required_approvals_json,validation_result,reason_codes_json,\
             coordinator_decision,confirmation_required,safe_execution_text,revision,draft_hash,bound_lease_epoch,bound_fencing_token,status,thread_key) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29)",
            params![
                id,
                source_message_id,
                scan_id,
                attempt_number,
                root_run_id,
                target_key,
                recipient_role,
                redacted,
                interpreted.intent,
                json!(interpreted.requested_roles).to_string(),
                json!(interpreted.referenced_fact_ids).to_string(),
                json!(interpreted.requested_contracts).to_string(),
                json!(interpreted.priority_changes).to_string(),
                json!(interpreted.proposed_scope_change).to_string(),
                interpreted.estimated_tokens,
                interpreted.estimated_requests,
                interpreted.side_effect_class,
                json!(interpreted.required_approvals).to_string(),
                interpreted.validation_result,
                json!(interpreted.reason_codes).to_string(),
                interpreted.coordinator_decision,
                i64::from(interpreted.confirmation_required),
                interpreted.safe_execution_text,
                revision,
                draft_hash,
                bound_lease_epoch,
                bound_fencing_token,
                interpreted.status,
                thread_key,
            ],
        )
        .map_err(|error| format!("无法保存用户指令草案：{error}"))?;
    if host_boundary_requested {
        transaction.execute(
            "INSERT INTO agent_host_boundary_candidates(\
             id,scan_id,attempt_number,root_run_id,target_key,source_kind,source_id,source_draft_id,\
             evidence_fact_refs_json,summary_redacted)\
             VALUES(?1,?2,?3,?4,?5,'human_directive',?6,?6,?7,?8)",
            params![
                uuid::Uuid::new_v4().to_string(), scan_id, attempt_number, root_run_id,
                target_key, id,
                if fact_refs_valid { json!(interpreted.referenced_fact_ids).to_string() } else { "[]".into() },
                "用户提出跨越 Web 测试边界的主机操作；当前任务仅记录请求，不执行，也不授予主机权限。",
            ],
        ).map_err(|error| format!("无法记录主机边界候选：{error}"))?;
    }
    let draft = load_draft(transaction, &id)?
        .ok_or_else(|| "directive_draft_not_found_after_insert".to_string())?;
    if draft.draft_hash != draft_hash || !draft_integrity_valid(&draft) {
        return Err("directive_draft_integrity_failed_after_insert".into());
    }
    Ok(draft)
}

fn parse_string_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

fn draft_integrity_hash(draft: &HumanDirectiveDraft, include_thread: bool) -> String {
    let mut material = json!({
        "sourceMessageId": draft.source_message_id,
        "scanId": draft.scan_id,
        "attemptNumber": draft.attempt_number,
        "rootRunId": draft.root_run_id,
        "targetKey": draft.target_key,
        "recipientRole": draft.recipient_role,
        "text": draft.text,
        "intent": draft.intent,
        "requestedRoles": draft.requested_roles,
        "referencedFactIds": draft.referenced_fact_ids,
        "requestedContracts": draft.requested_contracts,
        "priorityChanges": draft.priority_changes,
        "proposedScopeChange": draft.proposed_scope_change,
        "estimatedTokens": draft.estimated_tokens,
        "estimatedRequests": draft.estimated_requests,
        "sideEffectClass": draft.side_effect_class,
        "requiredApprovals": draft.required_approvals,
        "validationResult": draft.validation_result,
        "reasonCodes": draft.reason_codes,
        "coordinatorDecision": draft.coordinator_decision,
        "safeExecutionText": draft.safe_execution_text,
        "revision": draft.revision,
        "boundLeaseEpoch": draft.bound_lease_epoch,
        "boundFencingToken": draft.bound_fencing_token,
    });
    if include_thread {
        material["threadKey"] = json!(draft.thread_key);
    }
    if super::ordered_plan::append_hash_material(&mut material).is_err() {
        return String::new();
    }
    crate::agent_runtime::store::stable_hash(&material.to_string())
}

pub(super) fn draft_integrity_valid(draft: &HumanDirectiveDraft) -> bool {
    draft.draft_hash == draft_integrity_hash(draft, true)
        // Existing drafts were written before thread binding. Migration gives
        // them the team default, but their original hash must remain valid.
        || (draft.thread_key == "team" && draft.draft_hash == draft_integrity_hash(draft, false))
}

pub(super) fn confirmed_payload(draft: &HumanDirectiveDraft) -> serde_json::Value {
    let mut payload = json!({
        "sourceDraftId": draft.id,
        "sourceMessageId": draft.source_message_id,
        "threadKey": draft.thread_key,
        "intent": draft.intent,
        "requestedRoles": draft.requested_roles,
        "referencedFactIds": draft.referenced_fact_ids,
        "requestedContracts": draft.requested_contracts,
        "priorityChanges": draft.priority_changes,
        "proposedScopeChange": draft.proposed_scope_change,
        "estimatedBudget": { "tokens": draft.estimated_tokens, "requests": draft.estimated_requests },
        "sideEffectClass": draft.side_effect_class,
        "requiredApprovals": draft.required_approvals,
        "validationResult": draft.validation_result,
        "reasonCodes": draft.reason_codes,
        "coordinatorDecision": draft.coordinator_decision,
        "safeExecutionText": draft.safe_execution_text,
        "revision": draft.revision,
        "draftHash": draft.draft_hash,
    });
    if let Some(plan) = &draft.readonly_assessment_plan {
        payload["readonlyAssessmentPlan"] = json!(plan);
    }
    payload
}

pub(super) fn load_draft(
    connection: &Connection,
    id: &str,
) -> Result<Option<HumanDirectiveDraft>, String> {
    connection
        .query_row(
            "SELECT id,source_message_id,scan_id,attempt_number,root_run_id,target_key,recipient_role,text_redacted,intent,\
             requested_roles_json,referenced_fact_ids_json,requested_contracts_json,priority_changes_json,proposed_scope_change_json,\
             estimated_tokens,estimated_requests,side_effect_class,required_approvals_json,validation_result,reason_codes_json,\
             coordinator_decision,confirmation_required,safe_execution_text,revision,draft_hash,bound_lease_epoch,bound_fencing_token,status,confirmed_directive_id,thread_key \
             FROM agent_directive_drafts WHERE id=?1",
            [id],
            |row| {
                let scope_json: String = row.get(13)?;
                Ok(HumanDirectiveDraft {
                    id: row.get(0)?,
                    source_message_id: row.get(1)?,
                    scan_id: row.get(2)?,
                    attempt_number: row.get(3)?,
                    root_run_id: row.get(4)?,
                    target_key: row.get(5)?,
                    recipient_role: row.get(6)?,
                    text: row.get(7)?,
                    intent: row.get(8)?,
                    requested_roles: parse_string_list(&row.get::<_, String>(9)?),
                    referenced_fact_ids: parse_string_list(&row.get::<_, String>(10)?),
                    requested_contracts: parse_string_list(&row.get::<_, String>(11)?),
                    priority_changes: parse_string_list(&row.get::<_, String>(12)?),
                    proposed_scope_change: serde_json::from_str::<Option<String>>(&scope_json)
                        .unwrap_or(None),
                    readonly_assessment_plan: None,
                    estimated_tokens: row.get(14)?,
                    estimated_requests: row.get(15)?,
                    side_effect_class: row.get(16)?,
                    required_approvals: parse_string_list(&row.get::<_, String>(17)?),
                    validation_result: row.get(18)?,
                    reason_codes: parse_string_list(&row.get::<_, String>(19)?),
                    coordinator_decision: row.get(20)?,
                    confirmation_required: row.get::<_, i64>(21)? != 0,
                    safe_execution_text: row.get(22)?,
                    revision: row.get(23)?,
                    draft_hash: row.get(24)?,
                    bound_lease_epoch: row.get(25)?,
                    bound_fencing_token: row.get(26)?,
                    status: row.get(27)?,
                    confirmed_directive_id: row.get(28)?,
                    thread_key: row.get(29)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("无法读取用户指令草案：{error}"))
        .and_then(|draft| draft.map(|mut draft| {
            super::ordered_plan::attach(&mut draft)?;
            Ok(draft)
        }).transpose())
}

pub fn list_open_drafts(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<Vec<HumanDirectiveDraft>, String> {
    let ids = {
        let mut statement = connection
            .prepare(
                "SELECT id FROM agent_directive_drafts \
                 WHERE scan_id=?1 AND attempt_number=?2 AND status IN ('drafted','need_confirmation') \
                 ORDER BY created_at,id",
            )
            .map_err(|error| format!("无法准备未确认指令草案查询：{error}"))?;
        let rows = statement
            .query_map(params![scan_id, attempt_number], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("无法查询未确认指令草案：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法解码未确认指令草案：{error}"))?;
        rows
    };
    ids.into_iter()
        .map(|id| {
            load_draft(connection, &id)?.ok_or_else(|| format!("directive_draft_disappeared:{id}"))
        })
        .collect()
}
