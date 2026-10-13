use crate::{db, models::BrowserAuthSession, models::BrowserAuthSessionInput, AppState};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, COOKIE};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::mpsc, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

const AUTH_CAPTURE_SCRIPT: &str = r#"
(() => {
  if (window.__OVIRAPTOR_AUTH_CAPTURE__) return;
  const state = { requests: [], installedAt: new Date().toISOString() };
  const safeHeaders = (input) => {
    const result = {};
    try {
      new Headers(input || {}).forEach((value, key) => {
        if (String(value).length <= 8192) result[String(key).toLowerCase()] = String(value);
      });
    } catch (_) {}
    return result;
  };
  const push = (record) => {
    try {
      state.requests.push({ ...record, at: new Date().toISOString() });
      if (state.requests.length > 240) state.requests.splice(0, state.requests.length - 240);
    } catch (_) {}
  };
  const originalFetch = window.fetch;
  if (typeof originalFetch === 'function') {
    window.fetch = function(input, init = {}) {
      const method = String(init.method || (input && input.method) || 'GET').toUpperCase();
      const url = String((input && input.url) || input || '');
      const headers = { ...safeHeaders(input && input.headers), ...safeHeaders(init.headers) };
      const started = Date.now();
      return originalFetch.apply(this, arguments).then((response) => {
        push({ transport: 'fetch', method, url: response.url || url, headers, status: response.status, durationMs: Date.now() - started });
        return response;
      }, (error) => {
        push({ transport: 'fetch', method, url, headers, status: 0, error: String(error), durationMs: Date.now() - started });
        throw error;
      });
    };
  }
  const originalOpen = XMLHttpRequest.prototype.open;
  const originalSetHeader = XMLHttpRequest.prototype.setRequestHeader;
  const originalSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function(method, url) {
    this.__oviraptorMeta = { transport: 'xhr', method: String(method || 'GET').toUpperCase(), url: String(url || ''), headers: {}, started: Date.now() };
    return originalOpen.apply(this, arguments);
  };
  XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
    if (this.__oviraptorMeta && String(value).length <= 8192) this.__oviraptorMeta.headers[String(name).toLowerCase()] = String(value);
    return originalSetHeader.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function() {
    if (this.__oviraptorMeta) {
      this.addEventListener('loadend', () => push({ ...this.__oviraptorMeta, url: this.responseURL || this.__oviraptorMeta.url, status: this.status, durationMs: Date.now() - this.__oviraptorMeta.started }), { once: true });
    }
    return originalSend.apply(this, arguments);
  };
  Object.defineProperty(window, '__OVIRAPTOR_AUTH_CAPTURE__', { value: state, configurable: false });
})();
"#;

const AUTH_SNAPSHOT_SCRIPT: &str = r#"
(() => {
  const readStorage = (storage) => {
    const result = {};
    try {
      for (let index = 0; index < Math.min(storage.length, 96); index += 1) {
        const key = storage.key(index);
        if (!key || key.length > 512) continue;
        const value = storage.getItem(key);
        if (typeof value === 'string' && value.length <= 16384) result[key] = value;
      }
    } catch (_) {}
    return result;
  };
  return {
    url: location.href,
    title: document.title,
    localStorage: readStorage(window.localStorage),
    sessionStorage: readStorage(window.sessionStorage),
    requests: (window.__OVIRAPTOR_AUTH_CAPTURE__ && window.__OVIRAPTOR_AUTH_CAPTURE__.requests) || []
  };
})()
"#;

fn auth_session_row(row: &Row<'_>) -> rusqlite::Result<BrowserAuthSession> {
    let scope_text: String = row.get(8)?;
    let mut status: String = row.get(7)?;
    let expires_at: String = row.get(14)?;
    if status == "valid"
        && !chrono::DateTime::parse_from_rfc3339(&expires_at)
            .is_ok_and(|expires| expires > Utc::now())
    {
        status = "expired".into();
    }
    Ok(BrowserAuthSession {
        id: row.get(0)?,
        project_id: row.get(1)?,
        owner_scan_id: row.get(2)?,
        draft_scope_id: row.get(3)?,
        name: row.get(4)?,
        entry_url: row.get(5)?,
        final_url: row.get(6)?,
        status,
        scope_hosts: serde_json::from_str(&scope_text).unwrap_or_default(),
        cookie_count: row.get(9)?,
        header_count: row.get(10)?,
        storage_count: row.get(11)?,
        captured_request_count: row.get(12)?,
        last_validated_at: row.get(13)?,
        expires_at,
        last_error: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

const SESSION_COLUMNS: &str = "id,project_id,owner_scan_id,draft_scope_id,name,entry_url,final_url,status,scope_hosts_json,cookie_count,header_count,storage_count,captured_request_count,last_validated_at,expires_at,last_error,created_at,updated_at";

/// Operator inspection of the identities explicitly bound to this task. This
/// snapshot grants no execution permission and does not refresh expired sessions.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIdentityMaterial {
    session: BrowserAuthSession,
    document: Value,
}

#[tauri::command]
pub fn get_sentinel_scan_identity_material(
    state: State<'_, AppState>,
    project_id: i64,
    scan_id: String,
) -> Result<Vec<ScanIdentityMaterial>, String> {
    let connection = db::open(&state.db_path)?;
    scan_identity_material_on(&connection, project_id, &scan_id)
}

fn scan_identity_material_on(
    connection: &rusqlite::Connection,
    project_id: i64,
    scan_id: &str,
) -> Result<Vec<ScanIdentityMaterial>, String> {
    // Keep policy, ownership and bytes in one read snapshot. Never accept a
    // caller-provided identity ID or read all identities from a project.
    let tx = connection.unchecked_transaction().map_err(|e| e.to_string())?;
    let policy_text: String = tx.query_row(
        "SELECT context.policy_json FROM sentinel_scans scan JOIN sentinel_scan_contexts context ON context.scan_id=scan.id WHERE scan.id=?1 AND scan.project_id=?2",
        params![scan_id, project_id], |row| row.get(0),
    ).map_err(|_| "identity_material_task_scope_mismatch")?;
    let policy: Value = serde_json::from_str(&policy_text)
        .map_err(|_| "identity_material_policy_invalid")?;
    let ids = auth_session_ids_from_policy(&policy);
    if ids.len() > 16 { return Err("identity_material_count_exceeded".into()); }
    let mut total_bytes = 0usize;
    let mut result = Vec::new();
    for id in ids {
        let session = session_by_id(&tx, &id)?;
        if session.project_id != project_id || session.owner_scan_id != scan_id {
            return Err("identity_material_owner_scope_mismatch".into());
        }
        let text: String = tx.query_row(
            "SELECT session_json FROM browser_auth_sessions WHERE id=?1 AND project_id=?2 AND owner_scan_id=?3",
            params![id, project_id, scan_id], |row| row.get(0),
        ).map_err(|_| "identity_material_owner_scope_mismatch")?;
        total_bytes = total_bytes.checked_add(text.len()).ok_or("identity_material_size_exceeded")?;
        if text.len() > 256 * 1024 || total_bytes > 1024 * 1024 {
            return Err("identity_material_size_exceeded".into());
        }
        let document: Value = serde_json::from_str(&text)
            .map_err(|_| "identity_material_document_invalid")?;
        if !document.is_object() { return Err("identity_material_document_invalid".into()); }
        result.push(ScanIdentityMaterial { session, document });
    }
    // A read-only transaction deliberately ends without writing any state.
    Ok(result)
}

fn session_by_id(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<BrowserAuthSession, String> {
    connection
        .query_row(
            &format!("SELECT {SESSION_COLUMNS} FROM browser_auth_sessions WHERE id=?1"),
            [id],
            auth_session_row,
        )
        .map_err(|_| "登录会话不存在或已删除".to_string())
}

pub(crate) fn auth_session_ids_from_policy(policy: &Value) -> Vec<String> {
    let mut ids = policy
        .get("authSessionIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(id) = policy
        .get("authSessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        ids.push(id.to_string());
    }
    ids.sort();
    ids.dedup();
    ids
}

fn parse_http_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "登录地址不是有效 URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("登录地址必须是带主机名的 http:// 或 https:// URL".into());
    }
    Ok(url)
}

fn window_label(id: &str) -> String {
    format!("oviraptor-auth-{}", id.replace('-', ""))
}

fn capture_restore_status(value: &str) -> &'static str {
    match value {
        "valid" => "valid",
        "invalid" => "invalid",
        "expired" => "expired",
        _ => "needs_check",
    }
}

fn restore_cancelled_capture(
    connection: &rusqlite::Connection,
    session_id: &str,
    reason: &str,
) -> Result<Option<BrowserAuthSession>, String> {
    let state: Option<(String, String)> = connection
        .query_row(
            "SELECT status,capture_previous_status FROM browser_auth_sessions WHERE id=?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((status, previous)) = state else {
        return Ok(None);
    };
    if status != "capturing" {
        return Ok(None);
    }
    connection
        .execute(
            "UPDATE browser_auth_sessions SET status=?1,capture_previous_status='',last_error=?2,updated_at=datetime('now','localtime') WHERE id=?3 AND status='capturing'",
            params![capture_restore_status(&previous), reason, session_id],
        )
        .map_err(|error| error.to_string())?;
    session_by_id(connection, session_id).map(Some)
}

/// Closing the dedicated login WebView is a cancellation, not a failed
/// authentication attempt. Restore the state that existed before capture so
/// the same session can be validated or reopened without recreating the task.
pub(crate) fn browser_auth_window_closed(app: &AppHandle, label: &str) {
    if !label.starts_with("oviraptor-auth-") {
        return;
    }
    let state = app.state::<AppState>();
    let Ok(connection) = db::open(&state.db_path) else {
        return;
    };
    let session_ids = {
        let Ok(mut statement) =
            connection.prepare("SELECT id FROM browser_auth_sessions WHERE status='capturing'")
        else {
            return;
        };
        let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
            return;
        };
        rows.filter_map(Result::ok).collect::<Vec<_>>()
    };
    let Some(session_id) = session_ids
        .into_iter()
        .find(|session_id| window_label(session_id) == label)
    else {
        return;
    };
    if let Ok(Some(session)) = restore_cancelled_capture(
        &connection,
        &session_id,
        "登录窗口已关闭，本次捕获已取消；原会话未被判定失效，可重新打开或校验当前会话",
    ) {
        let _ = app.emit("browser-auth-session-updated", &session);
    }
}

#[cfg(not(target_os = "macos"))]
fn auth_profile_directory(app_data_dir: &Path, id: &str) -> std::path::PathBuf {
    app_data_dir.join("browser-auth-profiles").join(id)
}

#[cfg(target_os = "macos")]
fn macos_supports_isolated_data_store() -> bool {
    std::process::Command::new("/usr/bin/sw_vers")
        .args(["-productVersion"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|version| version.split('.').next()?.trim().parse::<u64>().ok())
        .is_some_and(|major| major >= 14)
}

fn token_like_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "auth",
        "token",
        "session",
        "jwt",
        "credential",
        "ticket",
        "login",
        "sso",
        "sid",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn auth_identity_fingerprint(document: &Value) -> Option<String> {
    let mut strong_material = Vec::new();
    let mut fallback_cookies = Vec::new();
    if let Some(cookies) = document.get("cookies").and_then(Value::as_array) {
        for cookie in cookies {
            let name = cookie.get("name").and_then(Value::as_str).unwrap_or("");
            let value = cookie.get("value").and_then(Value::as_str).unwrap_or("");
            if value.is_empty() {
                continue;
            }
            let record = format!(
                "cookie|{}|{}|{}|{}",
                cookie.get("domain").and_then(Value::as_str).unwrap_or(""),
                cookie.get("path").and_then(Value::as_str).unwrap_or("/"),
                name.to_ascii_lowercase(),
                value
            );
            if token_like_name(name) {
                strong_material.push(record.clone());
            }
            fallback_cookies.push(record);
        }
    }
    if let Some(headers) = document.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
                strong_material.push(format!("header|{}|{}", name.to_ascii_lowercase(), value));
            }
        }
    }
    for storage_name in ["localStorage", "sessionStorage"] {
        if let Some(storage) = document.get(storage_name).and_then(Value::as_object) {
            for (name, value) in storage {
                if token_like_name(name) {
                    if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
                        strong_material.push(format!(
                            "{}|{}|{}",
                            storage_name.to_ascii_lowercase(),
                            name.to_ascii_lowercase(),
                            value
                        ));
                    }
                }
            }
        }
    }
    let mut material = if strong_material.is_empty() {
        fallback_cookies
    } else {
        strong_material
    };
    material.sort();
    material.dedup();
    if material.is_empty() {
        return None;
    }
    let digest = Sha256::digest(material.join("\n").as_bytes());
    Some(format!("{digest:x}"))
}

pub(crate) fn distinct_session_documents_for_scan(
    connection: &rusqlite::Connection,
    session_ids: &[String],
    project_id: i64,
) -> Result<Vec<Value>, String> {
    let mut documents = Vec::new();
    let mut fingerprints = std::collections::HashMap::<String, String>::new();
    let mut common_hosts: Option<BTreeSet<String>> = None;
    for session_id in session_ids {
        let document = session_document_for_scan(connection, session_id, project_id)?;
        let fingerprint = auth_identity_fingerprint(&document).ok_or_else(|| {
            format!(
                "登录身份“{}”没有可比较的认证材料，请重新登录并完成捕获",
                value_name(&document)
            )
        })?;
        if let Some(existing_name) = fingerprints.insert(fingerprint, value_name(&document)) {
            return Err(format!(
                "登录身份“{}”与“{}”使用了相同认证材料，不能作为两个 IDOR 对照身份；请为每个账户打开独立登录窗口重新捕获",
                existing_name,
                value_name(&document)
            ));
        }
        let hosts = document
            .get("scopeHosts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|host| host.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        common_hosts = Some(match common_hosts {
            Some(current) => current.intersection(&hosts).cloned().collect(),
            None => hosts,
        });
        documents.push(document);
    }
    if documents.len() > 1 && common_hosts.as_ref().is_none_or(BTreeSet::is_empty) {
        return Err("所选登录身份没有共同作用域，无法对同一目标执行 IDOR 身份差异验证".into());
    }
    Ok(documents)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanIdentityMode {
    AnonymousOnly,
    SingleIdentity,
    IdentitySet,
}

/// Resolve task-bound identity handles at execution time. A name or an extra
/// policy ID is not proof of another account; only distinct captured material
/// on the actual target host can enable an identity comparison.
pub(crate) fn validated_scan_identities(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
) -> Result<(ScanIdentityMode, Vec<String>), String> {
    let (project_id, policy_text): (i64, String) = connection
        .query_row(
            "SELECT s.project_id,COALESCE(c.policy_json,'{}') FROM sentinel_scans s \
             LEFT JOIN sentinel_scan_contexts c ON c.scan_id=s.id WHERE s.id=?1",
            [scan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "扫描任务不存在，不能解析登录身份".to_string())?;
    let policy: Value = serde_json::from_str(&policy_text)
        .map_err(|_| "扫描身份策略损坏，不能解析登录身份".to_string())?;
    let ids = auth_session_ids_from_policy(&policy);
    if ids.is_empty() {
        return Ok((ScanIdentityMode::AnonymousOnly, Vec::new()));
    }
    let target_host = parse_http_url(target_url)?
        .host_str()
        .ok_or_else(|| "目标 URL 缺少主机名".to_string())?
        .to_ascii_lowercase();
    for id in &ids {
        let owner_scan_id: String = connection
            .query_row(
                "SELECT owner_scan_id FROM browser_auth_sessions WHERE id=?1 AND project_id=?2",
                params![id, project_id],
                |row| row.get(0),
            )
            .map_err(|_| "任务引用了不存在或其他工作空间的登录会话".to_string())?;
        if owner_scan_id != scan_id {
            return Err("登录身份未绑定当前扫描任务，不能跨任务复用".into());
        }
    }
    let documents = distinct_session_documents_for_scan(connection, &ids, project_id)?;
    for document in &documents {
        let in_scope = document
            .get("scopeHosts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|host| {
                host.trim_start_matches('.')
                    .eq_ignore_ascii_case(&target_host)
            });
        if !in_scope {
            return Err("登录身份未捕获当前目标主机，不能用于此目标的身份对照".into());
        }
    }
    let mode = if ids.len() == 1 {
        ScanIdentityMode::SingleIdentity
    } else {
        ScanIdentityMode::IdentitySet
    };
    Ok((mode, ids))
}

fn value_name(document: &Value) -> String {
    document
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| document.get("id").and_then(Value::as_str))
        .unwrap_or("未命名身份")
        .to_string()
}

fn login_like(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    [
        "/login",
        "/signin",
        "/sign-in",
        "/auth/login",
        "/passport/login",
        "cas/login",
        "oauth/authorize",
        "sso/login",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

fn host_from(value: &str) -> Option<String> {
    Url::parse(value).ok().and_then(|url| {
        url.host_str()
            .map(|host| host.trim_start_matches('.').to_ascii_lowercase())
    })
}

fn reusable_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "authorization"
            | "x-api-key"
            | "api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "x-xsrf-token"
    ) || (lower.starts_with("x-")
        && ![
            "x-request-id",
            "x-trace-id",
            "x-correlation-id",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
        ]
        .contains(&lower.as_str()))
}

fn normalize_snapshot(raw: String) -> Value {
    let first = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);
    if let Some(text) = first.as_str() {
        serde_json::from_str(text).unwrap_or(Value::Null)
    } else {
        first
    }
}

fn map_len(value: &Value) -> usize {
    value.as_object().map(Map::len).unwrap_or(0)
}

fn has_token_storage(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        map.keys().any(|key| {
            let lower = key.to_ascii_lowercase();
            ["token", "auth", "session", "jwt", "credential"]
                .iter()
                .any(|needle| lower.contains(needle))
        })
    })
}

#[tauri::command]
pub async fn open_browser_auth_session(
    app: AppHandle,
    state: State<'_, AppState>,
    input: BrowserAuthSessionInput,
) -> Result<BrowserAuthSession, String> {
    let entry_url = parse_http_url(&input.entry_url)?;
    let draft_scope_id = input.draft_scope_id.trim().to_string();
    let scan_id = input.scan_id.trim().to_string();
    for (label, value) in [
        ("登录会话草稿作用域", &draft_scope_id),
        ("扫描任务", &scan_id),
    ] {
        if !value.is_empty() && Uuid::parse_str(value).is_err() {
            return Err(format!("{label} ID 非法"));
        }
    }
    if draft_scope_id.is_empty() && scan_id.is_empty() {
        return Err("登录会话必须属于当前任务草稿或一个已存在的扫描任务".into());
    }
    let connection = db::open(&state.db_path)?;
    let project_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
            [input.project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !project_exists {
        return Err("工作空间不存在或已归档；恢复后才能建立登录会话".into());
    }
    let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    if Uuid::parse_str(&id).is_err() {
        return Err("登录会话 ID 非法".into());
    }
    let label = window_label(&id);
    let default_name = entry_url
        .host_str()
        .map(|host| format!("{host} 登录会话"))
        .unwrap_or_else(|| "浏览器登录会话".into());
    let name = if input.name.trim().is_empty() {
        default_name
    } else {
        input.name.trim().chars().take(100).collect()
    };
    if !scan_id.is_empty() {
        let scan_matches_project: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND project_id=?2)",
                params![scan_id, input.project_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !scan_matches_project {
            return Err("登录会话要绑定的扫描任务不存在或不属于当前工作空间".into());
        }
    }
    let exists: Option<(i64, String, String)> = connection
        .query_row(
            "SELECT project_id,owner_scan_id,draft_scope_id FROM browser_auth_sessions WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((project_id, owner_scan_id, existing_scope_id)) = exists.as_ref() {
        if *project_id != input.project_id {
            return Err("登录会话不属于当前工作空间".into());
        }
        if !owner_scan_id.is_empty() && owner_scan_id != &scan_id {
            return Err(
                "该登录会话属于另一个扫描任务，不能跨任务复用；请为当前任务重新登录".into(),
            );
        }
        if owner_scan_id.is_empty()
            && !existing_scope_id.is_empty()
            && existing_scope_id != &draft_scope_id
        {
            return Err("该登录会话属于另一个尚未提交的任务，不能跨任务复用".into());
        }
        if owner_scan_id.is_empty() && existing_scope_id.is_empty() {
            let legacy_bound = !scan_id.is_empty()
                && connection
                    .query_row(
                        "SELECT COALESCE(json_extract(policy_json,'$.authSessionId'),'')=?2 OR EXISTS(SELECT 1 FROM json_each(COALESCE(json_extract(policy_json,'$.authSessionIds'),'[]')) WHERE value=?2) FROM sentinel_scan_contexts WHERE scan_id=?1",
                        params![scan_id, id],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap_or(false);
            if !legacy_bound {
                return Err(
                    "这是旧版遗留的项目级会话，不能绑定新任务；请在当前任务重新登录".into(),
                );
            }
        }
    }
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return session_by_id(&connection, &id);
    }
    connection
        .execute(
            "INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,draft_scope_id,name,entry_url,status,last_error,capture_previous_status) VALUES(?1,?2,?3,?4,?5,?6,'capturing','','needs_check') ON CONFLICT(id) DO UPDATE SET name=excluded.name,entry_url=excluded.entry_url,capture_previous_status=CASE WHEN browser_auth_sessions.status='capturing' THEN COALESCE(NULLIF(browser_auth_sessions.capture_previous_status,''),'needs_check') ELSE browser_auth_sessions.status END,status='capturing',last_error='',updated_at=datetime('now','localtime')",
            params![id, input.project_id, scan_id, draft_scope_id, name, entry_url.as_str()],
        )
        .map_err(|error| error.to_string())?;
    drop(connection);

    let title = format!("登录并建立会话 · {name}");
    let mut builder = WebviewWindowBuilder::new(&app, label, WebviewUrl::External(entry_url))
        .title(title)
        .inner_size(1180.0, 820.0)
        .min_inner_size(760.0, 560.0)
        .center()
        .focused(true)
        .initialization_script(AUTH_CAPTURE_SCRIPT)
        .on_navigation(|url| matches!(url.scheme(), "http" | "https"));
    #[cfg(target_os = "macos")]
    {
        builder = if macos_supports_isolated_data_store() {
            builder.data_store_identifier(
                *Uuid::parse_str(&id)
                    .map_err(|_| "登录会话 ID 非法")?
                    .as_bytes(),
            )
        } else {
            // Older WKWebView versions have no named data stores. A
            // non-persistent store avoids inheriting cookies from the main app
            // or another captured identity.
            builder.incognito(true)
        };
    }
    #[cfg(not(target_os = "macos"))]
    {
        let profile_directory = auth_profile_directory(&state.app_data_dir, &id);
        std::fs::create_dir_all(&profile_directory)
            .map_err(|error| format!("无法创建独立身份目录：{error}"))?;
        builder = builder.data_directory(profile_directory);
    }
    if let Err(error) = builder.build() {
        let connection = db::open(&state.db_path)?;
        let _ = restore_cancelled_capture(
            &connection,
            &id,
            "登录窗口打开失败，本次捕获已取消；可重新打开后继续",
        );
        return Err(format!("登录窗口打开失败：{error}"));
    }
    let connection = db::open(&state.db_path)?;
    let session = session_by_id(&connection, &id)?;
    let _ = app.emit("browser-auth-session-updated", &session);
    Ok(session)
}

#[tauri::command]
pub async fn finish_browser_auth_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<BrowserAuthSession, String> {
    if Uuid::parse_str(&session_id).is_err() {
        return Err("登录会话 ID 非法".into());
    }
    let label = window_label(&session_id);
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| "登录窗口已关闭；请点击“重新登录”后再完成捕获".to_string())?;
    let final_url = window
        .url()
        .map_err(|error| format!("读取登录后地址失败：{error}"))?
        .to_string();
    let cookies = window
        .cookies()
        .map_err(|error| format!("读取浏览器 Cookie 失败：{error}"))?;
    let (sender, receiver) = mpsc::channel();
    window
        .eval_with_callback(AUTH_SNAPSHOT_SCRIPT, move |raw| {
            let _ = sender.send(raw);
        })
        .map_err(|error| format!("读取浏览器登录上下文失败：{error}"))?;
    let snapshot_raw =
        tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(8)))
            .await
            .map_err(|error| format!("登录上下文读取线程失败：{error}"))?
            .map_err(|_| "登录页面未及时返回会话信息；请保持窗口打开并重试".to_string())?;
    let snapshot = normalize_snapshot(snapshot_raw);
    let requests = snapshot
        .get("requests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let local_storage = snapshot
        .get("localStorage")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let session_storage = snapshot
        .get("sessionStorage")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut scope_hosts = BTreeSet::new();
    if let Some(host) = host_from(&final_url) {
        scope_hosts.insert(host);
    }
    let connection = db::open(&state.db_path)?;
    let (project_id, entry_url): (i64, String) = connection
        .query_row(
            "SELECT project_id,entry_url FROM browser_auth_sessions WHERE id=?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "登录会话不存在或已删除".to_string())?;
    if let Some(host) = host_from(&entry_url) {
        scope_hosts.insert(host);
    }

    let cookie_documents = cookies
        .iter()
        .map(|cookie| {
            if let Some(domain) = cookie.domain() {
                scope_hosts.insert(domain.trim_start_matches('.').to_ascii_lowercase());
            }
            json!({
                "name": cookie.name(),
                "value": cookie.value(),
                "domain": cookie.domain().unwrap_or(""),
                "path": cookie.path().unwrap_or("/"),
                "secure": cookie.secure().unwrap_or(false),
                "httpOnly": cookie.http_only().unwrap_or(false),
                "sameSite": cookie.same_site().map(|value| format!("{value:?}"))
            })
        })
        .collect::<Vec<_>>();

    let mut replay_headers = Map::new();
    let mut observed_header_names = BTreeSet::new();
    let mut successful_business_request = false;
    for request in &requests {
        if let Some(url) = request.get("url").and_then(Value::as_str) {
            if let Some(host) = host_from(url) {
                scope_hosts.insert(host);
            }
        }
        let request_url = request.get("url").and_then(Value::as_str).unwrap_or("");
        let status = request.get("status").and_then(Value::as_i64).unwrap_or(0);
        if (200..400).contains(&status) && !login_like(request_url) {
            successful_business_request = true;
        }
        if let Some(headers) = request.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                observed_header_names.insert(name.to_ascii_lowercase());
                if reusable_header(name) && value.as_str().is_some_and(|text| !text.is_empty()) {
                    replay_headers.insert(name.to_ascii_lowercase(), value.clone());
                }
            }
        }
    }
    let storage_count = map_len(&local_storage) + map_len(&session_storage);
    let has_auth_material = !cookie_documents.is_empty()
        || !replay_headers.is_empty()
        || has_token_storage(&local_storage)
        || has_token_storage(&session_storage);
    let valid = has_auth_material
        && (successful_business_request
            || (!login_like(&final_url)
                && (has_token_storage(&local_storage)
                    || has_token_storage(&session_storage)
                    || !cookie_documents.is_empty())));
    let status = if valid { "valid" } else { "needs_check" };
    let last_error = if valid {
        String::new()
    } else if !has_auth_material {
        "没有捕获到 Cookie、认证头或 Token Storage；请确认登录成功后重试".to_string()
    } else if login_like(&final_url) {
        "页面仍停留在登录/授权地址；会话尚未建立".to_string()
    } else {
        "已捕获会话材料，但尚缺少登录后业务请求；建议先点击一个登录后功能再完成捕获".to_string()
    };
    let scopes = scope_hosts.into_iter().collect::<Vec<_>>();
    let now = Utc::now();
    let expires_at = (now + ChronoDuration::hours(8)).to_rfc3339();
    let session_document = json!({
        "schemaVersion": 1,
        "id": session_id,
        "projectId": project_id,
        "entryUrl": entry_url,
        "finalUrl": final_url,
        "scopeHosts": scopes,
        "cookies": cookie_documents,
        "localStorage": local_storage,
        "sessionStorage": session_storage,
        "headers": replay_headers,
        "observedHeaderNames": observed_header_names.into_iter().collect::<Vec<_>>(),
        "capturedRequests": requests,
        "capturedAt": now.to_rfc3339(),
        "expiresAt": expires_at,
        "identityIsolation": {
            "sessionId": session_id,
            "mode": "dedicated-webview-data-store",
            "sharedWithOtherSessions": false
        },
        "replayPolicy": {
            "scope": "captured-hosts-only",
            "browserManagedHeaders": "regenerate",
            "authorizationBoundary": "record-401-403-without-global-stop",
            "stopOn": "confirmed-waf-or-challenge"
        }
    });
    connection
        .execute(
            "UPDATE browser_auth_sessions SET final_url=?1,status=?2,scope_hosts_json=?3,cookie_count=?4,header_count=?5,storage_count=?6,captured_request_count=?7,session_json=?8,last_validated_at=?9,expires_at=?10,last_error=?11,capture_previous_status='',updated_at=datetime('now','localtime') WHERE id=?12",
            params![
                final_url,
                status,
                serde_json::to_string(&scopes).map_err(|error| error.to_string())?,
                cookie_documents.len() as i64,
                replay_headers.len() as i64,
                storage_count as i64,
                requests.len() as i64,
                session_document.to_string(),
                now.to_rfc3339(),
                expires_at,
                last_error,
                session_id
            ],
        )
        .map_err(|error| error.to_string())?;
    let session = session_by_id(&connection, &session_id)?;
    drop(connection);

    // Only a genuinely valid capture closes the embedded login window.  The
    // old implementation closed it even for `needs_check`, so an automatic
    // poll could run once before the user finished SSO and silently terminate
    // the login flow.
    if valid {
        let toast_text = format!("登录成功，{} 已保存身份", session.name);
        let toast_json = serde_json::to_string(&toast_text).map_err(|error| error.to_string())?;
        let toast_script = format!(
            r#"(() => {{
                const id = '__oviraptor_auth_save_toast__';
                document.getElementById(id)?.remove();
                const toast = document.createElement('div');
                toast.id = id;
                toast.textContent = {toast_json};
                Object.assign(toast.style, {{
                    position: 'fixed', top: '18px', right: '18px', zIndex: '2147483647',
                    maxWidth: 'min(420px, calc(100vw - 36px))', padding: '12px 16px',
                    borderRadius: '12px', border: '1px solid rgba(93, 214, 159, .45)',
                    background: 'linear-gradient(135deg, #123b35, #185847)', color: '#effff8',
                    boxShadow: '0 14px 36px rgba(0,0,0,.24)', font: '600 14px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif',
                    opacity: '0', transform: 'translateY(-8px)', transition: 'opacity .18s ease, transform .18s ease'
                }});
                document.documentElement.appendChild(toast);
                requestAnimationFrame(() => {{ toast.style.opacity = '1'; toast.style.transform = 'translateY(0)'; }});
            }})();"#
        );
        let _ = window.eval(&toast_script);
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(1000)))
            .await
            .map_err(|error| format!("登录成功提示等待失败：{error}"))?;
        let _ = window.close();
    }
    let _ = app.emit("browser-auth-session-updated", &session);
    Ok(session)
}

#[tauri::command]
pub fn list_browser_auth_sessions(
    state: State<'_, AppState>,
    project_id: i64,
    draft_scope_id: String,
) -> Result<Vec<BrowserAuthSession>, String> {
    let draft_scope_id = draft_scope_id.trim();
    if Uuid::parse_str(draft_scope_id).is_err() {
        return Err("登录会话草稿作用域 ID 非法".into());
    }
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM browser_auth_sessions WHERE project_id=?1 AND owner_scan_id='' AND draft_scope_id=?2 ORDER BY CASE status WHEN 'valid' THEN 0 WHEN 'capturing' THEN 1 WHEN 'needs_check' THEN 2 ELSE 3 END,updated_at DESC"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, draft_scope_id], auth_session_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn list_sentinel_scan_auth_sessions(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<BrowserAuthSession>, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, policy_text): (i64, String) = connection
        .query_row(
            "SELECT scan.project_id,COALESCE(context.policy_json,'{}') FROM sentinel_scans AS scan LEFT JOIN sentinel_scan_contexts AS context ON context.scan_id=scan.id WHERE scan.id=?1",
            [&scan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "扫描任务不存在，无法读取登录会话".to_string())?;
    let policy: Value = serde_json::from_str(&policy_text).unwrap_or_else(|_| json!({}));
    let ids = auth_session_ids_from_policy(&policy);
    let mut sessions = Vec::new();
    for id in ids {
        let session = session_by_id(&connection, &id)
            .map_err(|_| format!("任务绑定的登录会话 {id} 已被删除；请保留任务会话后再重试"))?;
        if session.project_id != project_id {
            return Err("任务绑定了其他工作空间的登录会话，请重新创建任务会话".into());
        }
        if !session.owner_scan_id.is_empty() && session.owner_scan_id != scan_id {
            return Err(
                "任务引用了另一个任务的登录会话；为防止身份串用，本次不会复用该会话".into(),
            );
        }
        sessions.push(session);
    }
    Ok(sessions)
}

pub(crate) fn validate_draft_sessions_for_task(
    connection: &rusqlite::Connection,
    session_ids: &[String],
    project_id: i64,
    draft_scope_id: &str,
) -> Result<(), String> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let draft_scope_id = draft_scope_id.trim();
    if Uuid::parse_str(draft_scope_id).is_err() {
        return Err("当前登录身份没有有效的任务草稿作用域，请在本任务中重新登录".into());
    }
    for session_id in session_ids {
        let (owner_project_id, owner_scan_id, session_scope_id): (i64, String, String) = connection
            .query_row(
                "SELECT project_id,owner_scan_id,draft_scope_id FROM browser_auth_sessions WHERE id=?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "所选浏览器登录会话不存在".to_string())?;
        if owner_project_id != project_id {
            return Err("所选浏览器登录会话不属于当前工作空间".into());
        }
        if !owner_scan_id.is_empty() {
            return Err(
                "所选登录会话已经属于另一个扫描任务，不能跨任务复用；请为本任务重新登录".into(),
            );
        }
        if session_scope_id != draft_scope_id {
            return Err(
                "所选登录会话来自另一个任务草稿，不能跨任务复用；请为本任务重新登录".into(),
            );
        }
    }
    Ok(())
}

pub(crate) fn bind_draft_sessions_to_scan(
    connection: &rusqlite::Connection,
    session_ids: &[String],
    project_id: i64,
    draft_scope_id: &str,
    scan_id: &str,
) -> Result<(), String> {
    validate_draft_sessions_for_task(connection, session_ids, project_id, draft_scope_id)?;
    for session_id in session_ids {
        let changed = connection
            .execute(
                "UPDATE browser_auth_sessions SET owner_scan_id=?1,draft_scope_id='',updated_at=datetime('now','localtime') WHERE id=?2 AND project_id=?3 AND owner_scan_id='' AND draft_scope_id=?4",
                params![scan_id, session_id, project_id, draft_scope_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("登录会话绑定任务时发生并发变化；为避免串用身份，请重新登录后重试".into());
        }
    }
    Ok(())
}

fn waf_evidence(status: u16, headers: &reqwest::header::HeaderMap, body: &str) -> bool {
    let server = headers
        .get("server")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let lower = body.to_ascii_lowercase();
    let markers = [
        "cf-chl-",
        "cloudflare ray id",
        "attention required! | cloudflare",
        "aws waf",
        "akamai reference",
        "incapsula incident id",
        "sucuri website firewall",
        "web application firewall",
        "js challenge",
        "verify you are human",
        "人机验证",
        "访问验证",
    ];
    status == 429
        || server.contains("cloudflare") && lower.contains("challenge")
        || markers.iter().any(|marker| lower.contains(marker))
}

fn validation_request_noise(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let path = lower
        .split('#')
        .next()
        .unwrap_or(&lower)
        .split('?')
        .next()
        .unwrap_or(&lower);
    [
        ".avif", ".css", ".gif", ".ico", ".jpeg", ".jpg", ".png", ".svg", ".webp", ".woff",
        ".woff2",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
        || [
            "deviceprofile",
            "data_report_web",
            "sentry",
            "/envelope",
            "telemetry",
            "/beacon",
        ]
        .iter()
        .any(|marker| path.contains(marker))
}

fn browser_auth_validation_target(document: &Value, entry_url: &str, final_url: &str) -> String {
    let captured = document
        .get("capturedRequests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|request| {
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET");
            let status = request.get("status").and_then(Value::as_i64).unwrap_or(0);
            let url = request.get("url").and_then(Value::as_str).unwrap_or("");
            (method.eq_ignore_ascii_case("GET")
                && (200..400).contains(&status)
                && Url::parse(url)
                    .ok()
                    .is_some_and(|value| matches!(value.scheme(), "http" | "https"))
                && !login_like(url)
                && !validation_request_noise(url))
            .then(|| url.to_string())
        });
    captured.unwrap_or_else(|| {
        if final_url.trim().is_empty() {
            entry_url.to_string()
        } else {
            final_url.to_string()
        }
    })
}

fn validate_browser_auth_session_on(
    connection: &rusqlite::Connection,
    validation_session_id: &str,
) -> Result<(), String> {
    let (entry_url, final_url, previous_status, document_text): (String, String, String, String) = connection
        .query_row(
            "SELECT entry_url,final_url,status,session_json FROM browser_auth_sessions WHERE id=?1",
            [validation_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "登录会话不存在或已删除".to_string())?;
    let document: Value = serde_json::from_str(&document_text).unwrap_or_else(|_| json!({}));
    let target = browser_auth_validation_target(&document, &entry_url, &final_url);
    let mut headers = HeaderMap::new();
    if let Some(values) = document.get("headers").and_then(Value::as_object) {
        for (name, value) in values {
            let Some(text) = value.as_str() else { continue };
            if !reusable_header(name) { continue; }
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(text)) {
                headers.insert(name, value);
            }
        }
    }
    let target_host = host_from(&target).unwrap_or_default();
    let cookie_header = document
        .get("cookies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|cookie| {
            let domain = cookie.get("domain").and_then(Value::as_str).unwrap_or("").trim_start_matches('.').to_ascii_lowercase();
            domain.is_empty() || target_host == domain || target_host.ends_with(&format!(".{domain}"))
        })
        .filter_map(|cookie| Some(format!("{}={}", cookie.get("name")?.as_str()?, cookie.get("value")?.as_str()?)))
        .collect::<Vec<_>>()
        .join("; ");
    if !cookie_header.is_empty() {
        headers.insert(COOKIE, HeaderValue::from_str(&cookie_header).map_err(|_| "Cookie 包含无法重放的字符".to_string())?);
    }
    // This request carries captured credentials: retain normal certificate and
    // hostname verification, including on every redirect. Never retry insecurely.
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .map_err(|error| error.to_string())?;
    let now = Utc::now().to_rfc3339();
    match client.get(&target).headers(headers).send() {
        Ok(response) => {
            let status_code = response.status().as_u16();
            let response_url = response.url().to_string();
            let response_headers = response.headers().clone();
            let body = response.text().unwrap_or_default().chars().take(200_000).collect::<String>();
            let waf = waf_evidence(status_code, &response_headers, &body);
            let clear_logout = login_like(&response_url) && !login_like(&target);
            let (status, error) = if clear_logout {
                ("invalid", "会话校验被重定向回登录页；请重新登录".to_string())
            } else if waf {
                // WAF is a target fuse signal, not proof that the user's session expired.
                (previous_status.as_str(), "检测到明确 WAF/人机挑战特征；会话保留，扫描该目标时将自动熔断".to_string())
            } else if matches!(status_code, 401 | 403) {
                (previous_status.as_str(), "校验地址返回权限边界响应；单个 401/403 不判定会话失效，将由业务请求继续验证".to_string())
            } else if (200..400).contains(&status_code) {
                ("valid", String::new())
            } else {
                (capture_restore_status(&previous_status), format!("校验地址返回 HTTP {status_code}，不足以证明会话失效；已保留原状态，可在登录窗口访问业务功能后重新捕获"))
            };
            connection.execute(
                "UPDATE browser_auth_sessions SET status=?1,last_validated_at=?2,last_error=?3,updated_at=datetime('now','localtime') WHERE id=?4",
                params![status, now, error, validation_session_id],
            ).map_err(|error| error.to_string())?;
        }
        Err(error) => {
            connection.execute(
                "UPDATE browser_auth_sessions SET status=?1,last_error=?2,updated_at=datetime('now','localtime') WHERE id=?3",
                params![capture_restore_status(&previous_status),format!("会话校验请求失败，无法据此判定会话失效；已保留原状态：{error}"),validation_session_id],
            ).map_err(|db_error| db_error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn validate_browser_auth_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<BrowserAuthSession, String> {
    let db_path = state.db_path.clone();
    let validation_session_id = session_id.clone();
    let validation = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let connection = db::open(&db_path)?;
        validate_browser_auth_session_on(&connection, &validation_session_id)
    })
    .await
    .map_err(|error| format!("会话校验线程失败：{error}"))?;
    validation?;
    let connection = db::open(&state.db_path)?;
    let session = session_by_id(&connection, &session_id)?;
    let _ = app.emit("browser-auth-session-updated", &session);
    Ok(session)
}

#[tauri::command]
pub fn delete_browser_auth_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    if Uuid::parse_str(&session_id).is_err() {
        return Err("登录会话 ID 非法".into());
    }
    if let Some(window) = app.get_webview_window(&window_label(&session_id)) {
        let _ = window.close();
    }
    let connection = db::open(&state.db_path)?;
    let bound_scan = {
        let mut statement = connection
            .prepare(
                "SELECT scan.id,scan.task_name,context.policy_json FROM sentinel_scan_contexts AS context JOIN sentinel_scans AS scan ON scan.id=context.scan_id WHERE context.policy_json LIKE ?1 ORDER BY scan.updated_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let candidates = statement
            .query_map([format!("%{session_id}%")], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        candidates.into_iter().find(|(_, _, policy_text)| {
            serde_json::from_str::<Value>(policy_text)
                .ok()
                .is_some_and(|policy| auth_session_ids_from_policy(&policy).contains(&session_id))
        })
    };
    if let Some((scan_id, task_name, _)) = bound_scan {
        let title = if task_name.trim().is_empty() {
            scan_id
        } else {
            task_name
        };
        return Err(format!(
            "该登录会话仍绑定任务“{title}”；为避免任务令牌失效后无法重新登录，不能直接删除，请在该任务中使用“重新登录”"
        ));
    }
    let deleted = connection
        .execute(
            "DELETE FROM browser_auth_sessions WHERE id=?1",
            [&session_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("登录会话不存在或已经删除".into());
    }
    #[cfg(target_os = "macos")]
    if macos_supports_isolated_data_store() {
        if let Ok(uuid) = Uuid::parse_str(&session_id) {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = handle.remove_data_store(*uuid.as_bytes()).await;
            });
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let profile_directory = auth_profile_directory(&state.app_data_dir, &session_id);
        if profile_directory.is_dir() {
            let _ = std::fs::remove_dir_all(profile_directory);
        }
    }
    let _ = app.emit("browser-auth-session-deleted", &session_id);
    Ok(())
}

pub(crate) fn session_document_for_scan(
    connection: &rusqlite::Connection,
    session_id: &str,
    project_id: i64,
) -> Result<Value, String> {
    if session_id.trim().is_empty() {
        return Err("登录会话 ID 不能为空".into());
    }
    let (owner, name, status, expires_at, document): (i64, String, String, String, String) = connection
        .query_row(
            "SELECT project_id,name,status,expires_at,session_json FROM browser_auth_sessions WHERE id=?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|_| "所选浏览器登录会话不存在".to_string())?;
    if owner != project_id {
        return Err("所选浏览器登录会话不属于当前工作空间".into());
    }
    if status != "valid" {
        return Err("所选浏览器登录会话当前不是绿色有效状态；请重新登录或先完成校验".into());
    }
    if !chrono::DateTime::parse_from_rfc3339(&expires_at).is_ok_and(|expires| expires > Utc::now())
    {
        return Err("所选浏览器登录会话已超过 8 小时安全期限；请重新登录".into());
    }
    let mut value: Value = serde_json::from_str(&document)
        .map_err(|_| "浏览器登录会话数据损坏；请删除后重新登录".to_string())?;
    if let Some(object) = value.as_object_mut() {
        object.insert("name".into(), Value::String(name));
    }
    Ok(value)
}

pub(crate) fn write_session_document(path: &Path, document: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn mark_session_invalid(db_path: &Path, session_id: &str, reason: &str) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE browser_auth_sessions SET status='invalid',last_error=?1,updated_at=datetime('now','localtime') WHERE id=?2",
            params![reason, session_id],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_material_fixture() -> (std::path::PathBuf, rusqlite::Connection) {
        let root = std::env::temp_dir().join(format!("oviraptor-identity-material-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(9001,'Identity material'),(9002,'Other project')", []).unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,task_name) VALUES('task-a',9001,'A'),('task-b',9001,'B')", []).unwrap();
        connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('task-a','{\"authSessionIds\":[\"identity-a\",\"identity-b\"]}')", []).unwrap();
        for (id, value) in [("identity-a", "account-a"), ("identity-b", "account-b")] {
            let document = json!({"cookies":[{"name":"session_id","value":value}],"headers":{"authorization":format!("Bearer {value}")},"localStorage":{"password":format!("password-{value}")}});
            connection.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,expires_at,session_json) VALUES(?1,9001,'task-a',?1,'https://fixture.test','expired','2020-01-01T00:00:00Z',?2)", params![id, document.to_string()]).unwrap();
        }
        (root, connection)
    }

    #[test]
    fn identity_material_preserves_plaintext_without_revalidating_expired_sessions() {
        let (root, connection) = identity_material_fixture();
        let snapshot = scan_identity_material_on(&connection, 9001, "task-a").unwrap();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].document["cookies"][0]["value"], "account-a");
        assert_eq!(snapshot[1].document["headers"]["authorization"], "Bearer account-b");
        assert_eq!(snapshot[0].document["localStorage"]["password"], "password-account-a");
        assert!(snapshot.iter().all(|item| item.session.status == "expired"));
        assert!(session_document_for_scan(&connection, "identity-a", 9001).is_err());
        drop(connection); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identity_material_rejects_cross_project_cross_task_and_unbound_draft() {
        let (root, connection) = identity_material_fixture();
        assert!(scan_identity_material_on(&connection, 9002, "task-a").is_err());
        assert!(scan_identity_material_on(&connection, 9001, "missing").is_err());
        for (project, owner) in [(9002, "task-a"), (9001, "task-b"), (9001, "")] {
            connection.execute("UPDATE browser_auth_sessions SET project_id=?1,owner_scan_id=?2 WHERE id='identity-b'", params![project, owner]).unwrap();
            assert!(scan_identity_material_on(&connection, 9001, "task-a").is_err());
        }
        drop(connection); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identity_material_rejects_damaged_or_oversize_documents_atomically() {
        let (root, connection) = identity_material_fixture();
        for text in ["not-json".to_string(), "[]".to_string(), json!({"token":"x".repeat(256*1024)}).to_string()] {
            connection.execute("UPDATE browser_auth_sessions SET session_json=?1 WHERE id='identity-b'", [text]).unwrap();
            assert!(scan_identity_material_on(&connection, 9001, "task-a").is_err());
        }
        drop(connection); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelling_login_capture_restores_a_safe_previous_state() {
        assert_eq!(capture_restore_status("valid"), "valid");
        assert_eq!(capture_restore_status("expired"), "expired");
        assert_eq!(capture_restore_status("invalid"), "invalid");
        assert_eq!(capture_restore_status("capturing"), "needs_check");
        assert_eq!(capture_restore_status(""), "needs_check");
    }

    #[test]
    fn validation_prefers_a_successful_business_request_over_a_stale_final_url() {
        let document = json!({"capturedRequests":[
            {"method":"GET","url":"https://cdn.test/avatar/user.jpeg","status":200},
            {"method":"POST","url":"https://fp.test/deviceprofile/v4","status":200},
            {"method":"GET","url":"https://api.example.test/api/account/profile","status":200}
        ]});
        assert_eq!(
            browser_auth_validation_target(
                &document,
                "https://example.test/login",
                "https://example.test/stale-404"
            ),
            "https://api.example.test/api/account/profile"
        );
        assert!(validation_request_noise(
            "https://cdn.test/avatar/user.jpeg?size=100"
        ));
    }

    #[test]
    fn scan_policy_keeps_all_bound_auth_session_ids() {
        let ids = auth_session_ids_from_policy(&json!({
            "authSessionId": "identity-b",
            "authSessionIds": ["identity-a", "identity-b", "", "identity-a"]
        }));
        assert_eq!(ids, vec!["identity-a", "identity-b"]);
    }

    #[test]
    fn classifies_login_and_reusable_headers_without_request_ids() {
        assert!(login_like("https://example.test/auth/login?next=/"));
        assert!(reusable_header("Authorization"));
        assert!(reusable_header("X-CSRF-Token"));
        assert!(!reusable_header("X-Request-Id"));
        assert!(!reusable_header("Sec-Fetch-Site"));
    }

    #[test]
    fn detects_waf_but_not_plain_permission_denial() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(waf_evidence(
            403,
            &headers,
            "Cloudflare Ray ID: abc; verify you are human"
        ));
        assert!(!waf_evidence(
            403,
            &headers,
            "{\"message\":\"role not allowed\"}"
        ));
    }

    #[test]
    fn identity_fingerprint_detects_reused_and_distinct_accounts() {
        let first = json!({"cookies":[{"name":"session_id","value":"account-a","domain":"example.test","path":"/"}],"headers":{},"localStorage":{},"sessionStorage":{}});
        let reused = json!({"cookies":[{"name":"session_id","value":"account-a","domain":"example.test","path":"/"}],"headers":{},"localStorage":{},"sessionStorage":{}});
        let second = json!({"cookies":[{"name":"session_id","value":"account-b","domain":"example.test","path":"/"}],"headers":{},"localStorage":{},"sessionStorage":{}});
        assert_eq!(
            auth_identity_fingerprint(&first),
            auth_identity_fingerprint(&reused)
        );
        assert_ne!(
            auth_identity_fingerprint(&first),
            auth_identity_fingerprint(&second)
        );
    }

    #[test]
    fn malformed_expiry_is_neither_a_green_session_nor_reusable_credentials() {
        let root = std::env::temp_dir().join(format!("oviraptor-auth-expiry-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(9001,'Expiry test')",
                [],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,name,entry_url,status,session_json,expires_at) \
             VALUES('bad-expiry',9001,'Bad expiry','https://example.test','valid','{}','not-a-date')",
            [],
        ).unwrap();
        assert_eq!(
            session_by_id(&connection, "bad-expiry").unwrap().status,
            "expired"
        );
        assert!(session_document_for_scan(&connection, "bad-expiry", 9001)
            .unwrap_err()
            .contains("安全期限"));
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "auth_session_validation_tests.rs"]
mod validation_tests;
