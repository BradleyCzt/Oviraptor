# Mapper General ReAct 原合同与实施规范（2026-10-09）

状态：设计规范，尚未实现或验收。仅定义 SpaApiMapper 的真实 bounded loop；不扩大 IdentitySession、DeepInvestigator 或目标访问权限。当前默认出生版本须由主控在实施时复核，本规范不批准切换默认版本。

## 1. 当前生产事实与阻碍

`commands/multi_agent/prepare.rs` 在实际 Root bootstrap/合法 Mapper 分配之后，构造稳定的冻结证据输入，调用 `multi_agent_child_round`，再通过 `deliver_readonly_assessment` 完成费用结算、子任务关闭和 `evidence_summary / mapper:<assignment>` ACK。

`commands/multi_agent/child_transport.rs` 的 `specialist_round_transport_bound` 当前冻结两条 messages、空 tools，派发一次实际 SDK。`multi_agent/specialist/dispatch.rs` 使用 assignment/child 的单一 `agent_specialist_calls` 原请求记录；已 received 只能本地回放，变更 request hash 拒绝，executing/uncertain 不自动重试。它不是逐轮派发 API。

`commands/agent_native/coordinator_mapper_allocation.rs` 当前合法 Mapper grant 只预留 **1 个模型请求**。tokens 来自原当前余额扣除 Reviewer 15000 token floor，再取不超过 8000 的正数；请求余额至少 2（Mapper 1 + Reviewer 1）。任务内 `reservedModelRequests=1`，`scheduler/bootstrap.rs` 的 readonly prepare 与 running Mapper 验证也固定 1。`budget/model/mapper_allocation.rs::original_mapper_grant` 严核四条原 reserve、first worker、原 source/idempotency 和 requests=1。

因此不能把 shared core 套在旧 transport 外连续调用；不能改旧 worker grant 为 2；不能复制 received 票据充当新轮账单；不能以额外 Root advisory 替代 Mapper 自己的工具观察与下一次 SDK。

## 2. 新出生合同与兼容

实施先选择未占用的独立出生版本和命名规则，不在文档硬指定版本号。新合同至少冻结：角色 SpaApiMapper、原 task/revision/input hash、允许的工具 schema/hash、最大 paid rounds、总模型请求 grant、总 token grant、每轮 output cap、完整输入估算规则、证据 inventory hash、历史编码与大小上限。

第一可验收合同建议最大 2 paid rounds、最多 1 次 evidence.lookup；工具轮后下一模型可最终收口。参数非法也消耗工具次数；轮次耗尽返回有费用证据的 bounded/incomplete，不能伪造 completed。未来多轮必须显式新合同并另验，不能由配置热扩大。

原已出生合同继续走原单 SDK 分支，全部旧正向/负向保留。新 schema 初始化只建新表/守卫，不更新历史 root、plan、task、grant、签名或 receipts。旧没有 loop 原凭据的 worker 不能被新接口接纳或补签；创建新 attempt 才能采用新合同。

## 3. 合法 paid Root 分配与出生

Root 原 bootstrap 选择必须已付费且发布，原 scope、签名出生、basis、eventSequence、summaryHash、Coordinator 与 root invocation 原持有关系均照旧验证。

在同一受限写事务中：验证原新合同 → 核实际剩余总预算及 Reviewer floor → 计算 grant → 创建 readonly assignment/first worker/caps2 → reserve 原四维模型费用与原执行槽 → 冻结 loop task/原分配证明 → 再核 scope/预算 → commit。

两轮请求 grant 需至少 2，加 Reviewer 1，及当前其他原监督义务；不能照搬旧 requests>=2 公式。token grant 仍不超过原 8000 上限，若两次真实输入估算和 output cap 在此范围不能成立，应拒绝出生或缩小**冻结前的**输入/合理输出上限并独立验证，不提高总 cap 为测试变绿。不得用所谓剩余诊断代替原原子准入。

Root 给的是 bounded 原合同，不给目标请求、鉴权、HTTP、文件任意读取、任意写入、其他角色派发或新的 capability。

## 4. 每轮费用与 opaque 原来源

新增独立 loop journal，不破坏旧 `agent_specialist_calls` 的一 assignment 一调用语义。建议键 `(root,assignment,worker_attempt,round)`，记录 original request hash、parent received/tool audit hashes、dispatch、received/unsent/uncertain、actual provider usage、cache provenance 和原物理 SDK receipt。唯一约束、immutable update/delete/replace 守卫、精确 main schema/trigger 校验均必需。

`begin_round_on` 返回 opaque 新派发或原历史 replay；无 caller JSON 能签出派发权。原 received replay 不网络调用；dispatch/uncertain 无完整闭环拒绝重放，要求对账。逐轮记录四维实际输入/缓存/输出/模型请求费，source 包含原 round/worker/call 身份，不能覆盖前轮或重复 charge。

同 snapshot 的 pure reader 必须验证完整 request、原出生/task/grant、worker、provider receipt、费用四维与原 clock、前轮链、工具 observations 与累计 usage。private opaque receipt/result/token 才能参与下一轮与最终完成，不用 `Value`、bool、总数或 caller summary 作授权。

首次已收到模型响应后，原 invoice 先独立留存。费用与该轮成功/失败语义保持原受限事务；金融阶段采用显式 Unsettled / FinancialAttemptFailed / FinancialCommitted。取消或 schema refusal 可结原 bill 一次；已经尝试失败不得自动重试 writer；结清后的 publication/ACK failure 不再次计费。两个原错误均可见，invoice/原 owner 留到返回。

## 5. evidence.lookup 合同

工具只接受 `{evidenceId: string}`。ID 必须属于出生 inventory；没有 path、URL、SQL、shell、identity、cookie、header、offset 逃逸或外部文件入口。冻结 inventory 映射 ID → 安全相对 artifact → 原 bytes hash/type/size；绑定 target、scan、attempt、root 和 evidence revision。

读取前后重核原目录、非 symlink、原文件字节与 hash，限制读取大小；返回脱敏的真实结构化片段与 evidenceId/hash/截断标志，不凭空补事实。不得网络 GET，不自动解压、不执行脚本、不打开模型建议路径。原 artifact drift 应停止，不把变化后的内容作为新证据。

正常 observation 必须是原 lookup writer/audit 产生的同一值，shared core 不合成第二份。invalid ID/schema 参数产生确定性已审计 rejection，最多允许下一合法 paid round观察该错误；未知费用、scope 错误、取消、撤权、原来源损坏均停止，不自动纠错重试。

## 6. Shared adapter 与完成

新增 Mapper adapter 使用 `controlled_executor::Adapter`，不修改 Web/Root/Source 默认策略。before_round 同锁核新合同、原 worker/caps、supervisor、Coordinator、已知费用、输入上界/轮次余额；model_once 只一次真实 SDK；record_paid_response 持久原 invoice；before_tool 不授予新的权限；execute_tool 返回真实 lookup；audit/checkpoint 保存原历史链；下一请求包含实际 audited tool observation。

最终 JSON 保留现 Mapper 结构：summary、priorityContracts、risks，并要求证据引用只来自冻结 inventory/实际 observations。不能声称访问目标、确认漏洞、得到新合同。text-only final 和工具轮最终结果都通过严格 original semantic proof，不通过万能成功 fallback。

累计 usage 从所有原 received bills 聚合，不能只取最后 SDK usage。assignment settlement 一次释放未使用 reserve，保已 consumed/indeterminate，不退款实际调用。完成/ACK 与原 terminal/capability 撤销保持同事务，只有正常合法 running fresh producer可发新完成结果。

历史免费 reentry 先核原完整 closed source、累积费、原最终消息 ACK/outbox 与 closure；不重新读取 live 模型配置来创造新权，不二次 SDK、lookup、ACK、grant 或费用。历史 reader 可验证已撤 C 的原结果，但不能复活新执行。暂停/未知/缺闭环不补签。

## 7. 必须通过的真实验收

1. 新 birth 合法 paid Root 分配 2 请求，真实首 SDK 请求 evidence.lookup，只读原 artifact产生 observation，第二 SDK wire 包含同一真实 observation，最终原 ACK/已知累计费成立，目标 HTTP 数始终 0。
2. 首工具参数非法 → 原 audited rejection进入第二 SDK，0 artifact 任意读/0 target费；若第二轮需继续工具或模型而 cap不足，精确 bounded stop，不能伪 completed。
3. 已完成整入口重入：全表/SDK/工具/费用/ACK均不增，payload与原结果相同。
4. 首 paid response之后撤 Coordinator 或暂停：费用仍保留，0 lookup/下一SDK/业务 publication；历史读取不成为 fresh authorization。
5. 实际 SDK503/unknown：原请求费保留，0下一SDK，第二入口精确对账拒绝；不把无 SavedDecision 的 no-retry误称费用unknown。
6. 逐轮请求、output cap、总tokens/requests不足与 complete input上界未认证分别拒绝；原旧单SDKfixtures仍保持 requests1。
7. 冷改 scope、worker、request/parent hash、original paid decision、signed source、tool observation、artifact bytes、ACK/outbox任一拒绝；攻击仅临时库，精确触发器drop/restore+rollback，全表与网络不增。
8. 同连接/跨连接 witness、额外恶意trigger/同名替换、missing seal不backfill、费用事务rollback与postfee ACK/publication失败各有实际负向。

## 8. 逐文件实施与分工

| 所有权 | 文件/新叶 | 实施内容 |
|---|---|---|
| 主控共享 | `agent_runtime/web_mode/bootstrap.rs`、`root.rs` | 独立新出生合同/typed test prebirth；旧版本保持原JSON，默认切换另验 |
| 主控分配 | `commands/agent_native/coordinator_mapper_allocation.rs`、`budget/model/mapper_allocation.rs`、`scheduler/bootstrap.rs` | 新版本专用 requests/token冻结分配、原 Reviewer/监督 floor；旧函数保持1 |
| 财务 agent | 新 `multi_agent/specialist/mapper_loop_*` leaves | per-round immutable schema/dispatch/invoice/known/unknown/纯 original read/opaque chain；不共用覆盖旧行 |
| 执行 agent | 新 `commands/multi_agent/mapper_executor_adapter.rs`、lookup叶 | shared bounded core、实际只读 observation、input估算与stop hook，不碰其它角色 |
| 主控接线 | `commands/multi_agent/prepare.rs`、`child_transport.rs`、`readonly_assessment.rs` | 新birth选新adapter；完成累计usage/ACK；旧transport/source/Identity/Deep保持原接口 |
| 来源 agent | `specialist/original_received.rs`、`closed_audit.rs`或新Mapper纯读叶 | explicit Mapper loop分支、正常closure/原fees/历史ACK强核，不能泛化所有role |
| 测试 agent | 新 `agent_tests_mapper_controlled_executor.rs` | 独立实际localhost SDK/冻结artifact fixtures、正向2round/全入口replay/上述负向；统一由主控Cargo |

开始编码前先冻结财务 API、input/lookup opaque API 与新 birth字段，声明独占文件；shared include/schema/issuer/maincaller归主控。第一交付应是一个可审阅的新出生真实2轮闭环，不能用测试数量、空 adapter、只读诊断或新Root建议宣称 Mapper ReAct完成。

本规范不实施身份访问、Deep目标探测、资产暴露面、Strix、真实数据库迁移或数据清理。无生产代码改动，无编译/测试新证据。
