use super::{
    contains_any, controlled_action_guard, extract_fact_ids, proposals, push_unique, queue_actions,
    DraftInterpretation,
};
use crate::agent_runtime::secrets::redact_text_with;

fn explicit_url_scope_change(text: &str, target_key: &str) -> bool {
    let target = reqwest::Url::parse(target_key).ok();
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative) = ["http://", "https://"]
        .into_iter()
        .filter_map(|scheme| lower[search_from..].find(scheme))
        .min()
    {
        let start = search_from + relative;
        let end = text[start..]
            .char_indices()
            .find(|(offset, character)| {
                *offset > 0
                    && (character.is_whitespace() || "<>'\"()[]{}，。；、,;".contains(*character))
            })
            .map(|(offset, _)| start + offset)
            .unwrap_or(text.len());
        // A second URL inside a path/query is still a separate requested
        // destination (for example a redirect parameter), not proof that the
        // first URL's origin is authorized.
        let scheme_length = if lower[start..].starts_with("https://") {
            8
        } else {
            7
        };
        if ["http://", "https://"]
            .into_iter()
            .any(|scheme| lower[start + scheme_length..end].contains(scheme))
        {
            return true;
        }
        let requested = reqwest::Url::parse(text[start..end].trim_end_matches(['.', '!']));
        let same_origin = requested
            .ok()
            .zip(target.as_ref())
            .is_some_and(|(url, frozen)| {
                url.scheme() == frozen.scheme()
                    && url.host() == frozen.host()
                    && url.port_or_known_default() == frozen.port_or_known_default()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
        if !same_origin {
            return true;
        }
        search_from = end;
    }
    false
}

/// This is an early chat guard, not the Web Broker's authorization boundary.
/// A host request cannot be converted into a Web assignment or authorized by
/// confirming a chat draft; the product has no host scope or executor yet.
pub(super) fn explicit_host_boundary_request(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "ssh ",
            "ssh连接",
            "ssh 登录",
            "ssh登录",
            "远程 shell",
            "远程shell",
            "reverse shell",
            "remote shell",
            "host shell",
            "host scan",
            "主机验证",
            "主机扫描",
            "主机探测",
            "横向移动",
            "横向访问",
            "提权",
            "权限提升",
            "执行系统命令",
            "执行操作系统命令",
            "执行os命令",
            "远程执行命令",
            "远程命令",
            "主机文件",
            "主机进程",
            "主机账户",
            "/etc/passwd",
            "privilege escalation",
            "lateral movement",
        ],
    )
}

pub(super) fn interpret(text: &str, target_key: &str) -> DraftInterpretation {
    interpret_with_material(text, target_key, false)
}

pub(super) fn interpret_with_material(text: &str, target_key: &str, plaintext: bool) -> DraftInterpretation {
    let lower = text.to_ascii_lowercase();
    let redacted = if plaintext { text.to_string() } else { redact_text_with(text, None) };
    let secret = !plaintext && redacted != text;
    let host_boundary = explicit_host_boundary_request(text);
    let bypass = controlled_action_guard::requests_cleanup_bypass(&lower)
        || contains_any(
            &lower,
            &[
                "绕过 reviewer",
                "跳过 reviewer",
                "不要 reviewer",
                "不需要 reviewer",
                "绕过审查",
                "跳过审查",
                "忽略并发",
                "取消并发限制",
                "bypass reviewer",
                "skip reviewer",
                "ignore concurrency",
            ],
        );
    let scope_change = contains_any(
        &lower,
        &[
            "扩大范围",
            "扩展范围",
            "新增域名",
            "其他域名",
            "另一个域名",
            "widen scope",
            "expand scope",
            "another domain",
            "new domain",
        ],
    ) || explicit_url_scope_change(text, target_key);
    let budget_change = contains_any(
        &lower,
        &[
            "增加预算",
            "提高预算",
            "追加预算",
            "更多预算",
            "increase budget",
            "raise budget",
            "more budget",
        ],
    );
    let pause_or_disable = contains_any(
        &lower,
        &[
            "暂停",
            "停止",
            "不要再",
            "先不要",
            "禁用",
            "pause",
            "stop",
            "disable",
            "do not",
            "don't",
        ],
    );
    let controlled_write = controlled_action_guard::contains_active_write(&lower);
    let cleanup_contract = contains_any(
        &lower,
        &[
            "清理",
            "恢复",
            "回滚",
            "补偿",
            "cleanup",
            "restore",
            "rollback",
            "compensation",
        ],
    );
    let identity_change = contains_any(
        &lower,
        &[
            "新身份",
            "新账号",
            "刷新身份",
            "刷新会话",
            "重新登录",
            "new identity",
            "new account",
            "refresh identity",
            "refresh session",
        ],
    );

    let mut roles = Vec::new();
    for (role, aliases) in [
        ("coordinator", &["@coordinator", "@协调", "@总控"][..]),
        ("spa_api_mapper", &["@mapper", "@采集", "@映射"][..]),
        (
            "deep_investigator",
            &["@investigator", "@调查", "@深度调查"][..],
        ),
        ("evidence_reviewer", &["@reviewer", "@检查", "@审查"][..]),
        ("repo_mapper", &["@repo_mapper", "@仓库梳理"][..]),
        ("source_analyst", &["@source_analyst", "@源码分析"][..]),
    ] {
        if aliases.iter().any(|alias| lower.contains(alias)) {
            push_unique(&mut roles, role);
        }
    }
    if roles.is_empty() {
        roles.push("coordinator".into());
    }

    let mut contracts = Vec::new();
    if cleanup_contract {
        contracts.push("cleanup_compensation".into());
    }
    if identity_change {
        contracts.push("identity_capture".into());
    }
    if controlled_write {
        contracts.push("controlled_write".into());
    }

    let mut approvals = Vec::new();
    let mut reasons = Vec::new();
    let mut priority_changes = Vec::new();
    let mut intent = if roles.iter().any(|role| role != "coordinator") {
        "agent_proposal_request"
    } else {
        "priority_adjustment"
    };
    let mut validation = "valid";
    let mut decision = "accept";
    let mut status = "drafted";
    let mut side_effect = "read_only";
    let mut safe_execution_text = redacted.clone();
    let proposed_scope_change = scope_change.then(|| "authorization_scope_change_requested".into());

    if pause_or_disable {
        priority_changes.push("pause_or_deprioritize_requested_work".into());
    } else {
        priority_changes.push("evaluate_requested_priority_change".into());
    }

    if secret {
        intent = "credential_material_rejected";
        validation = "rejected";
        decision = "reject";
        status = "rejected";
        reasons.push("secret_material_detected".into());
        safe_execution_text.clear();
    } else if bypass {
        intent = "guardrail_bypass_rejected";
        validation = "rejected";
        decision = "reject";
        status = "rejected";
        reasons.push("mandatory_guardrail_bypass_requested".into());
        safe_execution_text.clear();
    } else if host_boundary {
        intent = "host_boundary_rejected";
        validation = "rejected";
        decision = "reject";
        status = "rejected";
        reasons.push("host_boundary_not_supported".into());
        safe_execution_text.clear();
    } else if controlled_write && !cleanup_contract {
        intent = "controlled_action_proposal";
        validation = "rejected";
        decision = "reject";
        status = "rejected";
        side_effect = "controlled_write";
        approvals.push("human_confirmation".into());
        approvals.push("cleanup_compensation_contract".into());
        reasons.push("cleanup_compensation_contract_missing".into());
        safe_execution_text.clear();
    } else if scope_change || budget_change {
        intent = "authorization_change_proposal";
        validation = "confirmation_required";
        decision = "need_confirmation";
        status = "need_confirmation";
        approvals.push("human_confirmation".into());
        if scope_change {
            approvals.push("scope_authorization_update".into());
            reasons.push("scope_change_requires_authorization_flow".into());
        }
        if budget_change {
            approvals.push("root_budget_update".into());
            reasons.push("root_budget_change_requires_authorization_flow".into());
        }
        safe_execution_text =
            format!("仅评估并生成授权变更提案，不得扩大当前冻结范围或增加根预算：{redacted}");
    } else if controlled_write || identity_change {
        intent = if identity_change {
            "identity_change_proposal"
        } else {
            "controlled_action_proposal"
        };
        validation = "confirmation_required";
        decision = "need_confirmation";
        status = "need_confirmation";
        approvals.push("human_confirmation".into());
        if controlled_write {
            side_effect = "controlled_write";
            approvals.push("cleanup_compensation_contract".into());
            reasons.push("controlled_write_requires_explicit_confirmation".into());
        }
        if identity_change {
            approvals.push("dedicated_identity_capture".into());
            reasons.push("identity_change_requires_dedicated_capture".into());
        }
        safe_execution_text = format!(
            "仅在当前冻结范围、现有 capability lease 和已批准清理合同内评估该请求；不得从聊天内容获取凭证或扩权：{redacted}"
        );
    } else {
        approvals.push("directive_confirmation".into());
        reasons.push("read_only_change_within_frozen_plan".into());
        if intent == "agent_proposal_request" {
            safe_execution_text = format!(
                "仅请求指定角色评估并提出提案；Coordinator 决定是否派发，且不得扩权：{redacted}"
            );
        }
    }

    // Only a complete, unambiguous command becomes a frozen scheduling action.
    // Other prose remains a model request, never an implicit queue mutation.
    let priority_family =
        if intent == "priority_adjustment" && status == "drafted" && !pause_or_disable {
            queue_actions::explicit_priority_family(&redacted)
        } else {
            None
        };
    if let Some(family) = priority_family {
        priority_changes = vec![format!("prioritize_family:{family}")];
        reasons.push("queue_priority_action_requires_confirmation".into());
    }

    DraftInterpretation {
        intent: intent.into(),
        requested_roles: roles,
        referenced_fact_ids: extract_fact_ids(&redacted),
        requested_contracts: contracts,
        priority_changes,
        proposed_scope_change,
        estimated_tokens: if intent == "agent_proposal_request" {
            proposals::PROPOSAL_TOKENS
        } else if priority_family.is_some() {
            0
        } else {
            (128 + redacted.chars().count() as i64 / 4).clamp(128, 800)
        },
        estimated_requests: if priority_family.is_some() { 0 } else { 1 },
        side_effect_class: side_effect.into(),
        required_approvals: approvals,
        validation_result: validation.into(),
        reason_codes: reasons,
        coordinator_decision: decision.into(),
        confirmation_required: status != "rejected",
        safe_execution_text,
        status: status.into(),
    }
}
