// §Q2 — the authorization and egress contract, proved against a real localhost socket.
// "Refused before the send" is measured by what the fixture received, not asserted from a
// mock that never touches a network stack.

use super::adapters::http::{PassiveClient, PassiveHttpLimits, RequestMeter};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::redaction::{
    credential_sightings, escape_csv_cell, evidence_excerpt, mask_value, redact_json, redact_text,
    CredentialGrade,
};
use super::scope::{ExposureDenial, ExposureScope};
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-q2-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    crate::db::open(&db_path).expect("临时库连接")
}

fn load_scope(connection: &Connection, project: i64, allow_private: bool) -> ExposureScope {
    ExposureScope::load(
        connection,
        project,
        &json!({"exposureAllowPrivateDestinations": allow_private}),
    )
    .expect("范围必须能装载")
}

fn client(max_hops: usize, cap: usize) -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        max_hops,
        max_response_bytes: cap,
        timeout: std::time::Duration::from_secs(5),
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn meter(max_requests: i64, max_bytes: i64) -> RequestMeter {
    RequestMeter {
        requests_used: 0,
        bytes_used: 0,
        max_requests,
        max_bytes,
    }
}

fn no_recheck() -> Result<(), ExposureDenial> {
    Ok(())
}

fn never_cancelled() -> bool {
    false
}

#[test]
fn q2_grants_are_origin_exact_and_every_widening_attempt_is_refused() {
    let root = sandbox("grants");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/ok",
        vec![response(200, &[("Content-Type", "text/html")], "public")],
    )]));
    seed_scope(&connection, 11, &format!("{}/", fixture.origin), true);
    seed_scope(&connection, 11, "https://other.invalid/", true);
    let scope = load_scope(&connection, 11, true);
    assert_eq!(scope.grants.len(), 2);
    assert!(scope.coarse_legacy, "现有 confirmed 行只能算粗粒度遗留范围");

    let allowed = reqwest::Url::parse(&fixture.url("/ok")).unwrap();
    assert!(scope.authorize(&allowed, "2026-10-07 12:00:00").is_ok());
    // Another port on the same host is a different origin.
    let other_port =
        reqwest::Url::parse(&format!("http://127.0.0.1:{}/", fixture.port + 1)).unwrap();
    assert_eq!(
        scope
            .authorize(&other_port, "2026-10-07 12:00:00")
            .unwrap_err()
            .code,
        "host_not_granted"
    );
    // A sibling name resolving to the same machine is not the approved host either.
    let sibling = reqwest::Url::parse(&format!("http://localhost:{}/ok", fixture.port)).unwrap();
    assert_eq!(
        scope
            .authorize(&sibling, "2026-10-07 12:00:00")
            .unwrap_err()
            .code,
        "host_not_granted"
    );
    let userinfo =
        reqwest::Url::parse(&format!("http://admin:pw@127.0.0.1:{}/ok", fixture.port)).unwrap();
    assert_eq!(
        scope
            .authorize(&userinfo, "2026-10-07 12:00:00")
            .unwrap_err()
            .code,
        "userinfo_denied"
    );
    let file = reqwest::Url::parse("file:///etc/passwd").unwrap();
    assert_eq!(
        scope
            .authorize(&file, "2026-10-07 12:00:00")
            .unwrap_err()
            .code,
        "scheme_denied"
    );

    // Without the explicit internal-destination switch the same URL is refused.
    let strict = load_scope(&connection, 11, false);
    let denial = strict
        .authorize(&allowed, "2026-10-07 12:00:00")
        .unwrap_err();
    assert_eq!(denial.code, "private_destination_denied");
    assert!(denial.message.contains("显式配置"), "{}", denial.message);

    // An expired window stops the next request, not the next run.
    let mut expired = scope.clone();
    expired.valid_until = Some("2020-01-01 00:00:00".to_string());
    assert_eq!(
        expired
            .authorize(&allowed, "2026-10-07 12:00:00")
            .unwrap_err()
            .code,
        "scope_expired"
    );
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_redirects_are_authorized_hop_by_hop_and_stay_inside_the_origin() {
    let root = sandbox("redirect");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![
        (
            "/chain",
            vec![
                response(302, &[("Location", "/deep?x=1")], ""),
                response(200, &[("Content-Type", "text/html")], "end"),
            ],
        ),
        (
            "/deep",
            vec![response(200, &[("Content-Type", "text/html")], "end")],
        ),
        (
            "/escape",
            vec![
                response(
                    302,
                    &[("Location", &format!("http://127.0.0.1:{}/ok", u16::MAX))],
                    "",
                ),
                response(200, &[], "never"),
            ],
        ),
        (
            "/downgrade",
            vec![
                response(302, &[("Location", "ftp://127.0.0.1/pub")], ""),
                response(200, &[], "never"),
            ],
        ),
        ("/loop", vec![response(302, &[("Location", "/loop")], "")]),
    ]));
    seed_scope(&connection, 12, &format!("{}/", fixture.origin), true);
    let scope = load_scope(&connection, 12, true);
    let fetcher = client(3, 4096);
    let mut budget = meter(20, 1_000_000);

    let walked = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/chain")).unwrap(),
            &mut budget,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap();
    assert_eq!(walked.status, 200);
    assert!(
        walked.final_url.contains("/deep?x=1"),
        "{}",
        walked.final_url
    );
    assert!(walked.hops.iter().any(|row| row.contains("/chain")));
    assert_eq!(fixture.request_count(), 2, "两跳都要走同一 origin");

    let escaped = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/escape")).unwrap(),
            &mut budget,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap_err();
    assert_eq!(
        escaped.code(),
        "redirect_cross_origin",
        "{}",
        escaped.message()
    );
    assert!(
        escaped.message().contains("已授权 origin"),
        "{}",
        escaped.message()
    );

    let downgraded = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/downgrade")).unwrap(),
            &mut budget,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap_err();
    // 非 http(s) 的跳转目标无法成为被批准的 origin，跟随必须停止。
    assert_eq!(
        downgraded.code(),
        "redirect_unparseable",
        "{}",
        downgraded.message()
    );

    // A redirect loop stops at the hop ceiling instead of chasing forever.
    let looping = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/loop")).unwrap(),
            &mut budget,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap_err();
    assert_eq!(looping.code(), "redirect_hop_limit");
    // chain 2 hops + escape 1 + downgrade 1 + loop 4 (hop ceiling 3 means 4 sends).
    assert_eq!(
        budget.requests_used, 8,
        "每一跳都要计费：{}",
        budget.requests_used
    );
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_budget_is_charged_before_the_socket_and_cancel_leaves_zero_new_requests() {
    let root = sandbox("budget");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![
        (
            "/ok",
            vec![response(200, &[("Content-Type", "text/plain")], "small")],
        ),
        (
            "/big",
            vec![response(
                200,
                &[("Content-Type", "text/plain")],
                &"x".repeat(5_000),
            )],
        ),
        (
            "/gone",
            vec![response(404, &[("Content-Type", "text/plain")], "nope")],
        ),
    ]));
    seed_scope(&connection, 13, &format!("{}/", fixture.origin), true);
    let scope = load_scope(&connection, 13, true);
    let fetcher = client(3, 1_000);

    // A failed request still costs: the plan forbids counting successes only.
    let mut charged = meter(10, 10_000);
    let missing = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/missing-page")).unwrap(),
            &mut charged,
            &no_recheck,
            &never_cancelled,
        )
        .expect("404 是一次真实尝试，不是拒绝");
    assert_eq!(missing.status, 404, "边界响应必须原样带回");
    assert_eq!(charged.requests_used, 1);

    // Per-response ceiling: truncated, and the kept bytes are what is charged.
    let big = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/big")).unwrap(),
            &mut charged,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap();
    assert!(big.truncated);
    assert_eq!(big.body.len(), 1_000);
    assert_eq!(
        charged.bytes_used,
        1_000 + missing.body.len() as i64,
        "只按保留字节计费"
    );
    assert_eq!(charged.requests_used, 2);

    // Exhausted request budget: the refusal happens before the send.
    let mut tiny = meter(1, 10_000);
    fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/ok")).unwrap(),
            &mut tiny,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap();
    let before = fixture.request_count();
    let blocked = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/ok")).unwrap(),
            &mut tiny,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap_err();
    assert_eq!(blocked.code(), "request_budget_exhausted");
    assert_eq!(fixture.request_count(), before, "超额后不得再出站");

    // Byte budget.
    let mut tight_bytes = meter(50, 10);
    let denied = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/big")).unwrap(),
            &mut tight_bytes,
            &no_recheck,
            &never_cancelled,
        )
        .unwrap_err();
    assert_eq!(denied.code(), "byte_budget_exhausted");

    // Cancelled: zero new requests, and the refusal is explicit.
    let settled = fixture.request_count();
    let cancelled = fetcher
        .fetch(
            &scope,
            &reqwest::Url::parse(&fixture.url("/ok")).unwrap(),
            &mut meter(50, 1_000_000),
            &no_recheck,
            &|| true,
        )
        .unwrap_err();
    assert_eq!(cancelled.code(), "cancelled");
    assert_eq!(fixture.request_count(), settled);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_revocation_and_project_removal_stop_the_next_request_not_the_run() {
    let root = sandbox("revoke");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/ok",
        vec![response(200, &[("Content-Type", "text/html")], "page")],
    )]));
    seed_scope(&connection, 14, &format!("{}/", fixture.origin), true);
    let scope = load_scope(&connection, 14, true);
    let fetcher = client(3, 4096);
    let revoked = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&revoked);
    let recheck = move || {
        if flag.load(Ordering::Relaxed) {
            return Err(ExposureDenial::new("authorization_revoked", "授权已撤销"));
        }
        Ok(())
    };
    let url = reqwest::Url::parse(&fixture.url("/ok")).unwrap();
    let mut budget = meter(10, 100_000);
    assert!(fetcher
        .fetch(&scope, &url, &mut budget, &recheck, &never_cancelled)
        .is_ok());
    let before = fixture.request_count();
    revoked.store(true, Ordering::Relaxed);
    let denial = fetcher
        .fetch(&scope, &url, &mut budget, &recheck, &never_cancelled)
        .unwrap_err();
    assert_eq!(denial.code(), "authorization_revoked");
    assert_eq!(fixture.request_count(), before, "撤销后的新请求必须为零");

    // The run's own meter is untouched by a denial: the budget is for outbound attempts.
    assert_eq!(budget.requests_used, 1);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_boundaries_are_recorded_as_boundaries_and_never_as_safe() {
    let root = sandbox("boundary");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![
        (
            "/401",
            vec![response(
                401,
                &[("Content-Type", "text/plain")],
                "login required",
            )],
        ),
        (
            "/403",
            vec![response(
                403,
                &[("Content-Type", "text/plain")],
                "forbidden",
            )],
        ),
        (
            "/429",
            vec![response(429, &[("Retry-After", "30")], "slow down")],
        ),
        (
            "/challenge",
            vec![response(
                200,
                &[("Content-Type", "text/html")],
                "<title>Just a moment… Captcha check</title>",
            )],
        ),
    ]));
    seed_scope(&connection, 15, &format!("{}/", fixture.origin), true);
    let scope = load_scope(&connection, 15, true);
    let fetcher = client(3, 4096);
    let mut budget = meter(20, 100_000);
    let cases = [
        ("/401", "permission"),
        ("/403", "permission"),
        ("/429", "rate_limited"),
        ("/challenge", "challenge"),
    ];
    for (path, expected) in cases {
        let answer = fetcher
            .fetch(
                &scope,
                &reqwest::Url::parse(&fixture.url(path)).unwrap(),
                &mut budget,
                &no_recheck,
                &never_cancelled,
            )
            .unwrap();
        assert_eq!(answer.boundary, expected, "{path} 的边界分类错了");
        assert!(!answer.status.to_string().is_empty());
    }
    assert_eq!(budget.requests_used, 4, "边界响应同样计费");
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_source_endpoints_are_not_authorization_for_newly_found_targets() {
    let root = sandbox("endpoint");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/ct",
        vec![response(200, &[("Content-Type", "application/json")], "[]")],
    )]));
    seed_scope(&connection, 16, &format!("{}/", fixture.origin), true);
    let scope = load_scope(&connection, 16, true);
    let fetcher = client(3, 4096);
    let mut budget = meter(10, 100_000);
    let listed = vec![fixture.origin.clone()];
    let ok = fetcher
        .fetch_source_endpoint(
            &reqwest::Url::parse(&fixture.url("/ct")).unwrap(),
            &listed,
            &mut budget,
            true,
            &never_cancelled,
        )
        .is_ok();
    assert!(ok, "配置的公共索引端点必须可达");
    let before = fixture.request_count();
    let unlisted = fetcher
        .fetch_source_endpoint(
            &reqwest::Url::parse("https://crt.sh/?q=other").unwrap(),
            &listed,
            &mut budget,
            false,
            &never_cancelled,
        )
        .unwrap_err();
    assert_eq!(unlisted.code(), "source_endpoint_not_configured");
    assert_eq!(fixture.request_count(), before);
    // Reading an index never widens the target grants.
    assert_eq!(scope.grants.len(), 1, "索引端点不能变成新的目标授权");
    assert_eq!(load_scope(&connection, 16, true).grants.len(), 1);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q2_redaction_is_one_authority_for_evidence_headers_exports_and_csv() {
    let raw = "client_secret = abcdefgh1234567890\nAuthorization: Bearer sk-live-9f8e7d6c5b4a\nurl: https://user:hunter2pass@internal.invalid/x";
    let redacted = redact_text(raw);
    assert!(!redacted.contains("abcdefgh1234567890"), "{redacted}");
    assert!(!redacted.contains("sk-live-9f8e7d6c5b4a"), "{redacted}");
    assert!(!redacted.contains("hunter2pass"), "{redacted}");
    assert!(redacted.contains("abc***90"), "{redacted}");

    assert_eq!(mask_value("short"), "***");
    assert_eq!(mask_value("abcdefghijkl"), "abc***kl(len=12)");

    let rows = credential_sightings(
        "api_key = YOUR_API_KEY_1234\napi_key = ZZZZZZZZZZZZZZZ1\nproxy-authorization: Basic Zm9vOmJhcg==",
        50,
    );
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        rows[0].grade,
        CredentialGrade::Placeholder,
        "模板值不能算候选"
    );
    assert_eq!(rows[1].grade, CredentialGrade::Candidate);
    assert!(!rows[1].masked.contains("ZZZZZZ"));

    // A header map cannot smuggle a cookie or a token into stored evidence.
    let headers = json!({"set-cookie": "session=abcdefghijklmnop", "x-trace": "ok"});
    let cleaned = redact_json(&headers);
    assert_eq!(cleaned["x-trace"], json!("ok"));
    assert!(
        !cleaned["set-cookie"]
            .as_str()
            .unwrap_or_default()
            .contains("abcdefghijklmnop"),
        "{cleaned}"
    );

    assert_eq!(
        escape_csv_cell("=HYPERLINK(\"http://x\")"),
        "'=HYPERLINK(\"http://x\")"
    );
    assert_eq!(escape_csv_cell("+1"), "'+1");
    assert_eq!(escape_csv_cell("普通文本"), "普通文本");

    let long = format!("{}{}", "y".repeat(300), " token=abcdefghijklmnop");
    let excerpt = evidence_excerpt(&long, 120);
    assert!(excerpt.chars().count() <= 120 + "…[已截断]".chars().count());
    assert!(excerpt.contains("已截断"), "{excerpt}");
    assert!(!excerpt.contains("abcdefghijklmnop"), "{excerpt}");
}

#[test]
fn q2_exclusions_name_why_an_asset_is_outside_the_run() {
    let root = sandbox("exclusions");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(17,'范围','active')",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO assets(id,asset_key,link,host) VALUES(1,'a1','https://one.invalid/','one.invalid')", [])
        .unwrap();
    connection
        .execute("INSERT INTO assets(id,asset_key,link,host) VALUES(2,'a2','https://two.invalid/','two.invalid')", [])
        .unwrap();
    connection
        .execute("INSERT INTO assets(id,asset_key,link,host) VALUES(3,'a3','https://three.invalid/','three.invalid')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO assets(id,asset_key,link) VALUES(4,'a4','ftp://four.invalid')",
            [],
        )
        .unwrap();
    // eligible + member
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(17,1)",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,exposure_eligible,status) VALUES(17,1,1,'confirmed')", [])
        .unwrap();
    // eligible but removed from the project
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(17,2,1)",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,exposure_eligible) VALUES(17,2,1)", [])
        .unwrap();
    // member but not eligible, and explicitly revoked
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(17,3)",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,exposure_eligible,authorization_status) VALUES(17,3,0,'revoked')", [])
        .unwrap();
    // non-http origin
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(17,4)",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,exposure_eligible) VALUES(17,4,1)", [])
        .unwrap();
    let scope = load_scope(&connection, 17, false);
    assert_eq!(scope.grants.len(), 1);
    let reasons: Vec<String> = scope
        .excluded
        .iter()
        .map(|row| format!("{}:{}", row.asset_id, row.reason))
        .collect();
    assert!(
        reasons.contains(&"2:asset_removed_from_project".to_string()),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&"3:authorization_revoked".to_string()),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&"4:non_http_origin".to_string()),
        "{reasons:?}"
    );
    // An archived project has no scope at all.
    connection
        .execute("UPDATE projects SET status='archived' WHERE id=17", [])
        .unwrap();
    let error = ExposureScope::load(&connection, 17, &json!({})).unwrap_err();
    assert_eq!(error.code, "project_inactive");
    let _ = std::fs::remove_dir_all(root);
}
