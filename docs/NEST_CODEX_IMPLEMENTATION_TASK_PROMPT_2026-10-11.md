# 给 Codex 的实现任务话术（直接复制下方代码块）

> 分工：Grok 已写深度分析；你（Codex）**只补代码与测试**（可在 DESIGN 文末追加「意图闸门」一小附录，勿另起炉灶写设计大文）。  
> 复制时从「任务：」开始，到最后一个句号结束。

```
任务：按仓库内已定稿文档与深度分析，完成 Nest「同质 Worker + 强主管」Phase 1.1 收尾：① 产品入口接通（prepare 让路不得把扫描打成失败）；② 意图产出闸门（目标未达成且无在跑/排队意图时禁止 0 意图空转）。干到完成定义满足为止，不要中途停下来问要不要继续，不要产出新的长设计文档。

必读（冲突：DESIGN > SPEC > ARCHITECTURE；增量硬规则以深度分析 §J 为准）：
1. docs/NEST_HOMOGENEOUS_CODEX_HANDOFF_DEEP_ANALYSIS_2026-10-11.md（深度分析，必读；§E.6 入口 + §J 意图闸门）
2. docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md
3. docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md
4. docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md
5. docs/NEST_HOMOGENEOUS_COORDINATOR_PHASE1_REVIEW_2026-10-11.md（§8 部分过期，以深度分析为准）
忽略：docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md

先核实再动手：
- 跑 cargo test --manifest-path src-tauri/Cargo.toml --lib homogeneous::（预期约 72 passed）。已绿的 worker.rs / delivery.rs / drain 路径不要重写。
- 控制面 Phase1.1（mailbox 消费、失败 report、reclaim、不手动 mark_started→reported）已有；你的主缺口是产品入口 + 意图闸门。

P0-A 产品入口（深度分析 §E.6）：
1) 冻结计划 scheduling_model=homogeneous_workers 时：activate 同质任务、跳过 legacy bootstrap。
2) 不得把目标打成 multi_agent_bootstrap_failed:…homogeneous_scan_legacy_bootstrap_stepped_aside。
3) 在 agent_backend.rs（或 prepare 改为 Ok 变体）特判 stepped_aside，转入同质 Root 循环：mint homogeneous-decision → 付费 tick → run_coordinator_turn → drain_for_lease。
4) 禁止修改 commands/exposure.rs、multi_agent_external_surface.rs 及任何 Asset Exposure / exposure 前端。
5) 新集成测：opt-in 不失败 + 假模型至少 decompose 或 assign+drain→某 slice reported；legacy 默认路径保持绿。

P0-B 意图产出闸门（深度分析 §J，写死）：
用语：意图 = TaskSlice + inflight assignment（assigned|running|reported，或 rejected 且可重试）；产出意图 = 本轮 decompose 新增 pending 和/或 assign；不要新造第 8 个 action。
1) 若目标未达成，且当前没有任何正在跑或排队中的意图（pending/可重试/inflight 全空）→ 本轮必须产出至少一个向目标推进的意图。产出 0 意图 = 非法：Rust 拒绝并确定性降级（至少派 1 条）。
2) 产出 0 意图允许，仅当仍有 inflight/排队，且结构化 zero_intent.reason 只能是 covered_by_inflight 或 blocked_on_inflight_output，并带 evidence_assignment_ids；缺字段/胡写 → 整单拒绝。该声明必须可审计（主管侧 = 现有 Coordinator 付费决策通道 + Rust 校验）。
3) 目标达成：accept 累积后 stop(goal_reached) 收官。
4) 状态机至少覆盖分析文档样例：第1轮无在跑必须派（如 3 条）；第2轮已有 2 条在跑可 0 新增但必须两理由之一；第3轮证据够则收官；第4+：部分失败重派、全失败换方向、超时回收后必须再补意图、reject_retry、用户引导插入（一次一事时引导可下一 tick）。
5) 可在 CoordinatorDecision 增加可选 zero_intent 对象；可增 reason_code zero_intent_illegal 或复用 degraded_path_used。同步在 DESIGN 文末加短附录「意图闸门」枚举即可。
6) 必加测：首轮 0 意图→拒/降级≥1；2 inflight+合法 blocked_on_inflight_output→允许；2 inflight 无理由→拒；reclaim 后空队列→必须补意图；goal_reached stop 收官。

硬禁止：
- 禁止平权、禁止 Worker 互聊互委派、禁止魔法扩容、禁止 Phase1 打开 target_touching/review 并行。
- 禁止发明七动作之外的主管 action 名。
- 禁止 commit/push；禁止碰用户真实库；测试只用临时库。
- 禁止复用/改写 camelCase DecisionSummary 为 snake_case。

工作方式：按深度分析 §H 的 PR-A→PR-E 推进；每块跑相关 cargo test；失败先修。

完成定义：
- P0-A、P0-B 全绿；homogeneous:: 回归全绿；legacy smoke 不红；diff 无 exposure。
- 对话里一句话汇报：改动路径 + 测试命令与结果。然后停止，交给窃蛋龙改造审核。不要自行宣布验收通过。
```
