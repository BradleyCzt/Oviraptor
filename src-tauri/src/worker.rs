use crate::{commands, db, models::SentinelScan, AppState};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const DEFAULT_WORKER_PORT: u16 = 19427;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024 * 1024;

struct WorkerServiceHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct WorkerServiceControl {
    service: Mutex<Option<WorkerServiceHandle>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalWorkerSettings {
    enabled: bool,
    port: u16,
    access_token: String,
    tailscale_ip: String,
    endpoint: String,
    running: bool,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalWorkerSettingsInput {
    enabled: bool,
    port: u16,
    access_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerNode {
    id: i64,
    name: String,
    endpoint: String,
    access_token: String,
    enabled: bool,
    last_seen_at: Option<String>,
    last_sync_at: Option<String>,
    last_error: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerNodeInput {
    id: Option<i64>,
    name: String,
    endpoint: String,
    access_token: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerControlInput {
    node_id: i64,
    scan_id: String,
    action: String,
}

fn setting(connection: &rusqlite::Connection, key: &str, fallback: &str) -> String {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| fallback.to_string())
}

fn save_setting(connection: &rusqlite::Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn generate_access_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn tailscale_candidates() -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            "tailscale.exe".into(),
            r"C:\Program Files\Tailscale\tailscale.exe".into(),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "tailscale".into(),
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale".into(),
            "/usr/local/bin/tailscale".into(),
            "/opt/homebrew/bin/tailscale".into(),
        ]
    } else {
        vec!["tailscale".into(), "/usr/bin/tailscale".into()]
    }
}

pub fn detect_tailscale_ip() -> Option<String> {
    tailscale_candidates().into_iter().find_map(|candidate| {
        let output = Command::new(candidate).args(["ip", "-4"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("100."))
            .map(str::to_string)
    })
}

fn worker_config(state: &AppState) -> Result<(bool, u16, String), String> {
    let connection = db::open(&state.db_path)?;
    let enabled = setting(&connection, "worker_enabled", "false") == "true";
    let port = setting(&connection, "worker_port", &DEFAULT_WORKER_PORT.to_string())
        .parse::<u16>()
        .unwrap_or(DEFAULT_WORKER_PORT);
    let mut token = setting(&connection, "worker_access_token", "");
    if token.trim().is_empty() {
        token = generate_access_token();
        save_setting(&connection, "worker_access_token", &token)?;
    }
    Ok((enabled, port, token))
}

fn write_json_response(stream: &mut TcpStream, status: u16, value: Value) {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

/// method, path, headers, body
type ParsedHttpRequest = (String, String, HashMap<String, String>, Vec<u8>);

fn parse_http_request(stream: &mut TcpStream) -> Result<ParsedHttpRequest, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end = loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("连接在请求完成前关闭".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("请求过大".into());
        }
        if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or_else(|| "缺少请求行".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("").to_string();
    let path = request_parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BYTES {
        return Err("请求正文过大".into());
    }
    while bytes.len() < header_end + content_length {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok((
        method,
        path,
        headers,
        bytes[header_end..bytes.len().min(header_end + content_length)].to_vec(),
    ))
}

fn worker_health(state: &AppState) -> Result<Value, String> {
    let connection = db::open(&state.db_path)?;
    let running_scans: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE status IN ('queued','scanning','pausing')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let completed_scans: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE status='completed'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Oviraptor Worker".into());
    Ok(json!({
        "service": "oviraptor-worker",
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": hostname,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tailscaleIp": detect_tailscale_ip().unwrap_or_default(),
        "runningScans": running_scans,
        "completedScans": completed_scans,
        "checkedAt": chrono::Utc::now().to_rfc3339(),
    }))
}

fn worker_projects(state: &AppState) -> Result<Value, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT p.id,p.name,CASE WHEN MAX(s.updated_at)>p.updated_at THEN MAX(s.updated_at) ELSE p.updated_at END,COUNT(s.id) FROM projects p JOIN sentinel_scans s ON s.project_id=p.id GROUP BY p.id,p.name,p.updated_at ORDER BY 3 DESC,p.id DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "name": row.get::<_, String>(1)?,
                "updatedAt": row.get::<_, String>(2)?,
                "scanCount": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(Value::Array(rows))
}

fn handle_worker_request(app: &AppHandle, mut stream: TcpStream, token: &str) {
    let request = parse_http_request(&mut stream);
    let (method, raw_path, headers, _body) = match request {
        Ok(request) => request,
        Err(error) => {
            write_json_response(&mut stream, 400, json!({ "error": error }));
            return;
        }
    };
    let expected_authorization = format!("Bearer {token}");
    if headers.get("authorization").map(String::as_str) != Some(expected_authorization.as_str()) {
        write_json_response(&mut stream, 401, json!({ "error": "Worker 访问令牌无效" }));
        return;
    }
    let path = raw_path.split('?').next().unwrap_or("/");
    let state = app.state::<AppState>();
    let result: Result<Value, String> = match (method.as_str(), path) {
        ("GET", "/v1/health") => worker_health(&state),
        ("GET", "/v1/environment") => commands::environment::check_environment(app.clone(), state.clone(), None)
            .and_then(|report| serde_json::to_value(report).map_err(|error| error.to_string())),
        ("GET", "/v1/projects") => worker_projects(&state),
        ("GET", "/v1/scans") => {
            crate::scanning::worker_bridge::list_sentinel_scans_inner(&state.db_path, None, Some(500), None, None)
                .and_then(|items| serde_json::to_value(items).map_err(|error| error.to_string()))
        }
        _ => {
            let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
            if method == "GET"
                && parts.len() == 4
                && parts[0] == "v1"
                && parts[1] == "projects"
                && parts[3] == "bundle"
            {
                parts[2]
                    .parse::<i64>()
                    .map_err(|_| "项目 ID 无效".to_string())
                    .and_then(|project_id| crate::scanning::worker_bridge::sentinel_project_bundle(&state, project_id))
            } else if method == "POST"
                && parts.len() == 4
                && parts[0] == "v1"
                && parts[1] == "scans"
            {
                let scan_id = parts[2].to_string();
                match parts[3] {
                    "pause" => crate::scanning::worker_bridge::pause_sentinel_scan(state.clone(), scan_id)
                        .and_then(|item| serde_json::to_value(item).map_err(|e| e.to_string())),
                    "resume" => crate::scanning::worker_bridge::resume_sentinel_scan(app.clone(), state.clone(), scan_id)
                        .and_then(|item| serde_json::to_value(item).map_err(|e| e.to_string())),
                    "cancel" => crate::scanning::worker_bridge::cancel_sentinel_scan(state.clone(), scan_id)
                        .map(|_| json!({ "ok": true })),
                    _ => Err("不支持的任务操作".into()),
                }
            } else {
                Err("NOT_FOUND".into())
            }
        }
    };
    match result {
        Ok(value) => write_json_response(&mut stream, 200, value),
        Err(error) if error == "NOT_FOUND" => {
            write_json_response(&mut stream, 404, json!({ "error": "接口不存在" }))
        }
        Err(error) => write_json_response(&mut stream, 500, json!({ "error": error })),
    }
}

fn stop_worker_service(control: &WorkerServiceControl) {
    let old = control
        .service
        .lock()
        .ok()
        .and_then(|mut value| value.take());
    if let Some(mut service) = old {
        service.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = service.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn restart_worker_service(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    stop_worker_service(&state.worker_service);
    let (enabled, port, token) = worker_config(&state)?;
    if !enabled {
        return Ok(());
    }
    let tailscale_ip = detect_tailscale_ip()
        .ok_or_else(|| "未检测到 Tailscale IPv4；请先安装并登录 Tailscale".to_string())?;
    let listener = TcpListener::bind((tailscale_ip.as_str(), port))
        .map_err(|error| format!("Worker 无法监听 {tailscale_ip}:{port}：{error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread_app = app.clone();
    let worker_thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => handle_worker_request(&thread_app, stream, &token),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(120));
                }
                Err(_) => thread::sleep(Duration::from_millis(250)),
            }
        }
    });
    let mut service = state
        .worker_service
        .service
        .lock()
        .map_err(|_| "Worker 状态锁不可用".to_string())?;
    *service = Some(WorkerServiceHandle {
        stop,
        thread: Some(worker_thread),
    });
    Ok(())
}

#[tauri::command]
pub fn get_local_worker_settings(state: State<AppState>) -> Result<LocalWorkerSettings, String> {
    let (enabled, port, access_token) = worker_config(&state)?;
    let tailscale_ip = detect_tailscale_ip().unwrap_or_default();
    let running = state
        .worker_service
        .service
        .lock()
        .map(|service| service.is_some())
        .unwrap_or(false);
    let endpoint = if tailscale_ip.is_empty() {
        String::new()
    } else {
        format!("http://{tailscale_ip}:{port}")
    };
    let status = if running {
        "Worker 正在运行，可从 Tailnet 内访问".into()
    } else if !enabled {
        "Worker 模式未启用".into()
    } else if tailscale_ip.is_empty() {
        "未检测到 Tailscale；请先在本机安装并登录".into()
    } else {
        "Worker 未运行；请保存设置重试".into()
    };
    Ok(LocalWorkerSettings {
        enabled,
        port,
        access_token,
        tailscale_ip,
        endpoint,
        running,
        status,
    })
}

#[tauri::command]
pub fn save_local_worker_settings(
    app: AppHandle,
    state: State<AppState>,
    input: LocalWorkerSettingsInput,
) -> Result<LocalWorkerSettings, String> {
    if !(1024..=65535).contains(&input.port) {
        return Err("Worker 端口必须在 1024 到 65535 之间".into());
    }
    let token = if input.access_token.trim().len() < 32 {
        generate_access_token()
    } else {
        input.access_token.trim().to_string()
    };
    let connection = db::open(&state.db_path)?;
    save_setting(
        &connection,
        "worker_enabled",
        if input.enabled { "true" } else { "false" },
    )?;
    save_setting(&connection, "worker_port", &input.port.to_string())?;
    save_setting(&connection, "worker_access_token", &token)?;
    restart_worker_service(&app)?;
    get_local_worker_settings(state)
}

fn worker_node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteWorkerNode> {
    Ok(RemoteWorkerNode {
        id: row.get(0)?,
        name: row.get(1)?,
        endpoint: row.get(2)?,
        access_token: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        last_seen_at: row.get(5)?,
        last_sync_at: row.get(6)?,
        last_error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn list_worker_nodes_inner(db_path: &std::path::Path) -> Result<Vec<RemoteWorkerNode>, String> {
    let connection = db::open(db_path)?;
    let mut statement = connection
        .prepare("SELECT id,name,endpoint,access_token,enabled,last_seen_at,last_sync_at,last_error,created_at,updated_at FROM worker_nodes ORDER BY enabled DESC,updated_at DESC,id DESC")
        .map_err(|error| error.to_string())?;
    let nodes = statement
        .query_map([], worker_node_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(nodes)
}

#[tauri::command]
pub async fn list_worker_nodes(
    state: State<'_, AppState>,
) -> Result<Vec<RemoteWorkerNode>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || list_worker_nodes_inner(&db_path))
        .await
        .map_err(|error| format!("Worker 列表读取线程失败：{error}"))?
}

fn normalize_endpoint(value: &str) -> Result<String, String> {
    let endpoint = value.trim().trim_end_matches('/').to_string();
    if !endpoint.starts_with("http://") {
        return Err("节点地址必须以 http:// 开头；Tailnet 传输本身已加密".into());
    }
    if endpoint["http://".len()..].contains('/') {
        return Err("节点地址只填写主机和端口，例如 http://100.64.0.8:19427".into());
    }
    Ok(endpoint)
}

#[tauri::command]
pub fn save_worker_node(
    state: State<AppState>,
    input: RemoteWorkerNodeInput,
) -> Result<i64, String> {
    if input.name.trim().is_empty() {
        return Err("节点名称不能为空".into());
    }
    if input.access_token.trim().len() < 32 {
        return Err("请粘贴 Worker 页面显示的完整访问令牌".into());
    }
    let endpoint = normalize_endpoint(&input.endpoint)?;
    let connection = db::open(&state.db_path)?;
    if let Some(id) = input.id {
        let changed = connection
            .execute(
                "UPDATE worker_nodes SET name=?1,endpoint=?2,access_token=?3,enabled=?4,updated_at=datetime('now','localtime') WHERE id=?5",
                params![input.name.trim(), endpoint, input.access_token.trim(), input.enabled as i64, id],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("远程节点不存在".into());
        }
        Ok(id)
    } else {
        connection
            .execute(
                "INSERT INTO worker_nodes(name,endpoint,access_token,enabled) VALUES(?1,?2,?3,?4)",
                params![
                    input.name.trim(),
                    endpoint,
                    input.access_token.trim(),
                    input.enabled as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_worker_node(state: State<AppState>, node_id: i64) -> Result<(), String> {
    db::open(&state.db_path)?
        .execute("DELETE FROM worker_nodes WHERE id=?1", [node_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn worker_node(state: &AppState, node_id: i64) -> Result<RemoteWorkerNode, String> {
    db::open(&state.db_path)?
        .query_row(
            "SELECT id,name,endpoint,access_token,enabled,last_seen_at,last_sync_at,last_error,created_at,updated_at FROM worker_nodes WHERE id=?1",
            [node_id],
            worker_node_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "远程节点不存在".to_string())
}

fn parse_http_endpoint(endpoint: &str) -> Result<(String, u16), String> {
    let authority = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| "仅支持 Tailnet 内的 http:// 节点地址".to_string())?;
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| "节点地址缺少端口".to_string())?;
    let port = port
        .parse::<u16>()
        .map_err(|_| "节点端口无效".to_string())?;
    if host.trim().is_empty() {
        return Err("节点主机名为空".into());
    }
    Ok((host.to_string(), port))
}

fn remote_request(endpoint: &str, token: &str, method: &str, path: &str) -> Result<Value, String> {
    let (host, port) = parse_http_endpoint(endpoint)?;
    let address = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("无法解析 Worker 地址：{error}"))?
        .next()
        .ok_or_else(|| "无法解析 Worker 地址".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(12))
        .map_err(|error| format!("无法连接 Worker：{error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| error.to_string())?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("发送 Worker 请求失败：{error}"))?;
    let mut response = Vec::new();
    let mut buffer = [0u8; 16384];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("读取 Worker 响应失败：{error}"))?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..count]);
        if response.len() > MAX_RESPONSE_BYTES {
            return Err("Worker 响应超过 512 MB".into());
        }
    }
    let header_end = response
        .windows(4)
        .position(|item| item == b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or_else(|| "Worker 返回的 HTTP 响应无效".to_string())?;
    let header = String::from_utf8_lossy(&response[..header_end]);
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(500);
    let body: Value = serde_json::from_slice(&response[header_end..])
        .map_err(|error| format!("Worker 返回的 JSON 无效：{error}"))?;
    if !(200..300).contains(&status) {
        return Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Worker 请求失败")
            .to_string());
    }
    Ok(body)
}

fn update_node_result(state: &AppState, node_id: i64, error: Option<&str>) {
    if let Ok(connection) = db::open(&state.db_path) {
        if let Some(error) = error {
            let _ = connection.execute(
                "UPDATE worker_nodes SET last_error=?1,updated_at=datetime('now','localtime') WHERE id=?2",
                params![error, node_id],
            );
        } else {
            let _ = connection.execute(
                "UPDATE worker_nodes SET last_seen_at=datetime('now','localtime'),last_error='',updated_at=datetime('now','localtime') WHERE id=?1",
                [node_id],
            );
        }
    }
}

fn test_worker_node_inner(state: State<AppState>, node_id: i64) -> Result<Value, String> {
    let node = worker_node(&state, node_id)?;
    match remote_request(&node.endpoint, &node.access_token, "GET", "/v1/health") {
        Ok(value) => {
            update_node_result(&state, node_id, None);
            Ok(value)
        }
        Err(error) => {
            update_node_result(&state, node_id, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn test_worker_node(app: AppHandle, node_id: i64) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || test_worker_node_inner(app.state(), node_id))
        .await
        .map_err(|error| format!("Worker 检测线程失败：{error}"))?
}

fn list_remote_worker_scans_inner(
    state: State<AppState>,
    node_id: i64,
) -> Result<Vec<SentinelScan>, String> {
    let node = worker_node(&state, node_id)?;
    let result = remote_request(&node.endpoint, &node.access_token, "GET", "/v1/scans");
    match result {
        Ok(value) => {
            let scans =
                serde_json::from_value(value).map_err(|error| format!("解析任务失败：{error}"))?;
            update_node_result(&state, node_id, None);
            Ok(scans)
        }
        Err(error) => {
            update_node_result(&state, node_id, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn list_remote_worker_scans(
    app: AppHandle,
    node_id: i64,
) -> Result<Vec<SentinelScan>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        list_remote_worker_scans_inner(app.state(), node_id)
    })
    .await
    .map_err(|error| format!("Worker 任务读取线程失败：{error}"))?
}

fn get_remote_worker_environment_inner(
    state: State<AppState>,
    node_id: i64,
) -> Result<Value, String> {
    let node = worker_node(&state, node_id)?;
    let result = remote_request(&node.endpoint, &node.access_token, "GET", "/v1/environment");
    match result {
        Ok(value) => {
            update_node_result(&state, node_id, None);
            Ok(value)
        }
        Err(error) => {
            update_node_result(&state, node_id, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn get_remote_worker_environment(app: AppHandle, node_id: i64) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        get_remote_worker_environment_inner(app.state(), node_id)
    })
    .await
    .map_err(|error| format!("Worker 环境读取线程失败：{error}"))?
}

fn control_remote_worker_scan_inner(
    state: State<AppState>,
    input: RemoteWorkerControlInput,
) -> Result<Value, String> {
    if !matches!(input.action.as_str(), "pause" | "resume" | "cancel") {
        return Err("不支持的远程任务操作".into());
    }
    let node = worker_node(&state, input.node_id)?;
    let path = format!("/v1/scans/{}/{}", input.scan_id, input.action);
    let result = remote_request(&node.endpoint, &node.access_token, "POST", &path);
    match result {
        Ok(value) => {
            update_node_result(&state, input.node_id, None);
            Ok(value)
        }
        Err(error) => {
            update_node_result(&state, input.node_id, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn control_remote_worker_scan(
    app: AppHandle,
    input: RemoteWorkerControlInput,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        control_remote_worker_scan_inner(app.state(), input)
    })
    .await
    .map_err(|error| format!("Worker 控制线程失败：{error}"))?
}

fn sync_worker_node_inner(state: State<AppState>, node_id: i64) -> Result<i64, String> {
    let node = worker_node(&state, node_id)?;
    let result: Result<i64, String> = (|| {
        let projects = remote_request(&node.endpoint, &node.access_token, "GET", "/v1/projects")?;
        let projects = projects
            .as_array()
            .ok_or_else(|| "Worker 项目列表格式无效".to_string())?;
        let mut imported = 0i64;
        let newest_remote_update = projects
            .iter()
            .filter_map(|project| project.get("updatedAt").and_then(Value::as_str))
            .max()
            .map(str::to_string);
        for project in projects.iter().filter(|project| {
            let updated_at = project
                .get("updatedAt")
                .and_then(Value::as_str)
                .unwrap_or("");
            node.last_sync_at
                .as_deref()
                .is_none_or(|last_sync| updated_at > last_sync)
        }) {
            let project_id = project
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "Worker 项目缺少 ID".to_string())?;
            let bundle = remote_request(
                &node.endpoint,
                &node.access_token,
                "GET",
                &format!("/v1/projects/{project_id}/bundle"),
            )?;
            // Remote exports have no local execution authority. Feed the shared
            // historical importer directly, without a plaintext temporary file.
            imported += crate::scanning::worker_bridge::import_sentinel_snapshot_bytes(
                &state.db_path,
                &state.app_data_dir,
                &serde_json::to_vec(&bundle).map_err(|error| error.to_string())?,
                None,
            )?;
        }
        if let Some(updated_at) = newest_remote_update {
            db::open(&state.db_path)?
                .execute(
                    "UPDATE worker_nodes SET last_sync_at=?1 WHERE id=?2",
                    params![updated_at, node_id],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(imported)
    })();
    match result {
        Ok(count) => {
            update_node_result(&state, node_id, None);
            Ok(count)
        }
        Err(error) => {
            update_node_result(&state, node_id, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn sync_worker_node(app: AppHandle, node_id: i64) -> Result<i64, String> {
    tauri::async_runtime::spawn_blocking(move || sync_worker_node_inner(app.state(), node_id))
        .await
        .map_err(|error| format!("Worker 同步线程失败：{error}"))?
}
