use super::*;

fn read_fixture_request(stream: &mut TcpStream) -> Option<()> {
    use std::io::Read;
    // Accepted sockets may inherit NONBLOCK on macOS. Read all headers before
    // replying so an unread request tail cannot turn the fixture into a reset.
    stream.set_nonblocking(false).ok()?;
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    while request.len() < 65_536 {
        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(length) => request.extend_from_slice(&buffer[..length]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(());
        }
    }
    None
}

fn row(label: &str, kind: &str, url: &str, evidence: &str) -> Record {
    [
        ("company", label),
        ("discovery_seed_kind", kind),
        ("link", url),
        ("asset_key", label),
        ("protocol", "http"),
        ("evidence", evidence),
        ("score", "45"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

#[test]
fn company_and_domain_hits_merge_before_refinement_without_claiming_ownership() {
    let rows = merge_discovery_rows(vec![
        row(
            "example.com",
            "domain",
            "http://EXAMPLE.com:80/",
            "精确主机:example.com",
        ),
        row(
            "集团有限公司",
            "company",
            "http://example.com",
            "标题全称:集团有限公司",
        ),
        row(
            "example.com",
            "domain",
            "http://example.com/#a",
            "证书域:example.com",
        ),
    ]);
    assert_eq!(rows.len(), 1);
    assert_eq!(text(&rows[0], &["company"]), "集团有限公司");
    assert_eq!(text(&rows[0], &["discovery_source_count"]), "2");
    assert_eq!(
        text(&rows[0], &["score"]),
        "45",
        "duplicate evidence must not inflate confidence"
    );
    assert!(!rows[0].contains_key("ownership_status"));
    let sources = discovery_sources(&rows[0]);
    assert_eq!(sources.len(), 3);
    assert!(sources
        .iter()
        .any(|source| source["label"] == "集团有限公司"));
    assert!(text(&rows[0], &["evidence"]).contains("证书域:example.com"));
    assert_eq!(
        merge_discovery_rows(rows.clone()),
        rows,
        "merge must be idempotent"
    );
}

#[test]
fn distinct_protocol_port_host_and_case_sensitive_paths_are_not_collapsed() {
    let urls = [
        "http://example.com/",
        "https://example.com/",
        "http://www.example.com/",
        "http://example.com:8080/",
        "http://example.com/A",
        "http://example.com/a",
        "http://example.com/a/",
        "http://example.com/?q=1",
        "http://example.com/?q=2",
    ];
    assert_eq!(
        merge_discovery_rows(
            urls.iter()
                .map(|url| row("集团", "company", url, "标题全称:集团"))
                .collect()
        )
        .len(),
        urls.len()
    );
}

#[test]
fn duplicate_hits_across_priority_stages_keep_all_sources_on_first_stage() {
    let mut stages = vec![
        (
            "P1".into(),
            vec![],
            vec![row(
                "example.com",
                "domain",
                "http://example.com/",
                "精确主机:example.com",
            )],
        ),
        (
            "P3".into(),
            vec![],
            vec![row(
                "集团",
                "company",
                "http://example.com:80/",
                "标题全称:集团",
            )],
        ),
        (
            "Q2".into(),
            vec![],
            vec![row(
                "另一公司",
                "company",
                "http://example.com/",
                "证书组织:另一公司",
            )],
        ),
    ];
    assert_eq!(merge_probe_stages(&mut stages), 2);
    assert_eq!(stages[0].2.len(), 1);
    assert!(stages[1].2.is_empty() && stages[2].2.is_empty());
    assert_eq!(discovery_sources(&stages[0].2[0]).len(), 3);
    assert!(stages[0].1.contains(&"discovery_sources_json".into()));
}

#[test]
fn persisted_discovery_retains_previous_round_sources_without_double_counting() {
    let company = json!({"label":"集团", "kind":"company", "ip":"1.2.3.4"});
    let domain = json!({"label":"example.com", "kind":"domain"});
    let (merged, count, truncated) = merge_persisted_discovery(
        &json!([company.clone()]).to_string(),
        &json!([company.clone(), domain]).to_string(),
    );
    assert_eq!(count, 2);
    let (again, count, again_truncated) =
        merge_persisted_discovery(&merged, &json!([company]).to_string());
    assert_eq!(count, 2);
    assert_eq!(merged, again);
    assert!(!truncated && !again_truncated);
}

#[test]
fn old_checkpoint_without_provenance_does_not_invent_an_extra_discovery_source() {
    let stored = json!([{"label":"集团","kind":"company","evidence":"标题全称:集团"},
        {"label":"example.com","kind":"domain","evidence":"精确主机:example.com"}])
    .to_string();
    let incoming =
        json!([{"label":"集团","kind":"","evidence":"","matchedQueries":"","assetKey":"old-key"}])
            .to_string();
    let (merged, count, truncated) = merge_persisted_discovery(&stored, &incoming);
    assert_eq!(count, 2);
    assert!(!truncated);
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&merged)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn actual_probe_contacts_shared_endpoint_once_and_fresh_job_rechecks_it() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server_requests = requests.clone();
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    assert!(read_fixture_request(&mut stream).is_some());
                    server_requests.fetch_add(1, Ordering::SeqCst);
                    let body = "<html><title>集团门户</title><body>Welcome to the company portal</body></html>";
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("fixture server: {error}"),
            }
        }
    });
    let root = std::env::temp_dir().join(format!("oviraptor-discovery-{}", uuid::Uuid::new_v4()));
    let refined = root.join("refined");
    fs::create_dir_all(&refined).unwrap();
    let fields = [
        "company",
        "discovery_seed_kind",
        "asset_key",
        "link",
        "protocol",
        "evidence",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let url = format!("http://{address}/");
    write_csv(
        &refined.join("P1_active_strong.csv"),
        &fields,
        &[row("example.com", "domain", &url, "精确主机:example.com")],
    )
    .unwrap();
    write_csv(
        &refined.join("P2_strong_needs_validation.csv"),
        &fields,
        &[],
    )
    .unwrap();
    write_csv(
        &refined.join("P3_name_candidates.csv"),
        &fields,
        &[row("集团", "company", &url, "标题全称:集团")],
    )
    .unwrap();
    let options = ProbeOptions {
        workers: 4,
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
    };
    let cancel = AtomicBool::new(false);
    let output = root.join("probe");
    let first = probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &output,
        options.clone(),
        &cancel,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(first["merged_duplicates"], 1);
    let (_, results) = read_csv(&output.join("P1_web_alive.csv")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(discovery_sources(&results[0]).len(), 2);
    let resumed = probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &output,
        options.clone(),
        &cancel,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(resumed["reused_results"], 1);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &root.join("fresh-probe"),
        options,
        &cancel,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_collection_unions_company_and_domain_queries_without_losing_path_case() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            assert!(read_fixture_request(&mut stream).is_some());
            let body = json!({"error":false,"size":3,"results": [
                {"host":"http://example.com/A","link":"http://example.com/A","ip":"192.0.2.1","port":"80","protocol":"http"},
                {"host":"http://example.com/a","link":"http://example.com/a","ip":"192.0.2.1","port":"80","protocol":"http"},
                {"host":"http://example.com/","link":"http://example.com/","ip":"192.0.2.1","port":"80","protocol":"http"}
            ]}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let root =
        std::env::temp_dir().join(format!("oviraptor-collect-union-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let seeds = root.join("seeds.csv");
    fs::write(
        &seeds,
        "company,names,domains\n集团有限公司,集团有限公司,\n,,example.com\n",
    )
    .unwrap();
    let output = root.join("collection");
    let summary = collect_fofa_at(
        &seeds,
        &output,
        CollectOptions {
            key: "synthetic-fixture-key".into(),
            proxy: "".into(),
            page_size: 100,
            max_pages: 1,
            timeout: Duration::from_secs(3),
            interval: Duration::ZERO,
            full: false,
            enable_cidr24: false,
            profile: "professional".into(),
            cache: false,
        },
        &AtomicBool::new(false),
        &format!("http://{address}/search"),
        |_, _, _| {},
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(summary["queries"], 6);
    assert_eq!(summary["source_candidates"], 6);
    assert_eq!(summary["candidates"], 3);
    let (_, rows) = read_csv(&output.join("candidates.csv")).unwrap();
    assert_eq!(rows.len(), 3);
    let expected_queries = BTreeSet::from([
        "title=\"集团有限公司\"",
        "body=\"集团有限公司\"",
        "cert.subject.org=\"集团有限公司\"",
        "host=\"example.com\"",
        "domain=\"example.com\"",
        "cert.domain=\"example.com\"",
    ]);
    let expected_sources = BTreeSet::from([
        ("company", "集团有限公司"),
        ("domain", "example.com"),
    ]);
    for row in &rows {
        assert_eq!(text(row, &["company"]), "集团有限公司");
        let sources = discovery_sources(row);
        // Two logical seed sources still coalesce each URL once; all six
        // distinct query receipts survive instead of being collapsed to two.
        assert_eq!(discovery_source_count(&sources), 2);
        assert_eq!(text(row, &["discovery_source_count"]), "2");
        assert_eq!(sources.len(), 6);
        assert_eq!(
            sources.iter().map(|source| (source["kind"].as_str().unwrap(), source["label"].as_str().unwrap())).collect::<BTreeSet<_>>(),
            expected_sources
        );
        for (kind, label) in &expected_sources {
            assert_eq!(sources.iter().filter(|source| source["kind"] == *kind && source["label"] == *label).count(), 3);
        }
        assert_eq!(
            sources.iter().map(|source| source["matchedQueries"].as_str().unwrap()).collect::<BTreeSet<_>>(),
            expected_queries
        );
        assert_eq!(text(row, &["matched_queries"]).split(" | ").collect::<BTreeSet<_>>(), expected_queries);
        let request_hashes = sources.iter().map(|source| source["requestSha256"].as_str().unwrap()).collect::<BTreeSet<_>>();
        assert_eq!(request_hashes.len(), 6);
        assert!(request_hashes.iter().all(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())));
        for source in &sources {
            assert_eq!(source["page"], "1");
            assert_eq!(source["timeStatus"], "received");
            assert!(chrono::DateTime::parse_from_rfc3339(source["receivedAt"].as_str().unwrap()).is_ok());
            assert_eq!(source["link"].as_str().unwrap(), text(row, &["link"]));
        }
        assert_eq!(text(row, &["discovery_sources_truncated"]), "false");
        assert!(text(row, &["evidence"]).contains("标题全称:集团有限公司"));
        assert!(text(row, &["evidence"]).contains("精确主机:example.com"));
    }
    assert!(rows
        .iter()
        .any(|row| text(row, &["link"]) == "http://example.com/A"));
    assert!(rows
        .iter()
        .any(|row| text(row, &["link"]) == "http://example.com/a"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn optimizer_preserves_all_distinct_requested_entries_even_with_shared_redirect() {
    let root = std::env::temp_dir().join(format!("oviraptor-opt-union-{}", uuid::Uuid::new_v4()));
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    let urls = [
        "http://example.com/",
        "https://example.com/",
        "http://www.example.com/",
        "http://example.com:8080/",
        "http://example.com/A",
        "http://example.com/a",
        "http://example.com/a/",
        "http://example.com/?q=1",
        "http://example.com/?q=2",
    ];
    let rows = urls
        .iter()
        .map(|url| {
            let mut row = row("集团", "company", url, "标题全称:集团");
            row.insert("probe_outcome".into(), "web_alive".into());
            row.insert("probe_status_code".into(), "200".into());
            row.insert(
                "probe_effective_url".into(),
                "https://example.com/login".into(),
            );
            row
        })
        .collect::<Vec<_>>();
    let fields = [
        "company",
        "link",
        "protocol",
        "probe_outcome",
        "probe_status_code",
        "probe_effective_url",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    write_csv(&input.join("P1_web_alive.csv"), &fields, &rows).unwrap();
    let summary = optimize_src_assets(&input, &root.join("out")).unwrap();
    assert_eq!(summary["kept"], urls.len());
    assert_eq!(summary["excluded"], 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn known_service_protocols_on_one_host_port_are_not_discovery_duplicates() {
    let service = |protocol: &str, label: &str| -> Record {
        [
            ("company", label),
            ("discovery_seed_kind", "company"),
            ("host", "EXAMPLE.test"),
            ("port", "443"),
            ("protocol", protocol),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    };
    let merged = merge_discovery_rows(vec![
        service("ssh", "A"),
        service("redis", "A"),
        service("SSH", "B"),
        service("https", "A"),
    ]);
    assert_eq!(merged.len(), 3);
    let ssh = merged
        .iter()
        .find(|row| text(row, &["asset_key"]).starts_with("service:ssh|"))
        .unwrap();
    assert_eq!(discovery_source_count(&discovery_sources(ssh)), 2);
    assert!(!ssh.contains_key("ownership_status"));
}

#[test]
fn discovery_merge_chooses_receipt_order_keeps_provider_literal_and_matching_receipts() {
    let observation = |observed: &str, received: &str, ip: &str, title: &str| {
        let mut value = row("集团", "company", "https://same.example/", "title-query");
        for (key, field_value) in [
            ("lastupdatetime", observed),
            ("collection_provider_observed_at", observed),
            ("collection_received_at", received),
            ("ip", ip),
            ("title", title),
            ("collection_request_sha256", "request"),
            ("collection_payload_sha256", "payload"),
            ("collection_actual_fields_json", "[\"host\",\"ip\"]"),
            ("collection_rule_version", "v1"),
        ] {
            value.insert(key.into(), field_value.into());
        }
        value
    };
    let old = observation(
        "2026-10-10 10:00:00",
        "2026-10-15T00:00:00Z",
        "192.0.2.1",
        "old",
    );
    let new = observation(
        "2026-10-14 10:00:00",
        "2026-10-14T12:00:00Z",
        "192.0.2.2",
        "",
    );
    for rows in [
        vec![old.clone(), new.clone()],
        vec![new.clone(), old.clone()],
    ] {
        let merged = merge_discovery_rows(rows);
        assert_eq!(text(&merged[0], &["ip"]), "192.0.2.1");
        assert_eq!(text(&merged[0], &["title"]), "old");
        let sources = discovery_sources(&merged[0]);
        assert_eq!(
            sources.len(),
            1,
            "same logical query retains its latest observation"
        );
        assert_eq!(sources[0]["observedAt"], "2026-10-10 10:00:00");
        assert_eq!(sources[0]["receivedAt"], "2026-10-15T00:00:00Z");
        assert_eq!(sources[0]["payloadSha256"], "payload");
        assert_eq!(sources[0]["actualFieldsJson"], "[\"host\",\"ip\"]");
    }
    let unknown = observation("not-a-time", "", "192.0.2.3", "unknown");
    let merged = merge_discovery_rows(vec![new, unknown]);
    assert_eq!(text(&merged[0], &["ip"]), "192.0.2.2");
    assert_eq!(text(&merged[0], &["title"]), "");
}

#[test]
fn discovery_history_has_explicit_count_and_byte_limits_including_single_oversized_source() {
    let sources: Vec<_> = (0..200).map(|index| json!({"kind":"company", "label":format!("source-{index}"), "link":"https://same.example/", "observedAt":"2026-10-15T00:00:00Z"})).collect();
    let (stored, count, truncated) = merge_persisted_discovery("[]", &json!(sources).to_string());
    assert!(truncated);
    assert_eq!(count, DISCOVERY_SOURCE_LIMIT);
    assert!(stored.len() <= DISCOVERY_SOURCE_BYTES);
    let huge = json!([{"kind":"company", "label":"oversized", "evidence":"x".repeat(DISCOVERY_SOURCE_BYTES+1)}]).to_string();
    let (stored, count, truncated) = merge_persisted_discovery(&huge, "[]");
    assert_eq!((stored.as_str(), count, truncated), ("[]", 0, true));
    let (stored, count, truncated) = merge_persisted_discovery("not-json", "[]");
    assert_eq!((stored.as_str(), count, truncated), ("[]", 0, true));
    let mut oversized = row("oversized", "company", "https://same.example/", "query");
    oversized.insert(
        "matched_queries".into(),
        "x".repeat(DISCOVERY_SOURCE_BYTES + 1),
    );
    let merged = merge_discovery_rows(vec![oversized]);
    assert_eq!(text(&merged[0], &["discovery_sources_truncated"]), "true");
    assert!(text(&merged[0], &["discovery_sources_json"]).len() <= DISCOVERY_SOURCE_BYTES);
}

#[test]
fn discovery_newer_whole_snapshot_drops_missing_old_fields_without_refilling_them() {
    let mut old = row("集团", "company", "https://same.example/", "query");
    for (key, value) in [
        ("collection_received_at", "2026-10-14T00:00:00Z"),
        ("ip", "192.0.2.1"),
        ("domain", "old.example"),
        ("org", "old organization"),
        ("country", "old country"),
        ("title", "old title"),
    ] {
        old.insert(key.into(), value.into());
    }
    let mut new = row("集团", "company", "https://same.example/", "query");
    new.insert(
        "collection_received_at".into(),
        "2026-10-15T00:00:00Z".into(),
    );
    new.insert("ip".into(), "192.0.2.2".into());
    for records in [
        vec![old.clone(), new.clone()],
        vec![new.clone(), old.clone()],
    ] {
        let merged = merge_discovery_rows(records);
        assert_eq!(text(&merged[0], &["ip"]), "192.0.2.2");
        for key in ["domain", "org", "country", "title"] {
            assert!(
                !merged[0].contains_key(key),
                "{key} must be unknown in the selected snapshot"
            );
        }
        let sources = discovery_sources(&merged[0]);
        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources[0]["observation"],
            json!(super::super::asset_observation::source_snapshot(&merged[0]))
        );
        assert_eq!(sources[0]["receivedAt"], "2026-10-15T00:00:00Z");
    }
}

#[test]
fn discovery_equal_time_winner_and_receipt_are_identical_in_both_orders_and_on_reentry() {
    let observation = |ip: &str, title: &str, payload: &str| {
        let mut value = row("集团", "company", "https://same.example/", "query");
        for (key, field) in [
            ("ip", ip),
            ("title", title),
            ("collection_payload_sha256", payload),
            ("collection_request_sha256", "request"),
            ("collection_received_at", "2026-10-15T00:00:00Z"),
            ("collection_provider_observed_at", "2026-10-10 00:00:00"),
        ] {
            value.insert(key.into(), field.into());
        }
        value
    };
    let first = observation("192.0.2.1", "first", "payload-a");
    let second = observation("192.0.2.2", "second", "payload-b");
    let forward = merge_discovery_rows(vec![first.clone(), second.clone()]);
    let backward = merge_discovery_rows(vec![second, first]);
    assert_eq!(forward, backward);
    assert_eq!(merge_discovery_rows(forward.clone()), forward);
    let sources = discovery_sources(&forward[0]);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["ip"], text(&forward[0], &["ip"]));
    assert_eq!(
        sources[0]["payloadSha256"],
        text(&forward[0], &["collection_payload_sha256"])
    );
    assert_eq!(
        sources[0]["observation"],
        json!(super::super::asset_observation::source_snapshot(
            &forward[0]
        ))
    );
}
