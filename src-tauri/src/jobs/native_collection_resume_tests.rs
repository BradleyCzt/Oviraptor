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
                let endpoint =
                    reqwest::Url::parse(&format!("http://fixture.invalid{target}")).unwrap();
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
    let server = Server::new(|request| Reply::Json(payload(request, 1)));
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
    for (index, query) in queries.iter().enumerate() {
        let fields = if index == 0 { &safe } else { &configured };
        let endpoint = fofa_endpoint(&server.url, &options(false), query, 1, 1, fields).unwrap();
        let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
        let request = Request {
            query: query.query.clone(),
            page: 1,
            fields: fields.clone(),
        };
        fs::write(
            legacy.join(format!("{signature}.json")),
            payload(&request, 1).to_string(),
        )
        .unwrap();
    }
    let summary = collect(&fixture, &server, options(false)).unwrap();
    assert_eq!(summary["migrated_legacy_pages"], 3);
    assert_eq!(summary["candidates"], 3);
    assert!(server.requests().is_empty());
    assert_eq!(fixture.pages().len(), 3);
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
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert!(!fixture
        .output
        .join("collection-checkpoint-v1/binding.json")
        .exists());
    assert_eq!(
        collect(&fixture, &server, options(false)).unwrap()["reused_pages"],
        1
    );
    assert_eq!(server.requests().len(), 2);
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
    assert_eq!(summary["truncated_queries"], 3);
    assert_eq!(summary["coverage_complete"], false);
    assert_eq!(limited.state()["coverage_complete"], false);
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
