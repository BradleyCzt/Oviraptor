// G2 — GitHub. Endpoint injection for the production worker (G2-0), anonymous repository
// metadata search (G2-1), code search + controlled blob read behind the credential port and
// the token guards (G2-2), classification reuse (G2-3) and the closed loop (G2-4). Network
// tests talk to localhost fixtures only; the single live GitHub test is `#[ignore]` and runs
// only when asked for explicitly.

use super::adapters::github::{
    self, execute_code, execute_repositories, CredentialLookup, GithubEndpoint, GithubSession, GithubToken,
};
use super::adapters::http::{FetchBlock, PassiveClient, PassiveHttpLimits, ProviderAuthorization, RequestMeter};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::orchestrator::{start_or_reuse, ItemOutcome, KIND_GITHUB_CODE, KIND_GITHUB_REPOSITORIES};
use super::settings::{save, ExposureSettings};
use super::state::{ensure_exposure_schema, SourceStatus, WorkItemStatus};
use super::work_items::WorkItem;
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

const TOKEN: &str = "ghp_FixtureOnlyToken0123456789abcdefABCD";

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-exposure-g2-{tag}-{}", uuid::Uuid::new_v4()))
}

fn database(root: &Path) -> (Connection, PathBuf) {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    (connection, db_path)
}

fn client() -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(5),
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn meter(max_requests: i64) -> RequestMeter {
    RequestMeter { requests_used: 0, bytes_used: 0, max_requests, max_bytes: 50_000_000 }
}

fn never() -> bool {
    false
}

fn item(kind: &str, entity: &str) -> WorkItem {
    WorkItem {
        id: 1,
        run_id: 1,
        project_id: 71,
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

fn endpoint(fixture: &Fixture) -> GithubEndpoint {
    GithubEndpoint { api_origin: fixture.origin.clone(), allow_private: true }
}

fn token() -> CredentialLookup {
    CredentialLookup::Available(GithubToken::new(TOKEN).expect("测试 token 形态合法"))
}

fn api(status: u16, headers: &[(&str, &str)], body: &str) -> String {
    let mut all = vec![("Content-Type", "application/json; charset=utf-8")];
    all.extend_from_slice(headers);
    response(status, &all, body)
}

fn repo(id: i64, full_name: &str, description: &str) -> JsonValue {
    json!({
        "id": id,
        "full_name": full_name,
        "html_url": format!("https://github.com/{full_name}"),
        "private": false,
        "visibility": "public",
        "description": description,
        "fork": false,
        "archived": false,
        "stargazers_count": 3,
        "pushed_at": "2026-09-30T08:00:00Z",
        "default_branch": "main",
        "language": "Java",
        "topics": ["internal"],
    })
}

fn repo_page(total: i64, incomplete: bool, items: Vec<JsonValue>) -> String {
    json!({"total_count": total, "incomplete_results": incomplete, "items": items}).to_string()
}

fn repos(start: i64, count: i64) -> Vec<JsonValue> {
    (start..start + count).map(|i| repo(i, &format!("acme-dev/tool-{i}"), "example.test 内部工具")).collect()
}

fn next_link(origin: &str, query: &str, page: i64) -> String {
    let mut url = reqwest::Url::parse(&format!("{origin}/search/repositories")).unwrap();
    url.query_pairs_mut().append_pair("q", query).append_pair("per_page", "30").append_pair("page", &page.to_string());
    format!("<{url}>; rel=\"next\", <{url}>; rel=\"last\"")
}

fn observations_of(outcome: &ItemOutcome, kind: &str) -> usize {
    outcome.observations.iter().filter(|row| row.kind == kind).count()
}

fn run_repositories(fixture: &Fixture, entity: &str, session: &mut GithubSession, max_requests: i64) -> (ItemOutcome, i64) {
    let mut meter = meter(max_requests);
    let outcome = execute_repositories(&client(), &item(KIND_GITHUB_REPOSITORIES, entity), &mut meter, &endpoint(fixture), session, &[], &never);
    (outcome, meter.requests_used)
}

/// Every text cell of every table in the temporary database, for "the token is nowhere".
fn everything_stored(connection: &Connection) -> String {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .flatten()
        .collect();
    let mut all = String::new();
    for table in tables {
        let mut statement = connection.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for index in 0..columns {
                if let Ok(rusqlite::types::ValueRef::Text(text)) = row.get_ref(index) {
                    all.push_str(&String::from_utf8_lossy(text));
                    all.push('\n');
                }
            }
        }
    }
    all
}

// ───────────────────────────── worker helpers ─────────────────────────────

fn site() -> Fixture {
    Fixture::start(routes(vec![("/", vec![response(200, &[("Content-Type", "text/html")], "<html><title>首页</title></html>")])]))
}

fn github_project(connection: &Connection, project_id: i64, site: &Fixture, sources: &[&str], owners: &[&str]) -> JsonValue {
    seed_scope(connection, project_id, &site.origin, true);
    connection
        .execute(
            "UPDATE asset_ownership_profiles SET approved_domains_json=?2 WHERE project_id=?1",
            params![project_id, r#"["example.test"]"#],
        )
        .unwrap();
    let mut value = ExposureSettings::for_project(project_id);
    value.sources = sources.iter().map(|row| row.to_string()).collect();
    value.allow_private_destinations = true;
    value.timeout_seconds = 5;
    value.github_official_owners = owners.iter().map(|row| row.to_string()).collect();
    let current: i64 = connection
        .query_row("SELECT COALESCE(MAX(version),0) FROM exposure_settings WHERE project_id=?1", [project_id], |row| row.get(0))
        .unwrap();
    save(connection, &value, current).expect("配置必须能保存");
    super::exposure_scan_settings(connection, project_id).expect("配置必须可读")
}

fn fixture_endpoints(index: &Fixture, credential: CredentialLookup) -> super::SourceEndpoints {
    super::SourceEndpoints {
        ct: vec![index.origin.clone()],
        archive: vec![index.origin.clone()],
        github: endpoint(index),
        allow_private: true,
        github_credential: credential,
        document_any_port: false,
    }
}

fn run_worker(db_path: &Path, run_id: i64, project_id: i64, settings: &JsonValue, endpoints: super::SourceEndpoints) {
    super::use_test_endpoints_once(endpoints);
    super::execute_exposure_run(db_path.to_path_buf(), run_id, project_id, "g2-worker".into(), settings.clone());
}

fn source_result(connection: &Connection, run_id: i64, source: &str) -> (String, String) {
    connection
        .query_row(
            "SELECT status,error FROM exposure_source_results WHERE run_id=?1 AND source_key=?2",
            params![run_id, source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_else(|_| ("<missing>".into(), String::new()))
}

fn requests_used(connection: &Connection, run_id: i64) -> i64 {
    connection
        .query_row("SELECT requests_used FROM exposure_run_plans WHERE run_id=?1", [run_id], |row| row.get(0))
        .unwrap()
}

fn disposition(connection: &Connection, project_id: i64, url: &str) -> (String, String) {
    let id: i64 = connection
        .query_row("SELECT id FROM exposure_findings WHERE project_id=?1 AND source_url=?2", params![project_id, url], |row| row.get(0))
        .unwrap_or_else(|_| panic!("没有 {url} 的线索"));
    let view = super::classify::disposition_of(connection, project_id, id).unwrap().expect("必须有处置");
    (view.disposition.to_string(), view.source_channel.to_string())
}

// ───────────────────────────── G2-0 ─────────────────────────────

#[test]
fn g2_0_production_endpoints_are_constants_and_the_test_override_is_used_once() {
    let production = super::SourceEndpoints::production();
    assert_eq!(production.ct, vec!["https://crt.sh".to_string()]);
    assert_eq!(production.archive, vec!["https://web.archive.org".to_string()]);
    assert_eq!(production.github, GithubEndpoint { api_origin: "https://api.github.com".into(), allow_private: false });
    assert!(!production.allow_private);
    match &production.github_credential {
        CredentialLookup::Unconfigured { code, reason } => {
            assert_eq!(*code, "unconfigured");
            assert!(reason.contains("未填写凭据引用"), "{reason}");
            assert!(!reason.to_ascii_lowercase().contains("ghp_"), "{reason}");
        }
        other => panic!("生产凭据端口必须未装配：{other:?}"),
    }
    let fixture = site();
    super::use_test_endpoints_once(fixture_endpoints(&fixture, CredentialLookup::production()));
    assert_eq!(super::source_endpoints().github.api_origin, fixture.origin);
    // 用一次即清空：下一次取到的又是生产常量。
    assert_eq!(super::source_endpoints().github.api_origin, "https://api.github.com");
}

#[test]
fn g2_0_ct_and_wayback_reach_the_fixture_through_the_production_worker_and_usage_is_counted_once() {
    let site = site();
    let index = Fixture::start(routes(vec![
        ("/", vec![api(200, &[], r#"[{"name_value":"a.example.test\nb.example.test","not_before":"2026-01-01","not_after":"2027-01-01"}]"#)]),
        ("/cdx/search/cdx", vec![api(200, &[], r#"[["original","statuscode","mimetype","timestamp"],["https://a.example.test/config.json","200","application/json","20260101000000"]]"#)]),
    ]));
    let root = sandbox("index");
    let (connection, db_path) = database(&root);
    let project_id = 501;
    let settings = github_project(&connection, project_id, &site, &["passive_web", "certificate_transparency", "web_archive"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).expect("必须能开始");
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    assert_eq!(source_result(&connection, started.run_id, "certificate_transparency").0, "completed");
    assert_eq!(source_result(&connection, started.run_id, "web_archive").0, "completed");
    assert_eq!(index.request_count(), 2, "{:?}", index.requests());
    let ct_names: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND kind='ct_name'", [started.run_id], |row| row.get(0))
        .unwrap();
    assert_eq!(ct_names, 2);
    // 计量：三个工作项各自记增量，总数就是两个 fixture 实际收到的请求数。
    let sent = (index.request_count() + site.request_count()) as i64;
    assert_eq!(requests_used(&connection, started.run_id), sent, "请求数不能被逐项累计重复记账");
    let per_item: i64 = connection
        .query_row("SELECT COALESCE(SUM(requests),0) FROM exposure_work_items WHERE run_id=?1", [started.run_id], |row| row.get(0))
        .unwrap();
    assert_eq!(per_item, sent);
}

// ───────────────────────────── G2-1 ─────────────────────────────

#[test]
fn g2_1_pages_follow_the_real_link_header_and_stop_at_the_last_page() {
    let query = github::repository_query("example.test");
    let fixture = start_paged(&query, 35, 30);
    let mut session = GithubSession::default();
    let (outcome, used) = run_repositories(&fixture, "example.test", &mut session, 50);
    assert_eq!(used, 2);
    assert_eq!(fixture.request_count(), 2);
    assert!(fixture.requests()[1].contains("page=2"), "{:?}", fixture.requests());
    assert_eq!(observations_of(&outcome, "github_repository"), 35);
    assert_eq!(observations_of(&outcome, "github_search_page"), 2);
    assert_eq!(outcome.findings.len(), 35);
    assert_eq!(outcome.status, Some(WorkItemStatus::Completed));
    assert_eq!(outcome.source_status, Some(SourceStatus::Completed), "{}", outcome.reason);
    assert!(outcome.reason.contains("有界结果"), "{}", outcome.reason);
}

/// A fixture whose page 1 carries a Link to its own page 2.
fn start_paged(query: &str, total: i64, first: i64) -> Fixture {
    start_paged_with(query, total, first, total - first, &[], None)
}

fn start_paged_with(query: &str, total: i64, first: i64, second: i64, page1_headers: &[(&str, &str)], link_override: Option<String>) -> Fixture {
    // The responder is scripted per path; the Link must name the fixture's own origin, which is
    // known only after binding. Bind, read the port, then serve a fixture on that same port.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let origin = format!("http://127.0.0.1:{port}");
    let link = link_override.unwrap_or_else(|| next_link(&origin, query, 2));
    let mut headers: Vec<(&str, &str)> = vec![("Link", link.as_str())];
    headers.extend_from_slice(page1_headers);
    let page1 = api(200, &headers, &repo_page(total, false, repos(1, first)));
    // Page 2 links on to page 3 as GitHub would, so the plan's page cap — not a missing Link —
    // is what stops a third request.
    let link3 = next_link(&origin, query, 3);
    let page2 = api(200, &[("Link", link3.as_str())], &repo_page(total, false, repos(1000 + first, second.max(0))));
    super::fixture::Fixture::start_on(port, routes(vec![("/search/repositories", vec![page1, page2])]))
}

#[test]
fn g2_1_the_plan_reads_at_most_two_pages_and_names_the_unread_tail_and_the_api_window() {
    let query = github::repository_query("example.test");
    let fixture = start_paged_with(&query, 5_000, 30, 30, &[], None);
    let mut session = GithubSession::default();
    let (outcome, used) = run_repositories(&fixture, "example.test", &mut session, 50);
    assert_eq!(used, 2, "按计划最多两页");
    assert_eq!(fixture.request_count(), 2, "第 2 页带着第 3 页的 Link，也不能多读：{:?}", fixture.requests());
    assert_eq!(outcome.findings.len(), 60);
    assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial));
    assert!(outcome.reason.contains("未读尾部 940 条"), "{}", outcome.reason);
    assert!(outcome.reason.contains("超出接口结果窗口 1000 条"), "{}", outcome.reason);
}

#[test]
fn g2_1_incomplete_results_are_partial_and_an_empty_answer_is_completed_no_matches() {
    let incomplete = Fixture::start(routes(vec![("/search/repositories", vec![api(200, &[], &repo_page(2, true, repos(1, 2)))])]));
    let (outcome, _) = run_repositories(&incomplete, "example.test", &mut GithubSession::default(), 10);
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial));
    assert!(outcome.reason.contains("incomplete_results"), "{}", outcome.reason);

    let empty = Fixture::start(routes(vec![("/search/repositories", vec![api(200, &[], &repo_page(0, false, vec![]))])]));
    let (outcome, used) = run_repositories(&empty, "example.test", &mut GithubSession::default(), 10);
    assert_eq!(used, 1);
    assert_eq!(outcome.status, Some(WorkItemStatus::Completed));
    assert_eq!(outcome.source_status, Some(SourceStatus::CompletedNoMatches));
    assert!(outcome.findings.is_empty());
    assert!(outcome.reason.contains("不代表没有泄漏"), "{}", outcome.reason);
    assert!(!SourceStatus::CompletedNoMatches.is_gap());
}

#[test]
fn g2_1_each_provider_refusal_has_its_own_status_and_code_and_none_looks_empty() {
    type Case<'a> = (u16, Vec<(&'a str, &'a str)>, &'a str, SourceStatus, WorkItemStatus, &'a str);
    let cases: Vec<Case> = vec![
        (401, vec![], r#"{"message":"Bad credentials"}"#, SourceStatus::AuthFailed, WorkItemStatus::Failed, "github_auth_failed_401"),
        (403, vec![("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1791000000")], r#"{"message":"API rate limit exceeded"}"#, SourceStatus::RateLimited, WorkItemStatus::BlockedQuota, "github_rate_limited_403"),
        (403, vec![("Retry-After", "60")], r#"{"message":"You have exceeded a secondary rate limit"}"#, SourceStatus::RateLimited, WorkItemStatus::BlockedQuota, "github_rate_limited_403"),
        (403, vec![], r#"{"message":"Forbidden"}"#, SourceStatus::AuthFailed, WorkItemStatus::Failed, "github_forbidden_403"),
        (422, vec![], r#"{"message":"Validation Failed"}"#, SourceStatus::Failed, WorkItemStatus::Failed, "github_query_rejected_422"),
        (429, vec![("Retry-After", "30")], r#"{"message":"Too many"}"#, SourceStatus::RateLimited, WorkItemStatus::BlockedQuota, "github_rate_limited_429"),
        (503, vec![], r#"{"message":"Service unavailable"}"#, SourceStatus::Failed, WorkItemStatus::Failed, "github_unavailable_503"),
    ];
    let mut codes = Vec::new();
    for (status, headers, body, source, item_status, code) in cases {
        let fixture = Fixture::start(routes(vec![("/search/repositories", vec![api(status, &headers, body)])]));
        let (outcome, used) = run_repositories(&fixture, "example.test", &mut GithubSession::default(), 10);
        assert_eq!(used, 1, "{status}: 不自动重试");
        assert_eq!(fixture.request_count(), 1);
        assert_eq!(outcome.source_status, Some(source), "{status}: {}", outcome.reason);
        assert_eq!(outcome.status, Some(item_status), "{status}");
        assert!(outcome.reason.starts_with(code), "{status}: {}", outcome.reason);
        assert!(outcome.findings.is_empty());
        let page = outcome.observations.iter().find(|row| row.kind == "github_search_page").expect("错误页也要留观察");
        assert_eq!(page.status_code, i64::from(status));
        assert_eq!(page.extra["detail"]["code"], code);
        codes.push((status, outcome.reason.clone()));
    }
    let not_json = Fixture::start(routes(vec![("/search/repositories", vec![response(200, &[("Content-Type", "text/html")], "<html>just a moment</html>")])]));
    let (outcome, _) = run_repositories(&not_json, "example.test", &mut GithubSession::default(), 10);
    assert_eq!(outcome.source_status, Some(SourceStatus::ParseError));
    assert!(codes.iter().all(|(_, reason)| !reason.contains("未命中")));
}

#[test]
fn g2_1_once_github_says_the_quota_is_gone_later_items_send_nothing() {
    let fixture = Fixture::start(routes(vec![(
        "/search/repositories",
        vec![api(403, &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1791000000")], r#"{"message":"API rate limit exceeded"}"#)],
    )]));
    let mut session = GithubSession::default();
    let (first, _) = run_repositories(&fixture, "example.test", &mut session, 10);
    assert_eq!(first.source_status, Some(SourceStatus::RateLimited));
    let (second, used) = run_repositories(&fixture, "样本公司", &mut session, 10);
    assert_eq!(used, 0, "额度已用尽后不再发请求");
    assert_eq!(fixture.request_count(), 1);
    assert_eq!(second.source_status, Some(SourceStatus::RateLimited));
    assert_eq!(second.status, Some(WorkItemStatus::BlockedQuota));

    // remaining=0 on a successful page 1: page 2 is not requested even though Link says so.
    let query = github::repository_query("example.test");
    let paged = start_paged_with(&query, 100, 30, 30, &[("x-ratelimit-remaining", "0"), ("x-ratelimit-limit", "10")], None);
    let mut session = GithubSession::default();
    let (outcome, used) = run_repositories(&paged, "example.test", &mut session, 10);
    assert_eq!(used, 1);
    assert_eq!(outcome.findings.len(), 30, "第一页的结果保留");
    assert_eq!(outcome.source_status, Some(SourceStatus::RateLimited));
    assert!(outcome.reason.contains("未读尾部 70 条"), "{}", outcome.reason);
    let page = outcome.observations.iter().find(|row| row.kind == "github_search_page").unwrap();
    assert_eq!(page.extra["rateLimit"]["remaining"], 0);
    assert_eq!(page.extra["rateLimit"]["limit"], 10);
}

#[test]
fn g2_1_the_request_budget_cuts_pagination_and_keeps_what_was_read() {
    let query = github::repository_query("example.test");
    let fixture = start_paged(&query, 60, 30);
    let (outcome, used) = run_repositories(&fixture, "example.test", &mut GithubSession::default(), 1);
    assert_eq!(used, 1);
    assert_eq!(fixture.request_count(), 1);
    assert_eq!(outcome.findings.len(), 30);
    assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial));
    assert!(outcome.reason.contains("request_budget_exhausted"), "{}", outcome.reason);
    assert!(outcome.reason.contains("已保存"), "{}", outcome.reason);
}

#[test]
fn g2_1_private_or_malformed_rows_are_skipped_and_returned_urls_are_never_fetched() {
    let evil = Fixture::start(routes(vec![("/any", vec![response(200, &[], "should never be read")])]));
    let mut planted = repo(7, "acme-dev/portal", "门户");
    planted["html_url"] = json!(evil.url("/any/portal"));
    let mut private = repo(8, "acme-dev/secret", "私有");
    private["private"] = json!(true);
    let malformed = repo(9, "../../etc", "坏名字");
    let fixture = Fixture::start(routes(vec![(
        "/search/repositories",
        vec![api(200, &[], &repo_page(3, false, vec![planted, private, malformed]))],
    )]));
    let (outcome, _) = run_repositories(&fixture, "example.test", &mut GithubSession::default(), 10);
    assert_eq!(evil.request_count(), 0, "API 返回的 URL 不是出站授权");
    assert_eq!(outcome.findings.len(), 1);
    let hit = outcome.observations.iter().find(|row| row.kind == "github_repository").unwrap();
    assert_eq!(hit.url, "https://github.com/acme-dev/portal", "规范地址只由 full_name 构造");
    assert_eq!(hit.extra["providerUrlMismatch"], true);
    assert!(outcome.reason.contains("跳过 1 条非公开仓库"), "{}", outcome.reason);
    assert!(outcome.reason.contains("跳过 1 条名称不合规的结果（../../etc）"), "{}", outcome.reason);
    // Regression from the live check: a legacy login with a double hyphen is a real public repo.
    let legacy = github::repository_hit(&repo(10, "Rob--W/cors-anywhere", "proxy")).expect("旧式登录名必须接受");
    assert_eq!(legacy.canonical_url, "https://github.com/Rob--W/cors-anywhere");
    assert_eq!(legacy.owner, "rob--w");
    for unsafe_name in ["a/b/c", "a_b/c", "owner/..", "owner/re po", "/repo", "owner/"] {
        assert!(github::valid_full_name(unsafe_name).is_none(), "{unsafe_name} 不能用来拼地址");
    }
}

#[test]
fn g2_1_a_link_to_another_origin_or_query_is_not_followed() {
    let evil = Fixture::start(routes(vec![("/search/repositories", vec![api(200, &[], &repo_page(0, false, vec![]))])]));
    let query = github::repository_query("example.test");
    let foreign = next_link(&evil.origin, &query, 2);
    let fixture = start_paged_with(&query, 60, 30, 30, &[], Some(foreign));
    let (outcome, used) = run_repositories(&fixture, "example.test", &mut GithubSession::default(), 10);
    assert_eq!(used, 1);
    assert_eq!(evil.request_count(), 0);
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial));
    assert!(outcome.reason.contains("github_link_rejected"), "{}", outcome.reason);
    assert!(github::accept_next(&format!("{}/search/repositories?q=other&page=2", fixture.origin), &fixture.origin, "/search/repositories", &query, 2).is_err());
    assert!(github::accept_next(&format!("{}/search/code?q=x&page=2", fixture.origin), &fixture.origin, "/search/repositories", "x", 2).is_err());
    assert!(github::accept_next(&format!("{}/search/repositories?q=x&page=5", fixture.origin), &fixture.origin, "/search/repositories", "x", 2).is_err());
}

#[test]
fn g2_1_cancel_before_the_first_page_sends_nothing() {
    let fixture = Fixture::start(routes(vec![("/search/repositories", vec![api(200, &[], &repo_page(0, false, vec![]))])]));
    let mut meter = meter(10);
    let cancelled = || true;
    let outcome = execute_repositories(&client(), &item(KIND_GITHUB_REPOSITORIES, "example.test"), &mut meter, &endpoint(&fixture), &mut GithubSession::default(), &[], &cancelled);
    assert_eq!(outcome.status, Some(WorkItemStatus::Cancelled));
    assert_eq!(fixture.request_count(), 0);
}

#[test]
fn g2_1_repository_search_runs_through_the_production_worker_as_leads_not_assets() {
    let site = site();
    let found = vec![repo(11, "acme-dev/portal", "example.test 门户前端"), repo(12, "acme-official/sdk", "官方 SDK")];
    let index = Fixture::start(routes(vec![(
        "/search/repositories",
        vec![api(200, &[("x-ratelimit-remaining", "8"), ("x-ratelimit-limit", "10")], &repo_page(2, false, found)), api(200, &[], &repo_page(0, false, vec![]))],
    )]));
    let root = sandbox("worker");
    let (connection, db_path) = database(&root);
    let project_id = 511;
    let assets_before: i64 = connection.query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0)).unwrap();
    let settings = github_project(&connection, project_id, &site, &["passive_web", "github_repositories"], &["acme-official"]);
    let started = start_or_reuse(&connection, project_id, &settings).expect("必须能开始");
    // 计划：example.test 与实体全称「样本公司」各一条查询。
    let planned: Vec<String> = connection
        .prepare("SELECT entity FROM exposure_work_items WHERE run_id=?1 AND kind='github_repositories' ORDER BY id")
        .unwrap()
        .query_map([started.run_id], |row| row.get(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(planned, vec!["example.test".to_string(), "样本公司".to_string()]);
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    let (status, _) = source_result(&connection, started.run_id, "github_repositories");
    assert_eq!(status, "completed", "一条命中一条为空，来源整体是 completed");
    assert_eq!(index.request_count(), 2);
    assert!(index.requests().iter().all(|row| row.starts_with("/search/repositories?q=")));
    assert_eq!(disposition(&connection, project_id, "https://github.com/acme-dev/portal"), ("classification_pending".into(), "code_platform".into()));
    assert_eq!(disposition(&connection, project_id, "https://github.com/acme-official/sdk").0, "excluded_public_material");
    let assets_after: i64 = connection.query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0)).unwrap();
    assert_eq!(assets_after, assets_before + 1, "只有 fixture 授权站点是资产，GitHub 命中不建资产");
    let github_clusters: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_observations o WHERE o.run_id=?1 AND o.source_key='github_repositories' AND o.asset_id IS NOT NULL", [started.run_id], |row| row.get(0))
        .unwrap();
    assert_eq!(github_clusters, 0, "外部平台的观察不挂到任何资产，也就不会进集团域名簇");
    let pending_github: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_pending_discoveries WHERE project_id=?1 AND (entity LIKE '%github%' OR source_key LIKE 'github%')", [project_id], |row| row.get(0))
        .unwrap();
    assert_eq!(pending_github, 0);
    assert_eq!(requests_used(&connection, started.run_id), (index.request_count() + site.request_count()) as i64);
}

#[test]
fn g2_1_all_queries_empty_is_completed_no_matches_and_the_round_is_not_a_gap() {
    let site = site();
    let index = Fixture::start(routes(vec![("/search/repositories", vec![api(200, &[], &repo_page(0, false, vec![]))])]));
    let root = sandbox("empty");
    let (connection, db_path) = database(&root);
    let project_id = 512;
    let settings = github_project(&connection, project_id, &site, &["passive_web", "github_repositories"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).unwrap();
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    assert_eq!(source_result(&connection, started.run_id, "github_repositories").0, "completed_no_matches");
    let run: String = connection.query_row("SELECT status FROM exposure_runs WHERE id=?1", [started.run_id], |row| row.get(0)).unwrap();
    assert_eq!(run, "completed");
}

// ───────────────────────────── G2-2 ─────────────────────────────

#[test]
fn g2_2_code_search_without_a_credential_is_unconfigured_and_sends_nothing() {
    let site = site();
    let index = Fixture::start(routes(vec![("/search/code", vec![api(200, &[], r#"{"total_count":0,"items":[]}"#)])]));
    let root = sandbox("nocred");
    let (connection, db_path) = database(&root);
    let project_id = 521;
    let settings = github_project(&connection, project_id, &site, &["passive_web", "github_code"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).unwrap();
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    assert_eq!(index.request_count(), 0, "没有凭据时代码搜索不发任何请求");
    let (status, error) = source_result(&connection, started.run_id, "github_code");
    assert_eq!(status, "unconfigured");
    assert!(error.contains("未填写凭据引用"), "{error}");
    let run: String = connection.query_row("SELECT status FROM exposure_runs WHERE id=?1", [started.run_id], |row| row.get(0)).unwrap();
    assert_ne!(run, "completed", "未配置的来源是缺口，不报完成");
}

fn code_item(path: &str, sha: &str, full_name: &str, fragment: &str) -> JsonValue {
    json!({
        "name": path.rsplit('/').next().unwrap(),
        "path": path,
        "sha": sha,
        "html_url": format!("https://github.com/{full_name}/blob/0123456789abcdef0123456789abcdef01234567/{path}"),
        "repository": {"id": 77, "full_name": full_name, "private": false, "html_url": format!("https://github.com/{full_name}")},
        "text_matches": [{"fragment": fragment}],
    })
}

fn blob(sha: &str, content: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    json!({"sha": sha, "size": content.len(), "encoding": "base64", "content": encoded}).to_string()
}

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn g2_2_code_search_with_a_credential_reads_blobs_by_sha_and_the_token_only_reaches_the_api() {
    // A private repository is never taken, even though the credential could see it.
    let mut private = code_item("secret.txt", SHA_A, "acme-dev/private", "example.test");
    private["repository"]["private"] = json!(true);
    let hits = json!({"total_count": 4, "incomplete_results": false, "items": [
        code_item("deploy/app.yml", SHA_A, "acme-dev/ops", "host: db.example.test"),
        code_item("README.md", SHA_B, "acme-dev/ops", "see example.test"),
        code_item("docs/notice.txt", SHA_C, "acme-dev/ops", "example.test"),
        private,
    ]})
    .to_string();
    let fixture = Fixture::start(routes(vec![
        ("/search/code", vec![api(200, &[], &hits)]),
        ("/repos/acme-dev/ops/git/blobs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", vec![api(200, &[], &blob(SHA_A, "host: db.example.test\ndb_password: \"Zx9kq2Lm8Pw4Rt\"\n"))]),
        // A blob answer for another sha is discarded, never analysed as this file.
        ("/repos/acme-dev/ops/git/blobs/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", vec![api(200, &[], &blob(SHA_C, "db_password: \"Qq1Ww2Ee3Rr4Tt\""))]),
        ("/repos/acme-dev/ops/git/blobs/cccccccccccccccccccccccccccccccccccccccc", vec![api(200, &[], &blob(SHA_C, "example.test 公开说明"))]),
    ]));
    let mut meter = meter(20);
    let outcome = execute_code(&client(), &item(KIND_GITHUB_CODE, "example.test"), &mut meter, &endpoint(&fixture), &mut GithubSession::default(), &token(), &never);
    assert_eq!(meter.requests_used, 4, "1 次搜索 + 3 次 blob");
    let heads = fixture.heads();
    assert_eq!(heads.len(), 4);
    for head in &heads {
        assert!(head.to_ascii_lowercase().contains(&format!("authorization: bearer {}", TOKEN.to_ascii_lowercase())), "{head}");
        let line = head.lines().next().unwrap();
        assert!(!line.contains(TOKEN), "token 不能进 URL：{line}");
        assert!(head.contains("X-GitHub-Api-Version: 2022-11-28") || head.to_ascii_lowercase().contains("x-github-api-version: 2022-11-28"));
    }
    assert!(heads[0].to_ascii_lowercase().contains("accept: application/vnd.github.text-match+json"));
    assert_eq!(observations_of(&outcome, "github_code_hit"), 3);
    assert!(outcome.observations.iter().all(|row| !row.url.contains("acme-dev/private")), "私有仓库不收");
    assert_eq!(observations_of(&outcome, "github_blob_read"), 3, "摘要命中与正文读取分开记录");
    let credential = outcome.findings.iter().find(|row| row.url.ends_with("deploy/app.yml")).unwrap();
    assert_eq!(credential.category, "credential");
    assert!(credential.evidence.contains("已按 blob sha 读取并核对正文"), "{}", credential.evidence);
    assert!(!credential.evidence.contains("Zx9kq2Lm8Pw4Rt"), "证据只留掩码");
    let mismatched = outcome.findings.iter().find(|row| row.url.ends_with("README.md")).unwrap();
    assert_eq!(mismatched.category, "source_code", "sha 对不上的正文被丢弃，不能据此报凭据");
    assert!(outcome.reason.contains("github_blob_mismatch"), "{}", outcome.reason);
    assert_eq!(outcome.source_status, Some(SourceStatus::Partial), "有一份正文没读成，来源不是 completed");
    let read = outcome.observations.iter().find(|row| row.kind == "github_blob_read" && row.url.ends_with(SHA_B)).unwrap();
    assert_eq!(read.extra["verified"], false);
}

#[test]
fn g2_2_a_redirect_is_not_followed_and_the_token_never_reaches_the_other_origin() {
    let evil = Fixture::start(routes(vec![("/any", vec![api(200, &[], r#"{"total_count":0,"items":[]}"#)])]));
    let target = evil.url("/any/search/code");
    let fixture = Fixture::start(routes(vec![("/search/code", vec![response(302, &[("Location", target.as_str())], "")])]));
    let root = sandbox("redirect");
    let (connection, _) = database(&root);
    let mut meter = meter(10);
    let outcome = execute_code(&client(), &item(KIND_GITHUB_CODE, "example.test"), &mut meter, &endpoint(&fixture), &mut GithubSession::default(), &token(), &never);
    assert_eq!(evil.request_count(), 0, "跨站跳转不跟随");
    assert_eq!(outcome.source_status, Some(SourceStatus::Failed));
    assert!(outcome.reason.starts_with("github_redirect_not_followed"), "{}", outcome.reason);
    assert!(outcome.reason.contains(&evil.origin), "{}", outcome.reason);
    // Persist what the adapter produced and prove the token is nowhere in the database.
    super::fixture::seed_scope(&connection, 71, "http://127.0.0.1:9", true);
    for observation in &outcome.observations {
        super::observations::record(&connection, 1, 71, None, observation).ok();
    }
    assert!(!everything_stored(&connection).contains(TOKEN));
    assert!(!format!("{outcome:?}").contains(TOKEN));
}

#[test]
fn g2_2_a_credential_is_refused_for_any_other_origin_before_anything_is_sent() {
    let api_fixture = Fixture::start(routes(vec![]));
    let other = Fixture::start(routes(vec![("/x", vec![api(200, &[], "{}")])]));
    let secret = ProviderAuthorization::bearer(&api_fixture.origin, TOKEN);
    let mut meter = meter(10);
    let url = reqwest::Url::parse(&other.url("/x")).unwrap();
    let result = client().fetch_provider_api(&url, std::slice::from_ref(&other.origin), &[], Some(&secret), &mut meter, true, &never);
    match result {
        Err(FetchBlock::Denied(denial)) => assert_eq!(denial.code, "provider_credential_origin_mismatch"),
        other => panic!("必须在发送前拒绝：{:?}", other.map(|f| f.status)),
    }
    assert_eq!(other.request_count(), 0);
    assert_eq!(meter.requests_used, 0, "被拒绝的请求不扣额度，因为根本没发");
    // Userinfo and a non-configured origin are refused too.
    let with_user = reqwest::Url::parse(&format!("http://u:p@127.0.0.1:{}/x", other.port)).unwrap();
    assert!(matches!(client().fetch_provider_api(&with_user, std::slice::from_ref(&other.origin), &[], None, &mut meter, true, &never), Err(FetchBlock::Denied(d)) if d.code == "provider_userinfo_denied"));
    assert!(matches!(client().fetch_provider_api(&url, std::slice::from_ref(&api_fixture.origin), &[], None, &mut meter, true, &never), Err(FetchBlock::Denied(d)) if d.code == "provider_endpoint_not_configured"));
    assert!(matches!(client().fetch_provider_api(&url, std::slice::from_ref(&other.origin), &[], None, &mut meter, false, &never), Err(FetchBlock::Denied(d)) if d.code == "provider_endpoint_private_denied"));
    assert_eq!(other.request_count(), 0);
}

#[test]
fn g2_2_the_token_is_never_printed_echoed_or_stored() {
    let token_value = GithubToken::new(TOKEN).unwrap();
    assert_eq!(format!("{token_value:?}"), "GithubToken(<redacted>)");
    assert!(!format!("{:?}", token()).contains(TOKEN));
    assert!(!format!("{:?}", ProviderAuthorization::bearer("https://api.github.com", TOKEN)).contains(TOKEN));
    for bad in ["", "short", "ghp_has space inside", "ghp_crlf\r\nX-Evil: 1", "ghp_ünïcode_value"] {
        assert!(GithubToken::new(bad).is_none(), "{bad:?} 不能成为 token（防头部注入）");
    }
    // A provider that echoes the token in its error message: the stored reason is scrubbed.
    let echoed = format!(r#"{{"message":"Bad credentials for {TOKEN}"}}"#);
    let fixture = Fixture::start(routes(vec![("/search/code", vec![api(401, &[], &echoed)])]));
    let mut meter = meter(10);
    let outcome = execute_code(&client(), &item(KIND_GITHUB_CODE, "example.test"), &mut meter, &endpoint(&fixture), &mut GithubSession::default(), &token(), &never);
    assert_eq!(outcome.source_status, Some(SourceStatus::AuthFailed));
    assert!(!format!("{outcome:?}").contains(TOKEN), "{}", outcome.reason);
    assert!(!outcome.reason.contains(TOKEN));
}

#[test]
fn g2_2_the_code_search_worker_path_with_a_test_credential_stores_no_token() {
    let site = site();
    let hits = json!({"total_count": 1, "incomplete_results": false, "items": [code_item("conf/app.env", SHA_A, "acme-dev/ops", "db.example.test")]}).to_string();
    let index = Fixture::start(routes(vec![
        ("/search/code", vec![api(200, &[], &hits)]),
        ("/repos/acme-dev/ops/git/blobs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", vec![api(200, &[], &blob(SHA_A, "DB_HOST=db.example.test\napi_key: \"Lm8Pw4RtZx9kq2Q1\"\n"))]),
    ]));
    let root = sandbox("codeworker");
    let (connection, db_path) = database(&root);
    let project_id = 522;
    let settings = github_project(&connection, project_id, &site, &["passive_web", "github_code"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).unwrap();
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, token()));
    assert_eq!(source_result(&connection, started.run_id, "github_code").0, "completed");
    let url = "https://github.com/acme-dev/ops/blob/0123456789abcdef0123456789abcdef01234567/conf/app.env";
    assert_eq!(disposition(&connection, project_id, url), ("risk_candidate".into(), "code_platform".into()));
    let stored = everything_stored(&connection);
    assert!(!stored.contains(TOKEN), "token 不能出现在库里任何一格");
    assert!(!stored.contains("Lm8Pw4RtZx9kq2Q1"), "凭据原值不能入库");
}

// ───────────────────────────── G2-3 ─────────────────────────────

fn draft(category: &str, title: &str, url: &str, confidence: i64) -> super::observations::FindingDraft {
    super::observations::FindingDraft {
        category: category.into(),
        title: title.into(),
        url: url.into(),
        source_type: "github".into(),
        evidence: String::new(),
        severity: "low".into(),
        confidence,
        asset_id: None,
        content_hash: String::new(),
    }
}

#[test]
fn g2_3_code_platform_hits_reuse_the_g1_3_rules_and_only_credentials_become_risk() {
    use super::classify::{classify, page_context, PageContext};
    let plain = PageContext::default();
    let repo_draft = draft("repository", github::TITLE_REPOSITORY, "https://github.com/a/b", 30);
    let verdict = classify(&repo_draft, "github_repositories", &plain);
    assert_eq!((verdict.source_channel, verdict.disposition), ("code_platform", "classification_pending"));
    assert!(verdict.reason.contains("第一方归属未证实"), "{}", verdict.reason);
    let official = draft("repository", github::TITLE_OFFICIAL_REPOSITORY, "https://github.com/a/b", 30);
    assert_eq!(classify(&official, "github_repositories", &plain).disposition, "excluded_public_material");
    let code = draft("source_code", github::TITLE_CODE_HIT, "https://github.com/a/b/blob/x/y", 30);
    assert_eq!(classify(&code, "github_code", &plain).disposition, "classification_pending");
    let candidate = draft("credential", github::TITLE_CODE_CREDENTIAL, "https://github.com/a/b/blob/x/z", super::rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE + 40);
    assert_eq!(classify(&candidate, "github_code", &plain).disposition, "risk_candidate");
    let placeholder = draft("credential", github::TITLE_CODE_CREDENTIAL, "https://github.com/a/b/blob/x/z", super::rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE);
    assert_eq!(classify(&placeholder, "github_code", &plain).disposition, "excluded_public_material");
    // G1-3 procurement exclusion applies to a repository whose own words are a notice.
    let notice = page_context(
        "https://github.com/a/notices",
        "a/notices\n招标公告 项目编号 ZB-1 采购人 某公司 预算金额 100 万".as_bytes(),
        "text/plain",
    );
    assert_eq!(classify(&repo_draft, "github_repositories", &notice).disposition, "excluded_public_procurement");
    // A procurement page that also carries a credential stays with a human (mixed page).
    assert_eq!(classify(&candidate, "github_code", &notice).disposition, "classification_pending");
}

#[test]
fn g2_3_github_sources_are_opt_in_and_official_owners_are_validated() {
    let defaults = ExposureSettings::for_project(1);
    assert!(!defaults.sources.iter().any(|row| row.starts_with("github")), "GitHub 默认不启用");
    let json_default = defaults.to_settings_json();
    assert!(json_default.get("exposureGithubOfficialOwners").is_none(), "空名单不进冻结配置，旧计划的选项哈希不变");
    let parsed = ExposureSettings::from_json(1, &json!({"exposureSources": ["github_repositories", "github_code"], "exposureGithubOfficialOwners": ["Acme-Official", "acme-official", "acme-labs"]})).unwrap();
    assert_eq!(parsed.sources, vec!["github_repositories".to_string(), "github_code".to_string()]);
    assert_eq!(parsed.github_official_owners, vec!["acme-labs".to_string(), "acme-official".to_string()]);
    assert_eq!(ExposureSettings::from_json(1, &parsed.to_settings_json()).unwrap(), {
        let mut again = parsed.clone();
        again.version = parsed.version;
        again
    });
    // Legacy logins with odd hyphens are real (live check: `Rob--W`) and must be accepted.
    let legacy = ExposureSettings::from_json(1, &json!({"exposureGithubOfficialOwners": ["Rob--W", "-edge-"]})).unwrap();
    assert_eq!(legacy.github_official_owners, vec!["-edge-".to_string(), "rob--w".to_string()]);
    for bad in [json!(["---"]), json!(["a_b"]), json!(["has space"]), json!(["../x"]), json!(["x".repeat(40)]), json!("acme")] {
        assert!(ExposureSettings::from_json(1, &json!({"exposureGithubOfficialOwners": bad})).is_err(), "{bad}");
    }
    let many: Vec<String> = (0..51).map(|i| format!("owner{i}")).collect();
    assert!(ExposureSettings::from_json(1, &json!({"exposureGithubOfficialOwners": many})).is_err());
}

#[test]
fn g2_3_the_settings_view_lists_each_github_capability_separately() {
    let root = sandbox("view");
    let (connection, _) = database(&root);
    connection.execute("INSERT INTO projects(id,name,status) VALUES(531,'g2','active')", []).unwrap();
    let view = super::exposure_settings_view_in(&connection, 531).expect("配置视图");
    let caps = view["sourceCapabilities"].as_array().unwrap();
    let state = |cap: &str| caps.iter().find(|row| row["capability"] == cap).unwrap().clone();
    assert_eq!(state("repository_metadata")["state"], "configured");
    assert_eq!(state("repository_metadata")["credential"], "none");
    assert_eq!(state("default_branch_code_search")["state"], "unconfigured");
    assert_eq!(state("default_branch_code_search")["integration"], "unconfigured");
    assert!(state("default_branch_code_search")["reason"].as_str().unwrap_or("").contains("凭据引用"));
    assert_eq!(state("controlled_blob_read")["state"], "unconfigured");
    assert!(view["bounds"]["sources"].as_array().unwrap().iter().any(|row| row == "github_code"));
    assert!(!view["defaultSources"].as_array().unwrap().iter().any(|row| row == "github_code"));
}

// ───────────────────────────── G2-4 ─────────────────────────────

#[test]
fn g2_4_a_repository_lead_is_reviewed_and_a_changed_repository_comes_back_for_review() {
    use super::remediation::{review_scoped, top_revision};
    let site = site();
    let first = vec![repo(21, "acme-dev/portal", "example.test 门户前端")];
    let changed = vec![repo(21, "acme-dev/portal", "example.test 门户前端（含部署脚本）")];
    let index = Fixture::start(routes(vec![(
        "/search/repositories",
        vec![
            api(200, &[], &repo_page(1, false, first.clone())),
            api(200, &[], &repo_page(0, false, vec![])),
            api(200, &[], &repo_page(1, false, first)),
            api(200, &[], &repo_page(0, false, vec![])),
            api(200, &[], &repo_page(1, false, changed)),
            api(200, &[], &repo_page(0, false, vec![])),
        ],
    )]));
    let root = sandbox("loop");
    let (connection, db_path) = database(&root);
    let project_id = 541;
    let settings = github_project(&connection, project_id, &site, &["passive_web", "github_repositories"], &[]);
    let url = "https://github.com/acme-dev/portal";
    let round = |label: &str| {
        let started = start_or_reuse(&connection, project_id, &settings).unwrap_or_else(|e| panic!("{label}: {e}"));
        run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
        started.run_id
    };
    let r1 = round("第一轮");
    let id: i64 = connection.query_row("SELECT id FROM exposure_findings WHERE project_id=?1 AND source_url=?2", params![project_id, url], |row| row.get(0)).unwrap();
    let obs: i64 = connection.query_row("SELECT id FROM exposure_observations WHERE run_id=?1 AND url=?2", params![r1, url], |row| row.get(0)).unwrap();
    review_scoped(&connection, project_id, id, top_revision(&connection, id).unwrap(), "dismissed", "李四", "外包商的公开示例仓库", &[obs]).expect("复核写入");
    // 第二轮：同一仓库、同样内容——仍是同一条线索，人工结论不被覆盖。
    let _r2 = round("第二轮");
    let rows: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND source_url=?2", params![project_id, url], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1, "跨轮同一仓库只有一条线索");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
    assert_eq!(status, "dismissed");
    let needs_review = |connection: &Connection| -> i64 {
        connection
            .query_row("SELECT COALESCE(MAX(review_needed),0) FROM exposure_finding_versions WHERE finding_id=?1", [id], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(needs_review(&connection), 0);
    // 第三轮：仓库描述变了——新版本，提示重新复核，旧结论保留不被冒用。
    let _r3 = round("第三轮");
    assert_eq!(needs_review(&connection), 1, "内容变化必须要求重新复核");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [id], |row| row.get(0)).unwrap();
    assert_eq!(status, "dismissed");
    assert_eq!(index.request_count(), 6);
}

/// Bounded live check against the real GitHub API: one anonymous repository-search request
/// (the meter allows exactly one), public metadata only, nothing written anywhere but memory.
/// Run explicitly: `cargo test --lib g2_live -- --ignored --nocapture`.
#[test]
#[ignore]
fn g2_live_github_anonymous_repository_search_once() {
    let mut meter = meter(1);
    let mut session = GithubSession::default();
    let outcome = execute_repositories(
        &PassiveClient::new(PassiveHttpLimits { timeout: std::time::Duration::from_secs(20), ..PassiveHttpLimits::default() }).unwrap(),
        &item(KIND_GITHUB_REPOSITORIES, "example.com"),
        &mut meter,
        &GithubEndpoint::production(),
        &mut session,
        &[],
        &never,
    );
    let page = outcome.observations.iter().find(|row| row.kind == "github_search_page");
    eprintln!(
        "G2_LIVE requests={} status={:?} source={:?} repos={} rate={:?} page_status={:?} reason={}",
        meter.requests_used,
        outcome.status,
        outcome.source_status,
        outcome.findings.len(),
        session.last_rate,
        page.map(|row| row.status_code),
        outcome.reason
    );
    for row in outcome.observations.iter().filter(|row| row.kind == "github_repository").take(3) {
        eprintln!("G2_LIVE_REPO {} id={} pushed={}", row.url, row.extra["providerObjectId"], row.extra["pushedAt"]);
    }
    assert!(meter.requests_used <= 1);
    assert!(page.is_some(), "必须真的拿到一页（或一个有状态的拒绝）");
}

#[test]
fn g2_0_credential_ref_reads_only_through_the_injected_lookup_and_never_echoes_a_secret() {
    use super::adapters::github::{CredentialLookup, GithubToken};
    let mut calls = 0;
    let empty = CredentialLookup::from_ref_with("", |_| { calls += 1; Ok(None) });
    assert!(matches!(empty, CredentialLookup::Unconfigured { .. }));
    assert_eq!(calls, 0, "空引用不读钥匙串");
    let bad = CredentialLookup::from_ref_with("../token", |_| { calls += 1; Ok(None) });
    assert!(matches!(bad, CredentialLookup::Unconfigured { .. }));
    assert_eq!(calls, 0, "不合规引用不读钥匙串");
    let secret = "fixtureTokenValue99";
    let ready = CredentialLookup::from_ref_with("exposure-github", |_| Ok(Some(secret.to_string())));
    assert!(matches!(ready, CredentialLookup::Available(_)));
    let debug = format!("{ready:?}");
    assert!(!debug.contains(secret), "{debug}");
    let leaked = "not a token !!!";
    let refused = CredentialLookup::from_ref_with("exposure-github", |_| Ok(Some(leaked.to_string())));
    match refused {
        CredentialLookup::Unconfigured { reason, .. } => assert!(!reason.contains(leaked), "{reason}"),
        CredentialLookup::Available(_) => panic!("非法形态不能变成可用凭据"),
    }
    let _ = GithubToken::new(secret);
    let parsed = super::settings::ExposureSettings::from_json(1, &serde_json::json!({"exposureGithubCredentialRef": "exposure-github", "exposureConcurrency": 40})).unwrap();
    assert_eq!(parsed.github_credential_ref, "exposure-github");
    assert_eq!(parsed.concurrency, 1, "送来的并行度被忽略");
    assert!(parsed.to_settings_json().get("exposureConcurrency").is_none());
    assert!(super::settings::ExposureSettings::from_json(1, &serde_json::json!({"exposureGithubCredentialRef": "has space"})).is_err());
}

// ───────────────────────────── P1 修复回归（2026-10-09） ─────────────────────────────

/// P1-3：一轮已经计划好之后撤销授权（或归档项目），证书/归档/代码平台这些外部来源的工作项
/// 不能再发出任何请求，而是记为“授权阻塞”，缺口留在分母里。
#[test]
fn rem_p1_3_external_sources_send_nothing_after_the_scope_is_revoked_mid_round() {
    for (case, revoke) in [
        ("revoked", "UPDATE asset_ownership_decisions SET exposure_eligible=0,authorization_status='revoked' WHERE project_id=?1"),
        ("archived", "UPDATE projects SET status='archived' WHERE id=?1"),
        // 只撤销其中一个资产：仍有别的准入资产，但本轮计划时的授权范围已经变了。
        ("one_revoked", "UPDATE asset_ownership_decisions SET exposure_eligible=0,authorization_status='revoked' WHERE project_id=?1 AND asset_id=(SELECT MIN(asset_id) FROM project_assets WHERE project_id=?1)"),
    ] {
        let other = site();
        let site = site();
        let index = Fixture::start(routes(vec![
            ("/", vec![api(200, &[], r#"[{"name_value":"a.example.test","not_before":"2026-01-01","not_after":"2027-01-01"}]"#)]),
            ("/cdx/search/cdx", vec![api(200, &[], r#"[["original","statuscode","mimetype","timestamp"]]"#)]),
            ("/search/repositories", vec![api(200, &[], r#"{"total_count":0,"incomplete_results":false,"items":[]}"#)]),
        ]));
        let root = sandbox(&format!("rem-revoke-{case}"));
        let (connection, db_path) = database(&root);
        let project_id = 591;
        let settings = github_project(&connection, project_id, &site, &["passive_web", "certificate_transparency", "web_archive", "github_repositories"], &[]);
        if case == "one_revoked" {
            seed_scope(&connection, project_id, &other.origin, true);
        }
        let started = start_or_reuse(&connection, project_id, &settings).expect("计划时仍有授权");
        connection.execute(revoke, [project_id]).unwrap();
        run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
        assert_eq!(index.request_count(), 0, "{case}：撤权后外部来源仍发出了请求 {:?}", index.requests());
        assert_eq!(site.request_count(), 0, "{case}：撤权后站点仍收到请求");
        let blocked: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND source_key<>'passive_web' AND status='blocked_authorization' AND requests=0",
                [started.run_id],
                |row| row.get(0),
            )
            .unwrap();
        let external: i64 = connection
            .query_row("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND source_key<>'passive_web'", [started.run_id], |row| row.get(0))
            .unwrap();
        assert!(external >= 3, "{case}：计划里应有外部来源工作项");
        if case != "archived" {
            assert_eq!(blocked, external, "{case}：每个外部工作项都记为授权阻塞且零请求");
        } else {
            let done: i64 = connection
                .query_row("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND status IN ('completed','completed_no_matches')", [started.run_id], |row| row.get(0))
                .unwrap();
            assert_eq!(done, 0, "{case}：归档项目的轮次不能有任何工作项算作完成");
        }
        // 只有仍准入的另一个站点（one_revoked）可以收到被动请求；记账与它一致。
        assert_eq!(requests_used(&connection, started.run_id), other.request_count() as i64, "{case}");
        let _ = std::fs::remove_dir_all(root);
    }
}

// ───────────────────────────── FIX3（2026-10-09 17:29） ─────────────────────────────

/// FIX3-R4：项内撤权。外部来源的第一个请求已经发出、响应尚未返回时撤权（夹具在回复前执行撤权）。
/// 本项记为授权阻塞；之后零新请求；所有外部工作项显示授权阻塞。
/// FIX4 P1／新增（T21）语义变更：在途那一个请求发出时授权有效，其结果保留并逐条标注授权时间点（原为整项丢弃）；新发现不入队。
#[test]
fn fix3_r4_revoking_while_an_external_request_is_in_flight_keeps_its_marked_result_and_sends_nothing_more() {
    let site = site();
    let root = sandbox("fix3-r4");
    let (connection, db_path) = database(&root);
    let project_id = 592;
    let hook_db = db_path.clone();
    let mut fired = false;
    let index = Fixture::start_hooked(
        routes(vec![
            ("/", vec![api(200, &[], r#"[{"name_value":"a.example.test","not_before":"2026-01-01","not_after":"2027-01-01"},{"name_value":"b.example.test","not_before":"2026-01-01","not_after":"2027-01-01"}]"#)]),
            ("/cdx/search/cdx", vec![api(200, &[], r#"[["original","statuscode","mimetype","timestamp"],["https://a.example.test/x","200","text/html","20260101000000"]]"#)]),
            ("/search/repositories", vec![api(200, &[], r#"{"total_count":0,"incomplete_results":false,"items":[]}"#)]),
        ]),
        Box::new(move |_path: &str| {
            if !fired {
                fired = true;
                let revoker = crate::db::open(&hook_db).unwrap();
                revoker
                    .execute("UPDATE asset_ownership_decisions SET exposure_eligible=0,authorization_status='revoked' WHERE project_id=?1", [project_id])
                    .unwrap();
            }
        }),
    );
    let settings = github_project(&connection, project_id, &site, &["certificate_transparency", "web_archive", "github_repositories"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).expect("计划时仍有授权");
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    assert_eq!(index.request_count(), 1, "撤权后只允许那一个在途请求：{:?}", index.requests());
    let one = |sql: &str| -> i64 { connection.query_row(sql, [started.run_id], |row| row.get(0)).unwrap() };
    let external = one("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND source_key<>'passive_web'");
    let blocked = one("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND source_key<>'passive_web' AND status='blocked_authorization'");
    assert!(external >= 3);
    assert_eq!(blocked, external, "所有外部工作项（含在途那一项）都显示授权阻塞");
    assert_eq!(one("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND requests=1 AND status='blocked_authorization' AND reason LIKE '%已保留并标注授权时间点%'"), 1);
    let kept = one("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND source_key<>'passive_web'");
    assert!(kept >= 1, "在途请求发出时授权有效，结果保留");
    assert_eq!(
        one("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND source_key<>'passive_web' AND json_extract(evidence_json,'$.detail.scopeGrant.withdrawnDetectedAt') IS NOT NULL"),
        kept,
        "保留的每条都带授权时间点"
    );
    assert_eq!(one("SELECT COUNT(*) FROM exposure_pending_discoveries WHERE run_id=?1 AND source_key<>'passive_web'"), 0, "在途发现不入队");
    assert_eq!(one("SELECT COALESCE(SUM(requests),0) FROM exposure_work_items WHERE run_id=?1 AND source_key<>'passive_web'"), 1, "已发出的请求照实记账");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX4（2026-10-09 20:37） ─────────────────────────────

fn fix4_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    port
}

const FIX4_RESET: &str = "1791000000";

fn fix4_reset_text() -> String {
    chrono::DateTime::from_timestamp(1_791_000_000, 0).unwrap().with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 本地假 GitHub：第 1 页满 30 条并带指向自身第 2 页的 Link，第 2 页由调用方给出。
fn fix4_two_pages(query: &str, total: i64, page2: String) -> Fixture {
    let port = fix4_port();
    let link = next_link(&format!("http://127.0.0.1:{port}"), query, 2);
    let page1 = api(200, &[("Link", link.as_str()), ("x-ratelimit-remaining", "1"), ("x-ratelimit-limit", "30")], &repo_page(total, false, repos(1, 30)));
    Fixture::start_on(port, routes(vec![("/search/repositories", vec![page1, page2])]))
}

/// FIX4 P2 GitHub：末页恰好 x-ratelimit-remaining=0。末页结果保留、完整完成、不标截断；
/// 之后同一能力不再发任何请求（下一项直接记额度阻塞并带重置时间）。
#[test]
fn fix4_p2_github_last_page_with_zero_quota_is_complete_kept_and_nothing_more_is_sent() {
    let query = github::repository_query("example.test");
    for (case, total, second) in [("短末页", 45, 15), ("满末页", 60, 30)] {
        let page2 = api(
            200,
            &[("x-ratelimit-remaining", "0"), ("x-ratelimit-limit", "30"), ("x-ratelimit-reset", FIX4_RESET), ("x-ratelimit-resource", "search")],
            &repo_page(total, false, repos(1000, second)),
        );
        let fixture = fix4_two_pages(&query, total, page2);
        let mut session = GithubSession::default();
        let (outcome, used) = run_repositories(&fixture, "example.test", &mut session, 50);
        assert_eq!(used, 2, "{case}");
        assert_eq!(outcome.findings.len() as i64, total, "{case}: 末页结果保留");
        assert_eq!(outcome.status, Some(WorkItemStatus::Completed), "{case}: {}", outcome.reason);
        assert_eq!(outcome.source_status, Some(SourceStatus::Completed), "{case}: 完整结束不能标截断/失败：{}", outcome.reason);
        assert!(!outcome.reason.contains("未读尾部") && !outcome.reason.contains("rate_limited"), "{case}: {}", outcome.reason);
        let (next_item, used) = run_repositories(&fixture, "样本公司", &mut session, 50);
        assert_eq!(used, 0, "{case}: 末页已报额度 0，后续不再发请求");
        assert_eq!(fixture.request_count(), 2, "{case}: {:?}", fixture.requests());
        assert_eq!(next_item.status, Some(WorkItemStatus::BlockedQuota), "{case}: {}", next_item.reason);
        assert_eq!(next_item.source_status, Some(SourceStatus::RateLimited));
        assert!(next_item.reason.contains(&fix4_reset_text()), "{case}: 记录重置时间：{}", next_item.reason);
    }
}

/// FIX4 P2 GitHub：额度在末页之前耗尽（第 1 页 remaining=0 / 第 2 页 403 / 第 2 页 429）：
/// 已读页保留；标截断（未读尾部）、不绿；记录重置时间；不再发请求。
#[test]
fn fix4_p2_github_quota_gone_before_the_last_page_is_truncated_with_reset_and_kept() {
    let query = github::repository_query("example.test");
    let limited = |status: u16| {
        api(status, &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", FIX4_RESET)], r#"{"message":"API rate limit exceeded"}"#)
    };
    // a) 第 1 页成功但 remaining=0，Link 指向第 2 页。
    let port = fix4_port();
    let link = next_link(&format!("http://127.0.0.1:{port}"), &query, 2);
    let page1 = api(200, &[("Link", link.as_str()), ("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", FIX4_RESET)], &repo_page(45, false, repos(1, 30)));
    let early = Fixture::start_on(port, routes(vec![("/search/repositories", vec![page1, api(200, &[], &repo_page(45, false, repos(1000, 15)))])]));
    let cases: Vec<(&str, Fixture, i64)> = vec![
        ("第1页remaining=0", early, 1),
        ("第2页403", fix4_two_pages(&query, 45, limited(403)), 2),
        ("第2页429", fix4_two_pages(&query, 45, limited(429)), 2),
    ];
    for (case, fixture, sent) in cases {
        let mut session = GithubSession::default();
        let (outcome, used) = run_repositories(&fixture, "example.test", &mut session, 50);
        assert_eq!(used, sent, "{case}");
        assert_eq!(outcome.findings.len(), 30, "{case}: 已读页保留");
        assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota), "{case}");
        assert_eq!(outcome.source_status, Some(SourceStatus::RateLimited), "{case}: 不绿");
        assert!(outcome.reason.contains("未读尾部 15 条"), "{case}: 标截断：{}", outcome.reason);
        assert!(outcome.reason.contains(&fix4_reset_text()), "{case}: 记录重置：{}", outcome.reason);
        let (_, more) = run_repositories(&fixture, "样本公司", &mut session, 50);
        assert_eq!(more, 0, "{case}");
        assert_eq!(fixture.request_count() as i64, sent, "{case}: {:?}", fixture.requests());
    }
}

/// FIX4 P1／新增（T21）：第 1 页请求发出后（响应返回前）撤权。第 2 页零请求；第 1 页已合法取得的观察保留并带
/// 授权时间点标记；本项授权阻塞，来源 partial，覆盖不变绿。
#[test]
fn fix4_new_t21_revoking_before_page_two_keeps_page_one_marked_and_sends_no_page_two() {
    let site = site();
    let root = sandbox("fix4-s3");
    let (connection, db_path) = database(&root);
    let project_id = 593;
    let query = github::repository_query("example.test");
    let port = fix4_port();
    let link = next_link(&format!("http://127.0.0.1:{port}"), &query, 2);
    let page1 = api(200, &[("Link", link.as_str())], &repo_page(45, false, repos(1, 30)));
    let page2 = api(200, &[], &repo_page(45, false, repos(1000, 15)));
    let hook_db = db_path.clone();
    let mut fired = false;
    let index = Fixture::start_hooked_on(
        port,
        routes(vec![("/search/repositories", vec![page1, page2])]),
        Box::new(move |_path: &str| {
            if !fired {
                fired = true;
                let revoker = crate::db::open(&hook_db).unwrap();
                revoker
                    .execute("UPDATE asset_ownership_decisions SET exposure_eligible=0,authorization_status='revoked' WHERE project_id=?1", [project_id])
                    .unwrap();
            }
        }),
    );
    let settings = github_project(&connection, project_id, &site, &["github_repositories"], &[]);
    let started = start_or_reuse(&connection, project_id, &settings).expect("计划时仍有授权");
    run_worker(&db_path, started.run_id, project_id, &settings, fixture_endpoints(&index, CredentialLookup::production()));
    assert_eq!(index.request_count(), 1, "第 2 页零请求：{:?}", index.requests());
    let one = |sql: &str| -> i64 { connection.query_row(sql, [started.run_id], |row| row.get(0)).unwrap() };
    let kept = one("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND source_key='github_repositories' AND kind<>'github_search_page'");
    assert_eq!(kept, 30, "第 1 页观察保留");
    assert_eq!(
        one("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND source_key='github_repositories' AND json_extract(evidence_json,'$.detail.scopeGrant.verifiedAt') IS NOT NULL AND json_extract(evidence_json,'$.detail.scopeGrant.withdrawnDetectedAt') IS NOT NULL"),
        31,
        "每条保留观察（含页观察）都带授权时间点"
    );
    assert_eq!(one("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND requests=1 AND status='blocked_authorization' AND reason LIKE '%已保留%'"), 1);
    assert_eq!(one("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND status='completed' AND source_key='github_repositories'"), 0);
    let (status, _) = source_result(&connection, started.run_id, "github_repositories");
    assert_eq!(status, "partial", "覆盖矩阵不能变绿");
    // T21：保留的观察不得自动确认。
    assert_eq!(one("SELECT COUNT(*) FROM exposure_findings f JOIN exposure_finding_observations fo ON fo.finding_id=f.id WHERE fo.run_id=?1 AND f.status IN ('confirmed','verified')"), 0, "禁止自动确认");
    assert_eq!(one("SELECT COUNT(*) FROM exposure_incidents WHERE project_id=(SELECT project_id FROM exposure_runs WHERE id=?1) AND status='verified'"), 0, "禁止自动确认事件");
    let _ = std::fs::remove_dir_all(root);
}
