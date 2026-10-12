> 2026-10-09 v10 原自然过期替代履行验收终态：`capability-expiry-capacity-natural.log` 2 passed / 0 failed / 0 ignored，603.67s。真实原600秒worker、原20分钟任务、实际SDK费用和signed撤权；正向原交付未paid仍pending1、实际finish付费expiry评估后pending0，全表/SDK重放不增；unknown费用保持pending1且阻断新工作、无delivery/Root新增SDK。当前App bin build exit0 19.96s（`capability-expiry-current-app-build.log`，Auth新合同修改之前），未安装替换用户App。默认仍v9；正常cap与expiry义务只覆盖显式v10 localhost验收，failed/cancelled、多worker恢复及全角色能力事件尚未完成。
> 下一Authorization输出监督开始实施：设计 `NEST_A03_AUTHORIZATION_OUTPUT_IMPLEMENTATION_2026-10-09.md` 已冻结，支持一个child全部N组、每组三侧与聚合verified0..N/费用3*N。新v11精确birth合同/optional规则只增声明，旧JSON跳过None保持原字节；容量leaf未include，纯读collection leaf并行开发，签名聚合output/生产hook/容量接线/真实验收尚缺。当前为未接通代码与只读设计，不能称功能。Master Goal active；不覆盖Exposure业务、未修改真实数据库/CAS、不提交。

> 2026-10-09 当前扩大负向验收：capability 9/9（37.55s，`capability-capacity-scope-publication-runtime.log`），新增真实 paid 来源异Root/异target/外worker只读拒绝及临时库精确撤销原publication后 pending恢复1、rollback原全表；不删除生产数据、不假造原费用。当前原 `root_budget_` 38/38（96.77s，`capability-expiry-root-budget-regression.log`），扩大后严格Clippy exit0（17.03s，`capability-expiry-expanded-clippy.log`）。原自然600秒测试session53255仍live，未重启、未缩TTL；其终态不能由上述局部通过替代。当前App bin build已启动（`capability-expiry-current-app-build.log`），尚未终态；默认v9、Master active。

> 2026-10-09 请求预留终态验收：`capability-capacity-boundary-runtime.log` 7/7（27.04s，零忽略）；真实已出生 v10 worker pending=1，同只读快照下剩余10拒绝新分配、11可分配1，原旧公式9仍可分配1，无账目或SDK新增。关联 `supervision_` 29/29（117.73s，`capability-capacity-supervision.log`）；严格 lib+tests Clippy exit0（16.05s，`capability-capacity-clippy.log`）。App bin build 已启动，终态尚待确认；未安装替换用户App。这些是局部 localhost/临时库证据，不是全量回归或Master完成。默认仍v9，Goal active。

> 2026-10-09 capability 请求预留更新：显式 v10 的真实正常 Web 关闭、Root 付费评估、零写入重放、原费用未知、Root 503、签名/跨 scope/撤权组 6/6 通过（23.18s，`capability-capacity-runtime.log`）。诊断新增 `pendingCapabilityTransitions`：原 worker 已出生为1，关闭但未评估仍1，原匹配付费 publication 后0，503仍1且阻断新工作。新 worker 分配及实际 HTTP claim 已计入这项请求余量；这是 requests 维，不是 Token/USD 保证。`cargo check --lib --tests` exit0 31.39s。新增容量边界第七项及关联监督回归正在验收，未取得终态前不称通过。
> 默认出生仍 v9；v10 仅显式验收。独立只读审查确认有效 v6–v9 沿用旧公式、合法 Authorization 不误拒；expired/failed/cancelled Web 暂保守保留待办，合法多 worker epoch/recovery、无原 owner 的更老 Root 兼容尚未完成。当前正常 capability 生产闭环不等于全角色能力订阅、全 A03 或 Master 完成。Goal active；未修改真实数据或 asset，未提交。

# 正常 Web capability transition 专属闭环实现记录

日期：2026-10-09。本记录逐文件核对当前源码，并读取首次实际运行日志；不运行 Cargo、不改共享调用、不接触真实数据库。功能范围是正常成功 Web 关闭撤权后 advisory，不能据此称 REM-A03 通用 capability 或所有 child output 已完成。

## 真实生产调用

`src/commands/multi_agent_child_delivery.rs:4` 的 `complete_target_child_delivery` 现在返回 `Option<CapabilityTransitionGuard>`。仅 success + WebExecutor + 原出生合同声明 rule 时进入捕获：原同 IMMEDIATE 事务中先 `capture_before_on`，再实际 mailbox send → consume/ACK → scheduler finish/撤权/释放 lane → 原交付和事件后条件 → `issue_after_on` → commit，最后把 guard 返回。不是测试专用 issuer。

`src/commands/multi_agent/execution.rs` 的 `multi_agent_finish_execution` 在正常交付返回后用 `_capability_idle` 持有 guard，再调用 `native_coordinator_capability_transition_feedback`；Rust 具 Drop 的局部对象到作用域结束才释放，原 SDK/execution idle 生命周期覆盖 paid Root 调用及之后的本轮 follow-up。任一 paid 错误退出才释放。本轮没有新增 worker、续租、capability 或业务动作权限。

已完成正常交付重放分支也调用 feedback。feedback 内 publication replay 先审原 signed source，再审原 request/publication；存在原付费 publication 时只读返回。缺票据返回 None，坏签名/费用不转化为“缺失”；新评估才经过原 Root C/live capability/determinate/预算准入。共享 caller 不因旧 birth 自动创建票据。

## 合同与签名范围

`src/agent_runtime/web_mode/root/bootstrap.rs` 当前默认 fresh schemaVersion 为 **9**。capability transition 专属合同为显式 **10**，要求 `capabilityTransitionRule="original_closed_web_capability_transition_advisory_v1"`。本片尚未默认启用，不能用首次测试推断所有新任务已获得能力变化监督。

`src/agent_runtime/web_mode/capability_transition_receipt.rs:65` 的 `capture_before_on(&Transaction, actor, child, &VerifiedMode)` 返回字段私有 guard：同原 Connection、原 actor/child/worker、verified mode、撤权前能力集合、原费用事实、event floor、实际 SDK/execution idle guards。原 worker 必须 live，assignment/run running 且费用已结算，原 lane 正在占用。能力集合与 assignment 声明一致且全部未撤销。

`issue_after_on` 不接调用者 payload JSON，只接 typed guard 和实际 message id；当前 reader 必须正常 completed（非 expiry/pause/directive），原能力行仅 revokedAt 发生变化，其他 id/作用域/epoch/fence/能力名/期限/出生字段精确不变。必须实际 execution_result → Root 固定 route/correlation/revision、一次 ACK、两条正确 outbox 事件，原 SDK/HTTP/clock 费用保持精确一致。

唯一 FreshCapabilityTransitionReceipt 构造器在 producer 私有函数内。`VerifiedMode::sign_fresh_capability_transition_receipt` 只签该 typed 事实及原 ModeFact，域为 `oviraptor.original-web-capability-transition.v1`，不暴露 key、不提供 saved JSON 重签入口。签名事实绑定完整前后 capability 集合、normal_completed_execution_result cause、原 actor/worker/assignment/child、原费用与交付物理 hash，并声明 executionAllowed/recoveryImplemented 为 false。

## 存储与原费用

新 schema 仅一张 `agent_web_capability_transition_receipts`，WITHOUT ROWID，receipt id 等于 canonical fact hash；worker/message 唯一，signature 32 bytes，no_update/no_delete/no_replace。读取严格 schema inventory/TEMP shadow；缺数据不创建表、不 backfill、不重签旧合同。

`read_original_on(db,actor,child)` 使用原 birth keyscope 验签、canonical bytes/hash/行scope，重新检查 completed/无lane/无active capability，前后集合与当前原行一致，原 message/ACK/outbox 与签名物理 hash 一致。纯读，不重新 probe 已由外层持有的 idle inode。

费用不是 caller 自报：SDK 使用原 `web_lifetime::child_usage_original` 原 receipt/journal verifier；HTTP 每个实际 source 使用 `http_budget_storage::read_original_on` 原签名及 reserve/forfeit/reconcile 三条费用证明；clock 使用 `clock::original_worker_samples::original_worker_consumed_on` 历史 prefix 精确来源验证。十维 model/target/clock consumed 精确匹配、reserved/indeterminate 为零；关闭后 concurrency slot 必须归零。费用 hash 包含原 journal、原 child budget entries（排除正常关闭需要释放的 slot）及 HTTP seals。Root 之后自己付费新增的 clock 不加入 child changedFact，不因自身费用反馈循环。

所有签名 INSERT 仍在共享原关闭事务；错误必须向外传播并由该 IMMEDIATE 事务回滚，不能 commit 一半后补票据。reader 不安装或替换共享 writer authorizer。

## 本轮交叉审

已核对 capture/issue 在同一个真实关闭事务；前后 Connection pointer 绑定实际调用，typed fresh 无任意 JSON 入口；commit 返回 guard，execution 正常新交付分支持有至 paid 结束。未发现该正常调用链的事务/guard 生命周期缺陷。

历史重放的 reader 不重探 inode、不恢复已 closed child；它只能使用原 signed delivery/费用。新 Root 付费依旧走 live authority 和原预算。它不接受 expiry cause；expiry 专属交付继续独立规则与事实。

## 实际测试证据与限制

读取 `/tmp/oviraptor-master-capacity-concurrency-20261008/capability-transition-first-runtime.log`：

- `capability_transition_actual_finish_assesses_original_closed_transition_once` passed。
- `capability_transition_actual_finish_replays_without_new_sdk_or_writes` passed。
- 总计 2 passed / 0 failed / 0 ignored，运行 4.71s；这次编译镜像当时默认 birth 仍为8，测试显式构造 v10。当前源码默认已为9，这两个正向结果不能冒充 default9→v10 启用验收。

实际生产 caller 已连接，但本记录不宣称负向验收或整体完成。仍需原签名/cap set/费用/TTL冷改、未知费用、schema/trigger故障回滚、Root权限/预算先拒、commit后paid失败重试、旧v9零写拒绝、expiry不重复等实际测试终态。原 clock 表没有独立密码学签名，自洽冷改整段历史的边界仍依赖现不可变存储与本片原生产签名快照，禁止把旧clock JSON补签升级为可信出生证据。
