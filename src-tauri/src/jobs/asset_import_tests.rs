use super::*;

#[path = "asset_import_lock_tests.rs"]
mod asset_import_lock_tests;

#[test]
fn page_rule_production_import_isolates_future_ports_and_keeps_original_rule_state_on_reentry() {
    let mut f = Fixture::new();
    f.import(9001, &[("集团", "http://future.example:8001/", "web_alive", "", "[]")]);
    let asset: i64 = f.connection.query_row("SELECT id FROM assets", [], |r| r.get(0)).unwrap();
    f.connection.execute("UPDATE assets SET title='OneNet Welcome'", []).unwrap();
    let rule = crate::page_isolation::RuleInput { project_id: 9001, source_asset_id: asset, kind: "title_contains".into(), value: "OneNet".into() };
    let preview = crate::page_isolation::preview_on(&f.connection, rule).unwrap();
    let db_path = PathBuf::from(f.connection.path().unwrap());
    crate::page_isolation::apply_on(&mut f.connection, &db_path, crate::page_isolation::ApplyInput { rule: preview.rule, preview_token: preview.preview_token }).unwrap();
    let path = f.root.join("future.csv");
    fs::write(&path,"company,asset_key,link,protocol,title\n集团,http://future.example:8001/,http://future.example:8001/,http,OneNet Welcome\n集团,http://future.example:9002/,http://future.example:9002/,http,OneNet Welcome\n").unwrap();
    for _ in 0..2 { import_csv_with_progress(&mut f.connection,9002,9001,&path,|_|{}).unwrap(); }
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM project_assets WHERE is_deleted=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM asset_page_rule_effects WHERE original_decision='pending' AND original_note=''",[],|r|r.get::<_,i64>(0)).unwrap(),2);
}

#[test]
fn page_rule_production_import_failure_rolls_back_observations_associations_and_rule_effects() {
    let mut f = Fixture::new();
    f.import(9001, &[("集团", "http://source.example/", "web_alive", "", "[]")]);
    let asset: i64 = f.connection.query_row("SELECT id FROM assets", [], |r| r.get(0)).unwrap();
    f.connection.execute("UPDATE assets SET title='OneNet Welcome'", []).unwrap();
    let rule = crate::page_isolation::RuleInput { project_id:9001, source_asset_id:asset, kind:"title_contains".into(), value:"OneNet".into() };
    let preview=crate::page_isolation::preview_on(&f.connection,rule).unwrap();
    let db_path=PathBuf::from(f.connection.path().unwrap());
    crate::page_isolation::apply_on(&mut f.connection,&db_path,crate::page_isolation::ApplyInput{rule:preview.rule,preview_token:preview.preview_token}).unwrap();
    f.connection.execute_batch("CREATE TRIGGER fixture_page_import_fail BEFORE INSERT ON asset_page_rule_effects WHEN NEW.asset_id<>1 BEGIN SELECT RAISE(ABORT,'fixture_page_import_failure'); END;").unwrap();
    let path=f.root.join("new.csv");fs::write(&path,"company,asset_key,link,protocol,title\n集团,http://new.example/,http://new.example/,http,OneNet Welcome\n").unwrap();
    assert!(import_csv_with_progress(&mut f.connection,9002,9001,&path,|_|{}).unwrap_err().contains("fixture_page_import_failure"));
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM project_assets",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM asset_page_rule_effects",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}

struct Fixture {
    root: PathBuf,
    connection: rusqlite::Connection,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-asset-import-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(9001,'合成集团')", [])
            .unwrap();
        connection.execute("INSERT INTO runs(id,project_id,name) VALUES(9001,9001,'合成首轮'),(9002,9001,'合成复测')", []).unwrap();
        Self { root, connection }
    }
    fn import(&mut self, run: i64, rows: &[(&str, &str, &str, &str, &str)]) -> ImportResult {
        let path = self.root.join("input.csv");
        let mut writer = csv::Writer::from_path(&path).unwrap();
        writer
            .write_record([
                "company",
                "asset_key",
                "link",
                "protocol",
                "probe_outcome",
                "auto_excluded",
                "exclude_reason",
                "discovery_sources_json",
            ])
            .unwrap();
        for (label, url, outcome, excluded, sources) in rows {
            writer
                .write_record([
                    label,
                    url,
                    url,
                    "http",
                    outcome,
                    excluded,
                    "unreachable:fixture",
                    sources,
                ])
                .unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        import_csv_with_progress(&mut self.connection, run, 9001, &path, |_| {}).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn partial_collection_never_marks_historical_assets_missing() {
    let mut f = Fixture::new();
    f.import(
        9001,
        &[(
            "集团",
            "http://historical.example/",
            "web_alive",
            "",
            &json_source("集团", "company"),
        )],
    );
    let path = f.root.join("oviraptor.sqlite3");
    for summary in [
        serde_json::json!({"complete":true,"coverage_complete":false}),
        serde_json::json!({"complete":false,"coverage_complete":true}),
        serde_json::json!({}),
    ] {
        assert!(!mark_missing(&path, 9001, 9002, &summary).unwrap());
    }
    let count = || {
        f.connection
            .query_row(
                "SELECT count(*) FROM asset_events WHERE event_type='missing'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(count(), 0);
    assert!(mark_missing(
        &path,
        9001,
        9002,
        &serde_json::json!({"complete":true,"coverage_complete":true})
    )
    .unwrap());
    assert_eq!(count(), 1);
    assert!(mark_missing(
        &path,
        9001,
        9002,
        &serde_json::json!({"complete":true,"coverage_complete":true})
    )
    .unwrap());
    assert_eq!(count(), 1);
}

#[test]
fn failed_probe_clears_old_response_proof_and_global_group_trace_without_losing_manual_state() {
    let mut f = Fixture::new();
    f.import(
        9001,
        &[(
            "集团",
            "http://original.example/",
            "web_alive",
            "",
            &json_source("集团", "company"),
        )],
    );
    let old=serde_json::json!({"probe_response_fingerprint":"a".repeat(64),"body_sha256":"b".repeat(64),"probe_body_complete":"true","probe_body_bytes":"7","response_group_representative_asset_key":"foreign-project","response_group_members_json":"old-group"}).to_string();
    f.connection
        .execute("UPDATE assets SET extra_json=?1", [old])
        .unwrap();
    f.connection.execute("UPDATE project_assets SET decision='confirmed',note='人工保留',entry_opened_at='old-opened'",[]).unwrap();
    f.import(
        9002,
        &[(
            "集团",
            "http://original.example/",
            "unreachable",
            "1",
            &json_source("集团", "company"),
        )],
    );
    let raw: String = f
        .connection
        .query_row("SELECT extra_json FROM assets", [], |r| r.get(0))
        .unwrap();
    let extra: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(extra["probe_response_fingerprint"], "");
    assert_eq!(extra["probe_body_complete"], "");
    assert_eq!(extra["body_sha256"], "");
    assert!(extra
        .get("response_group_representative_asset_key")
        .is_none());
    assert!(extra.get("response_group_members_json").is_none());
    let state: (String, String, i64, String) = f
        .connection
        .query_row(
            "SELECT decision,note,is_deleted,entry_opened_at FROM project_assets",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "confirmed".into(),
            "人工保留".into(),
            0,
            "old-opened".into()
        )
    );
}

#[test]
fn asset_reimport_reuses_legacy_row_and_preserves_manual_review_seen_and_history() {
    let mut fixture = Fixture::new();
    fixture.connection.execute("INSERT INTO assets(id,asset_key,company,link,protocol,canonical_key,first_seen,last_alive) VALUES(9001,'legacy-key','集团','HTTP://EXAMPLE.com:80/','http','http://example.com:80','2020-first','2020-alive')", []).unwrap();
    fixture.connection.execute("INSERT INTO project_assets(project_id,asset_id,decision,note,first_seen,entry_opened_at,last_run_id) VALUES(9001,9001,'confirmed','人工确认保留','2020-project-first','2020-opened',9001)", []).unwrap();
    let company = json_source("集团", "company");
    let domain = json_source("example.com", "domain");
    let result = fixture.import(
        9002,
        &[
            ("集团", "http://example.com/", "web_alive", "", &company),
            (
                "example.com",
                "http://example.com:80/#x",
                "unreachable",
                "1",
                &domain,
            ),
        ],
    );
    assert_eq!(result.inserted, 0);
    let state: (i64, String, String, i64, String, String, i64) = fixture.connection.query_row(
        "SELECT pa.asset_id,pa.decision,pa.note,pa.is_deleted,pa.first_seen,pa.entry_opened_at,pa.last_run_id FROM project_assets pa WHERE pa.project_id=9001", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    assert_eq!(
        state,
        (
            9001,
            "confirmed".into(),
            "人工确认保留".into(),
            0,
            "2020-project-first".into(),
            "2020-opened".into(),
            9002
        )
    );
    let (count, first, outcome, extra): (i64, String, String, String) = fixture
        .connection
        .query_row(
            "SELECT COUNT(*),first_seen,probe_outcome,extra_json FROM assets",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (count, first.as_str(), outcome.as_str()),
        (1, "2020-first", "unreachable")
    );
    let extra: Value = serde_json::from_str(&extra).unwrap();
    assert_eq!(extra["discovery_source_count"], "2");
    let sources: Vec<Value> =
        serde_json::from_str(extra["discovery_sources_json"].as_str().unwrap()).unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(
        fixture
            .connection
            .query_row(
                "SELECT COUNT(*) FROM asset_events WHERE event_type='new'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

fn json_source(label: &str, kind: &str) -> String {
    serde_json::json!([{"label":label,"kind":kind}]).to_string()
}

#[test]
fn asset_reimport_never_resurrects_manually_archived_or_rejected_rows() {
    let mut fixture = Fixture::new();
    fixture.import(
        9001,
        &[(
            "集团",
            "http://example.com/",
            "web_alive",
            "",
            &json_source("集团", "company"),
        )],
    );
    fixture.connection.execute("UPDATE project_assets SET decision='rejected',note='人工排除',is_deleted=1,deleted_at='2020-archived',entry_opened_at='2020-opened' WHERE project_id=9001", []).unwrap();
    fixture.import(
        9002,
        &[(
            "example.com",
            "http://EXAMPLE.com:80/",
            "web_alive",
            "",
            &json_source("example.com", "domain"),
        )],
    );
    let state: (i64, String, String, i64, String) = fixture.connection.query_row(
        "SELECT COUNT(*),decision,note,is_deleted,deleted_at FROM project_assets WHERE project_id=9001", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        state,
        (
            1,
            "rejected".into(),
            "人工排除".into(),
            1,
            "2020-archived".into()
        )
    );
}

#[test]
fn asset_reimport_keeps_distinct_case_sensitive_paths_ports_and_schemes() {
    let mut fixture = Fixture::new();
    let source = json_source("集团", "company");
    let urls = [
        "http://example.com/A",
        "http://example.com/a",
        "http://example.com:8080/A",
        "https://example.com/A",
    ];
    let rows = urls
        .iter()
        .map(|url| ("集团", *url, "web_alive", "", source.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(fixture.import(9001, &rows).inserted, 4);
    assert_eq!(fixture.import(9002, &rows).inserted, 0);
    assert_eq!(
        fixture
            .connection
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[test]
fn asset_automatic_quarantine_still_applies_to_unreviewed_rows_and_can_recover() {
    let mut fixture = Fixture::new();
    let source = json_source("集团", "company");
    fixture.import(
        9001,
        &[("集团", "http://example.com/", "unreachable", "1", &source)],
    );
    assert_eq!(
        fixture
            .connection
            .query_row("SELECT is_deleted FROM project_assets", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    fixture.import(
        9002,
        &[(
            "example.com",
            "http://example.com/",
            "web_alive",
            "",
            &source,
        )],
    );
    let state: (i64, String, String, i64) = fixture
        .connection
        .query_row(
            "SELECT COUNT(*),decision,note,is_deleted FROM project_assets",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, (1, "pending".into(), "".into(), 0));
}

#[test]
fn asset_manual_events_override_stale_automatic_notes_on_recovery() {
    let mut fixture = Fixture::new();
    let source = json_source("集团", "company");
    for (url, event, decision, deleted) in [
        (
            "http://example.com/archived",
            "archived",
            "not_applicable",
            1,
        ),
        ("http://example.com/confirmed", "decision", "confirmed", 0),
    ] {
        fixture.import(9001, &[("集团", url, "unreachable", "", &source)]);
        let id: i64 = fixture
            .connection
            .query_row("SELECT id FROM assets WHERE link=?1", [url], |r| r.get(0))
            .unwrap();
        fixture.connection.execute("UPDATE project_assets SET decision=?1,is_deleted=?2,deleted_at=CASE WHEN ?2=1 THEN 'manual-archive-time' ELSE NULL END WHERE asset_id=?3", params![decision, deleted, id]).unwrap();
        fixture.connection.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(9001,?1,?2,'合成人工操作')", params![id, event]).unwrap();
        fixture.import(9002, &[("example.com", url, "web_alive", "", &source)]);
        let actual: (String, i64, String) = fixture
            .connection
            .query_row(
                "SELECT decision,is_deleted,note FROM project_assets WHERE asset_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(actual.0, decision);
        assert_eq!(actual.1, deleted);
        assert!(
            actual.2.starts_with("系统自动Web分类："),
            "UI left this old note; event must protect the manual state"
        );
    }
}

#[test]
fn actual_database_reprobe_preserves_discovery_sources_without_inventing_a_seed() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0; 4096];
        assert!(stream.read(&mut buffer).unwrap() > 0);
        let body = "<html><title>Company portal</title><body>Welcome to the portal</body></html>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let mut fixture = Fixture::new();
    let url = format!("http://{address}/");
    fixture.import(
        9001,
        &[
            (
                "集团",
                &url,
                "web_alive",
                "",
                &json_source("集团", "company"),
            ),
            (
                "example.com",
                &url,
                "web_alive",
                "",
                &json_source("example.com", "domain"),
            ),
        ],
    );
    let before: String = fixture
        .connection
        .query_row(
            "SELECT json_extract(extra_json,'$.discovery_sources_json') FROM assets",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let db_path = PathBuf::from(fixture.connection.path().unwrap());
    let refined = fixture.root.join("refined");
    assert_eq!(write_reprobe_sources(&db_path, 9001, &refined).unwrap(), 1);
    let output = fixture.root.join("probe");
    native_workers::probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &output,
        native_workers::ProbeOptions {
            workers: 2,
            timeout: Duration::from_secs(2),
            total_timeout: std::time::Duration::from_secs(8),
            retries: 0,
            max_body_bytes: 4096,
            include_other: false,
            include_weak: false,
            allow_private: true,
            strict_tls: false,
            scheme_fallback: false,
            content_threshold: 12,
            replace_default_content_rules: false,
            gambling_keywords: vec![],
            porn_keywords: vec![],
            negative_keywords: vec![],
            custom_keywords: vec![],
            priority_rate: 1000.0,
            other_rate: 1000.0,
            per_host_interval: Duration::ZERO,
        },
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    server.join().unwrap();
    import_csv_with_progress(
        &mut fixture.connection,
        9002,
        9001,
        &output.join("P1_web_alive.csv"),
        |_| {},
    )
    .unwrap();
    let (after, count): (String, String) = fixture.connection.query_row("SELECT json_extract(extra_json,'$.discovery_sources_json'),json_extract(extra_json,'$.discovery_source_count') FROM assets", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(after, before);
    assert_eq!(count, "2");
}
