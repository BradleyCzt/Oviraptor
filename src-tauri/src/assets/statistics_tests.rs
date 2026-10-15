use super::*;
use crate::db;
use rusqlite::params;
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    db_path: PathBuf,
    connection: Connection,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-inventory-statistics-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(9001,'Stats Main'),(9002,'Other project'),(9003,'Empty project')", []).unwrap();
        Self {
            root,
            db_path,
            connection,
        }
    }

    fn asset(&self, id: i64, decision: &str, deleted: i64, probe: &str, status: &str) {
        self.connection.execute("INSERT INTO assets(id,asset_key,host,title,probe_outcome,status_code,extra_json) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![id,format!("key-{id}"),format!("host-{id}.example"),format!("title-{id}"),probe,status,r#"{"syntheticPrivateMetadata":"not-for-chat"}"#]).unwrap();
        self.connection.execute("INSERT INTO project_assets(project_id,asset_id,decision,is_deleted,note,last_seen) VALUES(9001,?1,?2,?3,'private project note',?4)",params![id,decision,deleted,format!("2026-10-{:02}",1+id%28)]).unwrap();
    }

    fn seed_thirty(&self) {
        let decisions = [
            "pending",
            "uncertain",
            "confirmed",
            "rejected",
            "not_applicable",
            "",
            "historic_unrecognized",
        ];
        let probes = [
            "alive_clean",
            "web_alive",
            "web_restricted",
            "browser_render_required",
            "virtual_host_required",
            "web_abnormal",
            "tcp_alive_non_http",
            "blocked_content",
            "unreachable",
            "skipped",
            "",
            "historic_probe",
        ];
        let status_codes = [
            "", "200", "301", "403", "503", "101", "200bad", "099", " 200",
        ];
        for index in 0..30usize {
            self.asset(
                100 + index as i64 * 13,
                decisions[index % decisions.len()],
                match index {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                },
                probes[index % probes.len()],
                status_codes[index % status_codes.len()],
            );
        }
        self.connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(9002,100)",
                [],
            )
            .unwrap();
        self.connection.execute("INSERT INTO assets(id,asset_key,host,title) VALUES(15000,'other-key','other-only.example','other-private')", []).unwrap();
        self.connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(9002,15000)",
                [],
            )
            .unwrap();
        for id in [139, 152] {
            self.connection.execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status) VALUES(9001,?1,'excluded')", [id]).unwrap();
        }
        // Many real Sentinel associations cannot multiply inventory membership.
        for _ in 0..32 {
            self.connection.execute("INSERT INTO sentinel_targets(project_id,asset_id,url) VALUES(9001,126,'https://same-member.example/')", []).unwrap();
        }
    }

    fn stats(&self, filters: &ProjectAssetFilter) -> Value {
        serde_json::to_value(stats_on(&self.connection, 9001, filters).unwrap()).unwrap()
    }

    fn page(&self, filters: &ProjectAssetFilter, limit: i64, cursor: Option<&str>) -> Value {
        serde_json::to_value(search_on(&self.connection, 9001, filters, limit, cursor).unwrap())
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn dimension_total(stats: &Value, name: &str) -> i64 {
    stats[name]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_i64().unwrap())
        .sum()
}

#[test]
fn inventory_statistics_counts_exact_sparse_members_and_unknowns_without_join_inflation() {
    let fixture = Fixture::new();
    fixture.seed_thirty();
    let before = fixture.connection.total_changes();
    let stats = fixture.stats(&ProjectAssetFilter::default());
    assert_eq!(
        (
            stats["projectTotal"].as_i64(),
            stats["activeTotal"].as_i64(),
            stats["trashedTotal"].as_i64(),
            stats["unknownArchiveState"].as_i64(),
            stats["filteredTotal"].as_i64()
        ),
        (Some(30), Some(28), Some(1), Some(1), Some(28))
    );
    for dimension in ["byDecision", "byProbeOutcome", "byHttpStatusClass"] {
        assert_eq!(dimension_total(&stats, dimension), 28);
    }
    assert!(stats["byDecision"]["unknown"].as_i64().unwrap() > 0);
    assert!(stats["byProbeOutcome"]["unclassified"].as_i64().unwrap() > 0);
    assert!(stats["byProbeOutcome"]["unknown"].as_i64().unwrap() > 0);
    assert!(stats["byHttpStatusClass"]["unknown"].as_i64().unwrap() > 0);
    assert_eq!(stats["exclusions"]["projectDecisionExcluded"], 8);
    assert_eq!(stats["exclusions"]["ownershipExcluded"], 2);
    assert_eq!(stats["exclusions"]["denominator"], "filteredTotal");
    assert_eq!(stats["scope"]["unit"], "saved_asset_members");
    assert_eq!(stats["scope"]["filters"]["decisionView"], "all");
    assert_eq!(stats["scope"]["inventory"], "saved_not_current_liveness");
    assert!(chrono::DateTime::parse_from_rfc3339(stats["asOf"].as_str().unwrap()).is_ok());
    assert_eq!(stats["readConsistency"], "single_sqlite_read_snapshot");
    assert!(fixture.connection.is_autocommit());
    assert_eq!(fixture.connection.total_changes(), before);
    assert!(fixture
        .connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,126)",
            []
        )
        .is_err());
    assert_eq!(
        fixture.stats(&ProjectAssetFilter::default())["filteredTotal"],
        28
    );
}

#[test]
fn inventory_statistics_empty_project_is_known_zero_but_missing_project_is_error() {
    let fixture = Fixture::new();
    let empty = serde_json::to_value(
        stats_on(&fixture.connection, 9003, &ProjectAssetFilter::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(empty["projectTotal"], 0);
    assert_eq!(empty["filteredTotal"], 0);
    for dimension in ["byDecision", "byProbeOutcome", "byHttpStatusClass"] {
        assert_eq!(dimension_total(&empty, dimension), 0);
    }
    assert!(
        matches!(stats_on(&fixture.connection, 9999, &ProjectAssetFilter::default()), Err(error) if error=="asset_project_not_found")
    );
    assert!(
        matches!(search_on(&fixture.connection, 9999, &ProjectAssetFilter::default(), 20, None), Err(error) if error=="asset_project_not_found")
    );
    assert!(
        matches!(stats_on(&fixture.connection, 0, &ProjectAssetFilter::default()), Err(error) if error=="asset_project_invalid")
    );
    assert!(fixture.connection.is_autocommit());
}

#[test]
fn inventory_statistics_and_search_match_real_asset_list_member_filter_not_tab_summary() {
    let fixture = Fixture::new();
    fixture.seed_thirty();
    let variants = [
        json!({}),
        json!({"deletedView":"all"}),
        json!({"deletedView":"trash"}),
        json!({"decisionView":"pending"}),
        json!({"decisionView":"review"}),
        json!({"decisionView":"uncertain"}),
        json!({"decisionView":"confirmed"}),
        json!({"decisionView":"rejected"}),
        json!({"decisionView":"not_applicable"}),
        json!({"probeView":"browser_review"}),
        json!({"probeView":"browser_accessible"}),
        json!({"probeView":"restricted"}),
        json!({"probeView":"service"}),
        json!({"probeView":"abnormal"}),
        json!({"probeView":"blocked"}),
        json!({"probeOutcomeView":"alive_clean"}),
        json!({"probeOutcomeView":"web_alive"}),
        json!({"probeOutcomeView":"unreachable"}),
        json!({"deletedView":"all","decisionView":"confirmed","probeView":"browser_review","query":"host-"}),
    ];
    for variant in variants {
        let filters: ProjectAssetFilter = serde_json::from_value(variant.clone()).unwrap();
        let mut query = filters.normalized().unwrap().asset_query(9001).unwrap();
        query.page = 1;
        query.page_size = 200;
        let list = super::super::list_assets_on(&fixture.connection, &query).unwrap();
        let stats = fixture.stats(&filters);
        let page = fixture.page(&filters, 20, None);
        assert_eq!(stats["filteredTotal"], list.member_total, "{variant}");
        assert_eq!(page["total"], list.member_total, "{variant}");
        for dimension in ["byDecision", "byProbeOutcome", "byHttpStatusClass"] {
            assert_eq!(
                dimension_total(&stats, dimension),
                list.member_total,
                "{variant}"
            );
        }
        if filters.decision_view == "confirmed" && list.member_total > 0 {
            assert!(list.summary.all > list.member_total);
        }
    }
    // The summary still intentionally ignores decisionView but closes its own
    // denominator with a fixed unknown bucket, never dropping legacy values.
    let mut query = ProjectAssetFilter {
        decision_view: "confirmed".into(),
        ..Default::default()
    }
    .asset_query(9001)
    .unwrap();
    query.page_size = 200;
    let list = super::super::list_assets_on(&fixture.connection, &query).unwrap();
    assert_eq!(list.summary.all, 28);
    assert_eq!(
        list.summary.all,
        list.summary.pending
            + list.summary.uncertain
            + list.summary.confirmed
            + list.summary.rejected
            + list.summary.not_applicable
            + list.summary.unknown
    );
    assert_eq!(list.summary.unknown, 4);
}

#[test]
fn inventory_search_uses_all_eight_list_like_fields_and_preserves_wildcards() {
    let fixture = Fixture::new();
    for (index, column) in [
        "company",
        "host",
        "link",
        "ip",
        "domain",
        "title",
        "asset_key",
    ]
    .iter()
    .enumerate()
    {
        let id = 10 + index as i64;
        fixture.asset(id, "pending", 0, "", "");
        fixture
            .connection
            .execute(
                &format!("UPDATE assets SET {column}=?1 WHERE id=?2"),
                params![format!("unique-needle-{index}"), id],
            )
            .unwrap();
        let filters = ProjectAssetFilter {
            query: format!("needle-{index}"),
            ..Default::default()
        };
        let page = fixture.page(&filters, 20, None);
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["id"], id);
        assert_eq!(fixture.stats(&filters)["filteredTotal"], 1);
    }
    for (query, expected) in [("Stats Main", 7), ("%", 7), ("_", 7), ("%' OR 1=1 --", 0)] {
        let filters = ProjectAssetFilter {
            query: query.into(),
            ..Default::default()
        };
        assert_eq!(fixture.page(&filters, 20, None)["total"], expected);
        assert_eq!(fixture.stats(&filters)["filteredTotal"], expected);
    }
    assert_eq!(
        fixture.page(&ProjectAssetFilter::default(), 20, None)["searchSemantics"],
        "assets_list_like_8_fields"
    );
}

#[test]
fn inventory_search_keyset_cursor_is_stable_complete_and_bound_to_project_filters_order() {
    let fixture = Fixture::new();
    fixture.seed_thirty();
    let filter = ProjectAssetFilter::default();
    let first = fixture.page(&filter, 20, None);
    assert_eq!(
        (
            first["total"].as_i64(),
            first["returned"].as_i64(),
            first["hasMore"].as_bool()
        ),
        (Some(28), Some(20), Some(true))
    );
    let token = first["nextCursor"].as_str().unwrap();
    let second = fixture.page(&filter, 20, Some(token));
    assert_eq!(
        (
            second["total"].as_i64(),
            second["returned"].as_i64(),
            second["hasMore"].as_bool()
        ),
        (Some(28), Some(8), Some(false))
    );
    assert!(second["nextCursor"].is_null());
    let ids: Vec<_> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), 28);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ids[0], 126);
    assert_eq!(first["order"], "asset_id_asc");
    assert!(
        matches!(search_on(&fixture.connection,9002,&filter,20,Some(token)),Err(error) if error=="asset_cursor_scope_changed")
    );
    for changed in [
        json!({"query":"host"}),
        json!({"deletedView":"all"}),
        json!({"decisionView":"review"}),
        json!({"probeView":"browser_review"}),
        json!({"probeOutcomeView":"web_alive"}),
    ] {
        let changed: ProjectAssetFilter = serde_json::from_value(changed).unwrap();
        assert!(
            matches!(search_on(&fixture.connection,9001,&changed,20,Some(token)),Err(error) if error=="asset_cursor_scope_changed")
        );
    }
    let mut decoded: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(token.strip_prefix(CURSOR_PREFIX).unwrap())
            .unwrap(),
    )
    .unwrap();
    for (field, value) in [
        ("version", json!(2)),
        ("order", json!("last_seen_desc")),
        ("afterAssetId", json!(0)),
        ("afterAssetId", json!(-1)),
    ] {
        let original = decoded[field].clone();
        decoded[field] = value;
        let bad = format!(
            "{CURSOR_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&decoded).unwrap())
        );
        assert!(search_on(&fixture.connection, 9001, &filter, 20, Some(&bad)).is_err());
        decoded[field] = original;
    }
    for bad in ["", "not-json", "#"] {
        assert!(search_on(&fixture.connection, 9001, &filter, 20, Some(bad)).is_err());
    }
    let normalized = ProjectAssetFilter {
        query: "  ".into(),
        ..Default::default()
    };
    assert!(search_on(&fixture.connection, 9001, &normalized, 20, Some(token)).is_ok());
    // A cursor promises an ID position, not persistence of the previous read
    // snapshot. A later response reports its own exact current denominator.
    fixture.asset(1000, "pending", 0, "", "");
    let later = fixture.page(&filter, 20, Some(token));
    assert_eq!(later["total"], 29);
    assert_eq!(later["returned"], 9);
}

#[test]
fn inventory_service_rejects_unrecognized_filters_limits_and_project_override() {
    let fixture = Fixture::new();
    for bad in [
        json!({"projectId":9002}),
        json!({"sql":"SELECT 1"}),
        json!({"query":null}),
    ] {
        assert!(serde_json::from_value::<ProjectAssetFilter>(bad).is_err());
    }
    for bad in [
        json!({"deletedView":"unknown"}),
        json!({"decisionView":"unknown"}),
        json!({"probeView":"unknown"}),
        json!({"probeOutcomeView":"unknown"}),
        json!({"probeView":""}),
    ] {
        let bad: ProjectAssetFilter = serde_json::from_value(bad).unwrap();
        assert!(
            matches!(stats_on(&fixture.connection,9001,&bad),Err(error) if error=="asset_filter_invalid")
        );
        assert!(
            matches!(search_on(&fixture.connection,9001,&bad,20,None),Err(error) if error=="asset_filter_invalid")
        );
    }
    let long = ProjectAssetFilter {
        query: "界".repeat(121),
        ..Default::default()
    };
    assert!(
        matches!(stats_on(&fixture.connection,9001,&long),Err(error) if error=="asset_query_too_long")
    );
    for limit in [-1, 0, 21, i64::MAX] {
        assert!(
            matches!(search_on(&fixture.connection,9001,&ProjectAssetFilter::default(),limit,None),Err(error) if error=="asset_search_limit_invalid")
        );
    }
    assert!(fixture.connection.is_autocommit());
}

#[test]
fn inventory_read_snapshot_keeps_count_dimensions_and_page_coherent_during_other_writer() {
    let fixture = Fixture::new();
    fixture.asset(1, "pending", 0, "web_alive", "200");
    let writer = db::open(&fixture.db_path).unwrap();
    let transaction = fixture.connection.unchecked_transaction().unwrap();
    // The actual caller-owned read transaction pins the original inventory.
    let first =
        serde_json::to_value(stats_on(&transaction, 9001, &ProjectAssetFilter::default()).unwrap())
            .unwrap();
    writer.execute("UPDATE project_assets SET decision='rejected',is_deleted=1 WHERE project_id=9001 AND asset_id=1",[]).unwrap();
    writer
        .execute(
            "UPDATE assets SET probe_outcome='historic_probe',status_code='500' WHERE id=1",
            [],
        )
        .unwrap();
    let same =
        serde_json::to_value(stats_on(&transaction, 9001, &ProjectAssetFilter::default()).unwrap())
            .unwrap();
    let page = serde_json::to_value(
        search_on(&transaction, 9001, &ProjectAssetFilter::default(), 20, None).unwrap(),
    )
    .unwrap();
    assert_eq!(first["filteredTotal"], 1);
    assert_eq!(same["filteredTotal"], 1);
    assert_eq!(same["byDecision"]["pending"], 1);
    assert_eq!(same["byHttpStatusClass"]["success"], 1);
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["decision"], "pending");
    assert!(!transaction.is_autocommit());
    transaction.commit().unwrap();
    let fresh = fixture.stats(&ProjectAssetFilter::default());
    assert_eq!(fresh["filteredTotal"], 0);
    assert_eq!(fresh["trashedTotal"], 1);
    assert_eq!(
        fixture.page(&ProjectAssetFilter::default(), 20, None)["total"],
        0
    );
    assert!(fixture.connection.is_autocommit());
}

#[test]
fn inventory_http_status_classes_are_strict_and_safe_search_projection_is_bounded() {
    let fixture = Fixture::new();
    let statuses = [
        "", "100", "199", "200", "299", "300", "399", "400", "499", "500", "599", "600", "099",
        "200oops", " 200", "200 ", "20", "0200", "二百",
    ];
    for (index, status) in statuses.iter().enumerate() {
        fixture.asset(1 + index as i64, "future_arbitrary_decision", 0, "", status);
    }
    let stats = fixture.stats(&ProjectAssetFilter::default());
    assert_eq!(
        stats["byHttpStatusClass"],
        json!({"informational":2,"success":2,"redirect":2,"clientError":2,"serverError":2,"unclassified":1,"unknown":8})
    );
    let huge = "界".repeat(10000);
    fixture
        .connection
        .execute(
            "UPDATE assets SET host=?1,title=?1,protocol=?1,port=?1,status_code=?1 WHERE id=1",
            [&huge],
        )
        .unwrap();
    let page = fixture.page(&ProjectAssetFilter::default(), 20, None);
    let item = &page["items"][0];
    for (field, bound) in [
        ("host", 256),
        ("title", 256),
        ("protocol", 32),
        ("port", 16),
        ("statusCode", 16),
        ("decision", 32),
    ] {
        assert!(item[field].as_str().unwrap().len() <= bound);
    }
    assert_eq!(item["decision"], "unknown");
    assert_eq!(
        item.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        [
            "id",
            "host",
            "title",
            "protocol",
            "port",
            "statusCode",
            "decision"
        ]
        .map(str::to_string)
        .into_iter()
        .collect()
    );
    assert!(!page.to_string().contains("syntheticPrivateMetadata"));
    assert!(!page.to_string().contains("private project note"));
    assert!(serde_json::to_vec(&page).unwrap().len() < 32768);
}

#[test]
fn inventory_search_twenty_unicode_or_control_rows_fit_complete_json_without_skipping_cursor() {
    let fixture = Fixture::new();
    for id in 1..=25 {
        fixture.asset(id, "pending", 0, "", "");
    }
    for text in ["😀界".repeat(1024), "\u{0001}\"\\\n".repeat(1024)] {
        fixture
            .connection
            .execute(
                "UPDATE assets SET host=?1,title=?1,protocol=?1,port=?1,status_code=?1",
                [&text],
            )
            .unwrap();
        let first = fixture.page(&ProjectAssetFilter::default(), 20, None);
        let envelope = crate::agent_runtime::secrets::redact_json(
            &json!({"result":first,"source":"saved_project_assets"}),
        );
        assert!(serde_json::to_vec(&envelope).unwrap().len() <= 32768);
        assert_eq!(envelope["result"]["returned"], 20);
        assert_eq!(
            envelope["result"]["items"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["id"],
            20
        );
        assert_eq!(envelope["result"]["total"], 25);
        let token = envelope["result"]["nextCursor"].as_str().unwrap();
        let second = fixture.page(&ProjectAssetFilter::default(), 20, Some(token));
        assert_eq!(second["returned"], 5);
        assert_eq!(second["items"][0]["id"], 21);
        assert_eq!(second["items"][4]["id"], 25);
        assert_eq!(second["hasMore"], false);
        for item in envelope["result"]["items"].as_array().unwrap() {
            for (field, budget) in [
                ("host", 256),
                ("title", 256),
                ("protocol", 32),
                ("port", 16),
                ("statusCode", 16),
            ] {
                assert!(serde_json::to_vec(&item[field]).unwrap().len() <= budget);
            }
        }
    }
}

#[test]
fn inventory_search_redacts_complete_url_userinfo_before_scalar_clipping() {
    let fixture = Fixture::new();
    fixture.asset(1, "pending", 0, "", "");
    // Synthetic fixture credential; its distant '@' lies beyond every output
    // field budget and must still be seen by the existing full-text redactor.
    let password = format!("synthetic-private-prefix-{}", "x".repeat(1024));
    let url = format!("https://fixture-user:{password}@public.example/path");
    fixture
        .connection
        .execute(
            "UPDATE assets SET host=?1,title=?1,protocol=?1,port=?1,status_code=?1",
            [&url],
        )
        .unwrap();
    let page = fixture.page(&ProjectAssetFilter::default(), 20, None);
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains("synthetic-private-prefix"));
    assert!(!serialized.contains("fixture-user:"));
    assert_eq!(page["returned"], 1);
    assert_eq!(page["total"], 1);
}

#[test]
fn inventory_search_small_project_cursor_survives_final_redaction_and_reads_next_exact_member() {
    let fixture = Fixture::new();
    fixture
        .connection
        .execute(
            "INSERT INTO projects(id,name) VALUES(1,'Small project')",
            [],
        )
        .unwrap();
    for id in 1..=3 {
        fixture.asset(id, "pending", 0, "", "");
        fixture
            .connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(1,?1)",
                [id],
            )
            .unwrap();
    }
    let filter = ProjectAssetFilter::default();
    let first =
        serde_json::to_value(search_on(&fixture.connection, 1, &filter, 1, None).unwrap()).unwrap();
    let first = json!({"ok":true,"data":first});
    let redacted = crate::agent_runtime::secrets::redact_json(&first);
    let token = first["data"]["nextCursor"].as_str().unwrap();
    assert_eq!(redacted["data"]["nextCursor"], token);
    assert!(token.starts_with(CURSOR_PREFIX));
    let second = serde_json::to_value(
        search_on(
            &fixture.connection,
            1,
            &filter,
            1,
            redacted["data"]["nextCursor"].as_str(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(second["items"][0]["id"], 2);
    assert_eq!(second["total"], 3);
    assert_eq!(second["returned"], 1);
    assert!(search_on(
        &fixture.connection,
        1,
        &filter,
        1,
        token.strip_prefix(CURSOR_PREFIX)
    )
    .is_err());
}
