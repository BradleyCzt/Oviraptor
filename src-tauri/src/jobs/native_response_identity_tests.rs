use super::*;

fn gzip_fixture_body(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

fn read_fixture_request(stream: &mut TcpStream) -> Option<String> {
    // macOS accepted sockets can inherit the listener's O_NONBLOCK. Consume
    // every request header before closing the response, otherwise an unread
    // tail or WouldBlock turns the intended fixture into a random TCP reset.
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
            return String::from_utf8(request).ok();
        }
    }
    None
}

fn response_row(url: &str, status: u16, body: &[u8]) -> Record {
    let hash = format!("{:x}", Sha256::digest(body));
    [
        ("company", "Group".into()),
        ("asset_key", url.into()),
        ("link", url.into()),
        ("probe_input_url", url.into()),
        ("probe_effective_url", url.into()),
        ("probe_outcome", "web_alive".into()),
        ("probe_status_code", status.to_string()),
        ("probe_title", "Welcome".into()),
        ("probe_content_type", "text/plain".into()),
        ("probe_body_bytes", body.len().to_string()),
        ("probe_body_limit", "4096".into()),
        ("probe_body_complete", "true".into()),
        ("probe_body_truncated", "false".into()),
        (
            "probe_response_fingerprint",
            full_response_fingerprint(status, body.len(), &hash, "text/plain"),
        ),
        ("body_sha256", hash),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}

#[test]
fn exact_response_requires_success_full_bytes_and_same_final_status() {
    let mut rows = vec![
        response_row("http://one.example/A", 200, b"welcome"),
        response_row("https://two.example/B?q=1", 200, b"welcome"),
        response_row("http://one.example/a", 200, b"welcomE"),
        response_row("http://one.example/A/", 200, b"welcome "),
        response_row("http://one.example/other-status", 201, b"welcome"),
    ];
    let summary = annotate_exact_response_groups(&mut rows, &[0, 1, 2, 3, 4]);
    assert_eq!((summary.groups, summary.duplicates), (1, 1));
    assert_eq!(
        text(&rows[0], &["response_duplicate_of"]),
        "https://two.example/B?q=1"
    );
    assert_eq!(text(&rows[1], &["response_group_member_count"]), "2");
    for row in &rows[2..] {
        assert!(text(row, &["response_group_key"]).is_empty());
    }
    let members: Vec<serde_json::Value> =
        serde_json::from_str(&text(&rows[1], &["response_group_members_json"])).unwrap();
    assert!(members
        .iter()
        .any(|member| member["inputUrl"] == "http://one.example/A"));
    assert!(members
        .iter()
        .any(|member| member["inputUrl"] == "https://two.example/B?q=1"));
}

#[test]
fn common_titles_prefixes_restrictions_and_missing_proof_never_group() {
    let original = response_row("http://one.example/", 200, b"welcome");
    let mutations = [
        ("probe_body_complete", ""),
        ("probe_body_truncated", "true"),
        ("probe_body_limit", "3"),
        ("probe_content_encoding", "gzip"),
        ("probe_content_range", "bytes 0-6/9999"),
        ("probe_body_bytes", "0"),
        ("probe_outcome", "browser_render_required"),
        ("probe_response_fingerprint", "old-proof"),
        ("probe_error", "read timed out"),
        ("body_sha256", "not-a-digest"),
    ];
    for (field, value) in mutations {
        let mut other = original.clone();
        other.insert(field.into(), value.into());
        assert!(exact_response_identity(&other).is_none(), "{field}");
    }
    for status in [204, 206, 301, 401, 403, 429, 500, 503] {
        let body = if status == 204 {
            &b""[..]
        } else {
            &b"welcome"[..]
        };
        let other = response_row("http://two.example/", status, body);
        assert!(exact_response_identity(&other).is_none(), "HTTP {status}");
    }
    let same_title_different_body = response_row("http://two.example/", 200, b"onenet");
    assert_eq!(
        text(&original, &["probe_title"]),
        text(&same_title_different_body, &["probe_title"])
    );
    assert_ne!(
        exact_response_identity(&original),
        exact_response_identity(&same_title_different_body)
    );
}

#[test]
fn representative_is_stable_group_trace_is_linear_and_later_changes_clear_it() {
    let rows = (0..300)
        .map(|index| response_row(&format!("http://node{index:03}.example/"), 200, b"onenet"))
        .collect::<Vec<_>>();
    let mut reversed = rows.clone();
    reversed.reverse();
    let selected = (0..rows.len()).collect::<Vec<_>>();
    let mut rows = rows;
    annotate_exact_response_groups(&mut rows, &selected);
    annotate_exact_response_groups(&mut reversed, &selected);
    assert_eq!(
        text(&rows[0], &["response_group_representative_url"]),
        "http://node000.example/"
    );
    assert!(reversed
        .iter()
        .all(|row| text(row, &["response_group_representative_url"]) == "http://node000.example/"));
    assert_eq!(
        rows.iter()
            .filter(|row| !text(row, &["response_group_members_json"]).is_empty())
            .count(),
        1
    );
    let members: Vec<serde_json::Value> =
        serde_json::from_str(&text(&rows[0], &["response_group_members_json"])).unwrap();
    assert_eq!(members.len(), 300);
    for (index, row) in rows.iter_mut().enumerate() {
        let url = text(row, &["link"]);
        *row = response_row(&url, 200, format!("unique {index}").as_bytes());
        row.insert("response_group_key".into(), "old-key".into());
    }
    let summary = annotate_exact_response_groups(&mut rows, &selected);
    assert_eq!(summary.groups, 0);
    assert!(rows.iter().all(|row| RESPONSE_GROUP_FIELDS
        .iter()
        .all(|field| row[*field].is_empty())));
}

#[test]
fn optimizer_preserves_all_members_and_does_not_quarantine_response_duplicates() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-response-opt-{}", uuid::Uuid::new_v4()));
    let input = root.join("probe");
    let output = root.join("optimized");
    fs::create_dir_all(&input).unwrap();
    let mut rows = vec![
        response_row("http://one.example/A", 200, b"welcome"),
        response_row("https://two.example/B", 200, b"welcome"),
        response_row("http://one.example/a", 201, b"welcome"),
        response_row("http://error.example/", 503, b"error"),
    ];
    rows[3].insert("probe_outcome".into(), "web_abnormal".into());
    let mut old_duplicate = rows[0].clone();
    old_duplicate.insert("company".into(), "Another discovery source".into());
    old_duplicate.insert("probe_outcome".into(), "unreachable".into());
    rows.push(old_duplicate);
    let fields = rows
        .iter()
        .flat_map(|row| row.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    write_csv(&input.join("P1_web_alive.csv"), &fields, &rows).unwrap();
    let summary = optimize_src_assets(&input, &output).unwrap();
    assert_eq!(summary["kept"], 2);
    assert_eq!(summary["response_duplicates"], 1);
    assert_eq!(summary["endpoint_duplicates"], 1);
    let (_, all) = read_csv(&output.join("all_probe_results.csv")).unwrap();
    assert_eq!(all.len(), 4);
    let duplicate = all
        .iter()
        .find(|row| text(row, &["link"]) == "http://one.example/A")
        .unwrap();
    assert_eq!(text(duplicate, &["auto_excluded"]), "0");
    assert_eq!(discovery_sources(duplicate).len(), 2);
    assert_eq!(
        text(duplicate, &["response_duplicate_of"]),
        "https://two.example/B"
    );
    assert_eq!(
        text(
            all.iter()
                .find(|row| text(row, &["link"]) == "http://error.example/")
                .unwrap(),
            &["auto_excluded"]
        ),
        "1"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_http_proves_full_eof_status_body_and_truncation_boundaries() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(200)))
                        .unwrap();
                    let Some(request) = read_fixture_request(&mut stream) else {
                        continue;
                    };
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let (status, body, declared) = match path {
                        "/same-a" | "/same-b" => (200, b"welcome".to_vec(), None),
                        "/gzip-welcome" => (200, gzip_fixture_body(b"welcome"), None),
                        "/gzip-title" => (200, gzip_fixture_body(b"<title>Welcome</title>A"), None),
                        "/gzip-bad" => (200, b"bad gzip".to_vec(), None),
                        "/unsupported-encoding" => (200, b"welcome".to_vec(), None),
                        "/status" => (201, b"welcome".to_vec(), None),
                        "/title-a" => (200, b"<title>Welcome</title>A".to_vec(), None),
                        "/title-b" => (200, b"<title>Welcome</title>B".to_vec(), None),
                        "/limit" => (200, vec![b'x'; 64], None),
                        "/tail-a" => (200, [vec![b'x'; 64], vec![b'A']].concat(), None),
                        "/tail-b" => (200, [vec![b'x'; 64], vec![b'B']].concat(), None),
                        "/empty" => (200, vec![], None),
                        "/partial" => (206, b"welcome".to_vec(), None),
                        "/broken" => (200, b"welcome".to_vec(), Some(999)),
                        _ => (404, b"missing".to_vec(), None),
                    };
                    let encoding = if path.starts_with("/gzip-") {
                        "Content-Encoding: gzip\r\n"
                    } else if path == "/unsupported-encoding" {
                        "Content-Encoding: madeup\r\n"
                    } else {
                        ""
                    };
                    let header = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{encoding}Connection: close\r\n\r\n", declared.unwrap_or(body.len()));
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("local response fixture: {error}"),
            }
        }
    });
    let root =
        std::env::temp_dir().join(format!("oviraptor-response-http-{}", uuid::Uuid::new_v4()));
    let refined = root.join("refined");
    let probe = root.join("probe");
    let optimized = root.join("optimized");
    fs::create_dir_all(&refined).unwrap();
    let fields = vec![
        "company".into(),
        "asset_key".into(),
        "link".into(),
        "protocol".into(),
    ];
    let paths = [
        "same-a",
        "same-b",
        "status",
        "title-a",
        "title-b",
        "limit",
        "tail-a",
        "tail-b",
        "empty",
        "partial",
        "broken",
        "gzip-welcome",
        "gzip-title",
        "gzip-bad",
        "unsupported-encoding",
    ];
    let input = paths
        .iter()
        .map(|path| {
            [
                ("company".into(), "Synthetic fixture".into()),
                ("asset_key".into(), (*path).into()),
                ("link".into(), format!("http://{address}/{path}")),
                ("protocol".into(), "http".into()),
            ]
            .into_iter()
            .collect::<Record>()
        })
        .collect::<Vec<_>>();
    write_csv(&refined.join("P1_active_strong.csv"), &fields, &input).unwrap();
    for filename in ["P2_strong_needs_validation.csv", "P3_name_candidates.csv"] {
        write_csv(&refined.join(filename), &fields, &[]).unwrap();
    }
    let options = ProbeOptions {
        workers: 4,
        timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        retries: 0,
        max_body_bytes: 64,
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
    let result = probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &probe,
        options,
        &AtomicBool::new(false),
        |_, _| {},
    );
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    result.unwrap();
    let summary = optimize_src_assets(&probe, &optimized).unwrap();
    assert_eq!(summary["response_groups"], 2);
    assert_eq!(summary["response_duplicates"], 3);
    let (_, all) = read_csv(&optimized.join("all_probe_results.csv")).unwrap();
    assert_eq!(all.len(), paths.len());
    let find = |path: &str| {
        all.iter()
            .find(|row| text(row, &["probe_input_url"]).ends_with(&format!("/{path}")))
            .unwrap()
    };
    assert_eq!(text(find("limit"), &["probe_body_complete"]), "true");
    for path in ["tail-a", "tail-b"] {
        assert_eq!(text(find(path), &["probe_body_complete"]), "false");
        assert!(text(find(path), &["response_group_key"]).is_empty());
    }
    assert_eq!(
        text(find("tail-a"), &["body_sha256"]),
        text(find("tail-b"), &["body_sha256"])
    );
    for path in [
        "status",
        "title-b",
        "limit",
        "empty",
        "partial",
        "broken",
        "gzip-bad",
        "unsupported-encoding",
    ] {
        assert!(
            text(find(path), &["response_group_key"]).is_empty(),
            "{path}"
        );
    }
    assert_ne!(text(find("broken"), &["probe_body_complete"]), "true");
    assert_ne!(text(find("gzip-bad"), &["probe_body_complete"]), "true");
    assert_eq!(
        text(find("gzip-welcome"), &["probe_response_fingerprint"]),
        text(find("same-a"), &["probe_response_fingerprint"])
    );
    assert_eq!(text(find("gzip-title"), &["probe_title"]), "Welcome");
    assert_eq!(
        text(find("gzip-title"), &["body_sha256"]),
        text(find("title-a"), &["body_sha256"])
    );
    fs::remove_dir_all(root).unwrap();
}

fn budget_options(timeout: Duration, total_timeout: Duration) -> ProbeOptions {
    ProbeOptions {
        workers: 4,
        timeout,
        total_timeout,
        retries: 50,
        max_body_bytes: 4096,
        include_other: false,
        include_weak: false,
        allow_private: true,
        strict_tls: false,
        scheme_fallback: true,
        content_threshold: 12,
        replace_default_content_rules: false,
        gambling_keywords: vec![],
        porn_keywords: vec![],
        negative_keywords: vec![],
        custom_keywords: vec![],
        priority_rate: 10_000.0,
        other_rate: 10_000.0,
        per_host_interval: Duration::ZERO,
    }
}

fn slow_fixture_probe(body_drip: bool, options: ProbeOptions) -> (Record, Duration, usize) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fixture_stop = stop.clone();
    let fixture_requests = requests.clone();
    let server = std::thread::spawn(move || {
        let mut handles = Vec::new();
        while !fixture_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    fixture_requests.fetch_add(1, Ordering::SeqCst);
                    let stop = fixture_stop.clone();
                    handles.push(std::thread::spawn(move || {
                        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
                        if read_fixture_request(&mut stream).is_none() { return; }
                        if body_drip {
                            if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\nConnection: close\r\n\r\n").is_err() { return; }
                            for _ in 0..100 {
                                if stop.load(Ordering::SeqCst) || stream.write_all(b"x").is_err() { break; }
                                std::thread::sleep(Duration::from_millis(20));
                            }
                        } else {
                            std::thread::sleep(Duration::from_millis(200));
                            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nwelcome");
                        }
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("slow fixture: {error}"),
            }
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let row = response_row(&format!("http://{address}/"), 200, b"welcome");
    let client = reqwest::blocking::Client::builder()
        .timeout(options.timeout)
        .build()
        .unwrap();
    let rules = native_content::ContentRules::from_options(&options);
    let pacing = native_probe_control::ProbePacing::new();
    let started = Instant::now();
    let result = probe_one(
        &row,
        &options,
        &client,
        &rules,
        &pacing,
        options.priority_rate,
        &AtomicBool::new(false),
    );
    let elapsed = started.elapsed();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    (result, elapsed, requests.load(Ordering::SeqCst))
}

#[test]
fn header_timeouts_and_many_retries_share_one_total_network_budget() {
    let (result, elapsed, requests) = slow_fixture_probe(
        false,
        budget_options(Duration::from_millis(40), Duration::from_millis(110)),
    );
    assert_eq!(text(&result, &["probe_entry_state"]), "deadline_exceeded");
    assert_eq!(text(&result, &["probe_outcome"]), "unreachable");
    assert!(
        (1..=4).contains(&requests),
        "a fresh full budget was granted to a retry: {requests}"
    );
    assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
    assert!(text(&result, &["probe_response_fingerprint"]).is_empty());
}

#[test]
fn continuously_dripping_body_cannot_extend_the_total_network_budget() {
    let (result, elapsed, requests) = slow_fixture_probe(
        true,
        budget_options(Duration::from_secs(2), Duration::from_millis(110)),
    );
    assert_eq!(text(&result, &["probe_entry_state"]), "deadline_exceeded");
    assert_eq!(requests, 1);
    assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
    assert_ne!(text(&result, &["probe_body_complete"]), "true");
    assert!(text(&result, &["probe_response_fingerprint"]).is_empty());
}

#[test]
fn tcp_fallback_requires_an_observed_literal_ip_and_respects_private_gate() {
    let started = Instant::now();
    assert!(!tcp_alive(
        "http://never-resolve-this.invalid/",
        "",
        Duration::from_secs(1),
        true
    ));
    assert!(started.elapsed() < Duration::from_millis(100));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://never-resolve-this.invalid:{port}/");
    assert!(!tcp_alive(
        &url,
        "127.0.0.1",
        Duration::from_millis(100),
        false
    ));
    assert!(tcp_alive(
        &url,
        "127.0.0.1",
        Duration::from_millis(100),
        true
    ));
}

#[test]
fn dynamic_workers_dispatch_next_fast_url_before_slow_first_url_finishes() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let slow_done = Arc::new(AtomicBool::new(false));
    let third_dispatched_early = Arc::new(AtomicBool::new(false));
    let fixture_stop = stop.clone();
    let fixture_slow_done = slow_done.clone();
    let fixture_third = third_dispatched_early.clone();
    let server = std::thread::spawn(move || {
        let mut handles = Vec::new();
        while !fixture_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let slow_done = fixture_slow_done.clone();
                    let third = fixture_third.clone();
                    handles.push(std::thread::spawn(move || {
                        stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                        let Some(request) = read_fixture_request(&mut stream) else { return };
                        let path = request.split_whitespace().nth(1).unwrap_or("");
                        if path == "/slow" {
                            std::thread::sleep(Duration::from_millis(200));
                            slow_done.store(true, Ordering::SeqCst);
                        } else if path == "/fast-b" && !slow_done.load(Ordering::SeqCst) {
                            third.store(true, Ordering::SeqCst);
                        }
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\nwelcome").unwrap();
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("dynamic worker fixture: {error}"),
            }
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let root =
        std::env::temp_dir().join(format!("oviraptor-dynamic-probe-{}", uuid::Uuid::new_v4()));
    let refined = root.join("refined");
    let output = root.join("probe");
    fs::create_dir_all(&refined).unwrap();
    let fields = vec![
        "company".into(),
        "asset_key".into(),
        "link".into(),
        "protocol".into(),
    ];
    let rows = ["slow", "fast-a", "fast-b"]
        .iter()
        .map(|path| {
            [
                ("company".into(), "Synthetic fixture".into()),
                ("asset_key".into(), (*path).into()),
                ("link".into(), format!("http://{address}/{path}")),
                ("protocol".into(), "http".into()),
            ]
            .into_iter()
            .collect::<Record>()
        })
        .collect::<Vec<_>>();
    write_csv(&refined.join("P1_active_strong.csv"), &fields, &rows).unwrap();
    for filename in ["P2_strong_needs_validation.csv", "P3_name_candidates.csv"] {
        write_csv(&refined.join(filename), &fields, &[]).unwrap();
    }
    let mut options = budget_options(Duration::from_secs(1), Duration::from_secs(1));
    options.workers = 2;
    options.retries = 0;
    options.scheme_fallback = false;
    let result = probe_assets(
        &refined,
        &refined.join("P1_active_strong.csv"),
        &output,
        options,
        &AtomicBool::new(false),
        |_, _| {},
    );
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    result.unwrap();
    assert!(
        third_dispatched_early.load(Ordering::SeqCst),
        "a completed fast worker waited for its slow batch peer"
    );
    let (_, rows) = read_csv(&output.join("P1_web_alive.csv")).unwrap();
    assert_eq!(rows.len(), 3);
    fs::remove_dir_all(root).unwrap();
}
