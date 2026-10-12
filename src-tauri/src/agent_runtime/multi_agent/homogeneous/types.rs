//! Phase 1 同质 Worker + 强主管：封闭集合的 Rust 镜像。
//!
//! 成员一律照抄 SPEC §1.1 / §1.7 / §2.5 / §2.6，禁止增删。这里只有值与判定，
//! 没有任何「默认宽容」：非法值必须返回 None，由调用方给出 reason_code。

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const PROFILE_HOMOGENEOUS_V1: &str = "homogeneous_v1";
pub const SCHEDULING_LEGACY: &str = "legacy_role_pipeline";
pub const SCHEDULING_HOMOGENEOUS: &str = "homogeneous_workers";

pub const ACTIONS: [&str; 7] = [
    "decompose",
    "assign",
    "accept",
    "reject_retry",
    "stop",
    "relay_directive",
    "reject_directive",
];
pub const TASK_STATES: [&str; 8] = [
    "created",
    "decomposed",
    "running",
    "partial_success",
    "succeeded",
    "failed",
    "cancelled",
    "stopped",
];
pub const SLICE_STATES: [&str; 7] = [
    "pending",
    "assigned",
    "in_progress",
    "reported",
    "accepted",
    "rejected",
    "cancelled",
];
pub const ASSIGNMENT_STATES: [&str; 8] = [
    "created",
    "assigned",
    "running",
    "reported",
    "accepted",
    "rejected_retry",
    "stopped",
    "cancelled",
];
pub const DIRECTIVE_STATES: [&str; 7] = [
    "received",
    "rule_screened",
    "awaiting_coordinator",
    "relayed",
    "rewritten",
    "rejected",
    "applied",
];
pub const VERDICTS: [&str; 3] = ["accept", "retry", "reject"];
pub const LANES: [&str; 3] = ["read_only_analysis", "target_touching", "review"];
pub const SCOPE_KINDS: [&str; 4] = ["path_prefix", "host", "asset_id", "source_type"];
pub const STOP_REASONS: [&str; 5] = [
    "goal_reached",
    "budget_exhausted",
    "user_cancelled",
    "degraded_path_used",
    "retry_limit_exceeded",
];
pub const RULE_SCREEN_RESULTS: [&str; 3] = ["pass", "need_confirm", "reject"];
/// SPEC §1.7 `coordinator_action`：与 SPEC §1.8 的 `action` 是两层名字，不可混用。
pub const DIRECTIVE_COORDINATOR_ACTIONS: [&str; 3] = ["relay", "rewrite", "reject"];
/// SPEC §2.6 `payload.mode`。
pub const RELAY_MODES: [&str; 2] = ["relay", "rewrite"];
pub const TERMINAL_HINTS: [&str; 3] = ["completed", "failed", "partial"];
/// SPEC §1.3 WorkerInstance.state。
pub const WORKER_STATES: [&str; 4] = ["idle", "busy", "draining", "dead"];
/// 深度分析 §J 的唯一增量；主管动作仍严格保持七个。
pub const ZERO_INTENT_REASONS: [&str; 2] = ["covered_by_inflight", "blocked_on_inflight_output"];

pub const MAX_SLICES: usize = 16;
pub const MAX_SCOPE_VALUE_LEN: usize = 512;
pub const MAX_MODEL_NOTES_LEN: usize = 200;
pub const MAX_FEEDBACK_ZH_LEN: usize = 500;
pub const MAX_RAW_TEXT_LEN: usize = 2000;

pub fn is_member(value: &str, set: &[&str]) -> bool {
    set.contains(&value)
}

/// SPEC pattern `^{prefix}-[a-f0-9]{min,max}$`（小写十六进制，长度含在区间内）。
pub fn valid_id(prefix: &str, value: &str, min: usize, max: usize) -> bool {
    let Some(body) = value.strip_prefix(&format!("{prefix}-")) else {
        return false;
    };
    let bytes = body.as_bytes();
    bytes.len() >= min
        && bytes.len() <= max
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (*byte >= b'a' && *byte <= b'f'))
}

pub fn valid_evidence_ref(value: &str) -> bool {
    valid_id("evd", value, 8, 32)
}

/// 无控制字符 + 长度区间：SPEC §1.7 / §2.7 对可读中文串的要求。
pub fn valid_readable_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max
        && !value
            .chars()
            .any(|ch| (ch.is_ascii_control() && ch != '\n' && ch != '\t') || ch == '\r')
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AllowlistRefs {
    pub slice_ids: Vec<String>,
    pub worker_ids: Vec<String>,
    pub tool_ids: Vec<String>,
    pub target_keys: Vec<String>,
    pub directive_ids: Vec<String>,
    pub assignment_ids: Vec<String>,
}

/// SPEC §1.8 CoordinatorDecision：主管唯一输出信封。枚举字段先收 String，
/// 再由 `decision.rs` 判定并给 reason_code——避免 serde 的枚举报错吞掉语义。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CoordinatorDecision {
    pub decision_id: String,
    pub action: String,
    pub parent_task_id: String,
    pub payload: JsonValue,
    pub allowlist_refs: AllowlistRefs,
    #[serde(default)]
    pub model_notes_zh: Option<String>,
    #[serde(default)]
    pub zero_intent: Option<ZeroIntent>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ZeroIntent {
    pub reason: String,
    pub evidence_assignment_ids: Vec<String>,
    pub note_zh: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MachineChecks {
    pub scope_in_bounds: bool,
    pub budget_ok: bool,
    pub tools_authorized: bool,
    pub output_schema_ok: bool,
    pub evidence_refs_exist: bool,
    pub evidence_revision_fresh: bool,
    pub no_cross_slice_write: bool,
}

impl MachineChecks {
    pub fn failed_keys(&self) -> Vec<&'static str> {
        let mut failed = Vec::new();
        if !self.scope_in_bounds {
            failed.push("scope_in_bounds");
        }
        if !self.budget_ok {
            failed.push("budget_ok");
        }
        if !self.tools_authorized {
            failed.push("tools_authorized");
        }
        if !self.output_schema_ok {
            failed.push("output_schema_ok");
        }
        if !self.evidence_refs_exist {
            failed.push("evidence_refs_exist");
        }
        if !self.evidence_revision_fresh {
            failed.push("evidence_revision_fresh");
        }
        if !self.no_cross_slice_write {
            failed.push("no_cross_slice_write");
        }
        failed
    }

    pub fn all_true(&self) -> bool {
        self.failed_keys().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ModelJudgement {
    pub adequate: bool,
    pub question_zh: String,
    pub answer_zh: String,
}

/// SPEC §2.1 decompose payload 的切片项（不含 state/retry_count：那是落库后的身份）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SliceDraft {
    pub slice_id: String,
    pub scope_kind: String,
    pub scope_value: String,
    pub lane: String,
    pub evidence_revision_base: i64,
    pub worker_capability_profile_id: String,
    pub max_retries: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DecomposePayload {
    pub slices: Vec<SliceDraft>,
    #[serde(default)]
    pub decomposition_basis_zh: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AssignPayload {
    pub slice_id: String,
    pub worker_id: String,
    pub reserved_tokens: i64,
    pub reserved_requests: i64,
    pub capability_lease: Vec<String>,
    pub lane: String,
    pub attempt: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AcceptPayload {
    pub assignment_id: String,
    pub slice_id: String,
    pub report_id: String,
    pub verdict: String,
    pub machine_checks: MachineChecks,
    pub model_judgement: Option<ModelJudgement>,
    pub reason_codes: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RejectRetryPayload {
    pub assignment_id: String,
    pub slice_id: String,
    pub report_id: String,
    pub verdict: String,
    pub machine_checks: MachineChecks,
    pub model_judgement: Option<ModelJudgement>,
    pub reason_codes: Vec<String>,
    pub retry_hint_zh: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StopPayload {
    pub stop_reason: String,
    pub accepted_slice_ids: Vec<String>,
    pub rejected_slice_ids: Vec<String>,
    pub summary_zh: String,
    pub publish_partial: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RelayDirectivePayload {
    pub directive_id: String,
    pub mode: String,
    pub rewritten_text: Option<String>,
    pub target_worker_id: Option<String>,
    pub broadcast_constraint: Option<String>,
    pub user_feedback_zh: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RejectDirectivePayload {
    pub directive_id: String,
    pub reason_code: String,
    pub reason_zh: String,
    pub suggestion_zh: String,
    pub user_feedback_zh: String,
}

/// SPEC §1.2 落库形态（含 state/retry_count）。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskSlice {
    pub slice_id: String,
    pub parent_task_id: String,
    pub scope_kind: String,
    pub scope_value: String,
    pub evidence_revision_base: i64,
    pub lane: String,
    pub worker_capability_profile_id: String,
    pub state: String,
    pub max_retries: i64,
    pub retry_count: i64,
    pub decomposition_model: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomogeneousTask {
    pub task_id: String,
    pub root_run_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_key: String,
    pub scheduling_model: String,
    pub state: String,
    pub evidence_revision_base: i64,
    pub allowed_parallel: i64,
    pub degraded: bool,
    pub degrade_reason_code: String,
    pub schema_fail_streak: i64,
    pub hard_reject_streak: i64,
    pub decided_by_lease_epoch: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerInstance {
    pub worker_id: String,
    pub capability_profile_id: String,
    pub assignment_id: String,
    pub state: String,
    pub lease_epoch: i64,
}

/// SPEC §1.5 WorkerReport。
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerReport {
    pub report_id: String,
    pub assignment_id: String,
    pub slice_id: String,
    pub worker_id: String,
    pub evidence_refs: Vec<String>,
    pub output: JsonValue,
    pub consumed_tokens: i64,
    pub consumed_requests: i64,
    pub terminal_hint: String,
}

/// SPEC §1.6 AcceptanceVerdict。
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptanceVerdict {
    pub verdict_id: String,
    pub assignment_id: String,
    pub slice_id: String,
    pub verdict: String,
    pub machine_checks: MachineChecks,
    pub model_judgement: Option<ModelJudgement>,
    pub reason_codes: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub decided_by_lease_epoch: i64,
    /// 承载列，不是 SPEC §1.6 的契约字段：值取自 SPEC §1.4 `Assignment.attempt`，
    /// 用于让同一 assignment 行的多次裁决（retry→retry→accept）都留痕不被覆盖。
    pub attempt: i64,
}

/// SPEC §1.7 HumanDirective（闸门侧）。
#[derive(Clone, Debug, PartialEq)]
pub struct GatedDirective {
    pub directive_id: String,
    pub parent_task_id: String,
    pub source_user_directive_id: String,
    pub raw_text_redacted: String,
    pub state: String,
    pub rule_screen_result: String,
    pub rule_reason_codes: Vec<String>,
    pub coordinator_action: String,
    pub rewritten_text: String,
    pub target_worker_id: String,
    pub broadcast_constraint: String,
    pub reason_code: String,
    pub reason_zh: String,
    pub suggestion_zh: String,
    pub user_feedback_zh: String,
}

impl GatedDirective {
    /// SPEC §2.7 / §6：驳回必须四字段齐备，缺一即整单拒绝。
    pub fn rejection_fields_complete(&self) -> bool {
        !self.reason_code.is_empty()
            && valid_readable_text(&self.reason_zh, MAX_FEEDBACK_ZH_LEN)
            && valid_readable_text(&self.suggestion_zh, MAX_FEEDBACK_ZH_LEN)
            && valid_readable_text(&self.user_feedback_zh, MAX_FEEDBACK_ZH_LEN)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomogeneousAssignment {
    pub assignment_id: String,
    pub coordinator_run_id: String,
    pub parent_task_id: String,
    pub slice_id: String,
    pub worker_id: String,
    pub worker_profile_id: String,
    pub target_key: String,
    pub lane: String,
    pub state: String,
    pub reserved_tokens: i64,
    pub reserved_requests: i64,
    pub capability_lease: Vec<String>,
    pub evidence_revision_base: i64,
    pub attempt: i64,
    pub dedup_key: String,
    pub trigger_code: String,
    /// SPEC §1.2 的切片范围快照（scope_kind/scope_value），落 task_slice_json 列。
    pub scope_kind: String,
    pub scope_value: String,
}
