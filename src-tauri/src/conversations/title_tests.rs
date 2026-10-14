use super::*;
use crate::conversations::{model::*, reply, tests::Fixture};
use crate::db;
use rusqlite::TransactionBehavior;
use uuid::Uuid;

fn running(f: &Fixture, item: &Conversation) -> (Conversation, String) {
    let receipt = reply::send(&f.path, f.input(item, "Help plan the SDK migration")).unwrap();
    let id = Uuid::new_v4().to_string();
    let db = db::open(&f.path).unwrap();
    db.execute(
        "INSERT INTO conversation_replies(id,conversation_id,user_message_id,base_revision,status,created_at,updated_at)
         VALUES(?1,?2,?3,?4,'running',?5,?5)",
        params![id, item.id, receipt.user_message.id, receipt.conversation.revision, store::stamp()],
    ).unwrap();
    (receipt.conversation, id)
}

fn apply_and_commit(f: &Fixture, item: &Conversation, reply_id: &str) -> bool {
    let mut db = db::open(&f.path).unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
    let applied = apply(&tx, item, reply_id).unwrap();
    if applied {
        // The title does not consume a second revision. The reply owns the bump.
        assert_eq!(store::conversation(&tx, item.project_id, &item.id).unwrap().revision, item.revision);
        store::bump(&tx, &item.id, item.revision).unwrap();
        tx.execute("UPDATE conversation_replies SET status='completed' WHERE id=?1", [reply_id]).unwrap();
    }
    tx.commit().unwrap();
    applied
}

#[test]
fn new_untitled_chats_opt_in_but_explicit_and_legacy_titles_are_protected() {
    let f = Fixture::new();
    let automatic = f.create();
    assert_eq!(automatic.title_source, "pending");
    let explicit = store::create(&f.path, CreateInput {
        project_id: 901, operation_id: Uuid::new_v4().to_string(), title: Some("新对话".into()),
    }).unwrap();
    let db = db::open(&f.path).unwrap();
    assert!(eligible(&db, &automatic).unwrap());
    assert_eq!(explicit.title_source, "manual");
    assert!(!eligible(&db, &explicit).unwrap());
    db.execute("DELETE FROM conversation_title_state WHERE conversation_id=?1", [&automatic.id]).unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    let legacy = store::conversation(&db, 901, &automatic.id).unwrap();
    assert_eq!(legacy.title_source, "legacy");
    assert!(!eligible(&db, &legacy).unwrap());
}

#[test]
fn title_schema_is_strict_scope_free_and_has_no_external_capability() {
    let schema = tool_schema();
    assert_eq!(schema.name, "conversation_set_title");
    assert_eq!(schema.parameters["additionalProperties"], false);
    assert_eq!(schema.parameters["properties"].as_object().unwrap().len(), 1);
    assert_eq!(schema.parameters["properties"]["title"]["maxLength"], 80);
    assert_eq!(schema.parameters["required"], json!(["title"]));
    for args in [json!({}), json!({"title":3}), json!({"title":"A","projectId":902}), json!({"title":"A","conversationId":"foreign"}), json!({"title":"\n\t"}), json!({"title":"line\nbreak"}), json!({"title":"bad\u{202e}title"}), json!({"title":"x".repeat(81)})] {
        assert!(parse_proposal(&args).is_err(), "{args}");
    }
    assert_eq!(parse_proposal(&json!({"title":"  SDK 迁移 🦖  "})).unwrap(), "SDK 迁移 🦖");
    assert!(parse_proposal(&json!({"title":"雪".repeat(80)})).is_ok());
}

#[test]
fn proposal_is_idempotent_and_only_successful_commit_changes_title_once() {
    let f = Fixture::new();
    let initial = f.create();
    let (item, reply_id) = running(&f, &initial);
    let db = db::open(&f.path).unwrap();
    let args = json!({"title":"SDK migration plan"});
    let first = propose(&db, &item, &reply_id, &args).unwrap();
    assert_eq!(first["status"], "proposed");
    assert_eq!(first, propose(&db, &item, &reply_id, &args).unwrap());
    assert_eq!(propose(&db, &item, &reply_id, &json!({"title":"Different"})).unwrap_err(), "conversation_title_proposal_conflict");
    let pending = store::conversation(&db, 901, &item.id).unwrap();
    assert_eq!(pending.title, initial.title);
    assert_eq!(pending.revision, item.revision);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM conversation_title_proposals WHERE reply_id=?1", [&reply_id], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert!(apply_and_commit(&f, &item, &reply_id));
    let saved = store::conversation(&db, 901, &item.id).unwrap();
    assert_eq!(saved.title, "SDK migration plan");
    assert_eq!(saved.title_source, "ai");
    assert_eq!(saved.revision, item.revision + 1);
    assert!(!eligible(&db, &saved).unwrap());
    assert!(!apply_and_commit(&f, &saved, &reply_id));
    let (next, next_reply) = running(&f, &saved);
    assert_eq!(propose(&db, &next, &next_reply, &args).unwrap_err(), "conversation_title_not_eligible");
    assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, "SDK migration plan");
}

#[test]
fn manual_rename_even_to_original_default_protects_against_queued_proposal() {
    let f = Fixture::new();
    let initial = f.create();
    let (item, reply_id) = running(&f, &initial);
    let db = db::open(&f.path).unwrap();
    propose(&db, &item, &reply_id, &json!({"title":"AI proposal"})).unwrap();
    // The isolated test owns no live transport lock, so update recovers the
    // synthetic orphan before applying the explicitly authored rename.
    let renamed = store::update(&f.path, UpdateInput {
        project_id: 901, conversation_id: item.id.clone(), expected_revision: item.revision,
        title: Some("新对话".into()), archived: None,
    }).unwrap();
    assert_eq!(renamed.title_source, "manual");
    assert!(!apply_and_commit(&f, &renamed, &reply_id));
    assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, "新对话");
    let (next, next_reply) = running(&f, &renamed);
    assert!(propose(&db, &next, &next_reply, &json!({"title":"Do not replace"})).is_err());
}

#[test]
fn automatic_title_cannot_cross_project_conversation_reply_or_revision() {
    let f = Fixture::new();
    let (item, reply_id) = running(&f, &f.create());
    let (other, other_reply) = running(&f, &f.create());
    let db = db::open(&f.path).unwrap();
    let args = json!({"title":"Scoped proposal"});
    let mut foreign = item.clone(); foreign.project_id = 902;
    assert!(eligible(&db, &foreign).is_err());
    assert!(propose(&db, &foreign, &reply_id, &args).is_err());
    assert!(propose(&db, &item, &other_reply, &args).is_err());
    assert!(propose(&db, &other, &reply_id, &args).is_err());
    let mut stale = item.clone(); stale.revision -= 1;
    assert!(propose(&db, &stale, &reply_id, &args).is_err());
    propose(&db, &item, &reply_id, &args).unwrap();
    assert!(!apply_and_commit(&f, &other, &reply_id));
    assert!(!apply_and_commit(&f, &stale, &reply_id));
    assert_eq!(store::conversation(&db, 901, &other.id).unwrap().title, other.title);
}

#[test]
fn cancelled_failed_archived_and_stale_replies_never_apply_titles() {
    for status in ["cancel_requested", "cancelled", "failed", "interrupted", "queued", "completed"] {
        let f = Fixture::new();
        let (item, reply_id) = running(&f, &f.create());
        let db = db::open(&f.path).unwrap();
        propose(&db, &item, &reply_id, &json!({"title":"Must stay pending"})).unwrap();
        db.execute("UPDATE conversation_replies SET status=?2 WHERE id=?1", params![reply_id, status]).unwrap();
        assert!(!apply_and_commit(&f, &item, &reply_id), "{status}");
        assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, item.title);
    }
    for sql in [
        "UPDATE conversations SET revision=revision+1 WHERE id=?1",
        "UPDATE conversations SET archived=1 WHERE id=?1",
        "UPDATE projects SET status='archived' WHERE id=(SELECT project_id FROM conversations WHERE id=?1)",
    ] {
        let f = Fixture::new();
        let (item, reply_id) = running(&f, &f.create());
        let db = db::open(&f.path).unwrap();
        propose(&db, &item, &reply_id, &json!({"title":"Must stay pending"})).unwrap();
        db.execute(sql, [&item.id]).unwrap();
        assert!(!apply_and_commit(&f, &item, &reply_id));
        assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, item.title);
    }
}

#[test]
fn proposal_and_provenance_roll_back_with_reply_transaction_and_detect_ignored_writes() {
    let f = Fixture::new();
    let (item, reply_id) = running(&f, &f.create());
    let mut db = db::open(&f.path).unwrap();
    propose(&db, &item, &reply_id, &json!({"title":"Atomic title"})).unwrap();
    assert_eq!(apply(&db, &item, &reply_id).unwrap_err(), "conversation_title_transaction_required");
    {
        let tx = db.transaction().unwrap();
        assert!(apply(&tx, &item, &reply_id).unwrap());
        // Simulate a later assistant write failure; dropping the tx rolls back.
    }
    let unchanged = store::conversation(&db, 901, &item.id).unwrap();
    assert_eq!(unchanged.title, item.title);
    assert_eq!(unchanged.title_source, "pending");
    db.execute_batch("CREATE TRIGGER ignore_ai_title BEFORE UPDATE ON conversation_title_state BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    {
        let tx = db.transaction().unwrap();
        assert_eq!(apply(&tx, &item, &reply_id).unwrap_err(), "conversation_title_write_not_persisted");
    }
    assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, item.title);
}

#[test]
fn failed_provenance_write_cannot_partially_rename_or_create_and_delete_cascades() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute_batch("CREATE TRIGGER ignore_manual_title BEFORE UPDATE ON conversation_title_state BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(store::update(&f.path, UpdateInput {
        project_id: 901, conversation_id: item.id.clone(), expected_revision: item.revision,
        title: Some("manual".into()), archived: None,
    }).is_err());
    assert_eq!(store::conversation(&db, 901, &item.id).unwrap().title, item.title);
    db.execute_batch("DROP TRIGGER ignore_manual_title; CREATE TRIGGER ignore_title_create BEFORE INSERT ON conversation_title_state BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(store::create(&f.path, CreateInput { project_id:901,operation_id:Uuid::new_v4().to_string(),title:None }).is_err());
    assert_eq!(db.query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
    db.execute_batch("DROP TRIGGER ignore_title_create;").unwrap();
    let (current, reply_id) = running(&f, &item);
    propose(&db, &current, &reply_id, &json!({"title":"Delete with chat"})).unwrap();
    db.execute("DELETE FROM conversations WHERE id=?1", [&item.id]).unwrap();
    for table in ["conversation_title_state", "conversation_title_proposals"] {
        assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    }
}
