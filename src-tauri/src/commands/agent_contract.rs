// Compatibility layer over `agent_runtime::contract` for the flat `commands`
// module. Canonical enums, budgets and terminal states live in the runtime
// module; this file keeps the shapes the existing scan, evidence and checkpoint
// code still speaks, and aliases the runtime vocabulary instead of repeating it.
use crate::agent_runtime::contract::{
    identity_provider_target, AgentBackendKind, COVERAGE_FAMILIES, ScopeClass, ScopeDecision,
    ScopeSource,
};

pub use crate::agent_runtime::contract::terminal_code;

const AGENT_STOP_WAF: &str = terminal_code::CONFIRMED_CHALLENGE;
const AGENT_STOP_RATE_LIMIT: &str = terminal_code::PERSISTENT_RATE_LIMIT;
const AGENT_STOP_HARD_TOKENS: &str = terminal_code::HARD_TOKEN_BUDGET;
const AGENT_STOP_SOFT_TOKENS: &str = terminal_code::SOFT_BUDGET_STALLED;
const AGENT_STOP_HARD_REQUESTS: &str = terminal_code::HARD_REQUEST_BUDGET;
const AGENT_STOP_SOFT_REQUESTS: &str = "soft_request_budget";
const AGENT_STOP_TIMEOUT: &str = terminal_code::MODEL_TRANSPORT;
const AGENT_STOP_NO_PROGRESS: &str = terminal_code::NO_PROGRESS_WINDOW;
const AGENT_STOP_USER_CANCELLED: &str = terminal_code::USER_CANCELLED;
const AGENT_STOP_MODEL_FAILURE: &str = terminal_code::MODEL_TRANSPORT;
const AGENT_STOP_CONFIGURATION: &str = terminal_code::CONFIGURATION;
const AGENT_STOP_EVIDENCE_INTEGRITY: &str = terminal_code::EVIDENCE_INTEGRITY;
const AGENT_STOP_TOOL_FAILURE: &str = "tool_failure";
const AGENT_STOP_SCOPE: &str = terminal_code::SCOPE_BOUNDARY;
const AGENT_STOP_FINISH: &str = terminal_code::FINISH_TARGET;
const AGENT_STOP_DERIVED: &str = terminal_code::LEDGER_COMPLETE;
const AGENT_STOP_UNSUPPORTED: &str = terminal_code::UNSUPPORTED_CAPABILITY;
const AGENT_STOP_RESUME_INCOMPATIBLE: &str = terminal_code::RESUME_INCOMPATIBLE;
const AGENT_STOP_PERSISTENCE: &str = terminal_code::PERSISTENCE_FAILURE;

const AGENT_COVERAGE_FAMILIES: [&str; 7] = COVERAGE_FAMILIES;

fn agent_not_applicable_is_real(reason: &str) -> bool {
    let text = reason.trim();
    if text.chars().count() < 4 {
        return false;
    }
    let omission = ["未覆盖", "未执行", "未测试", "未进行", "本轮未", "没有时间", "暂不"]
        .iter()
        .any(|needle| text.contains(needle));
    let structural_reason = [
        "缺少",
        "不存在",
        "不具备",
        "不适用",
        "没有可执行",
        "没有业务",
        "无业务",
        "无此",
        "目标未提供",
    ]
    .iter()
    .any(|needle| text.contains(needle));
    !omission || structural_reason
}

fn agent_coverage_family_label(family: &str) -> String {
    match family {
        "information_disclosure" => "信息泄露",
        "error_handling" => "错误处理",
        "authentication_session" => "认证与会话",
        "authorization" => "授权边界",
        "input_reflection_xss" => "输入反射与 XSS",
        "hidden_interface_discovery" => "隐藏接口发现",
        "business_flow" => "业务流程",
        other => other,
    }
    .to_string()
}

/// Classify a human-readable stop reason exactly once, so no backend invents
/// its own WAF, budget or no-progress vocabulary.
fn agent_stop_code(reason: &str) -> &'static str {
    let text = reason.to_ascii_lowercase();
    if text.contains(AGENT_STOP_UNSUPPORTED) {
        return AGENT_STOP_UNSUPPORTED;
    }
    if text.contains("完整性") || text.contains("integrity") {
        return AGENT_STOP_EVIDENCE_INTEGRITY;
    }
    if text.contains("配置") || text.contains("未安装") || text.contains("无法启动") {
        return AGENT_STOP_CONFIGURATION;
    }
    if text.contains("超出范围") || text.contains("scope") {
        return AGENT_STOP_SCOPE;
    }
    if hard_fuse_reason(reason) {
        if text.contains("429")
            || text.contains("限流")
            || text.contains("rate limit")
            || text.contains("rate-limit")
        {
            return AGENT_STOP_RATE_LIMIT;
        }
        return AGENT_STOP_WAF;
    }
    if text.contains("硬上限") || text.contains("绝对上限") {
        if text.contains("token") {
            return AGENT_STOP_HARD_TOKENS;
        }
        return AGENT_STOP_HARD_REQUESTS;
    }
    if text.contains("软预算") {
        if text.contains("token") {
            return AGENT_STOP_SOFT_TOKENS;
        }
        return AGENT_STOP_SOFT_REQUESTS;
    }
    if text.contains("秒没有") || (text.contains("超过") && text.contains("上限")) {
        return AGENT_STOP_TIMEOUT;
    }
    if text.contains("没有新增") || text.contains("无进展") || text.contains("没有不同") {
        return AGENT_STOP_NO_PROGRESS;
    }
    if text.contains("接口错误") || text.contains("接口返回") || text.contains("上下文") {
        return AGENT_STOP_MODEL_FAILURE;
    }
    AGENT_STOP_TOOL_FAILURE
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct AgentStop {
    code: &'static str,
    reason: String,
}

impl AgentStop {
    fn new(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    fn from_reason(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let code = agent_stop_code(&reason);
        Self { code, reason }
    }

    /// Only an explicit challenge or a persistent rate limit fuses the target;
    /// ordinary 401/403 stay permission boundaries.
    fn requires_fuse(&self) -> bool {
        matches!(self.code, AGENT_STOP_WAF | AGENT_STOP_RATE_LIMIT)
            || hard_fuse_reason(&self.reason)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentCompletion {
    summary: String,
    terminal_code: &'static str,
    /// `false` means the backend reports no coverage ledger; the UI must not
    /// render zero counts as "nothing covered".
    ledger_reported: bool,
    model_requests: i64,
    total_tokens: i64,
    verified_tool_results: i64,
    covered_families: Vec<String>,
    uncovered_families: Vec<String>,
    confirmed_findings: i64,
}

impl AgentCompletion {
    #[cfg(test)]
    fn without_ledger(summary: impl Into<String>, terminal_code: &'static str) -> Self {
        Self {
            summary: summary.into(),
            terminal_code,
            ledger_reported: false,
            model_requests: 0,
            total_tokens: 0,
            verified_tool_results: 0,
            covered_families: Vec::new(),
            uncovered_families: Vec::new(),
            confirmed_findings: 0,
        }
    }

    #[cfg(test)]
    fn bounded(summary: impl Into<String>) -> Self {
        Self::without_ledger(summary, AGENT_STOP_DERIVED)
    }

}

/// The terminal outcome vocabulary of the scan pipelines. The runtime reducer
/// maps it onto `TerminalState` (§15) via `runtime_adapter::BackendReport`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentTargetOutcome {
    Completed(AgentCompletion),
    BoundedCompleted(AgentCompletion),
    Incomplete(AgentStop),
    Limited(AgentStop),
    Failed(AgentStop),
    /// §11: the continuation could not inherit the parent's state. Distinct from
    /// `Failed` so the UI never reads it as a model or tool error.
    ResumeIncompatible(AgentStop),
    Cancelled,
}

impl AgentTargetOutcome {
    fn stop(&self) -> Option<&AgentStop> {
        match self {
            Self::Incomplete(stop)
            | Self::Limited(stop)
            | Self::Failed(stop)
            | Self::ResumeIncompatible(stop) => Some(stop),
            _ => None,
        }
    }

    fn completion(&self) -> Option<&AgentCompletion> {
        match self {
            Self::Completed(completion) | Self::BoundedCompleted(completion) => Some(completion),
            _ => None,
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Completed(completion) | Self::BoundedCompleted(completion) => {
                completion.summary.clone()
            }
            Self::Incomplete(stop)
            | Self::Limited(stop)
            | Self::Failed(stop)
            | Self::ResumeIncompatible(stop) => stop.reason.clone(),
            Self::Cancelled => AGENT_STOP_USER_CANCELLED.to_string(),
        }
    }

    /// §11/§12: the status string comes from the one canonical reducer, so a
    /// resumed and fresh Native attempts cannot disagree about the wording.
    fn terminal_status(&self) -> &'static str {
        use crate::agent_runtime::contract::TerminalState;
        if self.terminal_code() == AGENT_STOP_PERSISTENCE {
            return TerminalState::PersistenceFailure.to_sentinel_status();
        }
        let state = match self {
            Self::Completed(_) => TerminalState::Completed,
            Self::BoundedCompleted(_) => TerminalState::BoundedCompleted,
            Self::Incomplete(_) => TerminalState::Incomplete,
            Self::Limited(_) => TerminalState::Limited,
            Self::Failed(_) => TerminalState::Failed,
            Self::Cancelled => TerminalState::Cancelled,
            Self::ResumeIncompatible(_) => TerminalState::ResumeIncompatible,
        };
        state.to_sentinel_status()
    }

    fn terminal_code(&self) -> &'static str {
        match self {
            Self::Completed(completion) | Self::BoundedCompleted(completion) => {
                completion.terminal_code
            }
            Self::Incomplete(stop)
            | Self::Limited(stop)
            | Self::Failed(stop)
            | Self::ResumeIncompatible(stop) => stop.code,
            Self::Cancelled => AGENT_STOP_USER_CANCELLED,
        }
    }

    fn failed(reason: impl Into<String>) -> Self {
        Self::Failed(AgentStop::from_reason(reason))
    }

    fn incomplete(reason: impl Into<String>) -> Self {
        Self::Incomplete(AgentStop::from_reason(reason))
    }

    fn limited(reason: impl Into<String>) -> Self {
        Self::Limited(AgentStop::from_reason(reason))
    }

    /// The stop code is fixed here so no call site can invent another wording for
    /// an unusable continuation (§3.6, §11).
    /// §5.2: a local write failed, so the attempt stops with its own terminal state
    /// instead of pretending to be a model failure or an ordinary partial result.
    fn persistence_failure(reason: impl Into<String>) -> Self {
        Self::Failed(AgentStop {
            code: AGENT_STOP_PERSISTENCE,
            reason: reason.into(),
        })
    }

    fn resume_incompatible(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::ResumeIncompatible(AgentStop {
            code: AGENT_STOP_RESUME_INCOMPATIBLE,
            reason,
        })
    }

    #[cfg(test)]
    fn bounded_completed(summary: impl Into<String>) -> Self {
        Self::BoundedCompleted(AgentCompletion::bounded(summary))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentIdentity {
    key: String,
    session_id: Option<String>,
    anonymous: bool,
}

impl AgentIdentity {
    fn anonymous() -> Self {
        Self {
            key: "anonymous".to_string(),
            session_id: None,
            anonymous: true,
        }
    }

    fn scoped(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        Self {
            key: session_id.clone(),
            session_id: Some(session_id),
            anonymous: false,
        }
    }
}

/// A/B labels only exist when two identities really were captured. Single
/// account and anonymous tasks must never render 账号 A/B.
fn agent_identity_label(identities: &[AgentIdentity], key: &str) -> String {
    if key == "anonymous" {
        return "匿名会话".to_string();
    }
    let authenticated = identities
        .iter()
        .filter(|identity| !identity.anonymous)
        .collect::<Vec<_>>();
    match authenticated.iter().position(|identity| identity.key == key) {
        Some(_) if authenticated.len() < 2 => "当前身份".to_string(),
        Some(index) => format!("身份 {}（已认证）", (b'A' + index as u8) as char),
        None => key.to_string(),
    }
}

#[derive(Clone)]
struct AgentRunContext {
    tool_configuration: Option<crate::tool_supply::ToolAdministratorConfiguration>,
    supervision: Option<crate::agent_runtime::multi_agent::supervision_ticket::SupervisionTicket>,
    /// Orchestrated public-entry capture. Direct single-tool contexts do not
    /// schedule specialists; the production context builder enables this.
    external_surface: bool,
    scan_id: String,
    attempt_number: i64,
    target_url: String,
    target_dir: PathBuf,
    db_path: PathBuf,
    #[cfg(test)]
    route: FrontendRoute,
    execution_plan: AgentExecutionPlan,
    evidence: JsonValue,
    /// The SRC capability manifest staged for this target, shown to the model.
    capabilities: JsonValue,
    identities: Vec<AgentIdentity>,
    proxy: Option<String>,
    log_path: PathBuf,
    environment: ModelRuntimeEnv,
    /// What this task may drive a real browser with. `None` means the native
    /// backend has no browser runtime and must say so rather than pretend.
    browser: Option<AgentBrowserRuntime>,
    /// "继续未完成阶段" resumes the pending queue; "重新执行" starts fresh.
    resume: bool,
    /// The `agent_runs` row this attempt is registered under. `None` until the
    /// orchestrator has opened it, which keeps a directly-constructed context
    /// (tests, adapters) able to run without writing runtime facts.
    run: Option<AgentRunLedger>,
    /// The target-touching child receives a bounded slice of the root budget.
    /// Starting values come from the inherited checkpoint so a resumed child is
    /// charged only for work performed by its own assignment.
    run_budget: Option<AgentRunBudgetWindow>,
    /// Set when a continuation cannot inherit the parent's frozen plan. The loop
    /// must turn this into `resume_incompatible` instead of running (§3.6).
    plan_rejection: Option<NativeStateRejection>,
}

#[derive(Clone, Debug)]
struct AgentRunBudgetWindow {
    starting_tokens: i64,
    starting_requests: i64,
    hard_tokens: i64,
    hard_requests: i64,
}

/// Inputs needed to run one located action inside a real browser session.
#[derive(Clone)]
struct AgentBrowserRuntime {
    helper: PathBuf,
    runtime_path: OsString,
    no_proxy: String,
}

trait AgentBackend {
    fn kind(&self) -> AgentBackendKind;
    fn execute(&self, context: &AgentRunContext) -> AgentTargetOutcome;
}

/// Token growth and repeated tool calls are not progress (§6 rule 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AgentProgressDelta {
    new_endpoints: usize,
    new_response_shapes: usize,
    new_identity_differences: usize,
    new_parameters: usize,
    new_verdicts: usize,
    new_families: usize,
}

impl AgentProgressDelta {
    fn advanced(&self) -> bool {
        self.new_endpoints
            + self.new_response_shapes
            + self.new_identity_differences
            + self.new_parameters
            + self.new_verdicts
            + self.new_families
            > 0
    }

    fn signature(&self) -> String {
        format!(
            "e{}:r{}:i{}:p{}:v{}:f{}",
            self.new_endpoints,
            self.new_response_shapes,
            self.new_identity_differences,
            self.new_parameters,
            self.new_verdicts,
            self.new_families
        )
    }
}

const NATIVE_AGENT_STATE_STAGE: &str = "native_agent_state";
/// Where the pre-migration checkpoint is archived, so a migration stays auditable.
const NATIVE_AGENT_STATE_LEGACY_STAGE: &str = "native_agent_state_legacy";
const NATIVE_AGENT_STATE_SCHEMA_VERSION: i64 = 2;
const AGENT_EVIDENCE_STAGE: &str = "native-agent-evidence";
const AGENT_COVERAGE_STAGE: &str = "native-agent-coverage";
const AGENT_VULNERABILITY_STAGE: &str = "native-agent";

/// Legacy per-target checkpoint. The canonical run state now lives in
/// `agent_runs` plus `agent_events`; this row keeps the in-flight loop
/// resumable until the orchestrator moves onto the event store (Phase 3).
#[derive(Clone, Debug)]
struct NativeAgentState {
    schema_version: i64,
    attempt_number: i64,
    backend: String,
    evidence_hash: String,
    execution_plan_hash: String,
    completed_contract_keys: Vec<String>,
    exhausted_contract_keys: Vec<String>,
    covered_families: Vec<String>,
    pending_queue: Vec<String>,
    progress_signature: String,
    budget_usage: JsonValue,
    terminal_reason: String,
    turns: i64,
    no_progress_streak: i64,
    /// How many times a no-progress window was turned into a check instead of a stop.
    stall_checks: i64,
    /// One allowed doubling of the model-request ceiling while the queue is still open.
    budget_extensions: i64,
    token_usage: AgentTokenUsage,
    last_expansion_reason: String,
    /// Every request the native loop actually sent. Coverage and queue progress
    /// are derived from these, so a restart can neither re-grant HTTP budget nor
    /// re-run a contract that already produced a verdict.
    observed_requests: Vec<AgentRequestTrace>,
    discovery_rounds: i64,
    target_requests: i64,
    contract_attempts: Vec<(String, i64)>,
    contract_outcomes: Vec<(String, String)>,
    verdict_keys: Vec<String>,
    confirmed_findings: i64,
    /// §9: the coverage ledger Rust derived from executed work. A model may suggest
    /// a family, it may never write this list.
    coverage_evidence: Vec<CoverageEvidence>,
    /// §3.2: which attempt this one continues, and how it was started. A
    /// continuation keeps its own `attempt_number` for a fresh audit window and
    /// records the parent explicitly instead of pretending to be the parent.
    parent_attempt_number: Option<i64>,
    resume_kind: AgentResumeKind,
    /// Schema version the inherited checkpoint was written with, when any.
    inherited_checkpoint_schema: Option<i64>,
}

/// One executed request, as the chain ledger records it. `id` is what coverage
/// evidence and a confirmed finding bind to: a model may quote it but can never
/// invent one (§9.2, §10.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AgentRequestTrace {
    id: String,
    method: String,
    origin: String,
    path: String,
    identity: String,
    status: i64,
    family: String,
    contract_key: String,
    tool: String,
    invocation_id: i64,
    artifact_id: String,
    structure_hash: String,
    scope_class: String,
    /// Business parameter names the request carried, sorted. A control/test pair is
    /// only a contrast when identity or parameters differ (§10.2).
    #[serde(default)]
    parameters: Vec<String>,
}

/// One piece of coverage evidence the runtime derived from executed work (§9.2).
/// `request_record_ids` and `tool_invocation_ids` are the only things that can
/// make a family `covered`; `reason_code` says why it is not, when it is not.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverageEvidence {
    id: String,
    family: String,
    evidence_kind: String,
    contract_key: String,
    tool_invocation_ids: Vec<String>,
    request_record_ids: Vec<String>,
    verdict_id: String,
    result: String,
    reason_code: String,
}

impl NativeAgentState {
    fn fresh(
        attempt_number: i64,
        backend: AgentBackendKind,
        evidence_hash: &str,
        execution_plan_hash: &str,
        pending_queue: Vec<String>,
    ) -> Self {
        Self {
            schema_version: NATIVE_AGENT_STATE_SCHEMA_VERSION,
            attempt_number,
            backend: backend.as_str().to_string(),
            evidence_hash: evidence_hash.to_string(),
            execution_plan_hash: execution_plan_hash.to_string(),
            completed_contract_keys: Vec::new(),
            exhausted_contract_keys: Vec::new(),
            covered_families: Vec::new(),
            pending_queue,
            progress_signature: String::new(),
            budget_usage: serde_json::json!({}),
            terminal_reason: String::new(),
            turns: 0,
            no_progress_streak: 0,
            stall_checks: 0,
            budget_extensions: 0,
            token_usage: AgentTokenUsage::default(),
            last_expansion_reason: String::new(),
            observed_requests: Vec::new(),
            discovery_rounds: 0,
            target_requests: 0,
            contract_attempts: Vec::new(),
            contract_outcomes: Vec::new(),
            verdict_keys: Vec::new(),
            confirmed_findings: 0,
            coverage_evidence: Vec::new(),
            parent_attempt_number: None,
            resume_kind: AgentResumeKind::Initial,
            inherited_checkpoint_schema: None,
        }
    }

    /// A lineage-correct starting point for this attempt.
    fn start(
        context: &AgentRunContext,
        lineage: &AgentAttemptLineage,
        evidence_hash: &str,
        execution_plan_hash: &str,
        pending_queue: Vec<String>,
    ) -> Self {
        Self {
            attempt_number: context.attempt_number,
            parent_attempt_number: lineage.parent_attempt_number,
            resume_kind: lineage.resume_kind,
            ..Self::fresh(
                context.attempt_number,
                AgentBackendKind::Native,
                evidence_hash,
                execution_plan_hash,
                pending_queue,
            )
        }
    }

    /// A contract counts as finished only with a recorded verdict; attempts that
    /// hit the limit without one are exhausted, never completed.
    fn completed_contracts(&self) -> Vec<String> {
        self.contract_outcomes
            .iter()
            .filter(|(_, outcome)| {
                matches!(
                    outcome.as_str(),
                    "confirmed" | "rejected" | "exhausted" | "insufficient_evidence"
                )
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn attempts_for(&self, contract_key: &str) -> i64 {
        self.contract_attempts
            .iter()
            .find(|(key, _)| key == contract_key)
            .map(|(_, attempts)| *attempts)
            .unwrap_or(0)
    }

    fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "schemaVersion": self.schema_version,
            "attemptNumber": self.attempt_number,
            "backend": self.backend,
            "evidenceHash": self.evidence_hash,
            "executionPlanHash": self.execution_plan_hash,
            "completedContractKeys": self.completed_contract_keys,
            "exhaustedContractKeys": self.exhausted_contract_keys,
            "coveredFamilies": self.covered_families,
            "pendingQueue": self.pending_queue,
            "progressSignature": self.progress_signature,
            "budgetUsage": self.budget_usage,
            "terminalReason": self.terminal_reason,
            "turns": self.turns,
            "noProgressStreak": self.no_progress_streak,
            "stallChecks": self.stall_checks,
            "budgetExtensions": self.budget_extensions,
            "tokenUsage": self.token_usage.as_json(),
            "lastExpansionReason": self.last_expansion_reason,
            "observedRequests": self.observed_requests,
            "discoveryRounds": self.discovery_rounds,
            "targetRequests": self.target_requests,
            "contractAttempts": self.contract_attempts,
            "contractOutcomes": self.contract_outcomes,
            "verdictKeys": self.verdict_keys,
            "coverageEvidence": self.coverage_evidence,
            "confirmedFindings": self.confirmed_findings,
            "parentAttemptNumber": self.parent_attempt_number,
            "resumeKind": self.resume_kind.as_str(),
            "inheritedCheckpointSchema": self.inherited_checkpoint_schema,
        })
    }

    fn from_json(value: &JsonValue) -> Option<Self> {
        if value.get("schemaVersion").and_then(JsonValue::as_i64)
            != Some(NATIVE_AGENT_STATE_SCHEMA_VERSION)
        {
            return None;
        }
        let strings = |pointer: &str| -> Vec<String> {
            value
                .pointer(pointer)
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let usage = value.get("tokenUsage").cloned().unwrap_or(JsonValue::Null);
        let number = |key: &str| usage.get(key).and_then(JsonValue::as_i64).unwrap_or(0);
        Some(Self {
            schema_version: NATIVE_AGENT_STATE_SCHEMA_VERSION,
            attempt_number: value
                .get("attemptNumber")
                .and_then(JsonValue::as_i64)
                .unwrap_or(1),
            backend: value
                .get("backend")
                .and_then(JsonValue::as_str)
                .unwrap_or("native")
                .to_string(),
            evidence_hash: value
                .get("evidenceHash")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            execution_plan_hash: value
                .get("executionPlanHash")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            completed_contract_keys: strings("/completedContractKeys"),
            exhausted_contract_keys: strings("/exhaustedContractKeys"),
            covered_families: strings("/coveredFamilies"),
            pending_queue: strings("/pendingQueue"),
            progress_signature: value
                .get("progressSignature")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            budget_usage: value
                .get("budgetUsage")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            terminal_reason: value
                .get("terminalReason")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            turns: value.get("turns").and_then(JsonValue::as_i64).unwrap_or(0),
            no_progress_streak: value
                .get("noProgressStreak")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            stall_checks: value.get("stallChecks").and_then(JsonValue::as_i64).unwrap_or(0),
            budget_extensions: value
                .get("budgetExtensions")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            token_usage: AgentTokenUsage {
                input_tokens: number("inputTokens"),
                cached_input_tokens: number("cachedInputTokens"),
                output_tokens: number("outputTokens"),
                total_tokens: number("totalTokens"),
                model_requests: number("modelRequests"),
            },
            last_expansion_reason: value
                .get("lastExpansionReason")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            observed_requests: serde_json::from_value(
                value
                    .get("observedRequests")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .unwrap_or_default(),
            discovery_rounds: value
                .get("discoveryRounds")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            target_requests: value
                .get("targetRequests")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            contract_attempts: value
                .get("contractAttempts")
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_array)
                        .filter_map(|pair| {
                            Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_i64()?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            contract_outcomes: value
                .get("contractOutcomes")
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_array)
                        .filter_map(|pair| {
                            Some((
                                pair.first()?.as_str()?.to_string(),
                                pair.get(1)?.as_str()?.to_string(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            verdict_keys: strings("/verdictKeys"),
            coverage_evidence: value
                .get("coverageEvidence")
                .cloned()
                .map(|rows| serde_json::from_value::<Vec<CoverageEvidence>>(rows).unwrap_or_default())
                .unwrap_or_default(),
            confirmed_findings: value
                .get("confirmedFindings")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            parent_attempt_number: value
                .get("parentAttemptNumber")
                .and_then(JsonValue::as_i64)
                .map(|value| value.max(1)),
            resume_kind: AgentResumeKind::parse(
                value
                    .get("resumeKind")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("initial"),
            ),
            inherited_checkpoint_schema: value
                .get("inheritedCheckpointSchema")
                .and_then(JsonValue::as_i64),
        })
    }

    fn raw_checkpoint(db_path: &Path, scan_id: &str, url: &str) -> Option<String> {
        let connection = db::open(db_path).ok()?;
        let raw: String = connection
            .query_row(
                "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
                params![scan_id, url, NATIVE_AGENT_STATE_STAGE],
                |row| row.get(0),
            )
            .unwrap_or_default();
        (!raw.is_empty()).then_some(raw)
    }

    /// Read the checkpoint through the migrator. A v1 row is upgraded, written
    /// back as v2 and recorded as a migration event, with the original JSON kept
    /// for audit — it is never treated as "no checkpoint" and re-run (§4).
    fn load(db_path: &Path, scan_id: &str, url: &str) -> Result<Option<Self>, StateMigrationError> {
        let Some(raw) = Self::raw_checkpoint(db_path, scan_id, url) else {
            return Ok(None);
        };
        let Ok(parsed) = serde_json::from_str::<JsonValue>(&raw) else {
            return Err(StateMigrationError::CheckpointCorrupted(
                "检查点不是合法 JSON".to_string(),
            ));
        };
        let previous_version = parsed.get("schemaVersion").and_then(JsonValue::as_i64);
        let mut state = migrate_native_agent_state(&parsed)?;
        if previous_version != Some(NATIVE_AGENT_STATE_SCHEMA_VERSION) {
            let from = previous_version.unwrap_or(0);
            state.inherited_checkpoint_schema = Some(from);
            // §5.2: a migration that cannot be written back is not a migration. The
            // caller must stop rather than run on state it cannot restore.
            state
                .persist(db_path, scan_id, url)
                .map_err(StateMigrationError::PersistenceFailed)?;
            // The untouched original stays in its own row so an audit can compare
            // what was migrated from what is running now.
            write_agent_checkpoint(
                db_path,
                scan_id,
                url,
                NATIVE_AGENT_STATE_LEGACY_STAGE,
                &parsed,
            )
            .map_err(StateMigrationError::PersistenceFailed)?;
            append_native_event(
                db_path,
                scan_id,
                state.attempt_number,
                url,
                crate::agent_runtime::contract::AgentEventKind::CheckpointMigrated,
                serde_json::json!({
                    "fromSchemaVersion": from,
                    "toSchemaVersion": NATIVE_AGENT_STATE_SCHEMA_VERSION,
                    "original": parsed,
                }),
            );
        }
        Ok(Some(state))
    }

    fn read(db_path: &Path, scan_id: &str, url: &str) -> Option<Self> {
        Self::load(db_path, scan_id, url).ok().flatten()
    }

    /// §3.4 / §3.6: the only entry point the loop may use. A continuation
    /// inherits the parent attempt's ledger; anything that cannot be inherited
    /// safely is an explicit rejection rather than a silent fresh start.
    fn for_attempt(
        db_path: &Path,
        scan_id: &str,
        context: &AgentRunContext,
        evidence_hash: &str,
        plan_hash: &str,
    ) -> Result<Self, NativeStateRejection> {
        let lineage = agent_attempt_lineage(db_path, scan_id, context.attempt_number);
        let reject = |kind: NativeStateRejectionKind, detail: String| NativeStateRejection {
            kind,
            parent_attempt_number: lineage.parent_attempt_number,
            current_attempt_number: context.attempt_number,
            detail,
        };
        if lineage.resume_kind != AgentResumeKind::ContinueIncomplete {
            // 重新执行 (or the very first attempt): an independent attempt that
            // inherits no budget, queue, contracts or terminal identity.
            if let Err(error) = Self::clear(db_path, scan_id, &context.target_url) {
                return Err(reject(NativeStateRejectionKind::PersistenceFailed, error));
            }
            return Ok(Self::start(
                context,
                &lineage,
                evidence_hash,
                plan_hash,
                Vec::new(),
            ));
        }
        let parent = match Self::load(db_path, scan_id, &context.target_url) {
            Ok(Some(parent)) => parent,
            Ok(None) => {
                return Err(reject(
                    NativeStateRejectionKind::NoCheckpoint,
                    "该 URL 没有上一 attempt 的原生检查点".to_string(),
                ))
            }
            Err(StateMigrationError::CheckpointVersionTooNew(detail)) => {
                return Err(reject(NativeStateRejectionKind::SchemaTooNew, detail))
            }
            Err(StateMigrationError::CheckpointCorrupted(detail)) => {
                return Err(reject(NativeStateRejectionKind::Corrupted, detail))
            }
            Err(StateMigrationError::PersistenceFailed(detail)) => {
                return Err(reject(NativeStateRejectionKind::PersistenceFailed, detail))
            }
        };
        if parent.evidence_hash != evidence_hash {
            return Err(reject(
                NativeStateRejectionKind::EvidenceChanged,
                format!(
                    "父 attempt {}，当前 evidenceHash 与冻结值不同",
                    parent.attempt_number
                ),
            ));
        }
        if parent.execution_plan_hash != plan_hash {
            return Err(reject(
                NativeStateRejectionKind::PlanChanged,
                format!(
                    "父 attempt {} 的 planHash {} 与继承计划不一致",
                    parent.attempt_number, parent.execution_plan_hash
                ),
            ));
        }
        if parent.target_requests > 0 && parent.observed_requests.is_empty() {
            // A migrated checkpoint that cannot say *which* requests were already
            // spent must not be resumed as if it could (§4.2).
            return Err(reject(
                NativeStateRejectionKind::ResumeLedgerIncomplete,
                "旧检查点声明已消耗目标请求，但没有请求账本可继承".to_string(),
            ));
        }
        Ok(Self {
            schema_version: NATIVE_AGENT_STATE_SCHEMA_VERSION,
            attempt_number: context.attempt_number,
            parent_attempt_number: lineage.parent_attempt_number,
            resume_kind: lineage.resume_kind,
            inherited_checkpoint_schema: Some(parent.schema_version),
            evidence_hash: parent.evidence_hash.clone(),
            execution_plan_hash: parent.execution_plan_hash.clone(),
            // Transient identity of the parent is dropped: a new attempt must not
            // inherit a reason that would end it immediately (§3.4).
            terminal_reason: String::new(),
            no_progress_streak: 0,
            stall_checks: 0,
            budget_extensions: 0,
            progress_signature: String::new(),
            last_expansion_reason: String::new(),
            // Everything reusable is carried forward verbatim.
            backend: parent.backend.clone(),
            // Recomputed from the recorded outcomes rather than copied, so a
            // parent row that was written by an older build cannot carry a
            // contract the model never closed.
            completed_contract_keys: parent.completed_contracts(),
            exhausted_contract_keys: parent.exhausted_contract_keys.clone(),
            covered_families: parent.covered_families.clone(),
            pending_queue: parent.pending_queue.clone(),
            budget_usage: parent.budget_usage.clone(),
            turns: parent.turns,
            token_usage: parent.token_usage,
            observed_requests: parent.observed_requests.clone(),
            discovery_rounds: parent.discovery_rounds,
            target_requests: parent.target_requests,
            contract_attempts: parent.contract_attempts.clone(),
            contract_outcomes: parent.contract_outcomes.clone(),
            verdict_keys: parent.verdict_keys.clone(),
            coverage_evidence: parent.coverage_evidence.clone(),
            confirmed_findings: parent.confirmed_findings,
        })
    }


    fn persist(&self, db_path: &Path, scan_id: &str, url: &str) -> Result<(), String> {
        let mut connection = db::open(db_path)?;
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        // Pausing workers may checkpoint, replaced or deleted workers may not.
        let current = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2)
             AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan_id, self.attempt_number], |r| r.get::<_, bool>(0),
        ).map_err(|e| e.to_string())?;
        if !current { return Err("native_attempt_stopped_or_replaced".into()); }
        transaction.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4)
             ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
            params![scan_id, url, NATIVE_AGENT_STATE_STAGE, self.as_json().to_string()],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    /// "重新执行" clears the running/budget/terminal identity of the old attempt
    /// while confirmed findings stay in `sentinel_findings`. The execution plan
    /// row is deliberately kept: it is the pinned backend of the attempt that is
    /// about to run, and overwriting it here would let a mid-scan configuration
    /// edit switch the backend.
    fn clear(db_path: &Path, scan_id: &str, url: &str) -> Result<(), String> {
        let connection = db::open(db_path).map_err(|error| error.to_string())?;
        // A fresh rerun is only fresh if the previous working row is really gone;
        // a swallowed error here would hand attempt 2 attempt 1's queue (§5.2).
        connection
            .execute(
                "DELETE FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage IN (?3,?4)",
                params![scan_id, url, NATIVE_AGENT_STATE_STAGE, AGENT_COVERAGE_STAGE],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

/// How this attempt relates to the one before it (§3.2). "继续未完成阶段" creates
/// a new attempt in the app, so inheritance has to be explicit data, never an
/// attempt-number equality that can never hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentResumeKind {
    /// First attempt of this scan.
    Initial,
    /// "重新执行": an independent attempt that inherits nothing.
    Fresh,
    /// "继续未完成阶段": inherits the parent's frozen plan and ledger.
    ContinueIncomplete,
}

impl AgentResumeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Fresh => "fresh",
            Self::ContinueIncomplete => "resume",
        }
    }

    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "resume" => Self::ContinueIncomplete,
            "fresh" => Self::Fresh,
            _ => Self::Initial,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentAttemptLineage {
    resume_kind: AgentResumeKind,
    parent_attempt_number: Option<i64>,
}

/// Read the lineage from the typed attempt row, not from UI wording (§3.2).
fn agent_attempt_lineage(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
) -> AgentAttemptLineage {
    let Ok(connection) = db::open(db_path) else {
        return AgentAttemptLineage {
            resume_kind: AgentResumeKind::Initial,
            parent_attempt_number: None,
        };
    };
    let mode: Option<String> = connection
        .query_row(
            "SELECT execution_mode FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt_number],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let parent: Option<i64> = connection
        .query_row(
            "SELECT MAX(attempt_number) FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number<?2",
            params![scan_id, attempt_number],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();
    match AgentResumeKind::parse(mode.as_deref().unwrap_or("initial")) {
        AgentResumeKind::ContinueIncomplete => AgentAttemptLineage {
            resume_kind: AgentResumeKind::ContinueIncomplete,
            // Attempts are contiguous, so the previous row is the parent even if
            // the attempts table was pruned.
            parent_attempt_number: Some(parent.unwrap_or_else(|| (attempt_number - 1).max(1))),
        },
        other => AgentAttemptLineage {
            resume_kind: other,
            parent_attempt_number: None,
        },
    }
}

/// Why a continuation cannot be inherited. None of these may fall through to a
/// silent fresh restart (§3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeStateRejectionKind {
    NoCheckpoint,
    SchemaTooNew,
    Corrupted,
    EvidenceChanged,
    PlanChanged,
    ResumeLedgerIncomplete,
    /// §5.2: a local write failed. It is its own reason, not a model failure and
    /// not an incompatible checkpoint.
    PersistenceFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeStateRejection {
    kind: NativeStateRejectionKind,
    parent_attempt_number: Option<i64>,
    current_attempt_number: i64,
    detail: String,
}

impl NativeStateRejection {
    /// The wording is fixed by §3.6: the user must see "续跑状态不兼容，需要重新
    /// 执行", never a generic model failure.
    fn message(&self) -> String {
        let kind = match self.kind {
            NativeStateRejectionKind::NoCheckpoint => "父 attempt 没有可恢复状态",
            NativeStateRejectionKind::SchemaTooNew => "checkpoint 版本高于当前实现",
            NativeStateRejectionKind::Corrupted => "checkpoint 内容损坏",
            NativeStateRejectionKind::EvidenceChanged => "证据包已变化",
            NativeStateRejectionKind::PlanChanged => "冻结执行计划已变化",
            NativeStateRejectionKind::ResumeLedgerIncomplete => "旧 checkpoint 缺少请求账本",
            NativeStateRejectionKind::PersistenceFailed => "本地记录写入失败",
        };
        if self.kind == NativeStateRejectionKind::PersistenceFailed {
            return format!(
                "本地记录失败，已停止以避免重复消耗（{kind}；当前 attempt {}，细节：{}）",
                self.current_attempt_number, self.detail
            );
        }
        format!(
            "续跑状态不兼容，需要重新执行（{kind}；父 attempt {:?}，当前 attempt {}，细节：{}）",
            self.parent_attempt_number, self.current_attempt_number, self.detail
        )
    }
}

/// What `migrate_native_agent_state` refused to read (§4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
enum StateMigrationError {
    CheckpointVersionTooNew(String),
    CheckpointCorrupted(String),
    /// §5.2: the record could not be written, so nothing further may be spent.
    PersistenceFailed(String),
}

/// Read a persisted checkpoint of any supported schema. v2 parses directly, v1 is
/// migrated with conservative defaults, anything newer or unreadable is refused
/// loudly instead of being treated as absent (§4).
fn migrate_native_agent_state(
    value: &JsonValue,
) -> Result<NativeAgentState, StateMigrationError> {
    let corrupted = |detail: &str| StateMigrationError::CheckpointCorrupted(detail.to_string());
    if !value.is_object() {
        return Err(corrupted("checkpoint 不是 JSON 对象"));
    }
    let Some(version) = value.get("schemaVersion").and_then(JsonValue::as_i64) else {
        return Err(corrupted("checkpoint 缺少 schemaVersion"));
    };
    match version {
        NATIVE_AGENT_STATE_SCHEMA_VERSION => NativeAgentState::from_json(value)
            .ok_or_else(|| corrupted("v2 checkpoint 字段不完整")),
        1 => {
            let mut migrated = NativeAgentState::from_json(&upgrade_v1_to_v2(value))
                .ok_or_else(|| corrupted("v1 checkpoint 迁移后仍不完整"))?;
            migrated.schema_version = NATIVE_AGENT_STATE_SCHEMA_VERSION;
            Ok(migrated)
        }
        other => Err(StateMigrationError::CheckpointVersionTooNew(format!(
            "checkpoint schemaVersion={other} 高于本版本支持的 {NATIVE_AGENT_STATE_SCHEMA_VERSION}"
        ))),
    }
}

/// Map v1's two name lists onto the v2 outcome vocabulary. A completed contract
/// whose verdict was never recorded stays `unknown` rather than being rewritten as
/// a fresh rejection, so it is not silently re-closed either.
fn v1_contract_outcomes(completed: &[String], exhausted: &[String]) -> Vec<JsonValue> {
    completed
        .iter()
        .chain(exhausted.iter())
        .map(|key| {
            serde_json::json!([
                key,
                if exhausted.contains(key) { "exhausted" } else { "unknown" }
            ])
        })
        .collect()
}

/// v1 stored only the contract name lists and a free-form `budgetUsage`. Every
/// field that cannot be derived stays empty rather than being invented, so a
/// migrated run can never claim coverage it did not perform (§4.2).
fn upgrade_v1_to_v2(value: &JsonValue) -> JsonValue {
    let strings = |pointer: &str| -> Vec<String> {
        value
            .pointer(pointer)
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let completed = strings("/completedContractKeys");
    let exhausted = strings("/exhaustedContractKeys");
    let budget = value.get("budgetUsage").cloned().unwrap_or(JsonValue::Null);
    let derived = |key: &str| -> i64 { budget.get(key).and_then(JsonValue::as_i64).unwrap_or(0) };
    serde_json::json!({
        "schemaVersion": NATIVE_AGENT_STATE_SCHEMA_VERSION,
        "attemptNumber": value.get("attemptNumber").and_then(JsonValue::as_i64).unwrap_or(1),
        "backend": value.get("backend").and_then(JsonValue::as_str).unwrap_or("native"),
        "evidenceHash": value.get("evidenceHash").and_then(JsonValue::as_str).unwrap_or(""),
        "executionPlanHash": value.get("executionPlanHash").and_then(JsonValue::as_str).unwrap_or(""),
        "completedContractKeys": completed,
        "exhaustedContractKeys": exhausted,
        "coveredFamilies": strings("/coveredFamilies"),
        "pendingQueue": strings("/pendingQueue"),
        "progressSignature": value.get("progressSignature").and_then(JsonValue::as_str).unwrap_or(""),
        "budgetUsage": budget,
        "terminalReason": value.get("terminalReason").and_then(JsonValue::as_str).unwrap_or(""),
        "turns": value.get("turns").and_then(JsonValue::as_i64).unwrap_or(0),
        "noProgressStreak": value.get("noProgressStreak").and_then(JsonValue::as_i64).unwrap_or(0),
        "tokenUsage": value.get("tokenUsage").cloned().unwrap_or(JsonValue::Null),
        "lastExpansionReason": value.get("lastExpansionReason").and_then(JsonValue::as_str).unwrap_or(""),
        // v1 recorded no request ledger at all: never invent one.
        "observedRequests": [],
        "discoveryRounds": derived("discoveryRounds"),
        "targetRequests": derived("targetRequests"),
        "contractAttempts": completed
            .iter()
            .chain(exhausted.iter())
            .map(|key| serde_json::json!([key, 1]))
            .collect::<Vec<_>>(),
        "contractOutcomes": v1_contract_outcomes(&completed, &exhausted),
        "verdictKeys": [],
        "confirmedFindings": derived("confirmedFindings"),
        "parentAttemptNumber": null,
        "resumeKind": AgentResumeKind::Initial.as_str(),
        "inheritedCheckpointSchema": 1,
    })
}

/// One shared checkpoint upsert so `commands/*.rs` stop copy-pasting the SQL.
/// Phase 2 §5.2: a checkpoint write is a commit boundary, not a best effort. A
/// locked database gets a short, bounded retry; anything else is reported to the
/// caller, which stops the attempt instead of spending more budget on state it
/// cannot persist.
fn with_db_retry<T>(
    what: &str,
    mut attempt: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    const DELAYS: [u64; 3] = [0, 60, 180];
    let mut last = String::new();
    for delay in DELAYS {
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) => {
                let locked = error.contains("database is locked") || error.contains("SQLITE_BUSY");
                last = error;
                if !locked {
                    break;
                }
            }
        }
    }
    Err(format!("{what}失败：{last}"))
}

fn write_agent_checkpoint(
    db_path: &Path,
    scan_id: &str,
    url: &str,
    stage: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let payload = value.to_string();
    let (db_path, scan_id, url, stage) = (
        db_path.to_path_buf(),
        scan_id.to_string(),
        url.to_string(),
        stage.to_string(),
    );
    with_db_retry("检查点写入", || {
        let connection = db::open(&db_path).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, url, stage, payload],
            )
            .map_err(|error| error.to_string())
            .map(|_| ())
    })
}

fn read_agent_checkpoint(db_path: &Path, scan_id: &str, url: &str, stage: &str) -> JsonValue {
    let Ok(connection) = db::open(db_path) else {
        return JsonValue::Null;
    };
    let raw: String = connection
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![scan_id, url, stage],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if raw.is_empty() {
        JsonValue::Null
    } else {
        json(raw)
    }
}

fn agent_stable_hash(value: &JsonValue) -> String {
    crate::agent_runtime::store::stable_hash(&value.to_string())
}
