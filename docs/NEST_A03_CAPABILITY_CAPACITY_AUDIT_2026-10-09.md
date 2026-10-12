> 2026-10-09 v10 原自然过期替代履行验收终态：`capability-expiry-capacity-natural.log` 2 passed / 0 failed / 0 ignored，603.67s。真实原600秒worker、原20分钟任务、实际SDK费用和signed撤权；正向原交付未paid仍pending1、实际finish付费expiry评估后pending0，全表/SDK重放不增；unknown费用保持pending1且阻断新工作、无delivery/Root新增SDK。当前App bin build exit0 19.96s（`capability-expiry-current-app-build.log`，Auth新合同修改之前），未安装替换用户App。默认仍v9；正常cap与expiry义务只覆盖显式v10 localhost验收，failed/cancelled、多worker恢复及全角色能力事件尚未完成。
> 下一Authorization输出监督开始实施：设计 `NEST_A03_AUTHORIZATION_OUTPUT_IMPLEMENTATION_2026-10-09.md` 已冻结，支持一个child全部N组、每组三侧与聚合verified0..N/费用3*N。新v11精确birth合同/optional规则只增声明，旧JSON跳过None保持原字节；容量leaf未include，纯读collection leaf并行开发，签名聚合output/生产hook/容量接线/真实验收尚缺。当前为未接通代码与只读设计，不能称功能。Master Goal active；不覆盖Exposure业务、未修改真实数据库/CAS、不提交。

> 2026-10-09 当前扩大负向验收：capability 9/9（37.55s，`capability-capacity-scope-publication-runtime.log`），新增真实 paid 来源异Root/异target/外worker只读拒绝及临时库精确撤销原publication后 pending恢复1、rollback原全表；不删除生产数据、不假造原费用。当前原 `root_budget_` 38/38（96.77s，`capability-expiry-root-budget-regression.log`），扩大后严格Clippy exit0（17.03s，`capability-expiry-expanded-clippy.log`）。原自然600秒测试session53255仍live，未重启、未缩TTL；其终态不能由上述局部通过替代。当前App bin build已启动（`capability-expiry-current-app-build.log`），尚未终态；默认v9、Master active。

> 2026-10-09 下一过期履行片已接线：仅同原 worker 的 expired/worker_lease_expired 才调用原 signed expiry delivery + 匹配最终 paid publication 只读验证；有原publication才免正常关闭义务，缺失仍 pending、损坏报错，failed/cancelled没有豁免。`cargo check --lib --tests` exit0 9.93s（本片测试扩展前）。显式 v10 两项真实原600秒自然测试已启动：session53255，`capability-expiry-capacity-natural.log`，正向核1→交付未paid仍1→paid0、未知费用核1且阻断；严格Clippy session21061 同时启动。终态未取得，不能称本片通过，默认仍v9。
> 前一请求预留片 App bin build 已终态 exit0 1m30s（`capability-capacity-app-build.log`），其编译镜像早于本次expiry履行修改；未安装用户App。Master及REM全部范围继续active，真实数据和asset保持原样。

> 2026-10-09 请求预留终态验收：`capability-capacity-boundary-runtime.log` 7/7（27.04s，零忽略）；真实已出生 v10 worker pending=1，同只读快照下剩余10拒绝新分配、11可分配1，原旧公式9仍可分配1，无账目或SDK新增。关联 `supervision_` 29/29（117.73s，`capability-capacity-supervision.log`）；严格 lib+tests Clippy exit0（16.05s，`capability-capacity-clippy.log`）。App bin build 已启动，终态尚待确认；未安装替换用户App。这些是局部 localhost/临时库证据，不是全量回归或Master完成。默认仍v9，Goal active。

> 2026-10-09 capability 请求预留更新：显式 v10 的真实正常 Web 关闭、Root 付费评估、零写入重放、原费用未知、Root 503、签名/跨 scope/撤权组 6/6 通过（23.18s，`capability-capacity-runtime.log`）。诊断新增 `pendingCapabilityTransitions`：原 worker 已出生为1，关闭但未评估仍1，原匹配付费 publication 后0，503仍1且阻断新工作。新 worker 分配及实际 HTTP claim 已计入这项请求余量；这是 requests 维，不是 Token/USD 保证。`cargo check --lib --tests` exit0 31.39s。新增容量边界第七项及关联监督回归正在验收，未取得终态前不称通过。
> 默认出生仍 v9；v10 仅显式验收。独立只读审查确认有效 v6–v9 沿用旧公式、合法 Authorization 不误拒；expired/failed/cancelled Web 暂保守保留待办，合法多 worker epoch/recovery、无原 owner 的更老 Root 兼容尚未完成。当前正常 capability 生产闭环不等于全角色能力订阅、全 A03 或 Master 完成。Goal active；未修改真实数据或 asset，未提交。

# A03 capability transition：default10 前请求容量审计

日期：2026-10-09。只读源码审查，不改 cap/floor/生产默认值，不运行 cargo。主控已报告 default9 镜像 capability 6/6、supervision 29/29、budget 38/38；这些是有界验收记录，不等于 default10 全库或容量保证。

## 结论

现有 `supervision_capacity.rs` 已把版本10列为 enabled，但 `FOLLOWUP_REQUESTS=6` 与 executor 算式仍完全共用6–9。新 normal-closed Web capability Root 评估额外消耗一次真实 model request，**当前没有对应的专属剩余义务，也没有证据能把其称为已保证覆盖**。可由现余量成功执行；原真实 Root admission 能在不足时拒绝，不能保证必有额度完成。

## 当前准入事实

- `executor_requests(available)` 使用 checked_sub(6+1)，要求差额>=2，再 /2；注释6包含 Reviewer、剩余 Root floor、Client SDK/feedback、InputParser trigger/SDK，额外1是首个 Web 调用前预算评估。新 capability 完成监督没有被列入。
- `coordinator_executor_allocation.rs:50` 对 enabled 版本调用此算式，仍限制原 grant；没有为版本10另加保留项。
- 每个物理 HTTP claim 和 compare2 纯读预检共享 `require_http_headroom_for_count`；原根 determinate/owner核验后要求 `pending + request_count + 6` 个共享 model request。只是数值保留，没有生成 budget reservation 或授予权限。
- `pending_obligations` 初始只收 Web HTTP claim 的精确六元组；仅查 `basis.phase=new-evidence` 的原 paid publication，再经 Tick::project_publication 验签/真实模型费用/终态。localTools 中间结果不能消掉义务。
- 该 pending 不包含 capability closure、budget-consumption 或 http-budget-consumption 的独立 Root 评估义务。new-evidence 完成不代表这些均完成；相同 HTTP 可能引发数个不同原来源付费评估，不能用单个 publication 抵全部费用。
- `coordinator_capability_transition_hook` 新付费前后的原 authority/determinate 和真正 Tick admission 保留，预算不足只能拒绝/defer，不能 raise cap 或伪评估。历史 paid publication 重放零费用；生产正常关闭 receipt/撤权不会恢复权限。
- diagnostic 明确 authoritative=false、executionAllowed=false、futureTokenCapacityGuaranteed=false、USD not_enforced_by_this_ledger。None 数值上限或部分容量检查成功也不能解释为未来模型 token/USD/截止时间保证。

## 风险边界

即使初始 grant 的半数 surplus 实际常留出额外 request，也只是常见余量。多 lane、多轮 SDK/HTTP、更早其他 Root 评估、预算/token不足或截止过期都能用掉它。根真实请求数有限，token不是请求数；完整 input/output 上界和USD属于另一个门禁。当前6项注释也不是全当前A03付费流程的完整账单证明。不能拿现38项绿色证明所有后续监督必可完成。

## 建议最小版本化方案（待主控决策，未实施）

保持6–9原声明、原 grant、原 reader 原封不动。版本10需原 signed bootstrap 冻结额外义务规则，例如 `capabilityTransitionCapacityRule=retain_one_original_request_per_issued_web_until_paid_capability_publication_v1`。这是一项从原预算保留的请求，不增加总 hardcap、不发新grant。

建议 API 可采用新增版本化 overload，旧签名继续原结果：

```rust
executor_requests_for_original(available, original_bootstrap, outstanding_closed_obligations)
require_http_headroom_for_original_count(tx, lease, assignment, run, request_count)
capability_pending_original_on(snapshot, original_owner) -> Result<i64, String>
diagnostic(...) // 共用 typed 原判定，附独立 capability pending 数量
```

初次 Web 分配在同原 IMMEDIATE 事务按原signed版本10预留未来 normalclosed obligation；每个新 Web assignment 增1，同原 assignment 重放不增。发行保留式应从 available checked_sub(原6+首budget1+已存在未完cap义务+本次新cap1) 后再原 /2，不分配原held headroom。具体已有 lane 的保留是否已被 grant 排除须以原 owner/reserve ledger 核算，避免 double count；不能只将全局6粗加为7后声称多lane覆盖。

HTTP物理准入同 typed原 pending：要求旧原 request_count+6+未完成 cap义务，新增 compare2仍2，且真正claim重复检查；已完成 Web 若cap付费缺失仍保留义务。报告共用相同来源，不用序列化diagnostic授权。

义务从原版本10已issued Web assignment/immutable grant 产生，不能等执行已completed或receipt落盘后才算。仅当该 assignment 的独立 signed normalclose cap receipt 与精确 `capability-changed` changedFact 对应、Tick::project_publication 验原 paid received+terminal+publication、没有localTools且非空唯一 `assess:capability_changed` 时移除。原已settled、cap已revoked、receipt已signed、队列图节点、空建议、失败503、unknown费用均不能移除。撤销/自然expiry如何关闭义务应有独立 signed cause证明并消费原规则；不能用expiry receipt冒充normal capability完成。

若要把整个当前A03都变成请求容量保证，还需独立枚举真实SDK每call、HTTP消费和new-evidence义务，按各原signedsource+独立paidpub分别消除；本最小方案仅解决cap新增1，不声称其余已经覆盖。其他费用未知时保持阻断，不扩大预算给出保证。

## 验收要求

1. 真实版本10、小原hardcap、双lane：Web发新SDK/HTTP前纯读拒绝若会耗尽最后cap保留；原claim/费用都0新增；不raisecap。
2. 正常closed后原cap义务仍1；独立真实Root receivedpublication后变0；同fact重放0SDK/0写。
3. source冷改/缺签/跨assignment/旧epoch/仅settled/空建议/localTools/503或unknown无成功pub：保留义务，不伪保证。失败实际fees继续保留。
4. 旧6–9原合同与grant完全不变；没有只读补签或迁移旧JSON。
5. 额度保留只证明model请求数：token、USD、deadline分别真实门禁，报告不得标guaranteed。

本轮仅新增此文档。default10启用与版本化保留规则由主控决定。

## 后续终态义务设计（只读，未实施）

正常关闭义务不应按任意状态字符串或任意 lease-expired publication 归零。过期分支拟要求同原 worker 精确 expired/failureclass、typed 原 financial receipt 与 signed execution_expiry delivery、原撤权前后 capability 和原 scope/ACK/outbox，以及同 changed-fact 的最终付费 Root expiry publication 全部通过，才记录只读的替代履行 disposition。缺失保持 pending，损坏返回错误；没有新 SDK、授权、续租或恢复。必须补显式 v10 的真实原600秒自然过期正向及缺 publication、未知费用、跨 scope、撤权/签名损坏负向后才启用默认。failed/cancelled 缺专属终态签名和费用闭环，仍保留待办；不可将这些状态直接等价为已付费评估。多 worker recovery 和古老无owner协议另列未完成。
