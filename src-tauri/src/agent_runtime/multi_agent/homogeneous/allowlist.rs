//! DESIGN §3.1 / SPEC §0.6：候选只由 Rust 给出，模型只能挑 id。
//!
//! 快照是纯数据，DB 装载在 `store.rs`；这里做与数据库无关的引用一致性判定，
//! 便于 GOLD-07/23 一类用例离线断言。

use super::decision::{DecisionError, DecisionPayload};
use super::{reason, types};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AllowlistSnapshot {
    pub slice_ids: Vec<String>,
    pub worker_ids: Vec<String>,
    pub tool_ids: Vec<String>,
    pub target_keys: Vec<String>,
    pub directive_ids: Vec<String>,
    pub assignment_ids: Vec<String>,
    pub remaining_tokens: i64,
    pub remaining_requests: i64,
    pub basis_hash: String,
}

impl AllowlistSnapshot {
    fn candidates(&self, category: &str) -> &[String] {
        match category {
            "slice_ids" => &self.slice_ids,
            "worker_ids" => &self.worker_ids,
            "tool_ids" => &self.tool_ids,
            "target_keys" => &self.target_keys,
            "directive_ids" => &self.directive_ids,
            _ => &self.assignment_ids,
        }
    }

    pub fn basis_prefix(&self) -> &str {
        &self.basis_hash
    }
}

/// refs 里某一类的声明列表（生命周期跟着 refs，不跟着快照）。
fn declared_in<'a>(refs: &'a types::AllowlistRefs, category: &str) -> &'a Vec<String> {
    match category {
        "slice_ids" => &refs.slice_ids,
        "worker_ids" => &refs.worker_ids,
        "tool_ids" => &refs.tool_ids,
        "target_keys" => &refs.target_keys,
        "directive_ids" => &refs.directive_ids,
        _ => &refs.assignment_ids,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclaredIds {
    pub slice_ids: Vec<String>,
    pub worker_ids: Vec<String>,
    pub tool_ids: Vec<String>,
    pub target_keys: Vec<String>,
    pub directive_ids: Vec<String>,
    pub assignment_ids: Vec<String>,
}

/// 从已解析的 payload 里收集模型**引用**的既有实体 id。
///
/// `decompose` 的切片是新建对象而不是引用（SPEC §2.1 的合法示例里
/// `allowlist_refs.slice_ids` 就是空数组），因此不参与 refs 一致性检查；
/// 其 id 的合法性与确定性由 `apply::apply_decompose` 校验。
pub fn collect_payload_ids(payload: &DecisionPayload) -> DeclaredIds {
    let mut ids = DeclaredIds::default();
    match payload {
        DecisionPayload::Decompose(_) => {}
        DecisionPayload::Assign(body) => {
            ids.slice_ids = vec![body.slice_id.clone()];
            ids.worker_ids = vec![body.worker_id.clone()];
            ids.tool_ids = body.capability_lease.clone();
        }
        DecisionPayload::Accept(body) => {
            ids.slice_ids = vec![body.slice_id.clone()];
            ids.assignment_ids = vec![body.assignment_id.clone()];
        }
        DecisionPayload::RejectRetry(body) => {
            ids.slice_ids = vec![body.slice_id.clone()];
            ids.assignment_ids = vec![body.assignment_id.clone()];
        }
        DecisionPayload::Stop(body) => {
            ids.slice_ids = body
                .accepted_slice_ids
                .iter()
                .chain(body.rejected_slice_ids.iter())
                .cloned()
                .collect();
        }
        DecisionPayload::RelayDirective(body) => {
            ids.directive_ids = vec![body.directive_id.clone()];
            if let Some(worker) = body.target_worker_id.as_ref() {
                ids.worker_ids = vec![worker.clone()];
            }
        }
        DecisionPayload::RejectDirective(body) => {
            ids.directive_ids = vec![body.directive_id.clone()];
        }
    }
    ids
}

pub fn collect_decision_ids(
    payload: &DecisionPayload,
    decision: &types::CoordinatorDecision,
) -> DeclaredIds {
    let mut ids = collect_payload_ids(payload);
    if let Some(zero) = &decision.zero_intent {
        for assignment_id in &zero.evidence_assignment_ids {
            if !ids.assignment_ids.contains(assignment_id) {
                ids.assignment_ids.push(assignment_id.clone());
            }
        }
    }
    ids
}

fn categories(refs: &types::AllowlistRefs) -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("slice_ids", refs.slice_ids.clone()),
        ("worker_ids", refs.worker_ids.clone()),
        ("tool_ids", refs.tool_ids.clone()),
        ("target_keys", refs.target_keys.clone()),
        ("directive_ids", refs.directive_ids.clone()),
        ("assignment_ids", refs.assignment_ids.clone()),
    ]
}

fn payload_categories(declared: &DeclaredIds) -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("slice_ids", declared.slice_ids.clone()),
        ("worker_ids", declared.worker_ids.clone()),
        ("tool_ids", declared.tool_ids.clone()),
        ("target_keys", declared.target_keys.clone()),
        ("directive_ids", declared.directive_ids.clone()),
        ("assignment_ids", declared.assignment_ids.clone()),
    ]
}

/// SPEC §0.6 + §2.2 校验 4/5 + GOLD-07/23：
/// payload 引用的 id 必须被 allowlist_refs 声明，refs 必须是本轮候选的子集，
/// 预算不得超过 remaining_*。
pub fn verify(
    snapshot: &AllowlistSnapshot,
    refs: &types::AllowlistRefs,
    declared: &DeclaredIds,
) -> Result<(), DecisionError> {
    for (category, referenced) in payload_categories(declared) {
        let declared_list = declared_in(refs, category).clone();
        for value in referenced {
            if !declared_list.contains(&value) {
                return Err(DecisionError::new(
                    reason::ALLOWLIST_VIOLATION,
                    format!("payload 引用了未在 allowlist_refs.{category} 声明的 id：{value}"),
                ));
            }
        }
    }
    for (category, listed) in categories(refs) {
        let candidates = snapshot.candidates(category);
        for value in listed {
            if !candidates.contains(&value) {
                let code = if category == "target_keys" {
                    reason::URL_INVENTED
                } else if category == "tool_ids" {
                    // 候选外的工具串就是「未授权工具」（SPEC GOLD-05）；
                    // invented_tool 留给 Worker 输出里凭空出现的工具键（SPEC §1.5 非法例 3）。
                    reason::TOOL_NOT_AUTHORIZED
                } else {
                    reason::ALLOWLIST_VIOLATION
                };
                return Err(DecisionError::new(
                    code,
                    format!("{category} 不在 Rust 候选集内：{value}"),
                ));
            }
        }
    }
    Ok(())
}

/// SPEC §2.2 校验 4：capability_lease 必须是本 lane/profile 候选工具的子集。
pub fn verify_capability_lease(
    snapshot: &AllowlistSnapshot,
    lease: &[String],
) -> Result<(), DecisionError> {
    for tool in lease {
        if !snapshot.tool_ids.iter().any(|candidate| candidate == tool) {
            return Err(DecisionError::new(
                reason::TOOL_NOT_AUTHORIZED,
                format!("工具不在本切片候选内：{tool}"),
            ));
        }
    }
    Ok(())
}

/// SPEC §2.2 校验 5：预留不得超过主管剩余，且必须为正数之一（DESIGN §4 未放宽）。
pub fn verify_reserved_budget(
    snapshot: &AllowlistSnapshot,
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), DecisionError> {
    if reserved_tokens > snapshot.remaining_tokens
        || reserved_requests > snapshot.remaining_requests
    {
        return Err(DecisionError::new(
            reason::BUDGET_EXCEEDED,
            format!(
                "预留 tokens={reserved_tokens}/requests={reserved_requests} 超过剩余 {}/{}",
                snapshot.remaining_tokens, snapshot.remaining_requests
            ),
        ));
    }
    if reserved_tokens < 0 || reserved_requests < 0 {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "reserved_* 不得为负",
        ));
    }
    if reserved_tokens == 0 && reserved_requests == 0 {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "reserved_tokens 与 reserved_requests 不得同时为 0",
        ));
    }
    Ok(())
}
