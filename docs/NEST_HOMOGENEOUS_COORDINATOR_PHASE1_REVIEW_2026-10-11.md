# Nest 同质 Coordinator Phase 1 审核记录（2026-10-11）

> 审核方：Grok Bot（只读）｜ HEAD `14a7875`｜依据：DESIGN > SPEC > ARCHITECTURE  
> 未改业务代码、未 commit。测试命令见文末。

## 1. 总判

**Phase 1：部分达标（控制面 + 单测基本完成；生产闭环未接通）。**

不能宣布 Phase 1 Done：`run_coordinator_turn` 仅被测试夹具调用，Root 付费 tick → 解析 → `apply_decision` → Worker 执行/mailbox → `record_worker_report` 的现网接线仍缺。

## 2. 改动范围与 exposure

- 新建目录：`src-tauri/src/agent_runtime/multi_agent/homogeneous/`（整模块未跟踪）
- 接线相关已改：`multi_agent/mod.rs`、`scheduler/assignment.rs`、`prepare.rs`、`coordinator_frame.rs`、`coordinator_rust_policy.rs`、`coordinator_human_hook.rs`、`web_executor_adapter.rs`、`directive/confirmation.rs`、`db_initialize_columns.rs`、`db_schema.rs`（include schema.sql）、`agent_tests_homogeneous_frame.rs`
- **exposure**：工作区有 `exposure.rs` / `multi_agent_external_surface.rs` / Vue exposure 等脏改动，但**文件内无 `homogeneous` 引用**，视为并行会话产物，**非本特性引入**。实现方仍须遵守黑名单，diff 不得再碰。

## 3. DESIGN §0.1 清单对照

| 项 | 状态 | 路径 / 说明 |
|---|---|---|
| `homogeneous/` 模块 | **有** | `allowlist/apply/decision/decompose/directive_gate/gate/machine/store/types/…`，`mod.rs` 已 `pub(crate) mod homogeneous` |
| `scheduling_model` 默认 legacy | **有** | `db_initialize_columns.rs` ≈496–497（agent_runs 默认 `legacy_role_pipeline`）；`prepare.rs` ≈129–145；测 `scheduling_model_comes_from_the_frozen_scan_plan_and_defaults_to_legacy` |
| 新表 + assignments 增列 | **有** | `homogeneous/schema.sql`；`db_schema.rs` include；`db_initialize_columns.rs` ≈522–535（slice_id/scheduling_model/worker_profile_id/homogeneous_worker_id/attempt + 索引） |
| 仅 `read_only_analysis` 并行 | **有** | `apply.rs` ≈348–353 非只读 lane 拒派；配额 `ensure_quota` 只读默认 parallel=3 |
| dedupe 含 slice_id | **有** | `store.rs` ≈65–73；测 `dedup_key_contains_slice_id_so_three_slices_become_three_assignments` |
| 七动作严格解析 + 落库 | **有（库内）** | `decision.rs` + `apply.rs`；**无生产调用方**（见缺口） |
| 引导闸门四字段 + 驳回不 incomplete | **部分** | `directive_gate.rs`；确认后 `screen`（`confirmation.rs` ≈204）；`web_executor_adapter.rs` ≈16–28、146–160；主管 `reject_directive` 仍依赖未接线的 `run_coordinator_turn` |
| 确定性降级 | **有（库内）** | `run_coordinator_turn` streak→`deterministic_decompose`；测 GOLD-19 |
| frame kind | **部分** | `coordinator_frame.rs:343`；`coordinator_rust_policy.rs` 准入/allowed_actions；**无 frame 生产者把模型输出送进 `run_coordinator_turn`** |
| 关键 GOLD 测试 | **有且全绿** | 见 §5 |
| 不碰 exposure | **本特性未碰** | 工作区另有 exposure 脏文件，勿再改 |

## 4. SPEC 要点抽查

| 要求 | 结论 |
|---|---|
| 七动作封闭集合 | 通过（`types`/`reason`/`golden_tests::closed_sets`） |
| snake_case + deny_unknown | 通过（decision_tests GOLD-24/25） |
| 引导四字段 | 通过（reject_directive 缺字段拒；GOLD-13..15） |
| 驳回不得 incomplete | 代码路径有（`human_deferral_answered_by_reject`）；legacy 仍 incomplete |
| dedupe 含 slice_id | 通过 |
| 默认 legacy | 通过 |
| 只读 lane 才并行 | 通过 |

已知有意偏差（schema 注释已写）：`agent_acceptance_verdicts` UNIQUE 带 `attempt`（因同 slice 重跑复用 assignment 行）。可接受，须在设计偏差表留档。

## 5. 黄金集与测试结果

关键 GOLD（02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25）均有对应测试函数；非关键 01/04/08/10/16/21/22 也有。

```text
cargo test --manifest-path src-tauri/Cargo.toml --lib homogeneous::
→ 66 passed; 0 failed（约 3.4s）

另抽测 wiring：
scheduling_model_comes_from_the_frozen_scan_plan_and_defaults_to_legacy … ok
homogeneous_frame_kind_has_its_own_step_and_stays_out_of_legacy_inventory … ok
homogeneous_action_gate_only_admits_actions_the_rust_gate_allows … ok
legacy_scheduler_refuses_to_dispatch_on_a_homogeneous_coordinator_run … ok
```

## 6. 缺口与缺陷（优先级）

### P0（Phase 1 完成前必须做）

1. **接通主管一轮生产路径**  
   - 现状：`homogeneous::apply::run_coordinator_turn` 只被 `homogeneous/test_fixture.rs` 调用。  
   - 需要：在 Root tick / homogeneous-decision frame 的模型返回处调用它；付费发票与现有 tick 财务不变量对齐（DESIGN §7）；`advisoryOnly` 护栏之后必须有真实 `apply`。  
   - 相关已有碎片：`coordinator_rust_policy::native_homogeneous_action_gate`（仍写 advisoryOnly）。

2. **assign 后的执行载体**  
   - `apply.rs` ≈510 自承：Phase 1 未接通 child run / mailbox 下发。  
   - 需要：assign 成功后创建/复用 child、经现有 mailbox（不改 authority 路由）投递 slice 任务；完成回调 `record_worker_report` → assignment/slice → `reported`。

### P1（强烈建议同一迭代收尾）

3. **homogeneous-decision frame 生产者**：何时 mint frame、塞 allowlist/prompt（`homogeneous/prompt.rs`）、保证一次一事。  
4. **引导闭环 E2E**：gated 表 awaiting → 主管 `reject_directive`/`relay_directive` 经 `run_coordinator_turn` 落库 → 前端/聊天能读四字段（后端落库已有）。  
5. **租约到期回收**：`pool::reclaim_expired_worker` 与现网 heartbeat/expiry 接线（mod.rs 已标未接）。  
6. **回归**：跑一轮 legacy 相关 smoke，确认默认路径未强制 homogeneous。

### 非本特性但需知

- 工作区 exposure 大量未提交改动：继续实现时 **禁止** 纳入 homogeneous PR 范围。

## 7. 下一步（给 Qwen 的目标）

完成 P0-1 + P0-2，并补至少 1 个「临时库 + 假模型文本」的接线集成测试（decompose→assign→report→accept 或 reject_directive 不 incomplete）。完成后再交审；在此之前不得勾选 DESIGN §0.1 全部完成。


---

## 8. 复审（2026-10-11 上午，对照上文 P0）

复审时间：2026-10-11 约 10:03 CST｜HEAD 仍为 `14a7875`｜只读 + 跑测｜未改业务代码、未 commit。

### 8.1 总判（复审）

**对照上次 P0 三条：达标。**  
控制面生产接线（付费 tick → `run_coordinator_turn`、assign → mailbox 载体、集成测）已落地且测试绿。

**宣布 DESIGN §0.1「Phase 1 Done」前仍建议收尾**（非上次 P0 原文，但 `mod.rs` 自承 + 产品完整性）：

1. 尚无消费 `homogeneous_slice_dispatch` 的独立 Worker 运行时循环（`mark_started` / `record_worker_report` 现由测试或外部调用；载体与汇报 API 已通）。
2. heartbeat 侧到期扫描尚未接到现网（`pool::reclaim_expired_for_task` 已在 `run_coordinator_turn` 前触发，测 `an_expired_worker_carrier_is_reclaimed_before_the_next_decision` 绿）。
3. prepare 阶段对同质任务的 legacy bootstrap dispatch「让路」仍标未完成（`homogeneous/mod.rs` 注释）。

上述 1 可明确划入 Phase 2（真实只读分析执行），或作为 Phase 1.1 最小消费者；2/3 建议在宣布 Done 前补齐或写进已知限制。

### 8.2 P0 三条核对

| ID | 要求 | 结论 | 证据 |
|---|---|---|---|
| P0-1 | Root 付费 tick / homogeneous-decision 返回路径调用 `run_coordinator_turn`（非仅夹具） | **有** | `coordinator_tick_once.rs` ≈389–396：`homogeneous_text` 存在时在发票发布后调用 `native_homogeneous_apply_after_paid_tick`；该函数在 `coordinator_homogeneous_hook.rs:27–33` 内调用 `apply::run_coordinator_turn`。`coordinator_tick.rs:14–18` 无帧时 mint `native_homogeneous_frame_for_root`。集成测 `the_paid_tick_return_path_is_the_only_side_effect_entry` |
| P0-2 | assign 后 mailbox 下发；Worker `record_worker_report` → reported | **有** | `apply.rs` ≈518–520 调用 `delivery::dispatch_slice`；`delivery.rs:51–149` 建 child run/attempt 并 `mailbox::send`（kind=`homogeneous_slice_dispatch`）。`apply.rs:541+` `record_worker_report` 推到 `reported`。集成测 `three_read_only_dispatches_create_live_carriers_and_accept_settles_only_itself` 断言载体+消息+reported。**残留**：无自动消费该 mailbox kind 的 Worker 循环 |
| P0-3 | 集成测试（假模型链路）且绿 | **有** | `homogeneous/integration_tests.rs` 三测：三角色 dispatch+accept；awaiting 引导 relay/reject 经付费入口且任务不终态；tick 返回路径唯一副作用入口 |

### 8.3 P1 进度

| 项 | 状态 | 证据 |
|---|---|---|
| frame mint | **有** | `native_homogeneous_frame_for_root` / `native_homogeneous_decision_frame`（`coordinator_homogeneous_hook.rs`）；`coordinator_tick.rs` 无帧时 mint |
| 引导 awaiting_coordinator | **有** | 集成测 `an_awaiting_directive_is_relayed_and_rejected_through_the_paid_tick_entry`；`a_projected_rejection_continues_the_round_instead_of_marking_it_incomplete` 绿 |
| pool 回收 | **部分→基本有** | `run_coordinator_turn` 开头 `pool::reclaim_expired_for_task`；单测绿。heartbeat 全局扫描仍未接（mod 注释） |
| legacy smoke | **有** | `scheduling_model_defaults_to_legacy_*`、`legacy_scheduler_refuses_*`、`legacy_lane_occupancy_*`、golden `legacy_lane_table_*` |

### 8.4 exposure

- `exposure.rs` mtime 2026-10-09，早于本次 homogeneous 接线（09:40）；文件内仍无 `homogeneous` 引用。
- **结论：本特性未误碰 exposure**；工作区 exposure 脏文件仍属并行会话，实现方勿动。

### 8.5 测试结果（复审）

```text
cargo test --manifest-path src-tauri/Cargo.toml --lib homogeneous::
→ 70 passed; 0 failed（约 3.7s）
  含 integration_tests 全部 3 个

另：
a_projected_rejection_continues_the_round_instead_of_marking_it_incomplete … ok
scheduling_model_defaults_to_legacy_and_only_activates_on_explicit_opt_in … ok
an_expired_worker_carrier_is_reclaimed_before_the_next_decision … ok
```

### 8.6 建议

- **上次 P0：可关闭。**
- **宣布 Phase1 Done**：建议先书面确认「Worker mailbox 消费者」归 Phase 1.1 还是 Phase 2；并补 prepare bootstrap 让路或记入已知限制。
- **Phase 2 建议方向**（总览）：`target_touching` 受控并发、真实只读/触达执行质量、UI 切片进度、弱模型黄金集对准出国内 API。
