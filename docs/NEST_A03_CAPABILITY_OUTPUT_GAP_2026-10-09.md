# REM-A03 capability 与剩余 child output 生产缺口审计

日期：2026-10-09。证据为当前源码逐调用只读核对；本轮不运行 Cargo，不接真实数据库，不改 Exposure。本文件不是功能完成或测试通过声明。

## 当前结论

REM-A03 仍为 partial。已生产连接的输出不能等同于任意 child 输出事件订阅；能力门禁能拒绝调用，也不等同于能力变化触发 Root。最新 HTTP 费用监督及 Web expiry 专属交付是不同来源，应分别验收，不能补算为 capability-changed。

## 输出生产调用库存

| 来源 | 实际生产位置 | 当前界限 |
|---|---|---|
| Mapper | `src/commands/multi_agent/prepare.rs:260` → `native_coordinator_mapper_decision`；`agent_native/coordinator_mapper_hook.rs:1` | 原 Mapper receipt/闭合输出进入 Root；不是通用角色输出扫描 |
| IdentitySession | `multi_agent/prepare.rs:250` → `native_coordinator_identity_feedback` | 独立原 session 输出及 mailbox 事实 |
| ClientSide | `multi_agent/client_side_execution.rs:59` → `native_coordinator_client_feedback` | 只读角色原交付；该文件55另有 expiry feedback |
| InputParser | `multi_agent/input_parser_execution.rs:93` → `native_coordinator_input_parser_feedback`，104实际只读执行 | 已存在生产派发和 Root advisory；角色执行与输出 feedback 两阶段不能混为一次 |
| DeepInvestigator proposal | `multi_agent/gap_investigation.rs:77,99,227,236` → `native_coordinator_gap_feedback` | 按实际 proposal/replay 验证，不是任意字符串输出 |
| EvidenceReviewer | `multi_agent/gap_investigation.rs:123` → `NativeCoordinatorFrame::reviewer`；`agent_native/coordinator_frame.rs:114` | 实际为 insufficient_evidence review-gap 路径，绑定原 received receipt 和 verified completed mailbox；不等于任意 verdict 通用订阅 |
| WebExecutor 正常闭合 | `multi_agent/execution.rs:109,117,130,132,165,168` | SDK/HTTP consumption、protection、新 evidence 独立 trigger；execution_result 并没有独立通用 output kind |
| WebExecutor 到期 | `multi_agent/execution.rs:26-48` → 专属 expiry delivery / feedback | 新专属 paused 财务交付链；无 completed，无恢复权限；需以主控最新真实测试验收 |
| BusinessLogic / Authorization / ExternalSurface | `agent_native/coordinator_rust_policy.rs:31-35` reserved kinds | 明确 not_ready。不得仅改 wired 列表或构造 JSON 宣称输出闭环 |

`coordinator_rust_policy.rs:5-22` 的 wired 库存是准入映射，不是事件源证据；本轮可确认上述明确 caller。没有发现统一“任意 child envelope 到达即 Root”生产消费者。剩余三角色需要先完成各自派发、费用、原输出与收口，否则新增 Root frame 没有真实源。

## capability 写入口与缺口

| 写入口 | 实际行为 | 是否独立能力变化 Root trigger |
|---|---|---|
| `multi_agent/scheduler/child_grant.rs:29-52` | 原调度事务创建 child 并 INSERT capability lease，按 assignment/child/capability 唯一键 | 否；现 Root prepare/budget 分配并非此写入的签名事件 |
| `multi_agent/scheduler/heartbeat.rs:121` | 更新 capability lease 到期时间 | 否；不应将每次 TTL 心跳构造成 changed fact，否则重复付费监督 |
| `multi_agent/scheduler/lifecycle.rs:293` | child 终态收回 capability，299释放 lane | 否；有闭合后条件，但没有 capability 专属原事件票据 |
| `multi_agent/attempts/expiry.rs:111` | 精确原 worker 到期撤权 | 原 expiry receipt/issuer 已有 lease-expired 来源；不能额外重复算作 capability 已覆盖 |
| `multi_agent/directive/reconciliation.rs:320` | 人工对账相关 child 撤权 | 有 directive 语义链，不是 capability 通用变化订阅 |
| `multi_agent/directive/ordered_execution/closure/started.rs:50` | ordered pause 精确撤权 | 不能以当前 revoked_at 字段冒充 signed source |
| `multi_agent/specialist/readonly_closure.rs:69` / `commands/multi_agent/gap_validation.rs:315` | 只读关闭/失败清理撤权 | 执行门禁及闭合；无独立 capability trigger |

当前普通 capability 行可见权限、作用域及 revoked_at，但这些写入口未生成“出生时私有签名的 capability transition receipt”。`child_grant.rs` 注释中的“签发 lease”是持久化权限行，不可据此声称密码学签名。直接 hash 当前 capability 行或旧 JSON 后重签，会使历史修改成为可付费事实，禁止采用。

## 最小下一闭环：精确原 child 终态撤权 advisory

优先于同时实现三个尚无实际输出生产者的角色：选择一个现有真实 child 的终态关闭作为首片，建议 WebExecutor 正常交付，复用 `scheduler/lifecycle.rs` 原闭合事务及 `commands/multi_agent/execution.rs` foreground。首片仅“已闭合原 child 的能力撤销变化 → Root advisory”，不含新增能力、不含 TTL 更新、不含所有角色。

1. 新出生协议显式声明 capability transition rule，旧 birth 缺 rule 保持拒绝、不补签、不回填。不能悄悄沿用 v9 默认准入：协议扩展版本由主控决定，并测试旧版本零写拒绝。
2. 私有 typed producer 必须持有实际终态闭合事务，捕获精确原 assignment/worker/child、原 Root epoch/fence、撤销前原 capability 集合和撤销后集合、原 closed execution_result message/ACK/outbox。只允许已在该事务撤销的原行；不接受 saved JSON、任意前后快照或调用者自报费用。
3. 首片签名复用原 verified birth keyscope，并验证原执行费用：SDK 使用现 original Web receipt/journal/private cost verifier；HTTP 使用 `web_mode/http_budget_storage.rs::read_original_on` 原 seal 与三条原 target费用。能力变化事实只绑定这些已闭合原来源，不把 Root 自己本次费用/当前全局余额/TTL 写入稳定 changedFact。
4. 独立 immutable receipt 含明确 transition id、cause 与签名；同事务闭合+seal。任何 INSERT/签名/触发器失败必须回滚整个该片写入，不留下能被误认为已监督的 receipt。查询缺失仅返回 None，不建表、不补签。
5. 新 frame/hook 用稳定 receipt id 去重，在 `execution.rs` 正常闭合后调用；Root 必须仍具原有效 C/live capability/determinate/原预算准入。child 已撤权不能拿到新 child capability；Root 回答限 advisory，禁止 localTools/新 assignment/续租/恢复/调高预算。
6. 后续同事实重放只查原 request/response/publication 票据，不能二次 SDK；对账未知、原签名冷改、其他 root/worker 混入全部拒绝。到期撤权继续走专属 expiry，避免同一次撤权双付费。

## 后续 child output 路线

先选 BusinessLogic 一个角色，按 `input_parser` 现真实链拆解：Root prepare准入 → 原费用预留 → 私有角色SDK transport → 原输出schema验证 → original费用/输出封套 → mailbox ACK → frozen输出frame → Root advisory。必须先完成 producer，再将 `business-logic-output` 从 reserved 移到 wired；Authorization 与 ExternalSurface 各自独立验收，不能共用一个泛型任意JSON输出签名器。涉及访问/写入权限应坚持原 capability plan 与用户授权，Root反馈本身不产生执行授权。

## 必须测试及证据边界

- localhost 实际 Web 闭合与原费用：一次真实 transition、一次 Root paid、原 mailbox ACK，重放零第二调用。
- 撤权前/后集合冷改、signature冷改、异 root/assignment/worker、旧 birth rule、TEMP shadow/附加 trigger、seal失败：拒绝且无伪成功publication。
- Root C到期/撤权、硬预算耗尽/indeterminate：在付费前拒绝；不续 lease、不自动恢复。
- 正常完成撤权与 expiry/directive不得重复评估；heartbeat单独变动零新增 Root call。
- 最后一次调用后直接 finish 仍能触发，commit后paid失败重试不重复费用/交付。
- 当前本轮只有源码审计和设计文档，没有新增测试或生产功能。主控 Web expiry 自然测试、严格Clippy结果须另列真实终态，不采用旧日志推定本设计已通过。
