// Exercises the same DB/request path used by the Tauri validation command.
// Only disposable SQLite state and loopback listeners are used. All credentials
// and the untrusted TLS identity below are public, synthetic test fixtures.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

// Self-signed P-256 certificate with SANs 127.0.0.1 and localhost. It is never
// added to a trust store; the request must reject its untrusted issuer.
const TEST_CERT_DER: &str = "MIIB0zCCAXigAwIBAgIUD/Y3Q763FXIDS7WIAusK5dqauV0wCgYIKoZIzj0EAwIwMTEvMC0GA1UEAwwmT3ZpcmFwdG9yIHN5bnRoZXRpYyBsb29wYmFjayB0ZXN0IG9ubHkwIBcNMjYxMDEzMDQwNzQ4WhgPMjEyNjA5MTkwNDA3NDhaMDExLzAtBgNVBAMMJk92aXJhcHRvciBzeW50aGV0aWMgbG9vcGJhY2sgdGVzdCBvbmx5MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAECrtWDMM9CMfa5RVarPsZTMVEzHOM7NdgbOofYVqrnQxV5PI1xleHiNTw795878CPG5zzgpQ9yf1UEa5r8Vqmj6NsMGowHQYDVR0OBBYEFCjL49wMBwkRYuyi8ObTtOsJFaL1MB8GA1UdIwQYMBaAFCjL49wMBwkRYuyi8ObTtOsJFaL1MBoGA1UdEQQTMBGHBH8AAAGCCWxvY2FsaG9zdDAMBgNVHRMBAf8EAjAAMAoGCCqGSM49BAMCA0kAMEYCIQDauA3mxuynd18A7CgmwB+mwgcvng2ZSgGzOQPOrkRbOwIhAN1Mrr9ay4FgXcIgHSWf4iOTiIOYcW7r6hWHAgHmWF7k";
const TEST_KEY_DER: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg8WWz//zCb+qBDDadbUKJfVFXrSl+O/wzfiOOKa7XIvChRANCAAQKu1YMwz0Ix9rlFVqs+xlMxUTMc4zs12Bs6h9hWqudDFXk8jXGV4eI1PDv3nzvwI8bnPOClD3J/VQRrmvxWqaP";
const LAST_VALIDATED: &str = "2020-01-01T00:00:00Z";

struct Endpoint {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
    application_bytes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Endpoint {
    fn start(tls: bool, response: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let application_bytes = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let tls_config = tls.then(|| {
            Arc::new(
                rustls::ServerConfig::builder_with_provider(Arc::new(
                    rustls::crypto::aws_lc_rs::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(
                        STANDARD.decode(TEST_CERT_DER).unwrap(),
                    )],
                    PrivatePkcs8KeyDer::from(STANDARD.decode(TEST_KEY_DER).unwrap()).into(),
                )
                .unwrap(),
            )
        });
        let (worker_requests, worker_errors, worker_connections, worker_bytes, worker_stop) = (
            requests.clone(),
            errors.clone(),
            connections.clone(),
            application_bytes.clone(),
            stop.clone(),
        );
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                let (socket, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                };
                worker_connections.fetch_add(1, Ordering::SeqCst);
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let result = if let Some(config) = &tls_config {
                    let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                    serve_request(
                        &mut rustls::StreamOwned::new(connection, socket),
                        &response,
                        &worker_requests,
                        &worker_bytes,
                    )
                } else {
                    serve_request(&mut { socket }, &response, &worker_requests, &worker_bytes)
                };
                if let Err(error) = result {
                    worker_errors.lock().unwrap().push(error.to_string());
                }
            }
        });
        Self {
            url: format!(
                "{}://{address}/account/profile",
                if tls { "https" } else { "http" }
            ),
            requests,
            errors,
            connections,
            application_bytes,
            stop,
            worker: Some(worker),
        }
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }

    fn assert_certificate_rejected(&mut self) {
        // The client can return as soon as it sends the alert, before the server
        // processes it. Joining makes the receipt assertions race-free.
        self.finish();
        assert_eq!(
            self.connections.load(Ordering::SeqCst),
            1,
            "no insecure retry"
        );
        assert_eq!(
            self.application_bytes.load(Ordering::SeqCst),
            0,
            "TLS peer received application bytes"
        );
        assert!(
            self.requests.lock().unwrap().is_empty(),
            "TLS peer received HTTP credentials"
        );
        let errors = self.errors.lock().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("received fatal alert"),
            "expected client TLS alert, got {errors:?}"
        );
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "loopback validation fixture thread panicked during cleanup"
                );
            }
        }
    }
}

fn serve_request(
    stream: &mut (impl Read + Write),
    response: &str,
    requests: &Mutex<Vec<String>>,
    application_bytes: &AtomicUsize,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "no complete HTTP request",
            ));
        }
        application_bytes.fetch_add(size, Ordering::SeqCst);
        request.extend_from_slice(&buffer[..size]);
        if request.windows(4).any(|part| part == b"\r\n\r\n") {
            break;
        }
        assert!(
            request.len() < 64 * 1024,
            "fixture request is unexpectedly large"
        );
    }
    requests
        .lock()
        .unwrap()
        .push(String::from_utf8(request).unwrap());
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn ok_response() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK".into()
}

struct ValidationFixture {
    connection: Option<rusqlite::Connection>,
    root: std::path::PathBuf,
}

impl ValidationFixture {
    fn new(target: &str, previous_status: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-auth-validation-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(9001,'TLS validation fixture')",
                [],
            )
            .unwrap();
        let document = json!({
            "headers": {"Authorization":"Bearer synthetic-auth-token", "X-Api-Key":"synthetic-api-key"},
            "cookies": [
                {"domain":"127.0.0.1", "path":"/", "name":"session", "value":"synthetic-cookie", "secure":target.starts_with("https:")},
                {"domain":"other.invalid", "path":"/", "name":"unrelated", "value":"must-not-send"}
            ],
            "capturedRequests": [{"method":"GET", "url":target, "status":200}]
        });
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,name,entry_url,final_url,status,session_json,expires_at,last_validated_at) VALUES('fixture',9001,'Synthetic credentials',?1,?1,?2,?3,'2099-01-01T00:00:00Z',?4)",
            params![target, previous_status, document.to_string(), LAST_VALIDATED],
        ).unwrap();
        Self {
            connection: Some(connection),
            root,
        }
    }

    fn validate(&self) -> (String, String, String) {
        let connection = self.connection.as_ref().unwrap();
        validate_browser_auth_session_on(connection, "fixture").unwrap();
        connection.query_row(
            "SELECT status,last_validated_at,last_error FROM browser_auth_sessions WHERE id='fixture'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    }
}

impl Drop for ValidationFixture {
    fn drop(&mut self) {
        drop(self.connection.take());
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "failed to clean up synthetic validation database: {error}"
            );
        }
    }
}

fn assert_validation_failed(result: (String, String, String), previous_status: &str) {
    assert_eq!(
        result.0, previous_status,
        "transport failure must preserve the prior status"
    );
    assert_eq!(
        result.1, LAST_VALIDATED,
        "a rejected TLS peer is not a successful validation"
    );
    assert!(result.2.contains("会话校验请求失败"));
    for secret in [
        "synthetic-auth-token",
        "synthetic-api-key",
        "synthetic-cookie",
    ] {
        assert!(
            !result.2.contains(secret),
            "persisted failure exposed a credential"
        );
    }
}

#[test]
fn authenticated_validation_rejects_untrusted_tls_before_sending_credentials() {
    for previous_status in ["needs_check", "valid"] {
        let mut endpoint = Endpoint::start(true, ok_response());
        let fixture = ValidationFixture::new(&endpoint.url, previous_status);
        assert_validation_failed(fixture.validate(), previous_status);
        endpoint.assert_certificate_rejected();
    }
}

#[test]
fn authenticated_validation_rejects_untrusted_tls_on_a_redirect() {
    let mut tls_endpoint = Endpoint::start(true, ok_response());
    let mut redirect = Endpoint::start(
        false,
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            tls_endpoint.url,
        ),
    );
    let fixture = ValidationFixture::new(&redirect.url, "needs_check");
    assert_validation_failed(fixture.validate(), "needs_check");
    redirect.finish();
    assert_eq!(redirect.requests.lock().unwrap().len(), 1);
    tls_endpoint.assert_certificate_rejected();
}

#[test]
fn authenticated_validation_success_still_replays_scoped_synthetic_credentials() {
    let mut endpoint = Endpoint::start(false, ok_response());
    let fixture = ValidationFixture::new(&endpoint.url, "needs_check");
    let result = fixture.validate();
    endpoint.finish();
    assert_eq!(result.0, "valid");
    assert_ne!(result.1, LAST_VALIDATED);
    assert!(chrono::DateTime::parse_from_rfc3339(&result.1).is_ok());
    assert!(result.2.is_empty());
    let requests = endpoint.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_ascii_lowercase();
    assert!(request.starts_with("get /account/profile http/1.1\r\n"));
    assert!(request.contains("authorization: bearer synthetic-auth-token\r\n"));
    assert!(request.contains("x-api-key: synthetic-api-key\r\n"));
    assert!(request.contains("cookie: session=synthetic-cookie\r\n"));
    assert!(!request.contains("must-not-send"));
}
