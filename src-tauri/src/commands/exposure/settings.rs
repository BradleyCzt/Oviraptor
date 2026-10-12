// §E2 — the exposure surface's own collection settings. Strict by design: an unknown field,
// an out-of-range number or a wrong type is refused with the field name, because a silently
// ignored cap is how a bounded collector turns into an unbounded one. `null` means "use the
// default", `0` means "invalid where a positive is required" — the two are never conflated.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const SETTINGS_SCHEMA_VERSION: i64 = 1;

pub const SOURCES: &[&str] = &[
    "passive_web",
    "certificate_transparency",
    "web_archive",
    "github_repositories",
    "github_code",
    "document_link",
];

/// G2: a new project starts with the three G0 sources. The GitHub sources talk to a third-party
/// platform about the group's names, so they are an explicit operator choice, never a default
/// that silently adds outbound requests to existing projects.
pub const DEFAULT_SOURCES: &[&str] = &["passive_web", "certificate_transparency", "web_archive"];

/// G2: owner logins an operator has confirmed as the group's official open-source accounts.
pub const GITHUB_OFFICIAL_OWNERS_LIMIT: usize = 50;

/// G3: explicit public document URLs an operator wants read (one GET each, no redirect).
pub const DOCUMENT_URLS_LIMIT: usize = 20;
pub const DOCUMENT_NOTE_LIMIT: usize = 200;

/// G3: one operator-specified document. `public_material` is a human declaration ("this is a
/// public standard / product manual") that excludes the mention lead with the note as reason;
/// it never hides a credential-shaped value found in the same document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentUrl {
    pub url: String,
    pub public_material: bool,
    pub note: String,
}

/// Strict parser shared by the settings and the round planner.
pub fn parse_document_urls(value: Option<&JsonValue>) -> Result<Vec<DocumentUrl>, String> {
    let Some(rows) = value.filter(|row| !row.is_null()) else {
        return Ok(Vec::new());
    };
    let list = rows
        .as_array()
        .ok_or_else(|| "exposure_setting_invalid：exposureDocumentUrls 必须是数组".to_string())?;
    let mut entries: Vec<DocumentUrl> = Vec::new();
    for row in list {
        let (raw, public_material, note) = match row {
            JsonValue::String(text) => (text.clone(), false, String::new()),
            JsonValue::Object(map) => {
                if let Some(key) = map.keys().find(|key| !matches!(key.as_str(), "url" | "publicMaterial" | "note")) {
                    return Err(format!("exposure_setting_invalid：exposureDocumentUrls 条目有未知字段 {key}"));
                }
                let url = map.get("url").and_then(JsonValue::as_str).ok_or_else(|| {
                    "exposure_setting_invalid：exposureDocumentUrls 条目缺少 url".to_string()
                })?;
                let public_material = match map.get("publicMaterial") {
                    None | Some(JsonValue::Null) => false,
                    Some(flag) => flag.as_bool().ok_or_else(|| {
                        "exposure_setting_invalid：publicMaterial 必须是布尔".to_string()
                    })?,
                };
                let note = match map.get("note") {
                    None | Some(JsonValue::Null) => String::new(),
                    Some(text) => text
                        .as_str()
                        .ok_or_else(|| "exposure_setting_invalid：note 必须是字符串".to_string())?
                        .trim()
                        .to_string(),
                };
                (url.to_string(), public_material, note)
            }
            other => {
                return Err(format!("exposure_setting_invalid：exposureDocumentUrls 条目必须是字符串或对象，收到 {other}"))
            }
        };
        let url = super::adapters::documents::canonical_document_url(&raw)
            .map_err(|reason| format!("exposure_setting_invalid：文档地址 {raw} 无效：{reason}"))?;
        if note.chars().count() > DOCUMENT_NOTE_LIMIT {
            return Err(format!("exposure_setting_invalid：{url} 的说明超过 {DOCUMENT_NOTE_LIMIT} 字"));
        }
        if public_material && note.is_empty() {
            return Err(format!("exposure_setting_invalid：{url} 声明为公开材料时必须写明理由"));
        }
        if !entries.iter().any(|existing| existing.url == url) {
            entries.push(DocumentUrl { url, public_material, note });
        }
    }
    if entries.len() > DOCUMENT_URLS_LIMIT {
        return Err(format!(
            "exposure_setting_invalid：指定文档最多 {DOCUMENT_URLS_LIMIT} 个，收到 {}",
            entries.len()
        ));
    }
    entries.sort_by(|left, right| left.url.cmp(&right.url));
    Ok(entries)
}

/// Every bound is stated with its unit so the page and the error agree.
pub const MAX_REQUESTS_LIMIT: i64 = 200_000;
pub const MAX_RESPONSE_BYTES_LIMIT: i64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_BYTES_LIMIT: i64 = 8 * 1024 * 1024 * 1024;
pub const MAX_ITEMS_LIMIT: i64 = 200_000;
pub const CONCURRENCY_LIMIT: i64 = 8;
pub const TIMEOUT_SECONDS_LIMIT: i64 = 120;
pub const PAGE_SIZE_LIMIT: i64 = 1_000;
pub const AUTHORIZATION_DAYS_LIMIT: i64 = 365;

/// G1-0: the defaults are named so the reported limits and the collector read one number.
pub const DEFAULT_MAX_ITEMS: i64 = 500;
pub const DEFAULT_CT_PAGE_SIZE: i64 = 200;
pub const DEFAULT_ARCHIVE_PAGE_SIZE: i64 = 200;
/// G1-0: the executor claims one work item and sends one request at a time. A saved
/// `exposureConcurrency` is kept for compatibility but has no effect; the settings view says
/// so instead of letting a number suggest parallel collection.
pub const EFFECTIVE_CONCURRENCY: i64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureSettings {
    pub project_id: i64,
    pub version: i64,
    pub sources: Vec<String>,
    /// Requests per round, across every origin and source.
    pub max_requests: i64,
    /// Bytes kept per single response.
    pub max_response_bytes: i64,
    /// Bytes kept for the whole round.
    pub max_total_bytes: i64,
    /// Origins a round will plan; more than this is reported as an uncollected tail.
    pub max_items: i64,
    /// Stored for compatibility only: the executor is serial (`EFFECTIVE_CONCURRENCY`).
    pub concurrency: i64,
    pub timeout_seconds: i64,
    pub ct_page_size: i64,
    pub archive_page_size: i64,
    /// Off by default: enabling it is an explicit operator decision, tested only on localhost.
    pub allow_private_destinations: bool,
    /// How long a confirmed authorization stays valid for a new round; 0 means "no window".
    pub authorization_days: i64,
    /// G2: GitHub owners maintained by a human as official open source (lower-case logins).
    /// Serialized only when non-empty, so existing frozen plans keep their options hash.
    pub github_official_owners: Vec<String>,
    /// IR-04: keychain account name. Never a token. Omitted from JSON when empty.
    pub github_credential_ref: String,
    /// G3: explicit public document URLs; serialized only when non-empty (old hashes kept).
    pub document_urls: Vec<DocumentUrl>,
}

impl Default for ExposureSettings {
    fn default() -> Self {
        Self {
            project_id: 0,
            version: 1,
            sources: DEFAULT_SOURCES.iter().map(|row| row.to_string()).collect(),
            max_requests: 2_000,
            max_response_bytes: 1_000_000,
            max_total_bytes: 200_000_000,
            max_items: DEFAULT_MAX_ITEMS,
            concurrency: 1,
            timeout_seconds: 12,
            ct_page_size: DEFAULT_CT_PAGE_SIZE,
            archive_page_size: DEFAULT_ARCHIVE_PAGE_SIZE,
            allow_private_destinations: false,
            authorization_days: 0,
            github_official_owners: Vec::new(),
            github_credential_ref: String::new(),
            document_urls: Vec::new(),
        }
    }
}

impl ExposureSettings {
    pub fn for_project(project_id: i64) -> Self {
        Self {
            project_id,
            ..Self::default()
        }
    }

    /// Strict parse. Unknown keys are an error rather than a typo that silently disappears.
    pub fn from_json(project_id: i64, value: &JsonValue) -> Result<Self, String> {
        let mut base = Self::for_project(project_id);
        let Some(map) = value.as_object() else {
            return Err("exposure_setting_invalid：配置必须是 JSON 对象".to_string());
        };
        let known = [
            "exposureSources",
            "exposureMaxRequests",
            "exposureMaxResponseBytes",
            "exposureMaxTotalBytes",
            "exposureMaxItems",
            "exposureConcurrency",
            "exposureTimeoutSeconds",
            "exposureCtPageSize",
            "exposureArchivePageSize",
            "exposureAllowPrivateDestinations",
            "exposureAuthorizationDays",
            "exposureGithubOfficialOwners",
            "exposureDocumentUrls",
            "exposureGithubCredentialRef",
            "version",
        ];
        for key in map.keys() {
            if !known.contains(&key.as_str()) {
                return Err(format!("exposure_setting_invalid：未知字段 {key}"));
            }
        }
        let number = |key: &str, current: i64, max: i64, unit: &str| -> Result<i64, String> {
            match map.get(key) {
                None | Some(JsonValue::Null) => Ok(current),
                Some(value) => {
                    let parsed = value.as_i64().ok_or_else(|| {
                        format!("exposure_setting_invalid：{key} 必须是整数（{unit}）")
                    })?;
                    if parsed <= 0 || parsed > max {
                        return Err(format!(
                            "exposure_setting_invalid：{key} 必须在 1~{max} 之间（{unit}），收到 {parsed}"
                        ));
                    }
                    Ok(parsed)
                }
            }
        };
        if let Some(rows) = map.get("exposureSources").filter(|row| !row.is_null()) {
            let list = rows.as_array().ok_or_else(|| {
                "exposure_setting_invalid：exposureSources 必须是数组".to_string()
            })?;
            let mut next = Vec::new();
            for row in list {
                let name = row
                    .as_str()
                    .ok_or_else(|| "exposure_setting_invalid：来源必须是字符串".to_string())?;
                if !SOURCES.contains(&name) {
                    return Err(format!(
                        "exposure_setting_invalid：未知来源 {name}，可选 {}",
                        SOURCES.join("/")
                    ));
                }
                if !next.iter().any(|existing: &String| existing == name) {
                    next.push(name.to_string());
                }
            }
            if next.is_empty() {
                return Err("exposure_setting_invalid：至少启用一个来源".to_string());
            }
            base.sources = next;
        }
        base.max_requests = number(
            "exposureMaxRequests",
            base.max_requests,
            MAX_REQUESTS_LIMIT,
            "次/轮",
        )?;
        base.max_response_bytes = number(
            "exposureMaxResponseBytes",
            base.max_response_bytes,
            MAX_RESPONSE_BYTES_LIMIT,
            "字节/响应",
        )?;
        base.max_total_bytes = number(
            "exposureMaxTotalBytes",
            base.max_total_bytes,
            MAX_TOTAL_BYTES_LIMIT,
            "字节/轮",
        )?;
        base.max_items = number(
            "exposureMaxItems",
            base.max_items,
            MAX_ITEMS_LIMIT,
            "工作项/轮",
        )?;
        if let Some(value) = map.get("exposureConcurrency").filter(|row| !row.is_null()) {
            if !value.is_i64() {
                return Err("exposure_setting_invalid：exposureConcurrency 必须是整数".to_string());
            }
        }
        base.concurrency = EFFECTIVE_CONCURRENCY;
        base.timeout_seconds = number(
            "exposureTimeoutSeconds",
            base.timeout_seconds,
            TIMEOUT_SECONDS_LIMIT,
            "秒",
        )?;
        base.ct_page_size = number(
            "exposureCtPageSize",
            base.ct_page_size,
            PAGE_SIZE_LIMIT,
            "条/页",
        )?;
        base.archive_page_size = number(
            "exposureArchivePageSize",
            base.archive_page_size,
            PAGE_SIZE_LIMIT,
            "条/页",
        )?;
        if let Some(flag) = map
            .get("exposureAllowPrivateDestinations")
            .filter(|row| !row.is_null())
        {
            base.allow_private_destinations = flag.as_bool().ok_or_else(|| {
                "exposure_setting_invalid：exposureAllowPrivateDestinations 必须是布尔".to_string()
            })?;
        }
        base.authorization_days = match map.get("exposureAuthorizationDays") {
            None | Some(JsonValue::Null) => base.authorization_days,
            Some(value) if value.is_i64() && value.as_i64() == Some(0) => 0,
            Some(value) => {
                let days = value.as_i64().ok_or_else(|| {
                    "exposure_setting_invalid：exposureAuthorizationDays 必须是整数天数".to_string()
                })?;
                if !(0..=AUTHORIZATION_DAYS_LIMIT).contains(&days) {
                    return Err(format!(
                        "exposure_setting_invalid：exposureAuthorizationDays 必须在 0~{AUTHORIZATION_DAYS_LIMIT} 天，收到 {days}"
                    ));
                }
                days
            }
        };
        if let Some(rows) = map
            .get("exposureGithubOfficialOwners")
            .filter(|row| !row.is_null())
        {
            let list = rows.as_array().ok_or_else(|| {
                "exposure_setting_invalid：exposureGithubOfficialOwners 必须是数组".to_string()
            })?;
            let mut owners: Vec<String> = Vec::new();
            for row in list {
                let login = row.as_str().map(str::trim).unwrap_or_default().to_ascii_lowercase();
                if !github_login_valid(&login) {
                    return Err(format!(
                        "exposure_setting_invalid：GitHub 账号 {} 不是合法登录名（字母、数字、连字符，最长 39）",
                        row
                    ));
                }
                if !owners.contains(&login) {
                    owners.push(login);
                }
            }
            if owners.len() > GITHUB_OFFICIAL_OWNERS_LIMIT {
                return Err(format!(
                    "exposure_setting_invalid：正式开源账号最多 {GITHUB_OFFICIAL_OWNERS_LIMIT} 个，收到 {}",
                    owners.len()
                ));
            }
            owners.sort();
            base.github_official_owners = owners;
        }
        base.document_urls = parse_document_urls(map.get("exposureDocumentUrls"))?;
        base.github_credential_ref = match map.get("exposureGithubCredentialRef").filter(|row| !row.is_null()) {
            None => String::new(),
            Some(value) => {
                let text = value.as_str().ok_or_else(|| "exposure_setting_invalid：exposureGithubCredentialRef 必须是字符串".to_string())?.trim().to_string();
                if !text.is_empty() && !super::adapters::github::credential_ref_valid(&text) {
                    return Err("exposure_setting_invalid：凭据引用只允许字母、数字、. _ : -，最长 80，且不能是 token".to_string());
                }
                text
            }
        };
        Ok(base)
    }

    /// The shape `scope::ExposureScope::load` reads, so a persisted setting actually drives
    /// authorization, budget and egress instead of being decoration.
    pub fn to_settings_json(&self) -> JsonValue {
        let mut value = json!({
            "exposureSources": self.sources,
            "exposureMaxRequests": self.max_requests,
            "exposureMaxResponseBytes": self.max_response_bytes,
            "exposureMaxTotalBytes": self.max_total_bytes,
            "exposureMaxItems": self.max_items,
            "exposureTimeoutSeconds": self.timeout_seconds,
            "exposureCtPageSize": self.ct_page_size,
            "exposureArchivePageSize": self.archive_page_size,
            "exposureAllowPrivateDestinations": self.allow_private_destinations,
            "exposureAuthorizationDays": self.authorization_days,
            "version": self.version,
        });
        if !self.github_official_owners.is_empty() {
            value["exposureGithubOfficialOwners"] = json!(self.github_official_owners);
        }
        if !self.github_credential_ref.is_empty() {
            value["exposureGithubCredentialRef"] = json!(self.github_credential_ref);
        }
        if !self.document_urls.is_empty() {
            value["exposureDocumentUrls"] = json!(self
                .document_urls
                .iter()
                .map(|entry| json!({"url": entry.url, "publicMaterial": entry.public_material, "note": entry.note}))
                .collect::<Vec<_>>());
        }
        value
    }

    /// G1-0: the production reader of a frozen round. An empty payload (a round started
    /// before settings were frozen) or a corrupt one is refused with a stable code; the
    /// round is failed with that reason instead of running on today's defaults.
    pub fn from_frozen_strict(project_id: i64, version: i64, raw: &str) -> Result<Self, String> {
        let parsed: JsonValue = serde_json::from_str(raw).map_err(|error| {
            format!("[exposure_plan_settings_corrupt] 本轮冻结的配置 v{version} 不是有效 JSON，拒绝按默认值执行：{error}")
        })?;
        if !parsed.as_object().is_some_and(|map| !map.is_empty()) {
            return Err(format!(
                "[exposure_plan_settings_missing] 本轮没有冻结配置（v{version} 为空），拒绝按当前默认值执行，请重新开始一轮"
            ));
        }
        let mut settings = Self::from_json(project_id, &parsed).map_err(|error| {
            format!("[exposure_plan_settings_corrupt] 本轮冻结的配置 v{version} 无效：{error}")
        })?;
        settings.project_id = project_id;
        settings.version = version;
        Ok(settings)
    }

    /// Lenient legacy reader, kept for diagnostics and the existing round-trip test. No
    /// production path calls it since G1-0: see `from_frozen_strict`.
    pub fn from_frozen(project_id: i64, version: i64, raw: &str) -> Self {
        let parsed: JsonValue =
            serde_json::from_str(raw).unwrap_or_else(|_| JsonValue::Object(JsonMap::new()));
        let mut settings =
            Self::from_json(project_id, &parsed).unwrap_or_else(|_| Self::for_project(project_id));
        settings.project_id = project_id;
        settings.version = version;
        settings
    }
}

/// Reads the current settings for a project, or the defaults when none were saved yet.
pub fn load(connection: &Connection, project_id: i64) -> Result<ExposureSettings, String> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT version,settings_json FROM exposure_settings WHERE project_id=?1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((version, raw)) = row else {
        return Ok(ExposureSettings::for_project(project_id));
    };
    let parsed: JsonValue = serde_json::from_str(&raw).map_err(|error| {
        format!("[exposure_settings_corrupt] 已保存的暴露面配置 v{version} 不是有效 JSON，拒绝按默认值继续：{error}")
    })?;
    let mut settings = ExposureSettings::from_json(project_id, &parsed).map_err(|error| {
        format!("[exposure_settings_corrupt] 已保存的暴露面配置 v{version} 无效：{error}")
    })?;
    settings.version = version;
    Ok(settings)
}

/// Saves a new version. `expected_version` is the row the editor actually read: a stale
/// form cannot overwrite a newer edit, and the previous value stays in the history table.
///
/// G0/P1-1: the version read, the expectedVersion check, the history row and the CAS update
/// all happen inside one IMMEDIATE transaction. Two connections can no longer both read the
/// same version and then write one after the other: the second one waits for the write lock,
/// re-reads the moved version and gets `exposure_setting_conflict`, writing nothing.
pub fn save(
    connection: &Connection,
    settings: &ExposureSettings,
    expected_version: i64,
) -> Result<i64, String> {
    let transaction = rusqlite::Transaction::new_unchecked(
        connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(|error| format!("[exposure_setting_storage] 无法开始配置写事务：{error}"))?;
    let previous: Option<(i64, String)> = transaction
        .query_row(
            "SELECT version,settings_json FROM exposure_settings WHERE project_id=?1",
            [settings.project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let current = previous.as_ref().map_or(0, |(version, _)| *version);
    if current != expected_version {
        // Dropping the transaction rolls back: nothing was written.
        return Err(format!(
            "[exposure_setting_conflict] 当前版本是 v{current}，编辑器读到的是 v{expected_version}，已拒绝覆盖"
        ));
    }
    let next = current + 1;
    let mut stored = settings.clone();
    stored.version = next;
    let payload = stored.to_settings_json().to_string();
    if let Some((version, raw)) = &previous {
        transaction
            .execute(
                "INSERT INTO exposure_setting_versions(project_id,version,settings_json,actor) VALUES(?1,?2,?3,'system')",
                params![settings.project_id, version, raw],
            )
            .map_err(|error| error.to_string())?;
    }
    let written = transaction
        .execute(
            "INSERT INTO exposure_settings(project_id,version,settings_json,updated_at)
             VALUES(?1,?2,?3,datetime('now','localtime'))
             ON CONFLICT(project_id) DO UPDATE SET version=excluded.version,settings_json=excluded.settings_json,updated_at=excluded.updated_at
             WHERE exposure_settings.version=?4",
            params![settings.project_id, next, payload, current],
        )
        .map_err(|error| error.to_string())?;
    if written != 1 {
        return Err(format!(
            "[exposure_setting_conflict] 配置在写入时已被改动（期望 v{current}），已拒绝覆盖"
        ));
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(next)
}

/// (version, stored settings JSON, created_at) of one archived configuration.
pub type SettingVersionRow = (i64, String, String);

pub fn history(
    connection: &Connection,
    project_id: i64,
    limit: i64,
) -> Result<Vec<SettingVersionRow>, String> {
    Ok(history_window(connection, project_id, limit)?.0)
}

/// G0/P1-2: one window of the history, newest first, and whether older versions exist
/// beyond it. Reads `limit + 1` rows so "truncated" is a fact, not a constant; a row that
/// cannot be read is an error, never a silently shorter list.
pub fn history_window(
    connection: &Connection,
    project_id: i64,
    limit: i64,
) -> Result<(Vec<SettingVersionRow>, bool), String> {
    let limit = limit.max(1);
    let mut statement = connection
        .prepare(
            "SELECT version,settings_json,created_at FROM exposure_setting_versions
             WHERE project_id=?1 ORDER BY version DESC LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query_map(params![project_id, limit + 1], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<SettingVersionRow>, _>>()
        .map_err(|error| error.to_string())?;
    let truncated = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    Ok((rows, truncated))
}

/// GitHub login as GitHub actually serves it: 1–39 ASCII letters, digits or hyphens with at
/// least one letter/digit. Today's signup rule forbids leading/trailing/double hyphens, but
/// legacy accounts such as `Rob--W` exist (G2 live check), and refusing them would silently
/// drop real public repositories. Path safety only needs the character set and the length.
pub fn github_login_valid(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login.chars().any(|c| c.is_ascii_alphanumeric())
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_ranges_and_types_are_refused_with_the_field_name() {
        let ok = ExposureSettings::from_json(1, &json!({"exposureMaxRequests": 50})).unwrap();
        assert_eq!(ok.max_requests, 50);
        assert_eq!(ok.max_items, ExposureSettings::default().max_items);
        for (payload, needle) in [
            (json!({"exposureMaxRequest": 5}), "未知字段"),
            (json!({"exposureSources": ["fofa"]}), "未知来源"),
            (json!({"exposureSources": []}), "至少启用一个来源"),
            (json!({"exposureMaxRequests": 0}), "exposureMaxRequests"),
            (
                json!({"exposureMaxRequests": MAX_REQUESTS_LIMIT + 1}),
                "之间",
            ),
            (json!({"exposureConcurrency": "4"}), "必须是整数"),
            (
                json!({"exposureAllowPrivateDestinations": "true"}),
                "必须是布尔",
            ),
            (
                json!({"exposureAuthorizationDays": -1}),
                "exposureAuthorizationDays",
            ),
            (json!("[]"), "JSON 对象"),
        ] {
            let error = ExposureSettings::from_json(1, &payload).unwrap_err();
            assert!(error.contains(needle), "{payload} -> {error}");
        }
    }

    #[test]
    fn zero_and_null_keep_different_meanings() {
        // null 回落默认；0 对“必须有正数”的字段是无效值；0 对授权天数是“不设期限”。
        let defaulted =
            ExposureSettings::from_json(1, &json!({"exposureMaxRequests": null})).unwrap();
        assert_eq!(
            defaulted.max_requests,
            ExposureSettings::default().max_requests
        );
        assert!(ExposureSettings::from_json(1, &json!({"exposureMaxRequests": 0})).is_err());
        let windowless =
            ExposureSettings::from_json(1, &json!({"exposureAuthorizationDays": 0})).unwrap();
        assert_eq!(windowless.authorization_days, 0);
    }

    #[test]
    fn private_egress_stays_off_unless_it_is_written_explicitly() {
        assert!(!ExposureSettings::default().allow_private_destinations);
        let opened =
            ExposureSettings::from_json(1, &json!({"exposureAllowPrivateDestinations": true}))
                .unwrap();
        assert!(opened.allow_private_destinations);
    }

    #[test]
    fn saving_is_versioned_and_a_stale_editor_cannot_overwrite() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-exposure-e2-{}", uuid::Uuid::new_v4()));
        let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
        let connection = crate::db::open(&db_path).expect("连接");
        super::super::state::ensure_exposure_schema(&connection).expect("结构");
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(141,'范围','active')",
                [],
            )
            .unwrap();
        assert_eq!(
            load(&connection, 141).unwrap().version,
            1,
            "没有保存过时用默认值"
        );
        let mut settings = ExposureSettings::for_project(141);
        settings.max_requests = 4_000;
        settings.sources = vec!["passive_web".to_string()];
        assert_eq!(save(&connection, &settings, 0).unwrap(), 1);
        let saved = load(&connection, 141).unwrap();
        assert_eq!(saved.max_requests, 4_000);
        assert_eq!(saved.sources, vec!["passive_web".to_string()]);
        assert_eq!(saved.version, 1);
        // 第二个编辑基于旧版本提交必须被拒，且旧值仍在。
        let conflict = save(&connection, &saved, 0).unwrap_err();
        assert!(conflict.contains("exposure_setting_conflict"), "{conflict}");
        assert_eq!(load(&connection, 141).unwrap().max_requests, 4_000);
        let mut again = saved.clone();
        again.max_requests = 100;
        assert_eq!(save(&connection, &again, 1).unwrap(), 2);
        assert_eq!(load(&connection, 141).unwrap().max_requests, 100);
        let history = history(&connection, 141, 10).unwrap();
        assert_eq!(history.len(), 1, "被覆盖的那一版必须留档");
        assert_eq!(history[0].0, 1);
        assert!(history[0].1.contains("4000"), "{}", history[0].1);
        // 项目隔离：别的空间仍是默认值。
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(142,'另一','active')",
                [],
            )
            .unwrap();
        assert_eq!(load(&connection, 142).unwrap().max_requests, 2_000);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn g1_0_strict_frozen_reader_refuses_corrupt_and_empty_payloads() {
        let payload = json!({"exposureMaxRequests": 77, "exposureCtPageSize": 30});
        let rebuilt = ExposureSettings::from_frozen_strict(9, 4, &payload.to_string()).unwrap();
        assert_eq!((rebuilt.max_requests, rebuilt.ct_page_size, rebuilt.version), (77, 30, 4));
        let broken = ExposureSettings::from_frozen_strict(9, 4, "{not json").unwrap_err();
        assert!(broken.starts_with("[exposure_plan_settings_corrupt]"), "{broken}");
        let empty = ExposureSettings::from_frozen_strict(9, 4, "{}").unwrap_err();
        assert!(empty.starts_with("[exposure_plan_settings_missing]"), "{empty}");
        let array = ExposureSettings::from_frozen_strict(9, 4, "[]").unwrap_err();
        assert!(array.starts_with("[exposure_plan_settings_missing]"), "{array}");
        let out_of_range = ExposureSettings::from_frozen_strict(9, 4, r#"{"exposureMaxItems":0}"#).unwrap_err();
        assert!(out_of_range.starts_with("[exposure_plan_settings_corrupt]"), "{out_of_range}");
        let unknown = ExposureSettings::from_frozen_strict(9, 4, r#"{"exposureTurbo":true}"#).unwrap_err();
        assert!(unknown.contains("exposureTurbo"), "{unknown}");
    }

    #[test]
    fn a_frozen_round_can_be_rebuilt_from_its_own_stored_settings() {
        let payload = json!({"exposureMaxRequests": 77, "exposureSources": ["web_archive"], "exposureArchivePageSize": 40});
        let rebuilt = ExposureSettings::from_frozen(9, 4, &payload.to_string());
        assert_eq!(rebuilt.max_requests, 77);
        assert_eq!(rebuilt.version, 4);
        assert_eq!(rebuilt.archive_page_size, 40);
        // 损坏的冻结内容不能把预算变成 0：回落到默认并保留版本号。
        let broken = ExposureSettings::from_frozen(9, 4, "{not json");
        assert_eq!(
            broken.max_requests,
            ExposureSettings::default().max_requests
        );
        assert_eq!(broken.version, 4);
    }
}

