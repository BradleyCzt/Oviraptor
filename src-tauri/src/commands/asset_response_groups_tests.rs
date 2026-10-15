use crate::{asset_models::{AssetPage, AssetQuery, FilterCondition}, commands::assets::list_assets_on, db};
use rusqlite::params;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    connection: rusqlite::Connection,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-response-display-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(9001,'Synthetic group'),(9002,'Other project')", []).unwrap();
        Self { root, connection }
    }

    fn add(&self, id: i64, project: i64, extra: &JsonValue) {
        self.connection.execute(
            "INSERT INTO assets(id,asset_key,link,host,status_code,probe_outcome,extra_json) VALUES(?1,?2,?3,?3,'200','web_alive',?4)",
            params![id,format!("asset-{id}"),format!("https://example.test/entry-{id}"),extra.to_string()],
        ).unwrap();
        self.connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
                params![project, id],
            )
            .unwrap();
    }

    fn list(&self, query: &AssetQuery) -> AssetPage {
        list_assets_on(&self.connection, query).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn proof(body: &str) -> JsonValue {
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    let content_type = "text/html; charset=utf-8";
    let identity =
        serde_json::json!(["http-full-body-v1", 200, body.len(), hash, content_type]).to_string();
    serde_json::json!({
        "probe_outcome":"web_alive", "probe_status_code":"200", "probe_body_complete":"true",
        "probe_body_truncated":"false", "probe_body_bytes":body.len().to_string(),
        "probe_body_limit":"4096", "probe_content_type":content_type, "body_sha256":hash,
        "probe_response_fingerprint":format!("{:x}",Sha256::digest(identity.as_bytes())),
        "probe_content_encoding":"", "probe_content_range":"", "probe_error":""
    })
}

fn folded_query() -> AssetQuery {
    AssetQuery {
        project_id: Some(9001),
        page: 1,
        page_size: 10,
        collapse_identical_responses: true,
        deleted_view: "active".into(),
        sort_by: "host".into(),
        sort_direction: "asc".into(),
        ..Default::default()
    }
}

#[test]
fn asset_response_display_defaults_raw_and_never_changes_persisted_members() {
    let fixture = Fixture::new();
    let extra = proof("<p>Same complete response</p>");
    for id in 1..=3 {
        fixture.add(id, 9001, &extra);
    }
    let mut query: AssetQuery = serde_json::from_str("{}").unwrap();
    assert!(!query.collapse_identical_responses);
    assert!(query.response_fingerprint.is_empty());
    query.project_id = Some(9001);
    let raw = fixture.list(&query);
    assert_eq!((raw.total, raw.member_total, raw.items.len()), (3, 3, 3));
    let folded = fixture.list(&folded_query());
    assert_eq!(
        (
            folded.total,
            folded.member_total,
            folded.items.len(),
            folded.summary.all
        ),
        (1, 3, 1, 3)
    );
    assert_eq!(folded.items[0].id, 1);
    assert_eq!(folded.items[0].extra["response_display_group_count"], 3);
    let stored: Vec<String> = fixture
        .connection
        .prepare("SELECT extra_json FROM assets ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored, vec![extra.to_string(); 3]);
    let members: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM project_assets WHERE is_deleted=0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(members, 3);
}

#[test]
fn asset_response_display_selects_representatives_after_every_filter() {
    let fixture = Fixture::new();
    let extra = proof("same");
    fixture.add(1, 9001, &extra);
    fixture.add(2, 9001, &extra);
    let mut query = folded_query();
    query.search = "entry-2".into();
    let searched = fixture.list(&query);
    assert_eq!((searched.total, searched.member_total), (1, 1));
    assert_eq!(searched.items[0].id, 2);
    assert_eq!(searched.items[0].extra["response_display_group_count"], 1);
    query.search.clear();
    fixture.connection.execute("UPDATE project_assets SET decision='confirmed',entry_opened_at='seen-at' WHERE asset_id=2",[]).unwrap();
    query.decision_view = "confirmed".into();
    query.entry_seen_view = "seen".into();
    let confirmed = fixture.list(&query);
    assert_eq!((confirmed.total, confirmed.member_total), (1, 1));
    assert_eq!(confirmed.items[0].id, 2);
    assert_eq!(confirmed.summary.all, 1);
}

#[test]
fn asset_response_display_and_member_expansion_never_cross_projects() {
    let fixture = Fixture::new();
    let extra = proof("same");
    for (id, project) in [(1, 9001), (2, 9001), (3, 9002), (4, 9002)] {
        fixture.add(id, project, &extra);
    }
    // The same global technical asset can belong to two projects.
    fixture
        .connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(9002,1)",
            [],
        )
        .unwrap();
    let mut query = folded_query();
    query.project_id = None;
    let all = fixture.list(&query);
    assert_eq!((all.total, all.member_total), (2, 5));
    assert_eq!(
        all.items
            .iter()
            .map(|asset| asset.project_id)
            .collect::<HashSet<_>>(),
        HashSet::from([9001, 9002])
    );
    query.collapse_identical_responses = false;
    query.response_fingerprint = extra["probe_response_fingerprint"].as_str().unwrap().into();
    assert!(list_assets_on(&fixture.connection, &query)
        .unwrap_err()
        .contains("指定项目"));
    query.project_id = Some(9001);
    let expanded = fixture.list(&query);
    assert_eq!((expanded.total, expanded.member_total), (2, 2));
    assert!(expanded.items.iter().all(|asset| asset.project_id == 9001));
}

#[test]
fn asset_response_display_preserves_manual_seen_scan_and_ownership_semantics() {
    let fixture = Fixture::new();
    let extra = proof("same");
    for id in 1..=9 {
        fixture.add(id, 9001, &extra);
    }
    fixture
        .connection
        .execute(
            "UPDATE project_assets SET decision='confirmed' WHERE asset_id=2",
            [],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE project_assets SET note='Human note' WHERE asset_id=3",
            [],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE project_assets SET is_deleted=1,deleted_at='archived' WHERE asset_id=4",
            [],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE project_assets SET entry_opened_at='seen' WHERE asset_id=5",
            [],
        )
        .unwrap();
    fixture.connection.execute("INSERT INTO sentinel_scans(id,project_id,status) VALUES('display-complete',9001,'completed')",[]).unwrap();
    fixture.connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,asset_id,url) VALUES(9001,'display-complete',6,'https://example.test/entry-6')",[]).unwrap();
    fixture.connection.execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,authorization_status,exposure_eligible) VALUES(9001,7,'confirmed','allowed',1),(9001,8,'confirmed','revoked',0)",[]).unwrap();
    let mut query = folded_query();
    query.deleted_view = "all".into();
    let page = fixture.list(&query);
    assert_eq!((page.total, page.member_total, page.summary.all), (8, 9, 9));
    assert_eq!(page.summary.confirmed, 1);
    assert_eq!(page.summary.sent_to_agent, 1);
    let ids = page
        .items
        .iter()
        .map(|asset| asset.id)
        .collect::<HashSet<_>>();
    assert_eq!(ids, HashSet::from([1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(
        page.items.iter().find(|asset| asset.id == 1).unwrap().extra
            ["response_display_group_count"],
        2
    );
    query.conditions = vec![FilterCondition {
        field: "sentinelScanCount".into(),
        operator: "equals".into(),
        value: "1".into(),
        join: "and".into(),
    }];
    let sent_only = fixture.list(&query);
    assert_eq!((sent_only.total, sent_only.member_total), (1, 1));
    assert_eq!(sent_only.items[0].id, 6);
    query.conditions[0].operator = "notEquals".into();
    let not_sent = fixture.list(&query);
    assert_eq!((not_sent.total, not_sent.member_total), (7, 8));
    assert!(not_sent.items.iter().all(|asset| asset.id != 6));
}

#[test]
fn asset_response_display_pagination_counts_groups_and_original_members() {
    let fixture = Fixture::new();
    for group in 0..12 {
        let extra = proof(&format!("complete response {group}"));
        fixture.add(group * 2 + 1, 9001, &extra);
        fixture.add(group * 2 + 2, 9001, &extra);
    }
    let mut query = folded_query();
    let first = fixture.list(&query);
    assert_eq!(
        (first.total, first.member_total, first.items.len()),
        (12, 24, 10)
    );
    query.page = 2;
    let second = fixture.list(&query);
    assert_eq!(
        (second.total, second.member_total, second.items.len()),
        (12, 24, 2)
    );
    let ids = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|asset| asset.id)
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), 12);
    assert!(first
        .items
        .iter()
        .chain(second.items.iter())
        .all(|asset| asset.extra["response_display_group_count"] == 2));
}

#[test]
fn asset_response_display_refuses_legacy_partial_empty_failed_and_inconsistent_proofs() {
    let fixture = Fixture::new();
    let extra = proof("same");
    fixture.add(1, 9001, &extra);
    fixture.add(2, 9001, &extra);
    let cases = [
        ("probe_body_complete", ""),
        ("probe_body_truncated", "true"),
        ("probe_body_bytes", "0"),
        ("probe_body_limit", "1"),
        ("probe_body_bytes", "4tail"),
        ("probe_status_code", "206"),
        ("probe_error", "read_failed"),
        ("probe_content_encoding", "gzip"),
        ("probe_content_range", "bytes 0-3/100"),
        ("body_sha256", "invalid"),
        ("probe_response_fingerprint", "invalid"),
        ("probe_outcome", "unreachable"),
    ];
    for (offset, (key, value)) in cases.iter().enumerate() {
        let mut invalid = extra.clone();
        invalid[*key] = JsonValue::String((*value).into());
        fixture.add(offset as i64 + 3, 9001, &invalid);
    }
    let legacy_id = cases.len() as i64 + 3;
    fixture.add(
        legacy_id,
        9001,
        &serde_json::json!({"body_sha256":extra["body_sha256"]}),
    );
    let mut query = folded_query();
    query.page_size = 100;
    let page = fixture.list(&query);
    assert_eq!(
        (page.total, page.member_total),
        (cases.len() as i64 + 2, cases.len() as i64 + 3)
    );
    assert_eq!(
        page.items
            .iter()
            .filter(|asset| asset.extra["response_display_group_count"] == 2)
            .count(),
        1
    );
    fixture
        .connection
        .execute(
            "UPDATE assets SET probe_outcome='unreachable' WHERE id=2",
            [],
        )
        .unwrap();
    assert_eq!(
        fixture.list(&query).total,
        page.total + 1,
        "stale successful metadata cannot hide a current failed asset"
    );
}

#[test]
fn asset_response_display_can_read_a_callers_snapshot_without_committing_it() {
    let fixture = Fixture::new();
    fixture.add(1, 9001, &proof("same"));
    fixture.connection.execute_batch("BEGIN").unwrap();
    let page = fixture.list(&folded_query());
    assert_eq!((page.total, page.member_total), (1, 1));
    assert!(!fixture.connection.is_autocommit());
    fixture.connection.execute_batch("ROLLBACK").unwrap();
    assert!(fixture.connection.is_autocommit());
}

#[test]
fn asset_response_display_never_folds_different_response_fields_with_reused_fingerprints() {
    let fixture = Fixture::new();
    let extra = proof("same");
    fixture.add(1, 9001, &extra);
    for (id, key, value) in [
        (2, "body_sha256", "ab".repeat(32)),
        (3, "probe_body_bytes", "5".into()),
        (4, "probe_content_type", "text/plain".into()),
        (5, "probe_status_code", "201".into()),
    ] {
        let mut changed = extra.clone();
        changed[key] = JsonValue::String(value);
        fixture.add(id, 9001, &changed);
    }
    fixture
        .connection
        .execute("UPDATE assets SET status_code='201' WHERE id=5", [])
        .unwrap();
    let page = fixture.list(&folded_query());
    assert_eq!((page.total, page.member_total), (5, 5));
    assert!(page
        .items
        .iter()
        .all(|asset| asset.extra["response_display_group_count"] == 1));
}
