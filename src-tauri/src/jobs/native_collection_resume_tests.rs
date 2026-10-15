use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{mpsc, Mutex},
    thread,
};

#[derive(Clone, Debug)]
struct Request {
    query: String,
    page: usize,
    fields: Vec<String>,
}

enum Reply {
    Json(serde_json::Value),
    Status(u16),
    Hang(Arc<AtomicBool>, serde_json::Value),
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(mut handler: impl FnMut(&Request) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/search", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    Err(error) => panic!("local fixture accept: {error}"),
                };
                // macOS accepted sockets can inherit listener NONBLOCK.
                // A short WouldBlock is not a provider request failure.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let amount = match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(amount) => amount,
                    };
                    bytes.extend_from_slice(&buffer[..amount]);
                    if bytes.windows(4).any(|part| part == b"\r\n\r\n") || bytes.len() > 64 * 1024 {
                        break;
                    }
                }
                let message = String::from_utf8_lossy(&bytes);
                let Some(target) = message
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                else {
                    continue;
                };
                let endpoint = if target.starts_with("http://") || target.starts_with("https://") {
                    reqwest::Url::parse(target).unwrap()
                } else {
                    reqwest::Url::parse(&format!("http://fixture.invalid{target}")).unwrap()
                };
                let pairs = endpoint.query_pairs().collect::<HashMap<_, _>>();
                let request = Request {
                    query: String::from_utf8(
                        base64::engine::general_purpose::STANDARD
                            .decode(pairs.get("qbase64").unwrap().as_bytes())
                            .unwrap(),
                    )
                    .unwrap(),
                    page: pairs.get("page").unwrap().parse().unwrap(),
                    fields: pairs
                        .get("fields")
                        .unwrap()
                        .split(',')
                        .map(str::to_string)
                        .collect(),
                };
                recorded.lock().unwrap().push(request.clone());
                let (status, body) = match handler(&request) {
                    Reply::Json(value) => (200, value.to_string()),
                    Reply::Status(status) => (status, "{}".into()),
                    Reply::Hang(released, value) => {
                        while !released.load(Ordering::SeqCst) && !stopped.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        (200, value.to_string())
                    }
                };
                let _ = write!(stream, "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(worker),
        }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

struct Fixture {
    root: PathBuf,
    seeds: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-collection-resume-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let seeds = root.join("seeds.csv");
        fs::write(
            &seeds,
            "company,names,domains\nExample Group,,example.test\n",
        )
        .unwrap();
        Self {
            output: root.join("output"),
            root,
            seeds,
        }
    }

    fn state(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.output.join("collection_state.json")).unwrap())
            .unwrap()
    }

    fn pages(&self) -> Vec<PathBuf> {
        fs::read_dir(self.output.join("collection-checkpoint-v1/pages"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn options(cache: bool) -> CollectOptions {
    CollectOptions {
        key: "synthetic-resume-key+private".into(),
        proxy: String::new(),
        page_size: 1,
        max_pages: 0,
        timeout: Duration::from_secs(3),
        interval: Duration::ZERO,
        full: false,
        enable_cidr24: false,
        profile: "professional".into(),
        cache,
    }
}

fn payload(request: &Request, matches: usize) -> serde_json::Value {
    let identity = format!("{:x}", Sha256::digest(request.query.as_bytes()));
    let host = format!("http://example.test/{}-{}", &identity[..8], request.page);
    json!({"error":false,"size":matches,"results":[{"host":host,"link":host,"ip":"192.0.2.5","port":"80","protocol":"http","title":"local fixture"}]})
}

fn collect(
    fixture: &Fixture,
    server: &Server,
    options: CollectOptions,
) -> Result<serde_json::Value, String> {
    collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options,
        &AtomicBool::new(false),
        &server.url,
        |_, _, _| {},
    )
}

fn query_summaries(summary: &serde_json::Value) -> &[serde_json::Value] {
    summary["query_summaries"]
        .as_array()
        .expect("collection summaries must retain typed per-query coverage evidence")
}

fn assert_summary_contract(summary: &serde_json::Value) {
    for field in [
        "queries",
        "completed_queries",
        "failed_queries",
        "truncated_queries",
        "partial_queries",
        "invalid_records",
        "candidates",
        "reused_pages",
    ] {
        assert!(
            summary[field].is_u64(),
            "missing numeric {field}: {summary}"
        );
    }
    assert!(summary["complete"].is_boolean());
    assert!(summary["coverage_complete"].is_boolean());
    assert!(summary.get("error_categories").is_some());
    assert_eq!(
        query_summaries(summary).len() as u64,
        summary["queries"].as_u64().unwrap()
    );
    assert_eq!(
        summary["queries"].as_u64().unwrap(),
        summary["completed_queries"].as_u64().unwrap()
            + summary["failed_queries"].as_u64().unwrap()
    );
}

fn assert_query_coverage_reason(summary: &serde_json::Value, reason: &str) {
    assert_eq!(summary["coverage_complete"], false, "{summary}");
    for query in query_summaries(summary) {
        assert!(
            query["coverage_reasons"]
                .as_array()
                .expect("query coverage reasons must be structured codes")
                .iter()
                .any(|value| value.as_str() == Some(reason)),
            "missing {reason}: {query}"
        );
    }
}

fn assert_utc_receipt(value: &str) {
    let received = chrono::DateTime::parse_from_rfc3339(value)
        .expect("page receipt time must be an RFC3339 timestamp");
    assert_eq!(received.offset().local_minus_utc(), 0, "{value}");
}

fn assert_legacy_time_warning(summary: &serde_json::Value) {
    assert!(summary["compatibility_reasons"]
        .as_array()
        .expect("legacy compatibility facts must remain separate from current coverage")
        .iter()
        .any(|value| value.as_str() == Some("legacy_time_unknown")));
}

#[test]
fn actual_collection_resume_reuses_successful_pages_even_when_cache_is_disabled() {
    let broken = Arc::new(AtomicBool::new(true));
    let fail = broken.clone();
    let server = Server::new(move |request| {
        if request.query.starts_with("cert.domain=")
            && request.page == 2
            && fail.load(Ordering::SeqCst)
        {
            Reply::Status(400)
        } else {
            Reply::Json(payload(request, 2))
        }
    });
    let fixture = Fixture::new();
    let error = collect(&fixture, &server, options(false)).unwrap_err();
    assert!(error.contains("1/3"));
    assert!(error.contains("cert.domain=\"example.test\""));
    assert!(error.contains("http_client"));
    assert_eq!(fixture.pages().len(), 5);
    assert_summary_contract(&fixture.state());
    assert_eq!(fixture.state()["complete"], false);
    assert_eq!(fixture.state()["candidates"], 5);
    assert_eq!(server.requests().len(), 6);
    assert!(server
        .requests()
        .iter()
        .all(|request| request.fields == fofa_fields("professional")));
    assert!(!fixture.output.join("native-cache").exists());
    let (_, log) = read_csv(&fixture.output.join("query_log.csv")).unwrap();
    let failure = log
        .iter()
        .find(|row| text(row, &["status"]) == "error")
        .unwrap();
    assert_eq!(text(failure, &["fetched"]), "1");
    assert_eq!(text(failure, &["last_page"]), "2");
    assert_eq!(text(failure, &["partial"]), "true");

    broken.store(false, Ordering::SeqCst);
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["reused_pages"], 5);
    assert_eq!(summary["request_attempts"], 1);
    assert_eq!(summary["candidates"], 6);
    assert_eq!(fixture.state()["complete"], true);
    assert_eq!(server.requests().len(), 7);
    assert_eq!(server.requests().last().unwrap().page, 2);
    assert!(server
        .requests()
        .last()
        .unwrap()
        .query
        .starts_with("cert.domain="));
    assert_eq!(
        collect(&fixture, &server, options(false)).unwrap()["request_attempts"],
        0
    );
    assert_eq!(server.requests().len(), 7);
    let fresh = fixture.root.join("fresh");
    collect_fofa_at(
        &fixture.seeds,
        &fresh,
        options(false),
        &AtomicBool::new(false),
        &server.url,
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(server.requests().len(), 13);
}

#[test]
fn actual_collection_resume_imports_legacy_url_hash_pages_with_their_real_fields() {
    // The historical name remains traceable. Undated raw pages now retain
    // their original schema as compatibility evidence and are safely refetched;
    // they are no longer imported as if they had a fresh observation time.
    let server = Server::new(|request| {
        let mut value = payload(request, 1);
        value["results"][0]["title"] = "fresh provider response".into();
        Reply::Json(value)
    });
    let fixture = Fixture::new();
    let legacy = fixture.output.join("native-cache");
    fs::create_dir_all(&legacy).unwrap();
    let queries = build_fofa_queries(&fixture.seeds, false).unwrap();
    let configured = fofa_fields("professional");
    let safe = [
        "host",
        "link",
        "ip",
        "port",
        "protocol",
        "domain",
        "title",
        "icp",
        "org",
        "server",
        "status_code",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();
    let mut original_pages = BTreeMap::new();
    for (index, query) in queries.iter().enumerate() {
        let fields = if index == 0 { &safe } else { &configured };
        let endpoint = fofa_endpoint(&server.url, &options(false), query, 1, 1, fields).unwrap();
        let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
        let request = Request {
            query: query.query.clone(),
            page: 1,
            fields: fields.clone(),
        };
        let original = payload(&request, 1).to_string();
        fs::write(legacy.join(format!("{signature}.json")), &original).unwrap();
        original_pages.insert(signature, (fields.clone(), original));
    }
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["migrated_legacy_pages"], 3);
    assert_eq!(summary["reused_pages"], 0);
    assert_eq!(summary["request_attempts"], 3);
    assert_eq!(summary["candidates"], 3);
    assert_eq!(summary["coverage_complete"], true);
    assert_legacy_time_warning(&summary);
    assert_eq!(server.requests().len(), 3);
    assert!(server
        .requests()
        .iter()
        .all(|request| request.fields == configured));
    assert_eq!(fixture.pages().len(), 3);
    let receipts: serde_json::Value = serde_json::from_slice(
        &fs::read(
            fixture
                .output
                .join("collection-checkpoint-v1/legacy-receipts.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(receipts["version"], 1);
    let receipts = receipts["records"].as_array().unwrap();
    assert_eq!(receipts.len(), 3);
    for receipt in receipts {
        let signature = receipt["request_sha256"].as_str().unwrap();
        let (fields, original) = original_pages.get(signature).unwrap();
        assert_eq!(receipt["fields"], json!(fields));
        assert_eq!(receipt["received_at"], serde_json::Value::Null);
        assert_eq!(receipt["reason"], "legacy_time_unknown");
        assert_eq!(
            receipt["payload_sha256"],
            format!("{:x}", Sha256::digest(original.as_bytes()))
        );
        assert_eq!(
            fs::read_to_string(legacy.join(format!("{signature}.json"))).unwrap(),
            *original
        );
    }
    let (_, candidates) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    for row in candidates {
        assert_eq!(text(&row, &["title"]), "fresh provider response");
        assert_utc_receipt(&text(&row, &["collection_received_at"]));
    }
    let binding =
        fs::read_to_string(fixture.output.join("collection-checkpoint-v1/binding.json")).unwrap();
    assert!(!binding.contains(&options(false).key));
}

#[test]
fn actual_collection_resume_never_trusts_legacy_error_or_malformed_pages() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    let legacy = fixture.output.join("native-cache");
    fs::create_dir_all(&legacy).unwrap();
    for (index, query) in build_fofa_queries(&fixture.seeds, false)
        .unwrap()
        .iter()
        .enumerate()
    {
        let endpoint = fofa_endpoint(
            &server.url,
            &options(false),
            query,
            1,
            1,
            &fofa_fields("professional"),
        )
        .unwrap();
        let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
        let value = match index {
            0 => json!({"error":true,"errmsg":"old failure","results":[]}),
            1 => json!({"error":"false","results":[]}),
            _ => json!({"error":false,"results":[null,12,"malformed"]}),
        };
        fs::write(legacy.join(format!("{signature}.json")), value.to_string()).unwrap();
    }
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["reused_pages"], 0);
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fixture.pages().len(), 3);
}

#[test]
fn actual_collection_resume_legacy_key_change_cannot_silently_restart_all_queries() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    let legacy = fixture.output.join("native-cache");
    fs::create_dir_all(&legacy).unwrap();
    let queries = build_fofa_queries(&fixture.seeds, false).unwrap();
    let query = &queries[0];
    let endpoint = fofa_endpoint(
        &server.url,
        &options(false),
        query,
        1,
        1,
        &fofa_fields("professional"),
    )
    .unwrap();
    let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
    let request = Request {
        query: query.query.clone(),
        page: 1,
        fields: fofa_fields("professional"),
    };
    let path = legacy.join(format!("{signature}.json"));
    let original = payload(&request, 1).to_string();
    fs::write(&path, &original).unwrap();
    let mut changed = options(false);
    changed.key = "different-legacy-account-key".into();
    assert!(collect(&fixture, &server, changed)
        .unwrap_err()
        .contains("未静默重发"));
    assert!(server.requests().is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!fixture
        .output
        .join("collection-checkpoint-v1/binding.json")
        .exists());
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["reused_pages"], 0);
    assert_eq!(summary["request_attempts"], 3);
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn actual_collection_resume_cached_candidate_csv_bytes_and_ties_are_stable() {
    let server = Server::new(|request| Reply::Json(payload(request, 2)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let original = fs::read(fixture.output.join("candidates.csv")).unwrap();
    let review = fs::read(fixture.output.join("review_candidates.csv")).unwrap();
    for _ in 0..3 {
        let summary = collect(&fixture, &server, options(false)).unwrap();
        assert_eq!(summary["request_attempts"], 0);
        assert_eq!(
            fs::read(fixture.output.join("candidates.csv")).unwrap(),
            original
        );
        assert_eq!(
            fs::read(fixture.output.join("review_candidates.csv")).unwrap(),
            review
        );
    }
    assert_eq!(server.requests().len(), 6);
}

#[cfg(unix)]
#[test]
fn actual_collection_resume_same_directory_rejects_concurrent_execution() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let binding: native_collection_resume::CollectionBinding = serde_json::from_slice(
        &fs::read(fixture.output.join("collection-checkpoint-v1/binding.json")).unwrap(),
    )
    .unwrap();
    let owned =
        native_collection_resume::CollectionCheckpoint::open(&fixture.output, binding).unwrap();
    assert!(collect(&fixture, &server, options(false))
        .unwrap_err()
        .contains("已有任务"));
    assert_eq!(server.requests().len(), 3);
    drop(owned);
    assert_eq!(
        collect(&fixture, &server, options(false)).unwrap()["request_attempts"],
        0
    );
}

#[test]
fn actual_collection_resume_rejects_changed_seeds_options_key_and_endpoint_before_http() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let binding_path = fixture.output.join("collection-checkpoint-v1/binding.json");
    let binding = fs::read(&binding_path).unwrap();
    let candidates = fs::read(fixture.output.join("candidates.csv")).unwrap();
    let original_seeds = fs::read(&fixture.seeds).unwrap();
    fs::write(
        &fixture.seeds,
        "company,names,domains\nOther Group,,other.test\n",
    )
    .unwrap();
    assert!(collect(&fixture, &server, options(false))
        .unwrap_err()
        .contains("种子"));
    fs::write(&fixture.seeds, original_seeds).unwrap();
    for index in 0..5 {
        let mut changed = options(false);
        match index {
            0 => changed.page_size = 2,
            1 => changed.max_pages = 1,
            2 => changed.full = true,
            3 => changed.profile = "corporate".into(),
            _ => changed.enable_cidr24 = true,
        }
        assert!(collect(&fixture, &server, changed)
            .unwrap_err()
            .contains("选项"));
    }
    let mut changed = options(false);
    changed.key = "a-different-private-key".into();
    assert!(collect(&fixture, &server, changed)
        .unwrap_err()
        .contains("Key 已改变"));
    let error = collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options(false),
        &AtomicBool::new(false),
        &format!("{}/changed", server.url),
        |_, _, _| {},
    )
    .unwrap_err();
    assert!(error.contains("选项"));
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fs::read(binding_path).unwrap(), binding);
    assert_eq!(
        fs::read(fixture.output.join("candidates.csv")).unwrap(),
        candidates
    );
    let mut transport_only = options(true);
    transport_only.timeout = Duration::from_secs(9);
    transport_only.interval = Duration::from_millis(100);
    assert_eq!(
        collect(&fixture, &server, transport_only).unwrap()["request_attempts"],
        0
    );
}

#[test]
fn actual_collection_resume_retries_transient_http_errors_with_live_safe_messages() {
    let mut transient = 0;
    let server = Server::new(move |request| {
        if transient < 2 {
            transient += 1;
            Reply::Status(503)
        } else {
            Reply::Json(payload(request, 1))
        }
    });
    let fixture = Fixture::new();
    let mut messages = Vec::new();
    let summary = collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options(false),
        &AtomicBool::new(false),
        &server.url,
        |_, _, message| messages.push(message.to_string()),
    )
    .unwrap();
    assert_eq!(
        summary["request_attempts"],
        5,
        "summary={summary}, requests={:?}, messages={messages:?}",
        server.requests()
    );
    assert_eq!(server.requests().len(), 5);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.starts_with("FOFA 自动重试："))
            .count(),
        2
    );
    assert!(messages
        .iter()
        .all(|message| !message.contains(&options(false).key)));
}

#[test]
fn actual_collection_resume_stops_after_bounded_retries_and_preserves_partial_results() {
    let server = Server::new(|request| {
        if request.query.starts_with("cert.domain=") {
            Reply::Status(503)
        } else {
            Reply::Json(payload(request, 1))
        }
    });
    let fixture = Fixture::new();
    let mut messages = Vec::new();
    assert!(collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options(false),
        &AtomicBool::new(false),
        &server.url,
        |_, _, message| messages.push(message.to_string())
    )
    .is_err());
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request.query.starts_with("cert.domain="))
            .count(),
        3
    );
    assert_eq!(fixture.state()["complete"], false);
    assert_eq!(fixture.state()["candidates"], 2);
    assert_summary_contract(&fixture.state());
    assert!(fixture.state()["error_categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|category| category.as_str() == Some("http_server")));
    assert!(messages
        .iter()
        .any(|message| message.starts_with("FOFA 查询失败：") && message.contains("cert.domain=")));
    let (_, logs) = read_csv(&fixture.output.join("query_log.csv")).unwrap();
    let failure = logs
        .iter()
        .find(|row| text(row, &["status"]) == "error")
        .unwrap();
    assert_eq!(text(failure, &["error_category"]), "http_server");
    assert_eq!(text(failure, &["request_attempts"]), "3");
}

#[test]
fn actual_collection_resume_cancel_interrupts_in_flight_http_and_keeps_pages() {
    let released = Arc::new(AtomicBool::new(false));
    let gate = released.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let server = Server::new(move |request| {
        if request.query.starts_with("cert.domain=")
            && request.page == 2
            && !gate.load(Ordering::SeqCst)
        {
            started_tx.send(()).unwrap();
            Reply::Hang(gate.clone(), payload(request, 2))
        } else {
            Reply::Json(payload(request, 2))
        }
    });
    let fixture = Fixture::new();
    let seeds = fixture.seeds.clone();
    let output = fixture.output.clone();
    let api = server.url.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || {
        let mut policy = options(false);
        policy.timeout = Duration::from_secs(30);
        collect_fofa_at(&seeds, &output, policy, &worker_cancel, &api, |_, _, _| {})
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = Instant::now();
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(worker.join().unwrap().unwrap_err(), "__CANCELLED__");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fixture.state()["status"], "cancelled");
    assert_eq!(fixture.state()["complete"], false);
    assert_eq!(fixture.state()["candidates"], 1);
    assert_eq!(fixture.pages().len(), 1);
    released.store(true, Ordering::SeqCst);
    assert_eq!(
        collect(&fixture, &server, options(false)).unwrap()["reused_pages"],
        1
    );
    assert_eq!(server.requests().len(), 7);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request.query.starts_with("cert.domain=") && request.page == 1)
            .count(),
        1
    );
}

#[test]
fn actual_collection_resume_cancel_interrupts_the_retry_backoff() {
    let server = Server::new(|_| Reply::Status(503));
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let result = collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options(false),
        &cancel,
        &server.url,
        |_, _, message| {
            if message.starts_with("FOFA 自动重试：") {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(result.unwrap_err(), "__CANCELLED__");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(fixture.state()["complete"], false);
}

#[test]
fn actual_collection_resume_recovers_durable_pages_after_worker_unwind() {
    let server = Server::new(|request| Reply::Json(payload(request, 2)));
    let fixture = Fixture::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        collect_fofa_at(
            &fixture.seeds,
            &fixture.output,
            options(false),
            &AtomicBool::new(false),
            &server.url,
            |done, _, _| {
                if done == 1 {
                    panic!("local fixture worker terminated between queries");
                }
            },
        )
    }));
    assert!(result.is_err());
    assert_eq!(fixture.pages().len(), 2);
    assert_eq!(fixture.state()["complete"], false);
    assert_eq!(
        collect(&fixture, &server, options(false)).unwrap()["reused_pages"],
        2
    );
    assert_eq!(server.requests().len(), 6);
    assert_eq!(fixture.state()["candidates"], 6);
}

#[test]
fn actual_collection_resume_refetches_a_corrupt_page_and_refuses_a_corrupt_binding() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let page = fixture.pages().pop().unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&page).unwrap()).unwrap();
    saved["payload"]["results"] = json!([]);
    fs::write(page, saved.to_string()).unwrap();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["request_attempts"], 1);
    assert_eq!(summary["candidates"], 3);
    fs::write(
        fixture.output.join("collection-checkpoint-v1/binding.json"),
        "{broken",
    )
    .unwrap();
    assert!(collect(&fixture, &server, options(false))
        .unwrap_err()
        .contains("绑定损坏"));
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn actual_collection_resume_rejects_invalid_provider_payload_and_masks_request_secrets() {
    let server = Server::new(|request| {
        if request.query.starts_with("cert.domain=") {
            Reply::Json(json!({"error":false,"size":4}))
        } else {
            Reply::Json(payload(request, 1))
        }
    });
    let fixture = Fixture::new();
    assert!(collect(&fixture, &server, options(false))
        .unwrap_err()
        .contains("response_shape"));
    assert_eq!(fixture.pages().len(), 2);
    let key = &options(false).key;
    let masked = native_collection_resume_transport::sanitize(&format!("error sending request for url http://user:proxy-password@fixture.invalid/?key={key} → Connection refused (os error 61)"), key, "");
    assert!(masked.contains("Connection refused (os error 61)"));
    assert!(!masked.contains(key));
    assert!(!masked.contains("proxy-password"));
}

#[test]
fn actual_collection_resume_rate_limit_is_global_and_configured_page_limits_remain_visible() {
    let server = Server::new(|_| Reply::Status(429));
    let fixture = Fixture::new();
    assert!(collect(&fixture, &server, options(false))
        .unwrap_err()
        .contains("http_rate_limit"));
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fixture.state()["complete"], false);
    assert_eq!(fixture.state()["failed_queries"], 3);
    let proxy_server = Server::new(|_| Reply::Status(407));
    let proxy_failure = Fixture::new();
    assert!(collect(&proxy_failure, &proxy_server, options(false))
        .unwrap_err()
        .contains("proxy_auth"));
    assert_eq!(proxy_server.requests().len(), 1);
    assert_eq!(proxy_failure.state()["complete"], false);
    let limited_server = Server::new(|request| Reply::Json(payload(request, 2)));
    let limited = Fixture::new();
    let mut policy = options(false);
    policy.max_pages = 1;
    let summary = collect(&limited, &limited_server, policy).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["truncated_queries"], 3);
    assert_eq!(summary["partial_queries"], 3);
    assert_eq!(summary["coverage_complete"], false);
    assert_query_coverage_reason(&summary, "page_limit");
    assert_eq!(limited.state()["coverage_complete"], false);
    assert_eq!(limited_server.requests().len(), 3);
    assert!(limited_server
        .requests()
        .iter()
        .all(|request| request.page == 1));
}

#[test]
fn actual_collection_resume_short_page_cannot_prove_remaining_results_were_collected() {
    let server = Server::new(|request| Reply::Json(payload(request, 4)));
    let fixture = Fixture::new();
    let mut policy = options(false);
    policy.page_size = 2;
    let summary = collect(&fixture, &server, policy).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["failed_queries"], 0);
    assert_eq!(summary["truncated_queries"], 0);
    assert_eq!(summary["partial_queries"], 3);
    assert_query_coverage_reason(&summary, "short_page");
    assert_eq!(fixture.state()["coverage_complete"], false);
    assert_eq!(server.requests().len(), 3);
    let (_, logs) = read_csv(&fixture.output.join("query_log.csv")).unwrap();
    for log in logs {
        assert_eq!(text(&log, &["fetched"]), "1");
        assert_eq!(text(&log, &["total_matches"]), "4");
        assert_eq!(text(&log, &["partial"]), "true");
    }
}

#[test]
fn actual_collection_resume_unknown_total_keeps_fetching_full_pages_without_claiming_coverage() {
    for string_total in [false, true] {
        let server = Server::new(move |request| {
            let mut value = payload(request, 2);
            if string_total {
                value["size"] = "2".into();
            } else {
                value.as_object_mut().unwrap().remove("size");
            }
            if request.page >= 3 {
                value["results"] = json!([]);
            }
            Reply::Json(value)
        });
        let fixture = Fixture::new();
        let summary = collect(&fixture, &server, options(false)).unwrap();
        assert_summary_contract(&summary);
        assert_eq!(summary["complete"], true);
        assert_eq!(summary["failed_queries"], 0);
        assert_query_coverage_reason(&summary, "unknown_total");
        assert_eq!(fixture.state()["coverage_complete"], false);
        assert_eq!(server.requests().len(), 9, "string_total={string_total}");
        for query in query_summaries(&summary) {
            assert_eq!(query["fetched_rows"], 2);
            assert_eq!(query["valid_rows"], 2);
            assert_eq!(query["rejected_rows"], 0);
        }
    }
}

#[test]
fn actual_collection_resume_changed_total_remains_unproven_even_after_reaching_latest_count() {
    let server =
        Server::new(|request| Reply::Json(payload(request, if request.page == 1 { 2 } else { 3 })));
    let fixture = Fixture::new();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_query_coverage_reason(&summary, "total_changed");
    assert_eq!(fixture.state()["coverage_complete"], false);
    assert!(server.requests().iter().any(|request| request.page == 2));
    for query in query_summaries(&summary) {
        assert_eq!(query["fetched_rows"], 3);
        assert_eq!(query["valid_rows"], 3);
    }
}

#[test]
fn actual_collection_resume_empty_record_shapes_are_counted_and_never_become_candidates() {
    let server = Server::new(|_| Reply::Json(json!({"error":false,"size":2,"results":[[],{}]})));
    let fixture = Fixture::new();
    let mut policy = options(false);
    policy.page_size = 2;
    let summary = collect(&fixture, &server, policy).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["candidates"], 0);
    assert_eq!(summary["invalid_records"], 6);
    assert_query_coverage_reason(&summary, "invalid_records");
    for query in query_summaries(&summary) {
        assert_eq!(query["fetched_rows"], 2);
        assert_eq!(query["valid_rows"], 0);
        assert_eq!(query["rejected_rows"], 2);
    }
    let (_, candidates) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(candidates.is_empty());
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn actual_collection_resume_explicit_zero_total_empty_result_is_a_complete_empty_query() {
    let server = Server::new(|_| Reply::Json(json!({"error":false,"size":0,"results":[]})));
    let fixture = Fixture::new();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["coverage_complete"], true);
    assert_eq!(summary["invalid_records"], 0);
    assert_eq!(summary["candidates"], 0);
    assert_eq!(server.requests().len(), 3);
    for query in query_summaries(&summary) {
        assert_eq!(query["fetched_rows"], 0);
        assert_eq!(query["valid_rows"], 0);
        assert_eq!(query["rejected_rows"], 0);
        assert!(query["coverage_reasons"].as_array().unwrap().is_empty());
    }
}

#[test]
fn actual_collection_resume_field_fallback_records_actual_columns_and_incomplete_field_coverage() {
    let requested = fofa_fields("professional");
    let full_fields = requested.clone();
    let server = Server::new(move |request| {
        if request.fields == full_fields {
            Reply::Json(json!({"error":true,"errmsg":"field permission denied"}))
        } else {
            Reply::Json(payload(request, 1))
        }
    });
    let fixture = Fixture::new();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_query_coverage_reason(&summary, "field_downgrade");
    let requests = server.requests();
    let actual = requests
        .iter()
        .find(|request| request.fields != requested)
        .expect("account field restriction must cause a basic-field request")
        .fields
        .clone();
    assert!(!actual.is_empty());
    for query in query_summaries(&summary) {
        assert_eq!(query["requested_fields"], json!(requested));
        assert_eq!(query["actual_field_sets"], json!([actual]));
        assert_eq!(query["field_coverage_complete"], false);
    }
    let (_, candidates) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(!candidates.is_empty());
    for row in candidates {
        let fields: Vec<String> =
            serde_json::from_str(&text(&row, &["collection_actual_fields_json"])).unwrap();
        assert_eq!(fields, actual);
    }
}

#[test]
fn actual_collection_resume_replays_original_receipt_time_and_provider_observation_verbatim() {
    const OBSERVED: &str = "2024-01-02 03:04:05";
    let server = Server::new(|request| {
        let mut value = payload(request, 1);
        value["results"][0]["lastupdatetime"] = OBSERVED.into();
        Reply::Json(value)
    });
    let fixture = Fixture::new();
    let before = chrono::Utc::now();
    let first = collect(&fixture, &server, options(false)).unwrap();
    let after = chrono::Utc::now();
    assert_summary_contract(&first);
    let original = fs::read(fixture.output.join("candidates.csv")).unwrap();
    let (_, candidates) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(!candidates.is_empty());
    for row in &candidates {
        let received_at = text(row, &["collection_received_at"]);
        assert_utc_receipt(&received_at);
        let received = chrono::DateTime::parse_from_rfc3339(&received_at).unwrap();
        assert!(received.timestamp() >= before.timestamp());
        assert!(received.timestamp() <= after.timestamp());
        assert_eq!(text(row, &["collection_provider_observed_at"]), OBSERVED);
        assert_eq!(text(row, &["collection_page"]), "1");
        assert!(!text(row, &["collection_rule_version"]).is_empty());
        for field in ["collection_payload_sha256", "collection_request_sha256"] {
            let digest = text(row, &[field]);
            assert_eq!(digest.len(), 64, "{field}={digest}");
            assert!(digest.chars().all(|value| value.is_ascii_hexdigit()));
        }
        let actual: Vec<String> =
            serde_json::from_str(&text(row, &["collection_actual_fields_json"])).unwrap();
        assert_eq!(actual, fofa_fields("professional"));
    }
    for query in query_summaries(&first) {
        let first_at = query["received_at_first"].as_str().unwrap();
        let last_at = query["received_at_last"].as_str().unwrap();
        assert_utc_receipt(first_at);
        assert_eq!(
            first_at, last_at,
            "one successful page has one receipt time"
        );
    }
    // Cross a whole UTC second so regenerating a replay timestamp cannot pass
    // just because the first collection and replay happened in the same tick.
    thread::sleep(Duration::from_millis(1100));
    let replay = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(replay["request_attempts"], 0);
    assert_eq!(replay["reused_pages"], 3);
    assert_eq!(server.requests().len(), 3);
    assert_eq!(
        fs::read(fixture.output.join("candidates.csv")).unwrap(),
        original
    );
    for (original, restored) in query_summaries(&first).iter().zip(query_summaries(&replay)) {
        assert_eq!(restored["received_at_first"], original["received_at_first"]);
        assert_eq!(restored["received_at_last"], original["received_at_last"]);
    }
}

#[test]
fn actual_collection_resume_refetches_v1_pages_without_inventing_their_receipt_time() {
    let refreshed = Arc::new(AtomicBool::new(false));
    let fresh = refreshed.clone();
    let server = Server::new(move |request| {
        let mut value = payload(request, 1);
        if fresh.load(Ordering::SeqCst) {
            value["results"][0]["title"] = "fresh receipt after legacy recovery".into();
        }
        Reply::Json(value)
    });
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let page = fixture.pages().pop().unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&page).unwrap()).unwrap();
    let signature = saved["request_sha256"].as_str().unwrap().to_string();
    let payload_sha256 = saved["payload_sha256"].clone();
    let fields = saved["fields"].clone();
    saved.as_object_mut().unwrap().remove("received_at");
    saved.as_object_mut().unwrap().remove("rule_version");
    let original = saved.to_string();
    fs::write(&page, &original).unwrap();
    refreshed.store(true, Ordering::SeqCst);

    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["coverage_complete"], true);
    assert_eq!(summary["request_attempts"], 1);
    assert_eq!(summary["reused_pages"], 2);
    assert_legacy_time_warning(&summary);
    assert_eq!(server.requests().len(), 4);
    let archive = fixture
        .output
        .join("collection-checkpoint-v1/legacy-pages")
        .join(page.file_name().unwrap());
    assert_eq!(fs::read_to_string(archive).unwrap(), original);
    let receipts: serde_json::Value = serde_json::from_slice(
        &fs::read(
            fixture
                .output
                .join("collection-checkpoint-v1/legacy-receipts.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let records = receipts["records"].as_array().unwrap();
    let legacy = records
        .iter()
        .find(|value| value["request_sha256"].as_str() == Some(signature.as_str()))
        .unwrap();
    assert_eq!(legacy["received_at"], serde_json::Value::Null);
    assert_eq!(legacy["reason"], "legacy_time_unknown");
    assert_eq!(legacy["payload_sha256"], payload_sha256);
    assert_eq!(legacy["fields"], fields);
    let current: serde_json::Value = serde_json::from_slice(&fs::read(page).unwrap()).unwrap();
    assert_utc_receipt(current["received_at"].as_str().unwrap());
    assert_eq!(
        current["payload"]["results"][0]["title"],
        "fresh receipt after legacy recovery"
    );
}

#[test]
fn actual_collection_resume_success_payload_echoes_never_persist_keys_or_proxy_credentials() {
    const PROXY_USER: &str = "fixture-proxy-user";
    const PROXY_PASSWORD: &str = "fixture-proxy-password+private@secret";
    let policy = options(false);
    let key = policy.key.clone();
    let mut key_url = reqwest::Url::parse("http://fixture.invalid/").unwrap();
    key_url.query_pairs_mut().append_pair("key", &key);
    let encoded_key = key_url
        .query()
        .unwrap()
        .trim_start_matches("key=")
        .to_string();
    let echoed_key = key.clone();
    let echoed_encoded_key = encoded_key.clone();
    let server = Server::new(move |request| {
        let mut value = payload(request, 1);
        value["results"][0]["title"] = format!(
            "key={echoed_key}; encoded={echoed_encoded_key}; proxy={PROXY_USER}:{PROXY_PASSWORD}"
        )
        .into();
        value["provider_echo"] = json!({
            "nested":[{"key":echoed_key,"encoded_key":echoed_encoded_key}],
            "proxy_password":PROXY_PASSWORD,
            "proxy_url":format!("http://{PROXY_USER}:{PROXY_PASSWORD}@127.0.0.1:12345/")
        });
        Reply::Json(value)
    });
    // The same loopback fixture accepts absolute request URIs as an HTTP proxy.
    // Every origin and proxy address remains 127.0.0.1.
    let mut proxy_url = reqwest::Url::parse(&server.url).unwrap();
    proxy_url.set_path("/");
    proxy_url.set_username(PROXY_USER).unwrap();
    proxy_url.set_password(Some(PROXY_PASSWORD)).unwrap();
    let encoded_password = proxy_url.password().unwrap().to_string();
    let mut policy = policy;
    policy.proxy = proxy_url.to_string();
    let fixture = Fixture::new();
    let summary = collect(&fixture, &server, policy).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_eq!(server.requests().len(), 3);

    let password = PROXY_PASSWORD.to_string();
    let mut directories = vec![fixture.output.clone()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            let bytes = fs::read(&path).unwrap();
            let persisted = String::from_utf8_lossy(&bytes);
            for secret in [&key, &encoded_key, &password, &encoded_password] {
                assert!(
                    !persisted.contains(secret),
                    "successful response leaked configured credentials to {}",
                    path.display()
                );
            }
        }
    }
    let (_, candidates) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(!candidates.is_empty());
    for row in candidates {
        assert!(text(&row, &["host"]).starts_with("http://example.test/"));
        assert_eq!(text(&row, &["collection_payload_redacted"]), "true");
        assert_eq!(
            text(&row, &["collection_payload_hash_semantics"]),
            "sha256-redacted-json-v1"
        );
    }
    for page in fixture.pages() {
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(page).unwrap()).unwrap();
        assert_eq!(saved["payload_redacted"], true);
        assert_eq!(saved["payload_hash_semantics"], "sha256-redacted-json-v1");
        assert_eq!(
            saved["payload_sha256"],
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&saved["payload"]).unwrap())
            )
        );
    }
}

#[test]
fn actual_collection_resume_failed_legacy_refetch_preserves_unknown_time_and_typed_failure() {
    let server = Server::new(|_| Reply::Status(400));
    let fixture = Fixture::new();
    let legacy = fixture.output.join("native-cache");
    fs::create_dir_all(&legacy).unwrap();
    let queries = build_fofa_queries(&fixture.seeds, false).unwrap();
    let fields = fofa_fields("professional");
    let endpoint = fofa_endpoint(&server.url, &options(false), &queries[0], 1, 1, &fields).unwrap();
    let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
    let request = Request {
        query: queries[0].query.clone(),
        page: 1,
        fields,
    };
    let original = payload(&request, 1).to_string();
    let path = legacy.join(format!("{signature}.json"));
    fs::write(&path, &original).unwrap();
    let error = collect(&fixture, &server, options(false)).unwrap_err();
    assert!(error.contains("http_client"));
    let state = fixture.state();
    assert_summary_contract(&state);
    assert_eq!(state["complete"], false);
    assert_eq!(state["coverage_complete"], false);
    assert_eq!(state["failed_queries"], 3);
    assert_eq!(state["reused_pages"], 0);
    assert_eq!(state["candidates"], 0);
    assert_legacy_time_warning(&state);
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    let receipts: serde_json::Value = serde_json::from_slice(
        &fs::read(
            fixture
                .output
                .join("collection-checkpoint-v1/legacy-receipts.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let records = receipts["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["request_sha256"], signature);
    assert_eq!(records[0]["received_at"], serde_json::Value::Null);
    assert_eq!(records[0]["reason"], "legacy_time_unknown");
}

#[test]
fn actual_collection_resume_connection_errors_keep_safe_root_causes_and_categories() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let mut seen = String::new();
    collect_fofa_at(
        &fixture.seeds,
        &fixture.output,
        options(false),
        &cancel,
        &format!("http://{address}/search"),
        |_, _, message| {
            if message.starts_with("FOFA 自动重试：") {
                seen = message.into();
                cancel.store(true, Ordering::SeqCst);
            }
        },
    )
    .unwrap_err();
    assert!(seen.contains("connect"));
    assert!(seen.to_lowercase().contains("refused") || seen.contains("os error"));
    assert!(!seen.contains(&options(false).key));
    assert!(!seen.contains("key="));
}

#[test]
fn actual_collection_resume_transports_share_one_runtime_without_waiting_for_blocking_dns() {
    let first = native_collection_resume_transport::Transport::new(
        "fixture",
        "",
        Duration::from_secs(3),
        Duration::ZERO,
    )
    .unwrap();
    let runtime = first.runtime_for_test();
    let identity = runtime as *const tokio::runtime::Runtime;
    for _ in 0..100 {
        let transport = native_collection_resume_transport::Transport::new(
            "fixture",
            "",
            Duration::from_secs(3),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(
            transport.runtime_for_test() as *const tokio::runtime::Runtime,
            identity
        );
    }
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    // Model an OS resolver that cannot be cancelled while inside getaddrinfo.
    // Transport disposal must not wait for this shared blocking task.
    let resolver = runtime.spawn_blocking(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = Instant::now();
    drop(first);
    assert!(start.elapsed() < Duration::from_secs(1));
    let replacement = native_collection_resume_transport::Transport::new(
        "fixture",
        "",
        Duration::from_secs(3),
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(
        replacement.runtime_for_test() as *const tokio::runtime::Runtime,
        identity
    );
    release_tx.send(()).unwrap();
    runtime.block_on(resolver).unwrap();
}

#[test]
fn actual_collection_resume_duplicate_pages_stop_without_claiming_coverage() {
    let server = Server::new(|request| {
        let mut first = request.clone();
        first.page = 1;
        Reply::Json(payload(&first, 10))
    });
    let fixture = Fixture::new();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["complete"], true);
    assert_query_coverage_reason(&summary, "duplicate_page");
    assert_eq!(server.requests().len(), 6);
    assert!(server.requests().iter().all(|request| request.page <= 2));
}

#[test]
fn actual_collection_resume_changed_binding_is_bound_to_every_candidate_and_state() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    let first = collect(&fixture, &server, options(false)).unwrap();
    let binding = first["binding_sha256"].as_str().unwrap();
    assert_eq!(binding.len(), 64);
    assert_eq!(fixture.state()["binding_sha256"], binding);
    let (_, rows) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(!rows.is_empty());
    assert!(rows
        .iter()
        .all(|row| text(row, &["collection_binding_sha256"]) == binding));
    let other = Fixture::new();
    let mut changed = options(false);
    changed.max_pages = 1;
    let second = collect(&other, &server, changed).unwrap();
    assert_ne!(second["binding_sha256"], first["binding_sha256"]);
    assert_eq!(second["coverage_complete"], true);
}

#[test]
fn actual_collection_resume_short_secrets_are_redacted_when_embedded_and_encoded() {
    let raw = json!({"echo":"prefix:k+;proxy:u:p@;suffix","nested":["k%2B","u%3Ap%40","p%40"]});
    let (redacted, changed) = native_collection_resume_transport::redact_payload(
        &raw,
        "k+",
        "http://u:p%40@127.0.0.1:1234/",
    );
    assert!(changed);
    let bytes = serde_json::to_string(&redacted).unwrap();
    for secret in ["k+", "k%2B", "p@", "p%40", "u"] {
        assert!(!bytes.contains(secret), "{redacted}");
    }
    // Redaction can destroy a network identifier. Such a row is rejected by
    // the ordinary identity validator, rather than counted as full coverage.
    let (network, _) =
        native_collection_resume_transport::redact_payload(&json!({"host":"http://h/"}), "h", "");
    let rows = flatten_fofa(&json!([network]), &["host".into()]);
    assert!(!super::super::asset_identity::valid_network_identity(
        "",
        &text(&rows[0], &["host"]),
        "",
        "",
        ""
    ));
}

#[test]
fn actual_collection_resume_current_run_byte_budget_is_reported_before_another_request() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let pages = fixture.pages();
    let original = fs::read(&pages[0]).unwrap();
    let oversized = fixture.output.join("collection-checkpoint-v1/legacy-pages");
    fs::create_dir_all(&oversized).unwrap();
    let sparse = oversized.join(format!("{}.json", "f".repeat(64)));
    fs::File::create(&sparse)
        .unwrap()
        .set_len(native_collection_resume::MAX_RECEIPT_BYTES + 1)
        .unwrap();
    let neighbor = fixture.root.join("other-run");
    fs::create_dir_all(&neighbor).unwrap();
    fs::write(neighbor.join("keep.json"), b"synthetic unrelated evidence").unwrap();
    let error = collect(&fixture, &server, options(false)).unwrap_err();
    assert!(error.contains("resource_limited"), "{error}");
    let state = fixture.state();
    assert_summary_contract(&state);
    assert_eq!(state["complete"], false);
    assert_eq!(state["coverage_complete"], false);
    assert_eq!(state["retention"]["limit_reached"], true);
    assert!(state["coverage_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "retention_limit"));
    assert_eq!(server.requests().len(), 3);
    assert_eq!(fs::read(&pages[0]).unwrap(), original);
    assert_eq!(
        fs::read(neighbor.join("keep.json")).unwrap(),
        b"synthetic unrelated evidence"
    );
    assert!(sparse.exists());
}

#[test]
fn actual_collection_resume_expired_receive_time_refetches_without_forging_observation() {
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let fixture = Fixture::new();
    collect(&fixture, &server, options(false)).unwrap();
    let page = fixture.pages().pop().unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&page).unwrap()).unwrap();
    saved["received_at"] = (chrono::Utc::now() - chrono::Duration::days(8))
        .to_rfc3339()
        .into();
    fs::write(&page, saved.to_string()).unwrap();
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["request_attempts"], 1);
    assert_eq!(summary["reused_pages"], 2);
    assert_eq!(summary["retention"]["expired_pages"], 1);
    assert_eq!(summary["coverage_complete"], true);
    let current: serde_json::Value = serde_json::from_slice(&fs::read(&page).unwrap()).unwrap();
    assert_ne!(current["received_at"], saved["received_at"]);
    let (_, rows) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
    assert!(rows
        .iter()
        .all(|row| text(row, &["collection_provider_observed_at"]).is_empty()));
}

#[test]
fn actual_collection_resume_owned_storage_registers_all_artifacts_and_refuses_extra_http() {
    use crate::jobs::collection_retention::{CollectionStorage, Limits};
    fn owned_fixture(pages: usize) -> (Fixture, CollectionStorage) {
        let mut fixture = Fixture::new();
        let jobs = fixture.root.join("jobs");
        let run = jobs.join("synthetic-run");
        fs::create_dir_all(&run).unwrap();
        fixture.output = run.join("collection");
        let limits = Limits {
            run_bytes: 2 * 1024 * 1024,
            project_bytes: 4 * 1024 * 1024,
            global_bytes: 8 * 1024 * 1024,
            pages,
            ..Limits::default()
        };
        let storage = CollectionStorage::open(
            &jobs,
            &fixture.output,
            1,
            1,
            chrono::Utc::now().timestamp(),
            limits,
        )
        .unwrap();
        (fixture, storage)
    }
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
    let (fixture, storage) = owned_fixture(10);
    let summary = collect_fofa_at_impl(
        &fixture.seeds,
        &fixture.output,
        options(true),
        &AtomicBool::new(false),
        &server.url,
        Some(&storage),
        |_, _, _| {},
    )
    .unwrap();
    assert_summary_contract(&summary);
    assert_eq!(summary["coverage_complete"], true);
    assert_eq!(server.requests().len(), 3);
    assert_eq!(storage.report().unwrap().stored_pages, 3);
    assert!(
        !fixture.output.join("native-cache").exists(),
        "new runs must not duplicate raw pages"
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.output.join(".collection-retention-v1.json")).unwrap(),
    )
    .unwrap();
    let entries = manifest["entries"].as_object().unwrap();
    for path in [
        "collection-checkpoint-v1/binding.json",
        "collection_state.json",
        "query_plan.csv",
        "query_log.csv",
        "candidates.csv",
        "review_candidates.csv",
    ] {
        let entry = entries
            .get(path)
            .unwrap_or_else(|| panic!("unregistered {path}"));
        assert_eq!(entry["phase"], "available");
        let bytes = fs::read(fixture.output.join(path)).unwrap();
        assert_eq!(entry["hash"], format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(entry["bytes"], bytes.len());
    }
    for path in fixture.pages() {
        let relative = path
            .strip_prefix(&fixture.output)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(entries[relative]["kind"], "page");
        assert!(entries[relative]["receivedAt"].is_i64());
        assert_eq!(
            entries[relative]["hash"],
            format!("{:x}", Sha256::digest(fs::read(&path).unwrap()))
        );
    }
    assert_eq!(storage.report().unwrap().unknown_files, 0);
    // Exercise the same real collector with a one-page owned budget. The first
    // page is durable; page two is refused before HTTP and later queries do not
    // consume another request. This is not a mocked storage helper.
    let limited_server = Server::new(|request| Reply::Json(payload(request, 2)));
    let (limited, storage) = owned_fixture(1);
    let error = collect_fofa_at_impl(
        &limited.seeds,
        &limited.output,
        options(false),
        &AtomicBool::new(false),
        &limited_server.url,
        Some(&storage),
        |_, _, _| {},
    )
    .unwrap_err();
    assert!(error.contains("resource_limited"), "{error}");
    let state = limited.state();
    assert_summary_contract(&state);
    assert_eq!(state["complete"], false);
    assert_eq!(state["coverage_complete"], false);
    assert_eq!(state["failed_queries"], 3);
    assert_eq!(state["candidates"], 1);
    assert!(state["error_categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|code| code == "resource_limited"));
    assert_eq!(limited_server.requests().len(), 1);
    assert_eq!(limited.pages().len(), 1);
    assert_eq!(storage.report().unwrap().stored_pages, 1);
}

#[test]
fn actual_collection_resume_redacted_locators_never_become_rewritten_probe_targets() {
    for arrays in [false, true] {
        let server = Server::new(move |request| {
            if arrays {
                let row = request
                    .fields
                    .iter()
                    .map(|field| match field.as_str() {
                        "host" | "link" => json!("http://example.test/original"),
                        "ip" => json!("192.0.2.5"),
                        "port" => json!("80"),
                        "protocol" => json!("http"),
                        _ => json!(""),
                    })
                    .collect::<Vec<_>>();
                Reply::Json(json!({"error":false,"size":1,"results":[row]}))
            } else {
                Reply::Json(payload(request, 1))
            }
        });
        let fixture = Fixture::new();
        let mut policy = options(false);
        policy.key = "x".into();
        let summary = collect(&fixture, &server, policy).unwrap();
        assert_summary_contract(&summary);
        assert_eq!(summary["complete"], true);
        assert_eq!(summary["coverage_complete"], false);
        assert_eq!(summary["invalid_records"], 3);
        assert_eq!(summary["candidates"], 0);
        assert_query_coverage_reason(&summary, "redacted_locator");
        let (fields, rows) = read_csv(&fixture.output.join("candidates.csv")).unwrap();
        assert!(fields.iter().any(|field| field == "host"));
        assert!(
            rows.is_empty(),
            "sanitized eample.test is not a source target"
        );
        for page in fixture.pages() {
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(page).unwrap()).unwrap();
            assert_eq!(saved["redacted_locator_rows"], json!([0]));
            assert_eq!(saved["payload_redacted"], true);
            assert_eq!(
                saved["redaction_rule_version"],
                native_collection_resume_transport::REDACTION_RULE_VERSION
            );
        }
        let mut policy = options(false);
        policy.key = "x".into();
        let replay = collect(&fixture, &server, policy).unwrap();
        assert_eq!(replay["request_attempts"], 0);
        assert_eq!(replay["candidates"], 0);
        assert_eq!(replay["invalid_records"], 3);
        assert_eq!(server.requests().len(), 3);
    }
}

#[test]
fn actual_collection_resume_partial_page_overlap_is_not_proven_by_raw_row_count() {
    let server = Server::new(|request| {
        let mut original = request.clone();
        original.page = 1;
        let first = payload(&original, 4)["results"][0].clone();
        let mut other = first.clone();
        let endpoint = format!(
            "{}-{}",
            first["host"].as_str().unwrap(),
            if request.page == 1 { "B" } else { "C" }
        );
        other["host"] = endpoint.clone().into();
        other["link"] = endpoint.into();
        Reply::Json(json!({"error":false,"size":4,"results":[first,other]}))
    });
    let fixture = Fixture::new();
    let mut policy = options(false);
    policy.page_size = 2;
    let summary = collect(&fixture, &server, policy).unwrap();
    assert_eq!(summary["complete"], true);
    assert_query_coverage_reason(&summary, "duplicate_record");
    assert_eq!(server.requests().len(), 6);
    for query in query_summaries(&summary) {
        assert_eq!(query["fetched_rows"], 4);
        assert_eq!(query["valid_rows"], 4);
        assert!(!query["coverage_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "duplicate_page"));
    }
}
