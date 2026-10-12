// §Q3/Q5 — the durable state of an exposure run: work-item and source vocabularies, the
// additive schema, and the aggregation that decides whether a round may say "completed".
// A gap must be named; it can never be inferred from the absence of rows.

use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};

/// The additive Asset-only tables. Applied idempotently from here so the surface works
/// before the coordinator folds the same statements into the shared schema.
pub const EXPOSURE_SCHEMA: &str = include_str!("schema.sql");
/// Bumped whenever a classification rule changes, so an observation states which rules
/// produced it.
pub const EXPOSURE_RULE_VERSION: &str = "oviraptor.exposure.rules.v1";

pub fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A UTC string `seconds` ago, for lease and heartbeat comparisons.
pub fn shift_seconds_back(seconds: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::seconds(seconds.max(0)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// The same deadline in the future: how long a cached verdict is still allowed to answer.
pub fn shift_seconds_forward(seconds: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(seconds.max(0)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkItemStatus {
    Pending,
    Claimed,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    SkippedUnconfigured,
    BlockedQuota,
    BlockedAuthorization,
    Orphaned,
}

impl WorkItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::SkippedUnconfigured => "skipped_unconfigured",
            Self::BlockedQuota => "blocked_quota",
            Self::BlockedAuthorization => "blocked_authorization",
            Self::Orphaned => "orphaned",
        }
    }

    /// Unknown words are corruption, not `pending`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "claimed" => Some(Self::Claimed),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "skipped_unconfigured" => Some(Self::SkippedUnconfigured),
            "blocked_quota" => Some(Self::BlockedQuota),
            "blocked_authorization" => Some(Self::BlockedAuthorization),
            "orphaned" => Some(Self::Orphaned),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Claimed | Self::InProgress)
    }

    /// Everything but a clean completion is a coverage gap with a name.
    pub fn is_gap(self) -> bool {
        matches!(
            self,
            Self::Failed
                | Self::Cancelled
                | Self::SkippedUnconfigured
                | Self::BlockedQuota
                | Self::BlockedAuthorization
                | Self::Orphaned
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Completed,
    Partial,
    Failed,
    Cancelled,
    Unconfigured,
    Unsupported,
    ParseError,
    /// G2: the provider refused for quota (GitHub 403 with an exhausted rate limit, or 429).
    RateLimited,
    /// G2: the provider refused the identity (401, or 403 without a rate-limit signal).
    AuthFailed,
    /// G2: the query really ran and the bounded result window had no match. It is not a gap
    /// in execution, and it never means "nothing leaked".
    CompletedNoMatches,
}

impl SourceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unconfigured => "unconfigured",
            Self::Unsupported => "unsupported",
            Self::ParseError => "parse_error",
            Self::RateLimited => "rate_limited",
            Self::AuthFailed => "auth_failed",
            Self::CompletedNoMatches => "completed_no_matches",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "completed" => Some(Self::Completed),
            "partial" => Some(Self::Partial),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "unconfigured" => Some(Self::Unconfigured),
            "unsupported" => Some(Self::Unsupported),
            "parse_error" => Some(Self::ParseError),
            "rate_limited" => Some(Self::RateLimited),
            "auth_failed" => Some(Self::AuthFailed),
            "completed_no_matches" => Some(Self::CompletedNoMatches),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        true
    }

    pub fn is_gap(self) -> bool {
        !matches!(self, Self::Completed | Self::CompletedNoMatches)
    }

    /// How much a terminal state undermines coverage. The worst rank across a source's
    /// work items is what that source reports, so a failed index cannot be hidden by a
    /// successful page walk.
    pub fn severity_rank(self) -> i64 {
        match self {
            // A source with one matching query and one empty query has matches.
            Self::CompletedNoMatches => -1,
            Self::Completed => 0,
            Self::Partial => 1,
            Self::Cancelled | Self::Unconfigured | Self::Unsupported => 2,
            Self::ParseError | Self::RateLimited => 3,
            Self::Failed | Self::AuthFailed => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Partial,
    Inconclusive,
    Cancelled,
    Interrupted,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Inconclusive => "inconclusive",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "partial" => Some(Self::Partial),
            "inconclusive" => Some(Self::Inconclusive),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// Only `completed` may be read as "we looked at everything we planned to".
    pub fn is_covered(self) -> bool {
        self == Self::Completed
    }
}

#[derive(Clone, Debug)]
pub struct SourceOutcome {
    pub source_key: String,
    pub status: SourceStatus,
    pub reason: String,
    pub observed: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkItemCounts {
    pub total: i64,
    pub terminal: i64,
    pub open: i64,
    pub completed: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub unconfigured: i64,
    pub quota: i64,
    pub authorization: i64,
    pub orphaned: i64,
}

#[derive(Clone, Debug)]
pub struct Aggregation {
    pub status: RunStatus,
    pub reasons: Vec<String>,
    pub gaps: Vec<String>,
}

/// The single place that decides a run's final state. `completed` requires every planned
/// work item and every planned source to be terminal with no unexplained gap; anything
/// else enumerates what is missing instead of trusting that the thread exited cleanly.
pub fn aggregate(
    planned_sources: &[String],
    outcomes: &[SourceOutcome],
    counts: &WorkItemCounts,
    cancelled: bool,
    infra_failure: Option<&str>,
) -> Aggregation {
    let mut reasons = Vec::new();
    let mut gaps = Vec::new();
    if let Some(detail) = infra_failure {
        return Aggregation {
            status: RunStatus::Failed,
            reasons: vec![format!("基础设施失败，本轮结论不可用：{detail}")],
            gaps: Vec::new(),
        };
    }
    if cancelled {
        reasons.push("用户取消；已保存的观察与线索保留".to_string());
        return Aggregation {
            status: RunStatus::Cancelled,
            reasons,
            gaps: Vec::new(),
        };
    }
    if counts.total == 0 {
        reasons.push("计划工作项为空集，不能报覆盖完成".to_string());
        return Aggregation {
            status: RunStatus::Inconclusive,
            reasons,
            gaps: vec!["empty_plan".to_string()],
        };
    }
    for source in planned_sources {
        if !outcomes.iter().any(|row| row.source_key == *source) {
            gaps.push(format!("source_outcome_missing:{source}"));
            reasons.push(format!("来源 {source} 没有终态记录"));
        }
    }
    if counts.open > 0 {
        gaps.push(format!("work_items_open:{}", counts.open));
        reasons.push(format!("仍有 {} 个工作项未到终态", counts.open));
    }
    for (count, label, code) in [
        (counts.failed, "失败", "work_item_failed"),
        (counts.unconfigured, "未配置", "work_item_unconfigured"),
        (counts.quota, "超额", "work_item_quota"),
        (
            counts.authorization,
            "待授权/已撤销",
            "work_item_authorization",
        ),
        (counts.orphaned, "孤儿认领", "work_item_orphaned"),
    ] {
        if count > 0 {
            gaps.push(format!("{code}:{count}"));
            reasons.push(format!("{label} 工作项 {count} 项"));
        }
    }
    for outcome in outcomes {
        if outcome.status.is_gap() {
            gaps.push(format!(
                "source_{}:{}",
                outcome.status.as_str(),
                outcome.source_key
            ));
            let detail = if outcome.reason.trim().is_empty() {
                String::new()
            } else {
                format!("（{}）", outcome.reason)
            };
            reasons.push(format!(
                "来源 {} 终态为 {}{detail}",
                outcome.source_key,
                outcome.status.as_str()
            ));
        }
    }
    let clean = counts.open == 0
        && counts.failed == 0
        && counts.unconfigured == 0
        && counts.quota == 0
        && counts.authorization == 0
        && counts.orphaned == 0
        && gaps.is_empty()
        && counts.terminal == counts.total;
    Aggregation {
        status: if clean {
            RunStatus::Completed
        } else {
            RunStatus::Partial
        },
        reasons,
        gaps,
    }
}

/// Creates the additive tables. The active-run uniqueness index can legitimately fail on
/// a historical database that already holds two active rows; that is reported instead of
/// being silently dropped, and every other statement must succeed.
/// §E2 columns added after the first release. `ALTER TABLE` is not idempotent in SQLite, so
/// an already-migrated database reports "duplicate column" and that is the success case.
const EXPOSURE_MIGRATIONS: &[&str] = &[
    "ALTER TABLE exposure_run_plans ADD COLUMN settings_version INTEGER NOT NULL DEFAULT 1",
    "ALTER TABLE exposure_run_plans ADD COLUMN settings_json TEXT NOT NULL DEFAULT '{}'",
    // §E3 the cluster pass reads observations by identical bytes and by registered domain,
    // and a cluster has to name the observation its representative came from.
    "ALTER TABLE exposure_observations ADD COLUMN canonical_url TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_observations ADD COLUMN root_domain TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_clusters ADD COLUMN direct_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_clusters ADD COLUMN inherited_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_clusters ADD COLUMN representative_observation_id INTEGER",
    "ALTER TABLE exposure_clusters ADD COLUMN representative_run_id INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_clusters ADD COLUMN settings_version INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_clusters ADD COLUMN gap_note TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_cluster_members ADD COLUMN observation_id INTEGER",
    "ALTER TABLE exposure_cluster_members ADD COLUMN rule_version TEXT NOT NULL DEFAULT ''",
    // §E3 cache identity: same bytes under another source, URL or configuration version is a
    // different question, and an entry has to say when it stops being usable.
    "ALTER TABLE exposure_analysis_cache ADD COLUMN source_key TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_analysis_cache ADD COLUMN canonical_url TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_analysis_cache ADD COLUMN settings_version INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_analysis_cache ADD COLUMN expires_at TEXT NOT NULL DEFAULT ''",
    // Indexes come after their columns exist on an already-migrated database.
    "CREATE INDEX IF NOT EXISTS idx_exposure_observations_content ON exposure_observations(project_id, content_hash, asset_id)",
    "CREATE INDEX IF NOT EXISTS idx_exposure_observations_domain ON exposure_observations(project_id, root_domain, asset_id)",
    "CREATE INDEX IF NOT EXISTS idx_exposure_cache_content ON exposure_analysis_cache(project_id, content_hash, kind)",
    // §E3: whether one member carries its own finding is answered per observation row.
    "CREATE INDEX IF NOT EXISTS idx_exposure_finding_observation ON exposure_finding_observations(observation_id)",
    // §E5 a review decision has to say who made it, against which revision and which evidence.
    "ALTER TABLE exposure_review_events ADD COLUMN expected_revision INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE exposure_review_events ADD COLUMN references_json TEXT NOT NULL DEFAULT '[]'",
    // G1-1 a domain boundary names the rule that drew it: `psl:<snapshot>` / `exact_host`;
    // '' is a pre-G1 last-two-labels row, kept readable and never merged with new rows.
    "ALTER TABLE exposure_observations ADD COLUMN root_basis TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_clusters ADD COLUMN boundary_basis TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE exposure_clusters ADD COLUMN psl_version TEXT NOT NULL DEFAULT ''",
    "CREATE INDEX IF NOT EXISTS idx_exposure_observations_boundary ON exposure_observations(project_id, root_basis, root_domain, asset_id)",
];

pub fn ensure_exposure_schema(connection: &Connection) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    for statement in EXPOSURE_SCHEMA.split(';') {
        let trimmed = statement.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Err(error) = connection.execute_batch(&format!("{trimmed};")) {
            let text = error.to_string();
            let tolerated = trimmed.contains("ux_exposure_runs_single_active")
                && (text.contains("attempt to create unique index")
                    || text.contains("UNIQUE constraint failed"));
            if tolerated {
                warnings.push(format!(
                    "历史库已有同项目多个进行中的任务，活跃唯一索引未建立：{text}"
                ));
                continue;
            }
            if text.contains("already exists") || text.contains("duplicate column name") {
                continue;
            }
            // FIX2：共享表 asset_events 的索引只是加速；表不在（极旧或残缺的库）时不阻断暴露面初始化。
            if trimmed.contains("idx_asset_events_project_asset") && text.contains("no such table") {
                warnings.push(format!("asset_events 不存在，跳过归属事件索引：{text}"));
                continue;
            }
            return Err(format!("暴露面结构初始化失败：{text}"));
        }
    }
    // The additive columns come last: the tables have to exist before they can be altered.
    for statement in EXPOSURE_MIGRATIONS {
        if let Err(error) = connection.execute_batch(statement) {
            let text = error.to_string();
            if text.contains("duplicate column name") || text.contains("already exists") {
                continue;
            }
            return Err(format!("暴露面结构迁移失败：{text}"));
        }
    }
    Ok(warnings)
}

pub fn schema_tables_present(connection: &Connection) -> Result<bool, String> {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
             ('exposure_run_plans','exposure_work_items','exposure_observations','exposure_scope_receipts',\
              'exposure_finding_observations','exposure_finding_versions','exposure_clusters','exposure_cluster_members','exposure_analysis_cache','exposure_pending_discoveries',\
              'exposure_settings','exposure_setting_versions','exposure_classifications','exposure_finding_dispositions')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count == 14)
}

pub fn aggregation_json(aggregation: &Aggregation) -> JsonValue {
    json!({
        "status": aggregation.status.as_str(),
        "reasons": aggregation.reasons,
        "gaps": aggregation.gaps,
    })
}
