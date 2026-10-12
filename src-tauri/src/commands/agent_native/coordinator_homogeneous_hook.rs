// Phase 1 审核 P0-1：Root 付费 tick 的返回路径 → 同质主管决策落库。
//
// 付费不变量：一轮只有一次发票。这里在 `tick.received` 已结算、decision 事件已发布
// 之后执行副作用；拒绝/降级都属正常结果（streak 与确定性降级在 apply 内部处理），
// 绝不因为一次结构不合就重开付费轮。原始模型文本只在内存里传递，不落库。

/// 账本摘要必须只带本轮的固定 step（与其他 assess:* kind 的边界检查同构）。
fn native_homogeneous_require_bounded(summary: &JsonValue) -> Result<(), String> {
    if summary["suggestions"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
        && summary["suggestions"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                item.as_str()
                    == Some(crate::agent_runtime::multi_agent::homogeneous::journal::LEDGER_STEP)
            })
        })
    {
        return Ok(());
    }
    Err("root_homogeneous_step_not_bounded".into())
}

/// 把模型原文交给唯一副作用入口 `apply::run_coordinator_turn`，返回审计摘要。
/// 只有控制面自身无法完成的一轮才报错（租约失效等）；模型决策被硬拒不算错误。
pub(crate) fn native_homogeneous_apply_after_paid_tick(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    model_text: &str,
) -> Result<String, String> {
    use crate::agent_runtime::multi_agent::homogeneous::apply;
    let outcome = apply::run_coordinator_turn(db, actor, model_text)
        .map_err(|error| format!("root_homogeneous_turn_failed:{}", error.reason_code))?;
    Ok(match outcome {
        apply::TurnOutcome::Applied(detail) => format!("root_homogeneous_applied:{detail}"),
        apply::TurnOutcome::Refused {
            reason_code,
            schema_fail_streak,
            hard_reject_streak,
            ..
        } => format!(
            "root_homogeneous_refused:{reason_code}:schema:{schema_fail_streak}:reject:{hard_reject_streak}"
        ),
        apply::TurnOutcome::Degraded(_) => "root_homogeneous_degraded:deterministic".to_string(),
        apply::TurnOutcome::Idle(state) => format!("root_homogeneous_idle:{state}"),
    })
}

/// Phase 1.1：主管决策落地后，同一次付费轮的尾巴上让 Worker 消费已下发的切片。
/// 这里只调用控制面的消费者，不复制任何执行逻辑；drain 自身失败只留痕，
/// 不把已经结算的轮次打成错误——过期的载体由到期回收兜底，下一轮重新消费。
fn native_homogeneous_drain_workers(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> String {
    use crate::agent_runtime::multi_agent::homogeneous::worker;
    match worker::drain_for_lease(db, actor) {
        Ok(consumed) if consumed.is_empty() => "root_homogeneous_workers:idle".to_string(),
        Ok(consumed) => {
            let detail = consumed
                .iter()
                .map(|item| match item.reason_code {
                    Some(code) => format!("{}:{code}", item.assignment_id),
                    None => format!("{}:{}", item.assignment_id, item.terminal_hint),
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("root_homogeneous_workers:{}:{detail}", consumed.len())
        }
        Err(error) => format!(
            "root_homogeneous_worker_drain_refused:{}",
            error.reason_code
        ),
    }
}

/// P1-1：同质主管决策的 basis 生产（纯读）。
/// 只给出候选、允许动作与剩余预算——模型只能从这里挑 id（DESIGN §9）。
fn native_homogeneous_basis_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<JsonValue, String> {
    native_homogeneous_basis_directives_on(db, actor, None)
}
fn native_homogeneous_basis_directives_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    directives: Option<&[String]>,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{decompose, gate, intent, journal, machine, pool, reason, store};
    let task = store::load_task_by_root_run(db, &actor.root_run_id)
        .map_err(|error| error.reason_code.to_string())?
        .ok_or("root_homogeneous_task_missing")?;
    let mut allowed =
        gate::allowed_actions(db, &task).map_err(|error| error.reason_code.to_string())?;
    let snapshot = gate::snapshot_for_directives(db, &task, directives).map_err(|error| error.reason_code.to_string())?;
    if directives.is_some_and(|ids| ids.is_empty()) {allowed.retain(|action| action!="relay_directive" && action!="reject_directive");}
    let (remaining_tokens, remaining_requests) = store::remaining_budget(db, &actor.root_run_id)
        .map_err(|error| error.reason_code.to_string())?;
    // IDs alone are insufficient to produce a valid slice or assess a report.
    // Supply bounded Rust-derived contract data, including deterministic new slice IDs.
    let mut drafts = decompose::deterministic_candidates(db, &task)
        .map_err(|error| error.reason_code.to_string())?;
    drafts.extend(crate::commands::homogeneous_execution::drafts(db,&task)?);
    let drafts: Vec<JsonValue> = drafts.into_iter().map(|draft| json!({
        "slice_id": draft.slice_id, "scope_kind": draft.scope_kind, "scope_value": draft.scope_value,
        "lane": draft.lane, "evidence_revision_base": draft.evidence_revision_base,
        "worker_capability_profile_id": draft.worker_capability_profile_id, "max_retries": draft.max_retries
    })).collect();
    let slices = store::load_slices(db, &task.task_id).map_err(|error| error.reason_code.to_string())?;
    let mut slice_cards = Vec::new();
    for slice in &slices {
        let next_attempt: i64 = db.query_row(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM agent_assignments WHERE coordinator_run_id=?1 AND slice_id=?2 AND scheduling_model='homogeneous_workers'",
            params![task.root_run_id, slice.slice_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        slice_cards.push(json!({
            "slice_id": slice.slice_id, "scope_kind": slice.scope_kind, "scope_value": slice.scope_value,
            "lane": slice.lane, "state": slice.state, "evidence_revision_base": slice.evidence_revision_base,
            "retry_count": slice.retry_count, "max_retries": slice.max_retries, "next_attempt": next_attempt
        }));
        if crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(db, &actor.scan_id, actor.attempt_number)? {
            let minimum=crate::commands::homogeneous_execution::slice_minimum(db,actor,slice)?;
            if let Some(card) = slice_cards.last_mut() {
                card["minimum_reserved_tokens"] = json!(minimum.0);
                card["minimum_reserved_requests"] = json!(minimum.1);
                card["allowed_tools"]=json!(gate::tool_allowlist_for_task(db,&task,&slice.lane).map_err(|e|e.detail)?);
                if slice.lane=="review" {
                    let tx;let read=if db.is_autocommit(){tx=db.unchecked_transaction().map_err(|e|e.to_string())?;&*tx}else{db};
                    let fact=crate::commands::homogeneous_execution::read_fact(read,actor,slice.scope_value.strip_prefix("review-candidate:").ok_or("homogeneous_review_scope_invalid")?)?;
                    card["excluded_worker_ids"]=json!([fact["workerId"]]);
                }
            }
        }
    }
    let mut workers = Vec::new();
    for id in &snapshot.worker_ids {
        if let Some(worker) = pool::load_worker(db, id).map_err(|error| error.reason_code.to_string())? {
            workers.push(json!({"worker_id": worker.worker_id, "state": worker.state,
                "assignment_id": worker.assignment_id, "capability_profile_id": worker.capability_profile_id}));
        }
    }
    let mut reports = Vec::new();
    for id in &snapshot.assignment_ids {
        let assignment = store::load_assignment(db, id).map_err(|error| error.reason_code.to_string())?
            .ok_or("root_homogeneous_assignment_missing")?;
        if assignment.state != "reported" { continue; }
        let report_id = store::report_id_for(id, assignment.attempt);
        let Some(report) = store::load_report(db, &report_id).map_err(|error| error.reason_code.to_string())? else { continue; };
        let slice = slices.iter().find(|slice| slice.slice_id == assignment.slice_id)
            .ok_or("root_homogeneous_report_slice_missing")?;
        reports.push(json!({"assignment_id": id, "slice_id": report.slice_id, "report_id": report.report_id,
            "attempt": assignment.attempt, "terminal_hint": report.terminal_hint,
            "evidence_refs": report.evidence_refs, "output": report.output,
            "system_machine_checks": machine::recompute(db, &task, &assignment, slice, &report)}));
    }
    let intent_snapshot = intent::snapshot(db, &task).map_err(|error| error.reason_code.to_string())?;
    let mut directives = Vec::new();
    for id in &snapshot.directive_ids {
        if let Some(directive) = store::load_gated_directive(db, id).map_err(|error| error.reason_code.to_string())? {
            directives.push(json!({"directive_id": id, "raw_text_redacted": directive.raw_text_redacted,
                "rule_screen_result": directive.rule_screen_result, "rule_reason_codes": directive.rule_reason_codes}));
        }
    }
    Ok(json!({
        "phase": NATIVE_HOMOGENEOUS_FRAME_KIND,
        "step": journal::LEDGER_STEP,
        "homogeneousTask": {
            "taskId": task.task_id,
            "goal": crate::agent_runtime::multi_agent::homogeneous::model_worker::goal(db, &task)?,
            "state": task.state,
            "attemptNumber": task.attempt_number,
            "targetKey": task.target_key,
            "evidenceRevisionBase": task.evidence_revision_base,
            "allowedActions": allowed,
            "candidates": {
                "sliceIds": snapshot.slice_ids,
                "workerIds": snapshot.worker_ids,
                "toolIds": snapshot.tool_ids,
                "targetKeys": snapshot.target_keys,
                "directiveIds": snapshot.directive_ids,
                "assignmentIds": snapshot.assignment_ids
            },
            "basisHash": snapshot.basis_prefix(),
            "remainingTokens": remaining_tokens,
            "remainingRequests": remaining_requests,
            "allowedParallel": task.allowed_parallel,
            "degraded": task.degraded,
            "schemaFailStreak": task.schema_fail_streak,
            "hardRejectStreak": task.hard_reject_streak,
            "decompositionCandidates": drafts,
            "slices": slice_cards,
            "workers": workers,
            "reports": reports,
            "intentSnapshot": intent_snapshot,
            "directives": directives,
            "phase1Lane": "read_only_analysis",
            "executionMode": if crate::agent_runtime::multi_agent::homogeneous::execution_policy::read_original_on(db,&task.root_run_id)?.is_some(){"model_broker_v1"}else{"model_read_only_v1"},
            "goalReady":crate::commands::homogeneous_execution::goal_ready(db,&task)?,
            "executionRule": "只有原始冻结 model_broker_v1 才允许 target_touching/review；二者与其他占用必须串行。目标任务只用 authorization_probe 且模型预算为0，复核任务只用 evidence.read/review.write 且必须排除产出 Worker。每次从 Rust 候选选择切片和 idle Worker，禁止固定角色或扩容；goalReady=false 不得 goal_reached。先完成批准解析任务，再执行登记控制组，最后独立复核。",
            "reason_code_allowlist": reason::ALL.as_slice(),
            "payload_keys": {
                "decompose": ["slices", "decomposition_basis_zh"],
                "assign": ["slice_id", "worker_id", "reserved_tokens", "reserved_requests", "capability_lease", "lane", "attempt"],
                "accept": ["assignment_id", "slice_id", "report_id", "verdict", "machine_checks", "model_judgement", "reason_codes", "evidence_refs"],
                "reject_retry": ["assignment_id", "slice_id", "report_id", "verdict", "machine_checks", "model_judgement", "reason_codes", "retry_hint_zh", "evidence_refs"],
                "stop": ["stop_reason", "accepted_slice_ids", "rejected_slice_ids", "summary_zh", "publish_partial"],
                "relay_directive": ["directive_id", "mode", "rewritten_text", "target_worker_id", "broadcast_constraint", "user_feedback_zh"],
                "reject_directive": ["directive_id", "reason_code", "reason_zh", "suggestion_zh", "user_feedback_zh"]
            }
        }
    }))
}

/// P1-1：帧生产者。语义里只带候选哈希与允许动作，不授予任何执行权。
fn native_homogeneous_decision_frame(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorFrame, String> {
    let basis = native_homogeneous_basis_on(db, actor)?;
    Ok(NativeCoordinatorFrame {
        kind: NATIVE_HOMOGENEOUS_FRAME_KIND,
        semantic: json!({
            "rootRunId": actor.root_run_id,
            "scanId": actor.scan_id,
            "attemptNumber": actor.attempt_number,
            "targetKey": actor.target_key,
            "taskId": basis["homogeneousTask"]["taskId"],
            "allowedActions": basis["homogeneousTask"]["allowedActions"],
            "candidateBasisHash": basis["homogeneousTask"]["basisHash"],
            "candidateDirectiveIds": basis["homogeneousTask"]["candidates"]["directiveIds"],
            "advisoryOnly": true,
            "executionGrant": false,
            "dispatchGranted": false,
            "additionalScopeGranted": false
        }),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    })
}

/// 帧的候选快照必须与当前库内状态一致：漂移即拒，避免用旧 allowlist 付费问模型。
fn native_homogeneous_require_current_frame(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<JsonValue, String> {
    let frozen=frame.semantic["candidateDirectiveIds"].as_array().map(|ids| ids.iter().filter_map(|id|id.as_str().map(str::to_owned)).collect::<Vec<_>>());
    let frozen = if crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(db,&actor.scan_id,actor.attempt_number)? {frozen} else {None};
    let basis = native_homogeneous_basis_directives_on(db, actor, frozen.as_deref())?;
    if frame.kind != NATIVE_HOMOGENEOUS_FRAME_KIND
        || frame.semantic["candidateBasisHash"] != basis["homogeneousTask"]["basisHash"]
        || frame.semantic["rootRunId"] != JsonValue::String(actor.root_run_id.clone())
    {
        return Err("root_homogeneous_candidate_basis_changed".into());
    }
    Ok(basis)
}

/// P1-1 触发点：无帧的 Root 主管轮只有在 run 显式选择 `homogeneous_workers` 时才 mint
/// 同质主管帧（DESIGN §2.2 任务级开关）。默认 `legacy_role_pipeline` 的 Root 一帧都不
/// mint，旧付费轮的 basis / kind / 打分逻辑完全不变。
///
/// 显式同质任务的第一张帧就是原始 round 1；Tick::begin 记录真实模型事实，
/// 后续付费轮继续以该事实校验 provider/credential/schema，不调用 legacy bootstrap。
fn native_homogeneous_frame_for_root(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<NativeCoordinatorFrame>, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{store, types};
    let db = db::open(&context.db_path)?;
    if store::run_scheduling_model(&db, &actor.root_run_id)? != types::SCHEDULING_HOMOGENEOUS {
        return Ok(None);
    }
    Ok(Some(native_homogeneous_decision_frame(&db, actor)?))
}

/// Only a frozen homogeneous task with no original paid request may establish
/// its first model fact. After Tick::begin, the ordinary original-model guard applies.
fn native_homogeneous_birth_frame_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<bool, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{store, types};
    if !frame.is_some_and(|frame| frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND)
        || store::run_scheduling_model(db, &actor.root_run_id)? != types::SCHEDULING_HOMOGENEOUS
        || store::scheduling_model_from_plan(db, &actor.scan_id, actor.attempt_number)?
            != types::SCHEDULING_HOMOGENEOUS
    { return Ok(false); }
    let task = store::load_task_by_root_run(db, &actor.root_run_id)
        .map_err(|error| error.reason_code.to_string())?.ok_or("root_homogeneous_task_missing")?;
    let unstarted: bool = db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM agent_root_tick_receipts WHERE root_run_id=?1)",
        [&actor.root_run_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    Ok(task.state == "created" && unstarted)
}
