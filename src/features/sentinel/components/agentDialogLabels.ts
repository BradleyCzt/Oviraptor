// Presentation-only labels for the investigation timeline. These labels never
// determine execution status, authorization, or whether a finding is proven.
export function agentDialogLabels(tr: (zh: string, en: string) => string) {
  const roleLabel = (role: string) => ({
    operator: tr("你", "You"),
    coordinator: "调查",
    homogeneous_worker: tr("同质 Worker", "Worker"),
    spa_api_mapper: "采集",
    identity_session: tr("身份会话", "Identity & session"),
    web_executor: tr("Web 执行", "Web executor"),
    deep_investigator: tr("深度调查", "Deep investigator"),
    repo_mapper: tr("仓库梳理", "Repository mapper"),
    source_analyst: tr("源码分析", "Source analyst"),
    evidence_reviewer: "检查",
  }[role] ?? role);
  const eventLabel = (eventType: string) => ({
    root_decision: tr("决策摘要", "Decision summary"),
    homogeneous_guidance: tr("引导回执", "Guidance receipt"),
    homogeneous_decision: tr("主管裁决", "Coordinator decision"),
    worker_report: tr("Worker 汇报", "Worker report"),
    directive_draft: tr("指令草案", "Directive draft"),
    user_directive: tr("用户指令", "User directive"),
    mailbox_message: tr("智能体通信", "Agent message"),
    agent_run: tr("运行状态", "Run state"),
    assignment: tr("任务分派", "Assignment"),
    review_gate: tr("证据审查", "Evidence review"),
    host_boundary_candidate: tr("主机边界 · 仅记录", "Host boundary · recorded only"),
    request_review: tr("请求核对 · 人工声明", "Request review · operator attestation"),
    administrative_closure: tr("人工结案 · 未决结果保留", "Administrative closure · unsettled results retained"),
    closure_handoff: tr("独立任务交接 · 仅创建草稿", "Independent handoff · draft creation only"),
  }[eventType] ?? eventType);
  const diagnosticLabel = (code: string) => ({
    administratively_closed_unsettled: tr("已人工结案，原执行结果与费用仍需核对", "Administratively closed; original effects and costs still require review"),
    cleanup_unconfirmed: tr("清理回执未确认", "Cleanup receipt unconfirmed"),
    paused_requires_review: tr("已暂停，等待人工核对", "Paused; review required"),
    pause_waiting_for_quiescence: tr("暂停中，等待执行线程退出与清理确认", "Pausing; waiting for worker exit and cleanup confirmation"),
    cancelled: tr("任务已取消", "Task cancelled"),
    backend_retired: tr("旧执行能力已退役", "Legacy backend retired"),
    branch_failed: tr("分支失败", "Branch failed"),
    execution_incomplete: tr("执行未完整结束", "Execution incomplete"),
    incomplete_obligations: tr("终态与未完成义务冲突", "Terminal state conflicts with obligations"),
    completed_with_gaps: tr("结束，但有覆盖缺口", "Finished with coverage gaps"),
    completed: tr("已完成", "Completed"),
    native_receipts_unavailable: tr("缺少 Native 完成回执", "Native receipts unavailable"),
  }[code] ?? code);
  const gapStepLabel = (step: string) => ({
    observe_existing_evidence: tr("复核已有证据", "Review existing evidence"),
    request_new_contract: tr("申请下一次任务的人工控制组", "Request an operator control group for a new attempt"),
    manual_review: tr("请求人工复核", "Request human review"),
  }[step] ?? step);
  const gapReasonLabel = (reason: string) => ({
    human_review_required: tr("需要人工复核；未派发目标请求", "Human review required; no target request dispatched"),
    no_new_verified_fact_in_revision: tr("本证据版本没有新增已验证事实；不会重复审核", "No new verified fact in this revision; review will not repeat"),
    proposal_is_not_a_verified_execution_contract: tr("提案不是已校验的执行合同；未派发目标请求", "A proposal is not a verified execution contract; no target request dispatched"),
    operator_approval_new_attempt_required: tr("需由操作人在新任务中登记控制组；当前未授予目标请求", "An operator must register a control group for a new attempt; no target request granted here"),
  }[reason] ?? reason);
  return { roleLabel, eventLabel, diagnosticLabel, gapStepLabel, gapReasonLabel };
}

// Labels only. A human decision or queue receipt never becomes execution proof.
export function humanReviewPresentation(tr: (zh: string, en: string) => string) {
  const reviewLabel = (kind: string) => ({
    approve: tr("已确认，等待处理", "Confirmed, awaiting handling"),
    revise: tr("修改已保存", "Revision saved"),
    reject: tr("已拒绝，理由已保存", "Rejected, reason saved"),
  }[kind] ?? tr("判断记录需核对", "Decision requires review"));
  const actionLabel = (action: { reviewDisposition: string; executionState: string; capabilityState: string }) => {
    if (action.executionState !== "not_started") return tr("执行状态需核对", "Execution status requires review");
    if (action.capabilityState === "blocked") return tr("尚未执行 · 此动作受限", "Not started · action blocked");
    return action.reviewDisposition === "coordinator_queued"
      ? tr("已交协调层 · 尚未执行", "With Coordinator · not started")
      : tr("未入队 · 尚未执行", "Not queued · not started");
  };
  const actionReasonLabel = (code: string) => ({
    human_decision_terminal: tr("本次判断不派发执行", "This decision does not dispatch work"),
    proposal_role_decomposition_required: tr("角色任务需要独立分解", "Role work requires independent decomposition"),
    proposal_reviewer_requires_frozen_candidate: tr("审查需要已冻结的候选证据", "Review requires a frozen candidate"),
  }[code] ?? tr("具体原因见回执详情", "See receipt details for the reason"));
  const decisionLabel = (decision: string) => ({
    accept: tr("符合当前方案", "Within the current plan"),
    partially_accept: tr("部分可处理", "Partially applicable"),
    defer: tr("暂缓处理", "Deferred"),
    reject: tr("不予执行", "Not executable"),
    need_confirmation: tr("等待你的确认", "Awaiting your confirmation"),
  }[decision] ?? tr("处理状态需核对", "Handling status requires review"));
  const validationLabel = (validation: string) => ({
    valid: tr("草案已解析", "Draft parsed"),
    confirmation_required: tr("确认后再核验", "Checked again after confirmation"),
    rejected: tr("已拒绝", "Rejected"),
  }[validation] ?? tr("解析状态需核对", "Parsing status requires review"));
  const effectLabel = (effect: string) => ({
    read_only: tr("只读评估", "Read-only assessment"),
    controlled_write: tr("受控写入 · 需授权", "Controlled write · authorization required"),
    irreversible_blocked: tr("不可逆动作 · 已阻止", "Irreversible action · blocked"),
  }[effect] ?? tr("影响需核对", "Impact requires review"));
  const directiveReasonLabel = (code: string) => ({
    root_human_assessment_budget_unavailable: tr("原预算不足以评估此指令，已延后处理", "Original budget cannot cover assessment; directive deferred"),
    root_human_directive_deferred: tr("协调者暂缓此指令，尚未执行", "Coordinator deferred this directive; not executed"),
    root_human_step_not_bounded: tr("评估建议超出允许范围，尚未执行", "Assessment advice exceeds the permitted scope; not executed"),
  }[code] ?? code);
  return { reviewLabel, actionLabel, actionReasonLabel, decisionLabel, validationLabel, effectLabel, directiveReasonLabel };
}

// Context labels describe saved values; unknown values remain available in details.
export function dialogContextPresentation(tr: (zh: string, en: string) => string) {
  const statusLabel = (value: string) => ({
    draft: tr("待确认", "Awaiting confirmation"), queued: tr("待执行", "Queued"),
    scanning: tr("执行中", "Running"), pending: tr("等待 / 执行中", "Pending / running"),
    completed: tr("已结束", "Finished"), completed_with_gaps: tr("已结束 · 存在覆盖缺口", "Finished · coverage gaps"),
    partial: tr("待补充验证", "Needs validation"), failed: tr("失败", "Failed"),
    paused: tr("已暂停", "Paused"), pausing: tr("正在停止", "Stopping"),
    cancelled: tr("已取消", "Cancelled"), imported: tr("已导入", "Imported"),
    frontend_recon: tr("前端解析", "Frontend recon"), routed: tr("已分流", "Routed"),
    recon_only: tr("仅完成确定性侦察", "Deterministic recon only"),
    manual_review: tr("需人工复核", "Human review required"),
    protected_stop: tr("受保护停止", "Protected stop"), deferred: tr("已延后", "Deferred"),
    persistence_failure: tr("本地记录失败 · 已停止", "Local record failed · stopped"),
    resume_incompatible: tr("续跑不兼容 · 需重新执行", "Resume incompatible · new execution required"),
  }[value] ?? tr("状态需核对", "Status requires review"));
  const reviewStatusLabel = (value: string) => ({
    pending: tr("待审核", "Awaiting review"), confirmed: tr("已确认", "Confirmed"),
    rejected: tr("已驳回", "Rejected"), insufficient_evidence: tr("需补充证据", "More evidence needed"),
    not_applicable: tr("暂无适用审核", "No applicable review"), running: tr("审核中", "Review in progress"),
    failed: tr("审核失败", "Review failed"), superseded: tr("已由新版本替代", "Superseded"),
    needs_evidence: tr("需补充证据", "More evidence needed"),
  }[value] ?? tr("审核状态需核对", "Review status requires review"));
  const stageLabel = (value: string) => ({
    cleanup: tr("清理阶段", "Cleanup stage"), branch: tr("执行分支", "Execution branch"),
    target: tr("目标处理", "Target processing"), source: tr("源码处理", "Source processing"),
    scan: tr("任务整体", "Overall task"),
  }[value] ?? tr("阶段见记录详情", "Stage in record details"));
  const nextActionLabel = (value: string) => ({
    create_independent_task: tr("另建独立任务并重新授权", "Create and authorize an independent task"),
    verify_owned_cleanup: tr("核对所属容器清理", "Verify owned container cleanup"),
    review_before_manual_resume: tr("核对范围、身份与预算后人工继续", "Review scope, identity and budget before manual resume"),
    review_scope_before_new_attempt: tr("核对范围后创建新尝试", "Review scope before a new attempt"),
    inspect_branch_receipt: tr("查看分支回执及失败证据", "Inspect branch receipts and failure evidence"),
    select_native_new_attempt: tr("核对配置后创建 Native 新尝试", "Review configuration before a new Native attempt"),
    wait_for_receipts: tr("等待本次尝试的完成回执", "Wait for this attempt's completion receipts"),
    wait_for_worker_exit: tr("等待实际执行退出；不要重放请求", "Wait for actual worker exit; do not replay requests"),
    inspect_attempt_and_targets: tr("查看尝试及目标状态", "Inspect attempt and target states"),
    review_coverage_gaps: tr("检查覆盖缺口", "Review coverage gaps"),
    none: tr("未要求后续动作", "No further action requested"),
  }[value] ?? tr("处理建议见记录详情", "See record details for the next action"));
  const timestampLabel = (value: string) => {
    // Only convert timestamps with an explicit offset; never invent a timezone.
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,9})?)?(?:Z|[+-]\d{2}:\d{2})$/.test(value ?? ""))
      return value ? tr("原时间见记录详情", "Original time in record details") : tr("时间未记录", "Time not recorded");
    const date = new Date(value);
    if (!Number.isFinite(date.getTime())) return tr("时间需核对", "Time requires review");
    return new Intl.DateTimeFormat(tr("zh-CN", "en-US"), {
      year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
      hourCycle: "h23", timeZoneName: "short",
    }).format(date);
  };
  return { statusLabel, reviewStatusLabel, stageLabel, nextActionLabel, timestampLabel };
}
