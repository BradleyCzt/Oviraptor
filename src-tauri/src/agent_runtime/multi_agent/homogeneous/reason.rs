//! SPEC §1.1 的 37 条 reason_code + 深度分析 §J 授权的意图闸门增量。

pub const SCHEMA_INVALID: &str = "schema_invalid";
pub const UNKNOWN_ACTION: &str = "unknown_action";
pub const MISSING_REQUIRED_FIELD: &str = "missing_required_field";
pub const EXTRA_FIELD: &str = "extra_field";
pub const ALLOWLIST_VIOLATION: &str = "allowlist_violation";
pub const SLICE_OVERLAP: &str = "slice_overlap";
pub const BUDGET_EXCEEDED: &str = "budget_exceeded";
pub const TOOL_NOT_AUTHORIZED: &str = "tool_not_authorized";
pub const IDENTITY_CHANGE_FORBIDDEN: &str = "identity_change_forbidden";
pub const SCOPE_ESCAPE: &str = "scope_escape";
pub const URL_INVENTED: &str = "url_invented";
pub const CROSS_SLICE_WRITE: &str = "cross_slice_write";
pub const EVIDENCE_MISSING: &str = "evidence_missing";
pub const EVIDENCE_CONTRADICTION: &str = "evidence_contradiction";
pub const OUTPUT_SCHEMA_INVALID: &str = "output_schema_invalid";
pub const LANE_QUOTA_EXCEEDED: &str = "lane_quota_exceeded";
pub const WORKER_NOT_FOUND: &str = "worker_not_found";
pub const ASSIGNMENT_NOT_FOUND: &str = "assignment_not_found";
pub const DIRECTIVE_PRIVILEGE_ESCALATION: &str = "directive_privilege_escalation";
pub const DIRECTIVE_BUDGET_CHANGE: &str = "directive_budget_change";
pub const DIRECTIVE_IDENTITY_CHANGE: &str = "directive_identity_change";
pub const DIRECTIVE_CLEANUP_FORBIDDEN: &str = "directive_cleanup_forbidden";
pub const MACHINE_CHECK_FAILED: &str = "machine_check_failed";
pub const MODEL_JUDGEMENT_FAIL: &str = "model_judgement_fail";
pub const DUPLICATE_SLICE: &str = "duplicate_slice";
pub const STALE_EVIDENCE_REVISION: &str = "stale_evidence_revision";
pub const LEASE_INVALID: &str = "lease_invalid";
pub const DEGRADED_PATH_USED: &str = "degraded_path_used";
pub const USER_CANCELLED: &str = "user_cancelled";
pub const GOAL_REACHED: &str = "goal_reached";
pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";
pub const RETRY_LIMIT_EXCEEDED: &str = "retry_limit_exceeded";
pub const MARKDOWN_WRAPPED_JSON: &str = "markdown_wrapped_json";
pub const FREE_TEXT_INSTEAD_OF_JSON: &str = "free_text_instead_of_json";
pub const INVENTED_ROLE: &str = "invented_role";
pub const INVENTED_TOOL: &str = "invented_tool";
pub const INVENTED_STATE: &str = "invented_state";
pub const ZERO_INTENT_ILLEGAL: &str = "zero_intent_illegal";

pub const ALL: [&str; 38] = [
    SCHEMA_INVALID,
    UNKNOWN_ACTION,
    MISSING_REQUIRED_FIELD,
    EXTRA_FIELD,
    ALLOWLIST_VIOLATION,
    SLICE_OVERLAP,
    BUDGET_EXCEEDED,
    TOOL_NOT_AUTHORIZED,
    IDENTITY_CHANGE_FORBIDDEN,
    SCOPE_ESCAPE,
    URL_INVENTED,
    CROSS_SLICE_WRITE,
    EVIDENCE_MISSING,
    EVIDENCE_CONTRADICTION,
    OUTPUT_SCHEMA_INVALID,
    LANE_QUOTA_EXCEEDED,
    WORKER_NOT_FOUND,
    ASSIGNMENT_NOT_FOUND,
    DIRECTIVE_PRIVILEGE_ESCALATION,
    DIRECTIVE_BUDGET_CHANGE,
    DIRECTIVE_IDENTITY_CHANGE,
    DIRECTIVE_CLEANUP_FORBIDDEN,
    MACHINE_CHECK_FAILED,
    MODEL_JUDGEMENT_FAIL,
    DUPLICATE_SLICE,
    STALE_EVIDENCE_REVISION,
    LEASE_INVALID,
    DEGRADED_PATH_USED,
    USER_CANCELLED,
    GOAL_REACHED,
    BUDGET_EXHAUSTED,
    RETRY_LIMIT_EXCEEDED,
    MARKDOWN_WRAPPED_JSON,
    FREE_TEXT_INSTEAD_OF_JSON,
    INVENTED_ROLE,
    INVENTED_TOOL,
    INVENTED_STATE,
    ZERO_INTENT_ILLEGAL,
];

/// SPEC §2.7：`reject_directive` 只允许引导族的码。
pub const DIRECTIVE_RELATED: [&str; 7] = [
    DIRECTIVE_PRIVILEGE_ESCALATION,
    DIRECTIVE_BUDGET_CHANGE,
    DIRECTIVE_IDENTITY_CHANGE,
    DIRECTIVE_CLEANUP_FORBIDDEN,
    SCOPE_ESCAPE,
    URL_INVENTED,
    ALLOWLIST_VIOLATION,
];

/// 验收族（SPEC §5.3）。
pub const ACCEPTANCE_RELATED: [&str; 11] = [
    GOAL_REACHED,
    EVIDENCE_MISSING,
    EVIDENCE_CONTRADICTION,
    MACHINE_CHECK_FAILED,
    MODEL_JUDGEMENT_FAIL,
    RETRY_LIMIT_EXCEEDED,
    BUDGET_EXHAUSTED,
    SCOPE_ESCAPE,
    CROSS_SLICE_WRITE,
    OUTPUT_SCHEMA_INVALID,
    STALE_EVIDENCE_REVISION,
];

pub fn is_valid(code: &str) -> bool {
    ALL.contains(&code)
}
