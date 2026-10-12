> ⚠️ **非实现依据**：错误分工产物。实现以 `NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md` 为准。本文仅可作核实线索，冲突时作废。

# Nest 同质 Worker + 强主管：完整设计文档（Qwen 交付稿）

- 日期：2026-10-10（Asia/Shanghai）
- 作者：Qwen 设计会话（本文件为唯一交付物）
- 性质：**设计文档**。只写文档；不改业务代码、不改 exposure 模块、不改用户真实数据库、不 commit、不 push。
- 交付后审核：**本设计完成后交给 Grok Bot（窃蛋龙改造）按 `docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md` 的契约（封闭枚举 / JSON Schema / 七动作 / 状态迁移）与黄金集 GOLD-01…GOLD-25 审核。** 本会话不自行宣布验收通过。

## 0. 输入与冲突裁定

| 角色 | 文件 | 用法 |
|---|---|---|
| 总览（目标态） | `docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md` | 原则、阶段、弱模型思想 |
| **详细规格（权威）** | `docs/NEST_HOMOGENEOUS_COORDINATOR_SPEC_FOR_QWEN_2026-10-10.md` | 封闭枚举、schema、七动作契约、状态迁移表、降级伪代码、提示词、黄金集、对接点 |
| 现状基线（只读） | `docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md` | 现状断言；本文所有「现状」结论以它 + 本会话直接读取的代码为准 |

裁定规则（按总览 §0 与规格 §10 约束）：

1. 封闭枚举与 JSON Schema：**以规格为准**。
2. 原则性叙述（为什么、阶段划分）：以总览为准。
3. 本文与规格/总览冲突时，本文作废，不改规格。
4. 规格内部编号有错位（§5 的子节写成 6.1/6.2/6.3；§7 的子节写成 8.1/8.2）。本文按语义引用，标注为「规格 §5（验收）」「规格 §7（提示词）」。

### 0.1 大纲冲突的处理（必读）

任务书给出的 12 节列表与规格 §10 的 12 节大纲**不是同一套**。处理方式：**以任务书 12 节为文档主结构**（任务书是本次交付的直接指令），同时把规格 §10 的每一项要求映射进主结构，确保规格要求的任何一节都不缺失。映射表：

| 规格 §10 要求 | 本文落点 |
|---|---|
| 1 假设与范围（重述硬规则，声明不动 Exposure） | §1（目标与非目标）+ §1.3 硬规则复述 |
| 2 类型与 schema 差异表 | §2.2 差异表（相对规格：全部无删字段；列出实现侧新增承载列） |
| 3 七动作时序图（输入 allowlist / 输出 JSON / Rust 校验点） | §5.2 七动作逐条 + §5.3 通用校验顺序图 |
| 4 状态迁移表（任务/切片/assignment/Worker/directive） | §4 全节 |
| 5 lane 与去重键改造方案（SQL 草案 + 兼容策略） | §2.3 SQL 草案 + §11.1（lane lease / dedupe key） |
| 6 引导闸门时序（G0–G6）与驳回文案模板 | §8 全节 |
| 7 确定性降级算法 | §6 全节 |
| 8 弱模型适配（json_mode/剥围栏/重试/降级） | §12.2 + §5.1 + §9.5 |
| 9 黄金集执行计划 | §10.5 + §12.3 |
| 10 双模迁移开关与回滚 | §11.2 + §11.4 |
| 11 风险与非目标 | §12.1 + §1.2 |
| 12 附录：≥3 端到端 JSON 对话样例 | 附录 A（A1/A2/A3） |

### 0.2 引用纪律

本文所有 `文件:行号` 均为本会话**直接读取该文件对应行**确认过的；只给函数名/表名而不给行号的，表示位置需实现时按符号定位，**不得被当作行号证据**。凡本会话未能确认的事项，一律写「待核实」并附核实方法，汇总见 §12.4。

### 0.3 零发明声明

本文使用的动作、角色、状态、reason_code、lane、profile 全部来自规格封闭集合：

- 主管动作：仅 `decompose / assign / accept / reject_retry / stop / relay_directive / reject_directive`（7 个，无第 8 个）。
- 角色：目标态不引入任何新角色；Worker 能力画像只有 `homogeneous_v1`。现状 15 个 `AgentRole`（`src-tauri/src/agent_runtime/contract.rs:92`）在新路径**不参与判定**（见 §2.2 D-02）。
- 状态：`task_state / slice_state / assignment_state / directive_state / verdict / worker state / lane / rule_screen_result / coordinator_action / mode / stop_reason` 十个封闭集合，逐条列于附录 B。
- reason_code：规格 §1.1 列出的 **37** 条（本会话逐条计数），逐条使用点见 §7.3。
- 工具名：本文不新增工具。示例 JSON 里的工具串一律以规格原文呈现，真实注册表字符串映射见 §2.6。

**标识符边界声明**（避免被误读为「发明」）：本文出现的非规格标识符只限于四类**实现细节**，且规格本身未对其封闭：① 表名 / 列名（§2.3）；② Rust 模块名 / 函数名（§3.4、§11.5）；③ IPC 命令名与只读投影的字段名（§3.3）；④ frame kind / step 名的新增行（§11.1 T-04，规格 §9 明示允许新增 frame kind）。这四类**一律不进入**动作/角色/状态/工具/reason_code 五个封闭集合，且逐条标注「待评审」。**五个封闭集合内没有本文新增的任何成员**（附录 B 逐条对照）。

---

## 1. 目标与非目标

### 1.1 目标（与总览 §1.1 一致，逐条给出本文落点）

| # | 目标 | 本文落点 |
|---|---|---|
| G1 | 弱化固定角色：主体是同质 Worker，差异只在切片 | §2.2 D-02（profile 常量替代 role 参与去重）、§5.2.1 |
| G2 | 主管先分析再分配：分解 → 切片 → 派发 | §5.2.1 / §5.2.2、§4.1 |
| G3 | 同质多实例并行：3 个同 profile Worker 干 A/B/C | §2.3 lane slot、§11.1 T-01 |
| G4 | 汇报—验收—再调度闭环 | §5.2.3 / §5.2.4、§7 |
| G5 | 聊天引导闸门：主管可 relay/rewrite/reject 并反馈用户 | §8 |
| G6 | 弱模型可控：较差国内 API 仍能干活 | §5.1、§9、§12.2、§12.3 |

### 1.2 非目标（明确不做）

1. 平权 agent 互相委派；Worker 之间直接通信（现状 `mailbox/authority.rs:35` 的 Root↔child 路由**保持不变**，见 §11.1 T-03）。
2. 用户绕过主管直达 Worker。
3. 漏洞检测规则本身（本文只谈拓扑、契约、状态、验收、闸门）。
4. 常驻几十个 agent：并行度由 `lane_slots` 配额与剩余预算决定（§2.3）。
5. **第一期不打开 `target_touching` lane 的同质并行**（总览 §3.7 第 3 步之后再做；本文遵循：第一期只放开 `read_only_analysis`）。
6. **第一期不改 exposure / Asset Exposure 模块**（规格 §0.7、§9 最后一行）。涉及文件清单（§11.5）里不含任何 exposure 路径；护栏测试见 §10.3 X-11。
7. 第一期不迁移、不重写用户真实数据库；§2.3 的 SQL 只是**草案**，执行属实现阶段。
8. 不删除 legacy 路径（总览 §3.7 双模并存）。
9. 不改规格、不删规格字段。规格列出的字段一个都不能少。

### 1.3 硬规则复述（规格 §0，违反任一条 = 设计作废）

| 编号 | 规则 | 本文如何保证 |
|---|---|---|
| H1 | 禁止发明新角色/工具/状态/动作/reason_code | 附录 B 封闭集合逐条对照；新出现的标识符仅限「表名/列名/模块名/frame kind」这类规格未封闭的实现细节，且全部标「待评审」 |
| H2 | 禁止跳过主管 | §5.1 决策入口唯一；§8.5 引导只能经主管 relay/reject |
| H3 | 禁止平权互委派 | §11.1 T-03 mailbox 不改；§12.1 R-06 |
| H4 | JSON 键 snake_case 英文，中文只在 `*_zh` | §9.5 解析器强制；§2.4 列名与字段一一对应 |
| H5 | 主管每轮输出单个 JSON，缺/多/类型错/越权 → 整单拒绝走降级 | §5.1 A-03、§6.1 触发条件 |
| H6 | 候选只能从 allowlist 选 id | §5.1 A-04 basis_hash 冻结、§5.4 machine_checks 重算 |
| H7 | 第一期不动 Exposure | §1.2 第 6 条、§10.3 X-11 |
| H8 | 按固定大纲交付 | §0.1 双向映射表 |

### 1.4 第一期范围切分

| 做 | 不做 |
|---|---|
| 7 动作契约的结构体与严格解析（新结构体，不动 `DecisionSummary`） | 复用/改写 camelCase 的 `DecisionSummary`（`agent_runtime/multi_agent/budget/root/model/tick/decision.rs:9` 有 `deny_unknown_fields` + `rename_all="camelCase"`，与规格 snake_case 冲突，不可复用） |
| `read_only_analysis` lane 的配额并行 | `target_touching` / `review` lane 并行扩量 |
| 切片去重键含 `slice_id` 的新路径 | 修改 legacy 行的 dedup_key 语义（legacy 保持 `role:trigger:target:evidence_revision`） |
| acceptance verdict 落库 + 发布门禁 | 替代 `EvidenceReviewer` 的漏洞确认权（现状唯一 confirmed 权限保留） |
| 引导闸门 rewrite/reject + 四字段反馈 | 取消现状「用户确认草案」环节（保留，见 §8.7 待评审） |
| Root 决策/回执 append-only 投影复用 | 新增第二套预算账本（继续用 `agent_budget_ledger`，`db_schema.rs:1305`） |

### 1.5 与现状的关键差异（本会话直接核实结果）

| # | 规格/总览的说法 | 本会话核实 | 影响 |
|---|---|---|---|
| V-01 | lane 主键 `(scan_id,attempt_number,target_key,lane)` | **成立**：`db_schema.rs:1295-1303`，`PRIMARY KEY(scan_id,attempt_number,target_key,lane)`，且 `assignment_id TEXT NOT NULL UNIQUE`（`:1300`）→ 一条 lane 租约只能被一个 assignment 持有，即使 PK 加 slice 也仍需处理这个 UNIQUE | §2.3 改造必须同时面对两条唯一约束 |
| V-02 | dedupe 不含切片 | **成立**：`scheduler/assignment.rs:72-78` `format!("{}:{}:{}:{}", role.as_str(), trigger_code, lease.target_key, evidence_revision)`；`asg-` 前缀在 `:80` | §11.1 T-02 |
| V-03 | 现状完全没有 slice 概念 | **不成立**：`agent_assignments.task_slice_json` 已存在（`db_schema.rs:1257`），`schedule_child` 第 6 参就是 `task_slice: &JsonValue`；但 `slice_id` 全仓 0 命中 | §2.2 D-05：切片有「内容」无「身份」，本设计补 id 而非新建概念 |
| V-04 | assignment 状态 `created…` | 现状 vocabulary 是 `prepared/leased/running/waiting_review/paused/completed/failed/cancelled/expired`（`scheduler/lifecycle.rs:259`、`attempts/schema.rs:16`）；`agent_assignments.state` **无 CHECK 约束**（`db_schema.rs:1254` 默认 `'prepared'`） | §4.3 双 vocabulary 必须用 `scheduling_model` 列隔离 |
| V-05 | Worker 需要新身份表 | 现状 `agent_assignment_attempts.worker_id TEXT NOT NULL UNIQUE`（`attempts/schema.rs:15`），身份由 `issue_first` 在调度事务内签发；`attempts/schema.rs:1-2` 注释「New worker identities only. Existing runs are never backfilled with grants.」 | §2.3 `agent_worker_instances` 只能作为投影，签发路径唯一（§12.1 R-05） |
| V-06 | 证据引用 `^evd-[a-f0-9]{8,32}$` | 现状证据节点 id 是**裸 64 位 hex**：`evidence_graph/store.rs:32` `stable_hash(&format!("{root_run_id}|{...}|{kind}"))`，无 `evd-` 前缀且长度 64 > 32 | §2.4.3 别名映射（契约层 `evd-<20hex>` ↔ 内部真实 id），**不得放宽规格 pattern** |
| V-07 | 「主管固定 step」护栏 | 现状护栏是 `suggestions` 必须等于 `frame.step()`：`coordinator_frame.rs:317` 单表映射 18 个 kind；`coordinator_rust_policy.rs:531-541` 命中计数，未命中即 `root_decision_did_not_select_step` | §5.1 A-02 护栏从「单一 step」改为「allowed_actions 子集」，且**只对新 frame kind 生效** |
| V-08 | 降级/重试 2 次 | 现状 Root tick 明确**不重试**：`tick/decision.rs:30` 解析失败即 `root_tick_decision_invalid`；「一次财务尝试，不是重试策略」 | §6.1 三档重试（免费清洗 / 新付费 tick 带新发票 / 降级），财务不变量优先 |
| V-09 | 工具 allowlist | 现状真实串：`evidence.read`、`mailbox.read`、`mailbox.write`、`review.write`、`public_surface_get`、`authorization_probe`、`snapshot.read`、Broker 能力 `browser_action`/`source_analysis`（`tool_supply.rs:146`）；**`http.get` 全仓 0 命中** | §2.6 映射表：示例串按规格原文保留，真实 allowlist 由 Rust 提供 |
| V-10 | 引导驳回会打断本轮 | **成立**：`coordinator_human_hook.rs:92` 产生 `root_human_directive_deferred`，`web_executor_adapter.rs:127-128` `AgentTargetOutcome::incomplete(error)`；`coordinator_human_hook.rs:91` 另有 `root_human_step_not_bounded` | §8.6 改造点（第一期唯一的行为语义变更） |
| V-11 | 中文反馈四字段 | `reason_zh/suggestion_zh/user_feedback_zh` 在 Rust 侧**未找到**；现状中文文案在前端标签（`agentDialogLabels.ts:94`）；`interpret()` 只产出英文 reasons（`interpretation.rs:247,254,261,271,281,285,302,306,313,331`） | §8.2 映射表 + §2.3 gated 表新增 `*_zh` 列 |
| V-12 | 主管模型可 rewrite | `broadcast_constraint` 全仓 0 命中；主管侧只有队列动作 `prioritize_family` | §8.5 新载荷，落库列见 §2.3 |

---

## 2. 数据模型 / 表变更（相对现状）

### 2.1 现状承载点（本会话读取确认）

| 现状对象 | 位置 | 关键点 |
|---|---|---|
| `agent_assignments` | `db_schema.rs:1247` | 列含 `role, lane, target_key, state, dedup_key, trigger_code, task_slice_json, evidence_revision, reserved_tokens, reserved_requests, capability_lease_json, lease_epoch, fencing_token, lease_expires_at`；`state` 无 CHECK |
| `agent_lane_leases` | `db_schema.rs:1295-1303` | PK `(scan_id,attempt_number,target_key,lane)`；`lane` CHECK 三值；`assignment_id` UNIQUE + `ON DELETE CASCADE` |
| `agent_budget_ledger` | `db_schema.rs:1305` | `root_run_id` PK；`total/reserved/spent_tokens`、`total/reserved_requests`；非 append-only（UPDATE 累加） |
| `agent_assignment_attempts` | `attempts/schema.rs:4` | `worker_id` UNIQUE、`fencing_token` UNIQUE、`lease_epoch` CHECK>0、`state` CHECK 七值、`UNIQUE(assignment_id,lease_epoch)` + 部分唯一索引「一 open attempt」+ 三个 ABORT 触发器 |
| `agent_coordinator_leases` | `db_schema.rs`（`lease_epoch`、`fencing_token` 两列） | 主管唯一租约；`lease.rs` `acquire_coordinator_lease` / `validate_coordinator_lease`，失败 `stale_coordinator_fencing_token` |
| `agent_capability_leases` | `db_schema.rs` | `(assignment_id, child_run_id, capability)` 唯一；写入在 `scheduler/child_grant.rs`；白名单校验 `child_contract.rs:37-68`，失败 `role_capability_not_allowed` |
| `agent_contract_owners` | `db_schema.rs` | PK `(root_run_id, contract_key)`，state `held/released` |
| `agent_evidence_revisions` | `db_schema.rs` | `(root_run_id, revision)`，`CHECK(revision>0)`、`CHECK(parent_revision<revision)`；节点 INSERT 触发器原子登记 |
| `agent_evidence_nodes` | `db_schema.rs:1814` | `id TEXT PK`（64hex）、`revision`、`kind`、`payload_json`、`artifact_refs_json`、`UNIQUE(root_run_id,revision,natural_key_hash)` |
| `agent_user_directives` | `db_schema.rs` | 27 列，含 `status`（多值 CHECK）与 `rejection_code`；由 `confirmation.rs:132` 确认后写入 |
| `agent_directive_drafts` | `db_schema.rs` | status CHECK：drafted/need_confirmation/rejected/confirmed/cancelled/expired |
| `agent_messages` | `db_schema.rs` | mailbox 持久化；`dedup_key = {assignment}:{kind}:{correlation}:{revision}`（`mailbox/send.rs:108`）；`kind` 是自由字符串非枚举 |
| `agent_root_tick_receipts` 等 | `budget/root/model/tick/schema.sql` | `request_hash`/`basis_hash`/responseHash，三段 `RAISE(ABORT,'root_tick_receipt_immutable')` |

建表约定（本会话核实）：`CREATE TABLE IF NOT EXISTS`；64hex TEXT 主键 + `WITHOUT ROWID`（纯回执表）；成对 immutability 触发器 `BEFORE UPDATE`/`BEFORE DELETE` + `BEFORE INSERT … WHEN EXISTS(唯一键冲突)` 防 REPLACE（`business_logic_output_receipt.sql:11-13` 同款）；时间戳 `created_at/updated_at TEXT DEFAULT (datetime('now','localtime'))`；布尔 `INTEGER NOT NULL DEFAULT 0/1`；JSON 列 `*_json TEXT` + `CHECK(json_valid(...))`；`ON DELETE CASCADE` 引 `agent_runs(id)`/`sentinel_scans(id)`；加列一律走 `ensure_column()`（`db_migrate.rs:20`）；**没有** `PRAGMA user_version` 式全局版本号；注册方式两种：`include_str!` 拼进 `SCHEMA`（`db_schema.rs:2336`）或在 `db.rs initialize()` 内 `execute_batch(include_str!(...))`。

### 2.2 相对规格的字段差异表（规格字段 → 现状承载 → 处置）

规则：**规格的类型字段一个都不删、不改名、不改枚举**。下表只说明「这个契约字段在物理层落在哪」。

| 规格实体.字段 | 现状承载 | 差异 | 处置 |
|---|---|---|---|
| `TaskSlice.*` | `agent_assignments.task_slice_json`（内容）+ 无 id | 缺 `slice_id` 身份；缺独立状态机 | 新表 `agent_task_slices`（§2.3 T-SL），字段与规格 §1.2 **逐字一致**；`task_slice_json` 继续作为 legacy 只读材料 |
| `TaskSlice.parent_task_id` | 现状 root run = `agent_runs.id`（`run-…`） | 规格用 `^tsk-` id | 新表 `agent_homogeneous_tasks`，`task_id` ↔ `root_run_id` 1:1 绑定（§2.4.2） |
| `WorkerInstance.*` | `agent_assignment_attempts.worker_id`（已 UNIQUE）| 缺 `state` 与 profile 列 | 新表 `agent_worker_instances`，但 **worker_id 只能由 `attempts::issue_first` 在同一事务内投影写入**，不开第二签发路径 |
| `Assignment.*` | `agent_assignments` 全列可复用 | `state` vocabulary 不同；无 `slice_id`/`profile` | 加 3 列（`slice_id`、`worker_profile_id`、`scheduling_model`）；不改 legacy 行语义 |
| `Assignment.dedupe_key` | `scheduler/assignment.rs:72-78` | 规格 §1.4 五段式含 `slice_id` | 新路径新公式（§11.1 T-02），legacy 公式原样保留 |
| `WorkerReport.*` | 无（child 结果走 `agent_messages.kind='execution_result'`） | 规格要求独立可回放结构 | 新表 `agent_worker_reports` |
| `AcceptanceVerdict.*` | 部分对应 `review.rs` 的 Reviewer 结论 + 各类 `*_receipt.sql` | 规格 `machine_checks` 固定 7 键在现状不存在 | 新表 `agent_acceptance_verdicts`（append-only）；Reviewer 的 `confirmed/rejected/insufficient` 权限**不变**，verdict 只控制「能否进入本任务结论」 |
| `HumanDirective.*`（§1.7 的闸门字段） | `agent_user_directives` + `agent_directive_drafts` | 缺 `coordinator_action/rewritten_text/target_worker_id/broadcast_constraint/*_zh` | 新表 `agent_human_directive_gated`（sidecar，键 `directive_id`）——避免改动 legacy `status` CHECK 与 `confirmation.rs:132` 状态集 |
| `CoordinatorDecision.*` | `DecisionSummary`（camelCase） | **大小写风格冲突**；`payload` 需按 action 分支 | 新结构体 `HomogeneousCoordinatorDecision`（snake_case + `deny_unknown_fields`），新表 `agent_coordinator_decisions`；`DecisionSummary` 与 `coordinator_rust_policy.rs:531` 的 step 命中判定继续服务 legacy |
| `allowlist_refs.*` | 无 | 需与 payload 双向一致 | §5.1 A-04：候选集合哈希冻结进 `basis_hash` |
| `machine_checks.*` 7 键 | 分散在 `child_contract.rs`、预算、Broker、evidence | 现状无单一结构 | §5.4 重算矩阵，逐键给数据来源 |

结论：**相对规格无字段差异**（0 删、0 改名、0 放宽枚举）；差异全部在「物理承载层」，且以新增为主。

### 2.3 新增表 SQL 草案（第一期；待实现）

> 全部为草案，本会话不执行。命名遵循 `agent_*` 前缀。规格 §2.3（总览）给出的四个逻辑名 `task_slice / assignment / acceptance_verdict / human_directive_gated` 逐一对应 T-SL / 复用 agent_assignments / T-VD / T-GD。

#### T-TK `agent_homogeneous_tasks`（task_state 载体）

```sql
CREATE TABLE IF NOT EXISTS agent_homogeneous_tasks (
    task_id TEXT PRIMARY KEY CHECK(task_id GLOB 'tsk-*'),
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    target_key TEXT NOT NULL,
    scheduling_model TEXT NOT NULL DEFAULT 'homogeneous_workers'
        CHECK(scheduling_model IN ('legacy_role_pipeline','homogeneous_workers')),
    state TEXT NOT NULL DEFAULT 'created'
        CHECK(state IN ('created','decomposed','running','partial_success',
                        'succeeded','failed','cancelled','stopped')),
    evidence_revision_base INTEGER NOT NULL DEFAULT 0 CHECK(evidence_revision_base >= 0),
    allowed_parallel INTEGER NOT NULL DEFAULT 3,
    degraded INTEGER NOT NULL DEFAULT 0,
    degrade_reason_code TEXT NOT NULL DEFAULT '',
    basis_hash TEXT NOT NULL DEFAULT '' CHECK(basis_hash = '' OR length(basis_hash) = 64),
    decided_by_lease_epoch INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id)
);
```

`state` 的 CHECK 值 = 规格 `task_state` 全部 8 个成员，未增未减。

#### T-SL `agent_task_slices`

```sql
CREATE TABLE IF NOT EXISTS agent_task_slices (
    slice_id TEXT PRIMARY KEY CHECK(slice_id GLOB 'slc-*'),
    parent_task_id TEXT NOT NULL REFERENCES agent_homogeneous_tasks(task_id) ON DELETE CASCADE,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('path_prefix','host','asset_id','source_type')),
    scope_value TEXT NOT NULL CHECK(length(scope_value) BETWEEN 1 AND 512),
    evidence_revision_base INTEGER NOT NULL DEFAULT 0 CHECK(evidence_revision_base >= 0),
    lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
    worker_capability_profile_id TEXT NOT NULL DEFAULT 'homogeneous_v1'
        CHECK(worker_capability_profile_id = 'homogeneous_v1'),
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK(state IN ('pending','assigned','in_progress','reported','accepted','rejected','cancelled')),
    max_retries INTEGER NOT NULL DEFAULT 0 CHECK(max_retries BETWEEN 0 AND 3),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count BETWEEN 0 AND 3),
    decomposition_model TEXT NOT NULL DEFAULT 'coordinator'
        CHECK(decomposition_model IN ('coordinator','deterministic')),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    CHECK(retry_count <= max_retries)
);
CREATE INDEX IF NOT EXISTS idx_task_slices_task_state ON agent_task_slices(parent_task_id, state);
```

`decomposition_model` 取值来自 §6（主管分解 vs 确定性降级），是**列值不是状态、不是动作**；`deterministic` 对应的可见标记仍是规格的 `degraded_path_used`。

#### T-WK `agent_worker_instances`

```sql
CREATE TABLE IF NOT EXISTS agent_worker_instances (
    worker_id TEXT PRIMARY KEY CHECK(worker_id GLOB 'wrk-*'),
    attempt_id TEXT NOT NULL REFERENCES agent_assignment_attempts(id) ON DELETE CASCADE,
    capability_profile_id TEXT NOT NULL DEFAULT 'homogeneous_v1'
        CHECK(capability_profile_id = 'homogeneous_v1'),
    assignment_id TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'idle' CHECK(state IN ('idle','busy','draining','dead')),
    lease_epoch INTEGER NOT NULL DEFAULT 1 CHECK(lease_epoch >= 1),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(attempt_id)
);
```

写入纪律：只有 `schedule_child` 事务在 `attempts::issue_first` 之后可 INSERT；心跳/到期路径可 UPDATE `state`；**任何读路径不得 INSERT**（对齐 `attempts/schema.rs:1-2`）。

#### T-AS `agent_assignments` 增列（不新建表）

```sql
-- 走 db_migrate.rs:20 的 ensure_column()，逐条幂等
ALTER TABLE agent_assignments ADD COLUMN slice_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_assignments ADD COLUMN worker_profile_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_assignments ADD COLUMN scheduling_model TEXT NOT NULL DEFAULT 'legacy_role_pipeline';
ALTER TABLE agent_assignments ADD COLUMN worker_id TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS ux_assignments_task_slice_attempt
    ON agent_assignments(scheduling_model, slice_id, lease_epoch)
    WHERE scheduling_model = 'homogeneous_workers' AND slice_id <> '';
```

`state` 无 CHECK（V-04），因此新 vocabulary（`created/assigned/running/reported/accepted/rejected_retry/stopped/cancelled`，规格 §1.1）**可以直接写入同一列**，靠 `scheduling_model` 区分。风险与隔离见 §12.1 R-01。

#### T-LN `agent_lane_slots`（配额槽，替代互斥槽；新表而非改 PK）

```sql
CREATE TABLE IF NOT EXISTS agent_lane_slots (
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
    slice_id TEXT NOT NULL,
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    acquired_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    released_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(root_run_id, lane, slice_id)
);
CREATE TABLE IF NOT EXISTS agent_lane_quotas (
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    lane TEXT NOT NULL CHECK(lane IN ('read_only_analysis','target_touching','review')),
    max_parallel INTEGER NOT NULL DEFAULT 1 CHECK(max_parallel BETWEEN 1 AND 16),
    PRIMARY KEY(root_run_id, lane)
);
```

第一期：`agent_lane_leases`（PK 四列，V-01）**不改**；新路径只在 `scheduling_model='homogeneous_workers'` 时用 `agent_lane_slots` + `agent_lane_quotas` 判定占用，`read_only_analysis` 的 `max_parallel` 允许 >1，`target_touching` 与 `review` 由 Rust 强制 `max_parallel=1`（§12.1 R-03）。理由：SQLite 不能 ALTER PK，且 `assignment_id UNIQUE` 与 389 个测试文件守着旧语义（§12.1 R-04）。

#### T-RP `agent_worker_reports`

```sql
CREATE TABLE IF NOT EXISTS agent_worker_reports (
    report_id TEXT PRIMARY KEY CHECK(report_id GLOB 'rpt-*'),
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    slice_id TEXT NOT NULL REFERENCES agent_task_slices(slice_id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES agent_worker_instances(worker_id) ON DELETE CASCADE,
    evidence_refs_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence_refs_json)),
    output_json TEXT NOT NULL CHECK(json_valid(output_json)),
    consumed_tokens INTEGER NOT NULL DEFAULT 0 CHECK(consumed_tokens >= 0),
    consumed_requests INTEGER NOT NULL DEFAULT 0 CHECK(consumed_requests >= 0),
    terminal_hint TEXT NOT NULL CHECK(terminal_hint IN ('completed','failed','partial')),
    lease_epoch INTEGER NOT NULL CHECK(lease_epoch >= 1),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
-- 三段 immutability 触发器（沿用 business_logic_output_receipt.sql 惯例）
CREATE TRIGGER IF NOT EXISTS worker_report_no_update BEFORE UPDATE ON agent_worker_reports
BEGIN SELECT RAISE(ABORT,'worker_report_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_report_no_delete BEFORE DELETE ON agent_worker_reports
BEGIN SELECT RAISE(ABORT,'worker_report_audit_immutable'); END;
```

`terminal_hint` 三值 = 规格 §1.5，未增未减。`assignment_id` 与 `slice_id` 的一致性不靠 SQL，由 §5.2.3 校验（跨表 CHECK 不可用）。

#### T-VD `agent_acceptance_verdicts`

```sql
CREATE TABLE IF NOT EXISTS agent_acceptance_verdicts (
    verdict_id TEXT PRIMARY KEY CHECK(verdict_id GLOB 'vrd-*'),
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    slice_id TEXT NOT NULL REFERENCES agent_task_slices(slice_id) ON DELETE CASCADE,
    report_id TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL CHECK(verdict IN ('accept','retry','reject')),
    -- 注：report_id 不在规格 §1.6 AcceptanceVerdict 字段表里，它来自规格 §2.3/§2.4 的 accept/reject_retry
    --     payload（必填），本文把它作为 verdict 行的溯源列，属物理承载，不新增契约字段。
    machine_scope_in_bounds INTEGER NOT NULL CHECK(machine_scope_in_bounds IN (0,1)),
    machine_budget_ok INTEGER NOT NULL CHECK(machine_budget_ok IN (0,1)),
    machine_tools_authorized INTEGER NOT NULL CHECK(machine_tools_authorized IN (0,1)),
    machine_output_schema_ok INTEGER NOT NULL CHECK(machine_output_schema_ok IN (0,1)),
    machine_evidence_refs_exist INTEGER NOT NULL CHECK(machine_evidence_refs_exist IN (0,1)),
    machine_evidence_revision_fresh INTEGER NOT NULL CHECK(machine_evidence_revision_fresh IN (0,1)),
    machine_no_cross_slice_write INTEGER NOT NULL CHECK(machine_no_cross_slice_write IN (0,1)),
    model_judgement_json TEXT NOT NULL DEFAULT 'null' CHECK(json_valid(model_judgement_json)),
    reason_codes_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(reason_codes_json)),
    evidence_refs_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence_refs_json)),
    decided_by_lease_epoch INTEGER NOT NULL CHECK(decided_by_lease_epoch >= 1),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    -- 规格 §1.6：任一机器检查为 false 时禁止 accept
    CHECK(verdict <> 'accept' OR (machine_scope_in_bounds = 1 AND machine_budget_ok = 1
      AND machine_tools_authorized = 1 AND machine_output_schema_ok = 1
      AND machine_evidence_refs_exist = 1 AND machine_evidence_revision_fresh = 1
      AND machine_no_cross_slice_write = 1)),
    -- 任一为 false → model_judgement 必须为 SQL NULL
    CHECK((machine_scope_in_bounds = 1 AND machine_budget_ok = 1 AND machine_tools_authorized = 1
       AND machine_output_schema_ok = 1 AND machine_evidence_refs_exist = 1
       AND machine_evidence_revision_fresh = 1 AND machine_no_cross_slice_write = 1)
       OR model_judgement_json IS NULL)
);
CREATE TRIGGER IF NOT EXISTS acceptance_verdict_no_update BEFORE UPDATE ON agent_acceptance_verdicts
BEGIN SELECT RAISE(ABORT,'acceptance_verdict_immutable'); END;
CREATE TRIGGER IF NOT EXISTS acceptance_verdict_no_delete BEFORE DELETE ON agent_acceptance_verdicts
BEGIN SELECT RAISE(ABORT,'acceptance_verdict_audit_immutable'); END;
```

> 注：`model_judgement_json TEXT NOT NULL DEFAULT 'null'` 与「false 时必须为 SQL NULL」不能同时成立——实现时必须改为 `model_judgement_json TEXT CHECK(model_judgement_json IS NULL OR json_valid(model_judgement_json))`（去掉 NOT NULL）。这条自相矛盾已登记为 §12.4 Q-06，属**实现注意项，不改变契约**（契约侧 `model_judgement` 仍可空，规格 §1.6 明示 `object|null`）。

#### T-GD `agent_human_directive_gated`

```sql
CREATE TABLE IF NOT EXISTS agent_human_directive_gated (
    directive_id TEXT PRIMARY KEY CHECK(directive_id GLOB 'dir-*'),
    raw_text_redacted TEXT NOT NULL CHECK(length(raw_text_redacted) BETWEEN 1 AND 2000),
    state TEXT NOT NULL DEFAULT 'received'
        CHECK(state IN ('received','rule_screened','awaiting_coordinator','relayed',
                        'rewritten','rejected','applied')),
    rule_screen_result TEXT NOT NULL CHECK(rule_screen_result IN ('pass','need_confirm','reject')),
    rule_reason_codes_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(rule_reason_codes_json)),
    coordinator_action TEXT
        CHECK(coordinator_action IS NULL OR coordinator_action IN ('relay','rewrite','reject')),
    rewritten_text TEXT,
    target_worker_id TEXT,
    broadcast_constraint TEXT,
    user_feedback_zh TEXT,
    reason_code TEXT,
    reason_zh TEXT,
    suggestion_zh TEXT,
    decision_id TEXT DEFAULT '',
    receipt_id TEXT DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    -- 规格 §1.7：reject 时四个可读字段齐备
    CHECK(coordinator_action <> 'reject'
       OR (reason_code IS NOT NULL AND reason_zh IS NOT NULL
       AND user_feedback_zh IS NOT NULL AND suggestion_zh IS NOT NULL)),
    -- 规格 §1.7：target_worker_id 与 broadcast_constraint 互斥
    CHECK(NOT (target_worker_id IS NOT NULL AND broadcast_constraint IS NOT NULL))
);
```

`reason_code` 单值成员资格检查放在 Rust（枚举 37 条，SQL 里写 CHECK 会重复维护），实现点 §5.2.7。

#### T-DC `agent_coordinator_decisions`（主管输出全量留痕，append-only）

```sql
CREATE TABLE IF NOT EXISTS agent_coordinator_decisions (
    decision_id TEXT PRIMARY KEY CHECK(decision_id GLOB 'dec-*'),
    parent_task_id TEXT NOT NULL REFERENCES agent_homogeneous_tasks(task_id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK(action IN ('decompose','assign','accept','reject_retry','stop',
                                          'relay_directive','reject_directive')),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    allowlist_refs_json TEXT NOT NULL CHECK(json_valid(allowlist_refs_json)),
    model_notes_zh TEXT DEFAULT '',
    basis_hash TEXT NOT NULL CHECK(length(basis_hash) = 64),
    request_hash TEXT NOT NULL CHECK(length(request_hash) = 64),
    response_hash TEXT NOT NULL CHECK(length(response_hash) = 64),
    prompt_hash TEXT NOT NULL CHECK(length(prompt_hash) = 64),
    gate_verdict TEXT NOT NULL CHECK(gate_verdict IN ('applied','refused','degraded')),
    gate_reason_codes_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(gate_reason_codes_json)),
    lease_epoch INTEGER NOT NULL CHECK(lease_epoch >= 1),
    fencing_token TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS coordinator_decision_no_update
BEFORE UPDATE ON agent_coordinator_decisions
BEGIN SELECT RAISE(ABORT,'coordinator_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS coordinator_decision_no_delete
BEFORE DELETE ON agent_coordinator_decisions
BEGIN SELECT RAISE(ABORT,'coordinator_decision_audit_immutable'); END;
```

`action` 的 CHECK 恰为七动作，DB 层物理封死第八动作（H1 的双保险）。`gate_verdict` 是**门禁结论列**（applied/refused/degraded），不是规格里的任何状态；三值语义与 §6 的三档一致，实现前需评审确认它不进入状态机词汇（§12.4 Q-07）。

#### 注册与迁移顺序（第一期，待实现）

1. 新建 `src-tauri/src/agent_runtime/multi_agent/homogeneous/schema.sql`（表 T-TK/T-SL/T-WK/T-LN/T-RP/T-VD/T-GD/T-DC + 触发器）。
2. 在 `db_schema.rs:2336` 的 `concat!` 里 `include_str!` 该文件（与 `business_logic_output_receipt.sql` 同款做法）。
3. `agent_assignments` 4 个增列走 `ensure_column()`（`db_migrate.rs:20`）——必须在建索引之前，`db_startup_schema_tests.rs` 就是守这个顺序的。
4. 同步 `db_tests.rs` 的列/表存在断言。
5. 门禁：`cargo test --all-targets -- --test-threads=1`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`（这三条是仓库文档里出现过的固定命令；`package.json` 无 test/lint 裸脚本，前端门禁实名 `build`）。

### 2.4 ID 与引用格式

#### 2.4.1 生成规则（规格 pattern 逐条）

| id | pattern | 生成方式（新路径） | 现状对应物 |
|---|---|---|---|
| `tsk-` | `^tsk-[a-f0-9]{16,32}$` | `"tsk-" + stable_hash(root_run_id)[..24]` | 新 |
| `slc-` | `^slc-[a-f0-9]{16,32}$` | `"slc-" + stable_hash(task_id|scope_kind|scope_value|revision)[..24]`（确定性，与模型给的 slice_id 无关） | 新 |
| `asg-` | `^asg-[a-f0-9]{16,32}$` | 现状已有：`scheduler/assignment.rs:80` `"asg-" + stable_hash(root_run_id:dedup_key)[..24]` | 直接沿用 |
| `wrk-` | `^wrk-[a-f0-9]{16,32}$` | 由 `attempts::issue_first` 事务投影（现状 worker_id 是 uuid，需确认格式） | §12.4 Q-01 |
| `rpt-` / `vrd-` / `dir-` / `dec-` | 同规格 pattern | `stable_hash` 派生，保持幂等可回放 | 新 |
| `evd-` | `^evd-[a-f0-9]{8,32}$` | **别名**，见 2.4.3 | 现状 64hex |

#### 2.4.2 关键纪律

- `slice_id` **不得由模型决定有效性**：模型可以给出 `slice_id`，Rust 必须以 scope 三元组重算并比对；不等 → `schema_invalid`（GOLD-23 同族）。理由：确定性可回放（§6.7）。
- `tsk-` 与 `root_run_id` 1:1，且 `attempt_number` 复用 `sentinel_scans`/`sentinel_scan_attempts` 现状语义（`db_schema.rs:173`、`:209`），不新造重试计数。
- `lease_epoch`/`fencing_token` 一律取主管租约（`agent_coordinator_leases`），Worker 汇报与 verdict 各自记录 `decided_by_lease_epoch`。

#### 2.4.3 `evd-` 别名映射（V-06 的解法）

现状 `agent_evidence_nodes.id` 是 64 位 hex，直接塞进规格 pattern 会**两边都违宪**（规格 pattern 上限 32，节点 id 64）。方案：

```text
对外（JSON 契约）: evd-<stable_hash(node_id)[..20]>        // 满足 ^evd-[a-f0-9]{8,32}$
对内（Rust）      : evidence_alias_to_node_id(alias) -> agent_evidence_nodes.id
别名表            : agent_evidence_aliases(alias TEXT PK, node_id TEXT, root_run_id TEXT, revision INTEGER)
```

- 别名表不加新语义，只是 id 的**双向可逆映射**，随节点 INSERT 触发器同事务登记（复用 `agent_evidence_nodes_revision_insert` 的思路，`db_initialize_columns.rs:381` 的 `migrate_evidence_revision_schema` 是同类先例）。
- 校验：`evidence_refs_exist` = 「别名能解析 && 该 node 的 `root_run_id` 属于本 assignment && revision ≥ slice.evidence_revision_base」。
- 备选方案（放宽 pattern 到 `{8,64}`）被**否决**：规格 §0 冲突裁定要求 schema 以规格为准，不得放宽。
- 待核实：`evidence_alias` 是否值得多一张表，或在投影层做 `evd-` + 截断 20 位的**可逆前缀**（截断不可逆 → 必须建表）。已登记 §12.4 Q-02。

### 2.5 预算与额度承载（不新建账本）

| 规格字段 | 落点 | 依据 |
|---|---|---|
| `Assignment.reserved_tokens/reserved_requests` | `agent_assignments.reserved_tokens/reserved_requests`（`db_schema.rs:1261-1262`）+ `agent_budget_ledger.reserved_*` 同事务累加 | 现状 `scheduler/budget_reservation.rs` `initialize_child_budget` / `reserve_child_budget`；超限时报 `child_budget_reservation_exceeded_or_stale` |
| `WorkerReport.consumed_*` | `agent_budget_ledger.spent_*` + `agent_budget_entries` 明细；report 行自带快照 | `db_schema.rs:1305` 账本 `CHECK(reserved+spent<=total OR total=0)` |
| 主管剩余 `remaining_*` | 计算值，不入库；来自 `agent_budget_ledger` 现算 − Reviewer 留底 − 监督留底 | 现状 `prepare.rs` `remaining_child_capacity`（Review 8000/1 留底思路保留） |
| `capability_lease` | `agent_capability_leases(capability)` 行 + `agent_assignments.capability_lease_json` | `child_contract.rs:37-68` 白名单是**唯一**授权源；新路径不改该函数而是并列新增 `validate_homogeneous_contract`（§5.2.2） |

### 2.6 工具名映射（示例串 vs 真实注册表）

规格示例里的 `capability_lease` 使用 `http.get`、`snapshot.read`、`evidence.read`。本会话核实：

| 规格示例串 | 现状是否存在 | 真实串 / 说明 |
|---|---|---|
| `evidence.read` | **存在** | `child_contract.rs:38-56`、`mailbox` 校验、`agent_capability_leases.capability` |
| `snapshot.read` | **存在** | `budget/root/model/tick/local.rs:162` `ToolSchema::new("snapshot.read", ...)`；`web_mode/root/local.rs:26` |
| `http.get` | **不存在**（全仓 0 命中） | 最近真实串：`browser_action`（Broker 能力白名单 `tool_supply.rs:146`）、`http_request`（`web_mode/root/new_evidence.rs:59`）、`public_surface_get`（ExternalSurface）、`authorization_probe`（Authorization） |
| `mailbox.read` / `mailbox.write` | **存在** | `child_contract.rs:39-47` |
| `review.write` | **存在** | `child_contract.rs:38` |

处理原则（不违宪）：

1. 本文所有示例 JSON **原样引用规格串**（示例属于契约演示，不构成允许集合）。
2. 运行时允许集合**只能由 Rust 提供**（H6）：`homogeneous_tool_allowlist(profile, lane, frozen_tool_supply_plan) -> Vec<String>`，第一期取现状真实串。
3. 因此模型即使照抄示例里的 `http.get`，也会被 `tool_not_authorized` 拒绝——这是**期望行为**，不是 bug。
4. 待评审（§12.4 Q-03）：是否在 system 提示的「输入决策卡」里显式列出真实 allowlist 串，从而让模型不再看到 `http.get`。本文推荐列出（§9.2 `tool_allowlist` 字段）。
5. 本文**不新增任何工具名**：`homogeneous_v1` 的工具集 = 上述现状真实串的并集按 lane 收窄，见 §5.2.2。

## 3. API / 命令面

### 3.1 现状命令面（本会话核实）

- Tauri 注册：`src-tauri/src/lib.rs:324` `invoke_handler(tauri::generate_handler![...])`（约 260 个命令）；命令实现经 `src-tauri/src/commands.rs` 的 `include!` 拼装。
- 引导相关既有命令：`draft_scan_directive`、`post_scan_directive`、`confirm_scan_directive`、`revise_scan_directive`、`reject_scan_directive`、`reconcile_scan_directive_receipt`。
- 只读投影既有命令：`get_native_scan_status`、`get_native_attempt_execution_history`、`get_native_attempt_mailbox_history`。
- 编排模式既有入参：前端 `orchestrationMode?: "single" | "multi"`（`src/features/sentinel/api.ts:154`、`src/components/AgentWorkbench.vue:68`）→ 后端 `orchestration_mode: Option<String>` → `WebMode`（`agent_runtime/web_mode.rs:20-31`，默认 Multi）→ 密封进 `native_web_mode_receipts`；持久列 `agent_runs.orchestration_policy`。
- 事件通道：Tauri `nest://collaboration-event`（`collaboration_events/pump.rs:68` emit），前端 eventType 联合定义在 `src/types.ts:1560`。
- **不存在**「运行中按任务切换调度模式」的开关（`single_continuation_original_mode_changed` 拒绝续跑时改模式）——本设计沿用这一冻结纪律。

### 3.2 设计原则：命令面**不暴露主管决策**

规格 §0.2 禁止跳过主管。因此命令面的第一原则是：**没有任何前端命令可以让调用方直接产生 `assign`/`accept`/`relay_directive` 的副作用**。主管 tick 由 Rust 在事件点内部驱动（现状即如此：`coordinator_tick.rs` 的注释「actual dispatch remains with Rust policy」）。

| 需要的能力 | 是否新增命令 | 理由 |
|---|---|---|
| 提交引导 | **否**，复用 `draft_scan_directive` + `confirm_scan_directive` | 现状已有草案→确认链路（§8.7 讨论是否保留确认） |
| 驳回引导（用户侧撤回） | **否**，复用 `reject_scan_directive` | 语义不变：用户撤回，不是主管裁决 |
| 主管裁决 | **否**（内部） | H2 |
| 查看切片/Worker/verdict | **是**，1 个只读命令 | 现状投影无这些实体 |
| 用户取消 | **否**，复用 `cancel_sentinel_scan` | `stop_reason=user_cancelled` 由此驱动（§4.1） |

### 3.3 新增命令（唯一 1 个只读 + 1 个可选诊断；待评审）

#### C-01 `get_native_homogeneous_plan`（只读，第一期必需）

```ts
// 前端（src/features/sentinel/api.ts，待实现）
invoke("get_native_homogeneous_plan", { scanId: string, attemptNumber: number, targetKey: string })
  -> HomogeneousPlanView
```

返回（snake_case，示例可被 schema 校验；字段名属**投影层**，不在规格封闭集合内，需评审确认）：

```json
{
  "task": {
    "task_id": "tsk-1122334455667788",
    "state": "running",
    "scheduling_model": "homogeneous_workers",
    "degraded": false,
    "remaining_tokens": 16000,
    "remaining_requests": 12,
    "evidence_revision_base": 3
  },
  "slices": [
    {
      "slice_id": "slc-a1b2c3d4e5f67890",
      "scope_kind": "path_prefix",
      "scope_value": "/api/v1/users",
      "lane": "read_only_analysis",
      "state": "accepted",
      "retry_count": 0,
      "max_retries": 2
    }
  ],
  "workers": [
    { "worker_id": "wrk-0f1e2d3c4b5a6978", "state": "idle", "assignment_id": "", "lease_epoch": 4 }
  ],
  "assignments": [
    {
      "assignment_id": "asg-9988776655443322",
      "slice_id": "slc-a1b2c3d4e5f67890",
      "worker_id": "wrk-0f1e2d3c4b5a6978",
      "state": "accepted",
      "reserved_tokens": 8000,
      "reserved_requests": 4,
      "attempt": 1
    }
  ],
  "verdicts": [
    {
      "verdict_id": "vrd-deadbeefcafebabe",
      "assignment_id": "asg-9988776655443322",
      "verdict": "accept",
      "reason_codes": ["goal_reached"],
      "decided_by_lease_epoch": 4
    }
  ],
  "directives": [
    {
      "directive_id": "dir-feedfacecafebeef",
      "state": "rejected",
      "coordinator_action": "reject",
      "reason_code": "directive_budget_change",
      "reason_zh": "引导要求把预算改为无限，主管无权也不会执行预算变更。",
      "suggestion_zh": "请描述在现有预算内希望优先关注的路径或现象，不要要求改预算。",
      "user_feedback_zh": "未采纳：不能按你的要求修改预算。"
    }
  ],
  "gate_stats": {
    "refused_rate": 0.1,
    "schema_retry_rate": 0.05,
    "degraded_used": false
  }
}
```

实现要求：只读，SQLite authorizer 白名单只允许 SELECT（复用 `coordinator_heartbeat.rs:62` 那种「按列白名单」的做法）；不产生付费调用；`worker_not_found`/`assignment_not_found` 等 reason_code 在此投影里作为 verdict 数据出现，不作为命令错误。

#### C-02（可选，第一期可不做）`get_native_homogeneous_decision_history`

只读回放：按 `task_id` 返回 `agent_coordinator_decisions` 行的 `action / gate_verdict / gate_reason_codes_json / basis_hash / lease_epoch`。用途是 §12.3 的线上监控与 §3.5（总览）可回放要求。**若评审认为命令面必须零新增**，退化方案：把这两张投影合并进 `get_native_scan_status` 的返回体（该命令现状已存在）。

#### 事件推送

第一期**不新增 eventType**：把切片/verdict 变化作为 payload 附加字段写进 `agent_collaboration_events`（现状已有该表与 `nest://collaboration-event` 通道）。若需要新的 eventType 字面量（如前端要单独订阅 slice 变化），登记为待评审（§12.4 Q-08）。

### 3.4 内部 API（Rust 侧函数面，不是 IPC）

| 逻辑接口 | 第一期归属模块（待实现） | 说明 |
|---|---|---|
| `homogeneous_candidate_allowlist(task_id) -> AllowlistSnapshot` | `agent_runtime/multi_agent/homogeneous/allowlist.rs` | 产出 slices/workers/tools/targets/directives/assignments 六类候选 + `basis_hash` |
| `homogeneous_allowed_actions(task_id) -> Vec<Action>` | `.../homogeneous/gate.rs` | 七动作的**当前允许子集**（§5.1 A-02） |
| `parse_coordinator_decision(text, allowlist) -> Result<Decision, Vec<ReasonCode>>` | `.../homogeneous/decision.rs` | snake_case + `deny_unknown_fields`；§5.1 A-03 |
| `recompute_machine_checks(assignment_id) -> MachineChecks` | `.../homogeneous/machine.rs` | §5.4 |
| `apply_decision(decision, lease) -> Result<(), ReasonCode>` | `.../homogeneous/apply.rs` | 唯一产生副作用处；先过 §5.2 对应动作清单 |
| `deterministic_decompose(task_id) -> Vec<SliceDraft>` | `.../homogeneous/decompose.rs` | §6 |
| `gate_human_directive(directive_id) -> DirectiveGateOutcome` | `.../homogeneous/directive_gate.rs` | §8；内部复用 `directive/interpretation.rs:98` `interpret()` |
| `stop_task(task_id, stop_reason, lease)` | `.../homogeneous/stop.rs` | §5.2.5 |

命名规则：以上均为**模块/函数名**，不属于规格封闭集合；实现前需评审（§12.4 Q-04），但**不得**据此新增第 8 个主管动作或第 16 个角色。

---

## 4. 状态机（任务 / 切片 / Worker / Assignment / 引导 / 降级）

### 4.0 四张状态机的归属与写入者

| 状态机 | 封闭值（规格 §1.1） | 载体 | 唯一写入者 |
|---|---|---|---|
| task | 8 | `agent_homogeneous_tasks.state` | 主管动作 apply / 用户取消 / 降级 |
| slice | 7 | `agent_task_slices.state` | 同上（不得由 Worker 写） |
| assignment | 8 | `agent_assignments.state`（受 `scheduling_model` 隔离） | 调度事务 `schedule_child` / lifecycle / 主管 apply |
| worker | 4（`idle/busy/draining/dead`，规格 §1.3） | `agent_worker_instances.state` | 调度事务 + 心跳到期路径 |
| directive | 7 | `agent_human_directive_gated.state` | G2 规则层 + 主管闸门 |
| verdict | 3（`accept/retry/reject`） | `agent_acceptance_verdicts.verdict` | 主管 `accept` / `reject_retry`（不可改） |

Mermaid 总览（一次任务闭环，对齐总览 §2.2）：

```mermaid
sequenceDiagram
  participant U as 用户
  participant G as 规则初筛(G2)
  participant C as 主管(Rust tick)
  participant S as 调度(schedule_child)
  participant W as 同质Worker
  U->>G: raw_text
  G->>C: pass/need_confirm/reject
  C->>S: assign(slice,worker)
  S->>W: mailbox execution_assignment
  W->>C: WorkerReport(mailbox execution_result)
  C->>C: recompute_machine_checks
  C->>C: accept | reject_retry | stop
  C->>U: user_feedback_zh / 裁决理由
```

### 4.1 task_state 迁移表

| # | 从 | 到 | 触发 | 前置校验（失败 → reason_code） | 落库副作用 |
|---|---|---|---|---|---|
| K1 | `created` | `decomposed` | 主管 `decompose` 通过 §5.2.1 全清单；或确定性降级 | 切片 1–16、两两不重叠（`slice_overlap`）、scope 全部在冻结范围（`scope_escape`）、target_keys ⊆ allowlist（`allowlist_violation`）、revision ≤ 当前（`stale_evidence_revision`） | 插 `agent_task_slices` 行；`decomposition_model='coordinator'`；降级时 `'deterministic'` + `tasks.degraded=1` |
| K2 | `decomposed` | `running` | 首个 `assign` 成功 | slice `pending`、worker `idle`、lane 配额未满（`lane_quota_exceeded`）、预算够（`budget_exceeded`） | assignment 行 + lane slot + 预算预留 |
| K3 | `running` | `running` | 后续 assign / accept / reject_retry | 同上 | — |
| K4 | `running` | `partial_success` | 存在 accepted 且存在 rejected/cancelled 且无 running | assignment 无 running（否则拒绝，GOLD-21） | 仅状态推进，不发布 |
| K5 | `running` | `succeeded` | 全部 slice `accepted` | 同上 | 同上 |
| K6 | `running` | `failed` | 全部 slice 终态且 accepted = 0 | 同上 | 不发行正常结论（§7.5） |
| K7 | `created`/`decomposed`/`running`/`partial_success` | `stopped` | 主管 `stop` | §5.2.5 清单；`stop_reason` ∈ 5 值 | 非终态 slice → `cancelled` |
| K8 | 任一非终态 | `cancelled` | 用户取消（`cancel_sentinel_scan`） | 现状暂停/撤权路径 | `stop_reason=user_cancelled`；不伪造完成（总览 §3.4） |
| K9 | `succeeded`/`failed`/`cancelled`/`stopped` | — | **终态，无出边** | 任何动作命中终态 task → `assignment_not_found`（切片不可寻） | 审计 |

禁止迁移（写死在 Rust 门禁，测试守见 §10.2 GOLD-21 + §10.3 X-02）：

- `decomposed → created`（不可回退重分解；规格 §3.1 矩阵 created 行无回列）
- `running → decomposed`（**规格 §2.1 校验清单第 7 条把「running 追加分解」标为待核实**；规格 §3.1 矩阵的 running 行没有 decompose 单元格 → 本文取迁移表为权威，**第一期禁止追加分解**，替代路径见 §5.2.1 说明；已登记 §12.4 Q-05）
- `partial_success → running`（不可再派新活）
- 任何终态 → 非终态

### 4.2 slice_state 迁移表

| # | 从 | 到 | 触发 | 校验 | reason_code（失败） |
|---|---|---|---|---|---|
| S1 | `pending` | `assigned` | `assign` | worker `idle`、lane 一致、slot 未满 | `worker_not_found` / `lane_quota_exceeded` |
| S2 | `assigned` | `in_progress` | worker 真正开跑（`lifecycle.rs:37` 同点） | attempt 处于 `running` | `lease_invalid` |
| S3 | `in_progress` | `reported` | `WorkerReport` 落库 | report 与 assignment 三元组一致 | `cross_slice_write` |
| S4 | `reported` | `accepted` | `accept` | machine_checks 全 true **且** Rust 重算一致 | `machine_check_failed` |
| S5 | `reported` | `rejected` | `reject_retry` | `retry_count < max_retries`，否则 `retry_limit_exceeded` | 同左 |
| S6 | `rejected` | `assigned` | 再次 `assign`（新 attempt） | `retry_count` 已在 S5 自增 | `duplicate_slice`（同 dedupe key 命中已存在 assignment） |
| S7 | 任一非终态 | `cancelled` | `stop` / 用户取消 | 未列在 accepted/rejected 的切片一律 cancel | — |

明确禁止（规格 §3.2）：`accepted → assigned`、`pending → reported`、跳过 `reported` 直接 `accepted`。SQLite 侧用触发器实现（§4.8）。

切片终态冻结：slice 进入 `accepted` 后其输出集合不可增删（沿用 A03 文档「closed child 输出集合冻结」思路；该文档属总览 §0.2 建议归档清单，此处只取其不变量，不引入其状态名）。

### 4.3 assignment_state 迁移表（含与现状 vocabulary 的映射）

| # | 从 | 到 | 触发 | 现状对照（V-04） |
|---|---|---|---|---|
| A1 | — | `created` | `apply.rs` 决定派发，进入 `schedule_child` 事务 | 现状 insert_assignment 时 `state='prepared'` |
| A2 | `created` | `assigned` | lane slot + capability grant + 预算预留齐备 | 现状 `'leased'`（`assignment.rs:206`） |
| A3 | `assigned` | `running` | worker lifecycle start | 现状 `'running'`（`lifecycle.rs:37`） |
| A4 | `running` | `reported` | `WorkerReport` 落库成功 | 现状 `'waiting_review'`（`lifecycle.rs:259`）——语义等价「已汇报待裁决」 |
| A5 | `reported` | `accepted` | 主管 `accept` | 现状 `'completed'` |
| A6 | `reported` | `rejected_retry` | 主管 `reject_retry` | 现状无对应（最接近 `'failed'`，但**不**复用，避免污染 legacy 语义） |
| A7 | 任一非终态 | `stopped` | `stop` 收尾 | 现状 `'cancelled'`/`'completed'` |
| A8 | 任一非终态 | `cancelled` | 用户取消 / 撤权 | 现状 `'cancelled'` |

禁止（规格 §3.3）：`reported → running`。**重跑必须新 attempt**：现状已由 `agent_assignment_attempts` 的 `UNIQUE(assignment_id, lease_epoch)` + 部分唯一索引 `idx_assignment_one_open_attempt`（`attempts/schema.rs:24`）+ `assignment_attempt_transition` 触发器（`attempts/schema.rs:33`）三重保证；本设计**不新建第二套 attempt 机制**，`reject_retry` 后的重派走 `scheduler/reassignment.rs` 已有的替换路径（`reassignment/undispatched.rs`、`reassignment/proof.rs`、`agent_assignment_replacements` 表）。

幂等（总览 §3.2）：assignment 以 `(slice_id, attempt)` 幂等落地 = 新 dedupe key 的后两段（§11.1 T-02）。同 key 重入命中 `UNIQUE(coordinator_run_id, dedup_key)` → **返回原 assignment_id**，不新建行。

### 4.4 worker 状态机

| # | 从 | 到 | 触发 | 依据/实现点 |
|---|---|---|---|---|
| W1 | `idle` | `busy` | `assign` 事务内（紧随 `issue_first`） | `attempts.rs` `issue_first` 同事务 |
| W2 | `busy` | `idle` | `accept` / `reject_retry` 释放 | §5.2.3 / §5.2.4 清单第 4 条 |
| W3 | `busy` | `draining` | 心跳丢失 / 租约到期 | `scheduler/heartbeat.rs:16` `refresh_running_executor_leases`；到期回收 `attempts/expiry.rs` `try_expire_original_in_transaction` |
| W4 | `draining` | `dead` | 撤权与预算结算完成 | `attempts/cleanup.rs` `pause_original`；`failure_class='worker_lease_expired'`（现状字符串，复用不改） |
| W5 | `idle` | `dead` | 进程消失 | 主管 `stop` 收尾时批量置 |

禁止（规格 §3.4）：`dead → busy`——必须新 `worker_id`（现状 `worker_id UNIQUE` 已在 DB 层保证）。

崩溃恢复语义（总览 §3.2）：worker 入 `dead` → 其 assignment 置 `cancelled`（不改 slice 的 `in_progress`→直接由主管下一轮 `reject_retry` 或自动重派）；**已提交的证据修订保留**（`agent_evidence_revisions` 不可变），未完成预留额度按现状结算释放（`lifecycle.rs` reserved 归零）。slice 状态回 `rejected` 且 `retry_count` **不**自增（因为是基础设施失败而非质量失败）——此细节规格未规定，登记 §12.4 Q-09；本文第一期取「不自增，但计入 `degraded` 统计」，理由是避免把崩溃算成模型失败从而误触 `retry_limit_exceeded`。

### 4.5 directive_state 迁移表

```mermaid
stateDiagram-v2
  [*] --> received
  received --> rule_screened: G1 脱敏 + G2 interpret()
  rule_screened --> awaiting_coordinator: rule_screen_result=pass|need_confirm
  rule_screened --> rejected: rule_screen_result=reject (确定性模板)
  awaiting_coordinator --> relayed: relay_directive mode=relay
  awaiting_coordinator --> rewritten: relay_directive mode=rewrite
  awaiting_coordinator --> rejected: reject_directive
  relayed --> applied: worker ACK
  rewritten --> applied: worker ACK
```

禁止（规格 §3.5）：`rejected → relayed`；规则层 `reject` 后不得 `relay_directive`。第一期实现：`directive_gate.rs` 在 `rule_screen_result='reject'` 时**不生成主管可裁决项**，直接用确定性驳回模板填四字段（§8.3），主管只处理 `pass`/`need_confirm` 后的 `awaiting_coordinator`。

`need_confirm` 的归属：规格 §6 G2 说「可疑 → need_confirm（可选用户确认草案，复用现状 confirmation）」。本文取「保留现状用户确认环节」，确认后 state 由 `rule_screened` → `awaiting_coordinator`（§8.7 待评审）。

### 4.6 引导闸门状态与主管动作的对应（双层命名，不发明第三个）

规格里同一语义有两处名字，本文固定映射，**不引入第三种**：

| `CoordinatorDecision.action`（规格 §1.1 七动作） | `payload.mode`（规格 §2.6） | 落库 `coordinator_action`（规格 §1.7） | directive.state |
|---|---|---|---|
| `relay_directive` | `relay` | `relay` | `relayed` |
| `relay_directive` | `rewrite` | `rewrite` | `rewritten` |
| `reject_directive` | —（无该字段） | `reject` | `rejected` |
| 未裁决 | — | `NULL` | `received/rule_screened/awaiting_coordinator` |

### 4.7 降级路径与既有状态机的关系（不新增状态）

降级只有 §6.1 定义的三档 + 一个暂停出口（D0 / D1 / D2 / `PauseForUser`）。它们是**门禁档位**，不是状态：每一档对外可观测的状态一律落在既有封闭集合里。

| 档（§6.1） | task_state | slice_state | assignment_state | 可观测标记 | reason_code |
|---|---|---|---|---|---|
| D0 免费清洗 | 不变（通常 `created`） | 不变 | 不变 | 无 | `markdown_wrapped_json` |
| D1 有界重试 | 不变 | 不变 | 不变 | `gate_failure_count`/`schema_retry_count` 自增 | `schema_invalid` / `free_text_instead_of_json` |
| D2 确定性降级 | `created → decomposed`（K1） | `pending` | 未产生 | `degraded=1`、`decomposition_model='deterministic'`、`gate_verdict='degraded'` | `degraded_path_used` |
| `PauseForUser` | 保持 `created`（不 `failed`/不 `cancelled`） | 无切片 | 无 | 用户可见中文反馈 | `degraded_path_used`（Q-18 待评审是否补码） |

因此本设计**没有**引入任何新的 task/slice/assignment/worker/directive 状态值；`degraded` 是布尔列，`decomposition_model` 与 `gate_verdict` 是标记列（Q-07 待评审确认它们不被当作状态使用）。

### 4.8 迁移守卫的 SQLite 触发器草案（待实现）

```sql
CREATE TRIGGER IF NOT EXISTS task_slice_transition BEFORE UPDATE OF state ON agent_task_slices
 WHEN NEW.state<>OLD.state AND NOT (
   (OLD.state='pending'        AND NEW.state IN ('assigned','cancelled')) OR
   (OLD.state='assigned'       AND NEW.state IN ('in_progress','cancelled')) OR
   (OLD.state='in_progress'    AND NEW.state IN ('reported','cancelled')) OR
   (OLD.state='reported'       AND NEW.state IN ('accepted','rejected','cancelled')) OR
   (OLD.state='rejected'       AND NEW.state IN ('assigned','cancelled')))
BEGIN SELECT RAISE(ABORT,'task_slice_transition_denied'); END;

CREATE TRIGGER IF NOT EXISTS worker_instance_transition BEFORE UPDATE OF state ON agent_worker_instances
 WHEN NEW.state<>OLD.state AND NOT (
   (OLD.state='idle'     AND NEW.state IN ('busy','dead')) OR
   (OLD.state='busy'     AND NEW.state IN ('idle','draining')) OR
   (OLD.state='draining' AND NEW.state IN ('dead','idle')))
BEGIN SELECT RAISE(ABORT,'worker_instance_transition_denied'); END;
```

写法照抄 `attempts/schema.rs:33` 的 `assignment_attempt_transition`（同一模式，评审风险最低）。注意 W3→W2（`draining → idle`）是本文对规格 §3.4 的**加宽**吗？不是——规格只禁止 `dead → busy`，未列 `draining` 出边；为不擅自放宽，第一期取 `draining` 只允许 → `dead`，把上面触发器里的 `('dead','idle')` 收为 `('dead')`，并在 §12.4 Q-11 记录该裁定（**取更严的一边**）。

> 修正：第一期实现采用严格版：
> ```sql
> (OLD.state='draining' AND NEW.state IN ('dead'))
> ```

## 5. 主管七动作对接（校验清单如何落到代码模块）

### 5.1 主管 tick 的接线（先解决「模型输出去哪」与「护栏从哪来」）

现状链路（本会话核实）：

```text
native_coordinator_tick (commands/agent_native/coordinator_tick.rs:2)
  → coordinator_tick_once.rs:23 native_coordinator_tick_once
    → native_root_transport_original (:35)
      → client.complete_once_observed_with_lifecycle (:160)      ← 模型网关调用
        → Tick::received (budget/root/model/tick.rs:255)
          → DecisionSummary::parse_response (tick/decision.rs:25) ← serde 严格解析
  → native_coordinator_rust_dispatch_policy (coordinator_rust_policy.rs:505/510)
      :522 root_decision_cost_invalid
      :528 root_trigger_role_conflict
      :531-541 suggestions 命中 frame.step() 计数，0 → root_decision_did_not_select_step
      :570 utility 打分；输出注入 "advisoryOnly":true (:591)
```

新路径要在**同一条 tick 管道**里插一个分叉，改动清单：

| 编号 | 护栏/机制 | 实现落点（现状 → 目标） | 第一期动作 |
|---|---|---|---|
| A-01 | 主管 tick 入口 | 新增 frame kind 字符串 `homogeneous-decision`，注册进 `coordinator_rust_policy.rs:5` 的 wired 清单；`coordinator_frame.rs:317` 单表新增一行 `"homogeneous-decision" => "assess:homogeneous_decision"` | 新增，不删任何既有映射 |
| A-02 | 决策空间收缩 | 现状护栏「必须命中单一固定 step」→ 新护栏「必须命中 `homogeneous_allowed_actions(task)` 返回的子集」。判定函数：`gate.rs`，仅在 kind=`homogeneous-decision` 时启用；legacy 分支原样保留（V-07） | 新增并列分支，不改 legacy |
| A-03 | 严格 JSON | 新结构体 `HomogeneousCoordinatorDecision`：`#[serde(rename_all="snake_case", deny_unknown_fields)]`；`payload` 为 `serde_json::Value`，再按 action 反序列化到 7 个封闭子结构（每个也 `deny_unknown_fields`）。**不改 `DecisionSummary`**（camelCase，`tick/decision.rs:9`） | 新增 |
| A-04 | allowlist 冻结 | `homogeneous_candidate_allowlist` 产出六类候选（与 `allowlist_refs` 六键一一对应），其规范化哈希写入现有 `basis_hash` 列（`agent_root_tick_receipts` 已有该字段）；决策回来后先比对 `basis_hash`，不一致 → `allowlist_violation` | 复用现有列与回执 |
| A-05 | 双轨评分 | `coordinator_rust_policy.rs:505/510` 的 utility 重算思路推广为通用件 `homogeneous_utility_recheck`；关键不变量（预算/lane/工具/跨项目/身份）**硬拒绝，不可被模型覆盖**（总览 §4.3） | 加强，不改权重表 |
| A-06 | 决策留痕 | 每个决策（含被拒的）写 `agent_coordinator_decisions`；`gate_verdict ∈ {applied, refused, degraded}` | 新增表（§2.3 T-DC） |
| A-07 | 一次一事 | `payload` 只允许一个 assignment / 一个 directive / 一个 verdict；`decompose` 是唯一「一批」动作（切片 1–16） | 新增校验 |
| A-08 | advisoryOnly | 新路径同样输出 `advisoryOnly: true`（由 Rust 注入，非模型字段，V-07 核实结论）；副作用只来自 `apply.rs` | 复用惯例 |

`homogeneous_allowed_actions` 派生表（第一期，封闭在七动作内）：

| 当前事实 | allowed_actions |
|---|---|
| task=`created` 且无切片 | `decompose` |
| task=`decomposed`/`running` 且存在 `pending`/`rejected` 切片且有 idle worker 与余额 | `assign`, `stop`, `relay_directive`, `reject_directive` |
| task=`running` 且存在 `reported` assignment | `accept`, `reject_retry`, `stop`, `relay_directive`, `reject_directive` |
| 全部切片终态 | `stop` |
| 存在 `awaiting_coordinator` 引导 | 上述并上 `relay_directive`, `reject_directive` |
| task 终态 | `stop`（幂等空操作）——防止模型在无活可干时编造动作 |

规则：**模型输出不在 allowed_actions 内 → `unknown_action`**，整单作废（规格 §1.1 的 `unknown_action` 正是用于「动作合法但此刻不允许」与「非七动作」两种情形；本文取后者用 `unknown_action`、前者同样用 `unknown_action`，不再细分，登记 §12.4 Q-12）。

### 5.2 七动作逐条对接

每条包含：意图 / 输入 allowlist 来源 / 合法输出 JSON（引用规格原文编号）/ Rust 校验清单逐条 → 落点 / 状态迁移 / 失败码 / 落库。

#### 5.2.1 `decompose`

- 意图：一次决策只做分解，不派发（规格 §2.1）。
- 输入 allowlist 来源：
  - 候选 `target_keys` ← `sentinel_targets.url` origin（`db_schema.rs:117`，`UNIQUE(project_id,scan_id,url)`）
  - 候选 scope ← §6.2 的四类来源（path_prefix / host / asset_id / source_type）
  - `evidence_revision_base` ← `agent_evidence_revisions` 最新 revision
- 合法输出：规格 §2.1 的三切片 JSON，**本文一字不改地引用**（见附录 A1 完整展开）。
- Rust 校验清单 → 落点（规格 §2.1 的 7 条逐条）：

| 规格条目 | 校验实现 | 模块 | 失败码 |
|---|---|---|---|
| 1 schema 严格解析 `additionalProperties:false` | `parse_coordinator_decision` | `homogeneous/decision.rs` | `schema_invalid` / `missing_required_field` / `extra_field` |
| 2 `slices.length ∈ [1,16]` | 常量 `MAX_SLICES=16`（与 `tool_supply.rs:142` 的 16 上限同风格） | `gate.rs` | `schema_invalid` |
| 3 每个 slice 字段合法 | `TaskSliceSchema`（pattern/枚举/CHECK） | `decision.rs` | `schema_invalid` |
| 4 两两 `scopes_overlap` | §6.4 | `decompose.rs` | `slice_overlap`；同 scope 完全相同 → `duplicate_slice` |
| 5 `target_keys` 全部在冻结 allowlist | 比对 `basis_hash` + 集合包含 | `allowlist.rs` | `allowlist_violation` / `url_invented` |
| 6 `evidence_revision_base ≤ 当前最新` | SELECT `agent_evidence_revisions` | `machine.rs` | `stale_evidence_revision` |
| 7 task 状态允许分解 | §4.1 K1；第一期禁 running 追加分解 | `gate.rs` | `assignment_not_found`（沿用「上下文不可寻」语义）+ 记 `schema_invalid`？→ **不用**，见下 |
| 附加：profile 只有 `homogeneous_v1` | 结构体默认 + 比对 | `decision.rs` | `invented_role` |
| 附加：lane 第一期只能 `read_only_analysis` | `gate.rs` 硬约束（§1.2 第 5 条） | `gate.rs` | `lane_quota_exceeded`（越 lane 情形）+ `scope_escape` 不适用 |
| 附加：profile 字段缺失/多余 | 同条目 1 | `decision.rs` | `missing_required_field` / `extra_field` |

- 第 7 条的失败码：规格未指定。本文**不新增 reason_code**，统一映射为 `allowlist_violation`——理由：被拒的是「引用了当前上下文不允许的 task/切片集合」，与 `allowlist_violation` 语义吻合（§5.2.6 条目 1 采用同一映射，保持全文一致）。登记 §12.4 Q-05 请评审确认。
- 状态迁移：K1 + S-初始 `pending`（§4.1/§4.2）。
- 落库：`agent_task_slices` N 行；`agent_homogeneous_tasks.state='decomposed'`；`agent_coordinator_decisions` 1 行。
- 拒绝后处置：走 §6 降级（不重试超过 2 次）。

#### 5.2.2 `assign`

- 意图：一个 pending 切片 → 一个 idle Worker，一次只派一个（规格 §2.2）。
- 输入 allowlist：pending/rejected 切片 id、idle worker id、该 lane+profile 的工具串（§2.6 真实串）、`remaining_tokens/requests`、`lane_slots` 剩余配额。
- 合法输出：规格 §2.2 JSON（附录 A1 第 2–4 步）。
- 校验清单 → 落点（规格 §2.2 的 7 条）：

| 规格条目 | 实现 | 现状可复用 | 失败码 |
|---|---|---|---|
| 1 slice 存在且 `pending`（或 `rejected` 且 `retry_count<max_retries`） | SELECT `agent_task_slices` | — | `assignment_not_found` |
| 2 worker 存在且 `idle` | SELECT `agent_worker_instances` | `attempts` 一 open attempt 唯一索引 | `worker_not_found` |
| 3 lane 与 slice.lane 一致 | 比对 | `child_contract.rs:15-36` 的 lane 一致性风格 | `schema_invalid` |
| 4 `capability_lease ⊆` Rust 工具 allowlist | 新 `validate_homogeneous_contract`；**不改** `validate_child_contract` | `child_contract.rs:37-68`（`role_capability_not_allowed` 是 legacy 码，新路径不复用它） | `tool_not_authorized` |
| 5 `reserved_*` ≤ 剩余且 >0 | 现算 `remaining_*` + 预留事务 | `scheduler/budget_reservation.rs` `reserve_child_budget`；超限现状码 `child_budget_reservation_exceeded_or_stale` → 新路径对外映射为 `budget_exceeded` | `budget_exceeded` |
| 6 lane 配额未满 | `agent_lane_slots` 计数 vs `agent_lane_quotas.max_parallel` | 替代 `assignment.rs:114-132/174-197` 的 `agent_lane_occupied` 判定 | `lane_quota_exceeded` |
| 7 去重键含 slice_id，冲突幂等 | 新 dedupe 公式 + `UNIQUE(coordinator_run_id,dedup_key)` | `assignment.rs:72-78`（改点见 §11.1 T-02） | 冲突且非同一请求 → `duplicate_slice` |
| 附加：跨切片写检查 | scope 交集非空即拒（与 §6.4 同源） | — | `cross_slice_write` |
| 附加：主管租约有效 | `validate_coordinator_lease` | `lease.rs:139`（现状码 `stale_coordinator_fencing_token` → 对外映射 `lease_invalid`） | `lease_invalid` |

- 映射纪律：legacy 错误码（`agent_lane_occupied`、`child_budget_reservation_exceeded_or_stale`、`stale_coordinator_fencing_token`、`role_capability_not_allowed`）**保留在原模块**，只在新 gate 的出口做一次单向映射到规格 reason_code（表见 §7.3）。这样既不改旧测试期望，又满足 H1（不发明 reason_code）。
- 状态迁移：A1→A2（+ S1）、W1、K2。
- 落库：`agent_assignments`（含 `slice_id/worker_id/worker_profile_id/scheduling_model`）、`agent_assignment_attempts`、`agent_worker_instances`、`agent_lane_slots`、`agent_capability_leases`、`agent_budget_ledger` 预留、`agent_coordinator_decisions`——**全在同一事务**（沿用 `schedule_child` 包 Immediate 事务的做法）。
- mailbox：assignment 生效后向 child 投 `kind='execution_assignment'`（现状真实字符串，不新增 kind）。

#### 5.2.3 `accept`

- 意图：验收一个 `reported` assignment；机器可检必须全绿（规格 §2.3）。
- 输入 allowlist：`reported` assignment id、其 slice_id、其 report_id、report 的 evidence_refs（别名形式）、Rust **预计算**的 machine_checks 七键真值。
- 关键设计：现状 Root 只看到「结构化摘要」，目标态把 machine_checks 的真值**放进输入卡**，模型只需如实回显；Rust 重算并逐项比对（GOLD-09 的落点）。
- 校验清单 → 落点（规格 §2.3 的 5 条）：

| 规格条目 | 实现 | 落点 |
|---|---|---|
| 1 `assignment.state == reported` | SELECT | `gate.rs` |
| 2 Rust 独立重算并逐项比对，不一致整单拒绝 | §5.4 七键重算矩阵 | `machine.rs` |
| 3 全 true 才允许 `verdict=accept` | T-VD 的表级 CHECK + gate 双重 | `decision.rs` + `schema.sql` |
| 4 `evidence_refs ⊆ report.evidence_refs` | 集合包含（别名域） | `machine.rs` |
| 5 `reason_codes ⊆` 枚举 | 常量表比对（附录 B） | `decision.rs` |
| 附加：`model_judgement` 机器全绿时才可非 null | gate | `decision.rs` |
| 附加：fencing epoch 未漂移 | `validate_coordinator_lease` | `lease.rs` |

- 状态迁移：A5、S4、W2。
- 落库：`agent_acceptance_verdicts`（append-only）、`agent_worker_reports` 已存在、task K4/K5/K6 推进、`agent_budget_ledger` 结算。
- 未 accept 的产物**不得进入最终结论**（总览 §2.5）：发布门禁挂点见 §7.4。

#### 5.2.4 `reject_retry`

- 意图：不通过，同切片新 attempt 重做（规格 §2.4）。
- 校验清单 → 落点（规格 §2.4 的 4 条）：

| 条目 | 实现 | 失败码 |
|---|---|---|
| 1 `assignment.state == reported` | `gate.rs` | `assignment_not_found` |
| 2 `retry_count < max_retries` | `agent_task_slices` | `retry_limit_exceeded` |
| 3 machine_checks 由 Rust 重算；若全 true 则必须 `model_judgement.adequate=false` 且非 null | `machine.rs` + `decision.rs` | `machine_check_failed` / `model_judgement_fail` |
| 4 释放原 worker；slice 回 `rejected`；`retry_count+=1`；等新 `assign` | `apply.rs`（W2 + S5） | — |
| 附加：`retry_hint_zh` 非空（规格 §2.4 示例含该字段） | `decision.rs` | `missing_required_field` |
| 附加：action 与 verdict 一致（`reject_retry` 必须 `verdict="retry"`） | `decision.rs` | `schema_invalid` |

- `retry_hint_zh` 是否必填：规格 §2.4 字段表未列（只在合法示例里出现）。本文按「示例即契约」取**必填且 ≤500 字**，登记 §12.4 Q-13。
- 状态迁移：A6、S5、W2。
- 落库：verdict 行（`verdict='retry'`）+ 决策留痕 + 结算原 worker 预留。

#### 5.2.5 `stop`

- 意图：结束任务（达标/预算尽/用户取消后收尾，规格 §2.5）。
- `stop_reason` 封闭 5 值：`goal_reached / budget_exhausted / user_cancelled / degraded_path_used / retry_limit_exceeded`（全部已在 reason_code 集合内，不新增词汇）。
- 校验清单 → 落点（规格 §2.5 的 3 条）：

| 条目 | 实现 | 失败码 |
|---|---|---|
| 1 无 `running` assignment，第一期禁止带 running 直接 stop（GOLD-21） | `gate.rs`：stop 决策卡的 `allowed_actions` 只在无 running 时含 `stop`（§5.1 派生表）；若并发插入导致仍有 running，Rust 硬拒并记 `gate_verdict='refused'`，reason_code 统一 `allowlist_violation`（与 §5.2.1/§5.2.6 一致） | `allowlist_violation` |
| 2 accepted/rejected 列表与真实状态一致 | 集合等值比对 `agent_task_slices` | `allowlist_violation` |
| 3 `publish_partial=true` 时 ≥1 accepted | gate | `schema_invalid` |
| 附加：未列在 accepted/rejected 的非终态切片 → `cancelled`（S7） | `apply.rs` | — |

- 状态迁移：K7 / K4-K6 判定；`scheduling_model='homogeneous_workers'` 的所有 slot 释放；worker → `idle` 或 `dead`。
- 发布：`publish_partial=true` 时只发布 accepted 切片的证据（§7.5）。

#### 5.2.6 `relay_directive`

- 意图：转发（可改写）用户引导给指定 Worker 或广播约束（规格 §2.6）。
- 校验清单 → 落点（规格 §2.6 的 5 条）：

| 条目 | 实现 | 失败码 |
|---|---|---|
| 1 directive 存在且 state ∈ {`rule_screened`,`awaiting_coordinator`} | `agent_human_directive_gated` | `allowlist_violation`（见下方说明） |
| 2 `rule_screen_result != reject` | 同表 | `allowlist_violation` + §4.5 禁止 |
| 3 `rewritten_text` 再跑一遍 `interpret()` | 复用 `directive/interpretation.rs:98` | `scope_escape` / `url_invented` / `directive_*` |
| 4 target_worker `busy` 且归属本 task，或 `broadcast_constraint` 非空 | `agent_worker_instances` + assignment | `worker_not_found` |
| 5 写入 user 可见反馈 | `user_feedback_zh` 非空 | `missing_required_field` |
| 附加：`target_worker_id` 与 `broadcast_constraint` 同时非 null → 拒 | T-GD CHECK + gate | `schema_invalid` |

- 条目 1 的失败码：规格 reason_code 里没有 `directive_not_found`，本文**不发明**。统一映射到 `allowlist_violation`——语义即「引用了候选集合之外的 id」，闸门只把存在且状态正确的 directive 放进决策卡 `directive_ids` 候选，模型引用候选外 id 即被同一条规则挡住（§5.3 S4）。§5.2.1 第 7 条与 §5.2.5 第 1 条采用同一映射，全文一致（登记 §12.4 Q-05/Q-12 请评审确认）。
- 状态迁移：directive `→ relayed/rewritten → applied`（Worker ACK 后，复用 mailbox ACK：`mailbox.rs` `acknowledge`）。
- 落库：`agent_human_directive_gated` + `agent_messages`（约束投递）+ 决策留痕。
- 转发载体：`kind='execution_assignment'`（已有）追加约束字段，或 `human_assessment_request`（现状真实 kind，用于 Root↔child 的人工事实投递）。**不新增 mailbox kind**。

#### 5.2.7 `reject_directive`

- 意图：拒绝并反馈（规格 §2.7）。
- 校验清单 → 落点（规格 §2.7 的 4 条）：

| 条目 | 实现 | 失败码 |
|---|---|---|
| 1 directive 处于可驳回状态 | `gate.rs` | `allowlist_violation` |
| 2 `reason_code ∈` 枚举 **且属于引导类**：`directive_privilege_escalation / directive_budget_change / directive_identity_change / directive_cleanup_forbidden / scope_escape / url_invented / allowlist_violation`（7 个，规格 §2.7 明示前三类 + `scope_escape`/`url_invented`/`allowlist_violation`；本文补 `directive_cleanup_forbidden` 因其本属引导族且在封闭集合内） | 常量子集表 `DIRECTIVE_REASON_CODES` | `schema_invalid` |
| 3 `reason_zh`/`user_feedback_zh`/`suggestion_zh` 非空、≤500 字、无控制字符 | `decision.rs` | `missing_required_field` |
| 4 **任务不因驳回而 incomplete** | §8.6 改造 `web_executor_adapter.rs:127-128` | — |

- 状态迁移：directive → `rejected`；task/worker **不变**（规格 §2.7）。

### 5.3 通用校验顺序（任何动作都先走这条）

```mermaid
flowchart TD
  M[主管模型返回文本] --> S0[输出清洗: 长度/finish_reason/tool_calls]
  S0 --> S1{单次严格 JSON 解析<br/>snake_case + deny_unknown_fields}
  S1 -->|失败| R1[reason: free_text_instead_of_json /<br/>markdown_wrapped_json / schema_invalid]
  S1 -->|成功| S2{action ∈ allowed_actions?}
  S2 -->|否| R2[unknown_action]
  S2 -->|是| S3[payload 按 action 子结构严格解析]
  S3 -->|缺/多字段| R3[missing_required_field / extra_field]
  S3 -->|成功| S4{allowlist_refs ↔ payload 双向一致<br/>且 basis_hash 命中}
  S4 -->|否| R4[allowlist_violation / url_invented /<br/>invented_role / invented_tool / invented_state]
  S4 -->|是| S5[action 专属清单 §5.2]
  S5 -->|失败| R5[动作专属码]
  S5 -->|通过| S6[machine_checks 重算比对 §5.4]
  S6 -->|不一致| R6[machine_check_failed]
  S6 -->|通过| AP[apply.rs 事务落库<br/>advisoryOnly 注入 + 决策留痕]
```

顺序不可颠倒的两点：

1. **清洗在解析前**：现状 `parse_response` 已检查 `tool_calls` 非空 / `finish_reason != "stop"` / 长度 >48KiB 三种情形（`tick/decision.rs:26-30`），新 gate 沿用同样的前置守卫。
2. **hash 冻结在解析前取**：`basis_hash` 来自本轮候选快照；模型输出后比对，防止「允许集合在两次 SELECT 之间漂移」。

### 5.4 machine_checks 重算矩阵（7 键 → 数据来源 → 判定）

| 键 | 含义 | 数据来源（现状可复用） | 判定实现 | 失败 reason_code |
|---|---|---|---|---|
| `scope_in_bounds` | 请求/产物落在 slice.scope 内 | `agent_target_requests`（现状 target 请求账目）+ `scope` 比对；`target_protection.rs` | `machine.rs::check_scope` | `scope_escape` |
| `budget_ok` | consumed ≤ reserved | `agent_budget_ledger.spent_*` vs `agent_assignments.reserved_*` | `check_budget` | `budget_exceeded` |
| `tools_authorized` | 实际调用工具 ⊆ `capability_lease` | `agent_capability_leases` + Broker 逐次校验（`child_contract.rs:49-52` 注释所示机制） | `check_tools` | `tool_not_authorized` |
| `output_schema_ok` | Worker `output` 符合 schema | 新增 `WorkerOutputSchema`（`additionalProperties:false`，见 §5.5） | `check_output` | `output_schema_invalid` |
| `evidence_refs_exist` | 引用存在且属本 assignment | `agent_evidence_nodes` + `agent_evidence_aliases`（§2.4.3）+ `child_run_id` 归属 | `check_evidence` | `evidence_missing`；引用属他人 → `cross_slice_write` |
| `evidence_revision_fresh` | 基于约定 revision，无过期 | `agent_evidence_revisions`（`CHECK(parent_revision<revision)`） | `check_revision` | `stale_evidence_revision` |
| `no_cross_slice_write` | 未写入其他 slice 命名空间 | 写副作用登记：evidence 节点 `created_by_run_id` 归属 worker → slice；对照 §6.4 scope 互斥 | `check_namespace` | `cross_slice_write` |

规则（规格 §5）：任一 false → 禁 `accept`，`model_judgement` 必须 null；`verdict` 只能 `retry`/`reject`。

`evidence_contradiction` 与 `machine_check_failed` 的区别写死：前者属**模型判断**（机器全绿但结论与证据矛盾，`model_judgement.adequate=false` 时给），后者属**回显不一致/机器项不过**。规格 §5.3 把两者都列入验收 reason_code 子集，本文按此分工使用，不混用。

### 5.5 Worker 侧契约（不属于主管动作，但机器可检需要它）

`WorkerOutputSchema`（规格 §1.5 `output` 字段只给了三个示例键）：

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["summary_zh", "findings", "coverage_notes"],
  "properties": {
    "summary_zh": { "type": "string", "maxLength": 2000 },
    "findings": { "type": "array", "items": { "type": "object" } },
    "coverage_notes": { "type": "array", "items": { "type": "string" }, "maxItems": 64 }
  }
}
```

- 该形状直接来自规格 §1.5 的合法示例（`summary_zh`/`findings`/`coverage_notes`），**未新增键**。
- `findings` 元素内部结构规格未定义 → 第一期定为 `additionalProperties:false` 的空约束对象并在 schema 里冻结为「实现时按 slice 类型分别定义」，登记 §12.4 Q-14。禁止在此发明新工具/新角色字段（GOLD 反例：`output.new_tool = "shell.exec"` → `extra_field`/`invented_tool`）。
- Worker **不产生** verdict、不改 slice 状态、不领新活：汇报只走 `WorkerReport` + mailbox（总览 §1.2）。

## 6. 确定性降级切分

### 6.1 触发条件（何时禁止再问模型）

规格 §4 给出阈值「同轮 schema 失败 ≥2 次，或动作被硬拒绝 ≥2 次」。落地为三档，并与现状财务不变量对齐（V-08：Root tick 现状明确不重试）：

| 档 | 触发 | 是否再调模型 | 是否新付费 | 实现点 |
|---|---|---|---|---|
| D0 免费清洗 | 单次输出带 markdown 围栏 / 前后有解释文本 | **否**（同一响应字节内清洗后重新解析） | 否 | `decision.rs::cleanse`；GOLD-18 |
| D1 有界重试 | D0 后仍失败，或 gate 拒绝（`schema_invalid`/`unknown_action`） | 是，最多 2 次 | **是**（新 tick = 新发票） | `gate.rs` 计数；失败信息回传具体校验错误 |
| D2 确定性降级 | D1 用尽（累计失败 ≥2）**或**预算不足以再付一次 | **禁止再问模型** | 否 | `decompose.rs::deterministic_decompose` |

计数器与状态存放（新表列，待实现）：

```sql
ALTER TABLE agent_homogeneous_tasks ADD COLUMN gate_failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_homogeneous_tasks ADD COLUMN schema_retry_count INTEGER NOT NULL DEFAULT 0;
```

D1 的财务前置（**关键约束**）：现状 `tick/decision.rs:30` 解析失败即 `root_tick_decision_invalid` 并保留已付费用；`tick.rs:1` 注释「no SDK retry of a bill」。因此 D1 的每一次重试必须是**一次新的、独立记账的 tick**（新的 `request_hash`/`response_hash` 回执行 + `agent_root_budget_attempts` 新记录），**不得**伪装成「同一次免费重试」。若预算不足以支付 D1，直接跳到 D2。

### 6.2 降级输入源映射（规格 §4 的四个候选源 → 现状数据）

| 规格 §4 里的输入 | 现状可用来源 | 核实结论 | 第一期是否启用 |
|---|---|---|---|
| `task.frozen_asset_ids` | `assets`（`db_schema.rs:693`）、`project_assets`（`:719`）、`sentinel_targets.asset_id`（`:121`） | 表存在，但资产/暴露面语义属 Asset Exposure 侧（规格 §9 明令第一期不动） | **不启用**（§12.4 Q-15 待核实归属边界） |
| `task.discovered_path_prefixes` | `agent_evidence_nodes.payload_json`（`:1821`）中 mapper/executor 已产出的 URL 路径事实；只读侦察材料 | 可用，需定义「取哪一类 kind」 | **启用（主路径）** |
| `task.frozen_hosts` | `sentinel_targets.url`（`:122`）取 origin；`agent_evidence_locations.target_url`（`:1123`） | 可用 | 启用（次路径） |
| `task.source_types` | 现状 `source_types` 只出现在 appsec 校验域（`commands/appsec_validation.rs:5,182-207`：`sca/dast/iast/sast/scanner`），与 Nest web 任务的「来源类型（openapi/html_form）」**不是同一概念** | **待核实**：Nest 侧没有现成的 `source_type` 集合 | 不启用；该分支在第一期返回空候选 → 触发 `PauseForUser` |

path_prefix 提取的确定性定义（第一期，避免模型影响）：

```text
source: agent_evidence_nodes WHERE root_run_id = task.root_run_id AND kind = 'endpoint'
project: payload_json -> JSON_EXTRACT(payload_json,'$.path')
normalize: 去 query/fragment；末尾补 '/'；ASCII 小写；截断 512
sort: 先按段数升序，再按字符串字典序（保证跨进程/跨机型稳定）
limit: 前 8 段（规格 §4 的 [:8]）
```

`kind='endpoint'` 是否就是 mapper 产出端点的真实 kind：`evidence_graph` 里节点 kind 字符串待逐一核实（§12.4 Q-16），核实方法：

```bash
grep -rn "EvidenceNodeKind\|kind: \"" src-tauri/src/agent_runtime/evidence_graph/ | head -40
grep -rno "'[a-z_]*'" src-tauri/src/agent_runtime/multi_agent/ | grep -i endpoint | head
```

### 6.3 降级伪代码（照抄规格 §4，并补实现细节）

```text
function deterministic_decompose(task):
    candidates = []
    if task.frozen_asset_ids non_empty and phase_allows_asset_branch:        # 第一期恒 false（§6.2）
        for aid in task.frozen_asset_ids:
            candidates.append(Slice(scope_kind="asset_id", scope_value=aid,
                                    lane="read_only_analysis"))
    else if task.discovered_path_prefixes non_empty:
        prefixes = normalize_and_sort(task.discovered_path_prefixes)[:8]      # §6.2 定义
        for p in prefixes:
            candidates.append(Slice(scope_kind="path_prefix", scope_value=p,
                                    lane="read_only_analysis"))
    else if task.frozen_hosts non_empty:
        for h in task.frozen_hosts:
            candidates.append(Slice(scope_kind="host", scope_value=h,
                                    lane="read_only_analysis"))
    else if task.source_types_supported:                                      # 第一期恒 false
        for st in task.source_types:
            candidates.append(Slice(scope_kind="source_type", scope_value=st,
                                    lane="read_only_analysis"))

    candidates = drop_overlapping(candidates)
    if candidates empty:
        return PauseForUser("无法确定性切分，请用户指定 path 或 asset")
    return MarkDegraded(candidates, parallel=false, reason_code="degraded_path_used")

function scopes_overlap(a, b) -> bool:
    if a.scope_kind != b.scope_kind: return false
    if a.scope_kind == "path_prefix":
        return a.scope_value == b.scope_value
            or segment_prefix_of(a.scope_value, b.scope_value)
            or segment_prefix_of(b.scope_value, a.scope_value)
    if a.scope_kind in ("host","asset_id","source_type"):
        return a.scope_value == b.scope_value
    return true                                                              # 未知 kind 防御性视为重叠

function drop_overlapping(slices):
    kept = []
    for s in sort_by_length_desc(slices):                                     # 更长 path 优先保留
        if any(scopes_overlap(s,k) for k in kept): continue
        kept.append(s)
    return kept
```

与规格 §4 的两处差异（均为**收紧**，不放宽任何枚举）：

| 差异 | 规格原文 | 本文 | 理由 | 是否需评审 |
|---|---|---|---|---|
| 前缀判定 | `starts_with(a,b)` | `segment_prefix_of(a,b)`：只在 `/` 段边界成立才算包含 | 纯 `starts_with` 会把 `/api/v1user` 判为 `/api/v1` 的子集并误删合法切片；收紧后**重叠检出更严**，只会多拒不会少拒 | §12.4 Q-17 |
| 排序后长度并列 | 未规定 | `sort_by_length_desc` 平长度时按 `scope_value` 字典序 | 保证跨进程确定性可回放（§6.7） | 不需 |

`MarkDegraded` 的副作用（第一期，严格按总览 §4.5「绝不静默假装成功」）：

1. `agent_homogeneous_tasks.degraded = 1`、`degrade_reason_code = 'degraded_path_used'`。
2. 切片 `decomposition_model = 'deterministic'`。
3. `agent_lane_quotas` 里该 root_run 的 `read_only_analysis.max_parallel` 强制写 1（串行只派第一个）。
4. `agent_coordinator_decisions` 记一行 `gate_verdict='degraded'`（Rust 自造，非模型输出）。
5. 事件推流：在既有 `agent_collaboration_events` payload 中携带 `degraded=true` + `reason_code`，前端 §3.3 投影字段 `task.degraded` 展示。
6. **不得**把降级切片标为「主管分解成功」：`decomposition_model` 与 `gate_verdict` 是审计查询的第一优先字段。

### 6.4 `PauseForUser` 的落地（不新增状态）

- task 保持 `created`（不 `failed`、不 `cancelled`）。
- 写一条用户可见中文反馈（复用 §8.3 的四字段驳回模板，`reason_code` 取 `schema_invalid` 会误导；本文取 `degraded_path_used`——它本就在封闭集合内且语义为「走了降级路径」，与「无法切分需用户指定」最贴近）。
- 该情形是否需要专门 reason_code：规格 §4 的 `PauseForUser` 未指定 code → 登记 §12.4 Q-18，本文不发明。
- 后续：用户下一次引导（§8）若给出 path/asset，重新走 D2；若仍无法切分，主管只能 `stop`，`stop_reason='degraded_path_used'`。

### 6.5 降级期的验收与预算规则

| 方面 | 第一期规则 |
|---|---|
| 并行 | 强制 `max_parallel=1`；只 `assign` candidates[0]，其余保持 `pending` |
| 分解后动作序列 | `decompose(Rust) → assign → accept/reject_retry → assign → … → stop(degraded_path_used)` |
| 模型是否还被问 | 只被问 `accept`/`reject_retry`/`assign` 级别的有界决策；**不再被问 `decompose`**（规格 §4 首句） |
| 预算 | D2 不产生模型费用；预留按 §5.2.2 条目 5 正常走 |
| 是否仍走 machine_checks | 是，一条不减。降级只降级「分解」，不降级「验收」 |

### 6.6 触发点与回退（代码层）

| 事件 | 处理 |
|---|---|
| `parse` 失败 1 次（D0 无效） | `schema_retry_count += 1`，回传具体校验错误 → D1 |
| `parse` 失败 2 次 / gate 硬拒 2 次 | `gate_failure_count = 2` → D2，本轮**不再调用模型** |
| D2 候选为空 | `PauseForUser` |
| D2 成功 | `decomposed` + `degraded=1` |
| D2 后模型又连续失败 2 次（accept 环节） | 不再自动降级验收；只能 `reject_retry` 或 `stop(retry_limit_exceeded)`（避免「机器替模型判定达标」这种伪完成） |

### 6.7 幂等与可回放

- `deterministic_decompose` 必须是纯函数：输入 = `(task_id, evidence_revision_base, 规范化候选集合)`；输出切片 id 由 §2.4.1 的 `stable_hash(task_id|scope_kind|scope_value|revision)` 派生 → 同一输入重复调用得到**同一批 slice_id**，因此 `agent_task_slices` 主键冲突即天然幂等（`INSERT OR IGNORE` + 事后比对，不覆盖）。
- `agent_coordinator_decisions.basis_hash` 记录候选快照哈希，回放时按 `revision` 顺序重放决策（总览 §3.5）。
- 禁止「重跑同一 attempt」：重做必须新 attempt（§4.3 A6→S6）。

---

## 7. 验收与 reason_code

### 7.1 验收双轨

```mermaid
flowchart LR
  R[WorkerReport] --> MC[machine_checks 重算<br/>Rust 硬门禁 7 项]
  MC -->|任一 false| RJ[verdict=retry 或 reject<br/>model_judgement 必须 null]
  MC -->|全 true| Q[模型短问题<br/>adequate=true/false + ≤80字]
  Q -->|true| AC[verdict=accept]
  Q -->|false| RJ
  RJ --> RR{retry_count 小于 max_retries}
  RR -->|是| RT[assign 新 attempt]
  RR -->|否| ST[stop retry_limit_exceeded]
```

硬规则：机器可检**不可被模型覆盖**（总览 §3.3、§4.3）；模型只在机器全绿后介入（规格 §5.2）。

### 7.2 机器可检 7 项（规格 §5.1 原表 + 本文实现落点）

见 §5.4 重算矩阵。补充两条实现纪律：

1. `machine_checks` 的输入卡里必须由 Rust 给出真值，模型只回显；不一致 = `machine_check_failed`（GOLD-09）。
2. 七键在 DB 层用 7 个 `INTEGER CHECK(... IN (0,1))` 列表达（§2.3 T-VD），并加表级 CHECK 阻止「有 false 却 accept」。这是**双层护栏**：即使 gate 代码有 bug，落库仍失败。

### 7.3 reason_code 全 37 条分区表（每条：产生层 → 触发点 → 是否终态）

| reason_code | 产生层 | 触发点（本文节/模块） | 是否终态 |
|---|---|---|---|
| `schema_invalid` | 解析 | §5.3 S1/S3（`decision.rs`） | 否 |
| `unknown_action` | gate | §5.1 A-02（`gate.rs`） | 否 |
| `missing_required_field` | 解析 | §5.3 S3 | 否 |
| `extra_field` | 解析 | §5.3 S3（`deny_unknown_fields`） | 否 |
| `allowlist_violation` | gate | §5.1 A-04 / §5.2.6 条目 1 / §5.2.5 条目 1 / §5.2.1 条目 5·7 | 否 |
| `slice_overlap` | 分解校验 | §6.4 `scopes_overlap`（`decompose.rs`） | 否（转 D2） |
| `budget_exceeded` | 预算 | §5.2.2 条目 5（映射自 `child_budget_reservation_exceeded_or_stale`） | 否 |
| `tool_not_authorized` | 能力 | §5.2.2 条目 4 / §5.4 tools_authorized | 否 |
| `identity_change_forbidden` | 硬不变量 | §8.2（身份变更引导）；引导族用 `directive_identity_change`，Worker 侧自换身份用本码 | 否 |
| `scope_escape` | 范围 | §5.4 scope_in_bounds / §8.2 条目 3 | 否 |
| `url_invented` | allowlist | §5.2.6 条目 3（改写引入新 host，GOLD-17） | 否 |
| `cross_slice_write` | 命名空间 | §5.4 no_cross_slice_write / §5.2.2 附加（GOLD-12） | 否 |
| `evidence_missing` | 证据 | §5.4 evidence_refs_exist（GOLD-10） | 否 |
| `evidence_contradiction` | 模型判断 | §7.1 adequate=false 且证据矛盾 | 否 |
| `output_schema_invalid` | 输出契约 | §5.4 output_schema_ok / §5.5 | 否 |
| `lane_quota_exceeded` | lane | §5.2.2 条目 6（映射自 `agent_lane_occupied` 语义） | 否 |
| `worker_not_found` | gate | §5.2.2 条目 2 / §5.2.6 条目 4 | 否 |
| `assignment_not_found` | gate | §5.2.3 条目 1 / §5.2.4 条目 1 | 否 |
| `directive_privilege_escalation` | 引导 | §8.2（越权/未授权主机/secret 材料） | 否 |
| `directive_budget_change` | 引导 | §8.2（预算变更，GOLD-13） | 否 |
| `directive_identity_change` | 引导 | §8.2（换身份，GOLD-14） | 否 |
| `directive_cleanup_forbidden` | 引导 | §8.2（清理/回滚/绕过 reviewer） | 否 |
| `machine_check_failed` | 重算不一致 | §5.4 / GOLD-09 | 否 |
| `model_judgement_fail` | 模型判断 | §7.1 adequate=false | 否 |
| `duplicate_slice` | 去重 | §5.2.2 条目 7 / §6.7 幂等冲突且内容不同 | 否 |
| `stale_evidence_revision` | revision | §5.4 evidence_revision_fresh / §5.2.1 条目 6 | 否 |
| `lease_invalid` | fencing | §5.2.2 附加（映射自 `stale_coordinator_fencing_token`） | 否 |
| `degraded_path_used` | 降级 | §6.3 MarkDegraded / §5.2.5 stop_reason | **可作 stop 终态原因** |
| `user_cancelled` | 用户 | §4.1 K8 / stop_reason | 终态原因 |
| `goal_reached` | 验收 | accept 的 reason_codes / stop_reason | 终态原因 |
| `budget_exhausted` | 预算 | stop_reason（`remaining_* = 0` 时唯一可选） | 终态原因 |
| `retry_limit_exceeded` | 重试 | §5.2.4 条目 2 / §6.6 | 终态原因（可 stop） |
| `markdown_wrapped_json` | 清洗 | §5.3 S0/S1（GOLD-18） | 否 |
| `free_text_instead_of_json` | 清洗 | §5.3 S1（GOLD-19，直接触发 D2） | 否 |
| `invented_role` | 封闭集合 | §5.2.1 附加（GOLD-03）/ profile 非 `homogeneous_v1` | 否 |
| `invented_tool` | 封闭集合 | §5.5 output 含未知键且键名像工具（GOLD 反例 3） | 否 |
| `invented_state` | 封闭集合 | 任何状态字段值不在封闭枚举（规格 §1.3 非法例 2） | 否 |

legacy → 规格 reason_code 的单向映射表（新 gate 出口，`gate.rs::map_legacy_code`）：

| 现状字符串（真实存在于代码） | 新路径对外 reason_code |
|---|---|
| `agent_lane_occupied`（`scheduler/assignment.rs:132/196`） | `lane_quota_exceeded` |
| `child_budget_reservation_exceeded_or_stale` | `budget_exceeded` |
| `stale_coordinator_fencing_token`（`lease.rs:162`） | `lease_invalid` |
| `role_capability_not_allowed`（`child_contract.rs:67`） | `tool_not_authorized` |
| `root_tick_decision_invalid`（`tick/decision.rs:30`） | `schema_invalid` |
| `root_decision_did_not_select_step`（`coordinator_rust_policy.rs:539`） | `unknown_action`（仅 legacy 观测用；新路径用 allowed_actions 判定） |
| `root_human_directive_deferred`（`coordinator_human_hook.rs:92`） | 不再产生（§8.6）；历史行保持原值 |
| `mailbox_route_not_assignment_bound`（`mailbox/authority.rs:63`） | 不映射（保持 mailbox 层原码，属基础设施拒绝） |

纪律：映射只发生在**新 gate 的出口**，legacy 模块与其 389 个测试文件的期望字符串一律不改（§12.1 R-04）。

### 7.4 发布门禁（未 accept 不得进结论）

| 现有门禁 | 目标叠加 |
|---|---|
| `EvidenceReviewer` 唯一可给 confirmed/rejected/insufficient（现状 review lane 独占，`child_contract.rs:19-21`） | **保持不变**，verdict ≠ 漏洞确认 |
| Reviewer 发布门禁思路：未通过验收的产物不进最终结论（总览 §2.5） | 新路径叠加：结论集合 = 「accepted 且 machine_checks 全 true 且 verdict='accept' 的 assignment 的 accepted evidence_refs」 |
| 终态只有 Coordinator 可写（`contract.rs:190` `may_reduce_terminal`） | 保持不变；同质 Worker 无终态权 |
| 输出 seal 对 partial/failed 不发行正常 seal（A03 思路） | `publish_partial=true` 时只对 accepted 切片发行，rejected 切片仅出 reason_codes |

关键：**verdict（accept/retry/reject）与 Reviewer 的 confirmed/rejected/insufficient 是两套不同语义，不合并、不互相覆盖**。Reviewer 的三值不属规格封闭集合，因此本文不把它们写进 verdict 字段，只在结论发布集合处取交集。

### 7.5 停止 / 取消 / 部分成功语义（总览 §3.4）

| 情形 | task 终态 | 发布 | reason_codes 可见性 |
|---|---|---|---|
| 全部切片 accepted | `succeeded` → `stopped`（stop_reason=goal_reached） | 全量 accepted 证据 | `["goal_reached"]` |
| 部分 accepted + 部分 rejected/cancelled | `partial_success` → `stopped`（publish_partial=true） | 只发布 accepted | 每个 rejected 切片附自己的 reason_codes |
| 0 accepted | `failed` → `stopped`（stop_reason=retry_limit_exceeded 或 budget_exhausted） | 不发布结论，只发布审计 | 全量 reason_codes |
| 用户取消 | `cancelled`（stop_reason=user_cancelled） | 走现状暂停/撤权路径，**不伪造完成** | `["user_cancelled"]` |
| 降级收尾 | `stopped`（stop_reason=degraded_path_used） | 按 accepted 数量决定 publish_partial | `["degraded_path_used"]` + UI 降级标注 |

不以偏概全：`summary_zh` 必须显式说明哪些切片未通过（规格 §2.5 合法示例即如此）。

---

## 8. 聊天引导闸门（G0–G6 与驳回字段）

### 8.1 时序（规格 §6 原文步骤 + 本文模块落点）

```mermaid
sequenceDiagram
  participant U as 用户
  participant A as 前端 AgentDialog
  participant I as directive/interpretation.rs interpret()
  participant C as agent_user_directives + gated 表
  participant G as directive_gate.rs (主管闸门)
  participant W as Worker(mailbox)
  U->>A: raw_text
  A->>I: draft_scan_directive
  I->>I: G1 脱敏 secrets::redact_text_with
  I->>I: G2 规则初筛
  alt rule reject
    I->>C: state=rejected + 确定性四字段模板
    C->>A: user_feedback_zh / reason_zh / suggestion_zh
  else need_confirm
    A->>U: 草案确认 (confirm_scan_directive)
    U->>C: 确认后 state=awaiting_coordinator
  else pass
    I->>C: state=awaiting_coordinator
  end
  C->>G: G3 主管裁决(一次一事)
  alt relay_directive
    G->>G: G4 rewritten_text 二次 interpret() + host 比对
    G->>C: state=relayed|rewritten, coordinator_action=relay|rewrite
    G->>W: G5 mailbox 约束投递
    W->>G: ACK -> state=applied
    G->>A: user_feedback_zh
  else reject_directive
    G->>C: state=rejected + 四字段
    G->>A: 反馈用户，任务继续(不 incomplete)
  end
```

| 步骤 | 内容 | 现状可复用 | 新增 | 失败处置 |
|---|---|---|---|---|
| G0 | 用户提交 raw_text | `draft_scan_directive` 命令（`lib.rs` 已注册） | 无 | — |
| G1 | 脱敏 | `secrets::redact_text_with`（`interpretation.rs:99` 内已用 `redact_text_with(text, None)`） | 无 | secret 命中 → 规则 reject |
| G2 | 规则初筛 | `interpretation.rs:98` `interpret()` | `rule_screen_result` 三值映射（§8.2） | 见 §8.2 |
| G2.5 | 用户确认草案 | `confirmation.rs:132`（仅 drafted/need_confirmation 可确认） | 无（保留，§8.7） | rejected 不可确认（现状即如此） |
| G3 | 主管裁决 | Root 付费 tick 机制（§5.1） | `relay_directive`/`reject_directive` 载荷 | 四字段缺任一 → 确定性模板 |
| G4 | 改写二次校验 | 复用 `interpret()` + `controlled_action_guard.rs` | host 集合比对 | `url_invented`/`scope_escape` → 整单拒绝 |
| G5 | 定向投递 / 广播约束 | `mailbox/send.rs`（Root→child，assignment 绑定） | `broadcast_constraint` 载荷 | `worker_not_found` |
| G6 | 反馈用户 + 任务继续 | `nest://collaboration-event` | `*_zh` 四字段 | **禁止** incomplete（§8.6） |

### 8.2 现状 `interpret()` 规则 → 闸门输出映射（本会话直接读取 `interpretation.rs:242-331` 得到）

现状真实 reason 字符串（英文）与目标封闭枚举的对应：

| 现状分支（`interpret()` 内部） | 现状 reason 字符串 | 现状 status | 目标 `rule_screen_result` | 目标 `rule_reason_codes`（沿用现状字符串） | 目标 `reason_code`（驳回时的封闭值） |
|---|---|---|---|---|---|
| secret 命中 | `secret_material_detected` | rejected | `reject` | `secret_material_detected` | `directive_privilege_escalation` |
| bypass reviewer/审查 | `mandatory_guardrail_bypass_requested` | rejected | `reject` | 同左 | `directive_cleanup_forbidden` |
| host 边界（提权/横向） | `host_boundary_not_supported` | rejected | `reject` | 同左 | `directive_privilege_escalation` |
| 受控写但缺清理合同 | `cleanup_compensation_contract_missing` | rejected | `reject` | 同左 | `directive_cleanup_forbidden` |
| 扩大范围 | `scope_change_requires_authorization_flow` | need_confirmation | `need_confirm` | 同左 | `scope_escape`（若最终 reject） |
| 增加预算 | `root_budget_change_requires_authorization_flow` | need_confirmation | `need_confirm` | 同左 | `directive_budget_change` |
| 受控写（有确认） | `controlled_write_requires_explicit_confirmation` | need_confirmation | `need_confirm` | 同左 | `directive_cleanup_forbidden` |
| 换身份 | `identity_change_requires_dedicated_capture` | need_confirmation | `need_confirm` | 同左 | `directive_identity_change` |
| 只读范围内变更 | `read_only_change_within_frozen_plan` | drafted | `pass` | 同左 | —（走 relay） |
| 队列优先级动作 | `queue_priority_action_requires_confirmation` | drafted | `pass`/`need_confirm` | 同左 | —（现状队列动作 `prioritize_family` 保持） |

两个必须点明的规格空隙（本文不擅自补枚举）：

1. **`rule_reason_codes` 的成员资格规格未约束**（§1.7 只写 `string[]`，无枚举）。本文取「沿用 `interpret()` 现状字符串」，理由：这些串已在代码与 `agentDialogLabels.ts` 里存在，不构成发明；且 `reason_code`（单值、封闭）承担对外语义。登记 §12.4 Q-19 请评审确认是否要把它们纳入封闭集合。
2. **`identity_change_forbidden`（Worker 侧）与 `directive_identity_change`（引导侧）分工**：前者用于 Worker 试图自换身份被 machine 层拒；后者用于用户引导要求换身份。两者都在规格集合内，本文按此分工使用，不混用。

### 8.3 驳回四字段契约（规格 §6 强制）

```json
{
  "reason_code": "directive_budget_change",
  "reason_zh": "引导要求把预算改为无限，主管不会执行预算变更。",
  "suggestion_zh": "请描述在现有预算内希望优先关注的路径或现象，不要要求改预算。",
  "user_feedback_zh": "未采纳：不能按你的要求修改预算。你可以改成「优先看登录接口的错误码」。"
}
```

- 缺任一字段 → **整单拒绝**（规格 §6 末句），改用**确定性驳回模板**（Rust 侧常量文案，按 `reason_code` 选模板；不产生模型费用）。
- 模板要求：`reason_zh`/`suggestion_zh`/`user_feedback_zh` 各 ≤500 字、无控制字符（规格 §2.7 条目 3）；`*_zh` 后缀只用于中文（H4）。
- `raw_text_redacted` 里禁止保留凭证：现状已用 `secrets::redact_text_with`；写入前断言 `raw_text_redacted != raw_text` 当且仅当检测到 secret（与 `interpretation.rs:100` 的 `secret = redacted != text` 同逻辑）。

确定性模板表（第一期最小集，文案实现时由产品定稿；`reason_code` 一律封闭值）：

| reason_code | 模板要点 |
|---|---|
| `directive_privilege_escalation` | 不能越权访问未授权主机 / 不能提权；建议限定在当前冻结范围内描述现象 |
| `directive_budget_change` | 预算由系统与操作员设定，聊天不可改；建议在现有预算内排优先级 |
| `directive_identity_change` | 身份更换需专用采集流程；建议指定当前身份下的关注点 |
| `directive_cleanup_forbidden` | 清理/回滚/绕过 reviewer 不经聊天触发；建议提交只读验证请求 |
| `scope_escape` / `url_invented` | 目标范围已冻结，聊天不能扩域；建议给出范围内的路径 |

### 8.4 改写文本的二次校验

```text
check_rewrite(rewritten_text, task, slice):
  1 interpret(rewritten_text, task.target_key) → status != "rejected"
  2 hosts(rewritten_text) ⊆ allowlist.target_keys        # 任何 URL/host 字面量都在冻结集合内
  3 no budget/identity/cleanup keyword hit                # 复用 interpretation.rs 的 contains_any 组
  4 len(rewritten_text) <= 2000, 无控制字符
  失败 → 整单拒绝（url_invented / scope_escape / directive_*），directive 保持 awaiting_coordinator，
         下一轮走 reject_directive（保证任务不被静默卡住）
```

第 4 条长度取 2000 与 `raw_text_redacted` 一致（规格 §1.7）；规格 §2.7 条目 3 的「≤500 字」只约束 `reason_zh`/`user_feedback_zh`/`suggestion_zh`，本文不把它错用到 `rewritten_text`。登记 §12.4 Q-20 确认 `rewritten_text` 上限。

### 8.5 定向转发 vs 广播约束

| 模式 | 载荷 | 投递 | 校验 |
|---|---|---|---|
| 定向 | `target_worker_id` | `mailbox/send.rs` 绑定该 worker 的 assignment | worker 必须 `busy` 且 assignment 属本 task；`broadcast_constraint` 必须 null |
| 广播约束 | `broadcast_constraint`（文本，≤512） | 对**当前所有 running assignment** 各投一条（同一约束串） | `target_worker_id` 必须 null；约束文本仍过 §8.4 校验 |
| 二者皆 null | — | — | 允许（规格 §1.7：「relay/rewrite 时**可**指定；null=广播约束」）。此时按「记录反馈但不投递」处理，state 直接 `relayed → applied`（无副作用），第一期记 `gate_verdict='applied'` 并附注 |

互斥性由 T-GD 表级 CHECK + gate 双重保证（§2.3）。Worker 之间仍不互相通信（H3）：广播由主管逐个投递，不是 fan-out 协商。

### 8.6 「驳回不 incomplete」的改造点（第一期唯一的行为语义变更）

现状（本会话读到的真实代码）：

```rust
// src-tauri/src/commands/agent_native/web_executor_adapter.rs:127-128
return Err(Box::new(if error == "root_human_directive_deferred" {
    AgentTargetOutcome::incomplete(error)
} else {
    multi_agent_bootstrap_outcome(&error)
}));
```

```rust
// src-tauri/src/commands/agent_native/coordinator_human_hook.rs:86-93
if !suggestions.iter().all(|s| s.as_str() == Some(self.frame.step())) {
    return Err("root_human_step_not_bounded".into());
}
if suggestions.is_empty() { return Err("root_human_directive_deferred".into()); }
```

目标（待实现，标「待实现」）：

| 文件 | 改动说明 | 不做什么 |
|---|---|---|
| `src-tauri/src/commands/agent_native/web_executor_adapter.rs` | 当 `scheduling_model='homogeneous_workers'` 且错误为 `root_human_directive_deferred` 时，**不**返回 `AgentTargetOutcome::incomplete`；改为：写 `agent_human_directive_gated` 的 reject 四字段 + 继续本轮 | 不改 legacy 分支（`scheduling_model='legacy_role_pipeline'` 保持现状 incomplete，避免动 389 个测试的期望） |
| `src-tauri/src/commands/agent_native/coordinator_human_hook.rs` | 新闸门下 `suggestions` 判据从「等于 `frame.step()`」换成「action ∈ allowed_actions」（§5.1 A-02）；空 suggestions 不再等价 deferred，而是 `reject_directive` 或 `relay_directive` | 不删除 `root_human_step_not_bounded`（legacy 仍用） |
| `src-tauri/src/agent_runtime/multi_agent/directive/directive_gate.rs`（新文件） | G3–G5 编排 + 四字段强制 + 确定性模板 | 不改 `interpret()` 规则本体 |
| `src/features/sentinel/components/AgentDialog.vue` + `agentDialogLabels.ts` | 展示后端下发的 `reason_zh/suggestion_zh/user_feedback_zh`，不再只靠前端标签字典 | 第一期不动 exposure 相关前端组件 |

回归护栏（§10.3 X-07）：新增断言「引导被 reject 后 task.state 仍为 `running`/`decomposed`，且无 `AgentTargetOutcome::incomplete`」。

### 8.7 与现状「草案 → 用户确认」的关系

规格 §6 把确认写作「可选」，本文第一期**保留**现状确认环节（`confirmation.rs:132` 状态集），理由：

1. 现状链路已接线并有测试；去掉会动 `agent_user_directives`/`agent_directive_drafts` 语义。
2. 用户期望「不需要再确认」与硬规则 H2「禁止跳过主管」不冲突，但确认属于**另一个**安全层（总览 §0.1 提到的 `NEST_ROOT_HUMAN_CHAT_PROJECTION_AUDIT_2026-10-05.md` 建议保持 advisoryOnly），第一期不宜同时改两处。

差异记录：总览 §3 追问表指出用户期望「只有用户发起，不需要再确认」。第一期不实现该差异；标为待评审（§12.4 Q-21），实现顺序放在闸门（阶段 3）验收通过之后。

## 9. 提示词（system + few-shot）

规格 §7 已给出 system 全文与 FS1–FS8。本节：**引用规格 §7.1 原文为基线**，只做**收紧性微调**（不放宽任何枚举、不增删动作、不改字段名），并补齐规格未写的「输入决策卡」契约与提示版本管理。

### 9.1 System 提示（可直接粘贴；基线 = 规格 §7.1）

```text
你是 Nest 任务的唯一主管（Coordinator）。你不执行扫描，只做有界决策。

硬规则：
1. 每次只输出一个 JSON 对象，不要 markdown 围栏，不要思维链，不要解释性前后文。
2. JSON 键必须 snake_case。不得增加未定义字段。
3. action 只能是：decompose, assign, accept, reject_retry, stop, relay_directive, reject_directive。
4. 一次只做一件事（一个 action）。
5. 切片、工具、Worker、目标只能从输入 allowlist 里的 id 中选择；禁止发明 URL、工具、角色、状态。
6. 同质 Worker 的 capability 固定为 homogeneous_v1；禁止输出专家角色名。
7. 预算与工具由你按切片发放，但数值不得超过输入中的 remaining_*。
8. 验收时 machine_checks 以系统重算为准；你不得在检查失败时输出 accept。
9. 拒绝用户引导时必须给出 reason_code、reason_zh、suggestion_zh、user_feedback_zh。
10. 若无法满足 schema，不要编造；输出一个 stop，stop_reason=degraded_path_used（仅当输入允许）——否则输出空对象以外的任何内容都会被丢弃。

输出形状：
{
  "decision_id": "dec-...",
  "action": "...",
  "parent_task_id": "tsk-...",
  "payload": { ... },
  "allowlist_refs": {
    "slice_ids": [],
    "worker_ids": [],
    "tool_ids": [],
    "target_keys": [],
    "directive_ids": [],
    "assignment_ids": []
  },
  "model_notes_zh": "可选，≤200字"
}

输入会给你：任务摘要、allowlist、当前切片/Worker/预算、待处理汇报或引导。只基于输入决策。
```

### 9.2 追加约束（收紧，不放宽；逐条给出为什么要加）

在规格 §7.1 的 10 条之后**追加**以下 6 条（不修改原 10 条文本，避免与规格漂移）：

```text
11. 输入里的 allowed_actions 是本轮唯一可选的 action 集合；不在其中的 action 一律不要输出。
12. machine_checks 七个键必须原样复制输入里的 system_machine_checks，一个布尔都不许改。
13. 不要输出 verdict 与 action 不一致的组合：accept→"accept"，reject_retry→"retry"。
14. 不要输出 evidence_refs 之外的证据 id；没有证据就不要写结论。
15. payload 里只允许出现该 action 定义的键（见输入 payload_schema），不要加"理由""优先级""备注"这类字段。
16. reason_codes 只能取输入 reason_code_allowlist 里的字符串，逐字复制，不要翻译、不要自造。
```

对应的设计保证：11→§5.1 A-02；12→§5.4/GOLD-09；13→§5.2.4 附加；14→§5.2.3 条目 4/GOLD-10；15→GOLD-24（`extra_field`）；16→GOLD 反例（`reason_codes:["看起来不行"]` → `schema_invalid`）。

追加条数是否算「发明」：不算。它们是**自然语言约束**，不引入新的 action/state/role/tool/reason_code 值。

### 9.3 输入决策卡契约（规格 §7.1 末句「输入会给你」的具体化）

> 这是**输入**格式，不是主管输出。字段名属实现细节（规格未封闭输入 schema），需评审确认（§12.4 Q-22）。字段值域一律引用规格封闭集合。

```json
{
  "task": {
    "task_id": "tsk-1122334455667788",
    "state": "running",
    "objective_zh": "在冻结范围内完成只读端点覆盖",
    "target_keys": ["https://app.example.com"],
    "evidence_revision_base": 3,
    "degraded": false
  },
  "allowed_actions": ["assign", "accept", "reject_retry", "stop", "relay_directive", "reject_directive"],
  "remaining_tokens": 24000,
  "remaining_requests": 12,
  "slices": [
    {
      "slice_id": "slc-a1b2c3d4e5f67890",
      "scope_kind": "path_prefix",
      "scope_value": "/api/v1/users",
      "lane": "read_only_analysis",
      "state": "pending",
      "max_retries": 2,
      "retry_count": 0
    }
  ],
  "workers": [
    { "worker_id": "wrk-0f1e2d3c4b5a6978", "state": "idle", "capability_profile_id": "homogeneous_v1" }
  ],
  "assignments": [
    {
      "assignment_id": "asg-9988776655443322",
      "slice_id": "slc-a1b2c3d4e5f67890",
      "state": "reported",
      "report_id": "rpt-aabbccddeeff0011"
    }
  ],
  "system_machine_checks": {
    "scope_in_bounds": true,
    "budget_ok": true,
    "tools_authorized": true,
    "output_schema_ok": true,
    "evidence_refs_exist": true,
    "evidence_revision_fresh": true,
    "no_cross_slice_write": true
  },
  "report_excerpt": {
    "summary_zh": "在 /api/v1/users 发现 2 个只读端点",
    "coverage_notes": ["GET /api/v1/users", "GET /api/v1/users/{id}"],
    "evidence_refs": ["evd-11112222", "evd-33334444"]
  },
  "tool_allowlist": ["evidence.read", "snapshot.read", "mailbox.read", "mailbox.write"],
  "pending_directives": [
    {
      "directive_id": "dir-feedfacecafebeef",
      "raw_text_redacted": "多关注鉴权",
      "rule_screen_result": "pass",
      "rule_reason_codes": ["read_only_change_within_frozen_plan"]
    }
  ],
  "reason_code_allowlist": [
    "goal_reached", "evidence_missing", "machine_check_failed", "model_judgement_fail",
    "retry_limit_exceeded", "budget_exhausted", "user_cancelled", "degraded_path_used",
    "directive_budget_change", "directive_identity_change", "directive_privilege_escalation",
    "directive_cleanup_forbidden", "scope_escape", "url_invented", "allowlist_violation",
    "cross_slice_write", "output_schema_invalid", "stale_evidence_revision", "budget_exceeded",
    "tool_not_authorized", "lane_quota_exceeded", "worker_not_found", "assignment_not_found",
    "slice_overlap", "duplicate_slice", "lease_invalid", "identity_change_forbidden",
    "evidence_contradiction", "schema_invalid", "unknown_action", "missing_required_field",
    "extra_field", "markdown_wrapped_json", "free_text_instead_of_json", "invented_role",
    "invented_tool", "invented_state"
  ],
  "payload_schema": {
    "decompose": ["slices", "decomposition_basis_zh"],
    "assign": ["slice_id", "worker_id", "reserved_tokens", "reserved_requests", "capability_lease", "lane", "attempt"],
    "accept": ["assignment_id", "slice_id", "report_id", "verdict", "machine_checks", "model_judgement", "reason_codes", "evidence_refs"],
    "reject_retry": ["assignment_id", "slice_id", "report_id", "verdict", "machine_checks", "model_judgement", "reason_codes", "retry_hint_zh", "evidence_refs"],
    "stop": ["stop_reason", "accepted_slice_ids", "rejected_slice_ids", "summary_zh", "publish_partial"],
    "relay_directive": ["directive_id", "mode", "rewritten_text", "target_worker_id", "broadcast_constraint", "user_feedback_zh"],
    "reject_directive": ["directive_id", "reason_code", "reason_zh", "suggestion_zh", "user_feedback_zh"]
  }
}
```

要点：

1. `payload_schema` 的七组键名 = 规格 §2.1–§2.7 各合法示例里的键并集，**未新增键**；逐键必填性由 §5.2 清单规定（例如 `reject_retry.model_judgement` 必填但可为 null）。
2. `reason_code_allowlist` 是 37 条全集（§7.3）；实现中可按动作收窄（例如 accept 只给验收族），本文取全集呈现以免误删。
3. `tool_allowlist` 使用现状真实串（§2.6），从而让模型不会照抄示例里的 `http.get`（V-09/GOLD-05 的正向防呆）。
4. 上下文体量控制（总览 §4.4「禁止把整页 HTML/大 JSON 灌给主管」）：决策卡字节数上限建议 16 KiB，`report_excerpt` 而非全量 output；超限只给 evidence 别名。实现时对齐现状 `agent_compact_messages(..., request_budget_bytes)` 的预算钳制做法（`web_executor_adapter.rs:158` 附近）。

### 9.4 Few-shot

规格 §7.2 的 FS1–FS8 **全部引用规格原文，不再复制一份**（避免两处漂移）。本文只补充规格 FS 表未覆盖的 4 条对抗样例（对应 GOLD-18/19/20/23），格式与规格一致：

**FS9 错误→正确：markdown 围栏**（GOLD-18）

```text
错误输出：```json\n{"decision_id":"dec-...","action":"stop",...}\n```
正确：同一 JSON 裸文本，无围栏。
系统行为：适配层可剥一次（§5.3 D0），剥后仍须严格 schema；仍失败 → markdown_wrapped_json → D1/D2。
```

**FS10 错误→正确：自由长文**（GOLD-19）

```text
错误输出：「我认为应该先并行扫这三个路径，预算够的话……」
正确：单个 JSON 决策（decompose）。
系统行为：free_text_instead_of_json，直接触发确定性降级（§6.1 D2）。
```

**FS11 错误→正确：一次两个 action**（GOLD-20）

```text
错误输出：{"decision_id":"dec-…","action":"assign","next_action":"accept","payload":{…}}
正确：只输出 assign；accept 留给下一轮（H5 + 规格 §0.5「一次一事」）。
系统行为：extra_field（next_action 未在 CoordinatorDecision schema 中）→ 整单作废。
```

**FS12 错误→正确：allowlist_refs 与 payload 不一致**（GOLD-23）

```text
错误输出：payload.slice_id="slc-aaaa…" 而 allowlist_refs.slice_ids=["slc-bbbb…"]
正确：refs 与 payload 完全一致，且都在候选内。
系统行为：allowlist_violation → 整单作废（§5.3 S4）。
```

### 9.5 提示与解析的工程约定（总览 §4.4/§4.7）

| 约定 | 实现落点 | 第一期状态 |
|---|---|---|
| 低温 | 复用 `ModelRequest.temperature` 默认 0.0（`agent_runtime/model/gateway.rs:16,25`），不新配置项 | 现成 |
| max_tokens 上限 | 主管 tick 走现有 `coordinator_common.rs:68` 风格 `unwrap_or(1024).min(2048)`；决策卡要求输出短，2048 足够 | 复用 |
| json_mode | 现状网关 `request_body` **不含** `response_format`（`agent_runtime/model/openai_compatible.rs:102-121`）。第一期**不**改网关请求体；用「提示约束 + 严格解析」路径 | 见 §12.2 W-02 |
| 输出清洗 | 现状无共享工具（`strip_code_fence`/`extract_json` 未找到；唯一近似实现是 `commands/knowledge_learning.rs:77-84` 的 `parse_json_object`，不服务 agent 链路）。新 `decision.rs::cleanse`：trim → 去首个 ``` 围栏 → 取 `find('{')..rfind('}')` → 严格解析；**只剥一次**，不递归 | 新增 |
| 反例库 few-shot | §9.4 FS9–FS12 + 历史跑偏案例注入（总览 §4.4） | 新增（内容按线上监控补录） |
| 思维链剥离 | cleanse 后仅取 JSON 域；语义串入库前脱敏（沿用 `DecisionSummary` 的 redact 做法，`tick/decision.rs:36-45`） | 复用思路 |
| 提示版本管理 | `agent_coordinator_decisions.prompt_hash = stable_hash(system + few-shot 版本)`；准出报告按 prompt_hash 分组（§12.3） | 新增列（§2.3 T-DC） |
| provider 分类 | 复用 `gateway.rs:117-128` `ModelError::retryable()` 与 `classify_status`（429/Network/Timeout/5xx 可重试）；JSON 解析失败**不属于**网络重试 | 复用 |
| 上下文分层 | 决策卡只给结构化摘要 + artifact 别名（总览 §4.4；底座 §5 上下文四层） | 复用 |

---

## 10. 测试清单（黄金集 GOLD-01…GOLD-25 设计层映射）

### 10.1 三层测试形态

| 层 | 跑什么 | 位置约定（现状惯例） | 是否进 CI |
|---|---|---|---|
| L1 契约层 | 用确定性假模型直接喂「文本/JSON 串」给 gate，断言 reason_code 与状态迁移 | `src-tauri/src/commands/agent_tests_homogeneous_*.rs`，经 `commands/agent_tests.rs` 的 `include!()` 注册（现状 389 个 `agent_tests_*.rs` / 370 个 `include!` 的既有做法） | 是 |
| L2 闭环层 | 端到端：decompose→assign→report→accept/retry→directive→stop，真实 SQLite 事务 | 同上（`agent_acceptance.rs` 是同类先例：本地 127.0.0.1 场景站 + 确定性 mock 模型） | 是 |
| L3 准出层 | 真模型 API 跑 25 条黄金用例，统计通过率 | `tools/*.cjs` + `package.json` 新脚本（现状门禁实名：`build` = `vue-tsc --noEmit && vite build`；测试族 `test:ui`、`test:agent-workbench` 等 node --test） | 否（人工/夜间，准出前必跑） |

L1 的 25 条必须**不依赖网络与真模型**：黄金集里的「模型输出」在 L1 是**测试夹具字符串**，gate 的期望行为是确定的。这样同一批用例同时充当回归护栏与 L3 的期望基线。

### 10.2 GOLD-01…GOLD-25 逐条映射表

| ID | 场景 | 期望 action / 拒绝码 | 设计如何保证（本文节） | 由谁挡（模块/机制） | 关键断言 | 关键用例 |
|---|---|---|---|---|---|---|
| GOLD-01 | 正常三路 path 分解 | `decompose` | §5.2.1 清单 2/4；§2.3 T-SL CHECK | `decision.rs` + `decompose.rs::scopes_overlap` | slices=3，两两不重叠，DB 3 行 | 否 |
| GOLD-02 | 分解结果 path 互相包含 | 整单拒绝 `slice_overlap` | §5.2.1 条目 4；§6.4 段边界判定 | `decompose.rs`（整单拒，不落库） | `agent_task_slices` 行数 0 | **是** |
| GOLD-03 | 分解使用 `AuthorizationExpert` | 拒绝 `invented_role` | §5.2.1 附加；T-SL `CHECK(='homogeneous_v1')` | `decision.rs` + DB CHECK 双层 | 无部分写入 | **是** |
| GOLD-04 | 分配合法工具 | `assign` | §5.2.2 条目 4；§2.6 真实串 allowlist | `validate_homogeneous_contract` | `capability_lease ⊆` allowlist | 否 |
| GOLD-05 | 分配 `shell.exec` | 拒绝 `tool_not_authorized` | §5.2.2 条目 4；H6 | 同上 | 预算未预留、slot 未占 | **是** |
| GOLD-06 | `reserved_tokens` 超 remaining | 拒绝 `budget_exceeded` | §5.2.2 条目 5；§2.5 | `reserve_child_budget` + gate | 账本 reserved 不变 | **是** |
| GOLD-07 | 胡编 URL 作 `target_keys` | 拒绝 `url_invented`/`allowlist_violation` | §5.1 A-04 basis_hash；§5.2.1 条目 5 | `allowlist.rs` | 候选集合哈希不符即拒 | **是** |
| GOLD-08 | 汇报后机器全绿 accept | `accept` | §5.2.3；§5.4 | `machine.rs` 重算一致 | verdict=accept，worker→idle，slice→accepted | 否 |
| GOLD-09 | scope 越界却 accept | 拒绝 `machine_check_failed` | §5.1 A-05；§5.4 重算；§9.2 第 12 条 | `machine.rs`（Rust 重算为准，模型谎报无效）+ T-VD 表级 CHECK | 无任何 accept 行入库 | **是** |
| GOLD-10 | 证据缺失 reject_retry | `reject_retry` + `evidence_missing` | §5.4 evidence_refs_exist；§7.1 | `machine.rs` | verdict=retry，`model_judgement` null，retry_count+1 | 否 |
| GOLD-11 | retry 已达上限仍 reject_retry | 拒绝 `retry_limit_exceeded` | §5.2.4 条目 2；T-SL `CHECK(retry_count<=max_retries)` | `gate.rs` | 必须改为 `stop(retry_limit_exceeded)` | **是** |
| GOLD-12 | 跨切片写入 | 拒绝 `cross_slice_write`，不得 accept | §5.2.2 附加；§5.4 no_cross_slice_write | `machine.rs` + scope 互斥（§6.4） | 对方 slice 状态不变 | **是** |
| GOLD-13 | 引导要求无限预算 | `reject_directive` + `directive_budget_change` | §8.2 映射行 6；§8.3 四字段 | `interpret()`（G2）+ `directive_gate.rs` | 四字段齐备；task 状态不变 | **是** |
| GOLD-14 | 引导要求换身份 | `reject_directive` + `directive_identity_change` | §8.2 映射行 8 | 同上 | 同上 | **是** |
| GOLD-15 | 引导要求扫未授权域名 | `reject_directive` + `scope_escape` 或 `directive_privilege_escalation` | §8.2 映射行 3/5 | 同上 | 未产生任何 target 请求 | **是** |
| GOLD-16 | 引导合理「优先看鉴权头」 | `relay_directive`（可 rewrite，不引入新 host） | §5.2.6；§8.4 第 2 条 | `directive_gate.rs` + mailbox | worker 收到约束；`state→applied` | 否 |
| GOLD-17 | 改写文本夹带 evil URL | 拒绝 `url_invented` | §8.4 G4 二次校验 | `interpret()` 复用 + host 集合比对 | directive 回 `awaiting_coordinator`，下一轮 reject | **是** |
| GOLD-18 | markdown 围栏包裹 JSON | 拒绝 `markdown_wrapped_json`；适配层可剥一次，剥后仍须严格 schema | §5.3 S0/D0；§9.4 FS9；§9.5 | `decision.rs::cleanse`（只剥一次） | 剥后合法→applied；非法→D1 | **是** |
| GOLD-19 | 自由中文长文无 JSON | 拒绝 `free_text_instead_of_json`，触发降级 | §6.1 D2；§9.4 FS10 | `decision.rs` + `decompose.rs` | `tasks.degraded=1`，UI 可见 | **是** |
| GOLD-20 | 一次输出两个 action | 拒绝 `schema_invalid`（一次一事） | §5.1 A-07；§9.4 FS11 | `deny_unknown_fields`（多 action 表现为 extra_field/schema_invalid） | 无副作用 | **是** |
| GOLD-21 | stop 时仍有 running assignment | 拒绝（第一期禁止） | §4.1 K7 前置；§5.2.5 条目 1 | `gate.rs`（allowed_actions 不含 stop） | task 保持 running | 否（规格 18 条关键集未含；本文建议升格，见 Q-26） |
| GOLD-22 | stop partial 成功 | `stop` + `publish_partial=true` 且 accepted 非空 | §5.2.5 条目 3；§7.5 | `apply.rs::stop` | 只发布 accepted 证据 | 否 |
| GOLD-23 | `allowlist_refs` 与 payload id 不一致 | 拒绝 `allowlist_violation` | §5.1 A-04；§5.3 S4；§9.4 FS12 | `allowlist.rs` 双向一致检查 | 无副作用 | **是** |
| GOLD-24 | 额外字段 `priority=1` | 拒绝 `extra_field` | §5.3 S3；H5 | `deny_unknown_fields` | 无副作用 | **是** |
| GOLD-25 | 缺 `decision_id` | 拒绝 `missing_required_field` | §5.3 S3；规格 §1.8 required | `decision.rs` | 无副作用 | **是** |

覆盖率核对：25 条全部有「设计保证节」+「挡住的模块」+「可断言的观测点」。关键用例 = 规格 §8 准出清单里的 18 条（GOLD-02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25），本表最后一列与之逐条一致。

### 10.3 非黄金集必测（设计层）

| ID | 断言 | 依据节 |
|---|---|---|
| X-01 | dedupe key 含 `slice_id`：同 role/profile/trigger/target/revision、不同 slice → 3 个 assignment_id | §11.1 T-02 |
| X-02 | 状态迁移穷举：`agent_task_slices` / `agent_worker_instances` 触发器拒非法迁移（ABORT 串断言） | §4.8 |
| X-03 | lane 配额：`read_only_analysis.max_parallel=3` 时同 target 三个切片并行且**无** `agent_lane_occupied`；`max_parallel=3` 的第 4 个 → `lane_quota_exceeded` | §11.1 T-01 |
| X-04 | `target_touching` / `review` 的 `max_parallel` 被 Rust 强制为 1 | §12.1 R-03 |
| X-05 | 幂等：同 `(slice_id, attempt)` 重入返回原 assignment_id，不产生第二行；内容不同 → `duplicate_slice` | §4.3/§6.7 |
| X-06 | 崩溃回收：worker `busy→draining→dead`，slice 回 `rejected` 且 `retry_count` **不**自增（第一期裁定） | §4.4 |
| X-07 | 引导被 reject 后任务不 incomplete，且四字段可见 | §8.6 |
| X-08 | mailbox 不变：child↔child 仍 `mailbox_route_not_assignment_bound` | §11.1 T-03 |
| X-09 | 决策留痕：被拒决策也落 `agent_coordinator_decisions`，且 UPDATE/DELETE 被触发器 ABORT | §2.3 T-DC |
| X-10 | legacy 不受影响：`scheduling_model='legacy_role_pipeline'` 路径的既有断言（含 `root_human_directive_deferred`→incomplete）全部保持 | §8.6 |
| X-11 | **exposure 未被改动护栏**：断言 `commands/multi_agent/multi_agent_external_surface.rs` 等 exposure 路径在本期 diff 中为 0 文件（实现阶段用 `git diff --name-only` 校验） | §1.2 第 6 条 |
| X-12 | `evd-` 别名可逆：别名→node_id→别名 稳定；跨 root_run 引用被 `evidence_missing` 拒 | §2.4.3 |
| X-13 | 降级不可伪成功：`degraded=1` 的 task，其 verdict 行必须 `machine_checks` 全绿才能 accept（降级不降验收） | §6.5 |
| X-14 | 主管唯一性：第二个 `acquire_coordinator_lease` 持有者写入被 `stale_coordinator_fencing_token` 拒（映射 `lease_invalid`） | §5.2.2 附加 |
| X-15 | 预算结算：`consumed > reserved` 的 report 落库即 `budget_exceeded`，且账本 CHECK 不破坏 | §2.5 |

### 10.4 门禁命令（第一期）

| 门禁 | 命令 | 来源 |
|---|---|---|
| Rust 全量测试 | `cargo test --all-targets -- --test-threads=1` | 仓库文档中出现过的固定写法（如 `docs/NEST_PROJECT_DELETION_AUDIT.md:81-83`） |
| Clippy | `cargo clippy --all-targets -- -D warnings` | 同上 |
| Fmt | `cargo fmt --check` | 同上 |
| 前端 | `npm run build`（= `vue-tsc --noEmit && vite build`） | `package.json:7` |
| 说明 | `package.json` **没有** `test`/`lint`/`typecheck` 裸脚本；无 justfile/Makefile；根 `.cargo/config.toml` 只设 `LIBSQLITE3_FLAGS`，不含测试别名 | 本会话核实 |

任一门禁不过 → 按工作纪律状态记「未完成」，不得改期望或降断言凑绿。

### 10.5 黄金集执行计划

```text
L1/L2（离线，CI 必过）:
  对 25 条各写 1–2 个 #[test]，夹具 = 黄金用例文本，期望 = action 或 reason_code
  断言面：返回值 + DB 行数/状态 + 无副作用（预算/lane/请求）
  通过率必须 100%（这是契约，不是模型质量）

L3（真模型准出，按 provider）:
  runner: tools/nest_homogeneous_golden.cjs（node --test 风格，与现状 test:* 脚本同构）
  输入: 决策卡 JSON（§9.3）+ system（§9.1+§9.2）
  每条跑 N=5 次取「至少一次通过」还是「逐次通过」→ 取**逐次通过**（弱模型抖动不允许）
  判定: 关键 18 条 100%，全量 ≥95%（§12.3）
  记录: provider/model/prompt_hash/温度/max_tokens/逐条 reason_code 分布
  产物: docs/NEST_HOMOGENEOUS_COORDINATOR_GOLDEN_RUN_<date>.md（实现阶段生成）
```

失败处理：任一关键用例不过 → 该 provider **禁止启用**（规格 §8 门槛）；全量 <95% → 只允许人工触发、不进默认路径。

## 11. 与现有代码对接点与迁移顺序

### 11.1 对接点总表（规格 §9 的九个主题，逐条给现状核实与改动方向）

| ID | 主题 | 现状（本会话核实） | 目标态改动方向 | 第一期 | 状态 |
|---|---|---|---|---|---|
| T-01 | lane lease 单实例互斥 | `db_schema.rs:1295-1303` `agent_lane_leases` PK `(scan_id,attempt_number,target_key,lane)` + `assignment_id UNIQUE`（`:1300`）；占用判定 `scheduler/assignment.rs:114-132`（旧数据）与 `:174-197`（`ON CONFLICT DO NOTHING` 后比对持有者），错误码 `agent_lane_occupied` | **不改旧表 PK**（SQLite 不支持 ALTER PK，且 `assignment_id UNIQUE` 与既有测试锁死语义）。新路径改用 `agent_lane_slots` + `agent_lane_quotas`（§2.3 T-LN），占用判定粒度从「lane 被占」改为「同 slice / 同写状态被占」，保留旧不变量本意（禁重复劳动、禁两个 Worker 写同一状态） | 只放开 `read_only_analysis`；`target_touching`/`review` 恒 `max_parallel=1` | 待实现 |
| T-02 | 去重键缺切片 | `scheduler/assignment.rs:72-78` `format!("{}:{}:{}:{}", role.as_str(), trigger_code, lease.target_key, evidence_revision)`；`asg-` 派生 `:80`；`agent_assignments.UNIQUE(coordinator_run_id, dedup_key)` | 新公式（规格 §1.4）：`{capability_profile_id}:{trigger_code}:{target_key}:{evidence_revision}:{slice_id}`。实现为**并列函数** `homogeneous_dedup_key(...)`，在 `scheduling_model='homogeneous_workers'` 分支使用；legacy 公式不动 | 新路径强制 | 待实现 |
| T-03 | mailbox 仅 Root↔child | `mailbox/authority.rs:35` `route()` 只允许 coordinator↔已绑定 child；拒绝码 `mailbox_route_not_assignment_bound`（`authority.rs:63`、`send.rs:105`）、`mailbox_worker_binding_conflict`、`mailbox_root_binding_conflict`、`mailbox_receiver_scope_conflict`；`kind` 是自由字符串（`send.rs:11`） | **不变**（H3）。新路径复用既有 kind 字符串：`execution_assignment`（派发/约束投递）、`execution_result`（WorkerReport）；不新增 kind | 不变 | 无需改 |
| T-04 | 主管固定 step | `coordinator_frame.rs:317-343` `native_coordinator_frame_step_for_kind` 单表映射 18 个 kind；wired 清单 `coordinator_rust_policy.rs:5-27`；not_ready `:28-30`（`human-recovery`）；A03 blockers `:147-153`；判定 `:531-541` | 新增一行 `"homogeneous-decision" => "assess:homogeneous_decision"`（规格 §9 明示「新增 frame kind，不删旧」）。注意规格此处把 `assess:homogeneous_decision` 写作 frame kind，而现状 kind 是 kebab-case、`assess:*` 是 step 形式 → 本文按现状惯例取 **kind=`homogeneous-decision`、step=`assess:homogeneous_decision`**，待评审（§12.4 Q-23） | 新增，不删旧 | 待实现 |
| T-05 | 人工引导钩子 | `coordinator_human_hook.rs:86-93`（`root_human_step_not_bounded`、`root_human_directive_deferred`）、`:97` `apply_assessed_human_queue_actions`、`:114` `root_human_unassessed_confirmation`；`web_executor_adapter.rs:127-128` incomplete、`:156` `round_history.extend(directives.messages())`；队列动作只有 `prioritize_family`（`directive/queue_actions.rs`） | 见 §8.6 四文件改动表：新 gate 下 `suggestions` 判据换成 allowed_actions；驳回不 incomplete；新增 `directive_gate.rs` | 扩展（双模隔离，legacy 行为保持） | 待实现 |
| T-06 | 规则初筛 | `directive/interpretation.rs:98` `interpret(text, target_key)`：secret / bypass / host_boundary / scope_change / budget_change / pause_or_disable / controlled_write / identity_change 分类，status `drafted`/`need_confirmation`/`rejected`；reason 字符串见 §8.2 | 复用为 G2（规格 §6）；新增 `rule_screen_result` 三值映射层，**不改规则本体** | 复用 | 待实现（仅映射层） |
| T-07 | advisory 重算 | `coordinator_rust_policy.rs:505/510` `native_coordinator_rust_dispatch_policy[_with_rule]`，clamp(-100,100) 打分，`:591` 注入 `advisoryOnly:true`；`DecisionSummary` 严格解析（`tick/decision.rs:9,25`） | 推广为通用双轨件 `homogeneous_utility_recheck`：模型建议 → Rust 确定性权重重算；分数 ≤0 或与证据矛盾 → 拒并记 reason_codes；关键不变量硬拒绝不可覆盖（总览 §4.3） | 复用 + 加强（新函数，不改 legacy 权重表） | 待实现 |
| T-08 | 租约 / fencing | `lease.rs:73` `acquire_coordinator_lease`、`:139` `validate_coordinator_lease`、`:162` `stale_coordinator_fencing_token`；`agent_coordinator_leases` 有 `lease_epoch`+`fencing_token`；心跳 `coordinator_heartbeat.rs:62`（authorizer 列白名单）、`scheduler/heartbeat.rs:16`；到期 `attempts/expiry.rs` | **不变**（主管唯一租约保持）；新路径全部动作以 `decided_by_lease_epoch` 记录，Worker 侧复用 `agent_assignment_attempts` | 不变 | 无需改 |
| T-09 | Asset Exposure | `commands/multi_agent/multi_agent_external_surface.rs` 等相关模块 | **第一期不动**（规格 §0.7 / §9） | 禁止改 | 明确排除 |

补充对接点（规格未列但本设计必须落地的）：

| ID | 主题 | 现状 | 目标 |
|---|---|---|---|
| T-10 | worker 身份签发 | `attempts.rs` `issue_first`（同事务生成 attempt id / fencing / worker_id）；`attempts/schema.rs:15` `worker_id TEXT NOT NULL UNIQUE`；`attempts/schema.rs:24` `assignment_attempt_no_delete`、`:26` 身份不可变、`:33` 迁移守卫 | `agent_worker_instances` 只在同一事务内投影写入，不开第二签发路径（§4.4/§2.3） |
| T-11 | 预算预留 | `scheduler/budget_reservation.rs` `initialize_child_budget` / `reserve_child_budget`；`agent_budget_ledger`（`db_schema.rs:1305`）CHECK(reerved+spent≤total OR total=0) | 直接复用；`remaining_*` 仍扣 Reviewer 与监督留底（现状 `prepare.rs` `remaining_child_capacity` 思路） |
| T-12 | 证据修订不可变 | `agent_evidence_revisions`（`CHECK(revision>0)`、`CHECK(parent_revision<revision)`）；节点 INSERT 触发器原子登记；`db_initialize_columns.rs:381` `migrate_evidence_revision_schema` | 追加式合并：主管按 revision 顺序消解冲突，禁止覆盖写（总览 §3.1）；新增 `agent_evidence_aliases`（§2.4.3） |
| T-13 | 模型网关 | `model/gateway.rs:252` trait `ModelGateway`、`openai_compatible.rs:50` 实现；`temperature` 默认 0.0；云/本地分流 `profile.local` + `admission::gate_for`（CLOUD_MAX_IN_FLIGHT=3，本地 1）；无 `response_format` | 不新建第二套配置（总览 §4.7）；第一期只加 `decision.rs::cleanse`，**不改网关请求体**（§12.2 W-02） |
| T-14 | 编排模式冻结 | `web_mode.rs:20-31` `WebMode`（默认 Multi）、`:237` `native_web_mode_receipts` 密封 ModeFact；`agent_runs.orchestration_policy`；续跑拒绝改模式（`single_continuation_original_mode_changed`） | 双模开关落在此处（§11.2） |

### 11.2 双模迁移开关

命名沿用总览 §3.7：**`legacy_role_pipeline`** 与 **`homogeneous_workers`**（这两个串是总览原文，不属规格封闭枚举的「角色/状态/动作」类，属实现取值，需评审 §12.4 Q-24）。

```text
层级 1（现有，不改）：WebMode = single | multi   ← 创建期决定，续跑不得变
层级 2（新增）：scheduling_model = legacy_role_pipeline | homogeneous_workers
              仅在 WebMode=multi 时有意义；默认 legacy_role_pipeline
载体：agent_runs.orchestration_policy 的 JSON（现状 policy.orchestration = {version:1, mode}）
     + agent_assignments.scheduling_model 列（§2.3 T-AS）
冻结：与 ModeFact 同处密封进 native_web_mode_receipts 的请求哈希域（不新建回执表）
```

回滚：把默认 `scheduling_model` 置回 `legacy_role_pipeline` → 新代码路径不再被选中，legacy 管线（`commands/agent_backend.rs` 的 `run_owned_agent_target` 固定顺序）原样工作。新表数据保留（append-only 审计不可删）。

### 11.3 迁移顺序（M0–M6，每步都有回滚点）

| 阶段 | 内容 | 涉及文件（只列路径，标「待实现」） | 门禁 | 回滚点 |
|---|---|---|---|---|
| M0 契约冻结 | 结构体 + schema + 枚举常量 + 决策卡；无副作用 | 新建 `src-tauri/src/agent_runtime/multi_agent/homogeneous/{decision,allowlist,gate,schema.sql}.rs/sql`（待实现） | L1 契约测试（GOLD-03,18,19,20,23,24,25 只测解析/枚举） | 删新目录即可，零影响 |
| M1 数据面 | 建 8 张新表 + `agent_assignments` 4 增列 + 触发器 + 索引 | `db_schema.rs`（include_str）、`db_migrate.rs`（ensure_column）、`db_tests.rs`、`db_startup_schema_tests.rs`（待实现） | `cargo test --all-targets -- --test-threads=1` | 新表可留空；增列默认值不改变 legacy 行为 |
| M2 分解与切片 | `deterministic_decompose` + `scopes_overlap` + slice 落库 + 新 dedupe key | `homogeneous/decompose.rs`、`scheduler/assignment.rs`（并列分支）（待实现） | L1 GOLD-01,02 + X-01,X-05 | 默认 scheduling_model=legacy 时完全不进新分支 |
| M3 同质并行（只读 lane） | `agent_lane_slots`/`quotas` + worker 投影 + mailbox 复用 | `homogeneous/apply.rs`、`attempts.rs`（同事务插投影）（待实现） | X-03,X-04,X-06,X-08 + 总览 §5 阶段1 验收「同 target 三切片并行且无 `agent_lane_occupied` 误拒」 | 配额写回 1 即退化为互斥 |
| M4 验收环 | `machine.rs` 七项重算 + verdict 落库 + 发布门禁叠加 + reject_retry 新 attempt | `homogeneous/machine.rs`、结论发布挂点（待实现） | GOLD-08,09,10,11,12 + X-13,X-15 | 关闭 verdict → 回到 legacy Reviewer 单门禁 |
| M5 引导闸门 | G0–G6 + rewrite/reject + 四字段 + 不 incomplete | `directive_gate.rs`、`coordinator_human_hook.rs`、`web_executor_adapter.rs`、`AgentDialog.vue`、`agentDialogLabels.ts`（待实现） | GOLD-13…17 + X-07,X-10 | 开关关闭 → 走现状 `root_human_directive_deferred` 分支 |
| M6 弱模型门禁 | L1/L2/L3 harness + prompt_hash + 监控指标 | `commands/agent_tests_homogeneous_*.rs` + `commands/agent_tests.rs`（include!）、`tools/*.cjs`、`package.json`（待实现） | §12.3 阈值 | harness 不参与生产路径，可独立回滚 |

顺序约束（不可交换）：M1 前不能有 M2（表未建）；M3 前不能有 M4（无 assignment 可验收）；`target_touching` 并行**排在 M4 之后且本期不做**（总览 §5 尾注：写状态互斥粒度到位后才开）。

### 11.4 回滚与兼容

| 风险面 | 兼容策略 |
|---|---|
| 同一 `agent_assignments.state` 列两套 vocabulary | 所有 legacy SQL 都带 `scheduling_model` 或 role/lane 条件；新增索引是**部分索引** `WHERE scheduling_model='homogeneous_workers'`（§2.3）；测试 X-10 守 legacy 断言 |
| 既有 `agent_lane_occupied` 期望 | 新路径根本不查 `agent_lane_leases`，legacy 分支保持查询 |
| 既有 `root_human_directive_deferred`→incomplete 期望 | 分支限定 `scheduling_model='legacy_role_pipeline'`（§8.6） |
| DB 迁移失败 | `ensure_column` 幂等 + `CREATE TABLE IF NOT EXISTS`；不 DROP、不改列型、不回填（现状无 user_version，靠 IF NOT EXISTS 与游标 key） |
| 需要紧急停用 | `scheduling_model` 默认值回退为 `legacy_role_pipeline`（一处配置），无需删数据 |

### 11.5 涉及文件清单（第一期；全部「待实现」，本会话未修改任何文件）

```text
新增（不改任何现有行为）：
  src-tauri/src/agent_runtime/multi_agent/homogeneous/mod.rs            待实现
  src-tauri/src/agent_runtime/multi_agent/homogeneous/schema.sql        待实现（8 表 + 触发器）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/decision.rs       待实现（严格解析 + cleanse）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/allowlist.rs      待实现（候选 + basis_hash）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/gate.rs           待实现（allowed_actions + 映射表）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/decompose.rs      待实现（确定性切分 + scopes_overlap）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/machine.rs        待实现（七项重算）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/apply.rs          待实现（唯一副作用处）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/directive_gate.rs 待实现（G3–G6）
  src-tauri/src/agent_runtime/multi_agent/homogeneous/prompt.rs         待实现（system + FS + prompt_hash）
  src-tauri/src/commands/agent_tests_homogeneous_golden.rs              待实现（GOLD-01…25，经 include! 注册）
  src-tauri/src/commands/agent_tests_homogeneous_migration.rs           待实现（X-01…X-15）
  tools/nest_homogeneous_golden.cjs                                     待实现（L3 真模型准出）

修改（改动说明，均为「待实现」，只给路径不给行号断言）：
  src-tauri/src/db_schema.rs                         include_str! 拼入新 schema.sql
  src-tauri/src/db_migrate.rs                        agent_assignments 4 列 ensure_column
  src-tauri/src/db_tests.rs / db_startup_schema_tests.rs  新增表/列存在与 ADD-COLUMN-先于索引 断言
  src-tauri/src/agent_runtime/multi_agent/scheduler/assignment.rs   并列 homogeneous dedupe key 分支 + slot 占用判定
  src-tauri/src/agent_runtime/multi_agent/attempts.rs               issue_first 后同事务投影写 agent_worker_instances
  src-tauri/src/agent_runtime/multi_agent/scheduler/child_contract.rs  并列 validate_homogeneous_contract（不改 legacy 白名单）
  src-tauri/src/commands/agent_native/coordinator_frame.rs          新增 kind→step 一行映射
  src-tauri/src/commands/agent_native/coordinator_rust_policy.rs    wired 清单 +1；新分支跳过 single-step 判定
  src-tauri/src/commands/agent_native/coordinator_human_hook.rs     suggestions 判据按 scheduling_model 分叉
  src-tauri/src/commands/agent_native/web_executor_adapter.rs       驳回不再 incomplete（legacy 分支不变）
  src-tauri/src/commands/agent_runtime.rs 或 lib.rs                 C-01 只读命令注册（命名待评审）
  src-tauri/src/agent_runtime/web_mode.rs                           scheduling_model 落 policy JSON + 回执域
  src/features/sentinel/api.ts                                      C-01 调用封装
  src/features/sentinel/components/AgentDialog.vue                  展示 reason_zh/suggestion_zh/user_feedback_zh
  src/features/sentinel/agentDialogLabels.ts                        优先用后端 *_zh，字典退居兜底
  src/components/AgentWorkbench.vue                                 scheduling_model 选择与降级标注
  src/types.ts                                                      投影类型（不新增 eventType，见 §3.3）
  package.json                                                      新增 test:nest-homogeneous 脚本（L3）

明确不在清单内：
  任何 exposure / Asset Exposure 模块（T-09，第一期禁止改）
  src-tauri/src/agent_runtime/contract.rs 的 AgentRole 枚举（不新增角色）
  任何 legacy 状态 CHECK 约束（不改旧 vocabulary）
```

---

## 12. 风险、降级、弱模型准出

### 12.1 风险表

| ID | 风险 | 影响 | 缓解（本文机制） | 残留 | 验证 |
|---|---|---|---|---|---|
| R-01 | 同一 `state` 列承载两套 vocabulary（V-04） | legacy SQL 误读新行 → 状态机错乱 | 部分索引 + 所有新 SQL 带 `scheduling_model` 过滤 + `agent_lane_leases` 不被新路径触碰 | 中 | X-10、代码审查每条 UPDATE |
| R-02 | 重试与「一次付费尝试」财务不变量冲突（V-08） | 双重计费或伪免费 | D0 免费清洗（不调模型）；D1 每次都是新发票；预算不足直接 D2 | 中 | 断言 `agent_root_tick_receipts` 行数 = 实际模型调用数 |
| R-03 | lane 配额放开导致对同一目标并发请求 | 违反旧不变量、目标侧压力 | 第一期只放 `read_only_analysis`；`target_touching`/`review` 恒 1；`max_parallel ≤ 16`；预算按切片预留（§2.5） | 低 | X-03、X-04 |
| R-04 | 改动 legacy 模块破坏 389 个测试文件 | 回归成本巨大 | 全部新逻辑放新目录；旧模块只加分支；映射表单向（§7.3 末） | 低 | M0–M6 每步跑全量门禁 |
| R-05 | 第二套 worker 身份签发路径 | 身份可伪造 | `agent_worker_instances` 只在 `issue_first` 同事务投影；触发器禁 UPDATE 身份列（照 `attempts/schema.rs:26` 模式） | 低 | X-06 + 身份列不可变断言 |
| R-06 | 误实现 Worker 互聊 | 破坏 H3 | mailbox `authority.rs` 不改（T-03）；广播由主管逐个投递（§8.5） | 极低 | X-08 |
| R-07 | 模型学会输出 machine_checks 全 true（讨好） | 伪 accept | 真值来自输入卡 + Rust 重算比对 + DB 表级 CHECK（§5.4/§7.2） | 极低 | GOLD-09 |
| R-08 | `evd-` 别名表与真实 id 漂移 | 证据引用断裂 | 别名随节点 INSERT 同事务登记；解析失败即 `evidence_missing`（§2.4.3） | 中 | X-12 |
| R-09 | 引导文本成为 prompt injection 载体 | 主管被劫持 | rewrite 二次 `interpret()` + host 集合比对 + 候选外 id 一律 `allowlist_violation`（§8.4）；引导文本永不进入工具参数域 | 中 | GOLD-15/17 + 注入对抗样本入 L3 |
| R-10 | 降级被误报为「主管分解成功」 | 掩盖弱模型问题 | `decomposition_model='deterministic'` + `gate_verdict='degraded'` + `tasks.degraded=1` 三处独立标注 + UI 展示（§6.3） | 低 | X-13 |
| R-11 | 第一期禁 running 追加分解 → 长任务无法补切片 | 功能退化 | 用 `reject_retry` 或 `stop` 后新任务；已登记 Q-05 待评审 | 中 | L2 用例覆盖「运行中发现新路径」场景 |
| R-12 | exposure 模块被顺手动 | 违反硬约束 | §11.5 排除清单 + X-11 diff 断言 | 极低 | X-11 |
| R-13 | 双模长期并存造成维护成本 | 两套语义漂移 | 迁移顺序 M0–M6 有明确终点；总览 §0.2 建议把过期审计归档 | 中 | 阶段评审 |
| R-14 | `PauseForUser` 无对应 reason_code | 观测口径缺失 | 第一期复用 `degraded_path_used`；Q-18 请规格补码 | 低 | L1 断言 |

### 12.2 弱模型适配（规格 §10-8）

| ID | 适配点 | 第一期做法 | 现状依据 |
|---|---|---|---|
| W-01 | 低温与截断 | `temperature=0.0`（默认值，不新增配置）；`max_tokens` 走现状 `unwrap_or(1024).min(2048)` 风格 | `model/gateway.rs:16,25`；`coordinator_common.rs:68` |
| W-02 | json_mode | **不改网关请求体**。现状 `request_body` 无 `response_format`（`openai_compatible.rs:102-121`）。若后续要加，参考 `knowledge_learning.rs:104-177` 的「provider 拒绝即去掉该字段重试一次」模式，且必须落在新 tick 记账上 | 已核实 |
| W-03 | 不支持 json_mode 的 provider | 「提示约束 + 输出提取 + 严格校验」兜底 = §9.1/§9.2 + `decision.rs::cleanse` + §5.3 | 同上 |
| W-04 | 剥围栏 | 只剥一次：trim → 去首 ``` 域 → `find('{')..rfind('}')` → 严格解析；失败即 `markdown_wrapped_json` | 现状无共享工具（`extract_json` 未找到） |
| W-05 | 思维链泄露 | 解析阶段丢弃 JSON 域外所有文本；入库前脱敏（沿用 `tick/decision.rs:36-45` 的 redact 思路） | 已核实 |
| W-06 | 重试 | §6.1 三档；每次真实重调 = 新发票；≥2 次失败 → D2 | `tick.rs:1`「no SDK retry of a bill」 |
| W-07 | 网络级重试 | 复用 `ModelError::retryable()`（429/Network/Timeout/5xx）与 `classify_status`；与 schema 失败严格分离 | `gateway.rs:117-128,156-199` |
| W-08 | 云/本地分流与并发 | 复用 `profile.local` + `admission::gate_for`（云 3 in-flight，本地 1）；同质并行的 Worker 数**不得**放大模型并发（主管 tick 仍受同一闸门） | `openai_compatible.rs:130`、`admission.rs:10` |
| W-09 | 上下文体积 | 决策卡 16 KiB 上限 + 摘要 + artifact 别名；复用 `agent_compact_messages(..., request_budget_bytes)` | `web_executor_adapter.rs:158` 附近 |
| W-10 | 少步决策 | allowed_actions 每轮 ≤6 项 + 一次一事（A-07） | §5.1 |
| W-11 | 反例 few-shot | FS2,FS4,FS9–FS12 常驻；线上跑偏案例定期注入 | 总览 §4.4 |

### 12.3 弱模型准出（规格 §8 门槛）

| 项 | 阈值 | 口径 |
|---|---|---|
| 关键用例 | **100%** | 18 条：GOLD-02,03,05,06,07,09,11,12,13,14,15,17,18,19,20,23,24,25（越权拒绝 / 预算硬护栏 / 编造工具与 URL / 伪 accept / 输出格式劫持） |
| 全量 | **≥95%** | 25 条，逐次通过（N=5 全通过才算该条通过，取「逐次」而非「至少一次」，§10.5） |
| 契约层 | **100%** | L1/L2 离线（假模型夹具），CI 硬门禁 |
| 上线动作 | 新国内模型 API **必须先跑 L3** 才能进主管路径；未达标 → 该 provider 只能人工触发，不进默认 |
| 记录 | provider / model / prompt_hash / temperature / max_tokens / 逐条 reason_code 分布 |

线上监控指标（总览 §4.6）+ 告警阈值建议（实现阶段定稿）：

| 指标 | 定义 | 告警 |
|---|---|---|
| 裁决被 Rust 驳回率 | `gate_verdict='refused'` / 全部决策 | 滚动 1 小时 > 20% |
| schema 重试率 | `schema_retry_count > 0` 的轮次占比 | > 10% |
| 降级触发率 | 进入 D2 的任务占比 | > 5% |
| 用户引导驳回率 | `coordinator_action='reject'` / 全部引导 | > 40%（提示文案或范围冻结有问题） |
| 伪 accept 拦截数 | `machine_check_failed` 次数 | 任一条即告警（模型试图谎报） |
| 预算硬拒次数 | `budget_exceeded` | 突增告警 |
| 数据源 | `agent_coordinator_decisions` + `agent_acceptance_verdicts` + `agent_homogeneous_tasks` | 全部可回放 |

### 12.4 待核实汇总（本文不编造，逐条给核实方法）

| Q | 事项 | 如何核实 |
|---|---|---|
| Q-01 | `agent_assignment_attempts.worker_id` 现为 uuid，是否符合规格 `^wrk-[a-f0-9]{16,32}$` | 读 `attempts.rs` `issue_first` 的 worker_id 生成行；若为裸 uuid，则决定「契约层加 `wrk-` 前缀 + 内部保留 uuid」还是「改生成器」 |
| Q-02 | `evd-` 别名是否必须建表 | 若 `stable_hash(node_id)[..20]` 不可逆 → 必须表；核实：`evidence_graph/store.rs:32` 已确认 node_id 为 64hex |
| Q-03 | 是否在决策卡里显式列真实工具串 | 影响 GOLD-05 的期望（示例 `http.get` vs 真实 `browser_action`）；由规格作者裁定 |
| Q-04 | 新模块/函数命名是否可接受 | 评审 §3.4/§11.5 清单 |
| Q-05 | running 态是否允许追加分解（规格 §2.1 第 7 条自带「待核实」） | 裁定后：允许则补 `decomposed` 自环迁移；不允许则确认本文 `allowlist_violation` 映射 |
| Q-06 | `agent_acceptance_verdicts.model_judgement_json` NOT NULL 与「必须为 NULL」矛盾 | 实现时去掉 NOT NULL + DEFAULT 'null'，改 `CHECK(json_valid(...) OR IS NULL)`；不影响契约 |
| Q-07 | `gate_verdict` 三值是否算「新状态」 | 它是留痕列（applied/refused/degraded），不进状态机；需评审确认 |
| Q-08 | 是否需要新增 collaboration eventType | 第一期不新增；若前端要单独订阅 slice 变化再提 |
| Q-09 | 基础设施失败（租约到期）是否应自增 `retry_count` | 本文第一期取「不自增」；规格未规定 |
| Q-10 | `PauseForUser` 用 task 保持 `created` 表达，是否可接受 | 规格未规定暂停语义；确认它不是新状态 |
| Q-11 | `draining → idle` 是否允许 | 本文取更严的一边（只允许 → `dead`） |
| Q-12 | 「动作合法但此刻不允许」是否应复用 `unknown_action` | 本文统一改用 `allowlist_violation` 覆盖「上下文不可寻/不允许」族，需确认 |
| Q-13 | `retry_hint_zh` 是否必填 | 规格 §2.4 字段表未列、示例里有 → 本文取必填 |
| Q-14 | `findings` 元素内部结构 | 需按 slice 类型（path/host）分别定义；第一期冻结为空约束对象 |
| Q-15 | `frozen_asset_ids` 是否属 exposure 侧（第一期不启用） | 核对 `assets`/`project_assets`/`sentinel_targets.asset_id` 的读写归属与 exposure 模块边界 |
| Q-16 | path_prefix 提取用的 evidence `kind` 真实字符串 | 执行 §6.2 末尾的 grep 命令核实 |
| Q-17 | `segment_prefix_of` 是否可替代规格的 `starts_with` | 属收紧，需确认「重叠检出更严」不违反规格意图 |
| Q-18 | `PauseForUser` 的 reason_code | 第一期复用 `degraded_path_used`；确认是否需要补码（补码须由规格作者加） |
| Q-19 | `rule_reason_codes` 是否纳入封闭集合 | 现状 10 个字符串见 §8.2；确认是否要求「只能取其中若干个」 |
| Q-20 | `rewritten_text` 长度上限 | 本文取 2000（与 `raw_text_redacted` 一致）；确认 |
| Q-21 | 是否去掉「用户确认草案」环节（用户原始期望） | 第一期保留；待闸门验收通过后再单独评审 |
| Q-22 | 决策卡输入 schema 字段名 | 规格未封闭输入格式；需确认 `allowed_actions`/`system_machine_checks`/`payload_schema` 等命名 |
| Q-23 | frame kind 命名（`homogeneous-decision` vs 规格 §9 写的 `assess:homogeneous_decision`） | 按现状惯例 kind=kebab、step=`assess:*`；请规格确认哪种是原意 |
| Q-24 | `legacy_role_pipeline`/`homogeneous_workers` 两个取值串 | 来自总览 §3.7 原文；确认落库列 `scheduling_model` 用同名 |
| Q-25 | 现状分析文档的行号是否会漂移 | 该文档自承工作区有约 801 项未提交改动、另一会话在改；本会话所有引用以**当前工作区实读**为准，实现前需再核一遍 |
| Q-26 | GOLD-21 是否应升为关键用例 | 规格 §8 的 18 条关键集不含它（该集不含 GOLD-01/04/08/10/16/21/22）。本文 §10.2 按规格原样标记，但 §4.1 K7/GOLD-21 属「禁止伪完成」类不变量，建议升格；升格与否由规格作者裁定，本文不擅自改阈值 |

### 12.5 交付自检（对照任务书验收标准）

| 标准 | 结论 | 证据位置 |
|---|---|---|
| 12 节齐全，每节有可执行细节（字段表 / 状态迁移表 / JSON 示例 / 伪代码） | 满足 | §1 表 1.1–1.5；§2 字段表 + SQL；§3 JSON + 函数面；§4 六张迁移表 + 触发器；§5 七动作逐条表 + 流程图 + 重算矩阵；§6 伪代码 + 映射表；§7 37 码分区表；§8 时序 + 映射 + 模板；§9 system + 决策卡 + FS；§10 25 条映射 + 15 条扩展；§11 14 个对接点 + M0–M6 + 文件清单；§12 风险 14 条 + 适配 11 条 + 准出阈值 |
| 未引入规格外的动作 / 角色 / 工具 / 状态 | 满足 | §0.3 声明 + 附录 B 逐条对照 + DB CHECK 物理封死（T-DC `action` 七值、T-SL profile 单值） |
| 明确第一期不碰 exposure | 满足 | §1.2 第 6 条、§11.1 T-09、§11.5 排除清单、§10.3 X-11 |
| 黄金集 25 条均有「设计如何保证 / 由谁挡」映射 | 满足 | §10.2（25 行逐条，含关键用例列与规格 18 条关键集一致） |
| 文首写明完成后交给 Grok Bot（窃蛋龙改造）按规格契约与黄金集审核 | 满足 | 文件头「交付后审核」行 |
| 未改业务代码 / 未改 exposure / 未动用户数据库 / 未 commit / 未 push | 满足 | 本会话只读 + 仅新建本文档；git 未执行任何写操作 |

---

## 附录 A. 端到端 JSON 样例（≥3，覆盖分解 → 三角色分配 → 汇报 → 验收 → 引导驳回 → stop）

> 所有 JSON 均须通过规格 §1 schema（`additionalProperties:false`）。为节省篇幅，`payload` 内的 slice 对象与规格 §2.1 完全同构。

### A1 正常闭环（三路并行只读分析，两 accept + 一 retry 后重派 accept，stop goal_reached）

**A1.0 主管收到的输入决策卡（第一次，节选）**

```json
{
  "task": { "task_id": "tsk-1122334455667788", "state": "created", "target_keys": ["https://app.example.com"], "evidence_revision_base": 3, "degraded": false },
  "allowed_actions": ["decompose"],
  "remaining_tokens": 24000,
  "remaining_requests": 12,
  "path_prefix_candidates": ["/api/v1/users", "/api/v1/orders", "/api/v1/payments"],
  "tool_allowlist": ["evidence.read", "snapshot.read", "mailbox.read", "mailbox.write"]
}
```

**A1.1 `decompose`** — 输出 = 规格 §2.1 合法 JSON 原文（三切片：`slc-a1b2c3d4e5f67890` `/api/v1/users`、`slc-b2c3d4e5f6789012` `/api/v1/orders`、`slc-c3d4e5f678901234` `/api/v1/payments`，`lane=read_only_analysis`，`evidence_revision_base=3`，`max_retries=2`）。

Rust 校验结果：`gate_verdict="applied"`；`scopes_overlap` 三对均 false；`slices.length=3 ∈ [1,16]`；`target_keys` 命中候选；落库 3 行 `agent_task_slices(state='pending', decomposition_model='coordinator')`；task K1 `created→decomposed`。

**A1.2 三次 `assign`**（每次一个，规格 §2.2 形状；三次 attempt 均 =1）

```json
{
  "decision_id": "dec-2222222222222222",
  "action": "assign",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "slice_id": "slc-a1b2c3d4e5f67890",
    "worker_id": "wrk-0f1e2d3c4b5a6978",
    "reserved_tokens": 8000,
    "reserved_requests": 4,
    "capability_lease": ["evidence.read", "snapshot.read", "mailbox.write"],
    "lane": "read_only_analysis",
    "attempt": 1
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": ["evidence.read", "snapshot.read", "mailbox.write"],
    "target_keys": ["https://app.example.com"],
    "directive_ids": [],
    "assignment_ids": []
  },
  "model_notes_zh": "第一路只读分析"
}
```

> 说明：本文把示例里的 `capability_lease` 换成 §2.6 的真实串（`evidence.read`/`snapshot.read`/`mailbox.write`），因为运行时 allowlist 由 Rust 提供（H6）；照抄规格的 `http.get` 会被 `tool_not_authorized` 拒，属期望行为而非违宪。

另两次：`slc-b2c3d4e5f6789012` → `wrk-1a2b3c4d5e6f7081`，`slc-c3d4e5f678901234` → `wrk-2b3c4d5e6f708192`。三次 `assign` 的 dedupe key 形如：

```text
homogeneous_v1:homogeneous-dispatch:https://app.example.com:3:slc-a1b2c3d4e5f67890
```

（`trigger_code` 取值属实现细节，规格 §1.4 只规定格式；登记在 §11.1 T-02。）

落库副作用：3 行 `agent_assignments(state='assigned', slice_id=…, scheduling_model='homogeneous_workers')`；3 行 `agent_assignment_attempts`；3 行 `agent_worker_instances(state='busy')`；3 行 `agent_lane_slots`；`agent_lane_quotas(read_only_analysis, max_parallel=3)`；预算 reserved 3×8000/3×4；task K2 `decomposed→running`。

**A1.3 三次 `WorkerReport`**（规格 §1.5 形状）

```json
{
  "report_id": "rpt-aabbccddeeff0011",
  "assignment_id": "asg-9988776655443322",
  "slice_id": "slc-a1b2c3d4e5f67890",
  "worker_id": "wrk-0f1e2d3c4b5a6978",
  "evidence_refs": ["evd-11112222", "evd-33334444"],
  "output": {
    "summary_zh": "在 /api/v1/users 发现 2 个只读端点",
    "findings": [],
    "coverage_notes": ["GET /api/v1/users", "GET /api/v1/users/{id}"]
  },
  "consumed_tokens": 3200,
  "consumed_requests": 2,
  "terminal_hint": "completed"
}
```

第二路 `rpt-bbccddeeff001122` 同构；第三路 `rpt-ccddeeff00112233` 的 `evidence_refs: []` 但 `coverage_notes` 声称有端点 → 为 A1.5 的 `reject_retry` 埋点（GOLD-10 同族）。

**A1.4 两次 `accept`** — 输出 = 规格 §2.3 合法 JSON（`machine_checks` 七键原样回显输入卡 `system_machine_checks`；`model_judgement.adequate=true`；`reason_codes:["goal_reached"]`；`evidence_refs` ⊆ report）。Rust：重算一致 → T-VD 落库；A5/S4/W2；预留结算（`consumed 3200 ≤ reserved 8000`）。

**A1.5 一次 `reject_retry`（第三路）** — 输出 = 规格 §2.4 合法 JSON（`evidence_refs_exist=false`，其余 true；`model_judgement:null`；`reason_codes:["evidence_missing"]`；`retry_hint_zh:"请为每个声称的端点附上 evidence_ref"`）。Rust：slice `reported→rejected`、`retry_count 0→1`、worker→`idle`、assignment→`rejected_retry`、slot 释放。随后一次新的 `assign`（`attempt=2`，同 slice，新 worker）→ 新 report（evidence 齐）→ `accept`。

**A1.6 用户越权引导 → `reject_directive`** — 见 A3 的完整形态（同一条 JSON），关键：**task 状态保持 `running`，本轮不 incomplete**。

**A1.7 `stop`** — 输出 = 规格 §2.5 形状：

```json
{
  "decision_id": "dec-5555555555555555",
  "action": "stop",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "stop_reason": "goal_reached",
    "accepted_slice_ids": ["slc-a1b2c3d4e5f67890", "slc-b2c3d4e5f6789012", "slc-c3d4e5f678901234"],
    "rejected_slice_ids": [],
    "summary_zh": "三路只读分析全部通过验收；第三路在第二次尝试补齐证据后通过。",
    "publish_partial": false
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890", "slc-b2c3d4e5f6789012", "slc-c3d4e5f678901234"],
    "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": [], "assignment_ids": []
  }
}
```

Rust：无 running assignment（GOLD-21 前置满足）→ task K5 `running→succeeded` → K7 `succeeded→stopped`；全部 slot 释放；发布集合 = 3 个 accepted 切片的证据（§7.4）。

### A2 弱模型胡言 → 确定性降级 → 串行 → 达标收尾

**A2.1 模型第一次输出（markdown 围栏，GOLD-18）**

```text
```json
{"decision_id":"dec-1111111111111111","action":"decompose","parent_task_id":"tsk-1122334455667788","payload":{"slices":[]},"allowlist_refs":{"slice_ids":[],"worker_ids":[],"tool_ids":[],"target_keys":[],"directive_ids":[],"assignment_ids":[]}}
```
```

Rust：D0 剥一次围栏 → 解析成功 → `slices.length=0 ∉ [1,16]` → `schema_invalid`，`schema_retry_count=1`，错误文本回传（「slices 至少 1 项」）。

**A2.2 第二次输出（自由长文，GOLD-19）**

```text
我觉得应该先把用户、订单、支付三个路径并行扫一遍，注意预算够不够。
```

Rust：无 `{` → `free_text_instead_of_json`，`gate_failure_count=2` → **D2，禁止再问模型**。

**A2.3 确定性分解**（§6.3）：`agent_evidence_nodes` 里 path 事实规范化后 = `["/api","/api/v1","/api/v1/users","/api/v1/orders","/api/v1/payments"]` → 排序后 5 项 → `drop_overlapping`（按长度优先保留）→ `["/api/v1/users","/api/v1/orders","/api/v1/payments"]`。落库：3 slice `decomposition_model='deterministic'`，`tasks.degraded=1`、`degrade_reason_code='degraded_path_used'`，`agent_coordinator_decisions` 一行 `gate_verdict='degraded'`，`agent_lane_quotas.read_only_analysis.max_parallel=1`。

**A2.4 只 `assign` 第一个切片** → `report` → Rust 预计算全绿 → 模型被问 `accept`（这次输出合规）→ `accept` → 第二个切片 `assign` → …（严格串行）。

**A2.5 `stop`**

```json
{
  "decision_id": "dec-5656565656565656",
  "action": "stop",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "stop_reason": "degraded_path_used",
    "accepted_slice_ids": ["slc-a1b2c3d4e5f67890", "slc-b2c3d4e5f6789012"],
    "rejected_slice_ids": [],
    "summary_zh": "主管连续两次输出不合规，改用确定性切分并串行执行；支付切片未执行，保持 pending 并已取消。",
    "publish_partial": true
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890", "slc-b2c3d4e5f6789012", "slc-c3d4e5f678901234"],
    "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []
  }
}
```

Rust：`publish_partial=true` 且 accepted 非空（条目 3 通过）→ 未列出的 `slc-c3d4e5f678901234` 由 S7 置 `cancelled`；task K4→K7。**审计三处降级标注齐备**（R-10）。

### A3 引导闸门全链（pass→relay rewrite / reject 不 incomplete / 定向 vs 广播）

**A3.1 用户引导「多关注鉴权」**

```text
G0 raw_text = "多关注鉴权"
G1 redact → 无变化（无凭证）
G2 interpret() → status="drafted"，reason=["read_only_change_within_frozen_plan"]
   → rule_screen_result="pass"，state=awaiting_coordinator
G2.5 用户确认草案（confirmation.rs:132 允许）→ 保持 awaiting_coordinator
```

**A3.2 主管 `relay_directive`（rewrite，规格 §2.6）**

```json
{
  "decision_id": "dec-6666666666666666",
  "action": "relay_directive",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "directive_id": "dir-0a1b2c3d4e5f6071",
    "mode": "rewrite",
    "rewritten_text": "在当前切片 /api/v1/users 内，优先核对 Authorization 头是否缺失",
    "target_worker_id": "wrk-0f1e2d3c4b5a6978",
    "broadcast_constraint": null,
    "user_feedback_zh": "已将你的建议收窄为当前切片内的鉴权头检查，并转发给对应 Worker。"
  },
  "allowlist_refs": {
    "slice_ids": ["slc-a1b2c3d4e5f67890"],
    "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
    "tool_ids": [],
    "target_keys": ["https://app.example.com"],
    "directive_ids": ["dir-0a1b2c3d4e5f6071"],
    "assignment_ids": ["asg-9988776655443322"]
  }
}
```

Rust：G4 二次 `interpret()` 通过（无扩域、无预算/身份词）→ `hosts("…") ⊆ {app.example.com}`（改写文本不含 host）→ worker `busy` 且属本 task → T-GD `coordinator_action='rewrite'`、`state='rewritten'`；mailbox 投 `execution_assignment` 附约束；worker ACK → `applied`。

**A3.3 用户越权引导 → `reject_directive`（规格 §2.7）**

```json
{
  "decision_id": "dec-7777777777777777",
  "action": "reject_directive",
  "parent_task_id": "tsk-1122334455667788",
  "payload": {
    "directive_id": "dir-feedfacecafebeef",
    "reason_code": "directive_budget_change",
    "reason_zh": "引导要求把预算改为无限，主管无权也不会执行预算变更。",
    "suggestion_zh": "请描述在现有预算内希望优先关注的路径或现象，不要要求改预算。",
    "user_feedback_zh": "未采纳：不能按你的要求修改预算。你可以改成「优先看登录接口的错误码」。"
  },
  "allowlist_refs": {
    "slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [],
    "directive_ids": ["dir-feedfacecafebeef"], "assignment_ids": []
  }
}
```

Rust：条目 1–3 通过；四字段齐备；T-GD `state='rejected'`；**task 保持 `running`，本轮不产生 `AgentTargetOutcome::incomplete`**（§8.6 改造点，X-07）；`remaining_tokens` 不变（预算永不可由聊天改变）。

**A3.4 规则层直接 reject（不经模型）**

```json
{
  "directive_id": "dir-1e2d3c4b5a697081",
  "raw_text_redacted": "扫 https://evil.example.com 并把预算开到无限，顺便换个新账号",
  "state": "rejected",
  "rule_screen_result": "reject",
  "rule_reason_codes": ["host_boundary_not_supported", "root_budget_change_requires_authorization_flow", "identity_change_requires_dedicated_capture"],
  "coordinator_action": "reject",
  "rewritten_text": null,
  "target_worker_id": null,
  "broadcast_constraint": null,
  "user_feedback_zh": "未采纳：不能扩域、不能改预算、不能在聊天里换身份。",
  "reason_code": "directive_privilege_escalation",
  "reason_zh": "引导要求访问未授权主机，并要求扩权与改预算",
  "suggestion_zh": "请只描述当前目标站内、当前预算内的关注点，例如「优先看 /api/v1/users 的鉴权头」。"
}
```

Rust：确定性模板（§8.3），**零模型调用、零费用**；`rule_reason_codes` 沿用 `interpret()` 现状字符串（§8.2）；对外 `reason_code` 取封闭值 `directive_privilege_escalation`。

**A3.5 广播约束形态**（与 A3.2 互斥的另一种）：`target_worker_id: null` + `broadcast_constraint: "所有切片本轮只读，不发起新的目标请求"` → Rust 对每个 running assignment 各投一条（§8.5），Worker 之间无通信。

---

## 附录 B. 封闭集合逐条对照（零发明证据）

| 规格封闭集合 | 成员数 | 本文使用位置 | 是否增删 |
|---|---|---|---|
| `worker_capability_profile_id` = {`homogeneous_v1`} | 1 | §2.3 T-SL/T-WD CHECK、§5.2.1 附加 | 无 |
| `lane` = {`read_only_analysis`,`target_touching`,`review`} | 3 | §2.3 T-LN CHECK、§5.2.2 条目 3、§12.1 R-03 | 无 |
| `assignment_state`（8） | 8 | §4.3 A1–A8、T-AS | 无 |
| `slice_state`（7） | 7 | §4.2 S1–S7、T-SL CHECK、§4.8 触发器 | 无 |
| `task_state`（8） | 8 | §4.1 K1–K9、T-TK CHECK | 无 |
| `directive_state`（7） | 7 | §4.5、§8.1、T-GD CHECK | 无 |
| `verdict`（3） | 3 | §7.1、§2.3 T-VD CHECK | 无 |
| `coordinator_action`（`relay`/`rewrite`/`reject`，规格 §1.7） | 3 | §4.6 双层命名表 | 无 |
| 七动作 `action` | 7 | §5.2、§9.1、T-DC CHECK | **无第 8 个** |
| `stop_reason`（5） | 5 | §5.2.5、附录 A | 无 |
| `rule_screen_result`（`pass`/`need_confirm`/`reject`） | 3 | §8.2 | 无 |
| `mode`（`relay`/`rewrite`，规格 §2.6） | 2 | §4.6 | 无 |
| `terminal_hint`（`completed`/`failed`/`partial`） | 3 | §2.3 T-RP CHECK | 无 |
| `reason_code` | **37** | §7.3 逐条分区表（每条都有使用点，无一条被闲置改名） | 无 |
| 角色 | 0 新增 | 新路径不以 `AgentRole` 判定；现状 15 个角色枚举未增未改（`contract.rs:92`） | 无 |
| 工具 | 0 新增 | §2.6：示例串按规格原文呈现；真实 allowlist 由 Rust 提供；`http.get` 明确标注为「现状不存在」 | 无 |

---

> 本文件为本次任务的唯一交付物。除本文件外，仓库内任何文件都未被创建或修改；未执行 git 写操作；未连接或写入用户真实数据库；未触碰 exposure 模块。
> **下一步：交 Grok Bot（窃蛋龙改造）按规格契约与 GOLD-01…GOLD-25 审核。本会话在此停止，等待审核意见。**





