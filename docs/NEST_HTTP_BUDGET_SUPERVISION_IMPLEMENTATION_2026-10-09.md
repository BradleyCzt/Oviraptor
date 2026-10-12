# HTTP 原费用消费监督实施方案（2026-10-09）

状态：设计与只读源码证据，尚无本方案代码、测试或生产验收。只新增本文，未访问真实数据库，未运行 Cargo。此片补 REM-A03 的“预算变化”子项，不能用已有 new-evidence 触发代替。

## 现有真实账本路径

- `src-tauri/src/commands/agent_tools_http/exchange.rs:101` 在 send 前提交 HTTP claim，随后 runtime 计数增加。claim 即使进程死于 send 前也不自动退回。
- `agent_http_claim.rs:126` 对普通 Multi 调用 `budget::target::claim`，source 固定为 `http:{run_id}:{invocation_id}:{request_index}:{request_hash}`。
- `agent_runtime/multi_agent/budget/target.rs:54` 同事务先 require_determinate / 原 clock sample，再追加 target_requests 的 Reserve=1、Forfeit=1。请求在 response 不明时维持未决。
- `exchange.rs:137` 保存响应头回执；`agent_http_claim.rs:174` 精确原 claim 从 status=0 / received_at空更新，`:186` 调用 `receive_for_run`，按原 CostOwner 结算，不采用新 C 或替换 child。
- `target/receipt.rs:2` 的 `original_target_receipt` 是只读核验：原 worker/owner 精确绑定，原 source 必须恰有三个 target_requests 行（reserve、forfeit、reconcile，各amount=1），各stored key正确且不容多行。
- `agent_http_journal.rs:226` 的 `agent_http_header_receipt` 捕获原claim物理行；响应头receipt仅证明headers已收到，不证明body完整、目标成功或漏洞。

特别限定：当前普通 Multi 的上述 target::claim 只核算 target_requests。Single 的 RootTargetCall 接受 controlled-write 与 upload-byte 参数，是另一条合同。不能给 Multi HTTP 构造假的 upload_bytes / browser_actions / controlled_writes 消费。十维其它缺口应独立实现原收费链再接监督。

## 新出生合同：v8，不改变旧v7含义

v7 的 `budgetConsumptionRule = original_settled_web_call_advisory_v1` 精确限定 Web SDK。不得让同一值默默扩大为 HTTP 事件，也不得给存量v7临时添加字段。

建议新普通 Multi Root 的 bootstrap schemaVersion=8，完整保留v7 SDK字段，新增独立可选字段：

```json
{"httpBudgetConsumptionRule":"original_received_http_target_cost_advisory_v1"}
```

验证必须接受精确原v7以及更早JSON，v8新规则固定值。新字段仅private typed出生构造器设置，原冻结plan、Root birth seal及原finance contract一起绑定。旧Root没有该规则返回旧路径；出生seal证明原本声明了v8时，字段删除、改成v7或协议改写必须报完整性失败，不可降级。禁止初始化回填、批量签名存量claim或补新seal。

创建-only v8还须同步所有当前精确 `(6,7)` 版本分支（例如supervision_capacity），不提高原总cap。HTTP监督增加实际Root义务，应纳入新出生监督容量算法；有限容量不足必须在原target发送前拒绝或可解释defer，不能只让下一Root发现无余额。旧v6/v7分配规则保持原值与测试。

## 原proof与稳定事实

建议独立 kind=`http-budget-consumption`，step=`assess:http_budget_consumption`，trigger仍`capability_budget_state_changed`，仅advisory。

事件唯一键应包含原 root / scope / attempt / assignment / workerAttemptId / runId / coordinatorEpoch+fence / budgetAttempt / ordinal / invocationId / requestIndex / requestHash。每个物理HTTP claim恰好一个消费事实；同URL多个真实请求不合并，重定向/双侧比较也按真实claim分开。

语义fact包含：原claim身份与工具句柄、received header status及固定received时间、targetRequests=1、费用source、original-protocol标识。禁止原URL查询、body、cookie、认证材料进入Root摘要。哈希可供核验，但不能称“hash脱敏可逆”。

rows应精确捕获：

1. 原HTTP origin及claim（原row identity、全部固定scope/claim字段，received终态字段）。
2. 原tool_invocation固定身份/input hash/policy_allow字段，排除之后正常变化的status/output/finished_at。
3. 原scheduler出生绑定、原worker身份字段（排除正常续期TTL、终止state及finished_at）。
4. 原target source恰三行，含物理row identity/idempotency_key/amount/dimension/kind；调用original_target_receipt验证原绑定。
5. 原Root出生/冻结plan证明及本HTTP独立的原发行/收到来源证明。

排除Root自身监督SDK费用、全局当前余额、wall clock当前值、lease续期、之后新增evidence/client/reviewer行，保证第二次评估自己收费不会生成新事实。

仅从当前DB取hash不足以证明首次冷改前的原值。现 `ProtectionPreSendWitness` 是私有实际发送来源，但其canonical标识为 `original_send_authorization_unsealed`；WAF `FreshProtectionResponse` 还限定实际body artifact与WAF stop。不得把这些直接当成所有HTTP的签名费用receipt。

建议对v8新claim建立专门私有FreshHttpCostClaim/Receipt，复用同原path/inode issuer架构，但独立domain与receipt type。claim阶段在原claim writer同一事务绑定send前固定字段，received阶段只有真实headers transport持有的私有witness可以绑定status/received物理receipt和精确原三账行。保存canonical fact与独立seal；getter只能校验，不能从冷文件/JSON重新签名。所有received writer新增写权限须严格限定新receipt表，触发器不能写业务/其它Root/授权。迁移只建表，不写旧行。

## 可直接实施的代码路径

1. 冻结v8出生协议与容量义务；保留明确old-v7 fixture及新-v8 fixture。
2. 新私有HTTP费用receipt writer接 `claim_agent_http_request_on` 与 `receive_agent_http_headers_on` 原事务。headers收到后的费用写入始终允许保存原费用，即使原权限随后失效；保存费用不授权Root SDK。
3. 新 `coordinator_http_budget_frame.rs` 从原sealed receipt构造稳定frame；verify重读exactrows、seal、original_target_receipt及原Root birth。相同event已存Root请求但原fact变化，拒绝，不买新assessment。
4. 新 `coordinator_http_budget_hook.rs` 在活foreground原actor下枚举received receipts，付费tick → 原paid/publication复核 → Rust限step。无新child、grant、target、cap或C renewal。
5. `coordinator_frame` 与 `coordinator_rust_policy` 加inventory/classify/step映射；严禁把此advisory kind加入角色dispatch授权。
6. 下一SDK/target动作前，与现SDK预算消费hook并列执行；先纯读下一动作权限/硬额预检，物理claim仍原子复检。原未知费用优先拒绝，不能为未知target先买Root评估。
7. 在 `multi_agent_finish_execution` 原金融settlement、原supervisor/C核验之后、最终封套发布之前drain；完成重放共用原facts。末次HTTP后直接finish仍不遗漏。不可在headers/账本writer事务或holding实际HTTP invocation锁时发Root SDK。

“after received”指原receipt先保存，随后foreground边界消费，不要求headers返回时阻塞读取body。body未完整或WAF停止保留原已收费用；Root评估必须受原当前权威及停止规则约束，不能借费用确定重启目标。若要在adapter内主动评估，须先结束原物理HTTP生命周期并保存原结果，另行证明锁/取消/超时行为；首片不采用此扩展。

## 验收矩阵

| 场景 | 必需证据 |
| --- | --- |
| 一个实际localhost HTTP → headers receipt →下一动作 | target计数1；三费用行精确；HTTP预算Root SDK恰1；step限定；无增grant |
| 最后HTTP后直接finish | 最终封套前原budget评估恰1；new-evidence若执行是另一phase，分别计数 |
| 同原receipt完成重放 | 目标0新请求、Root0新请求、财务0新行；原seal/fee/claim不变 |
| 两物理claims，同URL或同evidence revision | 两独立event，各恰1；双侧identity保留每侧归属 |
| 目标未收到headers / transport未知 | 原reserve+forfeit保留未决；无消费Root SDK、无目标重发/退款 |
| headers收到body失败/截断 | target收费确定但结果不明；不伪造evidence成功，不自动继续目标；权威有效才允许原受限评估 |
| headers后取消/原C撤销/到期 | 原费用保存；Root/target0新请求；不得续租新C |
| 原Root监督预算不足 | 原cap不增；拒绝/defer可见；原费用保留，不能写成全部监督完成 |
| 冷改claim/status/received/source/worker/tool/fee金额或kind/receipt seal | 首次及重放均拒绝，零新SDK/target；不自动修复签名 |
| 原费用receipt写入故障/旁路触发器 | 原事务一致或明确未决；零业务/其它Root/授权写入 |
| Root已paid、publication故障/进程退出 | 两类原费用保留；只按同原回执恢复local publication，不重发SDK |
| v7真实旧Root / v8冷删字段 | v7保持旧语义不backfill；v8不得假装v7绕过 |
| finite最低capacity / unbounded合同 | 原有限headroom精确拒绝；无上限不invent scarcity；所有实际Root调用仍有原费用 |

实际测试须调用生产HTTP/SDK入口和原writer，不以手插三账行构造positive原费用；冷改只用于negative。目标请求和模型分别localhost计数，应用表快照核无额外写。完成focused tests后再严格Clippy、应用build；真实供应商/美元、桌面、全Master验收另列。

## 分工与完成定义

可并行：agent A独占v8 contract+private receipt writer；agent B独占frame/hook；agent C独占实际localhost验收fixture/tests；主控接生产入口、容量算法与合并门禁。共享include、版本库存和主文档由主控统一修改。不要让多个agent编辑同文件。

完成此片的定义是v8普通Multi实际HTTP原费用→独立预算监督→原回执→受限Rust评估，以及末次费用/重放/未知/撤权/冷改/故障验收通过。它仍不完成其它child预算变化、capability变更事件、Multi上传/写入/browser十维核算、Web租约到期或human-recovery。
