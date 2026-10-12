//! SPEC §1.8 / §2 / DESIGN §9：主管输出的严格解析。
//!
//! 规则：一次一个 JSON 对象、snake_case、`deny_unknown_fields`、枚举成员封闭。
//! 任何缺字段/多字段/类型错/越权字段都返回 SPEC §1.1 内的 reason_code，
//! 由调用方计入 streak 并决定是否走确定性降级（DESIGN §7）。

use super::{reason, types};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionError {
    pub reason_code: &'static str,
    pub detail: String,
}

impl DecisionError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code: code,
            detail: detail.into(),
        }
    }
}

/// 七动作 payload 的解析产物（DESIGN 附录 C 的分发依据）。
#[derive(Clone, Debug, PartialEq)]
pub enum DecisionPayload {
    Decompose(types::DecomposePayload),
    Assign(types::AssignPayload),
    Accept(types::AcceptPayload),
    RejectRetry(types::RejectRetryPayload),
    Stop(types::StopPayload),
    RelayDirective(types::RelayDirectivePayload),
    RejectDirective(types::RejectDirectivePayload),
}

/// SPEC §2.4 非法例 2：action 与 verdict 必须成对。
const ACTION_VERDICT_PAIRS: [(&str, &str); 2] = [("accept", "accept"), ("reject_retry", "retry")];

pub fn parse_decision_envelope(text: &str) -> Result<types::CoordinatorDecision, DecisionError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(DecisionError::new(
            reason::FREE_TEXT_INSTEAD_OF_JSON,
            "输出为空",
        ));
    }
    let fenced = trimmed.contains("```");
    let body = if fenced {
        strip_code_fences(trimmed)
    } else {
        trimmed.to_string()
    };
    let Some((object, consumed)) = first_json_object(&body) else {
        return Err(DecisionError::new(
            if fenced {
                reason::MARKDOWN_WRAPPED_JSON
            } else {
                reason::FREE_TEXT_INSTEAD_OF_JSON
            },
            "输出中没有 JSON 对象",
        ));
    };
    // 剥围栏后仍是严格 schema：围栏残留或第二个对象都算越界输出。
    let tail = strip_code_fences(&body[consumed..]).trim().to_string();
    if !tail.is_empty() {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            if tail.contains('{') {
                "一次只输出一个 JSON 对象"
            } else {
                "JSON 之外不得有解释性文本"
            },
        ));
    }
    let decision: types::CoordinatorDecision = match serde_json::from_str(&object) {
        Ok(parsed) => parsed,
        Err(error) => {
            // 空对象与声明为零意图却缺字段的信封不能再付费空转一轮。
            let raw: JsonValue = serde_json::from_str(&object).unwrap_or(JsonValue::Null);
            if raw.as_object().is_some_and(|object| object.is_empty())
                || raw.get("zero_intent").is_some_and(|zero| {
                    serde_json::from_value::<types::ZeroIntent>(zero.clone()).is_err()
                })
            {
                return Err(DecisionError::new(
                    reason::ZERO_INTENT_ILLEGAL,
                    error.to_string(),
                ));
            }
            return Err(serde_failure(&error, fenced));
        }
    };
    validate_envelope(&decision)?;
    Ok(decision)
}

fn serde_failure(error: &serde_json::Error, fenced: bool) -> DecisionError {
    let message = error.to_string();
    let code = if message.contains("missing field") {
        reason::MISSING_REQUIRED_FIELD
    } else if message.contains("unknown field") {
        reason::EXTRA_FIELD
    } else if message.contains("duplicate field") {
        reason::SCHEMA_INVALID
    } else if fenced {
        reason::MARKDOWN_WRAPPED_JSON
    } else {
        reason::SCHEMA_INVALID
    };
    DecisionError::new(code, format!("信封解析失败：{message}"))
}

fn validate_envelope(decision: &types::CoordinatorDecision) -> Result<(), DecisionError> {
    if !types::valid_id("dec", &decision.decision_id, 16, 32) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "decision_id 不符合 ^dec-[a-f0-9]{16,32}$",
        ));
    }
    if !types::valid_id("tsk", &decision.parent_task_id, 16, 32) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "parent_task_id 不符合 ^tsk-[a-f0-9]{16,32}$",
        ));
    }
    if !types::is_member(&decision.action, &types::ACTIONS) {
        return Err(DecisionError::new(
            reason::UNKNOWN_ACTION,
            format!("action 不在七动作内：{}", decision.action),
        ));
    }
    if let Some(notes) = decision.model_notes_zh.as_ref() {
        if notes.chars().count() > types::MAX_MODEL_NOTES_LEN {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                "model_notes_zh 超过 200 字",
            ));
        }
    }
    if let Some(zero) = &decision.zero_intent {
        if !types::ZERO_INTENT_REASONS.contains(&zero.reason.as_str())
            || zero.evidence_assignment_ids.is_empty()
            || zero
                .evidence_assignment_ids
                .iter()
                .any(|id| !types::valid_id("asg", id, 16, 32))
            || !types::valid_readable_text(&zero.note_zh, 80)
        {
            return Err(DecisionError::new(
                reason::ZERO_INTENT_ILLEGAL,
                "zero_intent 必须含封闭理由、真实 assignment 证据列表和 ≤80 字 note_zh",
            ));
        }
    }
    let refs = &decision.allowlist_refs;
    for list in [
        &refs.slice_ids,
        &refs.worker_ids,
        &refs.tool_ids,
        &refs.target_keys,
        &refs.directive_ids,
        &refs.assignment_ids,
    ] {
        if list.iter().any(|value| value.trim().is_empty()) {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                "allowlist_refs 含空字符串",
            ));
        }
    }
    Ok(())
}

pub fn check_action_allowed(action: &str, allowed_actions: &[String]) -> Result<(), DecisionError> {
    if allowed_actions
        .iter()
        .any(|candidate| candidate.as_str() == action)
    {
        return Ok(());
    }
    Err(DecisionError::new(
        reason::UNKNOWN_ACTION,
        format!("action 不在本轮 allowed_actions 内：{action}"),
    ))
}

/// 按 action 反序列化 payload，并做 SPEC 的字段级校验。
pub fn parse_payload(
    decision: &types::CoordinatorDecision,
) -> Result<DecisionPayload, DecisionError> {
    let payload = &decision.payload;
    let parsed = match decision.action.as_str() {
        "decompose" => {
            DecisionPayload::Decompose(from_payload::<types::DecomposePayload>(payload)?)
        }
        "assign" => DecisionPayload::Assign(from_payload::<types::AssignPayload>(payload)?),
        "accept" => DecisionPayload::Accept(from_payload::<types::AcceptPayload>(payload)?),
        "reject_retry" => {
            DecisionPayload::RejectRetry(from_payload::<types::RejectRetryPayload>(payload)?)
        }
        "stop" => DecisionPayload::Stop(from_payload::<types::StopPayload>(payload)?),
        "relay_directive" => {
            DecisionPayload::RelayDirective(from_payload::<types::RelayDirectivePayload>(payload)?)
        }
        "reject_directive" => DecisionPayload::RejectDirective(from_payload::<
            types::RejectDirectivePayload,
        >(payload)?),
        other => {
            return Err(DecisionError::new(
                reason::UNKNOWN_ACTION,
                format!("未知动作：{other}"),
            ))
        }
    };
    validate_payload_shape(&parsed)?;
    Ok(parsed)
}
fn from_payload<T: DeserializeOwned>(payload: &JsonValue) -> Result<T, DecisionError> {
    serde_json::from_value::<T>(payload.clone()).map_err(|error| {
        let message = error.to_string();
        let code = if message.contains("missing field") {
            reason::MISSING_REQUIRED_FIELD
        } else if message.contains("unknown field") {
            reason::EXTRA_FIELD
        } else {
            reason::SCHEMA_INVALID
        };
        DecisionError::new(code, format!("payload 解析失败：{message}"))
    })
}

fn validate_payload_shape(payload: &DecisionPayload) -> Result<(), DecisionError> {
    match payload {
        DecisionPayload::Decompose(body) => {
            if body.slices.is_empty() || body.slices.len() > types::MAX_SLICES {
                return Err(DecisionError::new(
                    if body.slices.is_empty() {
                        reason::ZERO_INTENT_ILLEGAL
                    } else {
                        reason::SCHEMA_INVALID
                    },
                    "slices 数量必须在 1..=16",
                ));
            }
            for slice in &body.slices {
                validate_slice_draft(slice)?;
            }
        }
        DecisionPayload::Assign(body) => {
            require_id("slc", &body.slice_id)?;
            require_id("wrk", &body.worker_id)?;
            if !types::is_member(&body.lane, &types::LANES) {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    format!("lane 不在封闭集合内：{}", body.lane),
                ));
            }
            if body.reserved_tokens < 0 || body.reserved_requests < 0 {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "reserved_* 不得为负",
                ));
            }
            if body.attempt < 1 {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "attempt 必须 >= 1",
                ));
            }
            if body
                .capability_lease
                .iter()
                .any(|tool| tool.trim().is_empty())
            {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "capability_lease 含空串",
                ));
            }
        }
        DecisionPayload::Accept(body) => {
            require_id("asg", &body.assignment_id)?;
            require_id("slc", &body.slice_id)?;
            require_id("rpt", &body.report_id)?;
            require_pair("accept", &body.verdict)?;
            require_judgement_when_green(body.machine_checks.all_true(), &body.model_judgement)?;
            validate_reason_codes(&body.reason_codes)?;
            validate_evidence_refs(&body.evidence_refs)?;
        }
        DecisionPayload::RejectRetry(body) => {
            require_id("asg", &body.assignment_id)?;
            require_id("slc", &body.slice_id)?;
            require_id("rpt", &body.report_id)?;
            require_pair("reject_retry", &body.verdict)?;
            // SPEC §1.6：任一机器检查为 false 时 model_judgement 必须为 null。
            if !body.machine_checks.all_true() && body.model_judgement.is_some() {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "机器检查未全绿时 model_judgement 必须为 null",
                ));
            }
            // SPEC §2.4 校验 3：机器全绿时必须有非空 model_judgement 且 adequate=false。
            if body.machine_checks.all_true() {
                let judgement = body.model_judgement.as_ref().ok_or_else(|| {
                    DecisionError::new(
                        reason::MISSING_REQUIRED_FIELD,
                        "机器全绿的 reject_retry 必须给出 model_judgement",
                    )
                })?;
                if judgement.adequate {
                    return Err(DecisionError::new(
                        reason::SCHEMA_INVALID,
                        "机器全绿且判定达标时应走 accept",
                    ));
                }
            }
            validate_reason_codes(&body.reason_codes)?;
            if !types::valid_readable_text(&body.retry_hint_zh, types::MAX_FEEDBACK_ZH_LEN) {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    "retry_hint_zh 必须非空且 <= 500 字",
                ));
            }
            validate_evidence_refs(&body.evidence_refs)?;
        }
        DecisionPayload::Stop(body) => {
            if !types::is_member(&body.stop_reason, &types::STOP_REASONS) {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    format!("stop_reason 不在封闭集合内：{}", body.stop_reason),
                ));
            }
            for slice_id in body
                .accepted_slice_ids
                .iter()
                .chain(body.rejected_slice_ids.iter())
            {
                require_id("slc", slice_id)?;
            }
            if body
                .accepted_slice_ids
                .iter()
                .any(|id| body.rejected_slice_ids.iter().any(|other| other == id))
            {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "accepted 与 rejected 不得重叠",
                ));
            }
            if !types::valid_readable_text(&body.summary_zh, types::MAX_FEEDBACK_ZH_LEN) {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    "summary_zh 必须非空且 <= 500 字",
                ));
            }
        }
        DecisionPayload::RelayDirective(body) => {
            require_id("dir", &body.directive_id)?;
            if !types::is_member(&body.mode, &types::RELAY_MODES) {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    format!("mode 不在封闭集合内：{}", body.mode),
                ));
            }
            if body.target_worker_id.is_some() && body.broadcast_constraint.is_some() {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "target_worker_id 与 broadcast_constraint 互斥",
                ));
            }
            if body.mode == "rewrite" && body.rewritten_text.as_deref().unwrap_or("").is_empty() {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    "mode=rewrite 时 rewritten_text 必填",
                ));
            }
            if !types::valid_readable_text(&body.user_feedback_zh, types::MAX_RAW_TEXT_LEN) {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    "user_feedback_zh 必须非空",
                ));
            }
        }
        DecisionPayload::RejectDirective(body) => {
            require_id("dir", &body.directive_id)?;
            // SPEC §6：驳回四字段缺一即整单拒绝。
            if body.reason_code.is_empty() {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    "reason_code 必填",
                ));
            }
            if !reason::is_valid(&body.reason_code) {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    format!("reason_code 不在封闭集合内：{}", body.reason_code),
                ));
            }
            if !reason::DIRECTIVE_RELATED.contains(&body.reason_code.as_str()) {
                return Err(DecisionError::new(
                    reason::SCHEMA_INVALID,
                    "reject_directive 只能用引导族 reason_code",
                ));
            }
            for (field, value) in [
                ("reason_zh", &body.reason_zh),
                ("suggestion_zh", &body.suggestion_zh),
                ("user_feedback_zh", &body.user_feedback_zh),
            ] {
                if !types::valid_readable_text(value, types::MAX_FEEDBACK_ZH_LEN) {
                    return Err(DecisionError::new(
                        reason::MISSING_REQUIRED_FIELD,
                        format!("{field} 必须非空、无控制字符且 <= 500 字"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_slice_draft(slice: &types::SliceDraft) -> Result<(), DecisionError> {
    require_id("slc", &slice.slice_id)?;
    if !types::is_member(&slice.scope_kind, &types::SCOPE_KINDS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("scope_kind 不在封闭集合内：{}", slice.scope_kind),
        ));
    }
    if slice.scope_value.trim().is_empty()
        || slice.scope_value.chars().count() > types::MAX_SCOPE_VALUE_LEN
        || slice.scope_value.chars().any(|ch| ch.is_ascii_control())
    {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "scope_value 非法",
        ));
    }
    if !types::is_member(&slice.lane, &types::LANES) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("lane 不在封闭集合内：{}", slice.lane),
        ));
    }
    if slice.worker_capability_profile_id != types::PROFILE_HOMOGENEOUS_V1 {
        return Err(DecisionError::new(
            reason::INVENTED_ROLE,
            format!(
                "worker_capability_profile_id 只能是 homogeneous_v1：{}",
                slice.worker_capability_profile_id
            ),
        ));
    }
    if slice.evidence_revision_base < 0 {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "evidence_revision_base 必须 >= 0",
        ));
    }
    if !(0..=3).contains(&slice.max_retries) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "max_retries 必须在 0..=3",
        ));
    }
    Ok(())
}

fn require_id(prefix: &str, value: &str) -> Result<(), DecisionError> {
    if types::valid_id(prefix, value, 16, 32) {
        return Ok(());
    }
    Err(DecisionError::new(
        reason::SCHEMA_INVALID,
        format!("{value} 不符合 ^{prefix}-[a-f0-9]{{16,32}}$"),
    ))
}

fn require_pair(action: &str, verdict: &str) -> Result<(), DecisionError> {
    if !types::is_member(verdict, &types::VERDICTS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("verdict 不在封闭集合内：{verdict}"),
        ));
    }
    if ACTION_VERDICT_PAIRS
        .iter()
        .any(|(left, right)| *left == action && *right != verdict)
    {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("action={action} 时 verdict 必须是 {verdict} 之外的固定值"),
        ));
    }
    Ok(())
}

fn require_judgement_when_green(
    green: bool,
    judgement: &Option<types::ModelJudgement>,
) -> Result<(), DecisionError> {
    if !green && judgement.is_some() {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "机器检查未全绿时不得给 model_judgement",
        ));
    }
    if green && judgement.is_none() {
        return Err(DecisionError::new(
            reason::MISSING_REQUIRED_FIELD,
            "accept 必须给出 model_judgement",
        ));
    }
    if let Some(body) = judgement {
        if green && !body.adequate {
            return Err(DecisionError::new(
                reason::MODEL_JUDGEMENT_FAIL,
                "机器全绿但模型判定不达标，应走 reject_retry",
            ));
        }
        for (field, value) in [
            ("question_zh", &body.question_zh),
            ("answer_zh", &body.answer_zh),
        ] {
            if value.trim().is_empty() {
                return Err(DecisionError::new(
                    reason::MISSING_REQUIRED_FIELD,
                    format!("model_judgement.{field} 不得为空"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_reason_codes(codes: &[String]) -> Result<(), DecisionError> {
    if codes.is_empty() {
        return Err(DecisionError::new(
            reason::MISSING_REQUIRED_FIELD,
            "reason_codes 不得为空",
        ));
    }
    for code in codes {
        if !reason::is_valid(code) {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                format!("reason_code 不在封闭集合内：{code}"),
            ));
        }
    }
    Ok(())
}

fn validate_evidence_refs(refs: &[String]) -> Result<(), DecisionError> {
    for reference in refs {
        if !types::valid_evidence_ref(reference) {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                format!("evidence_ref 不符合 ^evd-[a-f0-9]{{8,32}}$：{reference}"),
            ));
        }
    }
    Ok(())
}

/// SPEC §1.1：状态词只能取封闭成员。落库前与读回后都用它把守。
pub fn validate_state_word(word: &str, set: &[&str]) -> Result<(), DecisionError> {
    if types::is_member(word, set) {
        return Ok(());
    }
    Err(DecisionError::new(
        reason::INVENTED_STATE,
        format!("状态词不在封闭集合内：{word}"),
    ))
}

pub fn strip_code_fences(text: &str) -> String {
    let without_fences = text.replace("```", " ");
    let trimmed = without_fences.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("json")
        && (lowered[4..].is_empty() || trimmed[4..].starts_with(char::is_whitespace))
    {
        return trimmed[4..].trim().to_string();
    }
    without_fences
}

/// 扫描第一个完整的顶层 JSON 对象，返回其文本与其后的字节偏移。
pub fn first_json_object(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(begin) = start {
                        return Some((text[begin..=index].to_string(), index + 1));
                    }
                }
            }
            _ => {}
        }
    }
    None
}
