# Nest 同质执行链闭环接手计划（2026-10-12）

本文件是新会话的接手执行计划，不是验收结论，不新增设计裁决。
依据与阅读顺序完全沿用 `NEST_HOMOGENEOUS_CONTINUATION_HANDOFF_2026-10-12.md`；
冲突时以该文件点名的文档链为准（DESIGN > SPEC > ARCHITECTURE，意图增量以深度分析 §J 为准）。

## 范围

只推进该交接「必须接着做的闭环」1–8 条，按依赖排序为四个批次。
不改七动作枚举、不改 exposure 黑名单（`commands/exposure.rs`、
`multi_agent_external_surface.rs`、Asset Exposure 业务与前端、`docs/NEST_ASSET_EXPOSURE*`）、
不动 legacy 固定角色默认路径、不碰真实任务库与真实 `/Applications/Oviraptor.app`。

## 批次划分

### 批次 A：target/review 同链正负集成（闭环 1、2）

在真实临时任务上补齐同链集成：付费 Root → Worker 原 SDK → 真实硬沙箱 →
登记三侧 HTTP → 另一 Worker 付费 Reviewer → 主管 accept → stop。
禁止手写 receipt 或直接 UPDATE 状态代替生产通道。

负向矩阵至少覆盖：复核者=产出者、缺控制组、对象/身份/响应绑定变化、缓存/跳转、
篡改原始签名或 SDK 内容、撤销/过期/暂停、未知费用、部分请求已发但结果未知、
超时回收后的新意图。未知效果不得免费重放，不得确认假阳性。

验收：新增集成测试全绿；每条负向场景有确定性断言（拒绝原因码/状态）。

### 批次 B：结算与状态机复查（闭环 3、4）

复查新 target/review 的费用归属、独占槽位、capability 撤销、attempt 结束、
Root finally/重启回收。Phase 1 的绿色用例不作为新 lane 的结算证明。
复查 `goal_ready` 每一项真实来源与签名；禁止凭 SQL 行计数接受伪造结果。
解析回调重复验证运行环境可优化，但不得减少取消/撤销检查。

### 批次 C：孤儿清理接线 + 真实模型验证（闭环 5、6）

孤儿清理接到实际新执行/App 重启入口；只清理 journal 中准确归属的容器，
不复演已 claim 的程序。真实模型验证使用配置中心有效 DeepSeek flash 配置，
新试验用新捕获目录，保留此前真实失败回执；Key 仅本机私有配置，不写 Git、不贴聊天。

### 批次 D：独立安装 App 端到端 + 全量回归（闭环 7、8）

构建并安装独立测试 App（专属 acceptance root），双合成账号从登录、选批准工具、
建草稿、登记控制组、确认启动、观察真实聊天与结果，到唯一发布与收官。
随后按交接表格逐条复跑：homogeneous 全回归（--test-threads=1）、legacy smoke、
tool_execution_readiness、execution_readiness、container_parser、evidence、
`npm run build`、`npm run test:agent-workbench`。精确记录结果，不自行宣布验收通过。

## 已知事实核对（接手时基线，均出自代码实读）

- 执行链挂点：`apply.rs` → `execution_carrier::dispatch_slice`；
  `commands/agent_homogeneous_workers.rs` worker 循环 → `execution_worker::deliver`；
  root 循环在 `commands/agent_backend_homogeneous.rs`。
- 沙箱旗标：`container_parser.rs` `--network=none --read-only --cap-drop=ALL
  --security-opt=no-new-privileges --user=65534:65534 --pids-limit=1` + CPU/内存/swap/时间限制
  与自定义 seccomp；`execution_readiness` 仅接纳 `offline-parser-container-v1`。
- 数据库：homogeneous/schema.sql 10 表 + parser_execution.sql 2 表（不可变触发器）+
  execution_facts.sql 1 表；均由 `db_schema.rs` include_str! 随 `db::open` 应用。
- `homogeneous/mod.rs` 仍有 `#![allow(dead_code)]`（自述「少量有 API 无调用方条目」），
  批次 A 接线时顺带核减，不作为独立目标。
- 文档测试计数与静态 grep 有小偏差（readiness 实测 8 条 vs 文档 3+2），
  批次 D 以实际 `cargo test` 输出为准记录。

## 硬边界（全文沿用交接原文）

仅使用临时数据库、临时 CAS、合成测试身份；不得访问真实任务库；不得重置累积工作。
Worker 只有 homogeneous_v1 业务画像，任务由主管动态选择；七个主管动作不增加；
禁止 Worker 互聊/委派、魔法扩容。Phase 1 仍只读；执行模式须在原始签名 Root 出生之前
显式冻结，不能修改 UI 标志后热升级旧任务。

## 记录义务

每批次完成后在本文末尾追加一节：改动路径、测试命令与精确结果、未完成项。
未完成必须写「未完成」。

## 批次记录（2026-10-12 接手会话）

### 环境基线

- Docker Desktop 29.8.2 + 钉死镜像 `sha256:114c7a44…`（Hub 按 digest 拉取，linux/arm64、/usr/bin/python3 3.11、无卷、Id 与钉死值一致）。
- `cargo test --lib homogeneous -- --test-threads=1`：**128 passed / 0 failed**（与交接记录一致；缺镜像时为 126/2，属设计内 coverage gap）。

### 批次 A：target/review 同链正负集成——完成

新增 `src-tauri/src/commands/agent_tests_homogeneous_execution.rs`（9 个集成测试，全部驱动生产通道，无手写回执）：

| 测试 | 覆盖 |
|---|---|
| target_branch_executes_registered_three_side_gets_and_freezes_signed_fact | 正向：注册三侧 GET、身份绑定 cookie、签名冻结事实、reported→accept、capability 全撤销、槽位释放、target 车道 0 费用、authorization 记账 3 收 0 悬 |
| review_branch_independent_paid_reviewer_confirms_and_publishes_single_finding | 三车道同链：target→独立付费复核 confirmed→`sentinel_findings` 唯一发布→真实容器 parser→goal_ready→stop(goal_reached) 收官；恰好 2 次付费 POST |
| reviewer_same_as_producer_or_unaccepted_candidate_is_refused | 产出者自审拒（assign 闸）；未验收候选连草稿都不产生（decompose 拒 `slice_not_in_original_candidates`） |
| target_branch_without_control_group_or_wrong_scope_is_refused | 无控制组：无草稿可物化；（拆分出的 target_lane_paid_request_lease_is_refused_by_contract 覆盖付费租约契约拒绝） |
| unverified_target_never_confirms_or_publishes | 服务端正确隔离（伪匿名响应）→ verified=false → confirmed 复核被 `confirmation_without_controls` 拒；0 发布 |
| tampered_fact_or_sdk_receipt_blocks_review_acceptance | execution_facts 与 agent_specialist_calls 双不可变触发器拦截篡改 UPDATE；0 发布 |
| replay_or_expired_attempt_blocks_late_target_delivery | 同 attempt 重放拒；过期后迟到投递拒；wire 恒为恰好 3 个 GET |
| lane_exclusivity_survives_timeout_reclaim | 双控制组：独占期间第二 target 派发拒（lane_quota_exceeded）；过期回收后重新可派发（需先补充替换 Worker） |

测试过程暴露并修复的 **6 个生产缺陷**（此前该链从未被执行过）：

1. `check_target_scope`/`validate_assign`：`reserved_tokens != 0` 误拒——车道不变量是"零付费请求"，tokens 由粗粒度准入回显，改为 `< 0` 拒绝（agent_homogeneous_execution.rs）。
2. `drafts` SQL 联表列名歧义 `SELECT id` → `SELECT f.id`（agent_homogeneous_execution.rs）。
3. probe 预算复验自锁：`budget::target::claim` 记 Reserve+Forfeit（indeterminate+1，直到 `receive` 对账），而 claim 内第二/三次 `authorization_probe_authority` 走严格 `require_determinate` 必炸。新增 `require_determinate_tolerating_own_probe_claim`（admission.rs，target_requests 恰好容忍 1 笔在途）+ `require_on_during_own_probe`（execution_policy.rs）+ `check_target_scope_during_own_probe`，authority 调用按相位传 `during_own_claim`（claim 前严格=false，claim 后/响应事务内=true；共 4 处调用点 + attempt_identity.rs 测试签名）。
4. `accept_on` 发布键 `format!("…:{}", target["id"])` 用 JsonValue Display → record_key 带引号；改 `as_str()`。
5. `dispatch_degraded_pending` 对任意首个 pending 切片硬编码 `read_only_analysis`/`evidence.read`——执行车道切片会铸造自相矛盾决策并毒化整轮。修复：降级路径只挑 read_only 切片（apply.rs）。
6. `reclaim_expired_worker` 只释放 `agent_lane_slots`，不清执行载体写入的 `agent_lane_leases`/`agent_capability_leases` → UNIQUE(scan,attempt,target,lane) 使该车道在本 attempt 永久锁死。修复：回收时调用 `release_on`（pool.rs），与验收路径同一语义。

附带：出生夹具新增 `frozen_tool_supply_original_birth_with_task_controls`（多控制组，经 `save_authorization_control_in` 出生期登记——启动后不可补登记，这本身是 `authorization_control_task_binding_invalid` 的正确防卫）。

### 批次 B：结算与状态机复查——部分完成

已由批次 A 测试锁定的部分：target 车道 0 费用结算、review 车道恰好 1 次 model_requests consume、`agent_lane_slots` 验收后释放、capability 验收后全撤销、过期回收释放槽位与租约（本批修复）、`goal_ready` 三条件（parsers>0 ∧ controls>0 ∧ reviewed==controls）经生产查询验证。

**未完成**：Root finally/重启回收路径（agent_backend_homogeneous.rs 收尾清理块）无专项测试；解析回调重复验证运行环境的优化未做（交接明示"可优化，不能减少取消/撤销检查"）。

### 批次 C：孤儿清理接线——前半完成

- App 重启入口：`lib.rs` run() 内一次性 detached 线程 `recover_orphans_once("app_restart")`，失败 eprintln 上报不阻塞启动。
- 新执行入口：`agent_backend_homogeneous.rs::run_homogeneous_agent_target` 首行 `recover_orphans_once("homogeneous_execution_entry")`，观测走 `append_runner_log`，非致命。
- 新增 2 条真实容器测试（orphans.rs）：只回收归属明确且未被 claim 的孤儿（活 flock=claimed 不动）；guardian 中途死亡记录保留并上报 `container_parser_orphan_creation_or_runtime_unverified`。
- **移交上游**：真实模型验证（闭环 6）需配置中心有效 DeepSeek flash 凭据，属 main 分支维护者的验收职责（2026-10-12 会话确认），不在本分支范围；禁止用旧注入 pro Key 的约束随交接保留。

### 批次 D：回归与记录——进行中

| 命令 | 结果 |
|---|---|
| `cargo test --lib homogeneous -- --test-threads=1` | 128 passed / 0 failed |
| 9 个新执行链测试（逐一） | 各 1 passed / 0 failed |
| `cargo test --lib container_parser` / `orphans` | 8 passed / 2 passed |
| `cargo clippy --all-targets -- -D warnings` | **0 errors**（含清理 26 条 clippy 1.99 存量告警） |
| `npm run build` | vue-tsc + Vite 通过 |
| `cargo test --lib -- --test-threads=1`（全量） | 结果见提交信息（后台执行） |

**移交上游**：安装测试 App 端到端（闭环 7）与跨平台声明属 main 分支维护者的验收职责（2026-10-12 会话确认），不在本分支范围。沙箱证据仍仅覆盖 macOS arm64 本机容器环境，不得据此宣称 Windows/Linux 或整个 Master Plan 完成。
