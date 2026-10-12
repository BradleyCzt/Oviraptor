use super::*;

pub(crate) fn receipt_for(connection: &Connection, id: &str) -> Result<Option<Value>, String> {
    let record:Option<(i64,String,String,Value)>=connection.query_row(
        "SELECT sequence,receipt_json,created_at,receipt_id,revision,draft_hash,scan_id,attempt_number,
         root_run_id,target_key,thread_key,kind,argument_hash FROM agent_directive_human_reviews WHERE draft_id=?1",
        [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,json!({"receiptId":r.get::<_,String>(3)?,
            "draftId":id,"revision":r.get::<_,i64>(4)?,"draftHash":r.get::<_,String>(5)?,
            "scanId":r.get::<_,String>(6)?,"attemptNumber":r.get::<_,i64>(7)?,"rootRunId":r.get::<_,String>(8)?,
            "targetKey":r.get::<_,String>(9)?,"threadKey":r.get::<_,String>(10)?,"kind":r.get::<_,String>(11)?,
            "argumentHash":r.get::<_,String>(12)?})))).optional().map_err(|_|"directive_human_review_read_failed")?;
    record
        .map(|(sequence, raw, time, binding)| {
            let mut value: Value = serde_json::from_str(&raw)
                .map_err(|_| "directive_human_review_receipt_unverified")?;
            if binding
                .as_object()
                .unwrap()
                .iter()
                .any(|(key, wanted)| value[key] != *wanted)
            {
                return Err("directive_human_review_receipt_unverified".into());
            }
            let draft = draft_store::load_draft(connection, id)?
                .ok_or("directive_human_review_receipt_unverified")?;
            if !draft_store::draft_integrity_valid(&draft)
                || value["revision"] != draft.revision
                || value["draftHash"] != draft.draft_hash
                || value["scanId"] != draft.scan_id
                || value["attemptNumber"] != draft.attempt_number
                || value["rootRunId"] != draft.root_run_id
                || value["targetKey"] != draft.target_key
                || value["threadKey"] != draft.thread_key
            {
                return Err("directive_human_review_receipt_unverified".into());
            }
            let state = match value["kind"].as_str() {
                Some("approve") => "confirmed",
                Some("revise") => "cancelled",
                Some("reject") => "rejected",
                _ => return Err("directive_human_review_receipt_unverified".into()),
            };
            if draft.status != state
                || (state == "confirmed" && value["directiveId"] != draft.confirmed_directive_id)
            {
                return Err("directive_human_review_receipt_unverified".into());
            }
            verify_decision(connection, &draft, &value)?;
            value["sequence"] = json!(sequence);
            value["createdAt"] = json!(time);
            Ok(value)
        })
        .transpose()
}

fn verify_decision(
    db: &Connection,
    draft: &HumanDirectiveDraft,
    value: &Value,
) -> Result<(), String> {
    let invalid = "directive_human_review_receipt_unverified";
    let kind = value["kind"].as_str().ok_or(invalid)?;
    let reason = value["reason"].as_str().ok_or(invalid)?;
    let id = if kind == "approve" {
        draft.confirmed_directive_id.as_str()
    } else {
        ""
    };
    if value.as_object().is_none_or(|v| v.len() != 20)
        || value["receiptId"]
            .as_str()
            .is_none_or(|v| uuid::Uuid::parse_str(v).is_err())
        || value["terminal"] != true
        || value["executionCompleted"] != false
        || value["directiveId"] != id
        || value["actions"] != json!(actions(draft, kind, id))
    {
        return Err(invalid.into());
    }
    if kind == "revise" {
        let next = value["successorDraftId"].as_str().ok_or(invalid)?;
        let child = draft_store::load_draft(db, next)?.ok_or(invalid)?;
        if child.id == draft.id
            || child.source_message_id == draft.source_message_id
            || Some(child.revision) != draft.revision.checked_add(1)
            || child.draft_hash == draft.draft_hash
            || !draft_store::draft_integrity_valid(&child)
            || child.scan_id != draft.scan_id
            || child.attempt_number != draft.attempt_number
            || child.root_run_id != draft.root_run_id
            || child.target_key != draft.target_key
            || child.thread_key != draft.thread_key
            || child.recipient_role != draft.recipient_role
            || value["successorRevision"] != child.revision
            || value["successorHash"] != child.draft_hash
            || value["requiresConfirmation"] != child.confirmation_required
            || !reason.is_empty()
            || value["argumentHash"] != safe_argument_hash(kind, &child.text)
        {
            return Err(invalid.into());
        }
    } else if !value["successorDraftId"].is_null()
        || !value["successorRevision"].is_null()
        || !value["successorHash"].is_null()
        || value["requiresConfirmation"] != false
        || (kind == "approve" && (id.is_empty() || !reason.is_empty()))
        || (kind == "reject" && (reason.trim().is_empty() || reason.chars().count() > 500))
        || value["argumentHash"] != safe_argument_hash(kind, reason)
    {
        return Err(invalid.into());
    }
    Ok(())
}

pub(super) fn argument_hash(connection: &Connection, root: &str, kind: &str, argument: &str) -> Result<String, String> {
    let safe = crate::agent_runtime::multi_agent::homogeneous::material_policy::text(connection, root, argument.trim())?;
    Ok(safe_argument_hash(kind, &safe))
}

// Persisted text has already been redacted. Re-redacting its markers changes
// their fingerprint and would invalidate an otherwise original receipt.
fn safe_argument_hash(kind: &str, safe: &str) -> String {
    crate::agent_runtime::store::stable_hash(&json!([kind, safe]).to_string())
}

pub(super) fn record(
    connection: &Connection,
    draft: &HumanDirectiveDraft,
    kind: &str,
    argument: &str,
    directive_id: &str,
    successor: Option<&HumanDirectiveDraft>,
) -> Result<Value, String> {
    let actions = actions(draft, kind, directive_id);
    let receipt = json!({"receiptId":uuid::Uuid::new_v4().to_string(),"kind":kind,
        "draftId":draft.id,"revision":draft.revision,"draftHash":draft.draft_hash,
        "scanId":draft.scan_id,"attemptNumber":draft.attempt_number,"rootRunId":draft.root_run_id,
        "targetKey":draft.target_key,"threadKey":draft.thread_key,
        "argumentHash":argument_hash(connection, &draft.root_run_id, kind, argument)?,
        "reason":if kind=="reject" {crate::agent_runtime::multi_agent::homogeneous::material_policy::text(connection, &draft.root_run_id, argument.trim())?} else {String::new()},
        "directiveId":directive_id,"successorDraftId":successor.map(|d|d.id.as_str()),
        "successorRevision":successor.map(|d|d.revision),"successorHash":successor.map(|d|d.draft_hash.as_str()),
        "requiresConfirmation":successor.is_some_and(|d|d.confirmation_required),
        "terminal":true,"executionCompleted":false,"actions":actions});
    let raw = receipt.to_string();
    let changed=connection.execute("INSERT INTO agent_directive_human_reviews
        (receipt_id,draft_id,revision,draft_hash,scan_id,attempt_number,root_run_id,target_key,thread_key,kind,argument_hash,receipt_json)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![receipt["receiptId"].as_str(),draft.id,draft.revision,draft.draft_hash,draft.scan_id,
            draft.attempt_number,draft.root_run_id,draft.target_key,draft.thread_key,kind,
            receipt["argumentHash"].as_str(),raw]).map_err(|_|"directive_human_review_not_persisted")?;
    let exact: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_directive_human_reviews
        WHERE draft_id=?1 AND revision=?2 AND draft_hash=?3 AND scan_id=?4 AND attempt_number=?5
        AND root_run_id=?6 AND target_key=?7 AND thread_key=?8 AND kind=?9 AND receipt_json=?10
        AND receipt_id=?11 AND argument_hash=?12)",
            params![
                draft.id,
                draft.revision,
                draft.draft_hash,
                draft.scan_id,
                draft.attempt_number,
                draft.root_run_id,
                draft.target_key,
                draft.thread_key,
                kind,
                raw,
                receipt["receiptId"].as_str(),
                receipt["argumentHash"].as_str()
            ],
            |r| r.get(0),
        )
        .map_err(|_| "directive_human_review_not_persisted")?;
    if changed != 1 || !exact {
        return Err("directive_human_review_not_persisted".into());
    }
    receipt_for(connection, &draft.id)?.ok_or_else(|| "directive_human_review_not_persisted".into())
}

fn actions(draft: &HumanDirectiveDraft, kind: &str, directive_id: &str) -> Vec<Value> {
    draft
        .requested_roles
        .iter()
        .enumerate()
        .map(|(index, role)| {
            let other_roles = draft
                .requested_roles
                .iter()
                .filter(|r| r.as_str() != "coordinator")
                .count();
            // This is an immutable review action plan, never an execution receipt.
            let reason = if kind != "approve" {
                "human_decision_terminal"
            } else if draft.readonly_assessment_plan.as_ref().is_some_and(|p| p.schema_version == 2) {
                super::super::ordered_plan::NOT_CONNECTED
            } else if draft.intent == "agent_proposal_request" && other_roles != 1
                && !draft.readonly_assessment_plan.as_ref().is_some_and(|p|
                    p.schema_version == 3 && p.dispatch_state == "requires_dispatch_checks")
            {
                "proposal_role_decomposition_required"
            } else if draft.intent == "agent_proposal_request" && role == "evidence_reviewer" {
                "proposal_reviewer_requires_frozen_candidate"
            } else if draft.intent == "agent_proposal_request"
                && !matches!(
                    role.as_str(),
                    "coordinator" | "spa_api_mapper" | "deep_investigator"
                )
            {
                "proposal_role_decomposition_required"
            } else {
                ""
            };
            json!({"order":index+1,"role":role,"intent":draft.intent,
            "reviewDisposition":if kind=="approve" {"coordinator_queued"} else {"not_queued"},
            "capabilityState":if reason.is_empty() {"existing_policy_required"} else {"blocked"},
            "reasonCode":reason,"executionState":"not_started","directiveId":directive_id,
            "executionReceipt":Value::Null})
        })
        .collect()
}
