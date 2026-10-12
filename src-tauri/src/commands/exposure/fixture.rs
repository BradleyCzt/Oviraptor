// Test-only localhost fixture: a scripted HTTP responder that counts what actually left
// the process, because §Q2 asks for "refused before the send" to be measurable rather than
// asserted from a mock that never touches a socket.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Fixture {
    pub origin: String,
    pub port: u16,
    /// Every request line the fixture actually received, in arrival order.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// G2: the full request head (request line + headers) of every request, so a test can
    /// prove which credential header reached which origin.
    pub heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Maps a path to the raw HTTP responses to play for it; the last entry repeats.
pub fn routes(entries: Vec<(&str, Vec<String>)>) -> std::collections::HashMap<String, Vec<String>> {
    entries
        .into_iter()
        .map(|(path, rows)| (path.to_string(), rows))
        .collect()
}

/// `routes` maps a path to the raw HTTP responses to play for it; the last entry repeats.
pub fn response(status: u16, headers: &[(&str, &str)], body: &str) -> String {
    let reason = match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut text = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    text
}

/// FIX3-R4：夹具收到请求、回复之前调用的钩子。
pub type RequestHook = Box<dyn FnMut(&str) + Send>;

impl Fixture {
    pub fn start(routes: HashMap<String, Vec<String>>) -> Self {
        Self::start_on(0, routes)
    }

    /// G2: serve on a chosen port, for responses that must name the fixture's own origin
    /// (a Link header pointing at page 2). Port 0 picks a free one.
    pub fn start_on(port: u16, routes: HashMap<String, Vec<String>>) -> Self {
        Self::start_inner(port, routes, None)
    }

    /// FIX3-R4：每收到一个请求、回复之前先调用 `hook(path)`。测试用它在“请求已发出、响应未返回”
    /// 的时刻撤权，以确定性地复现在途请求。
    pub fn start_hooked(routes: HashMap<String, Vec<String>>, hook: RequestHook) -> Self {
        Self::start_inner(0, routes, Some(hook))
    }

    /// FIX4 P1／新增 T21：固定端口 + 钩子（第 1 页的 Link 要指向夹具自身）。
    pub fn start_hooked_on(port: u16, routes: HashMap<String, Vec<String>>, hook: RequestHook) -> Self {
        Self::start_inner(port, routes, Some(hook))
    }

    fn start_inner(port: u16, mut routes: HashMap<String, Vec<String>>, mut hook: Option<RequestHook>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("测试监听必须可用");
        let port = listener.local_addr().expect("测试端口").port();
        // A nonblocking accept is what lets the loop notice `stop` instead of hanging.
        listener.set_nonblocking(true).expect("非阻塞监听设置");
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let heads: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let server_heads = Arc::clone(&heads);
        let server_seen = Arc::clone(&seen);
        let server_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accepted sockets inherit the listener flags on some platforms;
                        // the responder needs a blocking read with a bounded wait.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let Some((path, head)) = read_request(&mut stream) else {
                            continue;
                        };
                        server_heads.lock().map(|mut rows| rows.push(head)).ok();
                        server_seen
                            .lock()
                            .map(|mut rows| rows.push(path.clone()))
                            .ok();
                        if let Some(hook) = hook.as_mut() {
                            hook(&path);
                        }
                        let key = path.split('?').next().unwrap_or(&path).to_string();
                        let fallback = if key.starts_with("/any") {
                            "/any".to_string()
                        } else {
                            key.clone()
                        };
                        let target = if routes.contains_key(&key) {
                            key
                        } else {
                            fallback
                        };
                        let Some(queue) = routes.get_mut(&target) else {
                            let _ = stream.write_all(response(404, &[], "not found").as_bytes());
                            let _ = stream.flush();
                            continue;
                        };
                        let reply = if queue.len() > 1 {
                            queue.remove(0)
                        } else {
                            queue[0].clone()
                        };
                        let _ = stream.write_all(reply.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if server_stop.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            origin: format!("http://127.0.0.1:{port}"),
            port,
            seen,
            heads,
            stop,
            thread: Some(thread),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin, path)
    }

    pub fn request_count(&self) -> usize {
        self.seen.lock().map(|rows| rows.len()).unwrap_or_default()
    }

    pub fn heads(&self) -> Vec<String> {
        self.heads.lock().map(|rows| rows.clone()).unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<String> {
        self.seen
            .lock()
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buffer).to_string();
    let first = text.lines().next()?;
    let mut parts = first.split_whitespace();
    let _method = parts.next()?;
    let path = parts.next()?.to_string();
    let head = text.split("\r\n\r\n").next().unwrap_or_default().to_string();
    Some((path, head))
}

/// A project with one authorized asset origin, ready for `ExposureScope::load`.
pub fn seed_scope(
    connection: &rusqlite::Connection,
    project_id: i64,
    origin: &str,
    eligible: bool,
) {
    use rusqlite::params;
    connection
        .execute(
            "INSERT OR IGNORE INTO projects(id,name,status) VALUES(?1,?2,'active')",
            params![project_id, format!("fixture-{project_id}")],
        )
        .unwrap();
    let asset_id = connection
        .query_row(
            "SELECT id FROM assets WHERE asset_key=?1",
            [origin],
            |row| row.get::<_, i64>(0),
        )
        .ok();
    let asset_id = match asset_id {
        Some(existing) => existing,
        None => {
            connection
                .execute(
                    "INSERT INTO assets(asset_key,link,host) VALUES(?1,?1,'127.0.0.1')",
                    [origin],
                )
                .unwrap();
            connection.last_insert_rowid()
        }
    };
    connection
        .execute(
            "INSERT OR IGNORE INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
            params![project_id, asset_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT OR IGNORE INTO asset_ownership_decisions(project_id,asset_id,status,authorization_status,exposure_eligible,confidence)
             VALUES(?1,?2,'confirmed','allowed',?3,90)",
            params![project_id, asset_id, i64::from(eligible)],
        )
        .unwrap();
    connection
        .execute(
            "INSERT OR IGNORE INTO asset_ownership_profiles(project_id,legal_name,policy_version) VALUES(?1,'样本公司',1)",
            [project_id],
        )
        .unwrap();
}
