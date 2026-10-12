# Nest 同质 Worker + 强主管：设计定稿（实现依据）

> 版本：2026-10-10 ｜ 作者：Grok Bot ｜ 性质：**设计定稿**
>
> **实现以本文 + `docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md` 为准。**
> 总览 `NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md` 仅作背景。
> 现状 `NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md` 仅作对照。
>
> **冲突裁定：DESIGN（本文）> SPEC_FOR_QWEN > ARCHITECTURE。** 封闭枚举/JSON Schema 形状仍以 SPEC 为准；本文负责「落到哪个文件/函数/表」。
> 曾由错误话术产出的 `docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md` **不作实现依据**，仅可作核实线索；与本文冲突时以本文为准。
>
> 只写文档，不改业务代码，不 commit。

---

> **Codex 交接（深度分析）**：[`NEST_HOMOGENEOUS_CODEX_HANDOFF_DEEP_ANALYSIS_2026-10-11.md`](NEST_HOMOGENEOUS_CODEX_HANDOFF_DEEP_ANALYSIS_2026-10-11.md)；实现话术：[`NEST_CODEX_IMPLEMENTATION_TASK_PROMPT_2026-10-11.md`](NEST_CODEX_IMPLEMENTATION_TASK_PROMPT_2026-10-11.md)。含产品入口缺口与意图产出闸门（§J）。

## 0. Phase 1 范围（Qwen 第一期必须做完的边界）

### 0.1 做（勾选完成即 Phase 1 Done）

- [x] 新建 `agent_runtime/multi_agent/homogeneous/` 模块（见 §3）：decision 解析、allowlist、apply、machine_checks、deterministic_decompose、directive_gate、stop
- [x] 任务级开关 `scheduling_model ∈ {legacy_role_pipeline, homogeneous_workers}`，**默认 legacy**；仅当显式 homogeneous 时走新路径
- [x] 新表：`agent_homogeneous_tasks`、`agent_task_slices`、`agent_lane_slots`、`agent_lane_quotas`、`agent_acceptance_verdicts`、`agent_human_directive_gated`；`agent_assignments` 增列（见 §4）
- [x] **仅** `read_only_analysis` lane 配额并行（同 target 多 slice 多 Worker）
- [x] 新路径 dedupe key **必须含 `slice_id`**
- [x] 主管七动作：严格 snake_case JSON 解析 + Rust 校验 + 状态迁移落库（SPEC §2）
- [x] 引导闸门：规则初筛复用 `interpret()`；主管可 `relay_directive`/`reject_directive`；驳回四字段回用户；**驳回不得**把本轮打成 `incomplete`
- [x] 确定性降级：schema/硬拒连续失败后禁止再问模型（SPEC §4）
- [x] 测试：至少覆盖黄金集关键用例 GOLD-02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25（100%）；其余尽量覆盖；路径见 §10
- [x] frame kind 新增（仅 homogeneous 路径）：`assess:homogeneous_decision` 等（见 §5.4）

> 勾选说明（实现者，2026-10-11）：以上十条按 §11 PR-A→PR-G 实现并逐条留有测试锚点，证据清单见交付汇报（homogeneous 自测 72/72、GOLD 逐条对 SPEC §7 期望、封闭集合脚本比对、DESIGN §4 DDL 逐列比对）。与本文的四条实现偏差已单独上报。
>
> 接线补记（实现者，2026-10-11 第二轮生产接线）：Root 付费 tick 的 producer（`coordinator_homogeneous_hook::native_homogeneous_frame_for_root` → tick 返回路径 `native_homogeneous_apply_after_paid_tick` → `apply::run_coordinator_turn`）、`assign` 之后的 child run / attempt / mailbox 载体（`homogeneous/delivery.rs`，不改 `mailbox/authority.rs` 路由）与到期回收（每轮决策前 `pool::reclaim_expired_for_task`）均已接通，并各有端到端/集成测（homogeneous 自测 78/78）。**仍未接**的两处（当时）：主管循环 legacy dispatch 阶段对同质任务的让路、`scheduler/heartbeat.rs` 侧的到期扫描调用方。
>
> Phase 1.1 补记（实现者，2026-10-11 第三轮）：REVIEW §8.1 三条收尾项已接通——(1) `homogeneous/worker.rs::drain_for_lease` 为最小 Worker 消费者：走现网收方 API（`mailbox::deliver_expected` + `acknowledge`）领取 `homogeneous_slice_dispatch`，`mark_started` → 切片 capability_lease 内的只读执行（只读现网证据节点 + DESIGN §4.8 别名；无模型调用、无目标访问、不碰 target_touching/review）→ `record_worker_report` 推到 `reported`；工具未授权 / lane 未开放 / 载体到期 / 用户取消 / 范围内无证据都写出 `terminal_hint='failed'` 的可复核汇报，reason 一律取 SPEC §1.1 既有枚举，绝不卡在 `running`；挂在主管付费 tick 结算后的同一尾巴上（`coordinator_tick_once`，legacy root 零副作用）。(2) 到期回收同时挂在 `supervisor::sweep` 心跳（`pool::reclaim_expired_for_root`），并把 `sweep` 的 due/children 谓词限定为 `scheduling_model='legacy_role_pipeline'`——旧提款写者要求 `agent_lane_leases` 与旧状态机，同质行两者都没有（这就是 `assignment_expiry_original_state_conflict` 的来源）。(3) prepare 对显式声明 homogeneous_workers 的冻结计划直接让路（`homogeneous_scan_legacy_bootstrap_stepped_aside`），不建固定角色 child、不占旧 lane、不付 bootstrap 轮；默认 legacy 计划的 prepare 路径逐字不变（测 `a_legacy_plan_still_runs_the_bootstrap_dispatch_unchanged`）。homogeneous 自测 83/83。**仍未做**：同质扫描的端到端编排（评审/终态投影/UI 切片进度）与模型驱动的只读分析质量，属 Phase 2。**验收结论仍由「窃蛋龙改造」裁定，本补记不构成验收通过。**

### 0.2 不做（Phase 1 禁止）

- 不改 Asset Exposure / `multi_agent_external_surface` / 任何 exposure 业务文件
- 不打开 `target_touching` / `review` lane 的同质并行
- 不删除、不改写 legacy 固定角色管线的默认行为
- 不引入第 8 个主管动作 / 新角色 / 新工具名 / 新状态名 / 新 reason_code（封闭集合见 SPEC §1.1）
- 不 commit / push；不碰用户真实数据库；测试只用临时库
- 不替代 EvidenceReviewer 的漏洞确认权（confirmed finding 门禁保持现状）

---

## 1. 目标 / 非目标 / 与现状差异

### 1.1 目标（用户 5 点 + 弱模型）

1. 弱化固定角色 → 同质 Worker（`homogeneous_v1`）+ 切片差异  
2. 主管先分解再分配  
3. 同质多实例并行（Phase 1：只读 lane）  
4. 汇报 → 主管验收 → 再派 / 重做 / 停止  
5. 聊天引导经主管审核：合理转发（可改写），不合理反馈用户  
6. 弱模型可控：结构化决策 + Rust 硬护栏 + 降级

### 1.2 非目标

平权互委派；Worker 互聊；用户直达 Worker；漏洞规则本身；常驻几十 agent；第一期不动 exposure。

### 1.3 与现状差异（实现相关）

| 点 | 现状 | Phase 1 目标 | 证据 / 对接 |
|---|---|---|---|
| 拓扑 | 固定角色星型 + 确定性管线 | 主管 + N 同质 Worker | 分析文档；`run_owned_agent_target` |
| lane | PK `(scan,attempt,target,lane)` 互斥 | 新表 `agent_lane_slots` 按 `(root_run,lane,slice_id)`；legacy 表不动 | `db_schema.rs` ≈1295–1303；`scheduler/assignment.rs` `agent_lane_occupied` |
| dedupe | `role:trigger:target:rev` 无 slice | homogeneous：`profile:trigger:target:rev:slice_id` | `scheduler/assignment.rs` ≈72–81 |
| 切片 | 有 `task_slice_json` 内容、无 `slice_id` 身份 | 新表 `agent_task_slices` + assignment.slice_id | `db_schema.rs` `task_slice_json` |
| 引导驳回 | `root_human_directive_deferred` → incomplete | reject 回用户，任务继续 | `coordinator_human_hook.rs`；`web_executor_adapter.rs` |
| 主管输出 | camelCase DecisionSummary + 固定 step | 新 snake_case `CoordinatorDecision`，不复用 DecisionSummary | SPEC §1.8；勿改旧 tick 决策结构 |

---

## 2. 模块划分与文件级改动清单

### 2.1 新建（Phase 1）

| 路径 | 职责 |
|---|---|
| `src-tauri/src/agent_runtime/multi_agent/homogeneous/mod.rs` | 模块出口 |
| `.../homogeneous/types.rs` | 与 SPEC 对齐的 Rust 类型（serde `rename_all="snake_case"`, `deny_unknown_fields`） |
| `.../homogeneous/decision.rs` | `parse_coordinator_decision`；剥 markdown 围栏；reason_code |
| `.../homogeneous/allowlist.rs` | 构造六类候选 + basis_hash |
| `.../homogeneous/gate.rs` | 当前允许的 action 子集 |
| `.../homogeneous/apply.rs` | **唯一副作用入口** `apply_decision` |
| `.../homogeneous/actions/{decompose,assign,accept,reject_retry,stop,relay_directive,reject_directive}.rs` | 七动作校验+落库（可合并为少文件，但逻辑分离） |
| `.../homogeneous/machine.rs` | `recompute_machine_checks` |
| `.../homogeneous/decompose.rs` | `deterministic_decompose` + `scopes_overlap` |
| `.../homogeneous/directive_gate.rs` | G0–G6；复用 `directive::interpretation::interpret` |
| `.../homogeneous/store.rs` | 新表 CRUD |
| `.../homogeneous/schema.sql` 或迁移片段 | DDL |
| `.../homogeneous/tests/` 或 `homogeneous_*_tests.rs` | 黄金集映射测 |

在 `agent_runtime/multi_agent/mod.rs` 增加：`pub(crate) mod homogeneous;`

### 2.2 修改（现有文件，最小侵入）

| 文件 | 改法 |
|---|---|
| `src-tauri/src/db_schema.rs` | 追加 Phase 1 新表 DDL（或经 migrate 路径） |
| `src-tauri/src/db_migrate.rs`（若存在 ensure_column） | `agent_assignments` 增列幂等 |
| `agent_runtime/multi_agent/scheduler/assignment.rs` | 当 `scheduling_model=homogeneous_workers`：走 slice 配额占用 + 含 slice_id 的 dedupe；**legacy 分支原样** |
| `commands/agent_native/coordinator_frame.rs` | 注册新 frame kind → step 映射（仅新 kind） |
| `commands/agent_native/coordinator_rust_policy.rs` | 新 kind：allowed_actions 子集校验，不替换旧 step 相等逻辑 |
| `commands/agent_native/coordinator_human_hook.rs` | homogeneous 路径：驳回/延期不 → incomplete；接入 directive_gate |
| `commands/agent_native/web_executor_adapter.rs` | 捕获 `root_human_directive_deferred` 时：若 scheduling_model=homogeneous 则继续轮次并投影反馈 |
| `commands/agent_native/directives.rs` / `directive/confirmation.rs` | 确认后写入 gated 表 state=`awaiting_coordinator`（待核实挂钩点） |
| `commands/agent_backend.rs` 或 `commands/multi_agent/prepare.rs` | 任务创建时写入 `scheduling_model`；默认 legacy |

### 2.3 明确不碰（Phase 1）

- `commands/multi_agent/multi_agent_external_surface.rs` 及一切 Asset Exposure / Qoder exposure 文档对应业务代码  
- Source 专用管线大改（Mapper v15 等）  
- 用户真实 DB / CAS / asset 目录  

### 2.4 可选只读 IPC（可第二期）

- `list_homogeneous_task_progress(scan_id, attempt)`：切片进度 + 裁决理由投影  
Phase 1 可不做 UI；若做只读命令，禁止暴露「手动注入主管决策」。

---

## 3. 内部调用链（七动作）

### 3.1 主管一轮（homogeneous 路径）

```text
Root 持有 CoordinatorLease
  → allowlist::snapshot(task_id)           // 六类候选 + remaining_budget
  → gate::allowed_actions(task_id)         // 当前可执行动作子集
  → 组装 frame(kind=assess:homogeneous_decision) + 付费 tick（复用现有 tick 财务骨架；决策体用新 parser）
  → 模型返回 text
  → decision::parse_coordinator_decision(text, allowlist)
       失败 → 记 reason；计数；达阈 → decompose::deterministic_decompose / PauseForUser
       成功 → apply::apply_decision(decision, lease)
                → 分发到 actions::*
                → 校验清单（SPEC 各动作 Rust 清单）
                → 单事务落库 + 状态迁移
```

**注意：** 旧 `DecisionSummary`（camelCase）**不要复用**。新决策结构独立；财务发票/publication 尽量复用现有 Root tick 收据表（待核实具体表名，实现时读 `agent_root_tick_receipts` 相关模块）。

### 3.2 各动作谁调谁

| action | apply 内调用 | 调度/旁路 |
|---|---|---|
| `decompose` | `store.insert_slices`；task `created→decomposed`；`scopes_overlap` 全对检查 | 不调 schedule_child |
| `assign` | 预算预留；`schedule_child`（homogeneous 分支）或 thin wrapper `schedule_homogeneous_worker`；lane_slots 占用；worker idle→busy | mailbox 下发 assignment（复用现有 envelope，待核实 kind 名） |
| `accept` | `machine::recompute` 必须全 true 且与 payload 一致；写 `agent_acceptance_verdicts`；slice/assignment→accepted；释放 worker/slot | 不自动 publish finding |
| `reject_retry` | recompute；retry_count 检查；assignment→rejected_retry；slice→rejected；释放 worker；等下次 assign | 新 attempt，禁止原地续命 |
| `stop` | 无 running（Phase 1 硬拒）；写 stop_reason；未终态 slice→cancelled；task 终态 | 撤权走现有 lifecycle |
| `relay_directive` | `directive_gate::validate_rewrite`（再跑 interpret）；写 gated 表；mailbox/约束写入目标 worker 上下文 | 禁止新 URL |
| `reject_directive` | 写四字段；directive→rejected；**不**返回 incomplete | 聊天投影更新 |

### 3.3 Worker 汇报闭环

```text
Worker 完成 → mailbox execution_result / 现有 specialist received 路径（待核实选用哪条）
  → homogeneous 适配器把结果规范成 WorkerReport（SPEC §1.5）
  → assignment.state = reported；slice = reported
  → 下一轮主管 tick 的 allowlist 出现该 assignment_id → 只能 accept | reject_retry | stop
```

---

## 4. 数据表 / 字段变更（DDL 草案）

> 全部草案，实现时经迁移执行；命名 `agent_*`。逻辑名对齐总览：task_slice / assignment / acceptance_verdict / human_directive_gated。

### 4.1 `agent_homogeneous_tasks`

```sql
CREATE TABLE IF NOT EXISTS agent_homogeneous_tasks (
  task_id TEXT PRIMARY KEY CHECK(task_id GLOB 'tsk-*'),
  root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  scan_id TEXT NOT NULL,
  attempt_number INTEGER NOT NULL,
  target_key TEXT NOT NULL,
  scheduling_model TEXT NOT NULL DEFAULT 'homogeneous_workers'
    CHECK(scheduling_model IN ('legacy_role_pipeline','homogeneous_workers')),
  state TEXT NOT NULL DEFAULT 'created'
    CHECK(state IN ('created','decomposed','running','partial_success',
                    'succeeded','failed','cancelled','stopped')),
  evidence_revision_base INTEGER NOT NULL DEFAULT 0,
  allowed_parallel INTEGER NOT NULL DEFAULT 3,
  degraded INTEGER NOT NULL DEFAULT 0,
  degrade_reason_code TEXT NOT NULL DEFAULT '',
  schema_fail_streak INTEGER NOT NULL DEFAULT 0,
  hard_reject_streak INTEGER NOT NULL DEFAULT 0,
  decided_by_lease_epoch INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(root_run_id)
);
```

### 4.2 `agent_task_slices`

```sql
CREATE TABLE IF NOT EXISTS agent_task_slices (
  slice_id TEXT PRIMARY KEY CHECK(slice_id GLOB 'slc-*'),
  parent_task_id TEXT NOT NULL REFERENCES agent_homogeneous_tasks(task_id) ON DELETE CASCADE,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('path_prefix','host','asset_id','source_type')),
  scope_value TEXT NOT NULL CHECK(length(scope_value) BETWEEN 1 AND 512),
  evidence_revision_base INTEGER NOT NULL DEFAULT 0,
  lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
  worker_capability_profile_id TEXT NOT NULL DEFAULT 'homogeneous_v1'
    CHECK(worker_capability_profile_id = 'homogeneous_v1'),
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK(state IN ('pending','assigned','in_progress','reported','accepted','rejected','cancelled')),
  max_retries INTEGER NOT NULL DEFAULT 2 CHECK(max_retries BETWEEN 0 AND 3),
  retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count BETWEEN 0 AND 3),
  decomposition_model TEXT NOT NULL DEFAULT 'coordinator'
    CHECK(decomposition_model IN ('coordinator','deterministic')),
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  CHECK(retry_count <= max_retries)
);
CREATE INDEX IF NOT EXISTS idx_task_slices_task_state ON agent_task_slices(parent_task_id, state);
```

### 4.3 `agent_assignments` 增列（幂等）

```sql
-- ensure_column 风格
ALTER TABLE agent_assignments ADD COLUMN slice_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_assignments ADD COLUMN scheduling_model TEXT NOT NULL DEFAULT 'legacy_role_pipeline';
ALTER TABLE agent_assignments ADD COLUMN worker_profile_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_assignments ADD COLUMN homogeneous_worker_id TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS ux_homo_slice_attempt
  ON agent_assignments(slice_id, attempt_number)
  WHERE scheduling_model = 'homogeneous_workers' AND slice_id <> '';
```

状态列：现状无 CHECK，homogeneous vocabulary（SPEC）与 legacy vocabulary **共存同一列**，靠 `scheduling_model` 隔离。读写路径必须先判 model，禁止混用。

### 4.4 lane 配额（不改旧 `agent_lane_leases` PK）

```sql
CREATE TABLE IF NOT EXISTS agent_lane_quotas (
  root_run_id TEXT NOT NULL,
  lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
  max_parallel INTEGER NOT NULL DEFAULT 1 CHECK(max_parallel BETWEEN 1 AND 16),
  PRIMARY KEY(root_run_id, lane)
);
CREATE TABLE IF NOT EXISTS agent_lane_slots (
  root_run_id TEXT NOT NULL,
  lane TEXT NOT NULL,
  slice_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  acquired_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  released_at TEXT NOT NULL DEFAULT '',
  PRIMARY KEY(root_run_id, lane, slice_id)
);
```

Phase 1：`read_only_analysis` 默认 `max_parallel=3`；`target_touching`/`review` 保持 1 且 homogeneous 路径拒绝并行扩量。

旧 `agent_lane_leases`：**legacy 继续用**；homogeneous **不写入**该表（避免 UNIQUE(assignment_id) 与互斥 PK 卡住多 slice）。

### 4.5 `agent_acceptance_verdicts`

```sql
CREATE TABLE IF NOT EXISTS agent_acceptance_verdicts (
  verdict_id TEXT PRIMARY KEY CHECK(verdict_id GLOB 'vrd-*'),
  assignment_id TEXT NOT NULL,
  slice_id TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('accept','retry','reject')),
  machine_checks_json TEXT NOT NULL,
  model_judgement_json TEXT NOT NULL DEFAULT '',
  reason_codes_json TEXT NOT NULL,
  evidence_refs_json TEXT NOT NULL,
  decided_by_lease_epoch INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(assignment_id, verdict) -- 同一 assignment 同 verdict 不重复；retry 后新 assignment
);
```

### 4.6 `agent_human_directive_gated`

```sql
CREATE TABLE IF NOT EXISTS agent_human_directive_gated (
  directive_id TEXT PRIMARY KEY CHECK(directive_id GLOB 'dir-*'),
  parent_task_id TEXT NOT NULL,
  source_user_directive_id TEXT NOT NULL DEFAULT '', -- 链到现状 agent_user_directives.id
  raw_text_redacted TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'received','rule_screened','awaiting_coordinator','relayed','rewritten','rejected','applied')),
  rule_screen_result TEXT NOT NULL CHECK(rule_screen_result IN ('pass','need_confirm','reject')),
  rule_reason_codes_json TEXT NOT NULL DEFAULT '[]',
  coordinator_action TEXT NOT NULL DEFAULT '', -- relay|rewrite|reject|''
  rewritten_text TEXT NOT NULL DEFAULT '',
  target_worker_id TEXT NOT NULL DEFAULT '',
  broadcast_constraint TEXT NOT NULL DEFAULT '',
  reason_code TEXT NOT NULL DEFAULT '',
  reason_zh TEXT NOT NULL DEFAULT '',
  suggestion_zh TEXT NOT NULL DEFAULT '',
  user_feedback_zh TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
```

### 4.7 Worker 实例

Phase 1 **可不新建**独立 worker 表：用 `agent_assignment_attempts.worker_id` 作为身份（现状已有）。若需 SPEC 的 idle/busy 投影，可加轻量表 `agent_worker_instances`（UNIQUE attempt_id），仅调度事务写入——**推荐 Phase 1.1**；Phase 1 可用 assignment 状态推导 busy/idle。

### 4.8 ID 与证据别名

- 契约层 id pattern 见 SPEC（`tsk-/slc-/asg-/wrk-/rpt-/vrd-/dir-/dec-/evd-`）。  
- 现状证据节点多为裸 64-hex：**契约层可用 `evd-` + 截断/映射表**，内部存真实 id；**不得放宽 SPEC pattern**。映射表可 `agent_evidence_aliases(evd_id, internal_id)`（Phase 1 若来不及，accept 时允许 evidence_refs 暂用内部 id 并在 machine_checks 标明——但黄金集仍按 SPEC；优先做别名）。

### 4.9 工具名

SPEC 示例常用 `http.get`；仓库真实工具串待核实（如 `snapshot.read`、`evidence.read`、`public_surface_get` 等）。**规则：allowlist 由 Rust 按注册表给出真实串；模型只能选 id；测试 fixture 用真实串。** SPEC 示例中的 `http.get` 在文档层保留，代码映射层翻译。

---

## 5. 对接点：现状 → 目标 → 具体改法

### 5.1 lane lease

| | |
|---|---|
| 现状 | `agent_lane_leases` 互斥；`schedule_child` 内 `ON CONFLICT DO NOTHING` → `agent_lane_occupied` |
| 目标 | homogeneous 用 `agent_lane_slots` + `agent_lane_quotas` |
| 改法 | 在 `scheduler/assignment.rs`（或新建 `scheduler/homogeneous_assignment.rs` 由 assignment.rs 转发）：若 `scheduling_model==homogeneous_workers`，跳过 `agent_lane_leases` 写入，改为：计数 `released_at=''` 的 slots `< max_parallel`，再 INSERT slot；冲突 slice 主键则幂等。legacy 零改动。 |

### 5.2 dedupe key

| | |
|---|---|
| 现状 | `format!("{}:{}:{}:{}", role, trigger, target, rev)` |
| 目标 | homogeneous：`format!("{}:{}:{}:{}:{}", "homogeneous_v1", trigger, target, rev, slice_id)` |
| 改法 | homogeneous 分支单独拼 key；**禁止**改 legacy 行语义。 |

### 5.3 mailbox

| | |
|---|---|
| 现状 | `mailbox/authority.rs` 仅 Root↔child |
| 目标 | 保持；Worker 不互聊 |
| 改法 | **不改** authority。homogeneous 仍经 Root 投递；若需新 envelope 字段带 `slice_id`，只扩展 payload，不扩展路由。 |

### 5.4 coordinator_frame

| | |
|---|---|
| 现状 | `native_coordinator_frame_step_for_kind` 固定 kind→step；suggestions 必须命中 step |
| 目标 | 新 kind `homogeneous-decision` → step `assess:homogeneous_decision`（名称待实现时与现有命名风格对齐，**不得**占用旧 kind） |
| 改法 | 在 `coordinator_frame.rs` 映射表加一行；`coordinator_rust_policy.rs` 对应该 step：校验 `action ∈ allowed_actions` 且 schema 已由 `parse_coordinator_decision` 通过。旧 kind 逻辑不动。 |

### 5.5 human_hook / web_executor_adapter

| | |
|---|---|
| 现状 | 未评估/延期 → `root_human_directive_deferred` → `AgentTargetOutcome::incomplete` |
| 目标 | reject/rewrite 反馈用户；任务继续 |
| 改法 | 1) `directive_gate` 产出后写 `agent_human_directive_gated`；2) `coordinator_human_hook.rs`：homogeneous 下「驳回」走成功路径+投影，不 raise deferred；3) `web_executor_adapter.rs`：若 error 是 deferred **且** model=homogeneous **且** gated 已写 reject 反馈，则 **不要** incomplete，继续 `round_history`。legacy 保持 incomplete。 |

### 5.6 interpret 规则初筛

复用 `agent_runtime/multi_agent/directive/interpretation.rs` 的 `interpret()`（约 :98）。G2 结果映射到 `rule_screen_result`。rewrite 文本必须再跑一遍 interpret。

---

## 6. 状态机实现要点

- **唯一写入者**：状态迁移只允许在 `apply_decision` 事务或调度事务（assign 触发的 schedule）内完成。  
- **守卫**：每个迁移对照 SPEC §3 表；非法迁移 → reason_code + 整单拒绝（不写库）。  
- **assignment vocabulary 隔离**：读 `agent_assignments.state` 前先读 `scheduling_model`。  
- **Worker**：Phase 1 可用 assignment 状态推导；心跳到期复用 `scheduler/heartbeat.rs` / lease 到期路径，将 slot `released_at` 填上，slice 回 `rejected` 或保持可 retry（与现有 expiry 语义对齐，待核实后选「可 retry」）。  
- **task 终态**：全部 slice accepted → 可 `stop(goal_reached)`；部分 → `partial_success` 再 stop；用户取消 → `cancelled`。

---

## 7. 确定性降级：何时触发

计数器在 `agent_homogeneous_tasks.schema_fail_streak` / `hard_reject_streak`：

1. `parse_coordinator_decision` 失败（schema/围栏/多 JSON）→ schema_fail_streak++  
2. parse 成功但 apply 硬拒（overlap/budget/allowlist 等）→ hard_reject_streak++  
3. 任一项 **≥ 2**（同 task 连续）→ **禁止再付费问模型**，调用 `deterministic_decompose`（算法照抄 SPEC §4）  
4. 降级成功：slices 写入，`decomposition_model='deterministic'`，`degraded=1`，`degrade_reason_code='degraded_path_used'`；Phase 1 **串行**只 assign 第一个 pending slice  
5. 降级失败（无候选）→ PauseForUser（任务保持 running/decomposed，聊天提示用户给 path/asset；**不假装成功**）  
6. 成功的合法决策 → 两 streak 清零  

财务：解析失败的付费 tick 如何结算，遵循现有「一次财务尝试不重试」不变量（`tick/decision` 相关）；允许「免费剥围栏重解析 1 次」但不新建发票。细节待核实现有 tick API 后在实现 PR 注明。

---

## 8. 引导闸门改造

逐步协议严格按 SPEC §6（G0–G6）。Phase 1 行为变更要点：

| 步骤 | 实现落点 |
|---|---|
| G0–G1 | 现有草案/脱敏入口（`draft_store` / 前端确认流） |
| G2 | `interpret()` → 写 gated.`rule_screen_result`；规则 reject 可直接四字段模板回用户（可不经模型） |
| G3 | 主管 `relay_directive` / `reject_directive` |
| G4 | rewrite 再 interpret |
| G5 | 投影 `reason_zh/suggestion_zh/user_feedback_zh`（前端标签映射可第二期；后端先落库） |
| G6 | **禁止** incomplete |

驳回四字段缺一 → 整单拒绝主管输出，改用确定性驳回模板（reason_code=`schema_invalid` 或 `directive_privilege_escalation` 视场景）。

保留现状「用户确认草案」环节（Phase 1 不删 confirmation）。

---

## 9. 弱模型适配（实现要点）

落在 `homogeneous/decision.rs`：

1. 若文本含 ` ``` ` 围栏：剥一层再 parse；仍失败 → `markdown_wrapped_json`  
2. `serde_json` + `deny_unknown_fields` + snake_case  
3. `action` 必须 ∈ gate.allowed_actions  
4. `allowlist_refs` 内 id 必须 ⊆ snapshot，且覆盖 payload 引用  
5. Provider 层：能开 json_mode 则开；不能则依赖剥围栏+严格解析（模型网关改动尽量小，可只在 homogeneous 调用处加参数）

提示词：直接使用 SPEC §7 system + few-shot；实现时放 `homogeneous/prompt.md` 或 `const SYSTEM_PROMPT`。

---

## 10. 测试计划

### 10.1 建议文件

| 文件 | 内容 |
|---|---|
| `src-tauri/src/agent_runtime/multi_agent/homogeneous/decision_tests.rs` | 解析/围栏/额外字段/缺字段 |
| `.../homogeneous/decompose_tests.rs` | overlap / 确定性降级 |
| `.../homogeneous/apply_tests.rs` | 七动作状态迁移 + 预算/工具 |
| `.../homogeneous/directive_gate_tests.rs` | 越权引导 / rewrite 夹带 URL |
| `.../homogeneous/machine_tests.rs` | machine_checks 重算 |
| `.../homogeneous/golden_tests.rs` | GOLD-01..25 映射 |

### 10.2 GOLD 映射（设计保证 / 谁挡）

| GOLD | 期望 | 挡在 |
|---|---|---|
| 01 | decompose 3 不重叠 | apply+scopes_overlap |
| 02 | 重叠整单拒 | scopes_overlap |
| 03 | invented_role | types 枚举 / serde |
| 04 | 合法 assign | allowlist+budget |
| 05 | shell.exec | tool_not_authorized |
| 06 | 超预算 | budget_exceeded |
| 07 | 胡编 URL | allowlist_violation |
| 08 | accept 全绿 | machine recompute |
| 09 | 谎报 checks | recompute 不一致拒 |
| 10 | evidence_missing → retry | reject_retry |
| 11 | 超 max_retries | retry_limit_exceeded |
| 12 | cross_slice_write | machine |
| 13–15 | 引导越权类 | interpret + reject_directive |
| 16 | 合理 relay/rewrite | directive_gate |
| 17 | rewrite 夹带 evil | interpret 二检 |
| 18 | markdown 围栏 | decision 剥/拒 |
| 19 | 自由长文 | free_text_instead_of_json |
| 20 | 双 action | schema |
| 21 | stop 时 running | stop 校验 |
| 22 | partial stop | stop payload |
| 23 | allowlist_refs 不一致 | parse/apply |
| 24 | extra_field | deny_unknown |
| 25 | 缺 decision_id | missing_required |

Phase 1 CI 建议命令（实现者写准）：

```bash
cargo test -p oviraptor --lib homogeneous::golden -- --nocapture
```

（crate 名以仓库为准，待核实。）

### 10.3 回归护栏

- legacy 路径：既有 multi_agent / coordinator 测不红  
- X-exposure：grep 变更集不含 exposure 路径  

---

## 11. 分阶段 PR / 提交切片（只规划，不要求真 commit）

| PR | 内容 | 验收一句话 |
|---|---|---|
| PR-A | DDL + types + parse/decision + 单测 GOLD-18/19/24/25 | 严格解析绿 |
| PR-B | store + decompose/overlap + deterministic_decompose + GOLD-01/02/03 | 分解/降级绿 |
| PR-C | schedule homogeneous 分支 + lane_slots + dedupe + assign + GOLD-04/05/06/07 | 三路只读并行不误报 occupied |
| PR-D | WorkerReport 适配 + machine_checks + accept/reject_retry + GOLD-08..12 | 验收环绿 |
| PR-E | directive_gate + human_hook/adapter 行为变更 + GOLD-13..17 | 驳回回用户且不 incomplete |
| PR-F | stop + 状态机收尾 + GOLD-21/22 + 接线 frame/policy | Phase 1 路径可跑通临时库 |
| PR-G | golden_tests 全量 + 文档勾选 Phase 1 | 关键 100%，全量 ≥95% |

回滚：任务开关默认 legacy；homogeneous 表可留空；出问题只关开关，不删 legacy。

---

## 12. 风险与回滚

| 风险 | 缓解 |
|---|---|
| assignment.state 双 vocabulary 混读 | 所有查询带 scheduling_model；单测锁 |
| 旧 lane_leases UNIQUE 误用 | homogeneous 不写该表 |
| 财务双花 / 重试 | 遵循现有 tick 一次尝试；降级不新开发票刷模型 |
| 证据 id 格式 | 别名表；不放宽 SPEC |
| 工具名示例 vs 注册表 | allowlist 只给真实串 |
| 误改 exposure | PR 检查清单 + 文件黑名单 |
| 弱模型胡言 | 严格 schema + streak 降级 + 黄金集准出 |

回滚开关：`scheduling_model` 默认 `legacy_role_pipeline`；feature 可再加编译开关 `homogeneous_workers`（可选）。

---

## 13. 给实现者的「完成定义」

当且仅当 §0.1 勾选全部完成，且：

1. 关键 GOLD 100%，全量 ≥95%  
2. legacy 相关测未故意破坏  
3. diff 不含 exposure 业务路径  
4. 对话汇报：改动路径列表 + 测试命令与结果  

然后停止，交给「窃蛋龙改造」审核。

---

## 附录 A. 关键类型落模块（不重复全文 Schema）

| SPEC 类型 | Rust 模块 |
|---|---|
| CoordinatorDecision | `homogeneous/types.rs` + `decision.rs` |
| TaskSlice | `types.rs` + `store.rs` |
| Assignment | 扩展现有 assignment 结构 / store |
| WorkerReport | `types.rs` + worker 适配器 |
| AcceptanceVerdict | `types.rs` + `store.rs` |
| HumanDirective（gated） | `types.rs` + `directive_gate.rs` |

枚举/reason_code **照抄 SPEC §1.1**，禁止增删。

## 附录 B. 文档关系

| 文件 | 角色 |
|---|---|
| 本文 DESIGN | 实现定稿 |
| SPEC_FOR_QWEN | 契约/Schema/黄金集 |
| ARCHITECTURE | 背景 |
| ANALYSIS | 现状对照 |
| DESIGN_BY_QWEN | **非依据**（错误分工产物，可归档） |
| IMPLEMENTATION_TASK_PROMPT | 给 Qwen 的实现话术 |


---

## 附录 C. 七动作 apply 校验伪代码（实现时逐行翻译）

```text
fn apply_decision(dec, lease, tx):
  require_executable_coordinator(tx, lease)
  task = load_task(tx, dec.parent_task_id)
  require task.root_run_id == lease.root_run_id
  require dec.action in allowed_actions(task)
  match dec.action:
    decompose => apply_decompose(...)
    assign => apply_assign(...)
    ...
  clear streaks on success
  commit

fn apply_decompose(payload, task, tx):
  require task.state in {created}  // Phase1: 首次分解；追加分解标待核实，默认禁止
  require 1 <= len(slices) <= 16
  for s in slices: validate_slice_fields(s)  // SPEC schema
  for each pair: if scopes_overlap: reject slice_overlap
  for s in slices: INSERT agent_task_slices state=pending
  UPDATE task state=decomposed

fn apply_assign(payload, task, tx):
  slice = load_slice(payload.slice_id)
  require slice.state in {pending, rejected} and retry_count < max_retries
  require slice.lane == payload.lane
  require slice.lane == read_only_analysis  // Phase1
  require payload.capability_lease ⊆ allowlist.tools_for(lane, profile)
  require payload.reserved_* <= remaining and >= 0
  require quota_available(task, lane)
  // schedule
  dedupe = format!("homogeneous_v1:{trigger}:{target}:{rev}:{slice_id}")
  insert assignment scheduling_model=homogeneous_workers, state=assigned
  insert lane_slot
  reserve budget (reuse existing reservation helpers if any)
  slice.state = assigned
  task.state = running if was decomposed
  // mailbox send — 待核实 envelope

fn apply_accept(...):
  require assignment.state == reported
  checks = recompute_machine_checks(assignment_id)
  require checks == payload.machine_checks  // 逐键
  require all checks true
  require payload.verdict == accept
  require evidence_refs ⊆ report.evidence_refs
  INSERT verdict; assignment=accepted; slice=accepted; release slot; worker idle

fn apply_reject_retry(...):
  require assignment.state == reported
  require slice.retry_count < max_retries else reject retry_limit_exceeded
  checks = recompute(...)
  // 若 checks 全 true，则必须 model_judgement.adequate == false
  INSERT verdict retry; assignment=rejected_retry; slice=rejected; retry_count++; release slot

fn apply_stop(...):
  require no assignment in {assigned,running,reported}  // Phase1 严格
  require stop_reason in enum
  if publish_partial: require accepted_slice_ids non-empty
  cancel leftover slices; set task terminal

fn apply_relay_directive(...):
  d = load_gated(directive_id)
  require d.state in {rule_screened, awaiting_coordinator}
  require d.rule_screen_result != reject
  text = rewritten_text if mode==rewrite else d.raw_text_redacted
  require interpret(text).ok  // 无越界
  require xor(target_worker_id, broadcast_constraint)
  update gated; project user_feedback_zh

fn apply_reject_directive(...):
  require reason_code, reason_zh, suggestion_zh, user_feedback_zh all non-empty
  require reason_code in directive-related set
  update gated state=rejected
  // DO NOT return incomplete to web executor
```

## 附录 D. Phase 1 文件黑名单（diff 不得出现）

```
**/multi_agent_external_surface*
**/asset_exposure*
**/AssetExposure*
docs/NEST_ASSET_EXPOSURE*
```

（实现时以仓库实际路径为准再补全；原则：外网暴露面业务零改动。）

## 附录 E. 意图闸门（深度分析 §J 增量）

- 主管仍只有七动作；意图为 pending / rejected 可重试 TaskSlice 或 assigned / running / reported assignment。产出为本轮实际新增 pending 或新 assign。
- `CoordinatorDecision.zero_intent` 可选；`reason` 封闭为 `covered_by_inflight | blocked_on_inflight_output`，必带非空 `evidence_assignment_ids` 与 ≤80 字 `note_zh`。证据必须在本任务回合开始的真实 inflight 中，且出现在 `allowlist_refs.assignment_ids`；不接受自由文本代替结构化声明。
- 非产出动作（accept / reject_retry / 引导）提交前必须通过该校验；最后一次 accept 可引用本轮开始的 reported，随后下一 tick `stop(goal_reached)` 收官。合法 stop 根据真实验收、预算、取消或重试状态审核。
- pending / 可重试 / inflight 全空而目标未达成：原单整单拒绝，同一付费轮确定性新增至少一条已授权只读 pending；新增 reason_code 仅 `zero_intent_illegal`，降级仍标 `degraded_path_used`。所有授权方向已验收或重试耗尽时用既有合法 stop 收尾，不用 PauseForUser 反复付费。
- 允许未终态任务追加新方向；不得复用或改写旧切片身份，不得与可推进/已验收范围重叠。全失败不提前关闭主管，后续可追加或 stop；回收后 dead Worker 只在现有容量内签发替换身份。
- Rust 将通过审核的决策、回合开始状态、zero 意图理由/证据、fallback `trigger_reason_code` 与 stop 原因写入不可变 `agent_homogeneous_decision_audit`；保持原 camelCase `DecisionSummary`。


## 附录 F. 产品只读模型 Worker 与聊天投影

- 新 Web Multi 草稿显式选择同质模式，冻结 `workerExecution=model_read_only_v1`、容量 3 和只读 lane；默认入口保持 legacy。只有 `assign` 唤起独立模型上下文，所有 Worker 共享 `homogeneous_v1`，任务职责由切片与主管批准约束决定。
- 主管与 Worker 均读取原冻结计划中的 `goal`；Worker 仅完成本次切片，不独自承担其他切片。计划摘要不匹配时拒绝读取目标，不能通过修改执行模式绕开校验。
- 模型调用复用原付费传输、预算、回执和退出证明。每次 assignment 一次有界、无工具模型调用；不能联网或写目标。捕获的 API 引用只能证明观测到引用，不能冒充 HTTP 执行或漏洞确认。
- 引导经现有七动作、规则闸门和主管批准，走主管到 Worker 的绑定 mailbox；已发送模型输入保持原样，晚到引导下一 tick 处理。只有实际读取约束的 Worker 才 ACK；投递与读取回执分别显示。
- 已提交主管裁决、Worker 原始汇报和引导状态进入现有协作事件序列，聊天支持实时、增量与历史恢复；费用未知保留原账，不伪造成功或免费重派。
- 新同质草稿冻结 `credentialVisibility=plaintext`：任务绑定的 Cookie、Token、密码等原材料可供内部身份比对与聊天只读查看；不得读取其他任务/项目身份，查看过期身份不重新授予执行权。模型服务密钥仍不进入任务材料。
- 拒绝计数纳入候选快照，防止新拒绝状态被误认为历史付费轮；达到阈值后在原容量、原预算内串行 `assign` 首条 pending/可重试切片，合法派发才清零计数，后续 accept 仍走付费裁决。
- DeepSeek V4 的 ≤2048 输出 token 结构化调用显式使用 `thinking.type=disabled`；该供应商参数随原请求冻结并校验，保持原输出预留与真实用量回执。大额度/其他模型保留供应商默认。


## 附录 G. 显式执行续作边界（2026-10-12）

- `model_broker_v1` 必须在原始 Root 签名出生前冻结到 `homogeneousExecution`，包括本次 Native task plan 摘要；原只读任务不得热升级。
- 批准解析工具需准确 runtimeDigest 与原批准包；代码及输入来自本次真实 Worker SDK 回执，硬沙箱结果仅作分析证据，同 attempt 不重放。
- 同质业务画像仍为 `homogeneous_v1`，技术载体权限按每次 lane 签发。目标控制与独立复核在此显式模式下独占串行，Phase 1 并行范围保持只读；主管动作仍为原七项。
- 目标、复核和安装 App 的完整链尚待正负集成与实际安装验证。续作和已知缺口见 `NEST_HOMOGENEOUS_CONTINUATION_HANDOFF_2026-10-12.md`，不能据本附录宣布完成。
