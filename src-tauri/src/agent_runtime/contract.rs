//! Backend-neutral runtime contract (§4, §15).
#![allow(dead_code)] // Phase 0 declares the whole contract; the broker (Phase 2),
                     // the orchestrator (Phase 3) and the scheduler (Phase 5) consume it. Every type
                     // here is exercised by tests.
use serde::{Deserialize, Serialize};

/// The coverage families every target owes. Single canonical definition; the
/// `commands` layer re-exports this instead of keeping its own copy.
pub const COVERAGE_FAMILIES: [&str; 7] = [
    "information_disclosure",
    "error_handling",
    "authentication_session",
    "authorization",
    "input_reflection_xss",
    "hidden_interface_discovery",
    "business_flow",
];

pub const RUNTIME_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentBackendKind {
    #[serde(rename = "native")]
    Native,
    #[serde(rename = "legacy_backend_removed")]
    LegacyRemoved,
}

impl AgentBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::LegacyRemoved => "legacy_backend_removed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "native" | "native_agent" | "native-agent" => Some(Self::Native),
            "legacy_backend_removed" => Some(Self::LegacyRemoved),
            _ => None,
        }
    }
}

/// Declared in Phase 0 as the target shape for the orchestrator; the scan
/// pipelines still carry the legacy mode string until Phase 3 (§17).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    Quick,
    Standard,
    Deep,
    Skip,
    ManualReview,
}

impl ScanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
            Self::Skip => "skip",
            Self::ManualReview => "manual_review",
        }
    }

    /// Unknown or empty modes fall back to `standard`, matching the existing
    /// web policy normalisation.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "quick" => Self::Quick,
            "deep" => Self::Deep,
            "skip" => Self::Skip,
            "manual_review" | "manual-review" => Self::ManualReview,
            _ => Self::Standard,
        }
    }

    pub fn runs_a_backend(self) -> bool {
        !matches!(self, Self::Skip | Self::ManualReview)
    }
}

/// Run rows store the role of every run, so this is live from Phase 0; the
/// scheduler that spawns non-coordinator roles lands in Phase 5.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Coordinator,
    SpaApiMapper,
    /// Read-only specialists over this attempt's sealed source analysis.
    RepoMapper,
    SourceAnalyst,
    /// The existing target-touching Web loop until bounded specialists replace it.
    WebExecutor,
    ExternalSurface,
    IdentitySession,
    Authorization,
    InputParser,
    Upload,
    BusinessLogic,
    Concurrency,
    ClientSide,
    DeepInvestigator,
    EvidenceReviewer,
}

impl AgentRole {
    /// Exhaustive inventory — keep in lockstep with the enum variants (U39).
    pub const ALL: &'static [AgentRole] = &[
        Self::Coordinator,
        Self::SpaApiMapper,
        Self::RepoMapper,
        Self::SourceAnalyst,
        Self::WebExecutor,
        Self::ExternalSurface,
        Self::IdentitySession,
        Self::Authorization,
        Self::InputParser,
        Self::Upload,
        Self::BusinessLogic,
        Self::Concurrency,
        Self::ClientSide,
        Self::DeepInvestigator,
        Self::EvidenceReviewer,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coordinator => "coordinator",
            Self::SpaApiMapper => "spa_api_mapper",
            Self::RepoMapper => "repo_mapper",
            Self::SourceAnalyst => "source_analyst",
            Self::WebExecutor => "web_executor",
            Self::ExternalSurface => "external_surface",
            Self::IdentitySession => "identity_session",
            Self::Authorization => "authorization",
            Self::InputParser => "input_parser",
            Self::Upload => "upload",
            Self::BusinessLogic => "business_logic",
            Self::Concurrency => "concurrency",
            Self::ClientSide => "client_side",
            Self::DeepInvestigator => "deep_investigator",
            Self::EvidenceReviewer => "evidence_reviewer",
        }
    }

    pub fn parse(value: &str) -> Self {
        Self::try_parse(value).unwrap_or(Self::Coordinator)
    }

    /// Persisted roles must be recognised before they can carry authority.
    /// Keep `parse` for legacy display callers, but storage readers reject an
    /// unknown word instead of treating it as a Coordinator.
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "coordinator" => Some(Self::Coordinator),
            "spa_api_mapper" => Some(Self::SpaApiMapper),
            "repo_mapper" => Some(Self::RepoMapper),
            "source_analyst" => Some(Self::SourceAnalyst),
            "web_executor" => Some(Self::WebExecutor),
            "external_surface" => Some(Self::ExternalSurface),
            "identity_session" => Some(Self::IdentitySession),
            "authorization" => Some(Self::Authorization),
            "input_parser" => Some(Self::InputParser),
            "upload" => Some(Self::Upload),
            "business_logic" => Some(Self::BusinessLogic),
            "concurrency" => Some(Self::Concurrency),
            "client_side" => Some(Self::ClientSide),
            "deep_investigator" => Some(Self::DeepInvestigator),
            "evidence_reviewer" => Some(Self::EvidenceReviewer),
            // Stage 1A §4.2: the retired names stay readable so history written by
            // an older build still loads, and the aliases land on the role that
            // owns that work now. Re-writing such a row produces the new value.
            "evidence_triage" => Some(Self::SpaApiMapper),
            "contract_verifier" => Some(Self::InputParser),
            "identity_comparator" => Some(Self::Authorization),
            "business_flow_analyst" => Some(Self::BusinessLogic),
            "attack_chain_correlator" => Some(Self::DeepInvestigator),
            "final_reviewer" => Some(Self::EvidenceReviewer),
            _ => None,
        }
    }

    /// Only the coordinator may reduce a terminal state (§19).
    pub fn may_reduce_terminal(self) -> bool {
        matches!(self, Self::Coordinator)
    }

    /// Roles that must schedule on the read_only_analysis lane (not upload/concurrency stubs).
    pub fn requires_read_only_analysis_lane(self) -> bool {
        matches!(
            self,
            Self::SpaApiMapper
                | Self::IdentitySession
                | Self::RepoMapper
                | Self::SourceAnalyst
                | Self::ClientSide
                | Self::InputParser
                | Self::BusinessLogic
        )
    }

    /// Roles admitted by `prepare_readonly_child` bootstrap.
    pub fn is_readonly_bootstrap_role(self) -> bool {
        self.requires_read_only_analysis_lane()
    }

    /// Web-root bootstrap roles that always preserve EvidenceReviewer floor
    /// (SpaApiMapper post-checks only when a web_mode projection exists).
    pub fn requires_web_reviewer_floor_on_bootstrap(self) -> bool {
        matches!(
            self,
            Self::ClientSide
                | Self::IdentitySession
                | Self::InputParser
                | Self::BusinessLogic
        )
    }

    /// Readonly specialist model binding lane is read_only_analysis for these
    /// roles (DeepInvestigator included; ExternalSurface stays target_touching).
    pub fn specialist_readonly_binding_lane(self) -> Option<&'static str> {
        match self {
            Self::SpaApiMapper
            | Self::IdentitySession
            | Self::DeepInvestigator
            | Self::RepoMapper
            | Self::SourceAnalyst
            | Self::ClientSide
            | Self::InputParser
            | Self::BusinessLogic => Some("read_only_analysis"),
            Self::EvidenceReviewer => Some("review"),
            Self::ExternalSurface => Some("target_touching"),
            _ => None,
        }
    }

    /// Roles whose undispatched lease expiry may mint a replacement worker (U17/U38).
    /// Authorization / Upload / Reviewer / Coordinator keep dedicated recovery paths.
    pub fn allows_undispatched_worker_replacement(self) -> bool {
        matches!(
            self,
            Self::SpaApiMapper
                | Self::IdentitySession
                | Self::DeepInvestigator
                | Self::ClientSide
                | Self::InputParser
                | Self::BusinessLogic
                | Self::ExternalSurface
        )
    }

    /// Web supervised readonly recovery (`prepare_supervised_readonly_child`).
    /// Source RepoMapper/SourceAnalyst keep dedicated recovery; DeepInvestigator
    /// is not on this bootstrap entry.
    pub fn allows_supervised_readonly_bootstrap(self) -> bool {
        matches!(
            self,
            Self::SpaApiMapper
                | Self::IdentitySession
                | Self::ClientSide
                | Self::InputParser
                | Self::BusinessLogic
        )
    }
}

/// Which orchestration a run uses. `multi` is activated only by a live fenced
/// Coordinator; the persisted default keeps historical rows on the single-run path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiAgentPolicy {
    #[default]
    Single,
    Shadow,
    Multi,
}

impl MultiAgentPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Shadow => "shadow",
            Self::Multi => "multi",
        }
    }

    /// Empty and unknown input means the behaviour the product ships today.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "shadow" => Self::Shadow,
            "multi" | "multi_agent" => Self::Multi,
            _ => Self::Single,
        }
    }
}

/// What a task is allowed to touch. A review task can never reach
/// the target, and the lane is stored as its own column rather than inside JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLane {
    TargetTouching,
    #[default]
    ReadOnlyAnalysis,
    Review,
}

impl AgentLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TargetTouching => "target_touching",
            Self::ReadOnlyAnalysis => "read_only_analysis",
            Self::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Self {
        Self::try_parse(value).unwrap_or(Self::ReadOnlyAnalysis)
    }

    /// A stored lane is either one of the three known words or the row is corrupt.
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "target_touching" => Some(Self::TargetTouching),
            "read_only_analysis" => Some(Self::ReadOnlyAnalysis),
            "review" => Some(Self::Review),
            _ => None,
        }
    }

    pub fn touches_target(self) -> bool {
        matches!(self, Self::TargetTouching)
    }
}

/// Stage 1A §4.4: the only lifecycle an assignment may follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentState {
    #[default]
    Prepared,
    Leased,
    Running,
    WaitingReview,
    Paused,
    NeedsEvidence,
    Completed,
    Failed,
    Cancelled,
    LeaseExpired,
}

impl AssignmentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::WaitingReview => "waiting_review",
            Self::Paused => "paused",
            Self::NeedsEvidence => "needs_evidence",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::LeaseExpired => "lease_expired",
        }
    }

    pub fn parse(value: &str) -> Self {
        Self::try_parse(value).unwrap_or(Self::Prepared)
    }

    /// A stored state outside this vocabulary is corruption, not "back to prepared":
    /// the latter would hand an unknown lifecycle back to the scheduler (§3.2).
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "prepared" => Some(Self::Prepared),
            "leased" => Some(Self::Leased),
            "running" => Some(Self::Running),
            "waiting_review" => Some(Self::WaitingReview),
            "paused" => Some(Self::Paused),
            "needs_evidence" => Some(Self::NeedsEvidence),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "lease_expired" => Some(Self::LeaseExpired),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::LeaseExpired
        )
    }
}

/// The transition table is closed: anything not listed here is refused, including
/// every move out of a terminal state (§4.4).
pub fn can_transition(from: AssignmentState, to: AssignmentState) -> bool {
    use AssignmentState::*;
    matches!(
        (from, to),
        (Prepared, Leased | Cancelled)
            | (Leased, Running | Cancelled | LeaseExpired)
            | (
                Running,
                WaitingReview
                    | Paused
                    | NeedsEvidence
                    | Completed
                    | Failed
                    | Cancelled
                    | LeaseExpired
            )
            | (Paused, Leased | Cancelled)
            | (NeedsEvidence, Leased | Cancelled)
            | (
                WaitingReview,
                Completed | NeedsEvidence | Failed | Cancelled
            )
    )
}

/// Declared in Phase 0 because the policy engine and the tool registry key on
/// it; the broker that consumes it lands in Phase 2 (§17).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    ReadOnly,
    ControlledWrite,
    IrreversibleBlocked,
}

impl SideEffectClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ControlledWrite => "controlled_write",
            Self::IrreversibleBlocked => "irreversible_blocked",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "controlled_write" => Self::ControlledWrite,
            "irreversible_blocked" => Self::IrreversibleBlocked,
            _ => Self::ReadOnly,
        }
    }
}

// The execution plan the runtime actually uses is the one in
// `commands::agent_runtime::AgentExecutionPlan`; §12 forbids a second public plan
// type, so the earlier Phase 0 scaffolding was folded into it rather than kept
// side by side. Identity handles live in `browser_auth_sessions`, and budget,
// scope, coverage and stop policy all travel with the frozen plan itself.

/// The six terminal states of §15.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Completed,
    BoundedCompleted,
    Incomplete,
    Limited,
    Failed,
    Cancelled,
    /// A continuation whose checkpoint cannot be reused: the user must re-run it
    /// explicitly. Never folded into `failed` (§3.6, §11).
    ResumeIncompatible,
    /// Phase 2 §5.2: local persistence failed, so no further budget may be spent.
    PersistenceFailure,
}

impl TerminalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::BoundedCompleted => "completed_with_gaps",
            Self::Incomplete => "paused",
            Self::Limited => "protected_stop",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::ResumeIncompatible => "resume_incompatible",
            Self::PersistenceFailure => "persistence_failure",
        }
    }

    /// Rows written before §11 used the old spellings, so `parse` accepts both.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "completed" => Some(Self::Completed),
            "completed_with_gaps" | "bounded_completed" | "boundedcompleted" => {
                Some(Self::BoundedCompleted)
            }
            "paused" | "incomplete" | "partial" => Some(Self::Incomplete),
            "protected_stop" | "limited" => Some(Self::Limited),
            "resume_incompatible" => Some(Self::ResumeIncompatible),
            "persistence_failure" => Some(Self::PersistenceFailure),
            "failed" => Some(Self::Failed),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// §11: the only terminal states a target may end in. Native, historical, resumed
    /// and fresh attempts all write their final status through here, so no call
    /// site assembles its own wording.
    pub fn to_sentinel_status(self) -> &'static str {
        self.as_str()
    }
}

/// Stable machine-readable reason codes.
pub mod terminal_code {
    pub const FINISH_TARGET: &str = "finish_target";
    pub const LEDGER_COMPLETE: &str = "coverage_ledger_complete";
    pub const HARD_TOKEN_BUDGET: &str = "hard_token_budget";
    pub const HARD_REQUEST_BUDGET: &str = "hard_request_budget";
    pub const HARD_WALL_TIME_BUDGET: &str = "hard_wall_time_budget";
    pub const SOFT_BUDGET_STALLED: &str = "soft_budget_without_progress";
    pub const NO_PROGRESS_WINDOW: &str = "no_progress_window";
    pub const CONFIRMED_CHALLENGE: &str = "confirmed_waf_or_challenge";
    pub const PERSISTENT_RATE_LIMIT: &str = "persistent_rate_limit";
    pub const UNSUPPORTED_CAPABILITY: &str = "unsupported_capability";
    pub const MODEL_TRANSPORT: &str = "model_provider_failure";
    pub const CONFIGURATION: &str = "configuration";
    pub const EVIDENCE_INTEGRITY: &str = "evidence_integrity";
    pub const USER_CANCELLED: &str = "user_cancelled";
    pub const SCOPE_BOUNDARY: &str = "scope_boundary";
    /// A continuation whose parent checkpoint cannot be reused safely. The attempt
    /// must be re-run explicitly; nothing is silently restarted (§11, §3.6).
    pub const RESUME_INCOMPATIBLE: &str = "resume_incompatible";
    /// Phase 2 §5.2: a local write failed and the run stopped before spending more.
    pub const PERSISTENCE_FAILURE: &str = "persistence_failure";
    /// Target effects or the local request ledger require human reconciliation.
    /// This is not an authorization to retry or refund a request.
    pub const REQUEST_RECONCILIATION_REQUIRED: &str = "request_reconciliation_required";
    pub const EXECUTION_AUTHORIZATION_DENIED: &str = "execution_authorization_denied";
}

/// Which outbound decision is being made. Every one of them goes through the same
/// scope gate (§5.2) so no tool can grow its own rule set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeSource {
    HttpReplay,
    IdentityComparison,
    DiscoveryProbe,
    BrowserEntry,
    /// A request the browser observed on its own; it is classified, never sent.
    BrowserObserved,
    /// The `location` of a 3xx we were given.
    RedirectTarget,
}

impl ScopeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HttpReplay => "http_replay",
            Self::IdentityComparison => "identity_comparison",
            Self::DiscoveryProbe => "discovery_probe",
            Self::BrowserEntry => "browser_entry",
            Self::BrowserObserved => "browser_observed",
            Self::RedirectTarget => "redirect_target",
        }
    }
}

/// What an outbound or observed URL turned out to be (§5.3). Only
/// `AuthorizedBusinessApi` may enter formal APIs, coverage or the model summary as
/// an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeClass {
    AuthorizedBusinessApi,
    AuthorizedDocument,
    AuthorizedStatic,
    ThirdPartyRequired,
    TelemetryOrNoise,
    OutOfScope,
}

impl ScopeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizedBusinessApi => "authorized_business_api",
            Self::AuthorizedDocument => "authorized_document",
            Self::AuthorizedStatic => "authorized_static",
            Self::ThirdPartyRequired => "third_party_required",
            Self::TelemetryOrNoise => "telemetry_or_noise",
            Self::OutOfScope => "out_of_scope",
        }
    }
}

/// The only three verdicts a scope check may return (§5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeDecision {
    Allow {
        normalized_origin: String,
        reason: String,
    },
    Ignore {
        reason: String,
    },
    Reject {
        code: String,
        reason: String,
    },
}

impl ScopeDecision {
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Allow { .. } => "",
            Self::Ignore { .. } => "ignored",
            Self::Reject { code, .. } => code.as_str(),
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Allow { reason, .. } | Self::Ignore { reason } | Self::Reject { reason, .. } => {
                reason
            }
        }
    }
}

/// A redirect target that may only be *shown* by the browser, never tested: an
/// identity provider jump is session collection, not an authorization probe (§5.4).
pub fn identity_provider_target(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    let Ok(parsed) = reqwest::Url::parse(&lowered) else {
        return false;
    };
    let path = parsed.path();
    let marks_path = [
        "login",
        "signin",
        "sign-in",
        "oauth",
        "authorize",
        "saml",
        "oidc",
        "cas",
        "passport",
        "idp",
        "account",
    ]
    .iter()
    .any(|marker| path.contains(marker));
    let marks_query = [
        "client_id",
        "redirect_uri",
        "response_type",
        "state",
        "scope",
    ]
    .iter()
    .any(|key| parsed.query_pairs().any(|(name, _)| name == *key));
    let host = parsed.host_str().unwrap_or_default();
    let idp_host = [
        "login.",
        "sso.",
        "auth.",
        "id.",
        "passport",
        "oauth",
        "oidc",
        "accounts.",
    ]
    .iter()
    .any(|marker| host.starts_with(marker) || host.contains(&format!(".{marker}")));
    marks_query || (marks_path && (idp_host || path.contains("authorize")))
}

/// What happened, as reported by any backend. The reducer turns this into the
/// only terminal state the run may hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSignals {
    pub cancelled: bool,
    pub configuration_error: Option<String>,
    pub protection_signal: Option<String>,
    pub hard_limit_reason: Option<String>,
    pub soft_budget_stall_reason: Option<String>,
    pub unsupported_capability: Option<String>,
    /// A continuation whose checkpoint could not be inherited (§3.6, §11). It is its
    /// own terminal state, never a model failure and never a silent fresh start.
    pub resume_incompatible: Option<String>,
    /// Phase 2 §5.2: the local record could not be written.
    pub persistence_failure: Option<String>,
    /// An uncertain request effect cannot be cleared by coverage or automatic resume.
    #[serde(default)]
    pub request_reconciliation_required: Option<String>,
    /// The current execution grant cannot be verified; no automatic renewal.
    #[serde(default)]
    pub execution_authorization_denied: Option<String>,
    pub ledger_closed: bool,
    pub pending_contracts: i64,
    pub evidence_records: i64,
    pub confirmed_findings: i64,
    pub covered_families: Vec<String>,
    pub required_families: Vec<String>,
    pub detail: String,
}

impl Default for TerminalSignals {
    fn default() -> Self {
        Self {
            cancelled: false,
            configuration_error: None,
            protection_signal: None,
            hard_limit_reason: None,
            soft_budget_stall_reason: None,
            unsupported_capability: None,
            resume_incompatible: None,
            persistence_failure: None,
            request_reconciliation_required: None,
            execution_authorization_denied: None,
            ledger_closed: false,
            pending_contracts: 0,
            evidence_records: 0,
            confirmed_findings: 0,
            covered_families: Vec::new(),
            required_families: COVERAGE_FAMILIES
                .iter()
                .map(|value| value.to_string())
                .collect(),
            detail: String::new(),
        }
    }
}

impl TerminalSignals {
    pub fn uncovered_families(&self) -> Vec<String> {
        self.required_families
            .iter()
            .filter(|family| !self.covered_families.iter().any(|value| value == *family))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Prepared,
    Running,
    Paused,
    Terminal,
    /// A run that was still open when its backend was retired. It is
    /// terminal, cannot be resumed and cannot be re-activated by any writer.
    LegacyBackendRemoved,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Terminal => "terminal",
            Self::LegacyBackendRemoved => "legacy_backend_removed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "terminal" => Self::Terminal,
            "legacy_backend_removed" => Self::LegacyBackendRemoved,
            _ => Self::Prepared,
        }
    }

    /// The run is over for every purpose: no resume, no re-open, no further spend.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal | Self::LegacyBackendRemoved)
    }
}

/// Append-only event vocabulary (§11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventKind {
    RunCreated,
    PlanFrozen,
    ModelRoundCompleted,
    ToolInvocationStarted,
    ToolInvocationCompleted,
    EvidenceProduced,
    CoverageUpdated,
    HypothesisUpdated,
    BudgetExpanded,
    ProtectionDetected,
    SnapshotWritten,
    LeaseExpired,
    TerminalReduced,
    /// A legacy checkpoint was migrated to the current schema (§4.2).
    CheckpointMigrated,
    /// An in-flight model request was cut short by a stop, with the transport
    /// timestamps that prove it really closed (§6.2).
    TransportCancelled,
}

impl AgentEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunCreated => "run_created",
            Self::PlanFrozen => "plan_frozen",
            Self::ModelRoundCompleted => "model_round_completed",
            Self::ToolInvocationStarted => "tool_invocation_started",
            Self::ToolInvocationCompleted => "tool_invocation_completed",
            Self::EvidenceProduced => "evidence_produced",
            Self::CoverageUpdated => "coverage_updated",
            Self::HypothesisUpdated => "hypothesis_updated",
            Self::BudgetExpanded => "budget_expanded",
            Self::ProtectionDetected => "protection_detected",
            Self::SnapshotWritten => "snapshot_written",
            Self::LeaseExpired => "lease_expired",
            Self::TerminalReduced => "terminal_reduced",
            Self::CheckpointMigrated => "checkpoint_migrated",
            Self::TransportCancelled => "transport_cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "plan_frozen" => Self::PlanFrozen,
            "model_round_completed" => Self::ModelRoundCompleted,
            "tool_invocation_started" => Self::ToolInvocationStarted,
            "tool_invocation_completed" => Self::ToolInvocationCompleted,
            "evidence_produced" => Self::EvidenceProduced,
            "coverage_updated" => Self::CoverageUpdated,
            "hypothesis_updated" => Self::HypothesisUpdated,
            "budget_expanded" => Self::BudgetExpanded,
            "protection_detected" => Self::ProtectionDetected,
            "snapshot_written" => Self::SnapshotWritten,
            "lease_expired" => Self::LeaseExpired,
            "terminal_reduced" => Self::TerminalReduced,
            "checkpoint_migrated" => Self::CheckpointMigrated,
            "transport_cancelled" => Self::TransportCancelled,
            _ => Self::RunCreated,
        }
    }
}

/// Typed mailbox: table and contract land in Phase 0, multi-agent scheduling
/// in Phase 5 (§17).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    Assignment,
    EvidenceProduced,
    HypothesisUpdated,
    CoverageUpdated,
    CapabilityUnavailable,
    BudgetReturned,
    Completed,
    Failed,
    Cancelled,
    /// Persisted mailbox vocabulary consumed by the fenced coordination loop.
    GapProposed,
    ProposalAssessed,
    ReviewRequested,
    ReviewDecided,
    HumanDirective,
}

impl AgentMessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assignment => "assignment",
            Self::EvidenceProduced => "evidence_produced",
            Self::HypothesisUpdated => "hypothesis_updated",
            Self::CoverageUpdated => "coverage_updated",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::BudgetReturned => "budget_returned",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::GapProposed => "gap_proposed",
            Self::ProposalAssessed => "proposal_assessed",
            Self::ReviewRequested => "review_requested",
            Self::ReviewDecided => "review_decided",
            Self::HumanDirective => "human_directive",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "evidence_produced" => Self::EvidenceProduced,
            "hypothesis_updated" => Self::HypothesisUpdated,
            "coverage_updated" => Self::CoverageUpdated,
            "capability_unavailable" => Self::CapabilityUnavailable,
            "budget_returned" => Self::BudgetReturned,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "gap_proposed" => Self::GapProposed,
            "proposal_assessed" => Self::ProposalAssessed,
            "review_requested" => Self::ReviewRequested,
            "review_decided" => Self::ReviewDecided,
            "human_directive" => Self::HumanDirective,
            _ => Self::Assignment,
        }
    }
}
