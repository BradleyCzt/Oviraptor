//! Upgrade real pre-page-rule tables through the full application initializer.
//! The fixture DDL is copied verbatim from 525f128, not built from current SCHEMA.
use super::*;
use crate::db;
use std::path::PathBuf;

struct HistoricalDatabase {
    root: PathBuf,
    path: PathBuf,
}

impl HistoricalDatabase {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-page-rule-history-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("oviraptor.sqlite3");
        let c = Connection::open(&path).unwrap();
        c.execute_batch(include_str!(
            "../tests/fixtures/page_isolation/legacy_525_schema.sql"
        ))
        .unwrap();
        c.execute_batch(
            "INSERT INTO projects(id,name) VALUES(9001,'Historical project'),(9002,'Control project');",
        )
        .unwrap();
        for id in 1..=8 {
            c.execute(
                "INSERT INTO assets(id,asset_key,host,link,title,status_code,probe_outcome) VALUES(?1,?2,?3,?4,'Welcome legacy','200','web_alive')",
                params![id, format!("history-{id}"), format!("history-{id}.invalid"), format!("https://history-{id}.invalid/")],
            )
            .unwrap();
            c.execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,?1),(9002,?1)",
                [id],
            )
            .unwrap();
        }
        c.execute_batch(
            "UPDATE project_assets SET note='人工保留的历史备注' WHERE project_id=9001 AND asset_id=1;
             UPDATE project_assets SET decision='confirmed',note='历史人工确认' WHERE project_id=9001 AND asset_id=2;
             UPDATE project_assets SET is_deleted=1,deleted_at='2020-01-02 03:04:05' WHERE project_id=9001 AND asset_id=3;
             UPDATE project_assets SET entry_opened_at='2020-02-03 04:05:06' WHERE project_id=9001 AND asset_id=4;
             INSERT INTO content_rules(id,keyword,normalized_keyword,source_asset_id,enabled) VALUES(71,'Legacy keyword','legacy keyword',1,0);",
        )
        .unwrap();
        Self { root, path }
    }

    fn upgrade(&self) -> Connection {
        assert_eq!(db::initialize(&self.root).unwrap(), self.path);
        db::open(&self.path).unwrap()
    }
}

impl Drop for HistoricalDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn membership_snapshot(c: &Connection) -> Vec<serde_json::Value> {
    let mut statement = c.prepare(
        "SELECT project_id,asset_id,decision,note,is_deleted,deleted_at,entry_opened_at FROM project_assets ORDER BY project_id,asset_id",
    ).unwrap();
    let rows = statement
        .query_map([], |r| {
            Ok(serde_json::json!([
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?
            ]))
        })
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

#[test]
fn page_rule_historical_database_full_initialize_preserves_manual_rows_and_legacy_catalog() {
    let f = HistoricalDatabase::new();
    let before = membership_snapshot(&Connection::open(&f.path).unwrap());
    let c = f.upgrade();
    assert_eq!(membership_snapshot(&c), before);
    assert!(list_on(&c, 9001).unwrap().is_empty());
    assert!(list_on(&c, 9002).unwrap().is_empty());
    assert_eq!(
        c.query_row(
            "SELECT keyword,enabled,source_asset_id FROM content_rules WHERE id=71",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            }
        )
        .unwrap(),
        ("Legacy keyword".into(), 0, 1)
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_page_rule_effects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(c);
    let reopened = f.upgrade();
    assert_eq!(membership_snapshot(&reopened), before);
    assert!(list_on(&reopened, 9001).unwrap().is_empty());
    assert_eq!(
        reopened
            .query_row(
                "SELECT COUNT(*) FROM content_rules WHERE id=71 AND enabled=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn page_rule_historical_partial_schema_install_completes_idempotently_without_reenabling() {
    let f = HistoricalDatabase::new();
    let c = Connection::open(&f.path).unwrap();
    // An interrupted execute_batch may have created the rules table but not
    // its index or the effects table. Reuse the actual first CREATE statement.
    let rules_ddl = include_str!("page_isolation.schema.sql")
        .split("CREATE INDEX")
        .next()
        .unwrap();
    c.execute_batch(rules_ddl).unwrap();
    c.execute("INSERT INTO asset_page_rules(id,project_id,kind,value,source_asset_id,enabled) VALUES(51,9001,'title_contains','welcome',1,0)", []).unwrap();
    let before = membership_snapshot(&c);
    drop(c);
    for _ in 0..2 {
        let c = f.upgrade();
        assert_eq!(membership_snapshot(&c), before);
        let rules = list_on(&c, 9001).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, 51);
        assert!(!rules[0].enabled);
        assert_eq!(rules[0].example_title, "Welcome legacy");
        assert_eq!(c.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='index' AND name='idx_asset_page_rules_project'", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM asset_page_rule_effects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn page_rule_historical_probe_repair_preserves_human_ownership_and_investigation_state() {
    let f = HistoricalDatabase::new();
    let c = Connection::open(&f.path).unwrap();
    c.execute_batch(
        "UPDATE assets SET probe_outcome=CASE WHEN id%2=0 THEN 'blocked_content' ELSE 'web_abnormal' END;
         INSERT INTO asset_events(project_id,asset_id,event_type) VALUES(9001,4,'decision');
         INSERT INTO asset_ownership_decisions(project_id,asset_id,source) VALUES(9001,5,'manual');
         INSERT INTO sentinel_scans(id,project_id,status) VALUES('history-readonly',9001,'completed');
         INSERT INTO sentinel_targets(project_id,scan_id,asset_id,url,status) VALUES(9001,'history-readonly',6,'https://history-6.invalid/','completed');",
    ).unwrap();
    let before = membership_snapshot(&c);
    drop(c);
    let c = f.upgrade();
    for asset in 1..=6 {
        let actual: (String, String, i64) = c.query_row("SELECT decision,note,is_deleted FROM project_assets WHERE project_id=9001 AND asset_id=?1", [asset], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let row = before
            .iter()
            .find(|r| r[0] == 9001 && r[1] == asset)
            .unwrap();
        assert_eq!(
            serde_json::json!([actual.0, actual.1, actual.2]),
            serde_json::json!([row[2], row[3], row[4]]),
            "historical protected asset {asset}"
        );
    }
    // An opened entry alone is not human classification: id 4 is protected by
    // its decision event, while the ordinary automatic rows still migrate.
    assert_eq!(
        c.query_row(
            "SELECT decision FROM project_assets WHERE project_id=9001 AND asset_id=7",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "not_applicable"
    );
    assert_eq!(
        c.query_row(
            "SELECT decision FROM project_assets WHERE project_id=9001 AND asset_id=8",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "rejected"
    );
    assert_eq!(
        c.query_row(
            "SELECT value FROM app_settings WHERE key='probe_classification_version'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "3"
    );
    drop(c);
    let reopened = f.upgrade();
    assert_eq!(
        reopened
            .query_row(
                "SELECT note FROM project_assets WHERE project_id=9001 AND asset_id=1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "人工保留的历史备注"
    );
}
