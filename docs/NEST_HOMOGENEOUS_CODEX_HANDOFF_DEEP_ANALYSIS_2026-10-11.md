# Nest 同质 Coordinator：交给 Codex 的深度分析（2026-10-11）

> 作者：Grok Bot｜只写文档｜HEAD `14a7875`｜核实时间 2026-10-11 约 15:53–16:00 CST  
> 性质：实现交接分析。**以当前工作区实读为准**；过期审核与代码冲突时以代码为准。  
> 追加硬规则：**意图产出闸门**（见 §E.8 / §J），必须落入实现。

---

## A. 一页结论

### 现在是什么
- **目标态**：唯一主管（Coordinator/Root）+ N 个**同质 Worker**（`homogeneous_v1`），**不是平权**多智能体。
- Worker 不互聊、不互委派、不扩权；并行度受配额与预算约束，**禁止按任务大小魔法扩容**。
- 开关：`scheduling_model`，默认 `legacy_role_pipeline`；仅冻结计划显式 `homogeneous_workers` 才走新路径。
- Phase1 **不开** `target_touching` / `review` 同质并行。

### 进度快照

| 层级 | 状态 |
|---|---|
| 审核 P0（tick→`run_coordinator_turn`、mailbox 载体、集成测） | **已达标** |
| Phase 1.1 控制面（`worker::drain_for_lease`、失败 report、supervisor reclaim、prepare 让路测） | **代码层已绿**（本机 `homogeneous::` **72 passed**） |
| **产品入口** | **未达标**：`prepare` 对同质 `Err("homogeneous_scan_legacy_bootstrap_stepped_aside")`，`agent_backend` 当 bootstrap **失败 finalize**，扫描进不了主管循环 |
| **意图产出闸门**（用户追加） | **未落地**：需 SPEC/DESIGN 增量 + Rust 护栏（见 §J）；对齐用语 = TaskSlice + inflight assignment |

### 用户目标 vs 现状

| 用户目标 | 现状 |
|---|---|
| 弱化固定角色，主管分析后分配 | 控制面有；默认 legacy |
| 同质多实例并行 A/B/C | 只读 lane 配额并行有；需 opt-in |
| 汇报→主管验收 | report/accept 有；tick 后可 drain |
| 引导经主管审核 | 闸门+四字段有 |
| 目标未达成时不能「空转 0 意图」 | **未实现**（§J） |
| 勾选 homogeneous 就能跑通一轮 | **断在 prepare 让路=失败**（§E.6） |

---

## B. 文档权威顺序

1. DESIGN：`docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md`  
2. SPEC：`docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md`  
3. ARCHITECTURE：`docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md`  
4. ANALYSIS：`docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md`  
5. PHASE1_REVIEW：`docs/NEST_HOMOGENEOUS_COORDINATOR_PHASE1_REVIEW_2026-10-11.md`（§8 部分过期）  
6. **本文** + 实现话术 `docs/NEST_CODEX_IMPLEMENTATION_TASK_PROMPT_2026-10-11.md`  

冲突：**DESIGN > SPEC > ARCHITECTURE**；封闭枚举/Schema 形状以 SPEC 为准，本文 §J 为必须落地的增量。  
**忽略：** `NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md`。

---

## C. 现状代码地图（实读符号；行号会漂）

### C.1 `agent_runtime/multi_agent/homogeneous/`

| 文件 | 职责 |
|---|---|
| `mod.rs` | 出口；注明 Phase 1.1 接线 |
| `types.rs` | SPEC 类型 / `SCHEDULING_HOMOGENEOUS` |
| `decision.rs` | `parse_decision_envelope` / `parse_payload` |
| `allowlist.rs` / `gate.rs` | 候选、refs、`allowed_actions`、`snapshot_for` |
| `apply.rs` | `run_coordinator_turn`、`apply_decision`、七动作、`mark_started`、`record_worker_report` |
| `delivery.rs` | `dispatch_slice`（mailbox kind=`homogeneous_slice_dispatch`） |
| `worker.rs` | `drain_for_lease`：领取→开跑→只读→汇报；失败也 `failed` report |
| `directive_gate.rs` | 引导初筛/改写/驳回 |
| `decompose.rs` / `machine.rs` / `pool.rs` / `store.rs` | 降级切分、machine_checks、回收、落库 |
| `journal.rs` / `prompt.rs` / `reason.rs` / `transitions.rs` / `schema.sql` | 摘要、提示、码表、迁移、DDL |
| `integration_tests.rs` 等 | 单测+集成 |

### C.2 生产接线

```text
计划 homogeneous_workers
  → prepare: set_run_scheduling_model + activate_task_for_lease
  → return Err(stepped_aside)          ← 当前
  → agent_backend: bootstrap_outcome → FAILED finalize   ← 断点

期望：
  → 特判 stepped_aside（勿改 exposure 文件）
  → 同质 Root 循环：
       coordinator_tick 无帧则 native_homogeneous_frame_for_root
       → tick_once 付费
       → native_homogeneous_apply_after_paid_tick → run_coordinator_turn
       → native_homogeneous_drain_workers → drain_for_lease
       → 【新增】意图产出闸门校验（§J）
```

| 模块 | 关键符号 |
|---|---|
| `commands/multi_agent/prepare.rs` | 同质激活后 `Err("homogeneous_scan_legacy_bootstrap_stepped_aside")` |
| `commands/agent_backend.rs` | `run_owned_agent_target`：prepare Err → finalize 失败 |
| `commands/multi_agent_external_surface.rs` | `multi_agent_bootstrap_outcome` 默认 failed——**禁止改此 exposure 相关文件**；在 backend 特判 |
| `coordinator_tick.rs` / `coordinator_tick_once.rs` | mint 帧；付费后 apply+drain |
| `coordinator_homogeneous_hook.rs` | `native_homogeneous_apply_after_paid_tick` / `native_homogeneous_drain_workers` |
| `scheduler/assignment.rs` | 同质 root 拒 legacy schedule |
| `supervisor.rs` | `reclaim_homogeneous_expired` → `pool::reclaim_expired_for_root` |

---

## D. 已绿测试（本机）

```text
cargo test --manifest-path src-tauri/Cargo.toml --lib homogeneous::
→ 72 passed; 0 failed
```

含自动消费者（不手动 `mark_started`）、失败 report、关键 GOLD、引导/ tick 入口集成测。  
**禁止无故重写**已绿的 `worker.rs` / `delivery.rs` / `apply` 核心。

---

## E. Codex 任务包（按优先级）

### E.1–E.5（控制面已完成 → 只准回归）
- mailbox 消费者、失败 report、supervisor/turn 前 reclaim、prepare 让路单测、不手动 mark_started→reported：已有且绿。

### E.6（必须新做）产品入口：让路 → 同质主管循环
1. 冻结计划 `homogeneous_workers` 时：激活同质任务、跳过 legacy bootstrap。  
2. **不得**出现 `multi_agent_bootstrap_failed:…stepped_aside`。  
3. 进入同质 Root 循环（mint→付费 tick→apply→drain）。  
4. legacy 默认路径零回归。  
5. **改 `agent_backend.rs`（或 prepare 返回 Ok 变体）特判**；**不要**改 `multi_agent_external_surface.rs`。  
6. 新集成测：opt-in 不失败 + 假模型至少 decompose 或 assign+drain→reported。

### E.7（必须新做）意图产出闸门 — 见 **§J**（全文硬规则）

### E.8 本轮明确不做
- 真实深网探测质量、`target_touching` 并行、UI、国内模型准出流水线（Phase 2）。  
- 发明第 8 主管动作（闸门做护栏，见 §J.2）。

---

## F. 明确否定
1. 不是平权。  
2. 不能魔法扩容。  
3. Worker 不互聊。  
4. Phase1 不开 target_touching 并行。  
5. 禁止发明七动作外的主管动作名（zero-intent 用护栏+可选 `zero_intent_reason` 字段，见 §J）。

---

## G. 风险与隔离
- HEAD `14a7875`；工作区大量无关脏文件（exposure 等）。  
- **黑名单**：`exposure.rs`、`multi_agent_external_surface.rs`、Asset Exposure、exposure 前端。  
- 临时库测试；不 commit/push；不碰用户真实库。

---

## H. 实现顺序与完成定义

| 步 | 内容 | 验收 |
|---|---|---|
| PR-A | 跑绿 `homogeneous::` 基线 | ≥72 passed |
| PR-B | 产品入口特判 stepped_aside→同质循环 | 不再 bootstrap_failed |
| PR-C | 假模型集成：至少一轮推进到 reported 或 decomposed | 新测绿 |
| PR-D | **意图闸门**（§J）：类型+Rust 拒绝 0 意图非法轮 + 两合法理由审核字段 + 状态机测 | §J 完成定义 |
| PR-E | legacy smoke；一句话汇报；停交窃蛋龙改造 | — |

**完成定义：**  
- [ ] E.6 入口接通 + 测绿  
- [ ] §J 闸门落地（非法 0 意图拒+降级；合法 0 意图仅两理由且结构化）  
- [ ] `homogeneous::` 全绿；legacy 不红  
- [ ] diff 无 exposure  

---

## I. 弱模型护栏
七动作 only；snake_case + deny_unknown；allowlist 选 id；Rust 重算 machine_checks；降级不假装成功；财务一次尝试不变量。

---

## J. 意图产出闸门（用户硬规则 · 必须实现）

### J.1 用语对齐（禁止另造「意图」实体乱飞）

| 用户说法 | 设计映射 |
|---|---|
| 意图 | **待推进的工作单元** = 尚未终态的 `TaskSlice`，或已 `assign` 且未 `accepted`/`cancelled`/`stopped` 的 **inflight assignment**（state ∈ `assigned`/`running`/`reported`/`rejected` 待重派） |
| 产出意图 | 本轮主管通过现有动作 **`decompose`（新增 pending slices）和/或 `assign`（把 pending/rejected 推入 inflight）** 使「可推进工作」增加 |
| 0 个意图 | 本轮结束后，相对回合开始：**没有新增 pending slice，也没有新的 assign**；且（若声称空转合法）必须带齐 §J.3 字段 |
| 目标达成 | 主管 `stop` 且 `stop_reason=goal_reached`（或任务进入 succeeded / 等价终态）；证据充分由 accept 累积 + stop 收官 |

**不新增第 8 个主管 `action`。** 闸门是对 `run_coordinator_turn` / `apply_decision` 结果的 **Rust 硬护栏**。  
若需结构化空转声明，仅允许在本轮决策信封上增加可选字段（见 J.3），或由 Rust 在「本轮未 decompose/assign」时要求模型下一短轮补字段——**优先：在同一 `CoordinatorDecision` 增加可选 `zero_intent` 对象**（增量，SPEC 级）。

### J.2 硬规则（可执行）

**规则 1 — 禁止空转停摆**  
若同时满足：
- 目标**未**达成（任务非终态，且未 `stop(goal_reached)`）；
- **当前没有任何 inflight 意图**（无 assignment 处于 `assigned|running|reported`，且无 `rejected` 且 `retry_count < max_retries` 可重派的切片也可视为「仍有可推进」——实现时：`pending`/`rejected可重试` 算「队列中」；仅当 pending/可重试/inflight **全空** 才算「没有任何正在跑或排队中的意图」）；

则本轮 **必须** 产出至少一个向目标推进的意图（至少 `decompose` 出 ≥1 slice，或对已有 pending `assign`）。  
**产出 0 意图 = 非法** → Rust **整单拒绝**本轮决策，记 reason（建议复用/增量 `degraded_path_used` 或新增 reason_code `zero_intent_illegal`，二选一写进增量，禁止再发明别的），并走 **确定性降级：至少派 1 条**（`deterministic_decompose` 取 1 个候选并允许随后 assign，或降级路径直接创建 1 个 pending slice）。

**规则 2 — 有条件允许 0 意图**  
产出 0 个意图 **允许**，但必须同时：
1. 目标仍未达成（若已达成应走 `stop`，不是 0 意图）；  
2. **存在** inflight 或排队中的意图（否则回到规则 1）；  
3. 带结构化理由，**只能**是下列枚举之一（`zero_intent_reason`）：  
   - `covered_by_inflight`：本轮本欲覆盖的方向已被现有意图覆盖；  
   - `blocked_on_inflight_output`：下一步依赖正在跑/待汇报的产出，硬派会让下游空转；  
4. 该声明 **必须交给主管审核**——此处「主管」= **Rust 护栏 + 已有 Coordinator 决策通道**（付费 tick 已评估），字段落库可审计；**禁止**自由中文当唯一依据。建议字段：

```json
"zero_intent": {
  "reason": "covered_by_inflight",
  "evidence_assignment_ids": ["asg-..."],
  "note_zh": "≤80字"
}
```

缺字段 / reason 不在枚举 / evidence 不指向本任务真实 inflight → **整单拒绝**（不得静默当合法空转）。

**规则 3 — 目标达成收官**  
证据足够 → `accept` 累积后 `stop`（`goal_reached`），任务自动收官；此后无需再产出意图。

### J.3 SPEC/DESIGN 增量（Codex 须同步改文档小段或仅代码常量+测试；文档增量可写在 DESIGN 文末「附录：意图闸门」）

新增封闭枚举（建议）：

```text
zero_intent_reason = covered_by_inflight | blocked_on_inflight_output
```

可选 reason_code 增量（若不用 degraded_path_used）：`zero_intent_illegal`。  
**仍然只有七个 action。**

### J.4 轮次状态机示例（必须写入实现者脑中）

**第 1 轮**  
- 前置：目标未达成；inflight=∅；pending=∅  
- 合法：`decompose` 出 3 条（或 decompose+assign）  
- 非法：任何「空决策 / 只 stop 非 goal / 0 意图」→ 拒+降级至少 1 条  

**第 2 轮**  
- 前置：目标未达成；已有 2 条 inflight（assigned/running）  
- 合法 A：再 `assign`/`decompose` 补方向  
- 合法 B：**0 新增**，但必须 `zero_intent.reason` ∈ {`covered_by_inflight`,`blocked_on_inflight_output`} 且 `evidence_assignment_ids` 指向那 2 条  
- 非法：0 新增且无 zero_intent / 理由胡写  

**第 3 轮**  
- 前置：汇报已 accept，覆盖足够  
- 合法：`stop` + `goal_reached` → 收官  
- 非法：再空转不 stop；或 goal 已满足却继续无意义派发（可后续优化，非本闸门最低要求）  

**第 4 轮+ 常见情况（扩写）**

| 情况 | 期望 |
|---|---|
| 部分失败（部分 reported 后 reject_retry） | 对可重试 slice `assign` 新 attempt；或 0 新增但 `blocked_on_inflight_output` 等其它仍在跑的 |
| 全部失败且 retry 耗尽 | 必须换方向：`decompose` 新 slice 或 `stop`（非 goal，如 `retry_limit_exceeded`）；**禁止**无理由 0 意图 |
| 超时回收（reclaim 后 inflight 变空） | 视同「无在跑/排队」→ **必须再补意图**（规则 1），除非直接合法 `stop` |
| 用户引导插入 | 先 `relay_directive`/`reject_directive`（可与同轮其它动作的「一次一事」冲突时：一次 tick 一事——优先完成闸门要求的推进或合法 zero_intent；引导可下一 tick） |
| 验收 `reject_retry` | 产生可重派队列 → 后续轮必须 assign 或合法 zero_intent（若仍有其它 inflight） |
| 预算耗尽 | `stop(budget_exhausted)`；不是 silent 0 意图 |
| 降级路径 | 规则 1 触发后确定性至少 1 slice，标 `degraded_path_used` |

### J.5 建议挂载点（代码）
- `apply::run_coordinator_turn` 成功 `Applied` 之后，或 `apply_decision` 事务提交前：计算 `inflight_or_queued`、`produced_intent`、`goal_reached`。  
- 非法 → 返回 Refused/触发 `deterministic_decompose`（至少 1）。  
- 合法 zero_intent → 校验字段后落库审计（可挂 task 行或独立小表；最小：写入 tick 日志 + 校验通过即可，表结构待评审）。

### J.6 闸门测试（Codex 必加）
1. 空任务首轮 0 意图 → 拒或降级出 ≥1 slice  
2. 2 条 inflight + 合法 `blocked_on_inflight_output` → 允许 0 新增  
3. 2 条 inflight + 无 zero_intent → 拒  
4. reclaim 后队列空 + 目标未达成 → 必须补意图  
5. accept 足够后 `stop(goal_reached)` → 收官  

---

## 附录：文档路径
- 话术：`docs/NEST_CODEX_IMPLEMENTATION_TASK_PROMPT_2026-10-11.md`
