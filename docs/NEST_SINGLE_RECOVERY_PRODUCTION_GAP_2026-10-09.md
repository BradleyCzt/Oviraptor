# Single 未知费用人工确认与专属 Recovery 出生：生产缺口及实施合同

日期：2026-10-09。仅当前源码只读审查与实施建议；本文件没有新增执行能力、费用确认或出生入口。未运行 cargo，历史文档测试数字不作为当前验收结果。范围不包含 Exposure。

## 1. 已有能力与实际缺口

已阅读 `NEST_MASTER_SINGLE_HISTORY_VERIFICATION_2026-10-09.md`，并复核下列生产链，不能把文档历史追加项拼成全库绿色。

| 能力 | 当前生产事实 | 缺口 |
|---|---|---|
| 原 Single 费用历史 | `budget/root/model/single_history.rs` 核原 owner、原 dispatch/received 与精确费用；`continuation/single_remaining.rs` 再核 SingleExit 和十维余额 | unknown 无完整原账单时不能通过，不能据此允许恢复 |
| 原费用队列 | `commands/native_scan_branches/root_cost_reconciliation.rs` 查询 received/uncertain；Single 已结算账单经过原核验后才从待办窗口跳过；异常进入 blocked | 显示 unknown 不代表能确认 |
| 现费用确认 | `tick/confirmation.rs` 使用 `preview::scoped_tick`，再进入 `tick/unpublished_projection.rs::original_request_for_projection` | 原 policy 必须 multi，Single 明确 `root_tick_requires_multi_coordinator`；preview binding 又必须原 received 行。`reconcile_forfeit` 是已有完整原 received 账单的对账，不是对无账单 unknown 人工录金额 |
| Single 预览 | `native_single_continuation_preview.rs` 校验当前 paused/partial、活动项目、原 Single mode/birth 和原 scope；返回 history/remaining/modelContract | 明确 `executionAllowed:false`、`approvalImplemented:false`、`birthImplemented:false`；不能作为确认/执行合同 |
| Single 新工作时间门禁 | `single_remaining.rs::remaining_for_new_work_on` 能额外核实时原 deadline | 当前仅 cfg(test)，没有生产批准消费者 |
| 保护恢复批准 | `native_protection_recovery_approval.rs` 签精确 scope，状态 approved_pending_fresh_birth；priced 版状态 approved_pending_priced_fresh_birth | 尚不是专属出生或完整共享 USD 消费器 |
| 共享 scope 预算草案 | `native_protection_recovery_scope_budget.rs` 有十维、USD、deadline、跨 roots 总额声明 | 状态 pending_dedicated_fresh_birth，targetRequestsGranted=0，未分配执行权限 |
| 财务续接 | `native_financial_continuation_worker.rs` 读取专属 birth 并运行冻结证据的 plain Root | 必须 WebMode::Multi，拒 bootstrap/liveBudgetObservation/localDeliberation；不能复用为原 Single 目标恢复 |

必须分开实现两个授权边界：A 只确认旧债；B 用户另行批准精确新范围并创建全新 Recovery。A 不得自动触发 B。旧 SingleExit 覆盖退出/时钟证明，不能替代新增未知费用确认的独立签名。

## 2. 第一片：原 Single unknown 的只财务确认

建议新增独立 runtime `budget/root/model/single_unknown_confirmation.rs`、私有 signature/schema、commands 叶文件与实际 UI 对账入口；保持现 Multi Tick policy 验证不变。以下是拟议 API，并非已实现符号：

```rust
preview_original_single_unknown_on(snapshot, scan, attempt, root, call)
    -> Result<SingleUnknownConfirmationProposal, String>;
confirm_original_single_unknown(db_path, selection, evidence, consent, request_id, proposal_hash)
    -> Result<VerifiedSingleUnknownConfirmation, String>;
read_original_single_confirmation_on(snapshot, root, call)
    -> Result<Option<VerifiedSingleUnknownConfirmation>, String>;
```

Proposal 必须从原 signed Single owner/mode/birth、原 call round/requestHash、真实 dispatched 与 uncertain 物理行、原 reserve/forfeit、原 terminal/退出及缓存来源捕获；用原 SDK/execution idle inode 排除正在运行的调用。没有原发出证明不允许将用户录入变成供应商账单。新增 typed constructor 只接受上述原来源与显式确认，拒任意保存 JSON 重签。

Evidence 分两类记录，不混称：可验证第三方账单/usage，或用户声明的费用。后者需显示“人工声明，非供应商响应”，绑定凭据摘要、声明理由、原 unknownReason 和实际可知字段；缺 cached-input 来源时不能默认真实 cached=0。未确认或证据不足保持 unknown，不能猜零、退款、改写 uncertain 为 received。以人工声明关闭 indeterminate 需独立版本合同和明确用户同意，不能扩展现 received validator 掩盖来源差异。

签名事实绑定版本、project/scan/attempt/root/owner/call/round/requestHash、原 mode/birth/exit 的 receipt ID、原物理 facts hash、原四维费用及 reserve/forfeit、证据摘要/来源类型、十维账目前后精确值、整数 nanoUSD 与 rate-card/cache provenance、requestId/proposalHash/consent。原私有发行器签名；saved reader 重读原行并验签，原时间/费用/来源漂移拒绝。不得输出原 key/endpoint secret。

单个 IMMEDIATE 事务及受限 authorizer 仅允许 append 财务调整、独立确认 receipt/签名；保留原 journal、旧费用、旧停止与旧权限。每 call 唯一确认，同 request+同 fact 幂等；不同金额/证据/原来源拒冲突。超原 reserve 的真实债必须显式记录并阻断后续预算，不能截断到 reserve，也不能通过新 grant 消掉旧债。确认完成后无 SDK/目标调用、无新 attempt、无租约续约、无恢复状态写。

## 3. 第二片：十维、USD 与时钟统一准入

原十维来自唯一 `budget::DIMENSIONS`：model_input_tokens、model_cached_tokens、model_output_tokens、model_requests、target_requests、browser_actions、controlled_writes、upload_bytes、concurrency_batches、wall_time_ms。当前 accounting 对后四类无生产物理 writer 的行动历史拒绝，不能凭平衡 reserve/release 声称已经可执行。新合同继续限制无实现的动作为零并拒其请求。

所有原来源先逐 call 验签、验真实账目，再做 checked_sub；缺 hard-limit 行、reserved/indeterminate 非零、溢出/超限均拒。原 None 保留无限含义，不能当零；新的有界执行批准必须显式冻结所需有限上限，不能借 None 跳过容量预检。

USD 使用整数 nanoUSD、checked 运算和明确 cached/input/output 计价语义；冻结原 ratecard/价格来源版本。单独维护一份 scan/scope 总账，所有新 roots 从同一总额 reserve，原十维 root 成本和真实费用要关联原 receipt，不能给每 URL 复制全额。模型请求 preflight 需完整序列化请求输入上界、精确 effective output cap、model/endpoint/credential/proxy hash；实际结算后释放仅原已证明未用部分。当前 Single model readonly helper 的当前配置输出 cap 不是旧模型冻结合同，完整输入上界仍缺，不允许批准。

新工作 deadline = min(原允许截止、新 scope 截止)。原 scan clock origin/原 elapsed/SingleExit 费用保留；停止期间是否计时必须来自原合同，不能把旧 wall_time_ms 的静态余额直接当未来时间，也不能出生时 now+旧时限。若 Recovery 使用新的独立 clock，必须是用户明确批准的新 scope 合同并关联旧 clock cutoff，不能声称沿用原余量。过期只财务确认允许，执行批准拒。

## 4. 第三片：专属 Recovery 批准与出生

建议新增独立 `SingleRecoveryAuthorization` / `FreshSingleRecoveryBirth`，不复用 Multi CoordinatorLease 或原 Single SDK 凭据。拟议生产入口：

```rust
preview_single_recovery_proposal_on(snapshot, selection, exact_new_scope)
    -> Result<SingleRecoveryProposal, String>;
approve_single_recovery(db_path, request_id, proposal_hash, explicit_consent)
    -> Result<VerifiedSingleRecoveryApproval, String>;
birth_single_recovery(db_path, approved_request_id)
    -> Result<VerifiedSingleRecoveryBirth, String>;
```

用户选择精确项目/原 scan/attempt/root、明确 URL/动作/模型、共享十维/USD/截止、冻结证据与完整模型请求边界；批准绑定所有原确认 receipt IDs/hash。scope 或模型配置变化必须新预览新同意。旧付款确认绝不等于新目标授权。

出生消费未撤销且未消费的批准，在 writer queue 与事务中重核活动项目/删除墓碑/当前 attempt/原停止保护、费用确定性、原 idle、模型及 scope hash、真实时间和共享 headroom。一次性创建全新 ID/原模式证明/冻结 plan/隔离 evidence/workdir/共享 budget 引用/原 clock 绑定、私有 birth 签名。不能复用旧根/旧 worker/旧 outbox，不复制未知任务，不改旧 cancelled/paused，不先签任意 JSON 再补原物理行。

DB 与文件出生需明确 crash recovery：pending 文件不可运行；birth commit 后经原签名和全物理校验才 dispatch。缺签名/文件/hash/共享账目均拒执行，不能失败降级普通 start。Single runner 必须消费专属 typed birth 才访问 SDK/目标；正常 start/resume 门禁继续拒旧 paid history。

## 5. 实施顺序与验收边界

1. 先实现 A 的原 unknown 来源 typed proposal/签名确认与当前 UI 显式同意。验真实 SDK uncertain：队列可见；原费用保留；确认仅账目/receipt 可变；重复零写；跨 scope、坏签、并发金额冲突、unknown 缺证据拒绝。不得用 cold 插行代替真实 transport 证据。
2. 单独实现共享十维/USD 与完整模型输入冻结；真实多 URL roots 竞争总额，不能复制额度；USD 小于十维允许量仍拒；费用未知/超债/真实 deadline 过期阻断；退款仅核原 closed receipt。
3. 再实现批准→专属出生→实际 Single runner→真实 SDK/HTTP→精确 settlement→停止/重放。只有这条生产链真实通过才称 Single Recovery 可执行。用户确认、出生、模型/目标发送都需独立负向 fault/撤销测试。
4. 当前文档/原 readonly preview 属只读诊断；原费用审计测试属有界历史证据；pending approval/scopeBudget 属持久合同骨架；均不算新恢复功能完成。

本轮仅新增此设计文件，无生产代码修改、无真实数据处理、无编译测试证据。
