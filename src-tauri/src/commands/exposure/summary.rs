// §Q1 — the scope summary the overview cards read. It counts the *same* population the
// collector walks, and reports the part of it this round cannot reach, instead of turning
// a truncated selection into a coverage claim.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::contract::{known_limits_for, ExposureLimit, ExposureQueryError};
use super::repository::eligible_asset_scope;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureScopeSummary {
    pub project_id: i64,
    /// An archived project has no collection scope at all.
    pub project_active: bool,
    /// Admission-passing members of this project, however many there are.
    pub eligible_assets_total: i64,
    /// How many of them one passive round is actually willing to take.
    pub eligible_assets_planned: i64,
    /// The hidden difference §4.1 asked to make visible.
    pub assets_beyond_plan: i64,
    pub findings_total: i64,
    pub findings_new: i64,
    pub findings_reviewing: i64,
    pub findings_confirmed: i64,
    pub findings_dismissed: i64,
    /// Critical/high still waiting for a human: `confirmed` and `dismissed` are answers,
    /// not open work.
    pub high_risk_open: i64,
    /// G1-0: the caps this project's next round applies (its saved settings), not a
    /// global table of numbers.
    pub known_limits: Vec<ExposureLimit>,
    /// `project_settings_v<N>` or `defaults`; the page states which one it shows.
    pub limits_basis: String,
    /// Set when the saved settings are unreadable: no round can start, so nothing is planned
    /// and no limit is invented.
    pub limits_error: Option<String>,
    /// G1-2 how this project's assets map onto the organisation directory (read-only).
    pub unit_projection: Option<super::units::UnitProjection>,
    /// Set when the projection could not be read; the page says so instead of showing zeros.
    pub unit_projection_error: Option<String>,
    /// G1-3 findings auto-excluded as public procurement / public material. They stay listed
    /// but never count as open high risk.
    pub findings_excluded: i64,
    pub findings_classification_pending: i64,
    /// Findings with no classification row: recorded before G1-3 (old history stays readable).
    pub findings_unclassified: i64,
    pub classify_policy_version: String,
    /// G4-2 the latest round as it runs: stage, source, per-source items, usage, gaps.
    pub live_run: Option<serde_json::Value>,
    pub live_run_error: Option<String>,
    /// G4-1 the coverage matrix; `None` with an error when unreadable, never an empty grid.
    pub coverage: Option<serde_json::Value>,
    pub coverage_error: Option<String>,
    /// G4-1 group-unique incidents and per-unit involvement.
    pub unit_statistics: Option<serde_json::Value>,
    pub unit_statistics_error: Option<String>,
}

pub fn exposure_scope_summary(
    connection: &Connection,
    project_id: i64,
) -> Result<ExposureScopeSummary, ExposureQueryError> {
    if project_id <= 0 {
        return Err(ExposureQueryError::new(
            "exposure_project_invalid",
            "缺少有效的工作空间 id，暴露面汇总不能跨项目统计",
        ));
    }
    let (total, _, project_active) = eligible_asset_scope(connection, project_id)?;
    // G1-0: the planned count and the limits come from the same saved settings the next
    // round will freeze. Corrupt settings refuse to start a round, so nothing is planned.
    let saved_version: Option<i64> = connection
        .query_row(
            "SELECT version FROM exposure_settings WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ExposureQueryError::storage(&error.to_string()))?;
    let (planned, known_limits, limits_basis, limits_error) =
        match super::settings::load(connection, project_id) {
            Ok(settings) => (
                if project_active { total.min(settings.max_items) } else { 0 },
                known_limits_for(&settings),
                saved_version.map_or_else(|| "defaults".to_string(), |version| format!("project_settings_v{version}")),
                None,
            ),
            Err(error) => (0, Vec::new(), "settings_corrupt".to_string(), Some(error)),
        };
    let (unit_projection, unit_projection_error) = match super::units::project_units(connection, project_id) {
        Ok(projection) => (Some(projection), None),
        Err(error) => (None, Some(error)),
    };
    let counters = connection
        .query_row(
            "SELECT COUNT(*),\
             COALESCE(SUM(status='new'),0),\
             COALESCE(SUM(status='reviewing'),0),\
             COALESCE(SUM(status='confirmed'),0),\
             COALESCE(SUM(status='dismissed'),0),\
             COALESCE(SUM(severity IN ('critical','high') AND status IN ('new','reviewing') AND COALESCE(dispositions.disposition,'') NOT IN ('excluded_public_procurement','excluded_public_material')),0),\
             COALESCE(SUM(dispositions.disposition IN ('excluded_public_procurement','excluded_public_material')),0),\
             COALESCE(SUM(dispositions.disposition='classification_pending'),0),\
             COALESCE(SUM(dispositions.disposition IS NULL),0) \
             FROM exposure_findings f LEFT JOIN exposure_finding_dispositions dispositions ON dispositions.finding_id=f.id \
             WHERE f.project_id=?1",
            [project_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .map_err(|error| ExposureQueryError::storage(&error.to_string()))?;
    let (live_run, live_run_error) = match super::live::live_run(connection, project_id) {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    };
    let beyond_plan = total.saturating_sub(planned);
    let (coverage, coverage_error) =
        match super::coverage::coverage_matrix(connection, project_id, unit_projection.as_ref(), beyond_plan) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
    let (unit_statistics, unit_statistics_error) =
        match super::coverage::unit_statistics(connection, project_id, unit_projection.as_ref()) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
    Ok(ExposureScopeSummary {
        project_id,
        project_active,
        eligible_assets_total: total,
        eligible_assets_planned: planned,
        assets_beyond_plan: total.saturating_sub(planned),
        findings_total: counters.0,
        findings_new: counters.1,
        findings_reviewing: counters.2,
        findings_confirmed: counters.3,
        findings_dismissed: counters.4,
        high_risk_open: counters.5,
        known_limits,
        limits_basis,
        limits_error,
        unit_projection,
        unit_projection_error,
        findings_excluded: counters.6,
        findings_classification_pending: counters.7,
        findings_unclassified: counters.8,
        classify_policy_version: super::classify::CLASSIFY_POLICY_VERSION.to_string(),
        live_run,
        live_run_error,
        coverage,
        coverage_error,
        unit_statistics,
        unit_statistics_error,
    })
}
