//! Application transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};
use crate::asset_models::{InterruptedJob};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub project_count: i64,
    pub asset_count: i64,
    pub alive_count: i64,
    pub pending_count: i64,
    pub new_count: i64,
    pub changed_count: i64,
    pub blocked_count: i64,
    pub running_jobs: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub reminder_days: i64,
    pub custom_icon: bool,
    pub deduplicated_assets: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsInput {
    pub reminder_days: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleProject {
    pub project_id: i64,
    pub project_name: String,
    pub days_since_update: Option<i64>,
    pub last_run_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    pub reminder_days: i64,
    pub stale_projects: Vec<StaleProject>,
    pub interrupted_jobs: Vec<InterruptedJob>,
}
