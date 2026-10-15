//! HackerOne builtin workspace commands and source adapter.
use crate::{
    db,
    hackerone_models::{HackerOneDetail, HackerOneEvent, HackerOneExclusion, HackerOneProgram, HackerOneScope},
    AppState,
};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, time::Duration};
use tauri::{AppHandle, State};

fn hackerone_program(row: &Row<'_>) -> rusqlite::Result<HackerOneProgram> {
    Ok(HackerOneProgram {
        id: row.get(0)?,
        handle: row.get(1)?,
        name: row.get(2)?,
        icon_url: row.get(3)?,
        policy: row.get(4)?,
        submission_state: row.get(5)?,
        program_state: row.get(6)?,
        offers_bounties: row.get::<_, i64>(7)? != 0,
        open_scope: row.get::<_, i64>(8)? != 0,
        fast_payments: row.get::<_, i64>(9)? != 0,
        safe_harbor: row.get::<_, i64>(10)? != 0,
        collaboration: row.get::<_, i64>(11)? != 0,
        last_synced_at: row.get(12)?,
        bookmarked: row.get::<_, i64>(13)? != 0,
        scope_count: row.get(14)?,
    })
}

const H1_PROGRAM_SELECT: &str = r#"SELECT p.id,p.handle,p.name,p.icon_url,p.policy,p.submission_state,p.program_state,p.offers_bounties,p.open_scope,p.fast_payments,p.safe_harbor,p.collaboration,p.last_synced_at,
 COALESCE(n.bookmarked,0),(SELECT COUNT(*) FROM hackerone_scopes s WHERE s.program_handle=p.handle AND s.active=1)
 FROM hackerone_programs p LEFT JOIN hackerone_notes n ON n.program_handle=p.handle"#;

#[tauri::command]
pub fn list_hackerone_programs(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<Vec<HackerOneProgram>, String> {
    let connection = db::open(&state.db_path)?;
    let needle = format!("%{}%", search.unwrap_or_default().trim());
    let sql=format!("{H1_PROGRAM_SELECT} WHERE (?1='%%' OR p.name LIKE ?1 OR p.handle LIKE ?1) ORDER BY COALESCE(n.bookmarked,0) DESC,p.offers_bounties DESC,p.name");
    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let programs = statement
        .query_map([needle], hackerone_program)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(programs)
}

#[tauri::command]
pub fn get_hackerone_detail(
    state: State<'_, AppState>,
    handle: String,
) -> Result<HackerOneDetail, String> {
    let connection = db::open(&state.db_path)?;
    let program = connection
        .query_row(
            &format!("{H1_PROGRAM_SELECT} WHERE p.handle=?1"),
            [handle.as_str()],
            hackerone_program,
        )
        .map_err(|e| e.to_string())?;
    let mut scope_statement=connection.prepare("SELECT id,asset_type,asset_identifier,eligible_for_submission,eligible_for_bounty,max_severity,instruction,updated_at FROM hackerone_scopes WHERE program_handle=?1 AND active=1 ORDER BY eligible_for_submission DESC,eligible_for_bounty DESC,asset_type,asset_identifier").map_err(|e|e.to_string())?;
    let scopes = scope_statement
        .query_map([handle.as_str()], |row| {
            Ok(HackerOneScope {
                id: row.get(0)?,
                asset_type: row.get(1)?,
                asset_identifier: row.get(2)?,
                eligible_for_submission: row.get::<_, i64>(3)? != 0,
                eligible_for_bounty: row.get::<_, i64>(4)? != 0,
                max_severity: row.get(5)?,
                instruction: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut exclusion_statement=connection.prepare("SELECT id,category,details,updated_at FROM hackerone_exclusions WHERE program_handle=?1 AND active=1 ORDER BY category").map_err(|e|e.to_string())?;
    let exclusions = exclusion_statement
        .query_map([handle.as_str()], |row| {
            Ok(HackerOneExclusion {
                id: row.get(0)?,
                category: row.get(1)?,
                details: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(HackerOneDetail {
        program,
        scopes,
        exclusions,
    })
}

#[tauri::command]
pub fn set_hackerone_bookmark(
    state: State<'_, AppState>,
    handle: String,
    bookmarked: bool,
) -> Result<(), String> {
    db::open(&state.db_path)?.execute("INSERT INTO hackerone_notes(program_handle,bookmarked) VALUES(?1,?2) ON CONFLICT(program_handle) DO UPDATE SET bookmarked=excluded.bookmarked",params![handle,bookmarked as i64]).map_err(|e|e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_hackerone_events(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<HackerOneEvent>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement=connection.prepare("SELECT id,program_handle,event_type,summary,created_at FROM hackerone_events ORDER BY id DESC LIMIT ?1").map_err(|e|e.to_string())?;
    let events = statement
        .query_map([limit.unwrap_or(100).clamp(1, 500)], |row| {
            Ok(HackerOneEvent {
                id: row.get(0)?,
                program_handle: row.get(1)?,
                event_type: row.get(2)?,
                summary: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(events)
}

const HACKERONE_API_BASE: &str = "https://api.hackerone.com/v1";

fn hackerone_get(
    client: &reqwest::blocking::Client,
    username: &str,
    token: &str,
    path_or_url: &str,
) -> Result<JsonValue, String> {
    let url = if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        path_or_url.to_string()
    } else {
        format!("{HACKERONE_API_BASE}{path_or_url}")
    };
    let response = client
        .get(&url)
        .basic_auth(username, Some(token))
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .map_err(|error| format!("HackerOne 请求失败：{error}"))?;
    let status = response.status();
    let body = response.text().map_err(|error| format!("读取 HackerOne 响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("HackerOne 返回 HTTP {}：{}", status.as_u16(), body.chars().take(500).collect::<String>()));
    }
    serde_json::from_str(&body).map_err(|error| format!("HackerOne 返回无法解析的 JSON：{error}"))
}

fn hackerone_pages(
    client: &reqwest::blocking::Client,
    username: &str,
    token: &str,
    path: &str,
) -> Result<Vec<JsonValue>, String> {
    let mut url = format!("{HACKERONE_API_BASE}{path}");
    let mut result = Vec::new();
    let mut pages = 0usize;
    while !url.is_empty() {
        pages += 1;
        if pages > 1_000 {
            return Err("HackerOne 分页超过安全上限".into());
        }
        let payload = hackerone_get(client, username, token, &url)?;
        if let Some(items) = payload.get("data").and_then(JsonValue::as_array) {
            result.extend(items.iter().cloned());
        }
        url = payload
            .pointer("/links/next")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
    }
    Ok(result)
}

fn h1_text(value: Option<&JsonValue>) -> String {
    value.and_then(JsonValue::as_str).unwrap_or("").to_string()
}

fn h1_flag(value: Option<&JsonValue>) -> i64 {
    i64::from(value.and_then(JsonValue::as_bool).unwrap_or(false))
}

fn save_native_hackerone_program(
    transaction: &rusqlite::Transaction<'_>,
    item: &JsonValue,
) -> Result<String, String> {
    let attributes = item.get("attributes").unwrap_or(&JsonValue::Null);
    let handle = h1_text(attributes.get("handle"));
    let handle = if handle.is_empty() { h1_text(item.get("id")) } else { handle };
    if handle.is_empty() { return Err("HackerOne 项目缺少 handle".into()); }
    let policy = h1_text(attributes.get("policy"));
    let policy_hash = format!("{:x}", Sha256::digest(policy.as_bytes()));
    let submission_state = h1_text(attributes.get("submission_state"));
    let previous: Option<(String, String)> = transaction
        .query_row("SELECT policy_hash,submission_state FROM hackerone_programs WHERE handle=?1", [&handle], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional().map_err(|error| error.to_string())?;
    let picture = h1_text(attributes.get("profile_picture"));
    let icon_url = if picture.is_empty() || picture.starts_with("http") { picture } else { format!("https://hackerone.com/{}", picture.trim_start_matches('/')) };
    transaction.execute(
        "INSERT INTO hackerone_programs(id,handle,name,icon_url,policy,policy_hash,submission_state,program_state,offers_bounties,open_scope,fast_payments,safe_harbor,collaboration,started_accepting_at,last_synced_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,datetime('now','localtime')) ON CONFLICT(handle) DO UPDATE SET id=excluded.id,name=excluded.name,icon_url=excluded.icon_url,policy=excluded.policy,policy_hash=excluded.policy_hash,submission_state=excluded.submission_state,program_state=excluded.program_state,offers_bounties=excluded.offers_bounties,open_scope=excluded.open_scope,fast_payments=excluded.fast_payments,safe_harbor=excluded.safe_harbor,collaboration=excluded.collaboration,started_accepting_at=excluded.started_accepting_at,last_synced_at=excluded.last_synced_at",
        params![h1_text(item.get("id")), handle, { let name=h1_text(attributes.get("name")); if name.is_empty(){handle.clone()}else{name} }, icon_url, policy, policy_hash, submission_state, h1_text(attributes.get("state")), h1_flag(attributes.get("offers_bounties")), h1_flag(attributes.get("open_scope")), h1_flag(attributes.get("fast_payments")), h1_flag(attributes.get("gold_standard_safe_harbor")), h1_flag(attributes.get("allows_bounty_splitting")), h1_text(attributes.get("started_accepting_at"))]
    ).map_err(|error| error.to_string())?;
    if let Some((old_hash, old_state)) = previous {
        if old_hash != policy_hash { transaction.execute("INSERT INTO hackerone_events(program_handle,event_type,summary) VALUES(?1,'policy_changed','Policy 内容发生变化')", [&handle]).map_err(|e|e.to_string())?; }
        if old_state != submission_state { transaction.execute("INSERT INTO hackerone_events(program_handle,event_type,summary) VALUES(?1,'submission_state_changed',?2)", params![handle, format!("提交状态：{old_state} → {submission_state}")]).map_err(|e|e.to_string())?; }
    }
    Ok(handle)
}

fn native_hackerone_sync_list(
    client: &reqwest::blocking::Client,
    connection: &mut rusqlite::Connection,
    username: &str,
    token: &str,
) -> Result<JsonValue, String> {
    let items = hackerone_pages(client, username, token, "/hackers/programs?page[size]=100")?;
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    for item in &items { save_native_hackerone_program(&transaction, item)?; }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"programs":items.len()}))
}

fn native_hackerone_sync_detail(
    client: &reqwest::blocking::Client,
    connection: &mut rusqlite::Connection,
    username: &str,
    token: &str,
    handle: &str,
) -> Result<JsonValue, String> {
    let program_payload = hackerone_get(client, username, token, &format!("/hackers/programs/{handle}"))?;
    let program = program_payload.get("data").cloned().unwrap_or(JsonValue::Null);
    let scopes = hackerone_pages(client, username, token, &format!("/hackers/programs/{handle}/structured_scopes?page[size]=100"))?;
    let exclusions = hackerone_pages(client, username, token, &format!("/hackers/programs/{handle}/scope_exclusions?page[size]=100"))?;
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    save_native_hackerone_program(&transaction, &program)?;
    let mut previous = HashMap::<String,(i64,i64,String,String)>::new();
    {
        let mut statement=transaction.prepare("SELECT id,eligible_for_submission,eligible_for_bounty,max_severity,instruction FROM hackerone_scopes WHERE program_handle=?1 AND active=1").map_err(|e|e.to_string())?;
        let rows=statement.query_map([handle],|row|Ok((row.get::<_,String>(0)?,(row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))).map_err(|e|e.to_string())?;
        for row in rows { let (id,value)=row.map_err(|e|e.to_string())?; previous.insert(id,value); }
    }
    let mut seen=HashSet::new();
    for item in &scopes {
        let attributes=item.get("attributes").unwrap_or(&JsonValue::Null);let id=h1_text(item.get("id"));if id.is_empty(){continue}seen.insert(id.clone());
        let current=(h1_flag(attributes.get("eligible_for_submission")),h1_flag(attributes.get("eligible_for_bounty")),h1_text(attributes.get("max_severity")),h1_text(attributes.get("instruction")));
        transaction.execute("INSERT INTO hackerone_scopes(id,program_handle,asset_type,asset_identifier,eligible_for_submission,eligible_for_bounty,max_severity,instruction,reference,created_at,updated_at,active) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1) ON CONFLICT(id) DO UPDATE SET program_handle=excluded.program_handle,asset_type=excluded.asset_type,asset_identifier=excluded.asset_identifier,eligible_for_submission=excluded.eligible_for_submission,eligible_for_bounty=excluded.eligible_for_bounty,max_severity=excluded.max_severity,instruction=excluded.instruction,reference=excluded.reference,updated_at=excluded.updated_at,active=1",params![id,handle,h1_text(attributes.get("asset_type")),h1_text(attributes.get("asset_identifier")),current.0,current.1,current.2,current.3,h1_text(attributes.get("reference")),h1_text(attributes.get("created_at")),h1_text(attributes.get("updated_at"))]).map_err(|e|e.to_string())?;
        let event=match previous.get(&id){None=>Some("scope_added"),Some(old) if old!=&current=>Some("scope_changed"),_=>None};
        if let Some(kind)=event { transaction.execute("INSERT INTO hackerone_events(program_handle,event_type,summary) VALUES(?1,?2,?3)",params![handle,kind,format!("{} Scope：{}",if kind=="scope_added"{"新增"}else{"变化"},h1_text(attributes.get("asset_identifier")))]).map_err(|e|e.to_string())?; }
    }
    for id in previous.keys().filter(|id|!seen.contains(*id)){transaction.execute("UPDATE hackerone_scopes SET active=0 WHERE id=?1",[id]).map_err(|e|e.to_string())?;transaction.execute("INSERT INTO hackerone_events(program_handle,event_type,summary) VALUES(?1,'scope_removed',?2)",params![handle,format!("移除 Scope：{id}")]).map_err(|e|e.to_string())?;}
    transaction.execute("UPDATE hackerone_exclusions SET active=0 WHERE program_handle=?1",[handle]).map_err(|e|e.to_string())?;
    for item in &exclusions {let a=item.get("attributes").unwrap_or(&JsonValue::Null);transaction.execute("INSERT INTO hackerone_exclusions(id,program_handle,category,details,updated_at,active) VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(id) DO UPDATE SET category=excluded.category,details=excluded.details,updated_at=excluded.updated_at,active=1",params![h1_text(item.get("id")),handle,h1_text(a.get("category")),h1_text(a.get("details")),h1_text(a.get("updated_at"))]).map_err(|e|e.to_string())?;}
    transaction.commit().map_err(|e|e.to_string())?;
    Ok(serde_json::json!({"handle":handle,"scopes":scopes.len(),"exclusions":exclusions.len()}))
}

#[tauri::command]
pub fn sync_hackerone(
    _app: AppHandle,
    state: State<AppState>,
    profile_id: i64,
    handle: Option<String>,
) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let settings_text: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .map_err(|_| "配置方案不存在".to_string())?;
    let settings: JsonValue = serde_json::from_str(&settings_text).map_err(|e| e.to_string())?;
    let get = |key: &str| {
        settings
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let username = get("hackerOneUsername");
    let token = get("hackerOneToken");
    if username.is_empty() || token.is_empty() {
        return Err("请先在配置中心填写 HackerOne API identifier 和 token".into());
    }
    let proxy = get("proxyUrl");
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent("oviraptor/1.1 native-hackerone");
    if !proxy.is_empty() {
        builder = builder.proxy(reqwest::Proxy::all(&proxy).map_err(|error| format!("代理配置无效：{error}"))?);
    }
    let client = builder.build().map_err(|error| format!("无法初始化 HackerOne 客户端：{error}"))?;
    let mut connection = db::open(&state.db_path)?;
    let result = if let Some(handle) = handle.filter(|value| !value.trim().is_empty()) {
        native_hackerone_sync_detail(&client, &mut connection, &username, &token, handle.trim())?
    } else {
        native_hackerone_sync_list(&client, &mut connection, &username, &token)?
    };
    serde_json::to_string(&result).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn add_hackerone_scopes_to_project(
    state: State<AppState>,
    handle: String,
    project_id: i64,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let mut statement=transaction.prepare("SELECT asset_type,asset_identifier FROM hackerone_scopes WHERE program_handle=?1 AND active=1 AND eligible_for_submission=1").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map([handle], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let mut added = 0;
    for (kind, raw) in rows {
        let lower = kind.to_lowercase();
        let (target_type, value) = if lower.contains("cidr") {
            ("cidr", raw)
        } else if lower.contains("ip address") {
            ("ip", raw)
        } else if lower.contains("domain") || lower.contains("wildcard") || lower == "url" {
            let without_scheme = raw.split("://").nth(1).unwrap_or(&raw);
            (
                "domain",
                without_scheme
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("*.")
                    .to_string(),
            )
        } else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        added+=transaction.execute("INSERT OR IGNORE INTO targets(project_id,target_type,value,normalized_value) VALUES(?1,?2,?3,lower(?3))",params![project_id,target_type,value]).map_err(|e|e.to_string())? as i64;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(added)
}
