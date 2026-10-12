//! DESIGN §3.1 / 附录 C：唯一的副作用入口。
//!
//! 一切状态迁移只能在这里发生；解析失败或硬拒在这里计入 streak，
//! 达到 DESIGN §7.3 的阈值后由 `run_coordinator_turn` 切到确定性降级。

use super::allowlist::{self, AllowlistSnapshot};
use super::decision::{self, DecisionError, DecisionPayload};
use super::decompose;
use super::decompose::{scopes_overlap, slice_id_for, DecomposeOutcome, Scope};
use super::{directive_gate, gate, intent, machine, pool, reason, store, types};
use crate::agent_runtime::multi_agent::lease::{
    require_executable_coordinator, validate_coordinator_lease, CoordinatorLease,
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug)]
pub enum TurnOutcome {
    /// 合法决策已落库。
    Applied(String),
    /// 决策被拒（streak 已更新）。
    Refused {
        reason_code: &'static str,
        detail: String,
        schema_fail_streak: i64,
        hard_reject_streak: i64,
    },
    /// 连续失败后走确定性降级（DESIGN §7），不再付费问模型。
    Degraded(DecomposeOutcome),
    /// 任务已终态或无活可干：不调用模型，也不降级。
    Idle(String),
}

impl TurnOutcome {
    pub fn applied(&self) -> bool {
        matches!(self, TurnOutcome::Applied(_))
    }

    pub fn refused_reason(&self) -> Option<&'static str> {
        match self {
            TurnOutcome::Refused { reason_code, .. } => Some(reason_code),
            _ => None,
        }
    }
}

fn task_of(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<types::HomogeneousTask, DecisionError> {
    store::load_task_by_root_run(connection, &lease.root_run_id)?.ok_or_else(|| {
        DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            "该 root_run 没有同质任务（scheduling_model 未显式设为 homogeneous_workers）",
        )
    })
}

/// §J 非法 0 意图立即拒绝原单；空队列在同一次付费轮内补一条确定性 pending。
/// 已有活时只拒绝，不把正在运行的方向重写或清空。
fn refuse_or_replenish(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    error: DecisionError,
) -> Result<TurnOutcome, DecisionError> {
    let streak = store::bump_hard_reject_streak(connection, &task.task_id)
        .map_err(|text| DecisionError::new(reason::SCHEMA_INVALID, text))?;
    if !intent::snapshot(connection, task)?.is_empty() {
        if streak >= gate::STREAK_LIMIT {
            if let Some(outcome) = dispatch_degraded_pending(connection, lease, task)? {
                return Ok(outcome);
            }
        }
        return Ok(TurnOutcome::Refused {
            reason_code: error.reason_code,
            detail: error.detail,
            schema_fail_streak: task.schema_fail_streak,
            hard_reject_streak: streak,
        });
    }
    let slices = store::load_slices(connection, &task.task_id)?;
    let mut candidates = decompose::deterministic_candidates(connection, task)?;
    // 本任务冻结目标始终是合法最后候选；无侦察节点也不能把首轮空转留给下一次付费。
    let host = decompose::host_of(&task.target_key);
    if !host.is_empty() {
        candidates.push(types::SliceDraft {
            slice_id: slice_id_for(&task.task_id, "host", &host, task.evidence_revision_base),
            scope_kind: "host".to_string(),
            scope_value: host,
            lane: "read_only_analysis".to_string(),
            evidence_revision_base: task.evidence_revision_base,
            worker_capability_profile_id: types::PROFILE_HOMOGENEOUS_V1.to_string(),
            max_retries: 2,
        });
    }
    let candidate = candidates.into_iter().find(|draft| {
        !slices.iter().any(|slice| {
            slice.slice_id == draft.slice_id
                || ((slice.state == "accepted"
                    || (slice.state != "cancelled"
                        && !(slice.state == "rejected" && slice.retry_count >= slice.max_retries)))
                    && scopes_overlap(
                        &Scope::new(&slice.scope_kind, &slice.scope_value),
                        &Scope::new(&draft.scope_kind, &draft.scope_value),
                    ))
        })
    });
    let snapshot = gate::snapshot_for(connection, task)?;
    let (payload, raw_payload, action, ids) = if let Some(draft) = candidate {
        let body = types::DecomposePayload {
            slices: vec![draft.clone()],
            decomposition_basis_zh: Some("非法0意图：确定性补一条已授权只读方向".to_string()),
        };
        (
            DecisionPayload::Decompose(body),
            serde_json::json!({}),
            "decompose",
            vec![draft.slice_id],
        )
    } else {
        // 所有可授权方向均已终态，禁止再次付费等待同样的空候选；用既有 stop 正式收尾。
        let goal = !slices.is_empty() && slices.iter().all(|slice| slice.state == "accepted");
        let body = types::StopPayload {
            stop_reason: if goal {
                "goal_reached"
            } else {
                "retry_limit_exceeded"
            }
            .to_string(),
            accepted_slice_ids: slices
                .iter()
                .filter(|slice| slice.state == "accepted")
                .map(|slice| slice.slice_id.clone())
                .collect(),
            rejected_slice_ids: slices
                .iter()
                .filter(|slice| slice.state != "accepted")
                .map(|slice| slice.slice_id.clone())
                .collect(),
            summary_zh: "所有冻结方向已验收或耗尽重试，主管确定性收尾".to_string(),
            publish_partial: slices.iter().any(|slice| slice.state == "accepted") && !goal,
        };
        (
            DecisionPayload::Stop(body),
            serde_json::json!({}),
            "stop",
            Vec::new(),
        )
    };
    let decision = types::CoordinatorDecision {
        decision_id: format!(
            "dec-{}",
            &crate::agent_runtime::store::stable_hash(&format!(
                "{}:zero-intent:{streak}:{}",
                task.task_id, snapshot.basis_hash
            ))[..24]
        ),
        action: action.to_string(),
        parent_task_id: task.task_id.clone(),
        payload: raw_payload,
        allowlist_refs: types::AllowlistRefs {
            slice_ids: Vec::new(),
            worker_ids: Vec::new(),
            tool_ids: Vec::new(),
            target_keys: vec![task.target_key.clone()],
            directive_ids: Vec::new(),
            assignment_ids: Vec::new(),
        },
        model_notes_zh: Some(format!("{}：{}", reason::ZERO_INTENT_ILLEGAL, error.detail)),
        zero_intent: None,
    };
    apply_decision_with_origin(connection, lease, &payload, &decision, &snapshot, true)?;
    store::clear_streaks(connection, &task.task_id)
        .map_err(|text| DecisionError::new(reason::SCHEMA_INVALID, text))?;
    let outcome = TurnOutcome::Degraded(if action == "stop" {
        DecomposeOutcome::Stopped(
            intent::latest_stop_reason(connection, &task.task_id)?.unwrap_or_default(),
        )
    } else {
        DecomposeOutcome::Degraded(ids)
    });
    if error.reason_code == reason::ZERO_INTENT_ILLEGAL {
        Ok(outcome)
    } else {
        // 保留原硬拒 reason；新增切片属于随后独立的确定性降级，原 proposal 仍整单作废。
        let schema = [
            reason::SCHEMA_INVALID,
            reason::MISSING_REQUIRED_FIELD,
            reason::EXTRA_FIELD,
            reason::MARKDOWN_WRAPPED_JSON,
            reason::FREE_TEXT_INSTEAD_OF_JSON,
            reason::UNKNOWN_ACTION,
            reason::INVENTED_ROLE,
        ]
        .contains(&error.reason_code);
        if schema {
            store::bump_schema_fail_streak(connection, &task.task_id)
                .map_err(|text| DecisionError::new(reason::SCHEMA_INVALID, text))?;
        } else {
            store::bump_hard_reject_streak(connection, &task.task_id)
                .map_err(|text| DecisionError::new(reason::SCHEMA_INVALID, text))?;
        }
        Ok(TurnOutcome::Refused {
            reason_code: error.reason_code,
            detail: format!("{}；degraded_path_used：{outcome:?}", error.detail),
            schema_fail_streak: i64::from(schema),
            hard_reject_streak: i64::from(!schema),
        })
    }
}

/// DESIGN §4.4：任务级配额。Phase 1 只给只读 lane 扩量，其余恒 1。
pub fn ensure_quota(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<(), DecisionError> {
    for lane in types::LANES {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_lane_quotas WHERE root_run_id=?1 AND lane=?2)",
                params![task.root_run_id, lane],
                |row| row.get(0),
            )
            .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, error.to_string()))?;
        if exists {
            continue;
        }
        let parallel = if lane == "read_only_analysis" {
            task.allowed_parallel
        } else {
            1
        };
        store::upsert_quota(connection, &task.root_run_id, lane, parallel)?;
    }
    Ok(())
}

/// DESIGN §3.1 的一轮：候选快照 → 严格解析 → refs 校验 → apply → streak 记账。
pub fn run_coordinator_turn(
    connection: &Connection,
    lease: &CoordinatorLease,
    model_text: &str,
) -> Result<TurnOutcome, DecisionError> {
    // 过期主管不得读取并改写账本，更不得付费问模型（DESIGN §12 防双写）。
    validate_coordinator_lease(connection, lease)
        .map_err(|error| DecisionError::new(reason::LEASE_INVALID, error))?;
    let task = task_of(connection, lease)?;
    // 预算账本必须在候选快照之前存在，否则 remaining_* 恒为 0，assign 永远不可选。
    store::ensure_ledger(connection, lease)?;
    // P1-3：先回收已到期而未汇报的 Worker 载体，再计算允许动作与候选（DESIGN §6）。
    pool::reclaim_expired_for_task(connection, &task)?;
    let task = task_of(connection, lease)?;
    let allowed = gate::allowed_actions(connection, &task)?;
    if allowed.is_empty() {
        if types::is_member(
            &task.state,
            &["succeeded", "failed", "cancelled", "stopped"],
        ) {
            return Ok(TurnOutcome::Idle(task.state));
        }
        // DESIGN §7.3：streak 达阈 → 禁止再问模型，直接降级。
        if intent::snapshot(connection, &task)?.is_empty() {
            return refuse_or_replenish(
                connection,
                lease,
                &task,
                intent::illegal("失败达到阈值且队列全空，确定性补意图"),
            );
        }
        if let Some(outcome) = dispatch_degraded_pending(connection, lease, &task)? {
            return Ok(outcome);
        }
        return Ok(TurnOutcome::Idle("degraded_existing_intents".to_string()));
    }
    let snapshot = gate::snapshot_for(connection, &task)?;
    let decision = match decision::parse_decision_envelope(model_text).and_then(|parsed| {
        let payload = decision::parse_payload(&parsed)?;
        if intent::snapshot(connection, &task)?.is_empty()
            && !matches!(
                payload,
                DecisionPayload::Decompose(_) | DecisionPayload::Assign(_)
            )
        {
            match &payload {
                DecisionPayload::Stop(body) => intent::validate_stop(connection, &task, body)?,
                _ => return Err(intent::illegal("空任务本轮必须产出意图，引导可下一 tick")),
            }
        }
        decision::check_action_allowed(&parsed.action, &allowed)?;
        Ok((parsed, payload))
    }) {
        Ok(parsed) => parsed,
        Err(error) => {
            if error.reason_code == reason::ZERO_INTENT_ILLEGAL
                || intent::snapshot(connection, &task)?.is_empty()
            {
                return refuse_or_replenish(connection, lease, &task, error);
            }
            let streak = store::bump_schema_fail_streak(connection, &task.task_id)
                .map_err(|detail| DecisionError::new(reason::SCHEMA_INVALID, detail))?;
            if streak >= gate::STREAK_LIMIT {
                // 已有队列/inflight 时停止付费决策，绝不重分解现有切片或压缩它们的配额。
                if let Some(outcome) = dispatch_degraded_pending(connection, lease, &task)? {
                    return Ok(outcome);
                }
                return Ok(TurnOutcome::Idle("degraded_existing_intents".to_string()));
            }
            return Ok(TurnOutcome::Refused {
                reason_code: error.reason_code,
                detail: error.detail,
                schema_fail_streak: streak,
                hard_reject_streak: store::read_streaks(connection, &task.task_id)
                    .map(|(_, hard)| hard)
                    .unwrap_or(0),
            });
        }
    };
    let (parsed, payload) = decision;
    let declared = allowlist::collect_decision_ids(&payload, &parsed);
    let applied = allowlist::verify(&snapshot, &parsed.allowlist_refs, &declared)
        .and_then(|()| apply_decision(connection, lease, &payload, &parsed, &snapshot));
    match applied {
        Ok(detail) => {
            store::clear_streaks(connection, &task.task_id)
                .map_err(|text| DecisionError::new(reason::SCHEMA_INVALID, text))?;
            Ok(TurnOutcome::Applied(detail))
        }
        Err(error) => {
            if error.reason_code == reason::ZERO_INTENT_ILLEGAL
                || intent::snapshot(connection, &task)?.is_empty()
            {
                return refuse_or_replenish(connection, lease, &task, error);
            }
            let streak = store::bump_hard_reject_streak(connection, &task.task_id)
                .map_err(|detail| DecisionError::new(reason::SCHEMA_INVALID, detail))?;
            if streak >= gate::STREAK_LIMIT {
                if let Some(outcome) = dispatch_degraded_pending(connection, lease, &task)? {
                    return Ok(outcome);
                }
                return Ok(TurnOutcome::Idle("degraded_existing_intents".to_string()));
            }
            Ok(TurnOutcome::Refused {
                reason_code: error.reason_code,
                detail: error.detail,
                schema_fail_streak: store::read_streaks(connection, &task.task_id)
                    .map(|(schema, _)| schema)
                    .unwrap_or(0),
                hard_reject_streak: streak,
            })
        }
    }
}

/// DESIGN §7.4: a pending deterministic slice must actually be dispatched.
/// A queued slice is not a running Worker and cannot consume its own mailbox.
/// Existing inflight work is preserved; only the first pending slice is used.
fn dispatch_degraded_pending(
    db: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
) -> Result<Option<TurnOutcome>, DecisionError> {
    if !super::model_worker::enabled(db, &task.scan_id, task.attempt_number)
        .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))? {
        return Ok(None);
    }
    let before = intent::snapshot(db, task)?;
    if !before.inflight_assignments.is_empty() { return Ok(None); }
    let Some(slice) = store::load_slices(db, &task.task_id)?.into_iter().find(|slice|
        slice.state == "pending" || (slice.state == "rejected" && slice.retry_count < slice.max_retries)
    ) else { return Ok(None); };
    pool::provision_workers(db, task, lease.lease_epoch, task.allowed_parallel)?;
    let Some(worker_id) = pool::worker_ids_in_state(db, &task.root_run_id, "idle")?.into_iter().next() else { return Ok(None); };
    let data = super::model_worker::input(db, task, &slice)
        .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
    let required = super::model_worker::minimum_tokens(&data);
    let snapshot = gate::snapshot_for(db, task)?;
    if snapshot.remaining_tokens < required || snapshot.remaining_requests < 1 { return Ok(None); }
    let attempt: i64 = db.query_row("SELECT COALESCE(MAX(attempt),0)+1 FROM agent_assignments WHERE coordinator_run_id=?1 AND slice_id=?2 AND scheduling_model='homogeneous_workers'", params![task.root_run_id,slice.slice_id], |row| row.get(0))
        .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e.to_string()))?;
    let value = serde_json::json!({
        "decision_id":format!("dec-{}", &crate::agent_runtime::store::stable_hash(&format!("{}:{}:degraded-dispatch:{attempt}",task.task_id,slice.slice_id))[..24]),
        "action":"assign","parent_task_id":task.task_id,
        "payload":{"slice_id":slice.slice_id,"worker_id":worker_id,"reserved_tokens":required,"reserved_requests":1,"capability_lease":["evidence.read"],"lane":"read_only_analysis","attempt":attempt},
        "allowlist_refs":{"slice_ids":[slice.slice_id],"worker_ids":[worker_id],"tool_ids":["evidence.read"],"target_keys":[],"directive_ids":[],"assignment_ids":[]}
    });
    let decision = decision::parse_decision_envelope(&value.to_string())?;
    let payload = decision::parse_payload(&decision)?;
    allowlist::verify(&snapshot, &decision.allowlist_refs, &allowlist::collect_decision_ids(&payload, &decision))?;
    let detail = apply_decision_with_origin(db, lease, &payload, &decision, &snapshot, true)?;
    // This is a successful legal decision (§7.6), not a paid retry of the
    // rejected model response. Later acceptance still needs its own paid tick.
    store::clear_streaks(db, &task.task_id).map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
    Ok(Some(TurnOutcome::Applied(detail)))
}

pub fn apply_decision(
    connection: &Connection,
    lease: &CoordinatorLease,
    payload: &DecisionPayload,
    decision: &types::CoordinatorDecision,
    snapshot: &AllowlistSnapshot,
) -> Result<String, DecisionError> {
    apply_decision_with_origin(connection, lease, payload, decision, snapshot, false)
}

fn apply_decision_with_origin(
    connection: &Connection,
    lease: &CoordinatorLease,
    payload: &DecisionPayload,
    decision: &types::CoordinatorDecision,
    snapshot: &AllowlistSnapshot,
    deterministic: bool,
) -> Result<String, DecisionError> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate).map_err(
            |error| {
                DecisionError::new(reason::SCHEMA_INVALID, format!("无法锁定主管决策：{error}"))
            },
        )?;
    let connection: &Connection = &transaction;
    validate_coordinator_lease(connection, lease)
        .map_err(|error| DecisionError::new(reason::LEASE_INVALID, error))?;
    require_executable_coordinator(connection, lease)
        .map_err(|error| DecisionError::new(reason::LEASE_INVALID, error))?;
    let task = store::load_task(connection, &decision.parent_task_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "任务不可寻"))?;
    if task.root_run_id != lease.root_run_id {
        return Err(DecisionError::new(
            reason::LEASE_INVALID,
            "决策所属任务不属于当前持有租约的主管",
        ));
    }
    if task.scheduling_model != types::SCHEDULING_HOMOGENEOUS {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            "该任务不是 homogeneous_workers 模式",
        ));
    }
    ensure_quota(connection, &task)?;
    let before = intent::snapshot(connection, &task)?;
    super::budget_bridge::admit(&transaction, lease, payload)?;
    let detail = match payload {
        DecisionPayload::Decompose(body) => {
            apply_decompose(connection, lease, &task, body, deterministic)?
        }
        DecisionPayload::Assign(body) => apply_assign(connection, lease, &task, body, snapshot)?,
        DecisionPayload::Accept(body) => apply_accept(connection, lease, &task, body)?,
        DecisionPayload::RejectRetry(body) => apply_reject_retry(connection, lease, &task, body)?,
        DecisionPayload::Stop(body) => apply_stop(connection, lease, &task, body)?,
        DecisionPayload::RelayDirective(body) => {
            apply_relay_directive(connection, lease, &task, body, snapshot)?
        }
        DecisionPayload::RejectDirective(body) => {
            apply_reject_directive(connection, body, snapshot)?
        }
    };
    super::budget_bridge::apply(&transaction, lease, payload)?;
    let produced = intent::produced(connection, &task, &before, payload)?;
    if !produced && !matches!(payload, DecisionPayload::Stop(_)) {
        intent::validate_zero(&before, decision.zero_intent.as_ref())?;
    } else if decision.zero_intent.is_some() {
        return Err(intent::illegal(
            "产出意图或合法 stop 不得同时声明 zero_intent",
        ));
    }
    if deterministic {
        store::set_task_degraded(connection, &task.task_id, reason::DEGRADED_PATH_USED)?;
        store::upsert_quota(connection, &task.root_run_id, "read_only_analysis", 1)?;
    }
    intent::audit(
        connection,
        &task,
        decision,
        payload,
        &before,
        produced,
        lease.lease_epoch,
        if deterministic {
            reason::ZERO_INTENT_ILLEGAL
        } else {
            ""
        },
    )?;
    transaction.commit().map_err(|error| {
        DecisionError::new(reason::SCHEMA_INVALID, format!("无法提交主管决策：{error}"))
    })?;
    Ok(detail)
}

fn current_revision(connection: &Connection, root_run_id: &str) -> Result<i64, DecisionError> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| DecisionError::new(reason::STALE_EVIDENCE_REVISION, error.to_string()))
}

/// SPEC §2.1：一次决策只做分解，切片必须互不重叠且 id 可回放。
fn apply_decompose(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::DecomposePayload,
    deterministic: bool,
) -> Result<String, DecisionError> {
    if !types::is_member(
        &task.state,
        &["created", "decomposed", "running", "partial_success"],
    ) {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            format!("任务状态 {} 不允许再分解", task.state),
        ));
    }
    if body.slices.is_empty() || body.slices.len() > types::MAX_SLICES {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "slices 数量必须在 1..=16",
        ));
    }
    let revision = current_revision(connection, &task.root_run_id)?;
    let existing = store::load_slices(connection, &task.task_id)?;
    for draft in &body.slices {
        if draft.lane!="read_only_analysis" {
            crate::commands::homogeneous_execution::validate_draft(connection,lease,task,draft).map_err(|e|DecisionError::new(reason::TOOL_NOT_AUTHORIZED,e))?;
        }
        if existing
            .iter()
            .any(|slice| slice.slice_id == draft.slice_id)
        {
            return Err(DecisionError::new(
                reason::DUPLICATE_SLICE,
                "追加分解必须新增切片，禁止改写已有身份",
            ));
        }
        if existing.iter().any(|slice| {
            (slice.state != "cancelled"
                && !(slice.state == "rejected" && slice.retry_count >= slice.max_retries))
                && scopes_overlap(
                    &Scope::new(&slice.scope_kind, &slice.scope_value),
                    &Scope::new(&draft.scope_kind, &draft.scope_value),
                )
        }) {
            return Err(DecisionError::new(
                reason::SLICE_OVERLAP,
                "追加切片与既有可推进或已验收切片重叠",
            ));
        }
        let expected = slice_id_for(
            &task.task_id,
            &draft.scope_kind,
            &draft.scope_value,
            draft.evidence_revision_base,
        );
        if expected != draft.slice_id {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                format!("slice_id 必须由 scope 确定性派生：期望 {expected}"),
            ));
        }
        if draft.evidence_revision_base > revision {
            return Err(DecisionError::new(
                reason::STALE_EVIDENCE_REVISION,
                format!(
                    "evidence_revision_base {} 超过当前 {revision}",
                    draft.evidence_revision_base
                ),
            ));
        }
    }
    for index in 0..body.slices.len() {
        for other in (index + 1)..body.slices.len() {
            let left = &body.slices[index];
            let right = &body.slices[other];
            if left.slice_id == right.slice_id {
                return Err(DecisionError::new(
                    reason::DUPLICATE_SLICE,
                    format!("重复切片 id：{}", left.slice_id),
                ));
            }
            if scopes_overlap(
                &Scope::new(&left.scope_kind, &left.scope_value),
                &Scope::new(&right.scope_kind, &right.scope_value),
            ) {
                // 整单拒绝：一行都不落库（GOLD-02）。
                return Err(DecisionError::new(
                    reason::SLICE_OVERLAP,
                    format!("切片重叠：{} 与 {}", left.scope_value, right.scope_value),
                ));
            }
        }
    }
    let ids = store::insert_slices(
        connection,
        &task.task_id,
        &body.slices,
        if deterministic {
            "deterministic"
        } else {
            "coordinator"
        },
    )?;
    let state = if task.state == "created" || task.state == "decomposed" {
        "decomposed"
    } else {
        "running"
    };
    store::set_task_state(connection, &task.task_id, state, lease.lease_epoch)?;
    Ok(format!("decompose:{}", ids.len()))
}

/// SPEC §3.3：还能被新 attempt 复用的行不算活跃占用。
fn is_active_assignment_state(state: &str) -> bool {
    state != "created" && state != "rejected_retry" && state != "cancelled"
}

/// SPEC §2.2：一个 pending 切片派给一个 idle Worker，一次只派一个。
fn apply_assign(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::AssignPayload,
    snapshot: &AllowlistSnapshot,
) -> Result<String, DecisionError> {
    let slice = store::load_slice(connection, &body.slice_id)?.ok_or_else(|| {
        DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("切片不可寻：{}", body.slice_id),
        )
    })?;
    if slice.parent_task_id != task.task_id {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            "切片不属于本任务",
        ));
    }
    if slice.state != "pending" && slice.state != "rejected" {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("切片状态 {} 不可分配", slice.state),
        ));
    }
    if slice.retry_count >= slice.max_retries && slice.state == "rejected" {
        return Err(DecisionError::new(
            reason::RETRY_LIMIT_EXCEEDED,
            "已达 max_retries，只能 stop 或永久 reject",
        ));
    }
    if task.degraded {
        // SPEC §4 / DESIGN §7 第 4 条：确定性降级期间第一期只串行派第一个未终结切片，
        // 其余切片保持 pending；候选表仍列全部切片，所以这里必须是硬护栏而不是候选裁剪。
        let open: Vec<String> = store::load_slices(connection, &task.task_id)?
            .into_iter()
            .filter(|item| {
                item.state == "pending"
                    || (item.state == "rejected" && item.retry_count < item.max_retries)
            })
            .map(|item| item.slice_id)
            .collect();
        if open.first() != Some(&body.slice_id) {
            return Err(DecisionError::new(
                reason::ALLOWLIST_VIOLATION,
                "降级期间只能串行派发第一个未终结切片",
            ));
        }
    }
    // DESIGN 附录 C：Phase 1 只读 lane 才允许同质并行。
    if slice.lane != "read_only_analysis" && super::execution_policy::read_original_on(connection,&task.root_run_id).map_err(|e|DecisionError::new(reason::LANE_QUOTA_EXCEEDED,e))?.is_none() {
        return Err(DecisionError::new(
            reason::LANE_QUOTA_EXCEEDED,
            format!("Phase 1 不开放 lane {} 的同质并行", slice.lane),
        ));
    }
    if body.lane != slice.lane {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "assign.lane 必须与切片 lane 一致",
        ));
    }
    if slice.lane != "read_only_analysis" {
        crate::commands::homogeneous_execution::validate_assign(connection,lease,&slice,body).map_err(|e|DecisionError::new(reason::TOOL_NOT_AUTHORIZED,e))?;
        let active:i64=connection.query_row("SELECT count(*) FROM agent_lane_slots WHERE root_run_id=?1 AND released_at=''",[&task.root_run_id],|r|r.get(0)).map_err(|e|DecisionError::new(reason::LANE_QUOTA_EXCEEDED,e.to_string()))?;
        if active!=0 {return Err(DecisionError::new(reason::LANE_QUOTA_EXCEEDED,"执行和复核必须独占；先验收并释放已有切片"));}
    } else if super::execution_policy::read_original_on(connection,&task.root_run_id).map_err(|e|DecisionError::new(reason::LANE_QUOTA_EXCEEDED,e))?.is_some(){
        let exclusive:i64=connection.query_row("SELECT count(*) FROM agent_lane_slots WHERE root_run_id=?1 AND lane IN ('target_touching','review') AND released_at=''",[&task.root_run_id],|r|r.get(0)).map_err(|e|DecisionError::new(reason::LANE_QUOTA_EXCEEDED,e.to_string()))?;
        if exclusive!=0 {return Err(DecisionError::new(reason::LANE_QUOTA_EXCEEDED,"目标和复核的独占占用尚未释放"));}
    }
    let dedup_key = store::dedup_key_for(
        &slice.slice_id,
        &task.target_key,
        slice.evidence_revision_base,
        store::HOMOGENEOUS_TRIGGER_CODE,
    );
    let assignment_id = store::assignment_id_for(&task.root_run_id, &dedup_key);
    // 身份冲突先于资源准入（SPEC §1.4 的去重键含 slice_id）：同一切片已有活跃
    // attempt 时必须报 duplicate_slice，不能被预算错误掩盖。
    let prior = store::load_assignment(connection, &assignment_id)?;
    if let Some(prior) = prior.as_ref() {
        if is_active_assignment_state(&prior.state) {
            return Err(DecisionError::new(
                reason::DUPLICATE_SLICE,
                format!(
                    "切片 {} 已有活跃 attempt（{}）",
                    slice.slice_id, prior.state
                ),
            ));
        }
    }
    let mut expected_attempt = if slice.state == "pending" {
        1
    } else {
        slice.retry_count + 1
    };
    // 复用同一行时 attempt 必须严格前进：租约到期回收不占质量重试额度
    // （DESIGN §6），所以下限取「上一 attempt + 1」而不是 retry_count。
    if let Some(prior) = prior.as_ref() {
        expected_attempt = expected_attempt.max(prior.attempt + 1);
    }
    if body.attempt != expected_attempt {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("attempt 必须是 {expected_attempt}：重跑必须新 attempt"),
        ));
    }
    let worker = pool::load_worker(connection, &body.worker_id)?.ok_or_else(|| {
        DecisionError::new(
            reason::WORKER_NOT_FOUND,
            format!("Worker 不可寻：{}", body.worker_id),
        )
    })?;
    if worker.state != "idle" {
        return Err(DecisionError::new(
            reason::WORKER_NOT_FOUND,
            format!("Worker 不在 idle：{}", worker.state),
        ));
    }
    if worker.lease_epoch > lease.lease_epoch {
        return Err(DecisionError::new(
            reason::LEASE_INVALID,
            "Worker 投影来自更新的 epoch",
        ));
    }
    // 工具候选按 lane/profile 收窄（SPEC §2.2 校验 4）。
    let lane_snapshot = AllowlistSnapshot {
        tool_ids: gate::tool_allowlist_for_task(connection,task,&slice.lane)?,
        ..snapshot.clone()
    };
    allowlist::verify_capability_lease(&lane_snapshot, &body.capability_lease)?;
    let (remaining_tokens, remaining_requests) =
        intent::coarse_budget_capacity(connection, &task.root_run_id)?;
    let budgeted = AllowlistSnapshot {
        // 仅把既有 hard=0 的 unlimited 维投影为比较上界；有限帽与原共享准入不变。
        remaining_tokens: remaining_tokens.unwrap_or(i64::MAX),
        remaining_requests: remaining_requests.unwrap_or(i64::MAX),
        ..snapshot.clone()
    };
    allowlist::verify_reserved_budget(&budgeted, body.reserved_tokens, body.reserved_requests)?;
    store::ensure_ledger(connection, lease)?;

    let assignment = types::HomogeneousAssignment {
        assignment_id: assignment_id.clone(),
        coordinator_run_id: task.root_run_id.clone(),
        parent_task_id: task.task_id.clone(),
        slice_id: slice.slice_id.clone(),
        worker_id: body.worker_id.clone(),
        worker_profile_id: types::PROFILE_HOMOGENEOUS_V1.to_string(),
        target_key: task.target_key.clone(),
        lane: slice.lane.clone(),
        state: "assigned".to_string(),
        reserved_tokens: body.reserved_tokens,
        reserved_requests: body.reserved_requests,
        capability_lease: body.capability_lease.clone(),
        evidence_revision_base: slice.evidence_revision_base,
        attempt: body.attempt,
        dedup_key: dedup_key.clone(),
        trigger_code: store::HOMOGENEOUS_TRIGGER_CODE.to_string(),
        scope_kind: slice.scope_kind.clone(),
        scope_value: slice.scope_value.clone(),
    };
    match store::insert_assignment(connection, &assignment)? {
        store::AssignmentInsert::Inserted(_) => {}
        store::AssignmentInsert::Existing(existing) => {
            // 同 dedup key 的重放：只有上一 attempt 已终结才允许带着新 attempt 复用原行。
            let prior = store::load_assignment(connection, &existing)?.ok_or_else(|| {
                DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "原 assignment 不可读")
            })?;
            if is_active_assignment_state(&prior.state) {
                return Err(DecisionError::new(
                    reason::DUPLICATE_SLICE,
                    format!(
                        "切片 {} 已有活跃 attempt（{}）",
                        slice.slice_id, prior.state
                    ),
                ));
            }
            if prior.attempt >= body.attempt {
                return Err(DecisionError::new(
                    reason::DUPLICATE_SLICE,
                    "重放必须携带更大的 attempt",
                ));
            }
            let changed = connection
                .execute(
                    "UPDATE agent_assignments SET state='assigned',attempt=?1,homogeneous_worker_id=?2,worker_profile_id=?3,\
                     reserved_tokens=?4,reserved_requests=?5,capability_lease_json=?6,slice_id=?7,updated_at=datetime('now','localtime') \
                     WHERE id=?8 AND scheduling_model='homogeneous_workers'",
                    params![
                        body.attempt,
                        body.worker_id,
                        types::PROFILE_HOMOGENEOUS_V1,
                        body.reserved_tokens,
                        body.reserved_requests,
                        serde_json::to_string(&body.capability_lease).map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?,
                        slice.slice_id,
                        existing
                    ],
                )
                .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("attempt 复用失败：{error}")))?;
            if changed != 1 {
                return Err(DecisionError::new(
                    reason::ASSIGNMENT_NOT_FOUND,
                    "原 assignment 不可复用",
                ));
            }
        }
    }
    store::take_slot(
        connection,
        &task.root_run_id,
        &slice.lane,
        &slice.slice_id,
        &assignment_id,
    )?;
    store::reserve_budget(
        connection,
        lease,
        body.reserved_tokens,
        body.reserved_requests,
    )?;
    pool::mark_busy(connection, &body.worker_id, &assignment_id)?;
    store::set_slice_state(connection, &slice.slice_id, "assigned")?;
    if task.state == "decomposed" {
        store::set_task_state(connection, &task.task_id, "running", lease.lease_epoch)?;
    }
    // P0-2：assign 成功后在同一事务里建立 child run / attempt 并经现有 mailbox 下发切片。
    // 失败即整单回滚，不会留下「已派发但无执行载体」的切片（DESIGN §5.3 不改 authority 路由）。
    if assignment.lane=="read_only_analysis" {super::delivery::dispatch_slice(connection, lease, &assignment)?;}
    else {super::execution_carrier::dispatch_slice(connection,lease,&assignment)?;}
    Ok(format!("assign:{assignment_id}:attempt:{}", body.attempt))
}

/// SPEC §3.3：Worker 真正开跑（assignment→running、slice→in_progress）。
pub fn mark_started(connection: &Connection, assignment_id: &str) -> Result<(), DecisionError> {
    let assignment = store::load_assignment(connection, assignment_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "assignment 不可寻"))?;
    if assignment.state != "assigned" {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("assignment 状态 {} 不能开跑", assignment.state),
        ));
    }
    store::set_assignment_state(connection, assignment_id, "running")?;
    store::set_slice_state(connection, &assignment.slice_id, "in_progress")?;
    // P0-2：载体与 assignment 同步开跑，mailbox 的 live 校验在汇报前一直成立。
    super::delivery::start_carrier(connection, assignment_id)
}

/// DESIGN §3.3：汇报适配器。校验三元组一致后把 assignment/slice 推到 reported。
pub fn record_worker_report(
    connection: &Connection,
    lease: &CoordinatorLease,
    report: &types::WorkerReport,
) -> Result<(), DecisionError> {
    let assignment = store::load_assignment(connection, &report.assignment_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "assignment 不可寻"))?;
    if assignment.slice_id != report.slice_id || assignment.worker_id != report.worker_id {
        return Err(DecisionError::new(
            reason::CROSS_SLICE_WRITE,
            "汇报的 slice_id/worker_id 与 assignment 不一致",
        ));
    }
    if assignment.state != "running" {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("assignment 状态 {} 不能汇报", assignment.state),
        ));
    }
    if report.consumed_tokens > assignment.reserved_tokens
        || report.consumed_requests > assignment.reserved_requests
    {
        return Err(DecisionError::new(
            reason::BUDGET_EXCEEDED,
            "consumed_* 超过 reserved_*",
        ));
    }
    if !machine::output_shape_valid(&report.output) {
        let code = if machine::output_invents_tool(&report.output) {
            reason::INVENTED_TOOL
        } else {
            reason::OUTPUT_SCHEMA_INVALID
        };
        return Err(DecisionError::new(
            code,
            "Worker output 不符合 SPEC §1.5 形状",
        ));
    }
    store::insert_report(connection, report, lease.lease_epoch)?;
    store::set_assignment_state(connection, &assignment.assignment_id, "reported")?;
    store::set_slice_state(connection, &assignment.slice_id, "reported")?;
    // P0-2：汇报已落库，只确认下发消息；载体活到裁决落地。
    super::delivery::report_carrier(connection, &assignment.assignment_id)
}

fn load_reported_context(
    connection: &Connection,
    task: &types::HomogeneousTask,
    assignment_id: &str,
) -> Result<
    (
        types::HomogeneousAssignment,
        types::TaskSlice,
        types::WorkerReport,
    ),
    DecisionError,
> {
    let assignment = store::load_assignment(connection, assignment_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "assignment 不可寻"))?;
    if assignment.state != "reported" {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("assignment 状态 {} 尚未汇报", assignment.state),
        ));
    }
    if assignment.parent_task_id != task.task_id {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            "assignment 不属于本任务",
        ));
    }
    let slice = store::load_slice(connection, &assignment.slice_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "切片不可寻"))?;
    let report = match store::load_report(
        connection,
        &store::report_id_for(assignment_id, assignment.attempt),
    )? {
        Some(found) => found,
        None => {
            let latest: Option<String> = connection
                .query_row(
                    "SELECT report_id FROM agent_worker_reports WHERE assignment_id=?1 ORDER BY created_at DESC LIMIT 1",
                    [assignment_id],
                    |row| row.get(0),
                )
                .ok();
            match latest {
                Some(id) => store::load_report(connection, &id)?.ok_or_else(|| {
                    DecisionError::new(reason::EVIDENCE_MISSING, "找不到该 assignment 的汇报")
                })?,
                None => {
                    return Err(DecisionError::new(
                        reason::EVIDENCE_MISSING,
                        "找不到该 assignment 的汇报",
                    ))
                }
            }
        }
    };
    Ok((assignment, slice, report))
}

fn release_after_verdict(
    connection: &Connection,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
    consumed_tokens: i64,
    consumed_requests: i64,
) -> Result<(), DecisionError> {
    store::release_slot(
        connection,
        &task.root_run_id,
        &assignment.lane,
        &assignment.slice_id,
    )?;
    crate::commands::homogeneous_execution::release_on(connection,assignment).map_err(|e|DecisionError::new(reason::SCHEMA_INVALID,e))?;
    if !assignment.worker_id.is_empty() {
        pool::mark_idle(connection, &assignment.worker_id)?;
    }
    store::settle_budget(
        connection,
        &task.root_run_id,
        assignment.reserved_tokens,
        assignment.reserved_requests,
        consumed_tokens,
        consumed_requests,
    )
}

/// 全部切片终态后推进任务态（SPEC §3.1）。
fn advance_task_state(
    connection: &Connection,
    task: &types::HomogeneousTask,
    lease_epoch: i64,
) -> Result<String, DecisionError> {
    let slices = store::load_slices(connection, &task.task_id)?;
    // 「closed」必须把还能重试的 rejected 算作未终态，否则单切片任务一次验收不通过就被判 failed：
    // 之后 gate 对终态任务不给任何动作，既不能再派新 attempt，也无法从 failed 走到 stop。
    let closed = |slice: &types::TaskSlice| match slice.state.as_str() {
        "accepted" | "cancelled" => true,
        "rejected" => slice.retry_count >= slice.max_retries,
        _ => false,
    };
    let open = slices.iter().filter(|slice| !closed(slice)).count();
    if open > 0 || slices.is_empty() {
        return Ok("running".to_string());
    }
    let accepted = slices
        .iter()
        .filter(|slice| slice.state == "accepted")
        .count();
    // SPEC §3.1：全部 accept 不由验收动作推进，必须等主管 stop 收尾；
    // 混合终态才提前落到 partial_success。
    let state = if accepted == 0 {
        // §J：全失败耗尽由下一次 decompose 换方向或 stop 收尾，不能提前 failed 禁用主管。
        return Ok("running".to_string());
    } else if accepted < slices.len() {
        "partial_success"
    } else {
        return Ok("running".to_string());
    };
    store::set_task_state(connection, &task.task_id, state, lease_epoch)?;
    Ok(state.to_string())
}

/// SPEC §2.3：验收。机器可检由 Rust 重算，模型谎报无效（GOLD-09）。
fn apply_accept(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::AcceptPayload,
) -> Result<String, DecisionError> {
    let (assignment, slice, report) = load_reported_context(connection, task, &body.assignment_id)?;
    if body.slice_id != assignment.slice_id {
        return Err(DecisionError::new(
            reason::CROSS_SLICE_WRITE,
            "accept.slice_id 与 assignment 不一致",
        ));
    }
    if body.report_id != report.report_id {
        return Err(DecisionError::new(
            reason::EVIDENCE_MISSING,
            "report_id 与该 assignment 的最新汇报不符",
        ));
    }
    let recomputed = machine::recompute(connection, task, &assignment, &slice, &report);
    if !machine::matches(&body.machine_checks, &recomputed) {
        return Err(DecisionError::new(
            reason::MACHINE_CHECK_FAILED,
            format!("模型回显与 Rust 重算不一致：{:?}", recomputed.failed_keys()),
        ));
    }
    if !recomputed.all_true() {
        return Err(DecisionError::new(
            reason::MACHINE_CHECK_FAILED,
            format!("机器可检未全绿：{:?}", recomputed.failed_keys()),
        ));
    }
    for reference in &body.evidence_refs {
        if !report.evidence_refs.contains(reference) {
            return Err(DecisionError::new(
                reason::EVIDENCE_MISSING,
                format!("evidence_ref 不在汇报内：{reference}"),
            ));
        }
    }
    let verdict = types::AcceptanceVerdict {
        verdict_id: store::verdict_id_for(&assignment.assignment_id, "accept", assignment.attempt),
        assignment_id: assignment.assignment_id.clone(),
        slice_id: assignment.slice_id.clone(),
        verdict: "accept".to_string(),
        machine_checks: recomputed,
        model_judgement: body.model_judgement.clone(),
        reason_codes: body.reason_codes.clone(),
        evidence_refs: body.evidence_refs.clone(),
        decided_by_lease_epoch: lease.lease_epoch,
        attempt: assignment.attempt,
    };
    crate::commands::homogeneous_execution::accept_on(connection,lease,&assignment).map_err(|e|DecisionError::new(reason::MODEL_JUDGEMENT_FAIL,e))?;
    store::insert_verdict(connection, &verdict)?;
    store::set_assignment_state(connection, &assignment.assignment_id, "accepted")?;
    store::set_slice_state(connection, &assignment.slice_id, "accepted")?;
    release_after_verdict(
        connection,
        task,
        &assignment,
        report.consumed_tokens,
        report.consumed_requests,
    )?;
    let state = advance_task_state(connection, task, lease.lease_epoch)?;
    // P0-2：裁决已落地，执行载体可以进入终态（attempt→completed、run→terminal）。
    super::delivery::close_carrier(connection, &assignment.assignment_id)?;
    Ok(format!("accept:{}:task:{state}", assignment.assignment_id))
}

/// SPEC §2.4：验收不通过，同切片新 attempt 重做。
fn apply_reject_retry(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::RejectRetryPayload,
) -> Result<String, DecisionError> {
    let (assignment, slice, report) = load_reported_context(connection, task, &body.assignment_id)?;
    if body.slice_id != assignment.slice_id {
        return Err(DecisionError::new(
            reason::CROSS_SLICE_WRITE,
            "reject_retry.slice_id 与 assignment 不一致",
        ));
    }
    if slice.retry_count >= slice.max_retries {
        return Err(DecisionError::new(
            reason::RETRY_LIMIT_EXCEEDED,
            format!("已达 max_retries={}", slice.max_retries),
        ));
    }
    let recomputed = machine::recompute(connection, task, &assignment, &slice, &report);
    if !machine::matches(&body.machine_checks, &recomputed) {
        return Err(DecisionError::new(
            reason::MACHINE_CHECK_FAILED,
            format!("模型回显与 Rust 重算不一致：{:?}", recomputed.failed_keys()),
        ));
    }
    if recomputed.all_true()
        && body.model_judgement.as_ref().map(|value| value.adequate) == Some(true)
    {
        return Err(DecisionError::new(
            reason::MODEL_JUDGEMENT_FAIL,
            "机器全绿且判定达标时应走 accept",
        ));
    }
    let verdict = types::AcceptanceVerdict {
        verdict_id: store::verdict_id_for(&assignment.assignment_id, "retry", assignment.attempt),
        assignment_id: assignment.assignment_id.clone(),
        slice_id: assignment.slice_id.clone(),
        verdict: "retry".to_string(),
        machine_checks: recomputed,
        model_judgement: body.model_judgement.clone(),
        reason_codes: body.reason_codes.clone(),
        evidence_refs: body.evidence_refs.clone(),
        decided_by_lease_epoch: lease.lease_epoch,
        attempt: assignment.attempt,
    };
    store::insert_verdict(connection, &verdict)?;
    store::set_assignment_state(connection, &assignment.assignment_id, "rejected_retry")?;
    store::bump_slice_retry(connection, &assignment.slice_id)?;
    store::set_slice_state(connection, &assignment.slice_id, "rejected")?;
    // 该 attempt 的 worker 授权到此结束：关闭载体后，重派才能写新的 attempt 身份。
    super::delivery::close_carrier(connection, &assignment.assignment_id)?;
    release_after_verdict(
        connection,
        task,
        &assignment,
        report.consumed_tokens,
        report.consumed_requests,
    )?;
    let _ = advance_task_state(connection, task, lease.lease_epoch)?;
    Ok(format!("reject_retry:{}", assignment.assignment_id))
}

/// SPEC §2.5：收尾。Phase 1 禁止带 running 直接 stop（GOLD-21）。
fn apply_stop(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::StopPayload,
) -> Result<String, DecisionError> {
    if !types::is_member(&body.stop_reason, &types::STOP_REASONS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("stop_reason 非法：{}", body.stop_reason),
        ));
    }
    let busy = store::list_assignment_ids_in_state(
        connection,
        &task.root_run_id,
        &["created", "assigned", "running", "reported"],
    )?;
    if !busy.is_empty() {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            format!("仍有 {busy:?} 未收尾，Phase 1 禁止带执行中的 assignment 停止"),
        ));
    }
    let slices = store::load_slices(connection, &task.task_id)?;
    let mut truth_accepted: Vec<String> = slices
        .iter()
        .filter(|slice| slice.state == "accepted")
        .map(|slice| slice.slice_id.clone())
        .collect();
    let mut truth_other: Vec<String> = slices
        .iter()
        .filter(|slice| slice.state != "accepted")
        .map(|slice| slice.slice_id.clone())
        .collect();
    truth_accepted.sort();
    truth_other.sort();
    let mut declared_accepted = body.accepted_slice_ids.clone();
    let mut declared_rejected = body.rejected_slice_ids.clone();
    declared_accepted.sort();
    declared_rejected.sort();
    if truth_accepted != declared_accepted {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            "accepted_slice_ids 与真实状态不一致",
        ));
    }
    if truth_other != declared_rejected {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            "rejected_slice_ids 与真实状态不一致",
        ));
    }
    if body.publish_partial && body.accepted_slice_ids.is_empty() {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "publish_partial=true 时至少要有 1 个 accepted",
        ));
    }
    // 先保留 Phase 1 执行中 stop 的原硬拒与真实列表校验，再校验 §J 收尾条件。
    intent::validate_stop(connection, task, body)?;
    store::cancel_leftover_slices(connection, &task.task_id)?;
    let state = if body.accepted_slice_ids.is_empty() {
        "failed"
    } else if body.rejected_slice_ids.is_empty() {
        "succeeded"
    } else {
        "partial_success"
    };
    store::set_task_state(connection, &task.task_id, "stopped", lease.lease_epoch)?;
    store::release_all_slots(connection, &task.root_run_id)?;
    // 未清理的执行中 assignment 已在前面的硬拒里挡掉；这里只做投影收尾。
    let _ = current_revision(connection, &task.root_run_id)?;
    Ok(format!(
        "stop:{state}:publish_partial:{}",
        body.publish_partial
    ))
}

/// SPEC §2.6：转发（可改写）用户引导。
fn apply_relay_directive(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    body: &types::RelayDirectivePayload,
    snapshot: &AllowlistSnapshot,
) -> Result<String, DecisionError> {
    let directive =
        store::load_gated_directive(connection, &body.directive_id)?.ok_or_else(|| {
            DecisionError::new(
                reason::ALLOWLIST_VIOLATION,
                format!("引导不可寻：{}", body.directive_id),
            )
        })?;
    if directive.state != "rule_screened" && directive.state != "awaiting_coordinator" {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("引导状态 {} 不可转发", directive.state),
        ));
    }
    if directive.rule_screen_result == "reject" {
        // SPEC §3.5：规则已拒的不得再走主管转发通道。
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "规则初筛已拒绝，禁止 relay",
        ));
    }
    if !snapshot.directive_ids.contains(&body.directive_id) {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            "引导不在候选内",
        ));
    }
    let text = if body.mode == "rewrite" {
        body.rewritten_text.clone().unwrap_or_default()
    } else {
        directive.raw_text_redacted.clone()
    };
    if body.mode == "rewrite" {
        let plaintext = super::material_policy::plaintext(connection, &task.scan_id, task.attempt_number)
            .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
        directive_gate::validate_rewrite_material(task, &text, plaintext)?;
    }
    let target_worker = body.target_worker_id.clone().unwrap_or_default();
    let broadcast = body.broadcast_constraint.clone().unwrap_or_default();
    if !target_worker.is_empty() {
        if !snapshot.worker_ids.contains(&target_worker) {
            return Err(DecisionError::new(
                reason::WORKER_NOT_FOUND,
                format!("目标 Worker 不在候选内：{target_worker}"),
            ));
        }
        let worker = pool::load_worker(connection, &target_worker)?
            .ok_or_else(|| DecisionError::new(reason::WORKER_NOT_FOUND, "Worker 不可寻"))?;
        if worker.state != "busy" || worker.assignment_id.is_empty() {
            return Err(DecisionError::new(
                reason::WORKER_NOT_FOUND,
                "引导只能转给正在执行本任务切片的 Worker",
            ));
        }
        let assignment = store::load_assignment(connection, &worker.assignment_id)?
            .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "assignment 不可寻"))?;
        if assignment.parent_task_id != task.task_id {
            return Err(DecisionError::new(
                reason::CROSS_SLICE_WRITE,
                "目标 Worker 不属于本任务",
            ));
        }
    }
    if !broadcast.is_empty() {
        if !types::valid_readable_text(&broadcast, types::MAX_SCOPE_VALUE_LEN) {
            return Err(DecisionError::new(
                reason::SCHEMA_INVALID,
                "broadcast_constraint 必须非空且 <= 512",
            ));
        }
        let plaintext = super::material_policy::plaintext(connection, &task.scan_id, task.attempt_number)
            .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
        directive_gate::validate_rewrite_material(task, &broadcast, plaintext)?;
        // 广播由主管逐个投递；Worker 之间不互聊（DESIGN §5.3）。
        let running = store::list_assignment_ids_in_state(
            connection,
            &task.root_run_id,
            &["assigned", "running"],
        )?;
        if running.is_empty() {
            return Err(DecisionError::new(
                reason::ASSIGNMENT_NOT_FOUND,
                "没有正在执行的 assignment 可接收广播约束",
            ));
        }
    }
    if target_worker.is_empty() && broadcast.is_empty() {
        // SPEC §2.6 校验 4：转发必须给出目标 Worker 或广播约束之一。
        // 两者都缺时不得把引导标成 applied——那等于静默吞掉用户引导。
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "relay_directive 需要 target_worker_id 或 broadcast_constraint 之一",
        ));
    }
    directive_gate::mark_relayed(
        connection,
        &body.directive_id,
        &body.mode,
        &text,
        &target_worker,
        &broadcast,
        &body.user_feedback_zh,
    )?;
    if super::model_worker::enabled(connection, &task.scan_id, task.attempt_number)
        .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))? {
        super::model_guidance::relay(connection, lease, &body.directive_id)
            .map_err(|e| DecisionError::new(reason::SCHEMA_INVALID, e))?;
    }
    Ok(format!(
        "relay_directive:{}:{}",
        body.mode, body.directive_id
    ))
}

/// SPEC §2.7：拒绝引导并反馈。任务与 Worker 状态一律不变。
fn apply_reject_directive(
    connection: &Connection,
    body: &types::RejectDirectivePayload,
    snapshot: &AllowlistSnapshot,
) -> Result<String, DecisionError> {
    let directive = store::load_gated_directive(connection, &body.directive_id)?
        .ok_or_else(|| DecisionError::new(reason::ALLOWLIST_VIOLATION, "引导不可寻"))?;
    if directive.state != "rule_screened" && directive.state != "awaiting_coordinator" {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("引导状态 {} 不可驳回", directive.state),
        ));
    }
    if !snapshot.directive_ids.contains(&body.directive_id) {
        return Err(DecisionError::new(
            reason::ALLOWLIST_VIOLATION,
            "引导不在候选内",
        ));
    }
    if !reason::DIRECTIVE_RELATED.contains(&body.reason_code.as_str()) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "reject_directive 只能用引导族 reason_code",
        ));
    }
    let stored = directive_gate::mark_rejected(
        connection,
        &body.directive_id,
        &body.reason_code,
        &body.reason_zh,
        &body.suggestion_zh,
        &body.user_feedback_zh,
    )?;
    if !stored.rejection_fields_complete() {
        return Err(DecisionError::new(
            reason::MISSING_REQUIRED_FIELD,
            "驳回四字段缺一",
        ));
    }
    // DESIGN §8.6：任务状态与 Worker 不变，本轮绝不 incomplete。
    Ok(format!("reject_directive:{}", body.directive_id))
}

/// 供 legacy 钩子在 reject 已回用户后继续本轮使用（DESIGN §5.5）。
pub fn rejected_feedback_is_visible(connection: &Connection, directive_id: &str) -> bool {
    directive_gate::rejected_feedback_ready(connection, directive_id)
}

pub fn decision_detail_json(outcome: &TurnOutcome) -> JsonValue {
    match outcome {
        TurnOutcome::Applied(detail) => serde_json::json!({"status": "applied", "detail": detail}),
        TurnOutcome::Refused {
            reason_code,
            detail,
            schema_fail_streak,
            hard_reject_streak,
        } => serde_json::json!({
            "status": "refused",
            "reason_code": reason_code,
            "detail": detail,
            "schema_fail_streak": schema_fail_streak,
            "hard_reject_streak": hard_reject_streak,
        }),
        TurnOutcome::Degraded(inner) => serde_json::json!({
            "status": "degraded",
            "outcome": format!("{inner:?}"),
        }),
        TurnOutcome::Idle(state) => serde_json::json!({"status": "idle", "task_state": state}),
    }
}

pub fn task_exists_for_root(connection: &Connection, root_run_id: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_homogeneous_tasks WHERE root_run_id=?1 AND scheduling_model='homogeneous_workers')",
            [root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

pub fn load_task_for_lease(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<types::HomogeneousTask, DecisionError> {
    task_of(connection, lease)
}
