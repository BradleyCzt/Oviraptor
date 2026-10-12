# WebExecutor 原租约到期监督实施方案（2026-10-09）

状态：本次只读源码复核与实施设计；只新增本文，未修改生产代码、运行 Cargo 或访问真实数据库。当前 HTTP 与预算片仍由主控分别验收，不能以本文、wired 库存或六角色测试声明 REM-A03 / Master 完成。

## 1. 原目标与精确缺口

Master 原六类事件中的第四类是 lease 到期：真实事件 → Root SDK → 原回执 → Rust 准入，且同一原物理事实去重。本片只补 WebExecutor；不是恢复、替换 worker、自动重试目标或全角色后台付费扫描。

当前已有真实机制：

- `src-tauri/src/agent_runtime/multi_agent/supervisor.rs` 的 `sweep/withdraw_due` 在自然时间到期后，通过原受限 `expiry_writer` 和 `NewlyWithdrawnWorker` 撤权、保存 original expiry receipt / issuance seal。后台 writer 不发模型或目标请求。
- `attempts/expiry.rs::try_expire_original_in_transaction` 将原 worker 标为 expired、原 assignment/run 标为 paused、撤销全部原 capability；保留 lane 与费用预留，未知费用没有被退款。原 worker/C/fence 绑定、deadline 及业务写入范围都复检。
- `commands/agent_native/coordinator_lease_closed.rs::lease_closed_mapper` 只接受 Mapper、InputParser、ClientSide、IdentitySession、DeepInvestigator、EvidenceReviewer 六角色。它要求原 signed expiry、原 specialist received invoice，以及原已结算、saved/ack 的 completed 本地交付。
- `coordinator_lease_hook.rs::native_coordinator_lease_expiry_feedback` 在 foreground 取得闭合来源、持有原 specialist SDK inode、调用 paid Tick、验证原 publication，并限定 `assess:lease_expired`。
- `coordinator_changed_request.rs::native_coordinator_require_original_lease_fact` 按原 expiry receiptId + workerAttemptId 拒绝来源漂移；当前时钟、原 C 的正常 TTL 变化、Root 新费用不能生成另一个事实。

Web 的具体断点：

1. Web 不使用 `agent_specialist_calls`，使用多轮 `agent_web_model_journal`；不能伪造 specialist 单轮费用替代它。
2. `multi_agent/execution.rs::multi_agent_finish_execution` 的现有原费用结算 helper 要求 run 为 running；自然撤权后的 run 为 paused。不能暂改 running 绕过权限检查。
3. `attempts::verify_expired_closed_audit` 的闭合形状是 assignment completed、run terminal/completed、零 reserves、无 lane/caps，不能把到期中断伪装成普通成功 completion。
4. 现六角色 hook 的 SDK inode 是 `specialist-sdk`。Web 必须核 `web-executor-sdk`、`web-executor-original-execution` 及原 tool/HTTP 生命周期，不能只换 role 名称或借用不同 inode。
5. production caller `agent_backend.rs` 在真实 `NativeAgentBackend.execute` 返回后调用 `multi_agent_finish_execution`。这是最小可达 foreground 收口入口；若原进程没有返回，后台只保留撤权事实，后续同原父安全重入才能做费用与只读评估，不自动拉起旧 Web。

以上为当前源码证据，不是本片真实运行通过证据。

## 2. 首片交付行为

原 Web 自然到期，后台保存 signed withdrawal；真实 Web SDK / tool / execution 已退出，原费用全部可核后，foreground 进行专属 **到期费用交付**，随后原活 Root/C 为该原 expiry fact 付费评估。Rust 只接受有界 advisory，不产生任何新 child、grant、cap、目标请求、C acquisition 或 worker renewal。

到期费用交付明确表示：原 worker 已失去权限，本次执行中断，费用与已收到回执已保存。它不表示目标成功、所有 body 完整、漏洞结论成立或 Master 完成。最终目标仍保持原到期/需人工处理语义。不能转普通 `execution_result` completed 来凑六角色 closure。

费用未知时只保存到期、原已知费用与未决记录，返回原 reconciliation-required。拒绝新 Root SDK、目标重发、退款和发布“监督完成”。首片不增加供猜测费用的零值 fallback，也不新增可执行 recovery 能力。

## 3. 新出生合同与旧来源保护

现 `web_mode/root/lease_expiry.rs` v1 的 closure 固定为 `original_cost_settled_saved_and_acknowledged`，消费者具体要求六角色 completed；不要默默将它解释成 Web paused 的新形状。

建议新普通 Multi 出生 bootstrap v9，保留 v8 的 SDK/HTTP 规则，新增独立 `webLeaseExpiryRule = original_expired_web_fee_delivery_advisory_v1`。旧 bootstrap ≤8 JSON、六角色 v1 closure 与已存 Root request 字节原样核验；不给旧 Root 添加字段、补 seal 或通过读取升级。

新 rule 的意义：原 signed Web withdrawal + 原已退出 transport + 原费用确定 + 专属到期费用 envelope 已 durable 保存/ack，才可进入 `lease-expired` 的 Root 评估。Root 的 leaseExpirySupervision v1 基础字段仍保留；Web 分支必须额外验证新 rule，不能仅凭 v1 基础存在执行。

所有 protocol 分支先验证原 birth seal 与原 finance owner，再决定 old no-op；v9 冷删 rule、改成 v8、跨 mode/scope 均拒绝。版本兼容与容量修改须列出精确库存；不能批量接受所有未知 version。

## 4. 原费用闭合证明：新增 typed 专属路径

建议新增 runtime `budget/model/expired_web_receipt.rs`，返回不可由 JSON 构造的 `ExpiredWebFinancialProof`。从原 signed expiry 和原 worker 取来源；不接受调用者自报 UsageDelta、checkpoint token_usage 或旧 summary 做账单。

复用并严格保留 `budget::model::settle_original_web_receipt` 的真实核验：

- 原 Root finance / original coordinator identity / original child receipt owner、全部 Web journal 的 cross-scope inventory。
- 多轮 dispatch 与 received/unsent 的精确原绑定、usageReported 与原 canonical charge；`web_lifetime::child_usage_original` 逐来源精确求和。
- 原 SDK inode idle；`web-executor-original-execution` inode 必须原已存在且可取得；没有 DB 行不等于没有实际 transport。
- 原 tool_invocations 不得 running；原 HTTP claims 全部取得 headers 或原协议明确未发送。status=0/received_at空保持未知。
- v8+ 原 HTTP fee receipts 的 immutable inventory / seal / 三账行核验，不能仅取图谱 observation。冷删 claim、费用或 seal 不得变成无事件。
- 十维原 worker balances 精确复核：本片只能清算原已发行/实际收费维度；不能替 Multi 发明 upload/browser/controlled-write 费用。任一维 indeterminate、残留未解释 reserved、负数或 overflow 都拒绝闭合。

物理核验应从现函数提取共用 helper，避免复制弱版；正常 running settlement 语义与原撤权后的费用结算权限要分开。新增 typed 证明只能授权同原 fees 本地收口，不授予 SDK 或业务发布。

事务内允许 run **保持 paused** 保存原 used_tokens/used_cached_tokens/used_requests，assignment reserves 精确结为零、budget_settled_at 保存；不能调用 current live worker gate、复活或改 C 来“完成”结算。原费用 ledger 权威仍来自 bound receipt。任一 projection/trigger 失败整笔回滚，原逐 call 收费不被删除。

目前 `attempts/expiry.rs` 的已有 expired audit 仅接受 paused-unsettled 或 completed-settled。若增加 paused-settled 的 Web 专属形状，必须通过新 rule + typed fee delivery 验证，不扩大其它角色 old predicate。不得只增加一个宽泛 OR 允许所有 paused 零预留。

## 5. 专属本地交付与状态形状

建议新增 `commands/multi_agent/web_expiry_delivery.rs`：

- 首次只接 typed `ExpiredWebFinancialProof`，写独立 kind `execution_expiry`、稳定 correlation `executor-expiry:{original_worker_attempt_id}` 与精确 dedup。
- payload 固定 schema/purpose、原 root/scope/assignment/worker/expiry receiptId、原模型累计 usage、原 HTTP fee receipt IDs、reason `worker_lease_expired`、`executionAllowed:false`、`recoveryImplemented:false`。不能复制原敏感 URL query/body/cookie/auth 或未验证模型结论。
- 本地 send + deliver + acknowledge 只表示费用中断报告已交付给原 Root。消息 writer须原当前 C 与专属 local fee proof，不能用原已撤 capability。当 C 失效时仍可保存原费用，但不能进行新业务 envelope、ack 或 paid assessment。
- worker 继续 expired，assignment/run 保持 paused + worker_lease_expired；财务 settled 与 execution completion 是不同字段。不能写 terminal/completed。lane 在该专属本地交付原子收口后释放，cap 必须已撤。若现 root closure/exceptional clock 不支持这个组合，补精确新出生 shape与负向测试，不能临时借用 completed。
- 新 `verify_expired_web_fee_delivery_on` 精确核原 withdrawal seal、fees、完整 message binding、payload、dedup、delivered/ack 原物理事实、协作事件、没有 live cap/lane。不写表、不创建锁、不补 JSON seal。
- 首次退出证明及锁应在 outer helper 中持有，跨原 fees/本地交付/frame/Tick 验证保持；避免每层再 probe 同一 inode导致自冲突。不要持有 SQLite writer或 HTTP invocation 内调用 Root SDK。

## 6. 稳定 Web lease frame 与付费入口

新增 `commands/agent_native/coordinator_lease_web.rs`，由既有 `lease_closed_mapper/verify_lease_closed_source` 的 Web 专属分支派入，不改六角色历史 shape。

语义：kind `lease-expired`，role `web_executor`，原 workerAttemptId + expiry receiptId + assignment/child/root/scope、专属 fee-delivery messageId、精确原 usage/HTTP fee receipt IDs、executionGrant=false、targetRequestsGranted=0。收费维度来源写清楚；不输出当前全局余额或动态时钟。

rows：原 signed expiry receipt/seal与不可变 worker identity、原 birth/frozen scope、新专属费用交付 envelope/ack、各原 Web received/unsent journal与原账行、HTTP sealed fee source、已撤 capability/释放 lane的原闭合证明。只冻结该原 worker 的来源，排除之后无关 child、Root 自身费用、更新中的全局事件/clock/TTL。

复用既有 `native_coordinator_require_original_lease_fact` 按 receiptId + workerAttemptId 稳定去重。相同 withdrawal重入当前 rows 漂移就报原来源冲突，不把漂移当成新可付费 fact。paid 后 publication故障，只按原 Tick/receipt允许的本地重放恢复，不发第二 SDK。

foreground producer `native_coordinator_web_lease_expiry_feedback(context, original_actor, original_child)`：

1. 原 signed birth + withdrawal +费用交付 typed/read-only proof。
2. 原 Root 当前 active scan/project/attempt、supervisor 与同原 C 当前 executable；不调用 acquire_coordinator_lease，不 renew worker。
3. 原共享 clock、Root SDK请求/token headroom准入；不足返回可解释停止/defer，不抬cap。
4. actual paid `native_coordinator_tick_for_frame` → exact frame reverify → original paid/publication确认。
5. Rust仅 `assess:lease_expired`，非空有界 suggestions 才称评估完成；空建议保留明确 defer。不能把step映射进角色dispatch权限。

`multi_agent_finish_execution` 应在最前面通过原 worker+signed expiry检测分流，**在正常 running settlement与普通finished_replay之前**走到期专属流程。正常活 Web 继续现有 SDK/HTTP consumption drain；expired分支不调用要求liveworker的消费hook来尝试恢复权限，也不以其它消费评估抵掉lease事件。

backend 接到 expiry结果仍走原异常目标 finalization，不再 Authorization/Reviewer/Client/InputParser目标续作。若 root closure不支持新 paused费用交付shape，须明确补该边界后才能声称该片全闭环。错误映射保留 worker_lease_expired / reconciliation-required / integrity / supervision-capacity 各自原因；不能都改“执行成功”。

## 7. 具体文件与并行分工

| 责任 | 新独占文件 | 必须由主控串行接入的共享位置 |
| --- | --- | --- |
| 原费用 typed proof +专属 paused原费用结算 | `agent_runtime/multi_agent/budget/model/expired_web_receipt.rs` | `budget/model.rs`及现 `original_web_receipt.rs`提取严格共用核验；`multi_agent/child_accounting.rs`只加typed paused分支 |
| 原到期费用本地交付/只读原闭合核验 | `commands/multi_agent/web_expiry_delivery.rs` | `attempts/expiry.rs`/expired audit特定新shape；mailbox/delivery scoped writer；root exceptional clock/closure若需特定库存 |
| Web lease frame +foreground hook | `commands/agent_native/coordinator_lease_web.rs`、`coordinator_lease_web_hook.rs` | sixrole frame Web派入与verify，`multi_agent/execution.rs`早分流、`agent_backend.rs`状态映射；不改其他预算消费者 |
| 新出生rule/capacity与兼容 | 主控独占root/bootstrap contract | new typed出生、birthseal、finance contract、version库存、shared supervision capacity；不自动补旧根 |
| 真localhost验收 | `commands/agent_tests_web_lease_expiry_feedback.rs` | 主控统一include/focused cargo/严格Clippy/appbuild；不得其它agent并跑Cargo |

分工前核文件不存在；任何共用物理核验提取须主控批准明确独占后实施，不让两个agent编辑同一文件。该拆分是建议，尚未创建上述生产文件。

## 8. 验收矩阵与证据范围

| 场景 | 必需真实证据 |
| --- | --- |
| 实际付费Web自然到期 | localhost实际Web SDK至少1次；原创建TTL自然流逝，production supervisor保存真实signed withdrawal；不手插expired/state/seal作正向；原SDK/执行退出后费用交付→lease Root恰1次→有界Rust评估，原worker始终expired |
| SDK响应在到期后才返回 | 原已收到真实invoice可保存/清算；新工具/目标0；Root仅在原idle+当前Root/C权限满足后评估，不把晚返回文本当新执行权限 |
| 实际HTTP后自然到期 | 真实localhostHTTP、原headers seal与三费用行保留；模型、目标计数分别断言；HTTP消费/新证据/lease各phase分开，不互相算次数 |
| 同到期费用交付重入 | 同原 receiptId/worker，目标0新请求、Root0新SDK、费用/封套0新行；前后全表快照一致 |
| 未dispatch自然到期 | 原从未dispatch/inode及无tool/effect原证明；费用确为0才允许真实零值；不能用“没有journal”单独证明；专属到期交付，不伪造模型invoice |
| SDK dispatch未知或HTTP没有headers | 原已收费/forfeit/预留原样保留；费用proof拒绝；lease评估0新SDK、目标0重放、无完成/退款封套 |
| tool/SDK/HTTP仍在执行 | 真实原inode占用/原running tool挡住闭合；不得因worker expired就认定transport已经退出 |
| 原Root/C撤销、过期、换fence/attempt/project | 可以核/保留原费用，拒新的交付或SDK；不acquire/续C、不签旧来源 |
| 初次/重入冷改来源 | expiry seal、deadline/worker、Web usage/source、HTTP claim/status/seal、费用金额/kind、payload/ack/event任一冷改都完整性拒绝；0新SDK/target，不重新签名 |
| paid评估publication故障/真实进程退出 | 原Web/HTTP及Root费用全保留；只允许同原paidTick本地恢复publication，累计RootSDK仍1；不凭历史JSON新签 |
| 原cap/clock不足 | 原有效hardcap与deadline不提高；停止/defer可见；不写监督完成；评估成功另需原足额cap真实正向 |
| 原sixrole/v8旧合同 | 原测试断言、payload字节与收费不变；oldRoot不出现新字段/新交付；新rule冷降级拒绝 |
| expiry/费用/交付writer触发器故障 | 本事务原子或原真实已付费用保留；projects/其它Root/授权0额外写；无假完成状态 |

当前生产 worker 初始 deadline 随原 C 的 600 秒发行；没有独立短 worker TTL 配置。`coordinator_heartbeat.rs` 仅更新 C 的 expiry/heartbeat，authorizer 不允许 child 写入；`supervisor.rs::sweep` 调用该 C-only heartbeat，不等于续 worker。`commands/agent_native/lease.rs::ExecutorLeaseHeartbeat` 在 foreground启动时调用 `refresh_running_executor_leases_authorized`，并在其独立后台线程每60秒调用普通 `refresh_running_executor_leases`；guard drop停止并join该线程。它与C-only parent heartbeat是两种独立生命期。真实自然验收可在原付费调用和执行退出、该ExecutorLeaseHeartbeat guard已drop/join后不再进入foreground，保留 worker 原 deadline，等待约600秒、由合法 parent C-only heartbeat保活；原Root clock本身必须从出生即足够覆盖此等待，不能运行中抬限。此为源码可行路径，尚无该600秒测试通过证据。不得新增纯测试短TTL生产特例或UPDATE fixture deadline/state制造“自然到期”正向。冷改只用负向临时DB并标证据范围。所有等待有界、在父模型监听器建好后出生，不抬cap避失败。旧六角色自然expiry相关现成测试用于回归，不能代替Web正向。

首片完成需上述实际链、重放/未知/撤权/故障门禁通过并且严格Clippy与应用build有新终态。第三方供应商、桌面交互、全Master验收另列。human-recovery、替换worker/补授权、其它target维度及通用capability事件仍不在本片内，REM-A03维持partial。
