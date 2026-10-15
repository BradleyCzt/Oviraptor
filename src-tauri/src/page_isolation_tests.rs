use super::*;
use crate::db;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    c: Connection,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-page-rules-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let c = db::open(&path).unwrap();
        c.execute(
            "INSERT INTO projects(id,name) VALUES(9001,'规则项目'),(9002,'另一项目')",
            [],
        )
        .unwrap();
        for id in 1..=6 {
            let proof = serde_json::json!({"body_sha256":if id==6{"b".repeat(64)}else{"a".repeat(64)},"probe_body_complete":if id==5{""}else{"true"},"probe_body_truncated":"false","probe_body_bytes":"80","probe_body_limit":"4096","probe_status_code":if id==2{"403"}else{"200"},"probe_content_type":if id==2{"text/plain"}else{"text/html"}});
            c.execute("INSERT INTO assets(id,asset_key,title,link,ip,port,status_code,probe_outcome,extra_json) VALUES(?1,?2,'OneNet Welcome',?3,?4,?5,?6,?7,?8)",params![id,format!("asset-{id}"),format!("http://192.0.2.{id}:{}/",8000+id),format!("192.0.2.{id}"),(8000+id).to_string(),if id==2{"403"}else{"200"},if id==2{"web_restricted"}else{"web_alive"},proof.to_string()]).unwrap();
            c.execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,?1),(9002,?1)",
                [id],
            )
            .unwrap();
        }
        c.execute(
            "UPDATE project_assets SET decision='confirmed' WHERE project_id=9001 AND asset_id=3",
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE project_assets SET note='人工备注' WHERE project_id=9001 AND asset_id=4",
            [],
        )
        .unwrap();
        c.execute("UPDATE project_assets SET entry_opened_at='original-opened' WHERE project_id=9001 AND asset_id=2",[]).unwrap();
        Self { root, path, c }
    }
    fn input(&self, kind: &str) -> RuleInput {
        RuleInput {
            project_id: 9001,
            source_asset_id: 1,
            kind: kind.into(),
            value: if kind == "title_contains" {
                "Welcome".into()
            } else {
                "client-substitute-is-ignored".into()
            },
        }
    }
    fn apply(&mut self, kind: &str) -> ApplyResult {
        let rule = self.input(kind);
        let preview = preview_on(&self.c, rule).unwrap();
        apply_on(
            &mut self.c,
            &self.path,
            ApplyInput {
                rule: preview.rule,
                preview_token: preview.preview_token,
            },
        )
        .unwrap()
    }
    fn deleted(&self, project: i64) -> i64 {
        self.c
            .query_row(
                "SELECT COUNT(*) FROM project_assets WHERE project_id=?1 AND is_deleted=1",
                [project],
                |r| r.get(0),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn page_rule_body_refuses_truncated_partial_invalid_and_unproven_payloads() {
    let f = Fixture::new();
    for (field, value) in [
        ("probe_body_truncated", "true"),
        ("probe_body_bytes", "80trailing"),
        ("probe_body_limit", "4096trailing"),
        ("probe_body_complete", ""),
        ("probe_content_range", "bytes 0-79/1000"),
        ("probe_content_encoding", "gzip"),
        ("probe_error", "failed body read"),
    ] {
        let original: String =
            f.c.query_row("SELECT extra_json FROM assets WHERE id=1", [], |r| r.get(0))
                .unwrap();
        let mut extra: serde_json::Value = serde_json::from_str(&original).unwrap();
        extra[field] = value.into();
        f.c.execute(
            "UPDATE assets SET extra_json=?1 WHERE id=1",
            [extra.to_string()],
        )
        .unwrap();
        assert!(
            preview_on(&f.c, f.input("body_sha256")).is_err(),
            "accepted {field}={value}"
        );
        f.c.execute("UPDATE assets SET extra_json=?1 WHERE id=1", [original])
            .unwrap();
    }
    f.c.execute("UPDATE assets SET extra_json='malformed' WHERE id=5", [])
        .unwrap();
    assert_eq!(
        preview_on(&f.c, f.input("title_contains")).unwrap().matched,
        6
    );
}

#[test]
fn page_rule_authorization_and_manual_history_are_project_scoped_and_protected() {
    let f = Fixture::new();
    f.c.execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,exposure_eligible) VALUES(9001,2,1),(9002,1,1)",[]).unwrap();
    let first = preview_on(&f.c, f.input("body_sha256")).unwrap();
    assert_eq!((first.eligible, first.protected), (1, 3));
    f.c.execute(
        "INSERT INTO asset_events(project_id,asset_id,event_type) VALUES(9001,1,'restored')",
        [],
    )
    .unwrap();
    let after = preview_on(&f.c, f.input("body_sha256")).unwrap();
    assert_eq!((after.eligible, after.protected), (0, 4));
}

#[test]
fn page_rule_late_restore_failure_rolls_back_rule_disable_and_every_restoration() {
    let mut f = Fixture::new();
    let applied = f.apply("body_sha256");
    f.c.execute_batch("CREATE TRIGGER fixture_restore_fail BEFORE UPDATE ON project_assets WHEN NEW.project_id=9001 AND NEW.asset_id=2 AND NEW.is_deleted=0 BEGIN SELECT RAISE(ABORT,'fixture_restore_failure'); END;").unwrap();
    assert!(disable_on(&mut f.c, &f.path, 9001, applied.rule_id)
        .unwrap_err()
        .contains("fixture_restore_failure"));
    assert!(list_on(&f.c, 9001).unwrap()[0].enabled);
    assert_eq!(f.deleted(9001), 2);
    assert_eq!(
        f.c.query_row(
            "SELECT COUNT(*) FROM asset_page_rule_effects WHERE active=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(f.c.is_autocommit());
}

#[test]
fn page_rule_body_ignores_ip_port_status_mime_and_preserves_other_project_and_manual_state() {
    let mut f = Fixture::new();
    let before: Vec<(i64, String, String)> = {
        let mut s =
            f.c.prepare("SELECT id,title,extra_json FROM assets ORDER BY id")
                .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let p = preview_on(&f.c, f.input("body_sha256")).unwrap();
    assert_eq!((p.matched, p.eligible, p.protected), (4, 2, 2));
    assert_eq!(p.rule.value, "a".repeat(64));
    let result = f.apply("body_sha256");
    assert_eq!((result.isolated, result.protected), (2, 2));
    assert_eq!(f.deleted(9001), 2);
    assert_eq!(f.deleted(9002), 0);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&result.backup_path).unwrap()).unwrap();
    assert_eq!(saved["inventory"].as_array().unwrap().len(), 4);
    let after: Vec<(i64, String, String)> = {
        let mut s =
            f.c.prepare("SELECT id,title,extra_json FROM assets ORDER BY id")
                .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(before, after);
    assert_eq!(
        f.c.query_row(
            "SELECT entry_opened_at FROM project_assets WHERE project_id=9001 AND asset_id=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "original-opened"
    );
    let undo = disable_on(&mut f.c, &f.path, 9001, result.rule_id).unwrap();
    assert_eq!(undo.restored, 2);
    assert_eq!(f.deleted(9001), 0);
    assert_eq!(f.c.query_row("SELECT COUNT(*) FROM project_assets WHERE project_id=9001 AND decision='pending' AND note=''",[],|r|r.get::<_,i64>(0)).unwrap(),4);
}
#[test]
fn page_rule_title_catches_legacy_and_variants_but_treats_wildcards_literally() {
    let mut f = Fixture::new();
    let applied = f.apply("title_contains");
    assert_eq!(applied.isolated, 4);
    assert_eq!(f.deleted(9001), 4);
    disable_on(&mut f.c, &f.path, 9001, applied.rule_id).unwrap();
    f.c.execute("UPDATE assets SET title='Welcome %_literal' WHERE id=6", [])
        .unwrap();
    let p = preview_on(
        &f.c,
        RuleInput {
            project_id: 9001,
            source_asset_id: 6,
            kind: "title_contains".into(),
            value: "%_".into(),
        },
    )
    .unwrap();
    assert_eq!((p.matched, p.eligible), (1, 1));
}
#[test]
fn page_rule_stale_preview_wrong_scope_unknown_proof_and_archived_project_are_rejected() {
    let mut f = Fixture::new();
    let p = preview_on(&f.c, f.input("title_contains")).unwrap();
    f.c.execute(
        "UPDATE project_assets SET decision='confirmed' WHERE project_id=9001 AND asset_id=2",
        [],
    )
    .unwrap();
    assert!(apply_on(
        &mut f.c,
        &f.path,
        ApplyInput {
            rule: p.rule,
            preview_token: p.preview_token
        }
    )
    .unwrap_err()
    .contains("重新预览"));
    assert_eq!(f.deleted(9001), 0);
    assert!(list_on(&f.c, 9001).unwrap().is_empty());
    assert!(!f.root.join("page-rule-backups").exists());
    let mut bad = f.input("body_sha256");
    bad.source_asset_id = 5;
    assert!(preview_on(&f.c, bad).unwrap_err().contains("完整正文"));
    let mut wrong = f.input("title_contains");
    wrong.project_id = 9003;
    assert!(preview_on(&f.c, wrong).is_err());
    f.c.execute(
        "DELETE FROM project_assets WHERE project_id=9002 AND asset_id=1",
        [],
    )
    .unwrap();
    let mut foreign = f.input("title_contains");
    foreign.project_id = 9002;
    assert!(preview_on(&f.c, foreign).unwrap_err().contains("当前项目"));
    f.c.execute("UPDATE projects SET status='archived' WHERE id=9001", [])
        .unwrap();
    assert!(preview_on(&f.c, f.input("title_contains")).is_err());
}
#[test]
fn page_rule_backup_failure_and_late_sql_failure_leave_no_partial_rule_or_isolation() {
    let mut f = Fixture::new();
    let p = preview_on(&f.c, f.input("title_contains")).unwrap();
    fs::write(f.root.join("page-rule-backups"), "fixture").unwrap();
    assert!(apply_on(
        &mut f.c,
        &f.path,
        ApplyInput {
            rule: p.rule,
            preview_token: p.preview_token
        }
    )
    .is_err());
    assert_eq!(f.deleted(9001), 0);
    assert!(list_on(&f.c, 9001).unwrap().is_empty());
    fs::remove_file(f.root.join("page-rule-backups")).unwrap();
    f.c.execute_batch("CREATE TRIGGER fixture_fail_page_effect BEFORE INSERT ON asset_page_rule_effects WHEN NEW.asset_id=2 BEGIN SELECT RAISE(ABORT,'fixture_page_rule_failure'); END;").unwrap();
    let p = preview_on(&f.c, f.input("title_contains")).unwrap();
    assert!(apply_on(
        &mut f.c,
        &f.path,
        ApplyInput {
            rule: p.rule,
            preview_token: p.preview_token
        }
    )
    .unwrap_err()
    .contains("fixture_page_rule_failure"));
    assert_eq!(f.deleted(9001), 0);
    assert!(list_on(&f.c, 9001).unwrap().is_empty());
    assert_eq!(
        f.c.query_row("SELECT COUNT(*) FROM asset_page_rule_effects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(f.c.is_autocommit());
}
#[test]
fn page_rule_overlapping_rules_transfer_ownership_and_restore_original_state_only_after_last_rule()
{
    let mut f = Fixture::new();
    let body = f.apply("body_sha256");
    let title = f.apply("title_contains");
    let first = disable_on(&mut f.c, &f.path, 9001, body.rule_id).unwrap();
    assert_eq!((first.restored, first.retained), (0, 2));
    assert_eq!(f.deleted(9001), 4);
    let last = disable_on(&mut f.c, &f.path, 9001, title.rule_id).unwrap();
    assert_eq!((last.restored, last.retained), (4, 0));
    assert_eq!(f.deleted(9001), 0);
    assert!(disable_on(&mut f.c, &f.path, 9002, title.rule_id).is_err());
}
#[test]
fn page_rule_manual_override_and_owned_restore_are_not_overwritten() {
    let mut f = Fixture::new();
    let result = f.apply("body_sha256");
    f.c.execute("UPDATE project_assets SET decision='confirmed',note='人工接管' WHERE project_id=9001 AND asset_id=1",[]).unwrap();
    f.c.execute(
        "INSERT INTO asset_events(project_id,asset_id,event_type) VALUES(9001,1,'decision')",
        [],
    )
    .unwrap();
    let tx =
        f.c.transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
    restore_manual_on(&tx, 9001, 2).unwrap();
    tx.execute("UPDATE project_assets SET is_deleted=0,deleted_at=NULL WHERE project_id=9001 AND asset_id=2",[]).unwrap();
    tx.execute(
        "INSERT INTO asset_events(project_id,asset_id,event_type) VALUES(9001,2,'restored')",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let rules = enabled_on(&f.c, 9001).unwrap();
    let tx = f.c.transaction().unwrap();
    apply_import_on(&tx, 9001, 2, &rules).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.deleted(9001), 1);
    let undo = disable_on(&mut f.c, &f.path, 9001, result.rule_id).unwrap();
    assert_eq!((undo.restored, undo.retained), (0, 1));
    assert_eq!(
        f.c.query_row(
            "SELECT note FROM project_assets WHERE project_id=9001 AND asset_id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "人工接管"
    );
}

#[test]
fn page_rule_same_applied_note_human_reconfirmation_survives_explicit_restore() {
    let mut f = Fixture::new();
    f.apply("body_sha256");
    // Reaffirm the current rejected state and identical automatic text. The
    // explicit human event, rather than a text difference, transfers ownership.
    f.c.execute(
        "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(9001,1,'decision','same note explicitly confirmed')",
        [],
    ).unwrap();
    let expected: (String, String) = f.c.query_row(
        "SELECT decision,note FROM project_assets WHERE project_id=9001 AND asset_id=1",
        [], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    let tx = f.c.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
    restore_manual_on(&tx, 9001, 1).unwrap();
    tx.execute("UPDATE project_assets SET is_deleted=0,deleted_at=NULL WHERE project_id=9001 AND asset_id=1", []).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.c.query_row("SELECT decision,note FROM project_assets WHERE project_id=9001 AND asset_id=1", [], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap(),expected);
    assert_eq!(f.c.query_row("SELECT active FROM asset_page_rule_effects WHERE project_id=9001 AND asset_id=1", [], |r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn page_rule_multi_statement_import_and_restore_require_the_original_transaction() {
    let mut f = Fixture::new();
    f.apply("body_sha256");
    let rules = enabled_on(&f.c, 9001).unwrap();
    let before = f.c.total_changes();
    assert!(apply_import_on(&f.c,9001,1,&rules).unwrap_err().contains("原资产事务"));
    assert!(restore_manual_on(&f.c,9001,1).unwrap_err().contains("原资产事务"));
    assert_eq!(f.c.total_changes(),before);
    assert_eq!(f.deleted(9001),2);
    assert_eq!(f.c.query_row("SELECT COUNT(*) FROM asset_page_rule_effects WHERE active=1", [], |r|r.get::<_,i64>(0)).unwrap(),2);
}

#[test]
fn page_rule_other_connection_reclassification_invalidates_preview_without_writes_or_backup() {
    let mut f = Fixture::new();
    let preview = preview_on(&f.c,f.input("title_contains")).unwrap();
    let other = db::open(&f.path).unwrap();
    other.execute("UPDATE project_assets SET decision='confirmed',note='人工接管' WHERE project_id=9001 AND asset_id=2", []).unwrap();
    let before = f.c.total_changes();
    assert!(apply_on(&mut f.c,&f.path,ApplyInput{rule:preview.rule,preview_token:preview.preview_token}).unwrap_err().contains("重新预览"));
    assert_eq!(f.c.total_changes(),before);
    assert!(list_on(&f.c,9001).unwrap().is_empty());
    assert_eq!(f.deleted(9001),0);
    assert!(!f.root.join("page-rule-backups").exists());
    assert_eq!(f.c.query_row("SELECT note FROM project_assets WHERE project_id=9001 AND asset_id=2", [], |r|r.get::<_,String>(0)).unwrap(),"人工接管");
}

#[test]
fn page_rule_preview_token_cannot_be_reused_for_another_shared_asset_project() {
    let mut f = Fixture::new();
    let preview = preview_on(&f.c,f.input("title_contains")).unwrap();
    let mut foreign = preview.rule;
    foreign.project_id = 9002; // The same physical source belongs to both projects.
    let before = f.c.total_changes();
    assert!(apply_on(&mut f.c,&f.path,ApplyInput{rule:foreign,preview_token:preview.preview_token}).unwrap_err().contains("重新预览"));
    assert_eq!(f.c.total_changes(),before);
    assert!(list_on(&f.c,9001).unwrap().is_empty());
    assert!(list_on(&f.c,9002).unwrap().is_empty());
    assert_eq!((f.deleted(9001),f.deleted(9002)),(0,0));
    assert!(!f.root.join("page-rule-backups").exists());
}
