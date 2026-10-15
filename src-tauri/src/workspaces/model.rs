//! Workspaces transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub asset_count: i64,
    pub pending_count: i64,
    pub target_count: i64,
    pub asset_run_count: i64,
    pub scan_count: i64,
    pub vulnerability_count: i64,
    pub validation_count: i64,
    pub active_fuse_count: i64,
    pub last_run_at: Option<String>,
    pub last_scan_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImpact {
    pub asset_count: i64,
    pub asset_event_count: i64,
    pub target_count: i64,
    pub asset_run_count: i64,
    pub saved_view_count: i64,
    pub sentinel_scan_count: i64,
    pub sentinel_target_count: i64,
    pub finding_count: i64,
    pub validation_count: i64,
    pub opportunity_count: i64,
    pub fuse_count: i64,
    pub appsec_vulnerability_count: i64,
    pub knowledge_count: i64,
    pub learning_candidate_count: i64,
    pub browser_auth_session_count: i64,
    pub other_record_count: i64,
    pub total_records: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
}
