//! A title is proposed by the existing bounded reply loop, never a hidden model
//! call. Only a successful assistant commit may apply it, without another
//! conversation revision. Explicit user titles and legacy titles stay protected.
use super::{model::Conversation, store};
use crate::agent_runtime::model::ToolSchema;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) const TOOL_NAME: &str = "conversation_set_title";

pub(super) fn tool_schema() -> ToolSchema {
    ToolSchema::new(
        TOOL_NAME,
        "为当前新对话提出一个简短、具体的标题，使用用户的语言，概括对话主题，不包含秘密或凭据。仅在主题清晰时调用一次；标题会在本轮回答成功保存时应用，手动标题不会被覆盖。不要把标题写入正文，也不要声称工具已经执行其他操作。",
        json!({"type":"object","properties":{"title":{"type":"string","minLength":1,"maxLength":80}},"required":["title"],"additionalProperties":false}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    title: String,
}

fn parse_proposal(args: &Value) -> Result<String, String> {
    let input: Proposal = serde_json::from_value(args.clone())
        .map_err(|_| "conversation_title_invalid_arguments".to_string())?;
    let value = input.title.trim();
    if value.is_empty()
        || value.chars().count() > 80
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err("conversation_title_invalid_arguments".into());
    }
    Ok(value.to_string())
}

pub(super) fn initialize(db: &Connection, id: &str, explicit: bool) -> Result<(), String> {
    let source = if explicit { "manual" } else { "pending" };
    if db.execute(
        "INSERT INTO conversation_title_state(conversation_id,source,updated_at) VALUES(?1,?2,?3)",
        params![id, source, store::stamp()],
    ).map_err(store::err)? != 1 {
        return Err("conversation_title_write_not_persisted".into());
    }
    let saved: String = db.query_row(
        "SELECT source FROM conversation_title_state WHERE conversation_id=?1", [id], |r| r.get(0),
    ).map_err(store::err)?;
    if saved != source {
        return Err("conversation_title_write_not_persisted".into());
    }
    Ok(())
}

pub(super) fn mark_manual(db: &Connection, id: &str) -> Result<(), String> {
    if db.execute(
        "INSERT INTO conversation_title_state(conversation_id,source,updated_at) VALUES(?1,'manual',?2)
         ON CONFLICT(conversation_id) DO UPDATE SET source='manual',source_reply_id=NULL,source_user_sequence=0,updated_at=excluded.updated_at",
        params![id, store::stamp()],
    ).map_err(store::err)? != 1 {
        return Err("conversation_title_write_not_persisted".into());
    }
    let saved: String = db.query_row(
        "SELECT source FROM conversation_title_state WHERE conversation_id=?1", [id], |r| r.get(0),
    ).map_err(store::err)?;
    if saved != "manual" {
        return Err("conversation_title_write_not_persisted".into());
    }
    Ok(())
}

pub(super) fn eligible(db: &Connection, item: &Conversation) -> Result<bool, String> {
    store::conversation(db, item.project_id, &item.id)?;
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversations c
         JOIN projects p ON p.id=c.project_id
         JOIN conversation_title_state s ON s.conversation_id=c.id
         WHERE c.id=?1 AND c.project_id=?2 AND c.archived=0 AND p.status='active' AND s.source='pending')",
        params![item.id, item.project_id], |r| r.get(0),
    ).map_err(store::err)
}

/// This local metadata proposal is immutable per reply and has no effect on the
/// displayed title or revision until `apply` runs in the assistant transaction.
pub(super) fn propose(
    db: &Connection,
    item: &Conversation,
    reply_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let title = parse_proposal(args)?;
    store::conversation(db, item.project_id, &item.id)?;
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_replies r
         JOIN conversations c ON c.id=r.conversation_id JOIN projects p ON p.id=c.project_id
         JOIN conversation_title_state s ON s.conversation_id=c.id
         WHERE r.id=?1 AND c.id=?2 AND c.project_id=?3 AND c.revision=?4 AND r.base_revision=?4
         AND r.status='running' AND c.archived=0 AND p.status='active' AND s.source='pending')",
        params![reply_id, item.id, item.project_id, item.revision], |r| r.get(0),
    ).map_err(store::err)?;
    if !active {
        return Err("conversation_title_not_eligible".into());
    }
    db.execute(
        "INSERT INTO conversation_title_proposals(reply_id,conversation_id,title,source_user_sequence,created_at)
         SELECT r.id,c.id,?5,m.sequence,?6 FROM conversation_replies r
         JOIN conversations c ON c.id=r.conversation_id JOIN projects p ON p.id=c.project_id
         JOIN conversation_title_state s ON s.conversation_id=c.id
         JOIN conversation_messages m ON m.id=r.user_message_id AND m.conversation_id=c.id
         WHERE r.id=?1 AND c.id=?2 AND c.project_id=?3 AND c.revision=?4 AND r.base_revision=?4
         AND r.status='running' AND c.archived=0 AND p.status='active' AND s.source='pending' AND m.role='user'
         ON CONFLICT(reply_id) DO NOTHING",
        params![reply_id, item.id, item.project_id, item.revision, title, store::stamp()],
    ).map_err(store::err)?;
    let saved: Option<String> = db.query_row(
        "SELECT title FROM conversation_title_proposals WHERE reply_id=?1 AND conversation_id=?2",
        params![reply_id, item.id], |r| r.get(0),
    ).optional().map_err(store::err)?;
    match saved {
        Some(saved) if saved == title => Ok(json!({"status":"proposed","title":title,"appliesOnSuccessfulReply":true})),
        Some(_) => Err("conversation_title_proposal_conflict".into()),
        None => Err("conversation_title_write_not_persisted".into()),
    }
}

/// Caller must hold the reply's transaction and invoke this before its single
/// revision bump. Failure/cancellation/stale completion must never call apply.
pub(super) fn apply(
    tx: &Connection,
    item: &Conversation,
    reply_id: &str,
) -> Result<bool, String> {
    if tx.is_autocommit() {
        return Err("conversation_title_transaction_required".into());
    }
    store::conversation(tx, item.project_id, &item.id)?;
    let candidate: Option<(String, i64)> = tx.query_row(
        "SELECT proposal.title,proposal.source_user_sequence FROM conversation_title_proposals proposal
         JOIN conversation_replies r ON r.id=proposal.reply_id AND r.conversation_id=proposal.conversation_id
         JOIN conversations c ON c.id=r.conversation_id JOIN projects p ON p.id=c.project_id
         JOIN conversation_title_state s ON s.conversation_id=c.id
         JOIN conversation_messages m ON m.id=r.user_message_id AND m.conversation_id=c.id
         WHERE r.id=?1 AND c.id=?2 AND c.project_id=?3 AND c.revision=?4 AND r.base_revision=?4
         AND r.status='running' AND c.archived=0 AND p.status='active' AND s.source='pending'
         AND m.role='user' AND proposal.source_user_sequence=m.sequence",
        params![reply_id, item.id, item.project_id, item.revision], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(store::err)?;
    let Some((title, sequence)) = candidate else { return Ok(false); };
    parse_proposal(&json!({"title":title}))?;
    if tx.execute(
        "UPDATE conversations SET title=?4 WHERE id=?1 AND project_id=?2 AND revision=?3",
        params![item.id, item.project_id, item.revision, title],
    ).map_err(store::err)? != 1 {
        return Err("conversation_title_write_not_persisted".into());
    }
    if tx.execute(
        "UPDATE conversation_title_state SET source='ai',source_reply_id=?2,source_user_sequence=?3,updated_at=?4
         WHERE conversation_id=?1 AND source='pending'",
        params![item.id, reply_id, sequence, store::stamp()],
    ).map_err(store::err)? != 1 {
        return Err("conversation_title_write_not_persisted".into());
    }
    let saved: (String, String, Option<String>, i64) = tx.query_row(
        "SELECT c.title,s.source,s.source_reply_id,s.source_user_sequence FROM conversations c
         JOIN conversation_title_state s ON s.conversation_id=c.id WHERE c.id=?1 AND c.project_id=?2",
        params![item.id, item.project_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).map_err(store::err)?;
    if saved != (title, "ai".to_string(), Some(reply_id.to_string()), sequence) {
        return Err("conversation_title_write_not_persisted".into());
    }
    Ok(true)
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
