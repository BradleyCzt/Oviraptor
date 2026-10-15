//! Scanning transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthSession {
    pub id: String,
    pub project_id: i64,
    pub owner_scan_id: String,
    pub draft_scope_id: String,
    pub name: String,
    pub entry_url: String,
    pub final_url: String,
    pub status: String,
    pub scope_hosts: Vec<String>,
    pub cookie_count: i64,
    pub header_count: i64,
    pub storage_count: i64,
    pub captured_request_count: i64,
    pub last_validated_at: String,
    pub expires_at: String,
    pub last_error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthSessionInput {
    pub id: Option<String>,
    pub project_id: i64,
    pub name: String,
    pub entry_url: String,
    #[serde(default)]
    pub draft_scope_id: String,
    #[serde(default)]
    pub scan_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRulePack {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub engine: String,
    pub repository: String,
    pub reference: String,
    pub local_path: String,
    pub previous_version: String,
    pub version: String,
    pub enabled: bool,
    pub builtin: bool,
    pub status: String,
    pub last_sync_at: String,
    pub error: String,
    pub added_count: i64,
    pub modified_count: i64,
    pub deleted_count: i64,
    pub change_summary: Value,
    pub progress: i64,
    pub progress_stage: String,
    pub progress_message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRulePackInput {
    pub key: String,
    pub name: String,
    pub engine: String,
    pub repository: String,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceToolStat {
    pub name: String,
    pub calls: i64,
    pub results: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceSummary {
    pub source_authority: String,
    pub scan_id: String,
    pub task_name: String,
    pub project_name: String,
    pub status: String,
    pub scan_type: String,
    pub model: String,
    pub run_count: i64,
    pub agent_count: i64,
    pub message_count: i64,
    pub reasoning_count: i64,
    pub tool_call_count: i64,
    pub tool_result_count: i64,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub hooked_request_count: i64,
    pub exact_request_capture: bool,
    pub usage_entry_count: i64,
    pub usage_agent_count: i64,
    pub token_usage_estimated: bool,
    pub instruction_hash: String,
    pub tools: Vec<AgentTraceToolStat>,
    pub knowledge_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceEvent {
    pub id: String,
    pub session_id: String,
    pub call_id: String,
    pub target_url: String,
    pub event_type: String,
    pub role: String,
    pub name: String,
    pub status: String,
    pub detail: String,
    pub detail_size: i64,
    pub detail_truncated: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceDetail {
    pub summary: AgentTraceSummary,
    pub events: Vec<AgentTraceEvent>,
    pub prompt_audit: Option<ModelPromptAudit>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelPromptAudit {
    pub capture_mode: String,
    pub source: String,
    pub capture_level: String,
    pub exact_model_request: bool,
    pub model: String,
    pub deployment: String,
    pub full_power: bool,
    pub recorded_at: String,
    pub instruction_sha256: String,
    pub instruction_chars: i64,
    pub instruction: Option<String>,
    pub notice: String,
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchScanInput {
    pub project_id: i64,
    pub task_name: String,
    pub scan_type: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub skill_ids: Vec<i64>,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub scan_mode: String,
    #[serde(default)]
    pub scope_mode: String,
    #[serde(default)]
    pub diff_base: String,
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub auth_profile_name: String,
    #[serde(default)]
    pub auth_type: String,
    #[serde(default)]
    pub auth_header_name: String,
    #[serde(default)]
    pub auth_value: String,
    #[serde(default)]
    pub auth_session_id: String,
    #[serde(default)]
    pub auth_session_ids: Vec<String>,
    #[serde(default)]
    pub auth_session_scope_id: String,
    #[serde(default)]
    pub ci_provider: String,
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default)]
    pub build_id: String,
    #[serde(default)]
    pub max_critical: i64,
    #[serde(default = "default_max_high")]
    pub max_high: i64,
    #[serde(default)]
    pub block_release: bool,
}

fn default_max_high() -> i64 {
    5
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelScan {
    pub id: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub status: String,
    pub current_checkpoint: String,
    pub task_path: String,
    pub previous_scan_id: String,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub scan_type: String,
    pub task_name: String,
    pub source_path: String,
    pub skill_names: String,
    pub attempt_count: i64,
    pub archived_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub requested_scan_mode: String,
    pub llm_model: String,
    pub llm_deployment: String,
    pub llm_full_power: bool,
    pub latest_attempt_number: i64,
    pub latest_attempt_status: String,
    pub latest_attempt_checkpoint: String,
    pub latest_attempt_stop_reason: String,
    #[serde(default)]
    pub administrative_closure_recorded: bool,
    #[serde(default)]
    pub closure_handoff_recorded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelScanAttempt {
    pub scan_id: String,
    pub attempt_number: i64,
    pub execution_mode: String,
    pub status: String,
    pub stage: String,
    pub checkpoint: String,
    pub stop_reason: String,
    pub work_dir: String,
    pub backend_plan_json: String,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub started_at: String,
    pub finished_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelTarget {
    pub id: i64,
    pub project_id: i64,
    pub scan_id: Option<String>,
    pub company: String,
    pub url: String,
    pub status: String,
    pub value_score: i64,
    pub scan_mode: String,
    pub routing_reason: String,
    pub last_attempt_number: i64,
    pub created_at: String,
    pub updated_at: String,
    pub scan_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFuseEntry {
    pub id: i64,
    pub project_id: i64,
    pub asset_id: Option<i64>,
    pub company: String,
    pub url: String,
    pub source_scan_id: String,
    pub reason: String,
    pub verdict: String,
    pub note: String,
    pub evidence: String,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFuseReviewInput {
    pub id: i64,
    pub verdict: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelCheckpoint {
    pub scan_id: String,
    pub url: String,
    pub stage: String,
    pub raw_json: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidation {
    pub id: i64,
    pub scan_id: String,
    pub url: String,
    pub finding_key: String,
    pub finding_kind: String,
    pub verdict: String,
    pub severity: String,
    pub note: String,
    pub evidence: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidationWorkItem {
    pub finding_id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub task_name: String,
    pub url: String,
    pub finding_key: String,
    pub finding_kind: String,
    pub title: String,
    pub original_severity: String,
    pub record_json: String,
    pub validation_id: Option<i64>,
    pub verdict: String,
    pub confirmed_severity: String,
    pub note: String,
    pub evidence: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidationInput {
    pub scan_id: String,
    pub url: String,
    #[serde(default = "default_validation_key")]
    pub finding_key: String,
    #[serde(default)]
    pub finding_kind: String,
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFinding {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub stage: String,
    pub kind: String,
    pub record_key: String,
    pub title: String,
    pub severity: String,
    pub record_json: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelOpportunity {
    pub id: i64,
    pub project_id: Option<i64>,
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_key: String,
    pub category: String,
    pub title: String,
    pub score: i64,
    pub status: String,
    pub confidence: String,
    pub why: Value,
    pub evidence: Value,
    pub recommended_action: Value,
    pub source: String,
    pub record: Value,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelOverviewStats {
    pub task_count: i64,
    pub url_count: i64,
    pub fingerprint_count: i64,
    pub api_count: i64,
    pub endpoint_count: i64,
    pub vulnerability_count: i64,
    pub high_risk_count: i64,
    /// Only current Native publications backed by a confirmed Reviewer decision.
    pub reviewer_confirmed_count: i64,
    pub reviewer_high_risk_count: i64,
    pub source_reviewer_confirmed_count: i64,
    pub source_review_audited_task_count: i64,
    pub source_review_unavailable_task_count: i64,
    pub source_review_unverified_task_count: i64,
    /// Historical/manual/unreviewed records, not Native Reviewer confirmations.
    pub other_vulnerability_count: i64,
    pub validated_count: i64,
    pub pending_vulnerability_count: i64,
    pub vulnerable_url_count: i64,
    pub active_fuse_count: i64,
    pub opportunity_count: i64,
    pub ready_opportunity_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationValidationInput {
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_id: Option<i64>,
    pub hypothesis_id: Option<i64>,
    pub api_key: Option<String>,
    pub identity_id: Option<String>,
    pub method: String,
    pub request_url: String,
    #[serde(default)]
    pub request_headers: Value,
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub response_status: i64,
    #[serde(default)]
    pub response_status_text: String,
    #[serde(default)]
    pub response_headers: Value,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub decoded_body: String,
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub ai_assessment: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub next_action: String,
    #[serde(default)]
    pub evidence_refs: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationValidation {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_id: Option<i64>,
    pub hypothesis_id: Option<i64>,
    pub api_key: String,
    pub identity_id: String,
    pub method: String,
    pub request_url: String,
    pub request_headers: Value,
    pub request_body: String,
    pub response_status: i64,
    pub response_status_text: String,
    pub response_headers: Value,
    pub response_body: String,
    pub decoded_body: String,
    pub verdict: String,
    pub severity: String,
    pub confidence: String,
    pub ai_assessment: String,
    pub note: String,
    pub next_action: String,
    pub evidence_refs: Value,
    pub created_at: String,
    pub updated_at: String,
}

fn default_validation_key() -> String {
    "url-summary".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationNode {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub node_key: String,
    pub node_type: String,
    pub label: String,
    pub confidence: String,
    pub value_score: i64,
    pub status: String,
    pub payload: Value,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationEdge {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub source_key: String,
    pub relation: String,
    pub target_key: String,
    pub confidence: String,
    pub evidence: Value,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationAction {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub action_key: String,
    pub state_key: String,
    pub action_type: String,
    pub label: String,
    pub outcome: String,
    pub value_score: i64,
    pub protocol: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationApiModel {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub api_key: String,
    pub method: String,
    pub url: String,
    pub normalized_path: String,
    pub source: String,
    pub confidence: String,
    pub auth_scope: String,
    pub parameters: Value,
    pub request_schema: Value,
    pub response_schema: Value,
    pub state_keys: Value,
    pub action_keys: Value,
    pub identity_keys: Value,
    pub observed_count: i64,
    pub baseline_status: String,
    pub payload: Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationHypothesis {
    pub id: i64,
    pub project_id: Option<i64>,
    pub scan_id: String,
    pub target_url: String,
    pub hypothesis_key: String,
    pub category: String,
    pub title: String,
    pub status: String,
    pub score: i64,
    pub confidence: String,
    pub contract: Value,
    pub evidence: Value,
    pub decision: Value,
    pub mutation_approval: Value,
    pub source_opportunity_key: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationIdentityDiff {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub api_key: String,
    pub left_identity_key: String,
    pub right_identity_key: String,
    pub difference_type: String,
    pub risk_score: i64,
    pub status: String,
    pub matrix: Value,
    pub created_at: String,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMetrics {
    pub scan_id: String,
    pub target_url: String,
    pub node_count: i64,
    pub edge_count: i64,
    pub state_count: i64,
    pub action_count: i64,
    pub api_count: i64,
    pub parameter_count: i64,
    pub hypothesis_count: i64,
    pub added_count: i64,
    pub changed_count: i64,
    pub removed_count: i64,
    pub duplicate_count: i64,
    pub information_gain: i64,
    pub token_worthy: bool,
    pub stop_reason: String,
    pub decision: Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationGraph {
    pub scan_id: String,
    pub target_url: String,
    pub nodes: Vec<InvestigationNode>,
    pub edges: Vec<InvestigationEdge>,
    pub actions: Vec<InvestigationAction>,
    pub apis: Vec<InvestigationApiModel>,
    pub related_services: Vec<Value>,
    pub hypotheses: Vec<InvestigationHypothesis>,
    pub identity_diffs: Vec<InvestigationIdentityDiff>,
    pub metrics: Option<InvestigationMetrics>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationHypothesisUpdateInput {
    pub hypothesis_id: i64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMutationApprovalInput {
    pub hypothesis_id: i64,
    pub approved: bool,
    pub max_attempts: Option<i64>,
    pub expires_minutes: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationOverview {
    pub target_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub api_count: i64,
    pub parameter_count: i64,
    pub hypothesis_count: i64,
    pub ready_hypothesis_count: i64,
    pub identity_diff_count: i64,
    pub token_worthy_count: i64,
    pub average_information_gain: i64,
    pub fact_count: i64,
    pub promoted_strategy_count: i64,
}

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
