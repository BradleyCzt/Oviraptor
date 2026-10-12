// Phase 2 §11: the acceptance fixture. One local application carries every
// scenario the acceptance list names, and the deterministic mock model drives the
// real tools over real HTTP on 127.0.0.1 — no Strix, no Docker, and no fabricated
// tool results.
#[cfg(test)]
mod agent_acceptance {
    use super::agent_tests::{
        agent_harness, closing_ledger, findings_for, model_round,
        retarget_model, seed_session,
        fresh_single_production_harness, freeze_fresh_single_production_harness,
        execute_fresh_single_production_harness, assert_fresh_single_surface,
    };
    use super::*;
    use crate::agent_runtime::contract::{ScopeClass, ScopeSource};

    /// The scenario app. Every answer is a fixed function of the request line and
    /// the account cookie, so a run is reproducible and a difference is meaningful.
    fn acceptance_site(request: String) -> (u16, &'static str, String) {
        let line = request.lines().next().unwrap_or_default().to_string();
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("GET").to_ascii_uppercase();
        let target = parts.next().unwrap_or("/").to_string();
        let (path, query) = match target.split_once('?') {
            Some((head, tail)) => (head.to_string(), tail.to_string()),
            None => (target.clone(), String::new()),
        };
        let cookie = request
            .lines()
            .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
            .unwrap_or_default()
            .to_lowercase();
        let account = if cookie.contains("cookie-alpha") {
            "member-a"
        } else if cookie.contains("cookie-beta") {
            "member-b"
        } else if cookie.contains("cookie-gamma") {
            "admin"
        } else {
            "anonymous"
        };
        let member = matches!(account, "member-a" | "member-b");
        let json = |body: String| (200, "application/json", body);

        // 1 anonymous SPA shell; it never mentions the hidden endpoint (9).
        if matches!(path.as_str(), "/" | "/login" | "/orders") {
            return (
                200,
                "text/html",
                "<!DOCTYPE html><html><body><div id=app>订单</div><a href=/orders>订单</a>\
                 <script src=/static/app.js></script><img src=/img/logo.png></body></html>"
                    .to_string(),
            );
        }
        // 15 static asset and telemetry noise, both same-origin.
        if path == "/static/app.js" {
            return (
                200,
                "application/javascript",
                "fetch('/api/orders?page=1');fetch('/api/profile');\n//# sourceMappingURL=/static/app.js.map"
                    .to_string(),
            );
        }
        if path == "/img/logo.png" {
            return (200, "image/png", "PNG-bytes".to_string());
        }
        if path == "/telemetry/ping" && method == "POST" {
            return (204, "text/plain", String::new());
        }
        // 14 source map exposure, which is also where the hidden GET lives (9).
        if path == "/static/app.js.map" {
            return json(
                r#"{"version":3,"sources":["webpack:///src/api/internal/metrics.ts"],"file":"app.js"}"#
                    .to_string(),
            );
        }
        if path == "/api/internal/metrics" {
            return json(r#"{"build":"1.9.0","queueDepth":3}"#.to_string());
        }
        // 5 expired or missing session, 2/3 one account and two equal accounts.
        if path == "/api/orders" {
            return match account {
                "anonymous" => (
                    401,
                    "application/json",
                    r#"{"error":"unauthenticated","loginUrl":"/login"}"#.to_string(),
                ),
                "admin" => json(
                    r#"{"items":[{"id":1,"owner":"u-a"},{"id":2,"owner":"u-b"}],"roleId":"r-admin","permissions":["read","write"]}"#
                        .to_string(),
                ),
                who => json(format!(
                    r#"{{"items":[{{"id":1,"owner":"u-{}"}}],"roleId":"r-member"}}"#,
                    if who == "member-a" { "a" } else { "b" }
                )),
            };
        }
        // 12 horizontal privilege: another account's report reads, unknown 404s.
        if path == "/api/report" {
            let wanted = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("target="))
                .unwrap_or("")
                .to_string();
            return match (member, wanted.as_str()) {
                (true, "u-b") => json(
                    r#"{"target":"u-b","records":[{"id":9,"note":"另一个账号的订单"}]}"#.to_string(),
                ),
                (true, "u-a") => json(
                    r#"{"target":"u-a","records":[{"id":1,"note":"自己的订单"}]}"#.to_string(),
                ),
                (true, _) => (404, "application/json", r#"{"error":"not_found"}"#.to_string()),
                _ => (401, "application/json", r#"{"error":"unauthenticated"}"#.to_string()),
            };
        }
        // Both members see the identical shape; only the admin holds more (4).
        if path == "/api/profile" {
            return match account {
                "admin" => json(
                    r#"{"userId":"u-admin","roleId":"r-admin","role":"admin","canWrite":true,"permissions":["write"]}"#
                        .to_string(),
                ),
                "anonymous" => (
                    401,
                    "application/json",
                    r#"{"error":"unauthenticated"}"#.to_string(),
                ),
                _ => json(
                    r#"{"userId":"u-self","roleId":"r-member","role":"member","canWrite":false}"#
                        .to_string(),
                ),
            };
        }
        // 6 an ordinary authorization refusal, with no challenge anywhere in it.
        if path == "/api/admin/users" {
            return if account == "admin" {
                json(r#"{"users":["u-a","u-b"]}"#.to_string())
            } else {
                (
                    403,
                    "application/json",
                    r#"{"error":"forbidden","role":"member"}"#.to_string(),
                )
            };
        }
        let reflected = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("q="))
            .unwrap_or("");
        // 10 reflection that escapes, so it cannot execute.
        if path == "/ui" {
            return (
                200,
                "text/html",
                format!(
                    "<html><body>搜索：{}</body></html>",
                    reflected.replace('<', "&lt;").replace('>', "&gt;")
                ),
            );
        }
        // 11 reflection that executes: the confirmable case.
        if path == "/ui/raw" {
            return (
                200,
                "text/html",
                format!("<html><body>搜索：{reflected}</body></html>"),
            );
        }
        // 13 a server error that leaks a path and a live credential.
        if path == "/api/debug/config" {
            return (
                500,
                "text/html",
                "Traceback: /var/app/.env\napi_token: sk-live-abcdef1234567890".to_string(),
            );
        }
        // 7 a Web application firewall answer.
        if path == "/api/waf" {
            return (
                403,
                "text/html",
                "<html>Attention Required! | Cloudflare Ray ID: 7f3b</html>".to_string(),
            );
        }
        // 8 a rate limit that never lifts.
        if path == "/api/rate-limited" {
            return (
                429,
                "text/plain",
                "Too Many Requests: retry after 60".to_string(),
            );
        }
        (404, "text/plain", "not found".to_string())
    }

    fn ask(method: &str, target: &str, cookie: &str) -> (u16, &'static str, String) {
        acceptance_site(format!(
            "{method} {target} HTTP/1.1\r\nhost: fixture.invalid\r\ncookie: {cookie}\r\n\r\n"
        ))
    }

    /// §11: the fixture really carries the fifteen named scenarios; an assertion
    /// about the agent is worthless if the app under it does not answer.
    #[test]
    fn acceptance_fixture_answers_every_named_scenario() {
        let (status, _, body) = ask("GET", "/", "");
        assert_eq!(status, 200);
        assert!(body.contains("id=app"), "1. an anonymous SPA shell");
        assert!(
            !body.contains("/api/internal/metrics"),
            "9. the hidden endpoint is never in the DOM"
        );
        assert_eq!(ask("GET", "/orders", "").0, 200, "1. SPA route");
        assert_eq!(
            ask("GET", "/api/profile", "cookie-alpha").0,
            200,
            "2. one account"
        );
        let alpha = ask("GET", "/api/profile", "cookie-alpha").2;
        let beta = ask("GET", "/api/profile", "cookie-beta").2;
        assert_eq!(alpha, beta, "3. two equal accounts answer alike");
        let admin = ask("GET", "/api/profile", "cookie-gamma").2;
        assert_ne!(alpha, admin, "4. a privileged account answers differently");
        assert!(admin.contains("permissions"));
        assert_eq!(
            ask("GET", "/api/orders", "").0,
            401,
            "5. an expired session is a plain 401 with a login URL"
        );
        assert_eq!(
            ask("GET", "/api/admin/users", "cookie-alpha").0,
            403,
            "6. ordinary authorization refusal"
        );
        assert_eq!(
            ask("GET", "/api/waf", "").0,
            403,
            "7. the challenge is reachable"
        );
        assert_eq!(ask("GET", "/api/rate-limited", "").0, 429, "8. rate limit");
        assert_eq!(
            ask("GET", "/api/internal/metrics", "").0,
            200,
            "9. hidden GET interface"
        );
        let escaped = ask("GET", "/ui?q=<svg>", "").2;
        assert!(
            escaped.contains("&lt;svg&gt;") && !escaped.contains("<svg>"),
            "10. reflected but escaped, so it cannot execute: {escaped}"
        );
        let raw = ask("GET", "/ui/raw?q=<svg>", "").2;
        assert!(
            raw.contains("<svg>"),
            "11. reflection that executes: {raw}"
        );
        assert!(
            ask("GET", "/api/report?target=u-b", "cookie-alpha")
                .2
                .contains("另一个账号的订单"),
            "12. horizontal privilege, positive half"
        );
        assert_eq!(
            ask("GET", "/api/report?target=u-zz", "cookie-alpha").0,
            404,
            "12. horizontal privilege, negative half"
        );
        assert!(
            ask("GET", "/api/debug/config", "")
                .2
                .contains("/var/app/.env"),
            "13. error message leak"
        );
        assert!(
            ask("GET", "/static/app.js.map", "")
                .2
                .contains("internal/metrics"),
            "14. source map exposure"
        );
        assert_eq!(ask("POST", "/telemetry/ping", "").0, 204, "15. telemetry");
        assert_eq!(ask("GET", "/img/logo.png", "").0, 200, "15. static asset");
    }

    fn class_of(context: &AgentRunContext, url: &str, resource_type: &str) -> ScopeClass {
        agent_scope_assess(
            context,
            url,
            "GET",
            resource_type,
            ScopeSource::BrowserObserved,
        )
        .class
    }

    /// §11: telemetry and static assets must never be treated as the formal API
    /// surface, and the hidden endpoint must not be sent at all once observed.
    #[test]
    fn acceptance_fixture_keeps_noise_out_of_the_business_api_surface() {
        let harness = agent_harness(
            "accept-noise",
            acceptance_site,
            vec![AgentIdentity::anonymous()],
        );
        let base = harness.context.target_url.clone();
        assert_eq!(
            class_of(&harness.context, &format!("{base}/api/orders"), "fetch"),
            ScopeClass::AuthorizedBusinessApi
        );
        for (path, kind) in [("/static/app.js", "script"), ("/img/logo.png", "image")] {
            assert_ne!(
                class_of(&harness.context, &format!("{base}{path}"), kind),
                ScopeClass::AuthorizedBusinessApi,
                "{path} is a browser asset, never part of the formal API surface"
            );
        }
        assert_eq!(
            class_of(&harness.context, &format!("{base}/telemetry/ping"), "fetch"),
            ScopeClass::TelemetryOrNoise
        );
        // Telemetry is screened before scope, so it can never be re-labelled as a
        // surface this run is allowed to spend a request on.
        assert_eq!(
            class_of(
                &harness.context,
                "https://telemetry.vendor.invalid/ping",
                "script"
            ),
            ScopeClass::TelemetryOrNoise
        );
        for external in [
            "https://api.other-vendor.invalid/v1/orders",
            "https://cdn.assets.invalid/app.js",
        ] {
            let class = class_of(&harness.context, external, "fetch");
            assert!(
                !matches!(
                    class,
                    ScopeClass::AuthorizedBusinessApi | ScopeClass::AuthorizedStatic
                ),
                "a third-party origin is never target surface: {external} -> {class:?}"
            );
        }
    }

    /// §11: only a confirmed challenge fuses the target, and it stops immediately.
    #[test]
    fn acceptance_fixture_waf_challenge_fuses_the_target() {
        // Terminal projection needs the original Single Root (creator/startup,
        // frozen plan, financial control); the legacy harness had no control,
        // so the terminal consumer refused it (budget history unverifiable).
        let mut harness = fresh_single_production_harness("accept-waf", acceptance_site);
        freeze_fresh_single_production_harness(&mut harness);
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/waf")}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/orders")}),
                    )],
                    900,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        let AgentTargetOutcome::Limited(stop) = outcome else {
            panic!("a WAF challenge must fuse, got {:?}", outcome.detail());
        };
        assert_eq!(stop.code, AGENT_STOP_WAF);
        assert!(stop.requires_fuse());
        let mut tally = AgentPipelineTally::default();
        record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally,
        );
        assert_fresh_single_surface(&harness, &url, "protected_stop", 1, 0);
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            1,
            "the run must not probe the site after the challenge"
        );
    }

    /// §11: a plain 401/403 is a boundary, not a block. The run keeps going and the
    /// target is never moved into the fuse zone.
    #[test]
    fn acceptance_fixture_plain_403_and_expired_login_do_not_fuse() {
        let mut harness = agent_harness(
            "accept-403",
            acceptance_site,
            vec![AgentIdentity::anonymous(), AgentIdentity::scoped("session-a")],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "");
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"session-a","method":"GET","url":format!("{url}/api/admin/users"),"family":"authentication_session"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/orders"),"family":"authentication_session"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["authentication_session"]), "stopReason":"收口"}),
                    )],
                    900,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Limited(_) | AgentTargetOutcome::Failed(_)),
            "boundary answers must not end the run: {:?}",
            outcome.detail()
        );
        assert_eq!(harness.site_seen.lock().unwrap().len(), 2);
        let connection = db::open(&harness.db_path).unwrap();
        let fused: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=9001 AND archived=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fused, 0, "a 403 is not a protection signal");
    }

    /// §11: a rate limit that never lifts is its own protection stop, distinct from
    /// a challenge and distinct from an authorization boundary.
    #[test]
    fn acceptance_fixture_persistent_rate_limit_stops_as_a_rate_limit() {
        let mut harness = agent_harness(
            "accept-429",
            acceptance_site,
            vec![AgentIdentity::anonymous()],
        );
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/rate-limited")}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/rate-limited")}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/orders")}),
                    )],
                    900,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        let AgentTargetOutcome::Limited(stop) = &outcome else {
            panic!("persistent 429 must stop the target, got {:?}", outcome.detail());
        };
        assert_eq!(stop.code, AGENT_STOP_RATE_LIMIT);
        assert!(stop.requires_fuse());
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            2,
            "one throttle is postponed, the second one stops the run"
        );
    }

    /// §11: neither equal accounts nor a personalized privileged profile prove
    /// cross-object authorization. A role contrast is only a candidate until
    /// an independently bound owner/object control group is available.
    #[test]
    fn acceptance_fixture_profile_contrasts_do_not_confirm_authorization() {
        let harness = agent_harness(
            "accept-pair",
            acceptance_site,
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
                AgentIdentity::scoped("session-c"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "");
        seed_session(&harness.db_path, "session-c", "cookie-gamma", "");
        let mut runtime = AgentToolRuntime::default();
        let compare = |runtime: &mut AgentToolRuntime, right: &str| {
            agent_execute_tool(
                &harness.context,
                runtime,
                "compare_identities",
                &serde_json::json!({
                    "leftIdentity":"session-a","rightIdentity":right,
                    "method":"GET","path":"/api/profile","family":"authorization",
                    "contractKey":"idor|/api/profile"
                }),
            )
            .model_view
        };
        let equal = compare(&mut runtime, "session-b");
        assert_eq!(
            equal["materialDifference"],
            serde_json::json!(false),
            "two equal accounts differ in nothing that matters: {equal}"
        );
        let privileged = compare(&mut runtime, "session-c");
        assert_eq!(
            privileged["materialDifference"],
            serde_json::json!(true),
            "the admin account holds authorization fields the member does not: {privileged}"
        );
        let control = value_first(&privileged["left"], &["requestId"]);
        let test = value_first(&privileged["right"], &["requestId"]);
        let artifact = value_first(&privileged, &["responseDifferenceArtifactId"]);
        let refused = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-equal","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/profile","title":"平权账号差异","severity":"medium",
                "confidence":0.7,
                "controlRequestId":value_first(&equal["left"], &["requestId"]),
                "testRequestId":value_first(&equal["right"], &["requestId"]),
                "responseDifferenceArtifactId":value_first(&equal, &["responseDifferenceArtifactId"]),
                "impact":"无","reproductionSteps":"两个平权账号重放"
            }),
        )
        .model_view;
        assert_ne!(
            refused["status"].as_str(),
            Some("confirmed"),
            "an equal pair may never be confirmed: {refused}"
        );
        let proposed = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-admin","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/profile","title":"成员账号取得管理员权限字段","severity":"medium",
                "confidence":0.85,
                "controlRequestId":control,"testRequestId":test,"responseDifferenceArtifactId":artifact,
                "impact":"低权账号可见高权配置","reproductionSteps":"用 session-c 重放同一请求"
            }),
        )
        .model_view;
        assert_eq!(proposed["status"], "insufficient_evidence", "{proposed}");
        assert_eq!(proposed["missingEvidence"][0], "authorization_object_control_missing");
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0
        );
    }

    /// §11: a fresh rerun throws away working state, preserves the A/B evidence,
    /// and does not turn an unproven profile contrast into a historical finding.
    #[test]
    fn acceptance_fixture_fresh_rerun_clears_working_state_and_keeps_the_record() {
        let harness = agent_harness(
            "accept-fresh",
            acceptance_site,
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-c"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "");
        seed_session(&harness.db_path, "session-c", "cookie-gamma", "");
        let url = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let pair = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"session-c",
                "method":"GET","path":"/api/profile","family":"authorization",
                "contractKey":"idor|/api/profile"
            }),
        )
        .model_view;
        agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-admin","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/profile","title":"成员账号取得管理员权限字段","severity":"medium",
                "confidence":0.85,
                "controlRequestId":value_first(&pair["left"], &["requestId"]),
                "testRequestId":value_first(&pair["right"], &["requestId"]),
                "responseDifferenceArtifactId":value_first(&pair, &["responseDifferenceArtifactId"]),
                "impact":"低权账号可见高权配置","reproductionSteps":"用 session-c 重放同一请求"
            }),
        );
        let evidence_before = findings_for(&harness.db_path, AGENT_EVIDENCE_STAGE).len();
        assert!(evidence_before >= 1, "the executed pair left evidence rows");
        // What a finished attempt leaves behind, and what a fresh one must drop.
        let mut working = NativeAgentState::fresh(
            harness.context.attempt_number,
            AgentBackendKind::Native,
            "evidence-hash",
            "plan-hash",
            vec!["family:authorization".to_string()],
        );
        working.target_requests = 2;
        working.terminal_reason = "已收口".to_string();
        working.persist(&harness.db_path, "agent-scan", &url).unwrap();
        assert!(
            NativeAgentState::read(&harness.db_path, "agent-scan", &url).is_some(),
            "the attempt has a working state before the reset"
        );
        NativeAgentState::clear(&harness.db_path, "agent-scan", &url).unwrap();
        assert!(
            NativeAgentState::read(&harness.db_path, "agent-scan", &url).is_none(),
            "a fresh attempt starts with no queue, no budget and no terminal reason"
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "an unproven profile contrast remains unconfirmed after reset"
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_EVIDENCE_STAGE).len(),
            evidence_before,
            "the executed requests survive the reset"
        );
        let raw = fs::read_dir(harness.root.join("target").join("agent-http"))
            .map(|entries| entries.count())
            .unwrap_or_default();
        assert!(raw > 0, "the raw HTTP artifacts stay on disk");
    }

    /// §11 item 13 with §6: a leaked credential in a server error is evidence, not
    /// something the model history may carry.
    #[test]
    fn acceptance_fixture_leaked_credential_never_reaches_the_model() {
        let mut harness = agent_harness(
            "accept-leak",
            acceptance_site,
            vec![AgentIdentity::anonymous()],
        );
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/debug/config"),"family":"information_disclosure"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["information_disclosure"]), "stopReason":"收口"}),
                    )],
                    900,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "{:?}",
            outcome.detail()
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert!(
            !prompts.contains("sk-live-abcdef1234567890"),
            "a leaked credential must not be replayed into the model context"
        );
        assert!(
            prompts.contains("api_token") || prompts.contains("/var/app/.env"),
            "the disclosure itself still has to reach the model as evidence"
        );
    }

    /// §10: a bounded close-out is finished work *with* holes, so it may never be
    /// counted or worded as a clean, uninterrupted finish.
    #[test]
    fn bounded_completions_never_read_as_a_clean_full_finish() {
        let mut harness = fresh_single_production_harness("accept-bounded", acceptance_site);
        freeze_fresh_single_production_harness(&mut harness);
        let mut tally = AgentPipelineTally::default();
        assert!(record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            AgentTargetOutcome::bounded_completed("预算内收口，仍有未覆盖族"),
            &mut tally
        ));
        assert_eq!(tally.completed_with_gaps, 1);
        assert_eq!(tally.completed, 0, "a gapped close-out is not a clean completion");
        assert_eq!(tally.counted(), 1, "it is still one answered target");
        tally.finalize(&harness.db_path, "agent-scan", 1);
        let connection = db::open(&harness.db_path).unwrap();
        let (status, checkpoint): (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='agent-scan'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed_with_gaps", "finished work must retain its coverage gaps in the machine-readable state");
        assert!(checkpoint.contains("带覆盖缺口完成 1"), "{checkpoint}");
        assert!(
            !checkpoint.contains("无异常中断"),
            "a gapped finish must not claim a clean run: {checkpoint}"
        );
    }

    /// §10: the recomputed scan summary keeps "finished with gaps" apart from a
    /// clean auto-verification, so the task list cannot show full coverage that
    /// never happened.
    #[test]
    fn native_run_is_never_described_as_a_strix_scan() {
        let mut harness = agent_harness(
            "accept-naming",
            acceptance_site,
            vec![AgentIdentity::anonymous()],
        );
        retarget_model(
            &mut harness,
            vec![model_round(
                &[(
                    "finish_target",
                    serde_json::json!({"coverage": closing_ledger(&["business_flow"]), "stopReason":"收口"}),
                )],
                900,
            )],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        let connection = db::open(&harness.db_path).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT COALESCE(routing_reason,'') || ' ' || COALESCE(scan_mode,'') FROM sentinel_targets WHERE scan_id='agent-scan'",
            )
            .unwrap();
        let stored: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        let checkpoint: String = connection
            .query_row(
                "SELECT current_checkpoint FROM sentinel_scans WHERE id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for text in stored.iter().chain(std::iter::once(&checkpoint)) {
            assert!(
                !text.contains("Strix"),
                "the native result surface still names the compatibility backend: {text}"
            );
        }
        assert!(!checkpoint.is_empty());
    }
}
