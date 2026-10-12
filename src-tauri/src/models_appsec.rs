// Source/appsec analysis, environment reports and the exposure-surface ledger.
// Included from models.rs.

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSecVulnerability {
    pub id: i64,
    pub project_id: i64,
    pub fingerprint: String,
    pub title: String,
    pub vulnerability_type: String,
    pub severity: String,
    pub status: String,
    pub confidence: String,
    pub asset: String,
    pub environment: String,
    pub url: String,
    pub http_method: String,
    pub parameter: String,
    pub file: String,
    pub symbol: String,
    pub start_line: i64,
    pub correlation_score: i64,
    pub correlation: Value,
    pub first_seen: String,
    pub last_seen: String,
    pub owner: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSecVulnerabilitySource {
    pub id: i64,
    pub vulnerability_id: i64,
    pub scan_id: String,
    pub finding_id: Option<i64>,
    pub source_type: String,
    pub source_key: String,
    pub engine: String,
    pub evidence: Value,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSecScanContext {
    pub scan_id: String,
    pub environment: String,
    pub auth_profile_name: String,
    pub auth_type: String,
    pub authenticated: bool,
    pub ci_provider: String,
    pub repository_url: String,
    pub branch: String,
    pub commit_sha: String,
    pub build_id: String,
    pub policy: Value,
    pub gate_status: String,
    pub gate_reason: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSecScanResult {
    pub vulnerabilities: Vec<AppSecVulnerability>,
    pub sources: Vec<AppSecVulnerabilitySource>,
    pub context: Option<AppSecScanContext>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentDependency {
    pub name: String,
    pub command: String,
    pub version: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub os: String,
    pub arch: String,
    pub node: String,
    pub dependencies: Vec<EnvironmentDependency>,
    pub checked_at: String,
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
