//! Assets transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: i64,
    pub project_id: i64,
    pub target_type: String,
    pub value: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetImportInput {
    pub project_id: i64,
    pub target_type: String,
    pub values: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRun {
    pub id: i64,
    pub project_id: i64,
    pub profile_id: Option<i64>,
    pub project_name: String,
    pub name: String,
    pub pipeline: String,
    pub status: String,
    pub stage: String,
    pub progress: f64,
    pub processed: i64,
    pub total: i64,
    pub output_dir: String,
    pub error: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedJob {
    pub run_id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub profile_id: Option<i64>,
    pub name: String,
    pub pipeline: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartJobInput {
    pub project_id: i64,
    pub profile_id: i64,
    pub name: String,
    #[serde(default = "default_pipeline")]
    pub pipeline: String,
}

fn default_pipeline() -> String {
    "collect".to_string()
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressEvent {
    pub run_id: i64,
    pub status: String,
    pub stage: String,
    pub progress: f64,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: i64,
    pub run_id: Option<i64>,
    pub level: String,
    pub stage: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetEvent {
    pub id: i64,
    pub project_id: i64,
    pub asset_id: i64,
    pub asset_key: String,
    pub company: String,
    pub host: String,
    pub event_type: String,
    pub summary: String,
    pub run_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub query: AssetQuery,
    pub fields: Vec<String>,
    #[serde(default)]
    pub chinese_headers: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub rows: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub inserted: i64,
    pub updated: i64,
    pub invalid: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: i64,
    pub project_id: i64,
    pub asset_key: String,
    pub company: String,
    pub host: String,
    pub link: String,
    pub ip: String,
    pub port: String,
    pub protocol: String,
    pub domain: String,
    pub title: String,
    pub status_code: String,
    pub probe_outcome: String,
    pub probe_entry_state: String,
    pub review_tier: String,
    pub content_category: String,
    pub score: String,
    pub decision: String,
    pub note: String,
    pub ownership_status: String,
    pub ownership_confidence: i64,
    pub authorization_status: String,
    pub exposure_eligible: bool,
    pub ownership_source: String,
    pub ownership_reason: String,
    pub is_deleted: bool,
    pub first_seen: String,
    pub last_seen: String,
    pub last_alive: Option<String>,
    pub extra: Value,
    pub sentinel_status: String,
    pub sentinel_scan_count: i64,
    pub sentinel_sent_at: Option<String>,
    pub project_first_seen: String,
    pub project_last_seen: String,
    pub last_run_id: Option<i64>,
    pub deleted_at: Option<String>,
    pub project_name: String,
    /// -1 unknown, 0 absent, 1 present. Set only after a probe reads a page.
    pub has_login: i64,
    pub has_register: i64,
    pub entry_opened_at: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetQuery {
    pub project_id: Option<i64>,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub conditions: Vec<FilterCondition>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub deleted_view: String,
    #[serde(default)]
    pub probe_view: String,
    #[serde(default)]
    pub probe_outcome_view: String,
    #[serde(default)]
    pub sentinel_view: String,
    #[serde(default)]
    pub decision_view: String,
    #[serde(default)]
    pub ownership_view: String,
    #[serde(default)]
    pub has_login_view: String,
    #[serde(default)]
    pub has_register_view: String,
    #[serde(default)]
    pub entry_seen_view: String,
    #[serde(default)]
    pub sort_by: String,
    #[serde(default)]
    pub sort_direction: String,
    /// Presentation only. Consumers of raw assets retain every member by default.
    #[serde(default)]
    pub collapse_identical_responses: bool,
    #[serde(default)]
    pub response_fingerprint: String,
    /// Restrict member expansion to the representative's current display group.
    #[serde(default)]
    pub response_representative_asset_id: Option<i64>,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    50
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCondition {
    pub field: String,
    pub operator: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "default_join")]
    pub join: String,
}

fn default_join() -> String {
    "and".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPage {
    pub items: Vec<Asset>,
    pub total: i64,
    /// Original members matching all filters, before presentation folding.
    pub member_total: i64,
    pub page: i64,
    pub page_size: i64,
    pub summary: AssetSummary,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    pub all: i64,
    pub pending: i64,
    pub uncertain: i64,
    pub confirmed: i64,
    pub rejected: i64,
    pub not_applicable: i64,
    pub sent_to_agent: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRuleInput {
    pub keyword: String,
    pub source_asset_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRuleApplyResult {
    pub rule_id: i64,
    pub keyword: String,
    pub matched_assets: i64,
    pub matched_project_assets: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInput {
    pub project_id: i64,
    pub asset_ids: Vec<i64>,
    pub decision: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssetSelection {
    pub project_id: i64,
    pub asset_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBulkDecisionInput {
    pub selections: Vec<AssetSelection>,
    pub decision: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBulkArchiveInput {
    pub selections: Vec<AssetSelection>,
    pub deleted: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetOwnershipProfile {
    pub project_id: i64,
    pub legal_name: String,
    pub jurisdiction: String,
    pub jurisdictions: Vec<String>,
    pub excluded_jurisdictions: Vec<String>,
    pub aliases: Vec<String>,
    pub approved_domains: Vec<String>,
    pub shared_domains: Vec<String>,
    pub excluded_names: Vec<String>,
    pub excluded_domains: Vec<String>,
    pub notes: String,
    pub policy_version: i64,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetOwnershipDecisionInput {
    pub selections: Vec<AssetSelection>,
    pub status: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub learn_domain_rule: bool,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetOwnershipSummary {
    pub all: i64,
    pub unreviewed: i64,
    pub attributed: i64,
    pub confirmed: i64,
    pub related: i64,
    pub third_party: i64,
    pub excluded: i64,
    pub exposure_eligible: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetOwnershipAssessmentResult {
    pub assessed: i64,
    pub summary: AssetOwnershipSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FofaApiTestInput {
    pub key: String,
    #[serde(default)]
    pub proxy_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FofaApiTestResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub account: String,
    pub plan: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureFinding {
    pub id: i64,
    pub project_id: i64,
    pub asset_id: Option<i64>,
    pub category: String,
    pub title: String,
    pub source_type: String,
    pub source_url: String,
    pub evidence_excerpt: String,
    pub severity: String,
    pub confidence: i64,
    pub status: String,
    pub note: String,
    pub asset_label: String,
    pub exposure_eligible: bool,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureFindingInput {
    pub project_id: i64,
    pub asset_id: Option<i64>,
    pub category: String,
    pub title: String,
    pub source_type: String,
    pub source_url: String,
    pub evidence_excerpt: String,
    pub severity: String,
    pub confidence: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureSummary {
    pub eligible_assets: i64,
    pub total: i64,
    pub new_count: i64,
    pub reviewing: i64,
    pub confirmed: i64,
    pub dismissed: i64,
    pub high_risk: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureRun {
    pub id: i64,
    pub project_id: i64,
    pub status: String,
    pub eligible_assets: i64,
    pub scanned_assets: i64,
    pub fetched_resources: i64,
    pub findings: i64,
    pub stage: String,
    pub current_source: String,
    pub error: String,
    pub created_at: String,
    pub started_at: String,
    pub completed_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureSourceResult {
    pub run_id: i64,
    pub source_key: String,
    pub status: String,
    pub item_count: i64,
    pub error: String,
    pub completed_at: String,
}
