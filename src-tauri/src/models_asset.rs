// The asset workspace: query, paging, bulk decisions and ownership assessment types.
// Included from models.rs.

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
