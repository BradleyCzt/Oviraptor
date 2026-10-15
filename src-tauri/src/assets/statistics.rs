//! Exact saved-inventory projections. Project authorization belongs to the
//! caller; filters can never select a different project or execute scan work.
use super::{asset_filter, validate_asset_query};
use crate::asset_models::AssetQuery;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MEMBER_FROM: &str = "FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id";
const SEARCH_SEMANTICS: &str = "assets_list_like_8_fields";
const READ_CONSISTENCY: &str = "single_sqlite_read_snapshot";
const ORDER: &str = "asset_id_asc";
// A server-generated inventory cursor is not an opaque provider credential.
// The explicit format also survives the existing generic secret scrubber for
// small project/asset IDs whose plain base64 token happens to be 24..200 bytes.
const CURSOR_PREFIX: &str = "asset-page:";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectAssetFilter {
    pub query: String,
    pub deleted_view: String,
    pub decision_view: String,
    pub probe_view: String,
    pub probe_outcome_view: String,
}

impl Default for ProjectAssetFilter {
    fn default() -> Self {
        Self {
            query: String::new(),
            deleted_view: "active".into(),
            decision_view: "all".into(),
            probe_view: "all".into(),
            probe_outcome_view: "all".into(),
        }
    }
}

impl ProjectAssetFilter {
    fn normalized(&self) -> Result<Self, String> {
        let filter = Self {
            query: self.query.trim().to_string(),
            deleted_view: self.deleted_view.trim().to_string(),
            decision_view: self.decision_view.trim().to_string(),
            probe_view: self.probe_view.trim().to_string(),
            probe_outcome_view: self.probe_outcome_view.trim().to_string(),
        };
        if filter.query.contains('\0') {
            return Err("asset_query_invalid".into());
        }
        if filter.query.chars().count() > 120 {
            return Err("asset_query_too_long".into());
        }
        if !matches!(filter.deleted_view.as_str(), "active" | "trash" | "all")
            || !matches!(
                filter.decision_view.as_str(),
                "all"
                    | "review"
                    | "pending"
                    | "uncertain"
                    | "confirmed"
                    | "rejected"
                    | "not_applicable"
            )
            || !matches!(
                filter.probe_view.as_str(),
                "all"
                    | "browser_review"
                    | "browser_accessible"
                    | "restricted"
                    | "service"
                    | "abnormal"
                    | "blocked"
            )
            || !matches!(
                filter.probe_outcome_view.as_str(),
                "all"
                    | "alive_clean"
                    | "web_alive"
                    | "web_restricted"
                    | "browser_render_required"
                    | "virtual_host_required"
                    | "web_abnormal"
                    | "tcp_alive_non_http"
                    | "blocked_content"
                    | "unreachable"
                    | "skipped"
            )
        {
            return Err("asset_filter_invalid".into());
        }
        Ok(filter)
    }

    fn asset_query(&self, project_id: i64) -> Result<AssetQuery, String> {
        let query = AssetQuery {
            project_id: Some(project_id),
            search: self.query.clone(),
            deleted_view: self.deleted_view.clone(),
            decision_view: self.decision_view.clone(),
            probe_view: self.probe_view.clone(),
            probe_outcome_view: self.probe_outcome_view.clone(),
            ..Default::default()
        };
        validate_asset_query(&query)?;
        Ok(query)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InventoryScope {
    project_id: i64,
    unit: &'static str,
    inventory: &'static str,
    filters: ProjectAssetFilter,
    search_semantics: &'static str,
}

impl InventoryScope {
    fn new(project_id: i64, filters: ProjectAssetFilter) -> Self {
        Self {
            project_id,
            unit: "saved_asset_members",
            inventory: "saved_not_current_liveness",
            filters,
            search_semantics: SEARCH_SEMANTICS,
        }
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DecisionCounts {
    pending: i64,
    uncertain: i64,
    confirmed: i64,
    rejected: i64,
    not_applicable: i64,
    unknown: i64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeOutcomeCounts {
    alive_clean: i64,
    web_alive: i64,
    web_restricted: i64,
    browser_render_required: i64,
    virtual_host_required: i64,
    web_abnormal: i64,
    tcp_alive_non_http: i64,
    blocked_content: i64,
    unreachable: i64,
    skipped: i64,
    unclassified: i64,
    unknown: i64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HttpStatusCounts {
    informational: i64,
    success: i64,
    redirect: i64,
    client_error: i64,
    server_error: i64,
    unclassified: i64,
    unknown: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExclusionCounts {
    project_decision_excluded: i64,
    ownership_excluded: i64,
    relationship: &'static str,
    denominator: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectInventoryStats {
    scope: InventoryScope,
    as_of: String,
    read_consistency: &'static str,
    search_semantics: &'static str,
    project_total: i64,
    active_total: i64,
    trashed_total: i64,
    unknown_archive_state: i64,
    filtered_total: i64,
    by_decision: DecisionCounts,
    by_probe_outcome: ProbeOutcomeCounts,
    by_http_status_class: HttpStatusCounts,
    exclusions: ExclusionCounts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedAssetItem {
    id: i64,
    host: String,
    title: String,
    protocol: String,
    port: String,
    status_code: String,
    decision: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedAssetPage {
    items: Vec<SavedAssetItem>,
    total: i64,
    returned: usize,
    limit: i64,
    has_more: bool,
    next_cursor: Option<String>,
    as_of: String,
    read_consistency: &'static str,
    search_semantics: &'static str,
    scope: InventoryScope,
    order: &'static str,
}

// This token binds a page position, not authorization or a durable snapshot.
// The caller supplies the independently authorized project on every request.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    version: u8,
    project_id: i64,
    filter_sha256: String,
    order: String,
    after_asset_id: i64,
}

fn cursor_filter_hash(filters: &ProjectAssetFilter) -> Result<String, String> {
    let bytes = serde_json::to_vec(filters).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn read_cursor(
    token: Option<&str>,
    project_id: i64,
    filters: &ProjectAssetFilter,
) -> Result<i64, String> {
    let Some(token) = token else { return Ok(0) };
    if token.is_empty() || token.len() > 1024 {
        return Err("asset_cursor_invalid".into());
    }
    let encoded = token
        .strip_prefix(CURSOR_PREFIX)
        .ok_or("asset_cursor_invalid")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "asset_cursor_invalid".to_string())?;
    let cursor: Cursor =
        serde_json::from_slice(&bytes).map_err(|_| "asset_cursor_invalid".to_string())?;
    if cursor.version != 1
        || cursor.project_id != project_id
        || cursor.order != ORDER
        || cursor.after_asset_id <= 0
        || cursor.filter_sha256 != cursor_filter_hash(filters)?
    {
        return Err("asset_cursor_scope_changed".into());
    }
    Ok(cursor.after_asset_id)
}

fn next_cursor(
    project_id: i64,
    filters: &ProjectAssetFilter,
    after_asset_id: i64,
) -> Result<String, String> {
    let cursor = Cursor {
        version: 1,
        project_id,
        filter_sha256: cursor_filter_hash(filters)?,
        order: ORDER.into(),
        after_asset_id,
    };
    Ok(format!(
        "{CURSOR_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(|error| error.to_string())?)
    ))
}

fn read_snapshot<T>(
    connection: &Connection,
    project_id: i64,
    read: impl FnOnce(&Connection, String) -> Result<T, String>,
) -> Result<T, String> {
    if project_id <= 0 {
        return Err("asset_project_invalid".into());
    }
    if connection.is_autocommit() {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let result = read_existing_snapshot(&transaction, project_id, read)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    } else {
        read_existing_snapshot(connection, project_id, read)
    }
}

fn read_existing_snapshot<T>(
    connection: &Connection,
    project_id: i64,
    read: impl FnOnce(&Connection, String) -> Result<T, String>,
) -> Result<T, String> {
    // This first DB read pins the snapshot before the clock and all counts.
    let exists = connection
        .query_row("SELECT id FROM projects WHERE id=?1", [project_id], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map_err(|error| error.to_string())?;
    if exists.is_none() {
        return Err("asset_project_not_found".into());
    }
    let as_of = connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?;
    read(connection, as_of)
}

pub(crate) fn stats_on(
    connection: &Connection,
    project_id: i64,
    filters: &ProjectAssetFilter,
) -> Result<ProjectInventoryStats, String> {
    let filters = filters.normalized()?;
    let query = filters.asset_query(project_id)?;
    let (filter, values) = asset_filter(&query, true);
    read_snapshot(connection, project_id, |connection, as_of| {
        let (project_total, active_total, trashed_total, unknown_archive_state) = connection.query_row(
            &format!("SELECT COUNT(DISTINCT a.id),COUNT(DISTINCT CASE WHEN pa.is_deleted=0 THEN a.id END),COUNT(DISTINCT CASE WHEN pa.is_deleted=1 THEN a.id END),COUNT(DISTINCT CASE WHEN pa.is_deleted IS NULL OR pa.is_deleted NOT IN (0,1) THEN a.id END) {MEMBER_FROM} WHERE pa.project_id=?1"), [project_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))
        ).map_err(|error| error.to_string())?;
        let (filtered_total, project_decision_excluded, ownership_excluded) = connection.query_row(
            &format!("SELECT COUNT(DISTINCT a.id),COUNT(DISTINCT CASE WHEN pa.decision IN ('rejected','not_applicable') THEN a.id END),COUNT(DISTINCT CASE WHEN o.status='excluded' THEN a.id END) {MEMBER_FROM} WHERE {filter}"), params_from_iter(values.iter()),
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))
        ).map_err(|error| error.to_string())?;
        let mut by_decision = DecisionCounts::default();
        let mut by_probe_outcome = ProbeOutcomeCounts::default();
        let mut by_http_status_class = HttpStatusCounts::default();
        // Stream groups into fixed output buckets. Arbitrary historic values
        // never become additional output keys or disclose their original text.
        for (column, dimension) in [
            ("pa.decision", 0),
            ("a.probe_outcome", 1),
            ("a.status_code", 2),
        ] {
            let sql = format!("SELECT COALESCE({column},''),COUNT(DISTINCT a.id) {MEMBER_FROM} WHERE {filter} GROUP BY COALESCE({column},'')");
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params_from_iter(values.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (value, count) = row.map_err(|error| error.to_string())?;
                match dimension {
                    0 => {
                        *match value.as_str() {
                            "" | "pending" => &mut by_decision.pending,
                            "uncertain" => &mut by_decision.uncertain,
                            "confirmed" => &mut by_decision.confirmed,
                            "rejected" => &mut by_decision.rejected,
                            "not_applicable" => &mut by_decision.not_applicable,
                            _ => &mut by_decision.unknown,
                        } += count
                    }
                    1 => {
                        *match value.as_str() {
                            "" => &mut by_probe_outcome.unclassified,
                            "alive_clean" => &mut by_probe_outcome.alive_clean,
                            "web_alive" => &mut by_probe_outcome.web_alive,
                            "web_restricted" => &mut by_probe_outcome.web_restricted,
                            "browser_render_required" => {
                                &mut by_probe_outcome.browser_render_required
                            }
                            "virtual_host_required" => &mut by_probe_outcome.virtual_host_required,
                            "web_abnormal" => &mut by_probe_outcome.web_abnormal,
                            "tcp_alive_non_http" => &mut by_probe_outcome.tcp_alive_non_http,
                            "blocked_content" => &mut by_probe_outcome.blocked_content,
                            "unreachable" => &mut by_probe_outcome.unreachable,
                            "skipped" => &mut by_probe_outcome.skipped,
                            _ => &mut by_probe_outcome.unknown,
                        } += count
                    }
                    _ => {
                        *match http_status_class(&value) {
                            "unclassified" => &mut by_http_status_class.unclassified,
                            "1xx" => &mut by_http_status_class.informational,
                            "2xx" => &mut by_http_status_class.success,
                            "3xx" => &mut by_http_status_class.redirect,
                            "4xx" => &mut by_http_status_class.client_error,
                            "5xx" => &mut by_http_status_class.server_error,
                            _ => &mut by_http_status_class.unknown,
                        } += count
                    }
                }
            }
        }
        Ok(ProjectInventoryStats {
            scope: InventoryScope::new(project_id, filters),
            as_of,
            read_consistency: READ_CONSISTENCY,
            search_semantics: SEARCH_SEMANTICS,
            project_total,
            active_total,
            trashed_total,
            unknown_archive_state,
            filtered_total,
            by_decision,
            by_probe_outcome,
            by_http_status_class,
            exclusions: ExclusionCounts {
                project_decision_excluded,
                ownership_excluded,
                relationship: "independent_axes_may_overlap_not_archive_state",
                denominator: "filteredTotal",
            },
        })
    })
}

fn http_status_class(value: &str) -> &'static str {
    if value.is_empty() {
        return "unclassified";
    }
    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return "unknown";
    }
    match value.as_bytes()[0] {
        b'1' => "1xx",
        b'2' => "2xx",
        b'3' => "3xx",
        b'4' => "4xx",
        b'5' => "5xx",
        _ => "unknown",
    }
}

fn bounded_text(value: String, max_json_bytes: usize) -> String {
    // Redact complete authorities first: clipping before a distant '@' could
    // turn a credential-bearing URL into an apparently harmless text prefix.
    let redacted = crate::agent_runtime::secrets::redact_text_with(&value, None);
    let mut result = String::new();
    let mut encoded_len = 2; // surrounding JSON quotes
    for character in redacted.chars() {
        // Measure actual JSON escaping, including six-byte control escapes.
        let additional = serde_json::to_vec(&character.to_string())
            .expect("a scalar string always serializes")
            .len()
            - 2;
        if encoded_len + additional > max_json_bytes {
            break;
        }
        result.push(character);
        encoded_len += additional;
    }
    result
}

pub(crate) fn search_on(
    connection: &Connection,
    project_id: i64,
    filters: &ProjectAssetFilter,
    limit: i64,
    cursor: Option<&str>,
) -> Result<SavedAssetPage, String> {
    if !(1..=20).contains(&limit) {
        return Err("asset_search_limit_invalid".into());
    }
    let filters = filters.normalized()?;
    let query = filters.asset_query(project_id)?;
    let (filter, values) = asset_filter(&query, true);
    let after_id = read_cursor(cursor, project_id, &filters)?;
    read_snapshot(connection, project_id, |connection, as_of| {
        let total = connection
            .query_row(
                &format!("SELECT COUNT(DISTINCT a.id) {MEMBER_FROM} WHERE {filter}"),
                params_from_iter(values.iter()),
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut page_values = values;
        page_values.push(SqlValue::Integer(after_id));
        page_values.push(SqlValue::Integer(limit + 1));
        let sql = format!("SELECT DISTINCT a.id,a.host,a.title,a.protocol,a.port,a.status_code,COALESCE(pa.decision,'') {MEMBER_FROM} WHERE ({filter}) AND a.id>? ORDER BY a.id ASC LIMIT ?");
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| error.to_string())?;
        let mut items = statement
            .query_map(params_from_iter(page_values.iter()), |row| {
                let decision: String = row.get(6)?;
                Ok(SavedAssetItem {
                    id: row.get(0)?,
                    host: bounded_text(row.get(1)?, 256),
                    title: bounded_text(row.get(2)?, 256),
                    protocol: bounded_text(row.get(3)?, 32),
                    port: bounded_text(row.get(4)?, 16),
                    status_code: bounded_text(row.get(5)?, 16),
                    decision: match decision.as_str() {
                        "" | "pending" => "pending",
                        "uncertain" => "uncertain",
                        "confirmed" => "confirmed",
                        "rejected" => "rejected",
                        "not_applicable" => "not_applicable",
                        _ => "unknown",
                    }
                    .into(),
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = if has_more {
            Some(next_cursor(
                project_id,
                &filters,
                items.last().ok_or("asset_cursor_empty_page")?.id,
            )?)
        } else {
            None
        };
        Ok(SavedAssetPage {
            returned: items.len(),
            items,
            total,
            limit,
            has_more,
            next_cursor,
            as_of,
            read_consistency: READ_CONSISTENCY,
            search_semantics: SEARCH_SEMANTICS,
            scope: InventoryScope::new(project_id, filters),
            order: ORDER,
        })
    })
}

#[cfg(test)]
#[path = "statistics_tests.rs"]
mod tests;
