import type { NativeScanStatus } from "../types";

type Translate = (zh: string, en: string) => string;
type Branch = NativeScanStatus["branches"][number];

// Status copy belongs to the Native run view, not the cross-feature i18n catalog.
export function createNativeRunStatusLabels(tr: Translate) {
  const statusLabel = (status: string) => ({
    pending: tr("等待结果", "Awaiting result"), prepared: tr("准备就绪", "Prepared"),
    running: tr("运行中", "Running"), scanning: tr("执行中", "In progress"),
    pausing: tr("暂停中", "Pausing"), paused: tr("已暂停", "Paused"),
    cancelled: tr("已取消", "Cancelled"), completed: tr("已完成", "Completed"),
    completed_with_gaps: tr("完成，但有覆盖缺口", "Completed with gaps"),
    partial: tr("部分完成", "Partial"), failed: tr("失败", "Failed"),
  }[status] ?? status);
  const dispatchLabel = (state?: string) => ({
    never_claimed: tr("尚未取得执行权；不会自动恢复派发", "Not yet claimed; dispatch is not automatically recovered"),
    claimed: tr("已持久记录执行权；不代表仍在运行或已经完成，不能直接重放", "Durable claim recorded; not proof of liveness or completion, no direct replay"),
    legacy_unknown: tr("历史执行缺少派发凭据；不能判断是否已经执行", "Historical dispatch receipt missing; execution may have occurred"),
    invalid_receipt: tr("派发凭据异常；需人工核对", "Invalid dispatch receipt; review required"),
  }[state ?? "legacy_unknown"] ?? tr("派发状态未知；需人工核对", "Unknown dispatch state; review required"));
  const closedWithoutDispatch = (branch: Branch) =>
    branch.status === "failed" && !!branch.report && typeof branch.report === "object"
    && "code" in branch.report && branch.report.code === "operator_closed_before_dispatch";

  function unexecutedFailureLabel(branch: Branch): string {
    const report = branch.report;
    if (branch.status !== "failed" || branch.dispatch?.state !== "never_claimed"
      || !report || typeof report !== "object" || Array.isArray(report)
      || !("executionStarted" in report) || report.executionStarted !== false
      || !("failurePhase" in report)) return "";
    if (report.failurePhase === "branch_admission" && "code" in report
      && report.code === "workbench_admission_rejected") {
      return tr("未执行·准入失败", "Not executed · admission rejected");
    }
    return report.failurePhase === "thread_spawn"
      ? tr("未执行·线程未启动", "Not executed · worker did not start") : "";
  }

  const branchStatusLabel = (branch: Branch) => closedWithoutDispatch(branch)
    ? tr("未执行·人工结束", "Not executed · operator closed")
    : unexecutedFailureLabel(branch) || statusLabel(branch.status);
  const branchDispatchDescription = (branch: Branch) => {
    if (closedWithoutDispatch(branch)) return tr("从未派发；旧尝试已结束，不可恢复。需检查配置后另行启动新尝试。", "Never dispatched; old attempt closed and cannot be recovered. Review configuration and separately start a new attempt.");
    if (unexecutedFailureLabel(branch)) return tr("本分支未开始执行，不会自动重试；其他独立分支保留各自状态。请检查失败原因，任务结束后再决定是否重试。", "This branch did not start and will not retry automatically. Independent branches retain their own state. Review the failure and decide whether to retry after the task ends.");
    return dispatchLabel(branch.dispatch?.state);
  };
  const roleLabel = (role: string) => ({
    coordinator: tr("协调智能体", "Root coordinator"), spa_api_mapper: tr("接口梳理", "API mapper"),
    web_executor: tr("Web 执行", "Web executor"), deep_investigator: tr("深入调查", "Investigator"),
    repo_mapper: tr("仓库梳理", "Repository mapper"),
    source_analyst: tr("源码分析", "Source analyst"),
    evidence_reviewer: tr("证据复核", "Evidence reviewer"), operator: tr("操作者", "Operator"),
  }[role] ?? role);
  const diagnosticLabel = (code: string) => ({
    administratively_closed_unsettled: tr("已人工结案 · 执行结果未结清", "Administratively closed · execution remains unsettled"),
    cleanup_unconfirmed: tr("清理回执未确认", "Cleanup receipt unconfirmed"),
    paused_requires_review: tr("已暂停，等待人工核对", "Paused; review required"),
    pause_waiting_for_quiescence: tr("暂停中，等待执行线程退出与清理确认", "Pausing; waiting for worker exit and cleanup confirmation"),
    cancelled: tr("任务已取消", "Task cancelled"),
    operator_closed_before_dispatch: tr("未派发尝试已人工结束", "Never-dispatched attempt closed by operator"),
    branch_failed: tr("分支失败", "Branch failed"),
    backend_retired: tr("旧执行能力已退役", "Legacy backend retired"),
    in_progress: tr("正在执行", "In progress"),
    execution_incomplete: tr("执行未完整结束", "Execution incomplete"),
    incomplete_obligations: tr("终态与未完成义务冲突", "Terminal state conflicts with obligations"),
    completed_with_gaps: tr("结束，但有覆盖缺口", "Finished with coverage gaps"),
    completed: tr("已完成", "Completed"),
    native_receipts_unavailable: tr("缺少 Native 完成回执", "Native receipts unavailable"),
  }[code] ?? code);
  const actionLabel = (action: string) => ({
    create_independent_task: tr("旧任务已封存；后续需新建独立任务并重新授权", "Old task sealed; create and authorize an independent task for further work"),
    verify_owned_cleanup: tr("核对所属容器并确认清理", "Verify owned container cleanup"),
    review_before_manual_resume: tr("核对范围、身份、预算后人工继续", "Review scope, identity and budget before manual resume"),
    review_scope_before_new_attempt: tr("核对范围后创建新尝试", "Review scope before a new attempt"),
    inspect_branch_receipt: tr("查看分支回执及失败证据", "Inspect branch receipt and failure evidence"),
    select_native_new_attempt: tr("核对配置，使用 Native 创建新尝试", "Review configuration; start a new Native attempt"),
    wait_for_receipts: tr("等待当前尝试的完成回执", "Wait for current attempt receipts"),
    wait_for_worker_exit: tr("等待实际执行退出；不能提前恢复或重放请求", "Wait for actual worker exit; do not resume or replay requests"),
    inspect_attempt_and_targets: tr("查看尝试及目标状态", "Inspect attempt and target states"),
    review_coverage_gaps: tr("检查覆盖缺口，按需创建新尝试", "Review coverage gaps; create a new attempt if needed"),
    none: tr("无需继续", "No further action"),
  }[action] ?? action);
  const obligationLabel = (kind: string) => ({
    branch_not_executed: tr("未执行分支", "Unexecuted branch"),
    cleanup: tr("清理", "Cleanup"), branch_pending: tr("分支待结束", "Branch pending"),
    branch_failed: tr("分支失败", "Branch failed"), branch_gap: tr("分支缺口", "Branch gap"),
    branch_unknown: tr("分支状态未知", "Branch status unknown"), target: tr("目标未完整结束", "Target unfinished"),
    source_gap: tr("源码缺口", "Source gap"),
  }[kind] ?? kind);

  return { actionLabel, branchDispatchDescription, branchStatusLabel, diagnosticLabel,
    obligationLabel, roleLabel, statusLabel };
}
