> 后续实现复核（2026-10-09）：本文下方保留的是此前源码快照，其中“最后 Web call 无 drain”和“HTTP 无独立费用监督”两项已被后续生产接线取代：`multi_agent_finish_execution` 关闭前接 SDK/HTTP consumption drain，实际 HTTP headers 与三条原 target费用绑定 immutable seal。最新关联回归 Root budget 38/38（36.32s）、HTTP receipt 9/9（38.46s），均 localhost。
> Web 到期专属 paused 财务交付及 Root advisory 已接入生产，当前严格 Clippy 与 App build 通过；原 600 秒自然过期增强测试仍在执行，默认 birth 仍 v8，不把接线和源码审查算作 v9 运行通过。capability 独立变化监督、剩余角色输出与 Single 人工确认/专属 Recovery birth 未完成，见 `NEST_A03_CAPABILITY_OUTPUT_GAP_2026-10-09.md` 与 `NEST_SINGLE_RECOVERY_PRODUCTION_GAP_2026-10-09.md`。

# REM-A03 六类监督触发缺口审计（2026-10-09）

本审计只读检查当前工作树，并读取已有临时测试日志；未运行 Cargo，未访问真实数据库。仅新增本文件。源码继续被其他 agent 修改，以下为本次检查时的快照，不是全量门禁或桌面验收。

## 原始验收范围

Master Plan 原精确清单见 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md:1068`：

1. 新 evidence revision。
2. child output envelope。
3. Reviewer decision。
4. lease 到期。
5. capability / 预算 / 保护变化。
6. 用户恢复 / 确认后的 HumanDirective。

每类均要求真实事件 → Root SDK → 原回执 → Rust 准入，并要求稳定 changed-fact 去重。Mapper 和 ClientSide 是第二类的具体生产者；预算分配、预算消费、WAF 保护停止是第五类的不同子项。不能把这些实现名称重新排列成“六类已经齐全”。

## 当前真实功能与剩余范围

| 原类别 | 已接生产路径 | 当前边界与缺口 |
| --- | --- | --- |
| 新 evidence revision | `multi_agent/execution.rs:85,121` 正常交付及已完成重放 → `coordinator_new_evidence_hook.rs:36` → 原 HTTP evidence frame → tick 原回执 | 枚举当前 WebExecutor 创建的 observed request_record 节点；每个物理 node 独立，同 revision 可有多个事实。触发在关闭交付后，尚不是任意角色/任意 evidence kind 的通用 revision 订阅器。 |
| child output envelope | `multi_agent/prepare.rs:260` 的 Mapper；Identity feedback；`multi_agent/client_side_execution.rs:59` 的 ClientSide；InputParser 与 Gap proposal 各有独立 hook | 已有多个真实生产者，不能称只有 Mapper 骨架。BusinessLogic / Authorization / ExternalSurface 的通用 Root output frame 库存仍 reserved/not_ready；其它角色须各自绑定原输出、任务、费用和 Rust 权限。 |
| Reviewer decision | `multi_agent/gap_investigation.rs:123` 取原 Reviewer frame → tick → Rust 角色准入 / Gap 调查 | 存在真实生产链，源码中 paid deferral 不发新 grant。覆盖当前 candidate/revision 缺口调查，不等于所有 Reviewer 结果都需要或已经进入一次 Root SDK。 |
| lease 到期 | Mapper prepare；InputParser、ClientSide、Identity、DeepInvestigator、EvidenceReviewer 的 foreground delivery/replay hook；`coordinator_lease_hook.rs:37` 原 current attempt expired 才评估 | `coordinator_lease_closed.rs:15` 只接受上述六角色，WebExecutor 缺席。不是后台全角色租约扫描/通知服务。原 Root/C 自身已经失效时必须拒绝，而不是自动续租后评估。 |
| capability / 预算 / 保护变化 | executor 开头原预算分配评估；新 v7 Web 已收结算 SDK 费用消费评估；原 WAF 关闭回执保护评估 | 仅部分子项完成。实际 HTTP / browser / upload / controlled write 等费用变化没有独立 budget-consumption frame；其它 child 的 SDK 费用也没有此 frame。能力撤销主要是拒绝执行边界，尚非通用授权变化事件 → Root 评估生产源。 |
| 用户恢复 / 确认后的 HumanDirective | `agent_native/executor.rs:215` → `apply_assessed_human_queue_actions` → 原 accepted directive frame → 付费 Root → 同原指令 Rust queue action | accepted/completed 原队列事实有生产链；`human-recovery` 仍在 not_ready 库存。普通队列指令、财务续接批准、旧 worker 自动恢复不是同一种授权，不能互相代替证据。 |

## 最新预算消费的准确边界

`coordinator_budget_consumption_frame.rs:19` 限定 WebExecutor；从 `agent_web_model_journal` 精确 dispatch / received 配对与原 settled 费用构造稳定事实。不是读取实时余额后随每次 Root 收费生成一个新事实。新 v7 出生合同独立声明 `budgetConsumptionRule`；旧合同不回填，冷移除协议不能降级绕过。

生产入口：

- `agent_native/model_transport.rs:51`：纯读 Web SDK preflight → 消费 hook → 原原子 admission。原能力、plan、identity、assignment、共享 clock 剩余和模型请求余额先拒绝；原 admission 仍做 sample、claim 与事务内复检。
- `agent_tools_dispatch.rs`：工具权限、保护停止、目标额度预检先于消费 hook；finish_target 排除 hook。物理 HTTP claim 再次原子授权与预算检查。
- hook 在下一次 SDK 或非 finish 工具动作前扫描已收到的 Web SDK calls。它不是 received 账单写入事务中的付费动作。

尚有具体遗漏：

1. 最后一个 Web SDK 返回后直接 finish_target，或退出而无下一动作时，最后这笔 call 可能没有进入预算消费监督。`multi_agent/execution.rs:1` 的 `multi_agent_finish_execution`（生产 caller `agent_backend.rs:775`）在 finish/replay 后续当前只接 protection、new-evidence、ClientSide、InputParser，没有 consumption drain。
2. HTTP 已消费 target_requests / upload_bytes 等费用后，下一次 hook 仍只枚举 Web SDK journal。新证据监督对 HTTP evidence 的评估属于第一类，不能抵算第五类预算变化验收。
3. Mapper / ClientSide / Identity / InputParser / Reviewer / DeepInvestigator 等 child 成本变化未独立进入预算消费事实。
4. SDK preflight 及物理 admission 拒绝可以防止越权/超限；这些负向拒绝不是“Root 已智能重新分配预算”。现消费评估仅 advisory，不增 grant、cap、权限或目标范围。

## 骨架与只读诊断

`coordinator_rust_policy.rs` 当前 wired 包含 new-evidence、lease-expired、protection-changed、human-directive、budget-allocation、budget-consumption 及多个角色输出。`human-recovery` 仍 not_ready，BusinessLogic / Authorization / ExternalSurface 输出仍 reserved。库存 wired 只能说明 admission 和映射存在；必须结合上表实际 caller 才能证明生产触发。

本审计对未覆盖角色、HTTP 费用、最后 Web call 的判断来自源码调用图，没有调用真实应用或构造数据库来证明这些路径。不要把诊断写成已修复功能。

## 已读取的测试证据

全部日志位于 `/tmp/oviraptor-master-capacity-concurrency-20261008/`。下面是已有终态，未在本审计重跑，不能据此说当前全部工作树通过。

| 日志 | 终态 | 可以证明 / 不可以证明 |
| --- | --- | --- |
| `root-budget-consumption-preflight.log` | 7 passed / 0 failed / 0 ignored，15.64s | localhost 实际 Web 两笔费用各评估一次；原事实重放；未知费用、撤权、冷改、SDK能力撤销及原 grant 耗尽拒绝。未证明最后call drain、HTTP成本及其它角色消费触发。 |
| `root-budget-allocation-v7-compat-fixed.log` | 8 / 0 / 0，24.02s | 原分配监督与明确 v6 五阶段生产正向；v7容量不足拒绝。v6正向不能充当v7新增消费的全链正向。 |
| `human-capacity-regression.log` | 30 / 0 / 0，146.79s | 原 HumanDirective、容量延后、精确已付回执与拒绝矩阵；不能证明not_ready human-recovery。 |
| `client-regression.log` | 26 / 0 / 0，56.89s | ClientSide实际生产SDK/交付与写边界；该组包括角色本身，不能全算Root反馈触发次数。 |
| `no-override-live-lease.log` | 1 / 0 / 0，3.99s | 原签名 Mapper 到期的全prepare foreground hook；不能外推全角色/后台扫描。 |
| `gap-review-snapshot-fixed.log` | 1 / 0 / 0，12.19s | 独立原 Reviewer 回执与缺口链收口，旧Root/费用保持。 |

`agent_tests_new_evidence_boundary.rs` 存在真实SDK并发、同事实重放及前/途中原授权到期测试；`agent_tests_client_root_feedback.rs` 存在Root反馈正向和已付/篡改/故障负向。本次没有读取其最新独立终态日志，因此仅列为测试源码，不新增通过数量。当前新改动是否通过严格 Clippy / app build 由主控最新终态确认。

## 建议下一项可验收闭环

优先补“最后一笔 Web 已结算 call → 正常关闭交付前 foreground 消费监督 → 原回执 → 关闭交付 / 完成重放”。这直接补现v7生产feature的终态遗漏，范围比一次实现十维事件总线清晰。

实施要求：

1. 具体候选位置为 `multi_agent/execution.rs:60` 原 supervisor/C 复核之后、`:72` 原完成重放判断之前：`:21` 的财务 settlement 已保存原费用，`:94` 的 `complete_target_child_delivery` 尚未发送最终封套；后者在 `multi_agent_child_delivery.rs:46` 才通过 `scheduler::finish_child_in_transaction` 终止 child 并释放权限。此入口共用初交付与完成重放，不阻断 finish_target 工具本身。不要在费用writer事务内发SDK，不要关闭后伪造新worker/C。已settlement的assignment预留为0，最终监督不能调用新SDK工作的preflight；须核原已付call事实与原Root权限，专门证明completed replay仍可精确验证原worker。
2. 正常 finish 与完成重放必须指向相同稳定call事实；如果父/C已失效，仅报告原证据且拒绝新SDK。
3. 原费用未知、账单未结算、Root监督容量不足继续拒绝/defer，不退款、不增加请求上限；关闭收口与未完成监督必须显式区分。
4. localhost真实fixture：一轮Web → 直接finish（没有下一SDK/target）仍恰好一次原消费评估；完成重放零新SDK/零财务写；两轮最后call也不能漏。
5. 补未知费用、到期、撤权、评估paid后publication故障/进程退出矩阵，原Web费用与Root新费用均保留。

随后独立设计“原已关闭 HTTP claim / 结算费用 → 稳定预算事件 → advisory Root评估”，冻结精确十维消费事实，逐物理费用来源去重；不要直接hash当前余额、TTL或Root自身新增监督费用，避免反馈循环。其它child成本与capability变化、Web租约到期、human-recovery按原清单继续。REM-A03维持 partial，不给总完成百分比。
