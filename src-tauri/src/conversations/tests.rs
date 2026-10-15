use super::{model::*, reply, store};
use crate::db;
use rusqlite::params;
use std::path::PathBuf;
use uuid::Uuid;

pub(super) struct Fixture {
    pub root: PathBuf,
    pub path: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-conversation-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let db = db::open(&path).unwrap();
        db.execute(
            "INSERT INTO projects(id,name) VALUES(901,'Conversation One'),(902,'Conversation Two')",
            [],
        )
        .unwrap();
        Self { root, path }
    }
    pub fn create(&self) -> Conversation {
        store::create(
            &self.path,
            CreateInput {
                project_id: 901,
                operation_id: Uuid::new_v4().to_string(),
                title: None,
            },
        )
        .unwrap()
    }
    pub fn read(&self, item: &Conversation) -> ConversationView {
        store::read(
            &self.path,
            ReadInput {
                project_id: 901,
                conversation_id: item.id.clone(),
                before_sequence: None,
                limit: None,
            },
        )
        .unwrap()
    }
    pub fn input(&self, item: &Conversation, text: &str) -> SendInput {
        SendInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: item.revision,
            operation_id: Uuid::new_v4().to_string(),
            content: text.into(),
            request_reply: false,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn conversations_are_independent_persistent_and_creation_is_idempotent() {
    let f = Fixture::new();
    let op = Uuid::new_v4().to_string();
    let make = || CreateInput {
        project_id: 901,
        operation_id: op.clone(),
        title: Some("Review planning".into()),
    };
    let first = store::create(&f.path, make()).unwrap();
    assert_eq!(store::create(&f.path, make()).unwrap().id, first.id);
    let second = f.create();
    assert_ne!(first.id, second.id);
    let saved = reply::send(&f.path, f.input(&first, "A persistent note")).unwrap();
    assert_eq!(saved.conversation.revision, 2);
    assert_eq!(f.read(&first).messages[0].content, "A persistent note");
    let db = db::open(&f.path).unwrap();
    for table in ["sentinel_scans", "agent_runs", "agent_messages"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert!(
        db.execute("DELETE FROM projects WHERE id=901", []).is_err(),
        "conversation owns its project and blocks raw deletion"
    );
}
#[test]
fn conversation_scope_and_revision_checks_preserve_other_projects() {
    let f = Fixture::new();
    let item = f.create();
    assert!(store::read(
        &f.path,
        ReadInput {
            project_id: 902,
            conversation_id: item.id.clone(),
            before_sequence: None,
            limit: None
        }
    )
    .is_err());
    let saved = reply::send(&f.path, f.input(&item, "first")).unwrap();
    assert_eq!(
        reply::send(&f.path, f.input(&item, "stale")).err().unwrap(),
        "conversation_revision_conflict"
    );
    assert_eq!(f.read(&item).messages.len(), 1);
    assert!(store::delete(
        &f.path,
        DeleteInput {
            project_id: 902,
            conversation_id: item.id.clone(),
            expected_revision: saved.conversation.revision
        }
    )
    .is_err());
    assert_eq!(f.read(&item).conversation.revision, 2);
}
#[test]
fn search_is_literal_and_history_pages_are_complete_without_model() {
    let f = Fixture::new();
    let mut item = f.create();
    for text in ["alpha", "literal 10%_done", "gamma", "delta"] {
        item = reply::send(&f.path, f.input(&item, text))
            .unwrap()
            .conversation;
    }
    let query = |value: &str| {
        store::list(
            &f.path,
            ListInput {
                project_id: 901,
                query: Some(value.into()),
                include_archived: None,
                limit: None,
                offset: None,
            },
        )
        .unwrap()
    };
    assert_eq!(query("10%_").total, 1);
    assert_eq!(query("not present").total, 0);
    let page = store::read(
        &f.path,
        ReadInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            before_sequence: None,
            limit: Some(2),
        },
    )
    .unwrap();
    assert!(page.has_more);
    assert_eq!(
        page.messages.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        vec![3, 4]
    );
    let earlier = store::read(
        &f.path,
        ReadInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            before_sequence: Some(3),
            limit: Some(2),
        },
    )
    .unwrap();
    assert!(!earlier.has_more);
    assert_eq!(
        earlier
            .messages
            .iter()
            .map(|m| m.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}
#[test]
fn typed_links_reject_foreign_scope_and_conversation_delete_preserves_evidence() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute_batch("INSERT INTO sentinel_scans(id,project_id,status,task_name) VALUES('same',901,'draft','Original scan'),('foreign',902,'draft','Foreign');INSERT INTO assets(id,asset_key,title) VALUES(7001,'one','Asset');INSERT INTO project_assets(project_id,asset_id) VALUES(901,7001);").unwrap();
    let link = |kind: &str, target: &str| LinkInput {
        kind: kind.into(),
        target_id: target.into(),
    };
    assert!(store::set_links(
        &f.path,
        SetLinksInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: 1,
            links: vec![link("scan", "foreign")]
        }
    )
    .is_err());
    let saved = store::set_links(
        &f.path,
        SetLinksInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: 1,
            links: vec![link("scan", "same"), link("asset", "7001")],
        },
    )
    .unwrap();
    assert!(f.read(&item).links.iter().all(|l| l.available));
    db.execute("DELETE FROM sentinel_scans WHERE id='same'", [])
        .unwrap();
    let read = f.read(&item);
    assert_eq!(read.links.len(), 2);
    assert!(
        !read
            .links
            .iter()
            .find(|l| l.kind == "scan")
            .unwrap()
            .available
    );
    store::delete(
        &f.path,
        DeleteInput {
            project_id: 901,
            conversation_id: item.id,
            expected_revision: saved.revision,
        },
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM assets WHERE id=7001", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scans WHERE id='foreign'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn archived_project_is_readable_but_cannot_accept_new_messages() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute("UPDATE projects SET status='archived' WHERE id=901", [])
        .unwrap();
    assert!(!f.read(&item).reply_availability.available);
    assert_eq!(
        reply::send(&f.path, f.input(&item, "must not save"))
            .err()
            .unwrap(),
        "conversation_project_archived"
    );
    assert!(f.read(&item).messages.is_empty());
}
#[test]
fn orphan_recovery_requires_the_same_os_lock_and_never_reissues() {
    let f = Fixture::new();
    let item = f.create();
    let saved = reply::send(&f.path, f.input(&item, "question")).unwrap();
    let db = db::open(&f.path).unwrap();
    let id = Uuid::new_v4().to_string();
    db.execute("INSERT INTO conversation_replies(id,conversation_id,user_message_id,base_revision,status,created_at,updated_at) VALUES(?1,?2,?3,2,'running','t','t')",params![id,item.id,saved.user_message.id]).unwrap();
    let owner = store::lock(&f.path, &item).unwrap();
    assert_eq!(f.read(&item).latest_reply.unwrap().status, "running");
    drop(owner);
    let recovered = f.read(&item).latest_reply.unwrap();
    assert_eq!(recovered.status, "interrupted");
    assert!(recovered.outcome_unknown);
    assert_eq!(f.read(&item).messages.len(), 1);
}
#[test]
fn ignored_message_insert_is_not_acknowledged_or_revision_bumped() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute_batch("CREATE TRIGGER conversation_test_ignore BEFORE INSERT ON conversation_messages BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(reply::send(&f.path, f.input(&item, "not persisted")).is_err());
    assert_eq!(f.read(&item).conversation.revision, 1);
    assert!(f.read(&item).messages.is_empty());
}
#[test]
fn additive_schema_reentry_preserves_existing_rows() {
    let f = Fixture::new();
    let item = f.create();
    let saved = reply::send(&f.path, f.input(&item, "keep")).unwrap();
    let db = db::open(&f.path).unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    assert_eq!(f.read(&item).messages[0].id, saved.user_message.id);
}

#[test]
fn process_reply_capacity_releases_after_owner_drops() {
    let first = super::ReplyPermit::acquire().unwrap();
    let second = super::ReplyPermit::acquire().unwrap();
    assert!(
        matches!(super::ReplyPermit::acquire(),Err(error) if error=="conversation_reply_capacity")
    );
    drop(first);
    let replacement = super::ReplyPermit::acquire().unwrap();
    drop(second);
    drop(replacement);
}
