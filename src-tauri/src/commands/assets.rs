//! Asset inventory, targets, decisions and asset-pipeline history.
//! Scan status is a read projection; this module cannot invoke scan execution.
use crate::{db, models::{
    Asset, AssetBulkArchiveInput, AssetBulkDecisionInput, AssetEvent, AssetPage,
    AssetQuery, AssetSummary, ContentRuleApplyResult, ContentRuleInput, DecisionInput,
    InterruptedJob, JobRun, LogEntry, StaleProject, StartupStatus, Target, TargetImportInput,
}, AppState};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Row};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use tauri::State;

pub(super) fn detect_target(value: &str) -> &'static str {
    let value = value.trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        return "domain";
    }
    if value.parse::<std::net::IpAddr>().is_ok() {
        return "ip";
    }
    if let Some((address, prefix)) = value.split_once('/') {
        if let (Ok(address), Ok(prefix)) =
            (address.parse::<std::net::IpAddr>(), prefix.parse::<u8>())
        {
            let valid = match address {
                std::net::IpAddr::V4(_) => prefix <= 32,
                std::net::IpAddr::V6(_) => prefix <= 128,
            };
            if valid {
                return "cidr";
            }
        }
    }
    if value.contains('.') && !value.contains(' ') {
        return "domain";
    }
    "company"
}

#[tauri::command]
pub fn import_targets(state: State<AppState>, input: TargetImportInput) -> Result<i64, String> {
    if !["auto", "company", "domain", "ip", "cidr", "icp", "asn"]
        .contains(&input.target_type.as_str())
    {
        return Err("不支持的目标类型".into());
    }
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut inserted = 0;
    for raw in input.values {
        let value = raw.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let target_type = if input.target_type == "auto" {
            detect_target(value)
        } else {
            input.target_type.as_str()
        };
        let normalized = value.to_lowercase();
        inserted += transaction.execute(
            "INSERT OR IGNORE INTO targets(project_id,target_type,value,normalized_value) VALUES(?1,?2,?3,?4)",
            params![input.project_id, target_type, value, normalized],
        ).map_err(|error| error.to_string())? as i64;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(inserted)
}

#[tauri::command]
pub fn list_targets(state: State<AppState>, project_id: i64) -> Result<Vec<Target>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,project_id,target_type,value,enabled,created_at FROM targets WHERE project_id=?1 ORDER BY id DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok(Target {
                id: row.get(0)?,
                project_id: row.get(1)?,
                target_type: row.get(2)?,
                value: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn remove_target(state: State<AppState>, target_id: i64) -> Result<(), String> {
    db::open(&state.db_path)?
        .execute("DELETE FROM targets WHERE id=?1", [target_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn field_sql(field: &str) -> Option<&'static str> {
    Some(match field {
        "assetKey" => "a.asset_key",
        "projectName" => "p.name",
        "company" => "a.company",
        "host" => "a.host",
        "link" => "a.link",
        "ip" => "a.ip",
        "port" => "a.port",
        "protocol" => "a.protocol",
        "domain" => "a.domain",
        "title" => "a.title",
        "statusCode" => "a.status_code",
        "probeOutcome" => "a.probe_outcome",
        "probeEntryState" => "a.probe_entry_state",
        "reviewTier" => "a.review_tier",
        "contentCategory" => "a.content_category",
        "score" => "a.score",
        "decision" => "pa.decision",
        "note" => "pa.note",
        "ownershipStatus" => "COALESCE(o.status,'unreviewed')",
        "ownershipConfidence" => "COALESCE(o.confidence,0)",
        "authorizationStatus" => "COALESCE(o.authorization_status,'unknown')",
        "exposureEligible" => "COALESCE(o.exposure_eligible,0)",
        "ownershipSource" => "COALESCE(o.source,'')",
        "ownershipReason" => "COALESCE(o.reason,'')",
        "isDeleted" => "pa.is_deleted",
        "firstSeen" => "a.first_seen",
        "lastSeen" => "a.last_seen",
        "lastAlive" => "a.last_alive",
        "projectFirstSeen" => "pa.first_seen",
        "projectLastSeen" => "pa.last_seen",
        "lastRunId" => "pa.last_run_id",
        "deletedAt" => "pa.deleted_at",
        "hasLogin" => "a.has_login",
        "hasRegister" => "a.has_register",
        "entrySeen" => "CASE WHEN COALESCE(pa.entry_opened_at,'')<>'' THEN '1' ELSE '0' END",
        "entryOpenedAt" => "pa.entry_opened_at",
        "sentinelStatus" => "COALESCE((SELECT COALESCE(ss.status,st.status,'sent') FROM sentinel_targets st LEFT JOIN sentinel_scans ss ON ss.id=st.scan_id WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))) ORDER BY st.updated_at DESC LIMIT 1),'not_sent')",
        "sentinelScanCount" => "(SELECT COUNT(DISTINCT st.scan_id) FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host)))",
        "sentinelSentAt" => "(SELECT MAX(st.updated_at) FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host)))",
        _ => return None,
    })
}

fn numeric_asset_field(field: &str) -> bool {
    matches!(
        field,
        "port" | "statusCode" | "score" | "isDeleted" | "lastRunId" | "sentinelScanCount" | "ownershipConfidence" | "exposureEligible" | "hasLogin" | "hasRegister"
    )
}

fn date_asset_field(field: &str) -> bool {
    matches!(
        field,
        "firstSeen"
            | "lastSeen"
            | "lastAlive"
            | "projectFirstSeen"
            | "projectLastSeen"
            | "deletedAt"
            | "sentinelSentAt"
            | "entryOpenedAt"
    )
}

pub(super) fn asset_filter(query: &AssetQuery, apply_decision_view: bool) -> (String, Vec<SqlValue>) {
    let deleted_view = if query.deleted_view.trim().is_empty() {
        if query.include_deleted {
            "all"
        } else {
            "active"
        }
    } else {
        query.deleted_view.as_str()
    };
    let mut where_parts = vec![match deleted_view {
        "trash" => "pa.is_deleted=1".to_string(),
        "all" => "1=1".to_string(),
        _ => "pa.is_deleted=0".to_string(),
    }];
    let mut values = Vec::new();
    if let Some(id) = query.project_id {
        where_parts.push("pa.project_id=?".into());
        values.push(SqlValue::Integer(id));
    }
    match query.probe_view.as_str() {
        "browser_review" => where_parts.push("COALESCE(a.probe_outcome,'') IN ('','alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule')".into()),
        "browser_accessible" => where_parts.push("a.probe_outcome IN ('alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule')".into()),
        "restricted" => where_parts.push("a.probe_outcome IN ('web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule')".into()),
        "service" => where_parts.push("a.probe_outcome='tcp_alive_non_http'".into()),
        "abnormal" => where_parts.push("a.probe_outcome IN ('web_abnormal','unreachable','skipped')".into()),
        "blocked" => where_parts.push("(a.probe_outcome='blocked_content' OR a.content_category IN ('gambling','porn','custom_rule'))".into()),
        _ => {}
    }
    if matches!(
        query.probe_outcome_view.as_str(),
        "web_alive"
            | "web_restricted"
            | "browser_render_required"
            | "virtual_host_required"
            | "web_abnormal"
            | "tcp_alive_non_http"
            | "blocked_content"
            | "unreachable"
            | "skipped"
            | "alive_clean"
    ) {
        where_parts.push("a.probe_outcome=?".into());
        values.push(SqlValue::Text(query.probe_outcome_view.clone()));
    }
    let sentinel_exists = "EXISTS (SELECT 1 FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))))";
    match query.sentinel_view.as_str() {
        "sent" => where_parts.push(sentinel_exists.into()),
        "not_sent" => where_parts.push(format!("NOT {sentinel_exists}")),
        _ => {}
    }
    if apply_decision_view {
        match query.decision_view.as_str() {
            "review" => {
                where_parts.push("COALESCE(pa.decision,'') IN ('','pending','uncertain')".into())
            }
            "pending" => where_parts.push("COALESCE(pa.decision,'') IN ('','pending')".into()),
            "uncertain" => where_parts.push("pa.decision='uncertain'".into()),
            "confirmed" => where_parts.push("pa.decision='confirmed'".into()),
            "rejected" => where_parts.push("pa.decision='rejected'".into()),
            "not_applicable" => where_parts.push("pa.decision='not_applicable'".into()),
            "ownership_scope" => where_parts.push("COALESCE(pa.decision,'pending') NOT IN ('rejected','not_applicable')".into()),
            _ => {}
        }
    }
    match query.ownership_view.as_str() {
        "unreviewed" => where_parts.push("COALESCE(o.status,'unreviewed')='unreviewed'".into()),
        "attributed" => where_parts.push("o.status='attributed'".into()),
        "confirmed" => where_parts.push("o.status='confirmed'".into()),
        "related" => where_parts.push("o.status='related'".into()),
        "third_party" => where_parts.push("o.status='third_party'".into()),
        "excluded" => where_parts.push("o.status='excluded'".into()),
        // O0：与归属门禁摘要卡（asset_ownership_summary.exposure_eligible）及下游暴露面范围同一口径：
        // 只看 exposure_eligible=1。旧口径额外要求 confirmed+allowed，会让卡片数字与列表总数对不上，
        // 并把下游实际会消费的异常行（例如历史遗留的非 confirmed 准入行）从预览里藏起来。
        "eligible" => where_parts.push("COALESCE(o.exposure_eligible,0)=1".into()),
        "review" => where_parts.push("COALESCE(o.status,'unreviewed') IN ('unreviewed','attributed')".into()),
        _ => {}
    }
    match query.has_login_view.as_str() {
        "yes" | "1" => where_parts.push("a.has_login=1".into()),
        "no" | "0" => where_parts.push("a.has_login=0".into()),
        "unknown" | "-1" => where_parts.push("a.has_login=-1".into()),
        _ => {}
    }
    match query.has_register_view.as_str() {
        "yes" | "1" => where_parts.push("a.has_register=1".into()),
        "no" | "0" => where_parts.push("a.has_register=0".into()),
        "unknown" | "-1" => where_parts.push("a.has_register=-1".into()),
        _ => {}
    }
    match query.entry_seen_view.as_str() {
        "seen" | "yes" | "1" => where_parts.push("COALESCE(pa.entry_opened_at,'')<>''".into()),
        "unseen" | "no" | "0" => where_parts.push("COALESCE(pa.entry_opened_at,'')=''".into()),
        _ => {}
    }
    if !query.search.trim().is_empty() {
        let pattern = format!("%{}%", query.search.trim());
        where_parts.push("(p.name LIKE ? OR a.company LIKE ? OR a.host LIKE ? OR a.link LIKE ? OR a.ip LIKE ? OR a.domain LIKE ? OR a.title LIKE ? OR a.asset_key LIKE ?)".into());
        for _ in 0..8 {
            values.push(SqlValue::Text(pattern.clone()));
        }
    }
    for condition in &query.conditions {
        let Some(column) = field_sql(&condition.field) else {
            continue;
        };
        if condition.value.trim().is_empty()
            && !matches!(condition.operator.as_str(), "isEmpty" | "notEmpty")
        {
            continue;
        }
        let expression = match condition.operator.as_str() {
            "equals" => {
                values.push(SqlValue::Text(condition.value.clone()));
                format!("{column}=?")
            }
            "notEquals" => {
                values.push(SqlValue::Text(condition.value.clone()));
                format!("{column}<>?")
            }
            "startsWith" => {
                values.push(SqlValue::Text(format!("{}%", condition.value)));
                format!("{column} LIKE ?")
            }
            "endsWith" => {
                values.push(SqlValue::Text(format!("%{}", condition.value)));
                format!("{column} LIKE ?")
            }
            "notContains" => {
                values.push(SqlValue::Text(format!("%{}%", condition.value)));
                format!("{column} NOT LIKE ?")
            }
            "isEmpty" => format!("COALESCE({column},'')=''"),
            "notEmpty" => format!("COALESCE({column},'')<>''"),
            "gte" => {
                values.push(SqlValue::Text(condition.value.clone()));
                if numeric_asset_field(&condition.field) {
                    format!("CAST({column} AS REAL)>=CAST(? AS REAL)")
                } else if date_asset_field(&condition.field) {
                    format!("COALESCE({column},'')>=?")
                } else {
                    format!("{column}>=?")
                }
            }
            "lte" => {
                values.push(SqlValue::Text(condition.value.clone()));
                if numeric_asset_field(&condition.field) {
                    format!("CAST({column} AS REAL)<=CAST(? AS REAL)")
                } else if date_asset_field(&condition.field) {
                    format!("COALESCE({column},'')<=?")
                } else {
                    format!("{column}<=?")
                }
            }
            _ => {
                values.push(SqlValue::Text(format!("%{}%", condition.value)));
                format!("{column} LIKE ?")
            }
        };
        let join = if condition.join.eq_ignore_ascii_case("or") {
            " OR "
        } else {
            " AND "
        };
        if join == " OR " && where_parts.len() > 1 {
            let previous = where_parts.pop().unwrap();
            where_parts.push(format!("({previous} OR {expression})"));
        } else {
            where_parts.push(expression);
        }
    }
    (where_parts.join(" AND "), values)
}

fn asset_order(query: &AssetQuery) -> String {
    let direction = if query.sort_direction.eq_ignore_ascii_case("asc") {
        "ASC"
    } else {
        "DESC"
    };
    let expression = match query.sort_by.as_str() {
        "company" => "a.company COLLATE NOCASE",
        "host" => "a.host COLLATE NOCASE",
        "title" => "a.title COLLATE NOCASE",
        "statusCode" => "CAST(COALESCE(NULLIF(a.status_code,''),'0') AS INTEGER)",
        "score" => "CAST(COALESCE(NULLIF(a.score,''),'0') AS REAL)",
        "decision" => "pa.decision",
        "ownershipStatus" => "COALESCE(o.status,'unreviewed')",
        "ownershipConfidence" => "COALESCE(o.confidence,0)",
        "exposureEligible" => "COALESCE(o.exposure_eligible,0)",
        "probeOutcome" => "a.probe_outcome",
        "hasLogin" => "a.has_login",
        "hasRegister" => "a.has_register",
        "entrySeen" => "CASE WHEN COALESCE(pa.entry_opened_at,'')='' THEN 0 ELSE 1 END",
        "firstSeen" => "a.first_seen",
        "projectLastSeen" => "pa.last_seen",
        "lastAlive" => "a.last_alive",
        _ => return "CASE WHEN a.review_tier LIKE 'P1%' THEN 1 WHEN a.review_tier LIKE 'P2%' THEN 2 WHEN a.review_tier LIKE 'P3%' THEN 3 ELSE 4 END, CAST(COALESCE(NULLIF(a.score,''),'0') AS REAL) DESC, a.last_seen DESC,a.id DESC".into(),
    };
    format!("{expression} {direction},a.id DESC")
}

pub(super) fn asset_from_row(row: &Row<'_>) -> rusqlite::Result<Asset> {
    Ok(Asset {
        id: row.get(0)?,
        project_id: row.get(1)?,
        asset_key: row.get(2)?,
        company: row.get(3)?,
        host: row.get(4)?,
        link: row.get(5)?,
        ip: row.get(6)?,
        port: row.get(7)?,
        protocol: row.get(8)?,
        domain: row.get(9)?,
        title: row.get(10)?,
        status_code: row.get(11)?,
        probe_outcome: row.get(12)?,
        probe_entry_state: row.get(13)?,
        review_tier: row.get(14)?,
        content_category: row.get(15)?,
        score: row.get(16)?,
        decision: row.get(17)?,
        note: row.get(18)?,
        ownership_status: row.get(19)?,
        ownership_confidence: row.get(20)?,
        authorization_status: row.get(21)?,
        exposure_eligible: row.get::<_, i64>(22)? != 0,
        ownership_source: row.get(23)?,
        ownership_reason: row.get(24)?,
        is_deleted: row.get::<_, i64>(25)? != 0,
        first_seen: row.get(26)?,
        last_seen: row.get(27)?,
        last_alive: row.get(28)?,
        extra: serde_json::from_str(&row.get::<_, String>(29)?)
            .unwrap_or_else(|_| serde_json::json!({})),
        sentinel_status: row.get(30)?,
        sentinel_scan_count: row.get(31)?,
        sentinel_sent_at: row.get(32)?,
        project_first_seen: row.get(33)?,
        project_last_seen: row.get(34)?,
        last_run_id: row.get(35)?,
        deleted_at: row.get(36)?,
        project_name: row.get(37)?,
        has_login: row.get(38)?,
        has_register: row.get(39)?,
        entry_opened_at: row.get(40)?,
    })
}

pub(super) const ASSET_SELECT: &str = r#"SELECT a.id,pa.project_id,a.asset_key,a.company,a.host,a.link,a.ip,a.port,a.protocol,a.domain,a.title,
 a.status_code,a.probe_outcome,a.probe_entry_state,a.review_tier,a.content_category,a.score,pa.decision,pa.note,
 COALESCE(o.status,'unreviewed'),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.source,''),COALESCE(o.reason,''),pa.is_deleted,
 a.first_seen,a.last_seen,a.last_alive,a.extra_json,
 COALESCE((SELECT COALESCE(ss.status,st.status,'sent') FROM sentinel_targets st LEFT JOIN sentinel_scans ss ON ss.id=st.scan_id WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))) ORDER BY st.updated_at DESC LIMIT 1),'not_sent'),
 (SELECT COUNT(DISTINCT st.scan_id) FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host)))),
 (SELECT MAX(st.updated_at) FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host)))),
 pa.first_seen,pa.last_seen,pa.last_run_id,pa.deleted_at,p.name,
 a.has_login,a.has_register,pa.entry_opened_at
 FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id"#;

#[tauri::command]
pub fn mark_asset_entry_opened(
    state: State<AppState>,
    project_id: i64,
    asset_id: i64,
) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let changed = connection
        .execute(
            "UPDATE project_assets SET entry_opened_at=datetime('now','localtime') WHERE project_id=?1 AND asset_id=?2",
            params![project_id, asset_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("这条资产不在当前项目里，没有标记为已看".into());
    }
    connection
        .query_row(
            "SELECT entry_opened_at FROM project_assets WHERE project_id=?1 AND asset_id=?2",
            params![project_id, asset_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_assets(
    state: State<'_, AppState>,
    query: AssetQuery,
) -> Result<AssetPage, String> {
    let connection = db::open(&state.db_path)?;
    let (filter, values) = asset_filter(&query, true);
    let (summary_filter, summary_values) = asset_filter(&query, false);
    let count_sql = format!(
        "SELECT COUNT(*) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id WHERE {filter}"
    );
    let total: i64 = connection
        .query_row(&count_sql, params_from_iter(values.iter()), |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    let sentinel_exists = "EXISTS (SELECT 1 FROM sentinel_targets st WHERE st.project_id=pa.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))))";
    let summary_sql = format!(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN COALESCE(pa.decision,'') IN ('','pending') THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN pa.decision='uncertain' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN pa.decision='confirmed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN pa.decision='rejected' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN pa.decision='not_applicable' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN {sentinel_exists} THEN 1 ELSE 0 END),0) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id WHERE {summary_filter}"
    );
    let summary = connection
        .query_row(
            &summary_sql,
            params_from_iter(summary_values.iter()),
            |row| {
                Ok(AssetSummary {
                    all: row.get(0)?,
                    pending: row.get(1)?,
                    uncertain: row.get(2)?,
                    confirmed: row.get(3)?,
                    rejected: row.get(4)?,
                    not_applicable: row.get(5)?,
                    sent_to_agent: row.get(6)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    let page = query.page.max(1);
    // The table is not virtualised.  Rendering 500-1000 rows creates tens of
    // thousands of Vue bindings and can trigger the macOS busy cursor.
    let page_size = query.page_size.clamp(10, 200);
    let sql = format!(
        "{ASSET_SELECT} WHERE {filter} ORDER BY {} LIMIT ? OFFSET ?",
        asset_order(&query)
    );
    let mut list_values = values;
    list_values.push(SqlValue::Integer(page_size));
    list_values.push(SqlValue::Integer((page - 1) * page_size));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(list_values.iter()), asset_from_row)
        .map_err(|error| error.to_string())?;
    Ok(AssetPage {
        items: rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?,
        total,
        page,
        page_size,
        summary,
    })
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[tauri::command]
pub async fn add_content_rule(
    state: State<'_, AppState>,
    input: ContentRuleInput,
) -> Result<ContentRuleApplyResult, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let keyword = input
            .keyword
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        let length = keyword.chars().count();
        if !(2..=200).contains(&length) {
            return Err("内容规则需为 2–200 个字符".to_string());
        }
        let normalized = keyword.to_lowercase();
        let pattern = format!("%{}%", escape_like(&keyword));
        let mut connection = db::open(&db_path)?;
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO content_rules(keyword,normalized_keyword,source_asset_id) VALUES(?1,?2,?3) ON CONFLICT(normalized_keyword) DO UPDATE SET keyword=excluded.keyword,source_asset_id=COALESCE(excluded.source_asset_id,content_rules.source_asset_id),enabled=1,updated_at=datetime('now','localtime')",
                params![keyword, normalized, input.source_asset_id],
            )
            .map_err(|error| error.to_string())?;
        let rule_id: i64 = transaction
            .query_row(
                "SELECT id FROM content_rules WHERE normalized_keyword=?1",
                [&normalized],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(
                "DROP TABLE IF EXISTS temp.matched_content_rule_assets; CREATE TEMP TABLE matched_content_rule_assets(asset_id INTEGER PRIMARY KEY);",
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO matched_content_rule_assets(asset_id) SELECT id FROM assets WHERE title LIKE ?1 ESCAPE '\\'",
                [&pattern],
            )
            .map_err(|error| error.to_string())?;
        let matched_assets: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM matched_content_rule_assets",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let matched_project_assets: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM project_assets pa JOIN matched_content_rule_assets matched ON matched.asset_id=pa.asset_id",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE assets SET content_category='custom_rule' WHERE id IN (SELECT asset_id FROM matched_content_rule_assets)",
                [],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE project_assets SET decision=CASE WHEN decision IN ('pending','uncertain','','not_applicable') THEN 'rejected' ELSE decision END,note=CASE WHEN decision IN ('pending','uncertain','','not_applicable','rejected') THEN ?1 ELSE note END WHERE asset_id IN (SELECT asset_id FROM matched_content_rule_assets)",
                [format!("系统自动内容规则：标题包含「{}」", keyword)],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch("DROP TABLE matched_content_rule_assets;")
            .map_err(|error| error.to_string())?;
        let profiles = {
            let mut statement = transaction.prepare("SELECT id,settings_json FROM config_profiles").map_err(|error| error.to_string())?;
            let rows = statement.query_map([], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?))).map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
            rows
        };
        for (profile_id, raw) in profiles {
            let mut settings: JsonValue = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
            if !settings.is_object() { settings = serde_json::json!({}); }
            let keywords = settings.as_object_mut().expect("settings object").entry("negativeKeywords").or_insert_with(|| serde_json::json!([]));
            if !keywords.is_array() { *keywords = serde_json::json!([]); }
            let list = keywords.as_array_mut().expect("negativeKeywords array");
            if !list.iter().any(|item| item.as_str().is_some_and(|value| value.eq_ignore_ascii_case(&keyword))) {
                list.push(JsonValue::String(keyword.clone()));
                transaction.execute("UPDATE config_profiles SET settings_json=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![settings.to_string(),profile_id]).map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(ContentRuleApplyResult {
            rule_id,
            keyword,
            matched_assets,
            matched_project_assets,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn update_decision(state: State<AppState>, input: DecisionInput) -> Result<(), String> {
    if ![
        "pending",
        "uncertain",
        "confirmed",
        "rejected",
        "not_applicable",
    ]
    .contains(&input.decision.as_str())
    {
        return Err("人工结论无效".into());
    }
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for asset_id in input.asset_ids {
        transaction
            .execute(
                "UPDATE project_assets SET decision=?1,note=?2 WHERE project_id=?3 AND asset_id=?4",
                params![input.decision, input.note, input.project_id, asset_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'decision',?3)",
            params![input.project_id, asset_id, format!("{}: {}", input.decision, input.note)],
        ).map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_asset_decisions(
    state: State<AppState>,
    input: AssetBulkDecisionInput,
) -> Result<i64, String> {
    if ![
        "pending",
        "uncertain",
        "confirmed",
        "rejected",
        "not_applicable",
    ]
    .contains(&input.decision.as_str())
    {
        return Err("人工结论无效".into());
    }
    if input.selections.is_empty() || input.selections.len() > 20_000 {
        return Err("请选择 1–20000 条资产".into());
    }
    let mut unique = HashSet::new();
    let selections = input
        .selections
        .into_iter()
        .filter(|item| unique.insert((item.project_id, item.asset_id)))
        .collect::<Vec<_>>();
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut changed = 0i64;
    for item in selections {
        let updated = transaction
            .execute(
                "UPDATE project_assets SET decision=?1,note=?2 WHERE project_id=?3 AND asset_id=?4",
                params![input.decision, input.note, item.project_id, item.asset_id],
            )
            .map_err(|error| error.to_string())? as i64;
        if updated == 0 {
            continue;
        }
        changed += updated;
        transaction.execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'decision',?3)",
            params![item.project_id, item.asset_id, format!("{}: {}", input.decision, input.note)],
        ).map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

#[tauri::command]
pub fn soft_delete_assets(
    state: State<'_, AppState>,
    project_id: i64,
    asset_ids: Vec<i64>,
    deleted: bool,
) -> Result<(), String> {
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for asset_id in asset_ids {
        transaction.execute(
            "UPDATE project_assets SET is_deleted=?1,deleted_at=CASE WHEN ?1=1 THEN datetime('now','localtime') ELSE NULL END WHERE project_id=?2 AND asset_id=?3",
            params![deleted as i64, project_id, asset_id],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,?3,?4)",
            params![project_id, asset_id, if deleted { "archived" } else { "restored" }, if deleted { "资产已移入回收站" } else { "资产已恢复" }],
        ).map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn soft_delete_asset_selections(
    state: State<'_, AppState>,
    input: AssetBulkArchiveInput,
) -> Result<i64, String> {
    if input.selections.is_empty() || input.selections.len() > 20_000 {
        return Err("请选择 1–20000 条资产".into());
    }
    let mut unique = HashSet::new();
    let selections = input
        .selections
        .into_iter()
        .filter(|item| unique.insert((item.project_id, item.asset_id)))
        .collect::<Vec<_>>();
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut changed = 0i64;
    for item in selections {
        let updated = transaction.execute(
            "UPDATE project_assets SET is_deleted=?1,deleted_at=CASE WHEN ?1=1 THEN datetime('now','localtime') ELSE NULL END WHERE project_id=?2 AND asset_id=?3",
            params![input.deleted as i64, item.project_id, item.asset_id],
        ).map_err(|error| error.to_string())? as i64;
        if updated == 0 {
            continue;
        }
        changed += updated;
        transaction.execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,?3,?4)",
            params![item.project_id, item.asset_id, if input.deleted { "archived" } else { "restored" }, if input.deleted { "资产已移入回收站" } else { "资产已恢复" }],
        ).map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

#[tauri::command]
pub async fn list_runs(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<JobRun>, String> {
    let connection = db::open(&state.db_path)?;
    let sql = if project_id.is_some() {
        "SELECT r.id,r.project_id,r.profile_id,p.name,r.name,r.pipeline,r.status,r.stage,r.progress,r.processed,r.total,r.output_dir,r.error,r.started_at,r.finished_at,r.created_at FROM runs r JOIN projects p ON p.id=r.project_id WHERE r.project_id=?1 ORDER BY r.id DESC LIMIT ?2"
    } else {
        "SELECT r.id,r.project_id,r.profile_id,p.name,r.name,r.pipeline,r.status,r.stage,r.progress,r.processed,r.total,r.output_dir,r.error,r.started_at,r.finished_at,r.created_at FROM runs r JOIN projects p ON p.id=r.project_id ORDER BY r.id DESC LIMIT ?1"
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let mapper = |row: &Row<'_>| {
        Ok(JobRun {
            id: row.get(0)?,
            project_id: row.get(1)?,
            profile_id: row.get(2)?,
            project_name: row.get(3)?,
            name: row.get(4)?,
            pipeline: row.get(5)?,
            status: row.get(6)?,
            stage: row.get(7)?,
            progress: row.get(8)?,
            processed: row.get(9)?,
            total: row.get(10)?,
            output_dir: row.get(11)?,
            error: row.get(12)?,
            started_at: row.get(13)?,
            finished_at: row.get(14)?,
            created_at: row.get(15)?,
        })
    };
    let rows = if let Some(id) = project_id {
        statement.query_map(params![id, limit.unwrap_or(100)], mapper)
    } else {
        statement.query_map([limit.unwrap_or(100)], mapper)
    }
    .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_logs(
    state: State<'_, AppState>,
    run_id: Option<i64>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<LogEntry>, String> {
    let connection = db::open(&state.db_path)?;
    crate::asset_logs::read(&connection, run_id, project_id, limit)
}

#[tauri::command]
pub async fn list_asset_events(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    event_type: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AssetEvent>, String> {
    let connection = db::open(&state.db_path)?;
    let mut filters = Vec::new();
    let mut values = Vec::<SqlValue>::new();
    if let Some(id) = project_id {
        filters.push("e.project_id=?");
        values.push(SqlValue::Integer(id));
    }
    if let Some(kind) = event_type.filter(|value| !value.is_empty()) {
        filters.push("e.event_type=?");
        values.push(SqlValue::Text(kind));
    }
    let where_sql = if filters.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    values.push(SqlValue::Integer(limit.unwrap_or(500)));
    let sql = format!("SELECT e.id,e.project_id,e.asset_id,a.asset_key,a.company,a.host,e.event_type,e.summary,e.run_id,e.created_at FROM asset_events e JOIN assets a ON a.id=e.asset_id {where_sql} ORDER BY e.id DESC LIMIT ?");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(AssetEvent {
                id: row.get(0)?,
                project_id: row.get(1)?,
                asset_id: row.get(2)?,
                asset_key: row.get(3)?,
                company: row.get(4)?,
                host: row.get(5)?,
                event_type: row.get(6)?,
                summary: row.get(7)?,
                run_id: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn startup_status(state: State<AppState>) -> Result<StartupStatus, String> {
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "UPDATE runs SET status='interrupted',stage='interrupted',error='上次运行时应用或电脑被关闭，任务未完成',finished_at=datetime('now','localtime') WHERE status IN ('queued','running','cancel_requested')",
            [],
        )
        .map_err(|error| error.to_string())?;
    let reminder_days: i64 = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='reminder_days'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "7".into())
        .parse()
        .unwrap_or(7);

    let mut stale_statement = connection.prepare(r#"
        SELECT p.id,p.name,
               MAX(CASE WHEN r.status IN ('completed','historical_import') OR r.stage='completed' THEN COALESCE(r.finished_at,r.created_at) END) AS last_run,
               CAST(julianday('now','localtime')-julianday(MAX(CASE WHEN r.status IN ('completed','historical_import') OR r.stage='completed' THEN COALESCE(r.finished_at,r.created_at) END)) AS INTEGER) AS days_old
        FROM projects p LEFT JOIN runs r ON r.project_id=p.id
        WHERE p.status='active'
          AND (EXISTS(SELECT 1 FROM targets t WHERE t.project_id=p.id AND t.enabled=1)
               OR EXISTS(SELECT 1 FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0))
        GROUP BY p.id,p.name
        HAVING last_run IS NULL OR days_old>=?1
        ORDER BY COALESCE(days_old,999999) DESC,p.name
    "#).map_err(|error| error.to_string())?;
    let stale_projects = stale_statement
        .query_map([reminder_days], |row| {
            Ok(StaleProject {
                project_id: row.get(0)?,
                project_name: row.get(1)?,
                last_run_at: row.get(2)?,
                days_since_update: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut interrupted_statement = connection
        .prepare(
            r#"
        SELECT r.id,r.project_id,p.name,r.profile_id,r.name,r.pipeline,r.created_at
        FROM runs r JOIN projects p ON p.id=r.project_id
        WHERE r.status='interrupted' ORDER BY r.id DESC LIMIT 30
    "#,
        )
        .map_err(|error| error.to_string())?;
    let interrupted_jobs = interrupted_statement
        .query_map([], |row| {
            Ok(InterruptedJob {
                run_id: row.get(0)?,
                project_id: row.get(1)?,
                project_name: row.get(2)?,
                profile_id: row.get(3)?,
                name: row.get(4)?,
                pipeline: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(StartupStatus {
        reminder_days,
        stale_projects,
        interrupted_jobs,
    })
}

#[tauri::command]
pub fn acknowledge_interrupted_run(state: State<AppState>, run_id: i64) -> Result<(), String> {
    db::open(&state.db_path)?
        .execute(
            "UPDATE runs SET status='restarted',stage='restarted' WHERE id=?1 AND status='interrupted'",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::asset_models::AssetQuery;
    use rusqlite::types::Value as SqlValue;

    pub(crate) fn asset_filter(
        query: &AssetQuery,
        apply_decision_view: bool,
    ) -> (String, Vec<SqlValue>) {
        super::asset_filter(query, apply_decision_view)
    }

    pub(crate) fn detect_target(value: &str) -> &'static str {
        super::detect_target(value)
    }
}
