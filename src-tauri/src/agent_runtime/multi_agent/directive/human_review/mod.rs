//! Human choices are durable decisions, separate from downstream execution.
use super::{draft_store, HumanDirectiveDraft};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
pub(super) mod guard;
mod scope;
mod store;
pub(crate) use store::receipt_for;

pub(crate) fn review(
    connection: &Connection,
    scan_id: &str,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
    kind: &str,
    argument: &str,
) -> Result<Value, String> {
    if !matches!(kind, "revise" | "reject") {
        return Err("directive_human_review_kind_invalid".into());
    }
    let argument = argument.trim();
    let limit = if kind == "reject" { 500 } else { 2000 };
    if argument.is_empty() || argument.chars().count() > limit {
        return Err("directive_human_review_text_invalid".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|_| "directive_human_review_lock_failed")?;
    let draft = draft_store::load_draft(&tx, draft_id)?.ok_or("directive_draft_not_found")?;
    if draft.scan_id != scan_id {
        return Err("directive_draft_scope_mismatch".into());
    }
    if draft.revision != revision || draft.draft_hash != draft_hash {
        return Err("directive_draft_stale_revision".into());
    }
    if !draft_store::draft_integrity_valid(&draft) {
        return Err("directive_draft_integrity_failed".into());
    }
    scope::current(&tx, &draft)?;
    if let Some(receipt) = receipt_for(&tx, draft_id)? {
        if receipt["kind"] != kind
            || receipt["argumentHash"] != store::argument_hash(&tx, &draft.root_run_id, kind, argument)?
        {
            return Err("directive_human_review_conflict".into());
        }
        // Retry only reads the original committed outcome; no new draft/queue.
        let successor = receipt["successorDraftId"]
            .as_str()
            .map(|id| draft_store::load_draft(&tx, id))
            .transpose()?
            .flatten();
        let value = json!({"receipt":receipt,"draft":successor});
        let value = crate::agent_runtime::multi_agent::homogeneous::material_policy::json(&tx, &draft.root_run_id, &value)?;
        tx.commit()
            .map_err(|_| "directive_human_review_commit_unconfirmed")?;
        return Ok(value);
    }
    if !matches!(draft.status.as_str(), "drafted" | "need_confirmation")
        || !draft.confirmed_directive_id.is_empty()
    {
        return Err("directive_human_review_already_confirmed_or_terminal".into());
    }
    if queued(&tx, &draft.id)? {
        return Err("directive_human_review_already_queued".into());
    }
    if kind == "revise"
        && crate::agent_runtime::multi_agent::homogeneous::material_policy::text(&tx, &draft.root_run_id, argument)? == draft.text
    {
        return Err("directive_human_review_revision_unchanged".into());
    }
    let protected = scope::protected(&tx, &draft)?;
    let writer = guard::Writer::install(&tx, guard::Mode::Review, &draft)?;
    let successor = if kind == "revise" {
        let next = revision
            .checked_add(1)
            .ok_or("directive_human_review_revision_invalid")?;
        Some(draft_store::create_revision_in_transaction(
            &tx,
            &draft.scan_id,
            draft.attempt_number,
            &draft.root_run_id,
            &draft.target_key,
            &draft.recipient_role,
            argument,
            &draft.thread_key,
            draft.bound_lease_epoch,
            &draft.bound_fencing_token,
            next,
        )?)
    } else {
        None
    };
    let final_status = if kind == "revise" {
        "cancelled"
    } else {
        "rejected"
    };
    let changed=tx.execute("UPDATE agent_directive_drafts SET status=?1,updated_at=datetime('now','localtime')
        WHERE id=?2 AND scan_id=?3 AND revision=?4 AND draft_hash=?5 AND status IN ('drafted','need_confirmation')
        AND confirmed_directive_id='' AND NOT EXISTS(SELECT 1 FROM agent_user_directives WHERE source_draft_id=?2)",
        params![final_status,draft.id,draft.scan_id,revision,draft_hash]).map_err(|_|"directive_human_review_not_persisted")?;
    if changed != 1 {
        return Err("directive_human_review_not_persisted".into());
    }
    let receipt = store::record(&tx, &draft, kind, argument, "", successor.as_ref())?;
    let after =
        draft_store::load_draft(&tx, &draft.id)?.ok_or("directive_human_review_not_persisted")?;
    if !scope::same_frozen(&draft, &after)
        || after.status != final_status
        || queued(&tx, &draft.id)?
    {
        return Err("directive_human_review_postcondition".into());
    }
    if let Some(next) = &successor {
        let saved = draft_store::load_draft(&tx, &next.id)?
            .ok_or("directive_human_review_successor_missing")?;
        if saved != *next
            || !draft_store::draft_integrity_valid(&saved)
            || queued(&tx, &next.id)?
            || (!matches!(
                saved.status.as_str(),
                "drafted" | "need_confirmation" | "rejected"
            ))
            || (saved.status != "rejected" && !saved.confirmation_required)
        {
            return Err("directive_human_review_successor_changed".into());
        }
    }
    scope::current(&tx, &draft)?;
    if scope::protected(&tx, &draft)? != protected {
        return Err("directive_human_review_scope_changed".into());
    }
    let value = crate::agent_runtime::multi_agent::homogeneous::material_policy::json(&tx, &draft.root_run_id, &json!({"receipt":receipt,"draft":successor}))?;
    writer.finish()?;
    tx.commit()
        .map_err(|_| "directive_human_review_commit_unconfirmed")?;
    Ok(value)
}

fn queued(connection: &Connection, id: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE source_draft_id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "directive_human_review_queue_unavailable".into())
}

pub(super) fn record_approval(
    connection: &Connection,
    draft: &HumanDirectiveDraft,
    id: &str,
    protected: &[String],
) -> Result<Value, String> {
    let receipt = store::record(connection, draft, "approve", "", id, None)?;
    let after = draft_store::load_draft(connection, &draft.id)?
        .ok_or("directive_human_review_not_persisted")?;
    let exact:bool=connection.query_row("SELECT COUNT(*)=1 AND SUM(CASE WHEN id=?2 AND scan_id=?3
        AND attempt_number=?4 AND root_run_id=?5 AND target_key=?6 AND recipient_role=?7 AND thread_key=?8
        AND text_redacted=?9 AND payload_json=?10 AND confirmed_revision=?11 AND confirmed_hash=?12
        AND status='pending' AND confirmation_at<>'' THEN 0 ELSE 1 END)=0
        FROM agent_user_directives WHERE source_draft_id=?1",
        params![draft.id,id,draft.scan_id,draft.attempt_number,draft.root_run_id,draft.target_key,draft.recipient_role,
            draft.thread_key,draft.safe_execution_text,draft_store::confirmed_payload(draft).to_string(),draft.revision,draft.draft_hash],|r|r.get(0))
        .map_err(|_|"directive_human_review_queue_unavailable")?;
    if !exact
        || !scope::same_frozen(draft, &after)
        || after.status != "confirmed"
        || after.confirmed_directive_id != id
    {
        return Err("directive_human_review_approval_postcondition".into());
    }
    scope::current(connection, draft)?;
    if scope::protected(connection, draft)? != protected {
        return Err("directive_human_review_scope_changed".into());
    }
    Ok(receipt)
}

pub(super) fn approval_scope(
    connection: &Connection,
    draft: &HumanDirectiveDraft,
) -> Result<Vec<String>, String> {
    scope::current(connection, draft)?;
    scope::protected(connection, draft)
}
