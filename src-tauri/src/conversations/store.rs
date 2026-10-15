use super::model::*;
use crate::db;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    path::Path,
};
use uuid::Uuid;

pub(super) const MAX_INTEGER: i64 = 9_007_199_254_740_991;
pub(super) const ACTIVE: &str = "('queued','running','cancel_requested')";
pub(super) const CONVERSATION_COLUMNS: &str = "c.id,c.project_id,c.title,c.archived,c.revision,
    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id=c.id),
    COALESCE((SELECT substr(content,1,160) FROM conversation_messages m WHERE m.conversation_id=c.id ORDER BY sequence DESC LIMIT 1),''),c.created_at,c.updated_at,
    COALESCE((SELECT source FROM conversation_title_state s WHERE s.conversation_id=c.id),'legacy')";
pub(super) const MESSAGE_COLUMNS: &str =
    "id,conversation_id,sequence,role,content,reply_to_message_id,created_at";
pub(super) const REPLY_COLUMNS: &str = "id,user_message_id,status,code,message,assistant_message_id,outcome_unknown,created_at,updated_at";
pub(super) fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub(super) fn err(error: rusqlite::Error) -> String {
    format!("conversation_storage:{error}")
}
fn uuid(value: &str) -> Result<(), String> {
    if Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value) {
        Ok(())
    } else {
        Err("conversation_invalid_id".into())
    }
}
pub(super) fn operation(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        Err("conversation_invalid_operation".into())
    } else {
        Ok(())
    }
}
fn title(value: &str) -> Result<String, String> {
    let text = value.trim();
    if text.is_empty() || text.chars().count() > 160 || text.chars().any(char::is_control) {
        Err("conversation_invalid_title".into())
    } else {
        Ok(text.into())
    }
}
pub(super) fn project(db: &Connection, id: i64, writable: bool) -> Result<(), String> {
    if !(1..=MAX_INTEGER).contains(&id) {
        return Err("conversation_invalid_project".into());
    }
    let status: Option<String> = db
        .query_row("SELECT status FROM projects WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    match status.as_deref() {
        None => Err("conversation_project_unavailable".into()),
        Some("active") => Ok(()),
        Some(_) if !writable => Ok(()),
        _ => Err("conversation_project_archived".into()),
    }
}
fn conversation_row(r: &Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        title_source: r.get(9)?,
        archived: r.get(3)?,
        revision: r.get(4)?,
        message_count: r.get(5)?,
        last_message_preview: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}
pub(super) fn message_row(r: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        sequence: r.get(2)?,
        role: r.get(3)?,
        content: r.get(4)?,
        reply_to_message_id: r.get(5)?,
        created_at: r.get(6)?,
    })
}
pub(super) fn reply_row(r: &Row<'_>) -> rusqlite::Result<Reply> {
    Ok(Reply {
        id: r.get(0)?,
        user_message_id: r.get(1)?,
        status: r.get(2)?,
        code: r.get(3)?,
        message: r.get(4)?,
        assistant_message_id: r.get(5)?,
        outcome_unknown: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}
pub(super) fn conversation(
    db: &Connection,
    project_id: i64,
    id: &str,
) -> Result<Conversation, String> {
    uuid(id)?;
    project(db, project_id, false)?;
    db.query_row(
        &format!(
            "SELECT {CONVERSATION_COLUMNS} FROM conversations c WHERE c.id=?1 AND c.project_id=?2"
        ),
        params![id, project_id],
        conversation_row,
    )
    .optional()
    .map_err(err)?
    .ok_or_else(|| "conversation_unavailable".into())
}
pub(super) fn revision(value: i64) -> Result<(), String> {
    if !(1..MAX_INTEGER).contains(&value) {
        Err("conversation_invalid_revision".into())
    } else {
        Ok(())
    }
}
pub(super) fn writable(
    db: &Connection,
    project_id: i64,
    id: &str,
    expected: i64,
    allow_archived: bool,
) -> Result<Conversation, String> {
    revision(expected)?;
    project(db, project_id, true)?;
    let item = conversation(db, project_id, id)?;
    if item.revision != expected {
        return Err("conversation_revision_conflict".into());
    }
    if item.archived && !allow_archived {
        return Err("conversation_archived".into());
    }
    let busy: bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM conversation_replies WHERE conversation_id=?1 AND status IN {ACTIVE})"),[id],|r|r.get(0)).map_err(err)?;
    if busy {
        return Err("conversation_reply_active".into());
    }
    Ok(item)
}
pub(super) fn bump(db: &Connection, id: &str, expected: i64) -> Result<(), String> {
    if db.execute("UPDATE conversations SET revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?2",params![id,expected,stamp()]).map_err(err)? != 1 {
        return Err("conversation_revision_conflict".into());
    }
    let actual: i64 = db
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if actual != expected + 1 {
        return Err("conversation_write_not_persisted".into());
    }
    Ok(())
}

/// The filename is a validated identity read from the database. Lock files are
/// stable and never unlinked: deleting a path would split the ownership domain.
pub(super) fn try_lock(path: &Path, item: &Conversation) -> Result<Option<File>, String> {
    uuid(&item.id)?;
    let directory = path
        .parent()
        .ok_or("conversation_lock_directory")?
        .join("conversation-locks");
    std::fs::create_dir_all(&directory).map_err(|_| "conversation_lock_directory".to_string())?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(directory.join(format!("{}.lock", item.id)))
        .map_err(|_| "conversation_lock_open".to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err("conversation_lock_failed".into()),
    }
}
pub(super) fn lock(path: &Path, item: &Conversation) -> Result<File, String> {
    try_lock(path, item)?.ok_or_else(|| "conversation_reply_active".into())
}
/// Only call with the conversation's OS lock held. No timer or process id is
/// evidence that a live transport stopped. Orphans are never automatically sent.
pub(super) fn recover_in(db: &Connection, id: &str) -> Result<(), String> {
    let active: Vec<(String, String)> = {
        let mut stmt=db.prepare(&format!("SELECT id,status FROM conversation_replies WHERE conversation_id=?1 AND status IN {ACTIVE}")).map_err(err)?;
        let rows = stmt
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    for (reply_id, status) in active {
        let changed=db.execute("UPDATE conversation_replies SET status='interrupted',code='reply_interrupted',message='上次回复已中断；不会自动重发。模型端是否完成及费用可能未知。',outcome_unknown=?2,updated_at=?3 WHERE id=?1 AND status=?4",params![reply_id,status!="queued",stamp(),status]).map_err(err)?;
        let saved = reply(db, id, &reply_id)?.ok_or("conversation_reply_missing")?;
        if changed != 1 || saved.status != "interrupted" {
            return Err("conversation_write_not_persisted".into());
        }
    }
    Ok(())
}
fn recover(path: &Path, project_id: i64, id: &str) -> Result<(), String> {
    let mut db = db::open(path)?;
    let item = conversation(&db, project_id, id)?;
    let active:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM conversation_replies WHERE conversation_id=?1 AND status IN {ACTIVE})"),[id],|r|r.get(0)).map_err(err)?;
    if !active {
        return Ok(());
    }
    let Some(_owner) = try_lock(path, &item)? else {
        return Ok(());
    };
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    conversation(&tx, project_id, id)?;
    recover_in(&tx, id)?;
    tx.commit().map_err(err)
}
pub(super) fn reply(
    db: &Connection,
    conversation_id: &str,
    id: &str,
) -> Result<Option<Reply>, String> {
    db.query_row(
        &format!(
            "SELECT {REPLY_COLUMNS} FROM conversation_replies WHERE conversation_id=?1 AND id=?2"
        ),
        params![conversation_id, id],
        reply_row,
    )
    .optional()
    .map_err(err)
}
pub(super) fn latest_reply(db: &Connection, id: &str) -> Result<Option<Reply>, String> {
    db.query_row(&format!("SELECT {REPLY_COLUMNS} FROM conversation_replies WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT 1"),[id],reply_row).optional().map_err(err)
}
fn page_limit(value: Option<i64>) -> Result<i64, String> {
    let limit = value.unwrap_or(100);
    if !(1..=100).contains(&limit) {
        Err("conversation_invalid_limit".into())
    } else {
        Ok(limit)
    }
}

pub(super) fn list(path: &Path, input: ListInput) -> Result<ConversationPage, String> {
    let mut db = db::open(path)?;
    let tx = db.transaction().map_err(err)?;
    project(&tx, input.project_id, false)?;
    let limit = page_limit(input.limit)?;
    let offset = input.offset.unwrap_or(0);
    if !(0..=MAX_INTEGER).contains(&offset) {
        return Err("conversation_invalid_offset".into());
    }
    let query = input.query.unwrap_or_default();
    let query = query.trim();
    if query.chars().count() > 200 {
        return Err("conversation_query_too_long".into());
    }
    let pattern = format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let filter="c.project_id=?1 AND (?2 OR c.archived=0) AND (c.title LIKE ?3 ESCAPE '\\' OR EXISTS(SELECT 1 FROM conversation_messages m WHERE m.conversation_id=c.id AND m.content LIKE ?3 ESCAPE '\\'))";
    let archived = input.include_archived.unwrap_or(false);
    let total = tx
        .query_row(
            &format!("SELECT count(*) FROM conversations c WHERE {filter}"),
            params![input.project_id, archived, pattern],
            |r| r.get(0),
        )
        .map_err(err)?;
    let items = {
        let mut stmt=tx.prepare(&format!("SELECT {CONVERSATION_COLUMNS} FROM conversations c WHERE {filter} ORDER BY c.updated_at DESC,c.id DESC LIMIT ?4 OFFSET ?5")).map_err(err)?;
        let rows = stmt
            .query_map(
                params![input.project_id, archived, pattern, limit, offset],
                conversation_row,
            )
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    tx.commit().map_err(err)?;
    Ok(ConversationPage { items, total })
}
pub(super) fn create(path: &Path, input: CreateInput) -> Result<Conversation, String> {
    operation(&input.operation_id)?;
    let title = title(input.title.as_deref().unwrap_or("新对话"))?;
    let mut db = db::open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    project(&tx, input.project_id, false)?;
    let existing:Option<(String,String)>=tx.query_row("SELECT id,create_title FROM conversations WHERE project_id=?1 AND create_operation_id=?2",params![input.project_id,input.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    if let Some((id, original)) = existing {
        if original != title {
            return Err("conversation_operation_conflict".into());
        }
        return conversation(&tx, input.project_id, &id);
    }
    project(&tx, input.project_id, true)?;
    let id = Uuid::new_v4().to_string();
    let now = stamp();
    if tx.execute("INSERT INTO conversations(id,project_id,create_operation_id,create_title,title,created_at,updated_at) VALUES(?1,?2,?3,?4,?4,?5,?5)",params![id,input.project_id,input.operation_id,title,now]).map_err(err)?!=1{return Err("conversation_write_not_persisted".into());}
    super::title::initialize(&tx, &id, input.title.is_some())?;
    let saved = conversation(&tx, input.project_id, &id)?;
    if saved.title != title || saved.revision != 1 {
        return Err("conversation_write_not_persisted".into());
    }
    tx.commit().map_err(err)?;
    Ok(saved)
}
fn link(db: &Connection, project_id: i64, kind: &str, target_id: &str) -> Result<Link, String> {
    let label:Option<String>=match kind {
        "scan" => db.query_row("SELECT CASE WHEN trim(task_name)<>'' THEN task_name ELSE id END FROM sentinel_scans WHERE id=?1 AND project_id=?2 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=sentinel_scans.id)",params![target_id,project_id],|r|r.get(0)).optional().map_err(err)?,
        "asset" => {
            let id=target_id.parse::<i64>().map_err(|_|"conversation_invalid_asset_id")?;
            if id<=0||id>MAX_INTEGER||id.to_string()!=target_id{return Err("conversation_invalid_asset_id".into());}
            db.query_row("SELECT COALESCE(NULLIF(a.title,''),NULLIF(a.link,''),a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id=?2 AND pa.is_deleted=0",params![project_id,id],|r|r.get(0)).optional().map_err(err)?
        },
        _ => return Err("conversation_invalid_link_kind".into()),
    };
    Ok(Link {
        kind: kind.into(),
        target_id: target_id.into(),
        available: label.is_some(),
        label: label.unwrap_or_else(|| target_id.into()),
    })
}
fn links(db: &Connection, item: &Conversation) -> Result<Vec<Link>, String> {
    let mut stmt=db.prepare("SELECT kind,target_id FROM conversation_links WHERE conversation_id=?1 ORDER BY kind,target_id").map_err(err)?;
    let rows = stmt
        .query_map([&item.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    rows.into_iter()
        .map(|(kind, id)| link(db, item.project_id, &kind, &id))
        .collect()
}
pub(super) fn read(path: &Path, input: ReadInput) -> Result<ConversationView, String> {
    let limit = page_limit(input.limit)?;
    if input
        .before_sequence
        .is_some_and(|n| !(1..=MAX_INTEGER).contains(&n))
    {
        return Err("conversation_invalid_cursor".into());
    }
    recover(path, input.project_id, &input.conversation_id)?;
    let mut db = db::open(path)?;
    let tx = db.transaction().map_err(err)?;
    let item = conversation(&tx, input.project_id, &input.conversation_id)?;
    let mut messages = {
        let mut stmt=tx.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM conversation_messages WHERE conversation_id=?1 AND (?2 IS NULL OR sequence<?2) ORDER BY sequence DESC LIMIT ?3")).map_err(err)?;
        let rows = stmt
            .query_map(
                params![item.id, input.before_sequence, limit + 1],
                message_row,
            )
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    let has_more = messages.len() > limit as usize;
    messages.truncate(limit as usize);
    messages.reverse();
    let links = links(&tx, &item)?;
    let latest_reply = latest_reply(&tx, &item.id)?;
    let tool_activity = super::tools::activity(&tx, &item, &messages)?;
    let reply_availability = super::reply::availability(&tx, &item);
    tx.commit().map_err(err)?;
    Ok(ConversationView {
        conversation: item,
        messages,
        has_more,
        links,
        reply_availability,
        latest_reply,
        tool_activity,
    })
}
pub(super) fn update(path: &Path, input: UpdateInput) -> Result<Conversation, String> {
    if input.title.is_none() && input.archived.is_none() {
        return Err("conversation_empty_update".into());
    }
    let title = input.title.as_deref().map(title).transpose()?;
    let mut db = db::open(path)?;
    let initial = conversation(&db, input.project_id, &input.conversation_id)?;
    let _owner = lock(path, &initial)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    recover_in(&tx, &initial.id)?;
    let item = writable(
        &tx,
        input.project_id,
        &initial.id,
        input.expected_revision,
        true,
    )?;
    let next_title = title.unwrap_or(item.title);
    let archived = input.archived.unwrap_or(item.archived);
    if tx
        .execute(
            "UPDATE conversations SET title=?2,archived=?3 WHERE id=?1",
            params![item.id, next_title, archived],
        )
        .map_err(err)?
        != 1
    {
        return Err("conversation_write_not_persisted".into());
    }
    if input.title.is_some() {
        super::title::mark_manual(&tx, &item.id)?;
    }
    bump(&tx, &item.id, input.expected_revision)?;
    let saved = conversation(&tx, input.project_id, &item.id)?;
    if saved.title != next_title || saved.archived != archived {
        return Err("conversation_write_not_persisted".into());
    }
    tx.commit().map_err(err)?;
    Ok(saved)
}
pub(super) fn set_links(path: &Path, input: SetLinksInput) -> Result<Conversation, String> {
    if input.links.len() > 50 {
        return Err("conversation_too_many_links".into());
    }
    let mut unique = BTreeSet::new();
    for value in &input.links {
        if value.target_id.is_empty()
            || value.target_id.len() > 256
            || value.target_id.chars().any(char::is_control)
            || !unique.insert((value.kind.clone(), value.target_id.clone()))
        {
            return Err("conversation_invalid_links".into());
        }
    }
    let mut db = db::open(path)?;
    let initial = conversation(&db, input.project_id, &input.conversation_id)?;
    let _owner = lock(path, &initial)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    recover_in(&tx, &initial.id)?;
    let item = writable(
        &tx,
        input.project_id,
        &initial.id,
        input.expected_revision,
        false,
    )?;
    for value in &input.links {
        if !link(&tx, input.project_id, &value.kind, &value.target_id)?.available {
            return Err("conversation_link_unavailable".into());
        }
    }
    tx.execute(
        "DELETE FROM conversation_links WHERE conversation_id=?1",
        [&item.id],
    )
    .map_err(err)?;
    for value in &input.links {
        if tx
            .execute(
                "INSERT INTO conversation_links(conversation_id,kind,target_id) VALUES(?1,?2,?3)",
                params![item.id, value.kind, value.target_id],
            )
            .map_err(err)?
            != 1
        {
            return Err("conversation_write_not_persisted".into());
        }
    }
    let actual: BTreeSet<_> = links(&tx, &item)?
        .into_iter()
        .map(|v| (v.kind, v.target_id))
        .collect();
    if actual != unique {
        return Err("conversation_write_not_persisted".into());
    }
    bump(&tx, &item.id, input.expected_revision)?;
    let saved = conversation(&tx, input.project_id, &item.id)?;
    tx.commit().map_err(err)?;
    Ok(saved)
}
pub(super) fn delete(path: &Path, input: DeleteInput) -> Result<(), String> {
    let mut db = db::open(path)?;
    let initial = conversation(&db, input.project_id, &input.conversation_id)?;
    let _owner = lock(path, &initial)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    recover_in(&tx, &initial.id)?;
    writable(
        &tx,
        input.project_id,
        &initial.id,
        input.expected_revision,
        true,
    )?;
    if tx
        .execute(
            "DELETE FROM conversations WHERE id=?1 AND project_id=?2 AND revision=?3",
            params![initial.id, input.project_id, input.expected_revision],
        )
        .map_err(err)?
        != 1
    {
        return Err("conversation_write_not_persisted".into());
    }
    for table in [
        "conversations",
        "conversation_messages",
        "conversation_links",
        "conversation_replies",
    ] {
        let key = if table == "conversations" {
            "id"
        } else {
            "conversation_id"
        };
        let remains: bool = tx
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key}=?1)"),
                [&initial.id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if remains {
            return Err("conversation_delete_not_persisted".into());
        }
    }
    tx.commit().map_err(err)
}
