use super::super::lease::{require_active_attempt, require_open_coordinator};
use super::{
    draft_store::{confirmed_payload, draft_integrity_valid, load_draft, validate_thread_key},
    fact_refs_current, UserDirective,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn confirm_draft(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    current_root_run_id: &str,
    current_target_key: &str,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
) -> Result<UserDirective, String> {
    confirm_draft_with_binding(
        connection,
        scan_id,
        Some((attempt_number, current_root_run_id, current_target_key)),
        draft_id,
        revision,
        draft_hash,
    )
}

/// The chat endpoint follows the immutable draft binding, not whichever target
/// happened to update most recently. Resolve and validate under the write lock.
pub fn confirm_bound_draft(
    connection: &Connection,
    scan_id: &str,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
) -> Result<UserDirective, String> {
    confirm_draft_with_binding(connection, scan_id, None, draft_id, revision, draft_hash)
}

fn confirm_draft_with_binding(
    connection: &Connection,
    scan_id: &str,
    binding: Option<(i64, &str, &str)>,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
) -> Result<UserDirective, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令草案：{error}"))?;
    let draft = load_draft(&transaction, draft_id)?
        .ok_or_else(|| "directive_draft_not_found".to_string())?;
    let (attempt_number, current_root_run_id, current_target_key) = match binding {
        Some(binding) => binding,
        None => {
            let attempt = transaction
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1 \
                 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
                    [scan_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "directive_scan_not_found".to_string())?;
            (
                attempt,
                draft.root_run_id.as_str(),
                draft.target_key.as_str(),
            )
        }
    };
    if draft.scan_id != scan_id || draft.attempt_number != attempt_number {
        return Err("directive_draft_scope_mismatch".into());
    }
    if draft.revision != revision || draft.draft_hash != draft_hash {
        return Err("directive_draft_stale_revision".into());
    }
    if !draft_integrity_valid(&draft) {
        return Err("directive_draft_integrity_failed".into());
    }
    if !draft.root_run_id.is_empty()
        && (draft.root_run_id != current_root_run_id || draft.target_key != current_target_key)
    {
        return Err("directive_draft_root_binding_changed".into());
    }
    if !fact_refs_current(
        &transaction,
        current_root_run_id,
        &draft.referenced_fact_ids,
    )? {
        return Err("directive_fact_reference_not_current".into());
    }
    if draft.status == "confirmed" {
        let directive = transaction
            .query_row(
                "SELECT id,text_redacted,status FROM agent_user_directives WHERE id=?1 AND source_draft_id=?2",
                params![draft.confirmed_directive_id, draft.id],
                |row| Ok(UserDirective { id: row.get(0)?, text: row.get(1)?, status: row.get(2)? }),
            )
            .optional()
            .map_err(|error| format!("无法读取已确认用户指令：{error}"))?
            .ok_or_else(|| "confirmed_directive_missing".to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(directive);
    }
    // Unbound historical/pre-run drafts remain readable, but must be recreated
    // with an explicit recipient before granting executable authority.
    if draft.root_run_id.is_empty()
        || draft.target_key.is_empty()
        || draft.bound_lease_epoch <= 0
        || draft.bound_fencing_token.is_empty()
    {
        return Err("directive_draft_recipient_not_bound".into());
    }
    require_active_attempt(&transaction, scan_id, attempt_number)?;
    require_open_coordinator(
        &transaction,
        scan_id,
        attempt_number,
        current_target_key,
        current_root_run_id,
    )?;
    if draft.status == "rejected" || draft.validation_result == "rejected" {
        return Err(format!(
            "directive_draft_rejected:{}",
            draft.reason_codes.join(",")
        ));
    }
    if !matches!(draft.status.as_str(), "drafted" | "need_confirmation") {
        return Err(format!("directive_draft_not_confirmable:{}", draft.status));
    }
    if !draft.bound_fencing_token.is_empty() {
        let valid: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 \
                 AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6 \
                 AND lease_expires_at>datetime('now','localtime'))",
                params![
                    draft.scan_id,
                    draft.attempt_number,
                    draft.target_key,
                    draft.root_run_id,
                    draft.bound_lease_epoch,
                    draft.bound_fencing_token
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法验证草案 Coordinator fencing：{error}"))?;
        if !valid {
            return Err("directive_draft_stale_fencing_token".into());
        }
    }
    validate_thread_key(
        &transaction,
        &draft.root_run_id,
        &draft.target_key,
        &draft.thread_key,
    )?;
    let protected = super::human_review::approval_scope(&transaction, &draft)?;
    let writer = super::human_review::guard::Writer::install(
        &transaction,
        super::human_review::guard::Mode::Approval,
        &draft,
    )?;
    let directive_id = uuid::Uuid::new_v4().to_string();
    let payload = confirmed_payload(&draft);
    transaction
        .execute(
            "INSERT INTO agent_user_directives(id,scan_id,attempt_number,root_run_id,target_key,recipient_role,text_redacted,payload_json,status,\
             source_draft_id,confirmed_revision,confirmed_hash,confirmation_at,thread_key) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?10,?11,datetime('now','localtime'),?12)",
            params![
                directive_id,
                draft.scan_id,
                draft.attempt_number,
                draft.root_run_id,
                draft.target_key,
                draft.recipient_role,
                draft.safe_execution_text,
                payload.to_string(),
                draft.id,
                draft.revision,
                draft.draft_hash,
                draft.thread_key,
            ],
        )
        .map_err(|error| format!("无法保存已确认用户指令：{error}"))?;
    let changed = transaction
        .execute(
            "UPDATE agent_directive_drafts SET status='confirmed',confirmed_directive_id=?1,confirmed_at=datetime('now','localtime'),\
             updated_at=datetime('now','localtime') WHERE id=?2 AND revision=?3 AND draft_hash=?4 AND status IN ('drafted','need_confirmation')",
            params![directive_id, draft.id, draft.revision, draft.draft_hash],
        )
        .map_err(|error| format!("无法确认用户指令草案：{error}"))?;
    if changed != 1 {
        return Err("directive_draft_confirmation_conflict".into());
    }
    super::human_review::record_approval(&transaction, &draft, &directive_id, &protected)?;
    // Complete the original approval writer before projecting into the
    // homogeneous business queue. Its financial/write authority stays intact.
    writer.finish()?;
    // DESIGN §5.6 / §8：确认过的引导在同质任务上进入闸门（G1 脱敏 + G2 规则初筛），
    // 由主管在后续裁决 relay/rewrite/reject。默认 legacy 时这里零副作用。
    let homogeneous = crate::agent_runtime::multi_agent::homogeneous::apply::task_exists_for_root(
        &transaction,
        &draft.root_run_id,
    );
    if matches!(homogeneous, Ok(true)) {
        if let Some(task) =
            crate::agent_runtime::multi_agent::homogeneous::store::load_task_by_root_run(
                &transaction,
                &draft.root_run_id,
            )
            .ok()
            .flatten()
        {
            let gated_id = format!(
                "dir-{}",
                &crate::agent_runtime::store::stable_hash(&directive_id)[..24]
            );
            screen_confirmed_homogeneous_guidance(
                &transaction,
                &task,
                &gated_id,
                &directive_id,
                &draft.safe_execution_text,
            )?;
            if super::human_review::approval_scope(&transaction, &draft)? != protected {
                return Err("directive_human_review_scope_changed".into());
            }
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交用户指令确认：{error}"))?;
    Ok(UserDirective {
        id: directive_id,
        text: draft.safe_execution_text,
        status: "pending".into(),
    })
}

// The projection shares the confirmation transaction so a rejected write
// rolls back the approval too. It cannot inherit the approval writer's rights.
fn screen_confirmed_homogeneous_guidance(
    connection: &Connection,
    task: &crate::agent_runtime::multi_agent::homogeneous::types::HomogeneousTask,
    gated_id: &str,
    directive_id: &str,
    text: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, homogeneous};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let previous = Rows::read(
        connection,
        "SELECT rowid,* FROM agent_human_directive_gated ORDER BY rowid",
        [],
    )?;
    let floor: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let events = Rows::read(
        connection,
        "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
        [floor],
    )?;
    struct QueueWriter<'a>(&'a Connection);
    impl Drop for QueueWriter<'_> {
        fn drop(&mut self) {
            let _ = self
                .0
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
    connection
        .authorizer(Some(|context: AuthContext<'_>| {
            let direct = context.database_name == Some("main") && context.accessor.is_none();
            let allowed = match context.action {
                AuthAction::Insert {
                    table_name: "agent_human_directive_gated",
                }
                | AuthAction::Update {
                    table_name: "agent_human_directive_gated",
                    ..
                } => direct,
                AuthAction::Insert {
                    table_name: "agent_collaboration_events",
                } => {
                    context.database_name == Some("main")
                        && matches!(
                            context.accessor,
                            Some(
                                "agent_collaboration_homogeneous_guidance_insert"
                                    | "agent_collaboration_homogeneous_guidance_update"
                            )
                        )
                }
                AuthAction::Read { .. }
                | AuthAction::Select
                | AuthAction::Function { .. }
                | AuthAction::Transaction { .. }
                | AuthAction::Savepoint { .. }
                | AuthAction::Recursive => true,
                _ => false,
            };
            if allowed {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }))
        .map_err(|_| "homogeneous_guidance_confirmation_guard_unavailable")?;
    let _writer = QueueWriter(connection);
    let expected =
        homogeneous::directive_gate::screen(connection, task, gated_id, directive_id, text)
            .map_err(|error| format!("引导闸门初筛失败：{}", error.reason_code))?;
    if homogeneous::store::load_gated_directive(connection, gated_id)
        .map_err(|e| e.reason_code.to_string())?
        != Some(expected.clone())
        || Rows::read(
            connection,
            "SELECT rowid,* FROM agent_human_directive_gated WHERE directive_id<>?1 ORDER BY rowid",
            [gated_id],
        )? != previous
        || Rows::read(
            connection,
            "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
            [floor],
        )? != events
    {
        return Err("homogeneous_guidance_confirmation_projection_changed".into());
    }
    let exact: bool = connection.query_row("SELECT COUNT(*)=1 AND SUM(CASE WHEN scan_id=?2 AND attempt_number=?3 AND entity_type='homogeneous_guidance' AND entity_id=?4 AND event_type='homogeneous_guidance' AND payload_json=?5 AND created_at<>'' THEN 0 ELSE 1 END)=0 FROM agent_collaboration_events WHERE sequence>?1", params![floor, task.scan_id, task.attempt_number, gated_id, serde_json::json!({"state":expected.state}).to_string()], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !exact {
        return Err("homogeneous_guidance_confirmation_event_invalid".into());
    }
    Ok(())
}

pub fn cancel_draft(
    connection: &Connection,
    scan_id: &str,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE agent_directive_drafts SET status='cancelled',cancelled_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?1 AND scan_id=?2 AND revision=?3 AND draft_hash=?4 AND status IN ('drafted','need_confirmation')",
            params![draft_id, scan_id, revision, draft_hash],
        )
        .map_err(|error| format!("无法取消用户指令草案：{error}"))?;
    if changed == 1 {
        Ok(())
    } else {
        Err("directive_draft_cancel_conflict".into())
    }
}
