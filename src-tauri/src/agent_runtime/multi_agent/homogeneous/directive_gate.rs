//! SPEC §6 / DESIGN §8：聊天引导闸门 G0–G6。
//!
//! G1 脱敏与 G2 规则初筛复用现状 `directive::interpretation::interpret()`（DESIGN §5.6），
//! 本模块只做「规则字符串 → SPEC 封闭值」的映射、二次校验与四字段驳回。

use super::decision::DecisionError;
use super::{reason, store, types};
use rusqlite::Connection;

/// DESIGN §8：`interpret()` 的 status → SPEC §1.7 的 rule_screen_result。
pub fn screen_result_for_status(status: &str) -> Result<&'static str, DecisionError> {
    match status {
        "drafted" => Ok("pass"),
        "need_confirmation" => Ok("need_confirm"),
        "rejected" => Ok("reject"),
        other => Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("未知规则初筛状态：{other}"),
        )),
    }
}

/// 现状 `interpret()` 产出的英文 reason → SPEC 封闭的引导族 reason_code。
pub fn reason_code_for_rule(code: &str) -> &'static str {
    match code {
        "root_budget_change_requires_authorization_flow" => reason::DIRECTIVE_BUDGET_CHANGE,
        "identity_change_requires_dedicated_capture" => reason::DIRECTIVE_IDENTITY_CHANGE,
        "scope_change_requires_authorization_flow" => reason::SCOPE_ESCAPE,
        "mandatory_guardrail_bypass_requested"
        | "cleanup_compensation_contract_missing"
        | "controlled_write_requires_explicit_confirmation" => reason::DIRECTIVE_CLEANUP_FORBIDDEN,
        // secret_material_detected / host_boundary_not_supported 及其它越权族
        _ => reason::DIRECTIVE_PRIVILEGE_ESCALATION,
    }
}

pub struct RejectionTemplate {
    pub reason_code: &'static str,
    pub reason_zh: &'static str,
    pub suggestion_zh: &'static str,
    pub user_feedback_zh: &'static str,
}

/// DESIGN §8.3：规则层直接 reject 时的确定性四字段模板（不经模型、不产生费用）。
pub fn rejection_template(codes: &[String]) -> RejectionTemplate {
    let primary = codes
        .iter()
        .map(|code| reason_code_for_rule(code))
        .find(|code| *code != reason::DIRECTIVE_PRIVILEGE_ESCALATION)
        .unwrap_or(reason::DIRECTIVE_PRIVILEGE_ESCALATION);
    match primary {
        reason::DIRECTIVE_BUDGET_CHANGE => RejectionTemplate {
            reason_code: reason::DIRECTIVE_BUDGET_CHANGE,
            reason_zh: "引导要求变更预算，主管不会执行预算变更。",
            suggestion_zh: "请在现有预算内描述希望优先关注的路径或现象。",
            user_feedback_zh: "未采纳：预算不能通过聊天修改。可以改成「优先看登录接口的错误码」。",
        },
        reason::DIRECTIVE_IDENTITY_CHANGE => RejectionTemplate {
            reason_code: reason::DIRECTIVE_IDENTITY_CHANGE,
            reason_zh: "身份更换需要专用采集流程，不能在聊天引导里直接切换。",
            suggestion_zh: "请指定在当前身份下要核对的接口或现象。",
            user_feedback_zh: "未采纳：不能在聊天中更换身份。",
        },
        reason::SCOPE_ESCAPE => RejectionTemplate {
            reason_code: reason::SCOPE_ESCAPE,
            reason_zh: "引导要求扩大范围，当前冻结范围外的目标不会被访问。",
            suggestion_zh: "请只描述当前已授权主机与路径内的关注点。",
            user_feedback_zh: "未采纳：不能扩域。",
        },
        reason::DIRECTIVE_CLEANUP_FORBIDDEN => RejectionTemplate {
            reason_code: reason::DIRECTIVE_CLEANUP_FORBIDDEN,
            reason_zh: "清理、回滚或绕过 reviewer 不经聊天引导触发。",
            suggestion_zh: "请提出一个只读的验证请求。",
            user_feedback_zh: "未采纳：该引导触及受控动作或审查护栏。",
        },
        _ => RejectionTemplate {
            reason_code: reason::DIRECTIVE_PRIVILEGE_ESCALATION,
            reason_zh: "引导要求越权访问未授权主机或提权，主管不会执行。",
            suggestion_zh:
                "请只描述当前目标站内、当前预算内的关注点，例如「优先看 /api/v1/users 的鉴权头」。",
            user_feedback_zh: "未采纳：不能越权，也不能修改预算或身份。",
        },
    }
}

/// 从文本里抽出主机字面量（SPEC §2.6 非法例 1：改写夹带新 host）。
pub fn hosts_in_text(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let tail = &rest[index + 3..];
        let host: String = tail
            .chars()
            .take_while(|ch| {
                !ch.is_whitespace() && *ch != '/' && *ch != '"' && *ch != '\'' && *ch != '>'
            })
            .collect();
        let normalized = super::decompose::host_of(&host);
        if !normalized.is_empty() && !hosts.contains(&normalized) {
            hosts.push(normalized);
        }
        rest = &tail[host.len().min(tail.len())..];
    }
    hosts
}

fn allowed_hosts(task: &types::HomogeneousTask) -> Vec<String> {
    vec![super::decompose::host_of(&task.target_key)]
}

/// G2：脱敏 + 规则初筛 + 落 gated 表。规则 reject 直接带四字段回用户。
pub fn screen(
    connection: &Connection,
    task: &types::HomogeneousTask,
    directive_id: &str,
    source_user_directive_id: &str,
    raw_text: &str,
) -> Result<types::GatedDirective, DecisionError> {
    let plaintext = super::material_policy::plaintext(connection, &task.scan_id, task.attempt_number)
        .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
    let (status, _decision, reason_codes, _safe_text) =
        crate::agent_runtime::multi_agent::directive::screen_directive_material(
            raw_text,
            &task.target_key,
            plaintext,
        );
    let screen_result = screen_result_for_status(&status)?;
    let redacted = if plaintext { raw_text.to_string() } else { crate::agent_runtime::secrets::redact_text_with(raw_text, None) };
    if redacted.chars().count() > types::MAX_RAW_TEXT_LEN {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "引导原文脱敏后超过 2000 字",
        ));
    }
    let (state, action, code, reason_zh, suggestion_zh, feedback_zh) = if screen_result == "reject"
    {
        let template = rejection_template(&reason_codes);
        (
            "rejected",
            "reject",
            template.reason_code,
            template.reason_zh,
            template.suggestion_zh,
            template.user_feedback_zh,
        )
    } else {
        ("awaiting_coordinator", "", "", "", "", "")
    };
    let directive = types::GatedDirective {
        directive_id: directive_id.to_string(),
        parent_task_id: task.task_id.clone(),
        source_user_directive_id: source_user_directive_id.to_string(),
        raw_text_redacted: redacted,
        state: state.to_string(),
        rule_screen_result: screen_result.to_string(),
        rule_reason_codes: reason_codes,
        coordinator_action: action.to_string(),
        rewritten_text: String::new(),
        target_worker_id: String::new(),
        broadcast_constraint: String::new(),
        reason_code: code.to_string(),
        reason_zh: reason_zh.to_string(),
        suggestion_zh: suggestion_zh.to_string(),
        user_feedback_zh: feedback_zh.to_string(),
    };
    store::upsert_gated_directive(connection, &directive)?;
    Ok(directive)
}

/// G4：改写文本的二次校验（DESIGN §8.4）。
pub fn validate_rewrite(task: &types::HomogeneousTask, text: &str) -> Result<(), DecisionError> {
    validate_rewrite_material(task, text, false)
}

pub fn validate_rewrite_material(task: &types::HomogeneousTask, text: &str, plaintext: bool) -> Result<(), DecisionError> {
    if text.chars().count() > types::MAX_RAW_TEXT_LEN {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "rewritten_text 超过 2000 字",
        ));
    }
    let (status, _decision, reason_codes, _safe_text) =
        crate::agent_runtime::multi_agent::directive::screen_directive_material(text, &task.target_key, plaintext);
    if status == "rejected" {
        let code = reason_codes
            .first()
            .map(|value| reason_code_for_rule(value))
            .unwrap_or(reason::DIRECTIVE_PRIVILEGE_ESCALATION);
        return Err(DecisionError::new(
            code,
            format!("改写文本触发规则初筛拒绝：{reason_codes:?}"),
        ));
    }
    for host in hosts_in_text(text) {
        if !allowed_hosts(task).contains(&host) {
            return Err(DecisionError::new(
                reason::URL_INVENTED,
                format!("改写文本引入未授权主机：{host}"),
            ));
        }
    }
    Ok(())
}

/// G5：relay/rewrite 的落库（state=relayed|rewritten）。
pub fn mark_relayed(
    connection: &Connection,
    directive_id: &str,
    mode: &str,
    rewritten_text: &str,
    target_worker_id: &str,
    broadcast_constraint: &str,
    user_feedback_zh: &str,
) -> Result<types::GatedDirective, DecisionError> {
    let mut directive = store::load_gated_directive(connection, directive_id)?
        .ok_or_else(|| DecisionError::new(reason::ALLOWLIST_VIOLATION, "引导不可寻"))?;
    directive.state = if mode == "rewrite" {
        "rewritten"
    } else {
        "relayed"
    }
    .to_string();
    directive.coordinator_action = mode.to_string();
    directive.rewritten_text = rewritten_text.to_string();
    directive.target_worker_id = target_worker_id.to_string();
    directive.broadcast_constraint = broadcast_constraint.to_string();
    directive.user_feedback_zh = user_feedback_zh.to_string();
    store::upsert_gated_directive(connection, &directive)?;
    Ok(directive)
}

/// G6：reject 的落库。SPEC §2.7 / DESIGN §8：任务状态与 Worker 一律不变。
pub fn mark_rejected(
    connection: &Connection,
    directive_id: &str,
    reason_code: &str,
    reason_zh: &str,
    suggestion_zh: &str,
    user_feedback_zh: &str,
) -> Result<types::GatedDirective, DecisionError> {
    let mut directive = store::load_gated_directive(connection, directive_id)?
        .ok_or_else(|| DecisionError::new(reason::ALLOWLIST_VIOLATION, "引导不可寻"))?;
    directive.state = "rejected".to_string();
    directive.coordinator_action = "reject".to_string();
    directive.reason_code = reason_code.to_string();
    directive.reason_zh = reason_zh.to_string();
    directive.suggestion_zh = suggestion_zh.to_string();
    directive.user_feedback_zh = user_feedback_zh.to_string();
    if !directive.rejection_fields_complete() {
        return Err(DecisionError::new(
            reason::MISSING_REQUIRED_FIELD,
            "驳回四字段缺一，整单拒绝主管输出",
        ));
    }
    store::upsert_gated_directive(connection, &directive)?;
    Ok(directive)
}

/// worker ACK 后 relayed/rewritten → applied（SPEC §3.5）。
pub fn mark_applied(connection: &Connection, directive_id: &str) -> Result<(), DecisionError> {
    let mut directive = store::load_gated_directive(connection, directive_id)?
        .ok_or_else(|| DecisionError::new(reason::ALLOWLIST_VIOLATION, "引导不可寻"))?;
    if directive.state != "relayed" && directive.state != "rewritten" {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "只有 relayed/rewritten 可以 applied",
        ));
    }
    directive.state = "applied".to_string();
    store::upsert_gated_directive(connection, &directive)
}

/// 供 web_executor_adapter 判断「本轮引导已被主管驳回并已回用户」，据此不再打 incomplete。
pub fn rejected_feedback_ready(connection: &Connection, directive_id: &str) -> bool {
    store::load_gated_directive(connection, directive_id)
        .ok()
        .flatten()
        .map(|directive| {
            directive.state == "rejected"
                && directive.coordinator_action == "reject"
                && directive.rejection_fields_complete()
        })
        .unwrap_or(false)
}
