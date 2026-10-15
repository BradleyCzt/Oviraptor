//! Hackerone transport contracts. Compatibility exports live in `crate::models`.
use serde::{Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HackerOneProgram {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub icon_url: String,
    pub policy: String,
    pub submission_state: String,
    pub program_state: String,
    pub offers_bounties: bool,
    pub open_scope: bool,
    pub fast_payments: bool,
    pub safe_harbor: bool,
    pub collaboration: bool,
    pub last_synced_at: String,
    pub bookmarked: bool,
    pub scope_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HackerOneScope {
    pub id: String,
    pub asset_type: String,
    pub asset_identifier: String,
    pub eligible_for_submission: bool,
    pub eligible_for_bounty: bool,
    pub max_severity: String,
    pub instruction: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HackerOneExclusion {
    pub id: String,
    pub category: String,
    pub details: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HackerOneDetail {
    pub program: HackerOneProgram,
    pub scopes: Vec<HackerOneScope>,
    pub exclusions: Vec<HackerOneExclusion>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HackerOneEvent {
    pub id: i64,
    pub program_handle: String,
    pub event_type: String,
    pub summary: String,
    pub created_at: String,
}
