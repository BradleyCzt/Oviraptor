// §Q1 — the read-only contract for the public exposure surface. These types are the
// only shape the UI and the repository agree on: a page carries its real filtered total,
// whether more rows exist, and the opaque keyset cursor that produced the next page.
// Nothing here can create a run, mutate a row or reach the network.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

/// Smallest and largest page the exposure queries accept. Anything outside is refused
/// rather than silently clamped, so a caller cannot ask for "everything" by accident.
pub const EXPOSURE_MIN_PAGE_SIZE: i64 = 10;
pub const EXPOSURE_MAX_PAGE_SIZE: i64 = 200;
/// A search box longer than this is a paste, not a query.
pub const EXPOSURE_MAX_SEARCH_CHARS: usize = 200;

/// The caps the collector really enforces (§4.1, G1-0). Each one is the number the code
/// path reads, not a separate claim: the asset cap is the default work-item cap of a round
/// (`settings.max_items`, shared by passive web, CT and archive items), the per-asset caps
/// are `adapters::passive_web::WalkBudget::default()`, the CT/Wayback caps are the default
/// `ct_page_size` / `archive_page_size`. G0 reported 200 assets / 40 queued / 20 CT domains,
/// none of which any code enforced.
pub const PASSIVE_ASSET_CAP: i64 = super::settings::DEFAULT_MAX_ITEMS;
pub const PASSIVE_RESOURCE_CAP_PER_ASSET: i64 = 20;
pub const PASSIVE_VISIT_QUEUE_CAP: i64 = 80;
pub const CT_NAMES_PER_DOMAIN: i64 = super::settings::DEFAULT_CT_PAGE_SIZE;
pub const WAYBACK_ROWS_PER_DOMAIN: i64 = super::settings::DEFAULT_ARCHIVE_PAGE_SIZE;
/// The window the legacy list commands still apply.
pub const LEGACY_FINDING_WINDOW: i64 = 500;
pub const LEGACY_RUN_WINDOW: i64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposureReviewStatus {
    New,
    Reviewing,
    Confirmed,
    Dismissed,
}

impl ExposureReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reviewing => "reviewing",
            Self::Confirmed => "confirmed",
            Self::Dismissed => "dismissed",
        }
    }

    /// An unknown word is a rejected query, not an empty filter. The legacy list command
    /// dropped unknown statuses into "no filter", which made a typo look like a result.
    pub fn parse_strict(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "new" => Some(Self::New),
            "reviewing" => Some(Self::Reviewing),
            "confirmed" => Some(Self::Confirmed),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// Stable error codes the page and the logs can branch on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureQueryError {
    pub code: &'static str,
    pub message: String,
}

impl ExposureQueryError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_status(value: &str) -> Self {
        Self::new(
            "exposure_status_invalid",
            format!("复核状态 {value} 不在 new/reviewing/confirmed/dismissed 里，已拒绝而不是当作无筛选"),
        )
    }

    pub fn invalid_page_size(value: i64) -> Self {
        Self::new(
            "exposure_page_size_invalid",
            format!("页长 {value} 超出 {EXPOSURE_MIN_PAGE_SIZE}~{EXPOSURE_MAX_PAGE_SIZE}，已拒绝"),
        )
    }

    pub fn invalid_cursor() -> Self {
        Self::new(
            "exposure_cursor_invalid",
            "分页游标不可解析或已被篡改，已拒绝并从第一页重新读取".to_string(),
        )
    }

    pub fn outside_project() -> Self {
        Self::new(
            "exposure_scope_mismatch",
            "该线索或任务不属于本项目，已拒绝读取".to_string(),
        )
    }

    pub fn storage(error: &str) -> Self {
        Self::new("exposure_query_failed", format!("暴露面读取失败：{error}"))
    }
}

/// Runs are keyed by a single descending id, so their cursor is just that id. It is
/// still opaque: the page never learns the internal numbering convention.
pub fn encode_id_cursor(id: i64) -> String {
    BASE64_STANDARD.encode(id.to_string())
}

pub fn decode_id_cursor(raw: &str) -> Result<i64, ExposureQueryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(i64::MAX);
    }
    let Ok(bytes) = BASE64_STANDARD.decode(trimmed) else {
        return Err(ExposureQueryError::invalid_cursor());
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(ExposureQueryError::invalid_cursor());
    };
    let id = text
        .trim()
        .parse::<i64>()
        .map_err(|_| ExposureQueryError::invalid_cursor())?;
    if id <= 0 {
        return Err(ExposureQueryError::invalid_cursor());
    }
    Ok(id)
}

/// The four DESC sort keys of the finding queue. A page boundary is "strictly below this
/// tuple", which needs no offset and cannot shift when rows are inserted above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureSortKey {
    pub rank: i64,
    pub confidence: i64,
    pub last_seen: String,
    pub id: i64,
}

impl ExposureSortKey {
    pub fn encode(&self) -> String {
        BASE64_STANDARD.encode(format!(
            "{}\u{1}{}\u{1}{}\u{1}{}",
            self.rank, self.confidence, self.last_seen, self.id
        ))
    }

    pub fn decode(raw: &str) -> Result<Self, ExposureQueryError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(Self {
                rank: i64::MAX,
                confidence: i64::MAX,
                last_seen: char::MAX.to_string().repeat(32),
                id: i64::MAX,
            });
        }
        let Ok(bytes) = BASE64_STANDARD.decode(trimmed) else {
            return Err(ExposureQueryError::invalid_cursor());
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return Err(ExposureQueryError::invalid_cursor());
        };
        let parts: Vec<&str> = text.split('\u{1}').collect();
        if parts.len() != 4 {
            return Err(ExposureQueryError::invalid_cursor());
        }
        let rank = parts[0]
            .parse::<i64>()
            .map_err(|_| ExposureQueryError::invalid_cursor())?;
        let confidence = parts[1]
            .parse::<i64>()
            .map_err(|_| ExposureQueryError::invalid_cursor())?;
        if !(1..=4).contains(&rank) || !(0..=100).contains(&confidence) {
            return Err(ExposureQueryError::invalid_cursor());
        }
        let last_seen = parts[2].to_string();
        let id = parts[3]
            .parse::<i64>()
            .map_err(|_| ExposureQueryError::invalid_cursor())?;
        if id <= 0 || last_seen.chars().any(|c| c == '\u{1}' || c.is_control()) {
            return Err(ExposureQueryError::invalid_cursor());
        }
        Ok(Self {
            rank,
            confidence,
            last_seen,
            id,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExposureFindingPageInput {
    pub project_id: i64,
    pub status: String,
    pub search: String,
    pub page_size: i64,
    pub cursor: String,
    /// G1-3 `''` = any; otherwise one of `classify::DISPOSITION_FILTERS`. Unknown words are refused.
    pub disposition: String,
}

/// Normalized query: the four values the SQL actually binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureFindingPageQuery {
    pub project_id: i64,
    /// `None` means "any status"; an unknown word never reaches this state.
    pub status: Option<ExposureReviewStatus>,
    pub search: Option<String>,
    pub page_size: i64,
    pub after: ExposureSortKey,
    /// G1-3 canonical disposition filter name; `None` means any.
    pub disposition: Option<&'static str>,
}

impl ExposureFindingPageInput {
    pub fn normalize(&self) -> Result<ExposureFindingPageQuery, ExposureQueryError> {
        if self.project_id <= 0 {
            return Err(ExposureQueryError::new(
                "exposure_project_invalid",
                "缺少有效的工作空间 id，暴露面线索不能跨项目读取",
            ));
        }
        let status = if self.status.trim().is_empty() {
            None
        } else {
            Some(
                ExposureReviewStatus::parse_strict(&self.status)
                    .ok_or_else(|| ExposureQueryError::invalid_status(self.status.trim()))?,
            )
        };
        if !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&self.page_size) {
            return Err(ExposureQueryError::invalid_page_size(self.page_size));
        }
        let search = {
            let trimmed = self.search.trim();
            if trimmed.is_empty() {
                None
            } else {
                let clipped: String = trimmed.chars().take(EXPOSURE_MAX_SEARCH_CHARS).collect();
                Some(clipped)
            }
        };
        let disposition = if self.disposition.trim().is_empty() {
            None
        } else {
            Some(
                super::classify::disposition_filter(self.disposition.trim()).ok_or_else(|| {
                    ExposureQueryError::new(
                        "exposure_disposition_invalid",
                        format!(
                            "处置筛选 {} 不在 {} 里，已拒绝而不是当作无筛选",
                            self.disposition.trim(),
                            super::classify::DISPOSITION_FILTERS.join("/")
                        ),
                    )
                })?,
            )
        };
        Ok(ExposureFindingPageQuery {
            project_id: self.project_id,
            status,
            search,
            page_size: self.page_size,
            after: ExposureSortKey::decode(&self.cursor)?,
            disposition,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposurePage<T> {
    pub items: Vec<T>,
    /// Empty once the last page is read; the page that returns rows always carries the
    /// cursor for the one after them.
    pub next_cursor: String,
    pub has_more: bool,
    /// Rows matching the same filter, across every page. Never a page length.
    pub total: i64,
    pub page_size: i64,
}

/// One known truncation of the current passive pipeline, in the user's words.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureLimit {
    pub key: String,
    pub label: String,
    pub cap: i64,
    pub effect: String,
}

/// The limits are reported by the backend so the page does not hard-code a number that
/// drifts away from the collector. Without a project this is the default configuration.
pub fn passive_known_limits() -> Vec<ExposureLimit> {
    known_limits_for(&super::settings::ExposureSettings::default())
}

/// G1-0: the limits one project's next round will actually apply, read from its saved
/// configuration and the walk budget the executor constructs.
pub fn known_limits_for(settings: &super::settings::ExposureSettings) -> Vec<ExposureLimit> {
    let walk = super::adapters::passive_web::WalkBudget::default();
    let rows = [
        (
            "passive_assets",
            "每轮工作项上限（被动网页按资产、证书/归档按域名各占一项）",
            settings.max_items,
            "按工作键顺序认领，超出的工作项本轮标为配额阻塞、不会被采集，差额在总览里单独列出",
        ),
        (
            "passive_resources",
            "每个资产的请求尝试上限（失败也计入）",
            walk.max_attempts as i64,
            "达到上限即停止该资产的抓取，未读的 HTML/JS/Map 记为本轮遗漏",
        ),
        (
            "visit_queue",
            "每个资产的待访与已访 URL 合计上限",
            walk.max_queued as i64,
            "达到上限后新发现的链接不再入队",
        ),
        (
            "ct_names_per_domain",
            "证书透明度每个域名保留的名称数上限",
            settings.ct_page_size,
            "crt.sh 一次返回全部结果；超出上限的名称不记录，该工作项标为配额阻塞（部分）",
        ),
        (
            "wayback_rows",
            "每个域名的 Wayback 索引行上限",
            settings.archive_page_size,
            "历史索引分页未实现，只取前若干行",
        ),
        (
            "finding_window",
            "旧线索接口的返回条数上限",
            LEGACY_FINDING_WINDOW,
            "新分页接口按游标读全量，旧接口仍只显示前若干条",
        ),
        (
            "run_window",
            "旧任务列表的返回条数上限",
            LEGACY_RUN_WINDOW,
            "更早的采集任务需要通过分页接口读取",
        ),
    ];
    rows.into_iter()
        .map(|(key, label, cap, effect)| ExposureLimit {
            key: key.to_string(),
            label: label.to_string(),
            cap,
            effect: effect.to_string(),
        })
        .collect()
}

/// `%` and `_` in a pasted URL are literal characters, not wildcards; without escaping,
/// one `%` would make the filtered total match the whole table.
pub fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    out.push('%');
    for character in value.chars() {
        if matches!(character, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_rejects_unknown_status_and_page_length() {
        let base = ExposureFindingPageInput {
            project_id: 7,
            status: "cnofirmed".into(),
            search: String::new(),
            page_size: 50,
            cursor: String::new(),
            disposition: String::new(),
        };
        let error = base.normalize().unwrap_err();
        assert_eq!(error.code, "exposure_status_invalid");
        assert!(error.message.contains("cnofirmed"), "{}", error.message);
        let long = ExposureFindingPageInput {
            page_size: 500,
            status: String::new(),
            ..base.clone()
        };
        assert_eq!(
            long.normalize().unwrap_err().code,
            "exposure_page_size_invalid"
        );
        let short = ExposureFindingPageInput {
            page_size: 5,
            status: String::new(),
            ..base.clone()
        };
        assert_eq!(
            short.normalize().unwrap_err().code,
            "exposure_page_size_invalid"
        );
        // A typo must not become "no filter at all".
        let empty_status = ExposureFindingPageInput {
            status: "   ".into(),
            ..base.clone()
        }
        .normalize()
        .unwrap();
        assert_eq!(empty_status.status, None);
        assert_eq!(
            ExposureFindingPageInput {
                status: "dismissed".into(),
                ..base.clone()
            }
            .normalize()
            .unwrap()
            .status,
            Some(ExposureReviewStatus::Dismissed)
        );
        assert_eq!(
            ExposureFindingPageInput {
                project_id: 0,
                ..base
            }
            .normalize()
            .unwrap_err()
            .code,
            "exposure_project_invalid"
        );
    }

    #[test]
    fn cursor_round_trips_and_refuses_corruption() {
        let key = ExposureSortKey {
            rank: 3,
            confidence: 88,
            last_seen: "2026-10-07 09:12:01".into(),
            id: 4_013,
        };
        let encoded = key.encode();
        assert_eq!(ExposureSortKey::decode(&encoded).unwrap(), key);
        for broken in ["not base64 \u{1}", "AAAAAAAA", "abcd", &encoded[..2]] {
            assert_eq!(
                ExposureSortKey::decode(broken).unwrap_err().code,
                "exposure_cursor_invalid",
                "{broken} 必须被拒绝"
            );
        }
        // An out-of-range tuple encodes legal bytes but an illegal key, and is refused too.
        let forged = ExposureSortKey {
            rank: 9,
            confidence: 88,
            last_seen: "2026-10-07 09:12:01".into(),
            id: 4_013,
        }
        .encode();
        assert_eq!(
            ExposureSortKey::decode(&forged).unwrap_err().code,
            "exposure_cursor_invalid"
        );
        // The empty cursor means "from the very first row", not an error.
        assert!(ExposureSortKey::decode("")
            .unwrap()
            .last_seen
            .chars()
            .all(|c| c == char::MAX));
    }

    #[test]
    fn search_is_escaped_and_clipped() {
        assert_eq!(escape_like("a%b"), "%a\\%b%");
        assert_eq!(escape_like("100_"), "%100\\_%");
        let query = ExposureFindingPageInput {
            search: format!("{}x", "y".repeat(300)),
            ..ExposureFindingPageInput {
                project_id: 1,
                status: String::new(),
                search: String::new(),
                page_size: 10,
                cursor: String::new(),
                disposition: String::new(),
            }
        }
        .normalize()
        .unwrap();
        assert_eq!(
            query.search.unwrap().chars().count(),
            EXPOSURE_MAX_SEARCH_CHARS
        );
    }

    #[test]
    fn limits_are_reported_with_their_effect() {
        let limits = passive_known_limits();
        assert_eq!(limits.len(), 7);
        let assets = limits
            .iter()
            .find(|row| row.key == "passive_assets")
            .expect("资产上限必须报告");
        assert_eq!(assets.cap, PASSIVE_ASSET_CAP);
        assert!(assets.effect.contains("不会"), "{}", assets.effect);
        assert!(limits
            .iter()
            .all(|row| !row.label.trim().is_empty() && row.cap > 0));
    }

    #[test]
    fn g1_0_reported_limits_are_the_numbers_the_collector_reads() {
        let walk = super::super::adapters::passive_web::WalkBudget::default();
        let defaults = passive_known_limits();
        let cap = |rows: &[ExposureLimit], key: &str| rows.iter().find(|row| row.key == key).map(|row| row.cap);
        assert_eq!(cap(&defaults, "visit_queue"), Some(walk.max_queued as i64), "G0 报告 40，实际 80");
        assert_eq!(cap(&defaults, "passive_resources"), Some(walk.max_attempts as i64));
        assert_eq!(PASSIVE_VISIT_QUEUE_CAP, walk.max_queued as i64);
        assert_eq!(PASSIVE_RESOURCE_CAP_PER_ASSET, walk.max_attempts as i64);
        assert_eq!(cap(&defaults, "passive_assets"), Some(super::super::settings::ExposureSettings::default().max_items));
        assert_eq!(cap(&defaults, "ct_domains"), None, "未执行的“20 个 CT 域名”上限不再报告");
        let project = super::super::settings::ExposureSettings {
            max_items: 37,
            ct_page_size: 11,
            archive_page_size: 13,
            ..super::super::settings::ExposureSettings::for_project(5)
        };
        let rows = known_limits_for(&project);
        assert_eq!(cap(&rows, "passive_assets"), Some(37));
        assert_eq!(cap(&rows, "ct_names_per_domain"), Some(11));
        assert_eq!(cap(&rows, "wayback_rows"), Some(13));
    }
}

/// §Q4 — a name or URL an external index handed back, waiting for a human to decide.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposurePending {
    pub id: i64,
    pub project_id: i64,
    pub run_id: i64,
    pub source_key: String,
    pub kind: String,
    pub entity: String,
    pub canonical_key: String,
    pub evidence_url: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposurePendingPageInput {
    pub project_id: i64,
    pub status: String,
    pub page_size: i64,
    pub cursor: String,
}

impl ExposurePendingPageInput {
    pub fn normalize(self) -> Result<(i64, String, i64, Option<i64>), ExposureQueryError> {
        if self.project_id <= 0 {
            return Err(ExposureQueryError::new(
                "exposure_project_invalid",
                "缺少有效的工作空间编号",
            ));
        }
        let status = if self.status.trim().is_empty() {
            String::new()
        } else if ["unreviewed", "merged", "rejected"]
            .contains(&self.status.trim().to_ascii_lowercase().as_str())
        {
            self.status.trim().to_ascii_lowercase()
        } else {
            return Err(ExposureQueryError::new(
                "exposure_status_invalid",
                format!("待归并状态 {} 无效", self.status),
            ));
        };
        let page_size = self.page_size.max(EXPOSURE_MIN_PAGE_SIZE);
        if !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&page_size) {
            return Err(ExposureQueryError::invalid_page_size(self.page_size));
        }
        let after = if self.cursor.trim().is_empty() {
            None
        } else {
            Some(decode_id_cursor(&self.cursor)?)
        };
        Ok((self.project_id, status, page_size, after))
    }
}
