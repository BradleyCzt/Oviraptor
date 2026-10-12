// §Q4 — the public indexes and their real terminal states. crt.sh and Wayback are mirrored
// on localhost so the parse, the truncation and the failure paths are observed rather than
// asserted from a comment.

use super::adapters::certificate_transparency::{execute as fetch_ct, names_for_domain, query_url};
use super::adapters::http::{PassiveClient, PassiveHttpLimits, RequestMeter};
use super::adapters::web_archive::{execute as fetch_archive, rows};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::orchestrator::{
    run_plan, start_or_reuse, ItemOutcome, KIND_CERTIFICATE_TRANSPARENCY, KIND_PASSIVE_WEB,
    KIND_WEB_ARCHIVE,
};
use super::rules;
use super::scope::ExposureScope;
use super::state::{ensure_exposure_schema, RunStatus, SourceStatus, WorkItemStatus};
use super::work_items::{claim_next, counts, plan, WorkItem, WorkItemSeed};
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-q4-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("临时库连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
}

fn client() -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(5),
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn item(kind: &str, entity: &str) -> WorkItem {
    WorkItem {
        id: 1,
        run_id: 1,
        project_id: 61,
        work_key: format!("{kind}|{entity}"),
        kind: kind.to_string(),
        source_key: kind.to_string(),
        asset_id: None,
        origin: String::new(),
        entity: entity.to_string(),
        input_hash: String::new(),
        status: WorkItemStatus::Pending,
        owner: String::new(),
        claim_version: 0,
        attempts: 0,
        observations: 0,
        requests: 0,
        bytes: 0,
    }
}

fn never_cancelled() -> bool {
    false
}

#[test]
fn q4_ct_names_are_kept_as_provenance_and_never_become_authorized_assets() {
    let root = sandbox("ct");
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(
            200,
            &[("Content-Type", "application/json")],
            r#"[{"name_value":"*.corp.invalid\ncorp.invalid\npay.corp.invalid","not_before":"2026-01-01","not_after":"2026-04-01"},{"name_value":"evil-other.invalid","not_before":"2026-01-01","not_after":"2026-04-01"}]"#,
        )],
    )]));
    let mut meter = RequestMeter {
        max_requests: 10,
        max_bytes: 1_000_000,
        ..Default::default()
    };
    let outcome: ItemOutcome = fetch_ct(
        &client(),
        &item(KIND_CERTIFICATE_TRANSPARENCY, "corp.invalid"),
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        &never_cancelled,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::Completed));
    assert_eq!(outcome.source_status, Some(SourceStatus::Completed));
    assert_eq!(outcome.observed, 2, "只保留请求根域内的名字：{outcome:?}");
    let names: Vec<String> = outcome
        .observations
        .iter()
        .filter(|row| row.kind == "ct_name")
        .map(|row| row.url.clone())
        .collect();
    assert!(
        names.contains(&"https://corp.invalid".to_string()),
        "{names:?}"
    );
    assert!(names.contains(&"https://pay.corp.invalid".to_string()));
    assert!(
        !names.iter().any(|row| row.contains("evil-other")),
        "混入其他根域的名字必须丢弃"
    );
    assert_eq!(outcome.pending.len(), 2);
    assert_eq!(outcome.findings.len(), 0, "证书名字本身不是线索");
    // 只有请求过的索引端点被访问；被发现的域名一次都没有被请求。
    assert_eq!(fixture.request_count(), 1);
    assert_eq!(
        names_for_domain(r#"[{"name_value":"CORP.INVALID"}]"#, "corp.invalid").len(),
        1,
        "大小写必须归一"
    );
    assert_eq!(
        query_url("https://crt.sh", "*.corp.invalid"),
        "https://crt.sh/?q=%25.corp.invalid&output=json"
    );
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q4_a_broken_index_is_a_parse_error_and_cannot_be_masked_by_a_good_source() {
    let root = sandbox("parse");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(
            200,
            &[("Content-Type", "text/html")],
            "<html>rate limited</html>",
        )],
    )]));
    seed_scope(&connection, 62, "http://good.invalid/", true);
    let config = json!({
        "exposureAllowPrivateDestinations": true,
        "exposureSources": [KIND_PASSIVE_WEB],
    });
    let started = start_or_reuse(&connection, 62, &config).expect("启动");
    let mut meter = RequestMeter {
        max_requests: 20,
        max_bytes: 1_000_000,
        ..Default::default()
    };
    let scope = ExposureScope::load(&connection, 62, &config).expect("范围");
    let fetcher = client();
    let outcome = fetch_ct(
        &fetcher,
        &item(KIND_CERTIFICATE_TRANSPARENCY, "corp.invalid"),
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        &never_cancelled,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::Failed));
    assert_eq!(outcome.source_status, Some(SourceStatus::ParseError));
    assert!(outcome
        .observations
        .iter()
        .any(|row| row.kind == "source_parse_error"));

    // 一个坏来源不能被好来源掩盖：聚合必须仍然点名它。
    let bad = format!("|{outcome:?}");
    assert!(!bad.is_empty());
    plan(
        &connection,
        started.run_id,
        62,
        1,
        &[WorkItemSeed {
            work_key: format!("{KIND_CERTIFICATE_TRANSPARENCY}|corp.invalid"),
            kind: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
            source_key: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
            asset_id: None,
            origin: String::new(),
            entity: "corp.invalid".to_string(),
            input_hash: "ct".to_string(),
        }],
    )
    .expect("补一项 CT");
    let fetcher2 = client();
    let mut meter2 = RequestMeter {
        max_requests: 20,
        max_bytes: 1_000_000,
        ..Default::default()
    };
    let report = run_plan(
        &connection,
        started.run_id,
        "owner-q4",
        &mut |row: &WorkItem| -> Result<ItemOutcome, String> {
            if row.kind == KIND_CERTIFICATE_TRANSPARENCY {
                return Ok(fetch_ct(
                    &fetcher2,
                    row,
                    &mut meter2,
                    std::slice::from_ref(&fixture.origin),
                    true,
                    &never_cancelled,
                ));
            }
            let origin = row.origin.clone();
            let revalidate = || -> Result<(), super::scope::ExposureDenial> {
                let url = reqwest::Url::parse(&format!("{origin}/"))
                    .map_err(|e| super::scope::ExposureDenial::new("bad", e.to_string()))?;
                scope.authorize(&url, &super::state::utc_now()).map(|_| ())
            };
            Ok(super::adapters::passive_web::execute(
                &fetcher2,
                &scope,
                row,
                &mut meter2,
                &super::adapters::passive_web::WalkBudget::default(),
                &revalidate,
                &never_cancelled,
            ))
        },
    )
    .expect("两轮混合必须收敛");
    assert_eq!(report.aggregation.status, RunStatus::Partial);
    assert!(
        report
            .aggregation
            .gaps
            .iter()
            .any(|gap| gap == "source_parse_error:certificate_transparency"),
        "CT 解析失败必须点名：{:?}",
        report.aggregation.gaps
    );
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q4_an_unconfigured_index_reports_unconfigured_rather_than_completed() {
    let root = sandbox("unconfigured");
    // 没有已批准域名：CT 项必须是 skipped_unconfigured，来源必须是 unconfigured。
    let mut meter = RequestMeter {
        max_requests: 5,
        max_bytes: 100_000,
        ..Default::default()
    };
    let outcome = fetch_ct(
        &client(),
        &item(KIND_CERTIFICATE_TRANSPARENCY, ""),
        &mut meter,
        &["https://crt.sh".to_string()],
        false,
        &never_cancelled,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::SkippedUnconfigured));
    assert_eq!(outcome.source_status, Some(SourceStatus::Unconfigured));
    assert_eq!(meter.requests_used, 0, "未配置时一个包都不能发");
    let archive = fetch_archive(
        &client(),
        &item(KIND_WEB_ARCHIVE, ""),
        &mut meter,
        &["https://web.archive.org".to_string()],
        false,
        200,
        &never_cancelled,
    );
    assert_eq!(archive.source_status, Some(SourceStatus::Unconfigured));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q4_wayback_keeps_index_time_separate_from_current_accessibility() {
    let root = sandbox("archive");
    let fixture = Fixture::start(routes(vec![(
        "/cdx/search/cdx",
        vec![response(
            200,
            &[("Content-Type", "application/json")],
            r#"[["url","statuscode","mimetype","timestamp"],
["https://corp.invalid/运维手册.pdf","200","application/pdf","20250101120000"],
["https://corp.invalid/app.js.map","200","application/json","20240202120000"],
["https://corp.invalid/pricing.pdf","200","application/pdf","20240303120000"]]"#,
        )],
    )]));
    let mut meter = RequestMeter {
        max_requests: 10,
        max_bytes: 1_000_000,
        ..Default::default()
    };
    let outcome = fetch_archive(
        &client(),
        &item(KIND_WEB_ARCHIVE, "corp.invalid"),
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        200,
        &never_cancelled,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::Completed));
    assert_eq!(outcome.observed, 3, "{:?}", outcome.observed);
    let archived = outcome
        .observations
        .iter()
        .find(|row| row.url.contains("运维手册"))
        .expect("内部文档必须留痕");
    assert_eq!(
        archived.indexed_at, "20250101120000",
        "收录时间必须单独保存"
    );
    assert_eq!(
        archived.current_accessible, None,
        "索引命中不代表当前仍可访问"
    );
    let titles: Vec<&str> = outcome
        .findings
        .iter()
        .map(|row| row.title.as_str())
        .collect();
    assert!(titles.contains(&"历史归档中存在疑似内部文档"), "{titles:?}");
    assert!(
        titles.contains(&"历史归档中存在高价值公开路径"),
        "{titles:?}"
    );
    assert_eq!(
        outcome
            .findings
            .iter()
            .filter(|row| row.url.contains("pricing.pdf"))
            .count(),
        0,
        "普通公开文档不能算内部线索"
    );
    let captured = rows(
        r#"[["url","statuscode","mimetype","timestamp"],["https://a/b","200","text/html","2024"]]"#,
    );
    assert_eq!(captured.len(), 1);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q4_an_index_that_hits_its_limit_is_partial_with_a_reason() {
    let root = sandbox("limit");
    let body = format!(
        "[[\"url\",\"statuscode\",\"mimetype\",\"timestamp\"],{}]",
        (0..200)
            .map(|index| format!(
                "[\"https://corp.invalid/p{index}\",\"200\",\"text/html\",\"2024\"]"
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let fixture = Fixture::start(routes(vec![(
        "/cdx/search/cdx",
        vec![response(
            200,
            &[("Content-Type", "application/json")],
            &body,
        )],
    )]));
    let mut meter = RequestMeter {
        max_requests: 10,
        max_bytes: 10_000_000,
        ..Default::default()
    };
    let outcome = fetch_archive(
        &client(),
        &item(KIND_WEB_ARCHIVE, "corp.invalid"),
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        200,
        &never_cancelled,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial));
    assert!(outcome.reason.contains("上限"), "{}", outcome.reason);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q4_the_pending_queue_is_idempotent_and_stays_unreviewed_across_rounds() {
    let root = sandbox("pending");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(63,'范围','active')",
            [],
        )
        .unwrap();
    // 第二轮已经收尾，所以活跃唯一索引允许它存在；这也证明同一项目不会同时挂两条进行中。
    for (run_id, status) in [(1, "running"), (2, "completed")] {
        connection
            .execute(
                "INSERT INTO exposure_runs(id,project_id,status) VALUES(?1,63,?2)",
                rusqlite::params![run_id, status],
            )
            .unwrap();
    }
    plan(
        &connection,
        1,
        63,
        1,
        &[WorkItemSeed {
            work_key: format!("{KIND_CERTIFICATE_TRANSPARENCY}|corp.invalid"),
            kind: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
            source_key: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
            asset_id: None,
            origin: String::new(),
            entity: "corp.invalid".to_string(),
            input_hash: "ct".to_string(),
        }],
    )
    .expect("计划");
    let claimed = claim_next(&connection, 1, "owner", 3_600)
        .unwrap()
        .expect("必须领到 CT 项");
    let body =
        r#"[{"name_value":"pay.corp.invalid","not_before":"2026-01-01","not_after":"2026-04-01"}]"#;
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "application/json")], body)],
    )]));
    let mut meter = RequestMeter {
        max_requests: 5,
        max_bytes: 1_000_000,
        ..Default::default()
    };
    let outcome = fetch_ct(
        &client(),
        &claimed,
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        &never_cancelled,
    );
    super::orchestrator::save_source_result(
        &connection,
        1,
        KIND_CERTIFICATE_TRANSPARENCY,
        SourceStatus::Completed,
        "",
        outcome.observed,
    )
    .expect("来源终态必须可写");
    let mut stored = 0;
    for observation in &outcome.observations {
        let (id, _) =
            super::observations::record(&connection, 1, 63, Some(claimed.id), observation)
                .expect("观察必须可写");
        stored += 1;
        for draft in &outcome.findings {
            super::observations::merge_finding(&connection, 1, 63, draft, id).ok();
        }
    }
    for pending in &outcome.pending {
        super::observations::record_pending(&connection, 63, 1, pending).expect("第一轮也要进队列");
    }
    assert_eq!(stored, 1);
    // 同名在第二轮再次出现：队列去重，但来源引用被本轮刷新，不新增第二条。
    connection
        .execute(
            "INSERT INTO exposure_work_items(run_id,project_id,work_key,kind,source_key,entity)
             VALUES(2,63,'ct2','certificate_transparency','certificate_transparency','corp.invalid')",
            [],
        )
        .unwrap();
    let second = claim_next(&connection, 2, "owner", 3_600)
        .unwrap()
        .expect("第二轮认领");
    let outcome2 = fetch_ct(
        &client(),
        &second,
        &mut meter,
        std::slice::from_ref(&fixture.origin),
        true,
        &never_cancelled,
    );
    for observation in &outcome2.observations {
        super::observations::record(&connection, 2, 63, Some(second.id), observation).ok();
        super::observations::record_pending(
            &connection,
            63,
            2,
            &super::orchestrator::PendingDiscovery {
                source_key: observation.source_key.clone(),
                kind: "domain".to_string(),
                entity: observation.url.replace("https://", ""),
                canonical_key: observation.url.replace("https://", ""),
                evidence_url: observation.url.clone(),
            },
        )
        .expect("待归并必须幂等");
    }
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_pending_discoveries WHERE project_id=63",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "同一项目同一来源的同一名字只留一条：{pending}");
    let state: String = connection
        .query_row(
            "SELECT status FROM exposure_pending_discoveries WHERE project_id=63",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "unreviewed", "外部索引结果不得自动通过归属门禁");
    let assets: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM assets WHERE host='pay.corp.invalid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assets, 0, "新名字不得直接变成资产");
    let counts = counts(&connection, 1).unwrap();
    assert_eq!(counts.total, 1);
    assert!(rules::repository_host("https://github.com/x"));
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}
