//! Real SQLite/CSV import regressions, using a temporary application database.
use super::*;
use std::collections::BTreeMap;

const RECEIVED: &str = "2026-10-01T10:00:00Z";
const CHECKED: &str = "2026-10-01T11:00:00Z";
const NEWER: &str = "2026-10-02T10:00:00Z";

fn import_on(f: &mut Fixture, run: i64, project: i64, changes: &[(&str, &str)]) -> ImportResult {
    let mut row = BTreeMap::from([
        ("company", "合成集团"),
        ("asset_key", "same-provider-key"),
        ("link", "https://observation.example/"),
        ("host", "observation.example"),
        ("protocol", "https"),
        ("ip", "198.51.100.10"),
        ("port", "443"),
    ]);
    row.extend(changes.iter().copied());
    let path = f.root.join("observation.csv");
    let mut writer = csv::Writer::from_path(&path).unwrap();
    writer.write_record(row.keys()).unwrap();
    writer.write_record(row.values()).unwrap();
    writer.flush().unwrap();
    drop(writer);
    import_csv_with_progress(&mut f.connection, run, project, &path, |_| {}).unwrap()
}
fn import(f: &mut Fixture, run: i64, changes: &[(&str, &str)]) -> ImportResult {
    import_on(f, run, 9001, changes)
}
fn extra(f: &Fixture) -> Value {
    let raw: String = f
        .connection
        .query_row(
            "SELECT extra_json FROM assets ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn clocks(f: &Fixture) -> (String, String, Option<String>, String, String) {
    f.connection.query_row("SELECT a.first_seen,a.last_seen,a.last_alive,pa.first_seen,pa.last_seen FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=9001 ORDER BY a.id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
}

#[test]
fn replay_preserves_source_project_and_positive_probe_times_without_new_change_event() {
    let mut f = Fixture::new();
    let row = [
        ("collection_received_at", RECEIVED),
        ("probe_checked_at", CHECKED),
        ("probe_outcome", "web_alive"),
        ("probe_title", "Measured portal"),
        ("probe_status_code", "200"),
    ];
    assert_eq!(import(&mut f, 9001, &row).inserted, 1);
    assert_eq!(
        clocks(&f),
        (
            RECEIVED.into(),
            RECEIVED.into(),
            Some(CHECKED.into()),
            RECEIVED.into(),
            RECEIVED.into()
        )
    );
    let result = import(&mut f, 9002, &row);
    assert_eq!((result.inserted, result.updated), (0, 0));
    assert_eq!(
        clocks(&f),
        (
            RECEIVED.into(),
            RECEIVED.into(),
            Some(CHECKED.into()),
            RECEIVED.into(),
            RECEIVED.into()
        )
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT count(*) FROM asset_events WHERE event_type='changed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        f.connection
            .query_row("SELECT last_run_id FROM project_assets", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        9002
    );
}

#[test]
fn older_and_unzoned_source_receipts_cannot_replace_precise_network_facts() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", NEWER),
            ("ip", "198.51.100.20"),
            ("title", "New source"),
            ("collection_provider_observed_at", "2026-10-01 09:00:00"),
        ],
    );
    for receipt in [RECEIVED, "2026-10-03 12:00:00", ""] {
        let result = import(
            &mut f,
            9002,
            &[
                ("collection_received_at", receipt),
                ("ip", "198.51.100.99"),
                ("title", "Stale source"),
                ("collection_provider_observed_at", "2099-12-31 23:59:59"),
            ],
        );
        assert_eq!(result.updated, 0);
    }
    let state: (String, String) = f
        .connection
        .query_row("SELECT ip,title FROM assets", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(state, ("198.51.100.20".into(), "New source".into()));
    assert_eq!(extra(&f)["collection_received_at"], NEWER);
    assert_eq!(
        extra(&f)["collection_provider_observed_at"],
        "2026-10-01 09:00:00"
    );
    assert_eq!(clocks(&f).1, NEWER);
    assert_eq!(clocks(&f).4, NEWER);
}

#[test]
fn older_or_unknown_probe_cannot_erase_precise_response_proof_or_auto_trash_current_asset() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", NEWER),
            ("probe_outcome", "web_alive"),
            ("probe_title", "Fresh response"),
            ("probe_status_code", "200"),
            ("has_login", "1"),
            ("has_register", "0"),
            ("body_sha256", "fresh-body"),
            ("probe_response_fingerprint", "fresh-fingerprint"),
        ],
    );
    for checked in [CHECKED, "", "2026-10-03 12:00:00"] {
        assert_eq!(
            import(
                &mut f,
                9002,
                &[
                    ("collection_received_at", RECEIVED),
                    ("probe_checked_at", checked),
                    ("probe_outcome", "unreachable"),
                    ("probe_title", ""),
                    ("probe_status_code", ""),
                    ("auto_excluded", "1"),
                    ("exclude_reason", "stale failure"),
                    ("has_login", "0"),
                    ("has_register", "0")
                ]
            )
            .updated,
            0
        );
    }
    let state:(String,String,String,i64,i64)=f.connection.query_row("SELECT a.probe_outcome,a.title,a.status_code,a.has_login,pa.is_deleted FROM assets a JOIN project_assets pa ON pa.asset_id=a.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        state,
        (
            "web_alive".into(),
            "Fresh response".into(),
            "200".into(),
            1,
            0
        )
    );
    assert_eq!(extra(&f)["body_sha256"], "fresh-body");
    assert_eq!(extra(&f)["probe_checked_at"], NEWER);
    assert_eq!(clocks(&f).2, Some(NEWER.into()));
}

#[test]
fn negative_positive_negative_probes_only_advance_alive_from_actual_positive_clock() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "unreachable"),
        ],
    );
    assert_eq!(clocks(&f).2, None);
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", NEWER),
            ("probe_outcome", "web_alive"),
        ],
    );
    assert_eq!(clocks(&f).2, Some(NEWER.into()));
    assert_eq!(
        f.connection
            .query_row("SELECT decision FROM project_assets", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", "2026-10-03T10:00:00+00:00"),
            ("probe_outcome", "unreachable"),
        ],
    );
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "web_alive"),
        ],
    );
    assert_eq!(clocks(&f).2, Some(NEWER.into()));
    assert_eq!(
        f.connection
            .query_row("SELECT probe_outcome FROM assets", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unreachable"
    );
}

#[test]
fn late_positive_and_tcp_alive_record_original_alive_time_without_replacing_current_negative() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", NEWER),
            ("probe_outcome", "unreachable"),
        ],
    );
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "tcp_alive_non_http"),
        ],
    );
    assert_eq!(clocks(&f).2, Some(CHECKED.into()));
    assert_eq!(
        f.connection
            .query_row("SELECT probe_outcome FROM assets", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unreachable"
    );
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", "2026-10-03T10:00:00Z"),
            ("probe_outcome", "tcp_alive_non_http"),
        ],
    );
    assert_eq!(clocks(&f).2, Some("2026-10-03T10:00:00Z".into()));
    assert_eq!(
        f.connection
            .query_row("SELECT decision FROM project_assets", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "not_applicable"
    );
}

#[test]
fn new_source_ip_marks_retained_probe_history_and_new_negative_probe_clears_it() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "web_alive"),
            ("probe_input_url", "https://observation.example/"),
            ("probe_title", "Old measured title"),
        ],
    );
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", NEWER),
            ("ip", "198.51.100.20"),
            ("title", "Source candidate"),
        ],
    );
    let history = extra(&f);
    assert_eq!(history["probe_history_reason"], "endpoint_changed");
    assert_eq!(history["probe_source_ip"], "198.51.100.10");
    assert_eq!(history["probe_observed_ip"], ""); // DNS source IP is not a measured peer.
    assert_eq!(
        history["probe_observed_endpoint"],
        "https://observation.example/"
    );
    assert_eq!(history["probe_observed_endpoint_basis"], "probe_input_url");
    assert_eq!(history["probe_checked_at"], CHECKED);
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", NEWER),
            ("ip", "198.51.100.20"),
            ("title", "Source candidate"),
            ("probe_checked_at", "2026-10-02T11:00:00Z"),
            ("probe_outcome", "unreachable"),
            ("probe_input_url", "https://observation.example/"),
            ("probe_title", ""),
            ("probe_status_code", ""),
        ],
    );
    assert!(extra(&f).get("probe_history_reason").is_none());
    assert_eq!(extra(&f)["probe_source_ip"], "198.51.100.20");
    let state: (String, String, String) = f
        .connection
        .query_row("SELECT ip,title,probe_outcome FROM assets", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(
        state,
        ("198.51.100.20".into(), "".into(), "unreachable".into())
    );
}

#[test]
fn source_after_probe_on_same_ip_is_unverified_and_literal_probe_ip_is_explicit() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "web_alive"),
            ("probe_input_url", "https://198.51.100.10/"),
        ],
    );
    assert_eq!(extra(&f)["probe_observed_ip"], "198.51.100.10");
    import(&mut f, 9002, &[("collection_received_at", NEWER)]);
    assert_eq!(extra(&f)["probe_history_reason"], "collection_after_probe");
}

#[test]
fn unknown_legacy_csv_keeps_compatibility_without_minting_observation_clocks() {
    let mut f = Fixture::new();
    import(&mut f, 9001, &[("probe_outcome", "web_alive")]);
    assert_eq!(
        clocks(&f),
        ("".into(), "".into(), None, "".into(), "".into())
    );
    assert_eq!(extra(&f)["probe_history_reason"], "unknown_probe_time");
    import(&mut f, 9002, &[("probe_outcome", "unreachable")]);
    assert_eq!(
        f.connection
            .query_row("SELECT probe_outcome FROM assets", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unreachable"
    );
    assert_eq!(clocks(&f).2, None);
}

#[test]
fn equal_receipt_allows_refine_and_equal_probe_allows_optimizer_classification() {
    let mut f = Fixture::new();
    import(
        &mut f,
        9001,
        &[("collection_received_at", RECEIVED), ("score", "20")],
    );
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("score", "90"),
            ("review_tier", "P1_强归属且近期活跃"),
            ("attribution_reason", "fixture refinement"),
        ],
    );
    assert_eq!(
        f.connection
            .query_row("SELECT score FROM assets", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "90"
    );
    assert_eq!(extra(&f)["attribution_reason"], "fixture refinement");
    let probe = [
        ("collection_received_at", RECEIVED),
        ("probe_checked_at", CHECKED),
        ("probe_outcome", "web_alive"),
        ("score", "90"),
        ("review_tier", "P1_强归属且近期活跃"),
    ];
    import(&mut f, 9001, &probe);
    let mut optimized = probe.to_vec();
    optimized.extend([
        ("auto_excluded", "1"),
        ("exclude_reason", "optimizer fixture"),
    ]);
    import(&mut f, 9002, &optimized);
    assert_eq!(
        f.connection
            .query_row("SELECT is_deleted FROM project_assets", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(clocks(&f).1, RECEIVED);
    assert_eq!(clocks(&f).2, Some(CHECKED.into()));
}

#[test]
fn equal_clock_conflicts_choose_the_same_whole_source_and_probe_in_either_import_order() {
    let first = [
        ("collection_received_at", RECEIVED),
        ("ip", "198.51.100.11"),
        ("title", "first"),
        ("domain", "first.example"),
        ("probe_checked_at", CHECKED),
        ("probe_outcome", "web_alive"),
        ("probe_title", "measured first"),
        ("probe_status_code", "200"),
    ];
    let second = [
        ("collection_received_at", RECEIVED),
        ("ip", "198.51.100.22"),
        ("title", "second"),
        ("domain", ""),
        ("probe_checked_at", CHECKED),
        ("probe_outcome", "unreachable"),
        ("probe_title", ""),
        ("probe_status_code", ""),
    ];
    let mut outputs = Vec::new();
    for rows in [[&first, &second], [&second, &first]] {
        let mut f = Fixture::new();
        for row in rows {
            import(&mut f, 9001, row);
        }
        outputs.push(
            f.connection
                .query_row(
                    "SELECT ip,domain,title,status_code,probe_outcome,extra_json FROM assets",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                        ))
                    },
                )
                .unwrap(),
        );
    }
    assert_eq!(outputs[0], outputs[1]);
    let extra: Value = serde_json::from_str(&outputs[0].5).unwrap();
    assert_eq!(outputs[0].0, extra["ip"].as_str().unwrap());
    assert_eq!(outputs[0].4, extra["probe_outcome"].as_str().unwrap());
    if outputs[0].0 == "198.51.100.22" {
        assert_eq!(outputs[0].1, "");
    }
}

#[test]
fn colliding_legacy_raw_key_keeps_distinct_protocols_and_the_original_manual_row() {
    let mut f = Fixture::new();
    let ssh = [
        ("link", ""),
        ("host", "service.example"),
        ("protocol", "ssh"),
        ("port", "443"),
        ("collection_received_at", RECEIVED),
    ];
    import(&mut f, 9001, &ssh);
    let original: i64 = f
        .connection
        .query_row("SELECT id FROM assets", [], |r| r.get(0))
        .unwrap();
    f.connection.execute("UPDATE project_assets SET decision='confirmed',note='manual protected',is_deleted=1,deleted_at='old trash',entry_opened_at='old opened'",[]).unwrap();
    f.connection.execute("INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary) VALUES(9001,?1,9001,'archived','manual')",[original]).unwrap();
    let smtp = [
        ("link", ""),
        ("host", "service.example"),
        ("protocol", "smtp"),
        ("port", "443"),
        ("collection_received_at", NEWER),
    ];
    assert_eq!(import(&mut f, 9002, &smtp).inserted, 1);
    assert_eq!(import(&mut f, 9002, &smtp).inserted, 0);
    assert_eq!(import(&mut f, 9002, &ssh).inserted, 0);
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let state:(String,String,String,i64,String)=f.connection.query_row("SELECT a.protocol,pa.decision,pa.note,pa.is_deleted,pa.entry_opened_at FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE a.id=?1",[original],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        state,
        (
            "ssh".into(),
            "confirmed".into(),
            "manual protected".into(),
            1,
            "old opened".into()
        )
    );
    let key: String = f
        .connection
        .query_row(
            "SELECT asset_key FROM assets WHERE protocol='smtp'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(key.contains("canonical-sha256:"));
}

#[test]
fn opaque_asset_key_without_valid_network_identity_is_rejected_before_writes() {
    let mut f = Fixture::new();
    let result = import(
        &mut f,
        9001,
        &[("link", ""), ("host", ""), ("ip", ""), ("port", "")],
    );
    assert_eq!((result.inserted, result.updated, result.invalid), (0, 0, 1));
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn provenance_budget_and_truncation_are_sticky_even_on_stale_observations() {
    let mut f = Fixture::new();
    let sources=serde_json::to_string(&(0..140).map(|n|serde_json::json!({"kind":"company","label":format!("fixture-{n}"),"evidence":"x".repeat(600)})).collect::<Vec<_>>()).unwrap();
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", NEWER),
            ("discovery_sources_json", &sources),
        ],
    );
    let stored = extra(&f);
    assert_eq!(stored["discovery_sources_truncated"], "true");
    let raw = stored["discovery_sources_json"].as_str().unwrap();
    assert!(raw.len() <= 65_536);
    assert!(serde_json::from_str::<Vec<Value>>(raw).unwrap().len() <= 128);
    import(
        &mut f,
        9002,
        &[
            ("collection_received_at", RECEIVED),
            ("discovery_sources_json", "[]"),
            ("discovery_sources_truncated", "false"),
        ],
    );
    assert_eq!(extra(&f)["discovery_sources_truncated"], "true");
}

#[test]
fn collection_binding_is_project_local_and_probe_or_unknown_csv_cannot_replace_it() {
    let mut f = Fixture::new();
    f.connection
        .execute("INSERT INTO projects(id,name) VALUES(9002,'另一项目')", [])
        .unwrap();
    f.connection
        .execute(
            "INSERT INTO runs(id,project_id,name) VALUES(9003,9002,'另一范围')",
            [],
        )
        .unwrap();
    let first = "a".repeat(64);
    let second = "b".repeat(64);
    import(
        &mut f,
        9001,
        &[
            ("collection_received_at", RECEIVED),
            ("collection_binding_sha256", &first),
        ],
    );
    import_on(
        &mut f,
        9003,
        9002,
        &[
            ("collection_received_at", NEWER),
            ("collection_binding_sha256", &second),
        ],
    );
    import(
        &mut f,
        9002,
        &[
            ("probe_checked_at", CHECKED),
            ("probe_outcome", "unreachable"),
            ("collection_binding_sha256", &second),
        ],
    );
    import(
        &mut f,
        9002,
        &[("collection_binding_sha256", "not-a-binding")],
    );
    let bindings: Vec<(i64, String)> = f
        .connection
        .prepare(
            "SELECT project_id,last_collection_binding FROM project_assets ORDER BY project_id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(bindings, vec![(9001, first), (9002, second)]);
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
