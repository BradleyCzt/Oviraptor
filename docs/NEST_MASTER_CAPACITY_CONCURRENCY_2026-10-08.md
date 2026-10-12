# REM-A03 容量计数、诊断与本轮回归（2026-10-08）

状态：继续开发中；Master 完整 REM-A01–A14 未完成。用户要求剩余额度 2% 时停止并总结，当前未触发。此文与主计划/进度顶部区分真实功能、只读诊断、骨架及测试证据。

## 真实功能

v6 Root 的每条物理 Web HTTP 在原 IMMEDIATE claim 事务内保留 pending+7 次模型请求最小容量。发现原计数把已验证的 Root 只读工具 publication 当作最终监督结论，实际两轮 SDK 测试复现 pending=0，预期 1。现在仍先完整验证 publication 的原模型付款、退出和时间线，再识别 localTools 中间步骤并保留义务，最终 decision 才解除。支付账目、物理六键与每条独立监督不变。

新测试最终证明中间时 pending=1、最终为 0、重复读取/重放无额外 SDK；原两轮费用重放邻近通过。失败日志保留，未跳过测试。

## 只读诊断与 UI

budget_gaps 对 v6 原合同增加 supervisionRequestCapacity，要求一致 SQLite snapshot。包含共享请求/token 余额、未完成监督的物理数量、下一 HTTP 所需最小请求次数，已知余额是否够、unknown 与 wall-time 拒绝原因。

所有输出固定 authoritative=false、executionAllowed=false、futureTokenCapacityGuaranteed=false、usdCapacityCoverage=not_enforced_by_this_ledger。读取不创建 Reserve、预算、付款或授权；old v1–v5/Single 不伪装有新版报告。v6 sidecar 缺失拒绝降级。真正执行仍重验原财务、撤权、保护和每项独立额度。

前端检查 safe integer、pending+7、六次最小后续、nullable 无限语义、已知阻碍枚举；任何伪许可/不一致数值拒绝。修复原合法 race_batch_dispatch_unimplemented coverage 被 UI 拒绝的问题，明确并发批次未接入。模型和目标账目标签改为“已接入的原费用账目仍有覆盖缺口”，消除旧“Single/Root 尚未接入”的错误文案。

## 骨架和缺口

当前仅请求最小容量，未保证未来 Root 任意多轮、未来 token/USD 成本、十维所有消费者或并行批次。共享 ledger 十维不含 USD：模型输入/缓存/输出/request、目标 request、browser、controlled writes、upload、concurrency、wall time。Recovery scope JSON 的 maxBudgetUsd 不等于已消费美元账本。

普通付费 Single 恢复测试实际拒绝 web_start_paid_history_requires_explicit_financial_contract；专属 Recovery birth、跨 Root 共享财务/起点时钟、可信 USD 消费仍待实施。保留该需求失败与普通启动门禁。已有 financial continuation 仅独立 Multi 只读 advisory Root，无 target/tools/saved queue，不能代称 Single 恢复已完成。

## 测试证据边界

证据 /tmp/oviraptor-master-capacity-concurrency-20261008/。

- capacity-after-local-fix.log：8/8，包含实际生产完整付费 HTTP/publication、Single SDK/目标账目、unknown/撤权、局部并发与中间步骤回归。
- 并发边界使用真实两连接、barrier、IMMEDIATE check/insert，前置物理 claim 是明确人工 fixture。没有并行模型 SDK/真实并行 worker 的结论。
- Single SDK 次数按真实 observed calls（当前夹具 13）和原 ledger 对齐；不假定单轮两次或伪造付款。
- client-regression.log：26/26；tick-sigkill-fixed.log：1/1，实际 SIGKILL 后原签名 supervisor 重入、原决策重放零新 SDK。
- gap-review-snapshot-fixed.log：1/1；审核前后 findings 完整字段逐字相同，初始化旧覆盖记录保留，非全库初始为空的错误假设。
- ui-full.log：907/907、0 skip，为标签最后调整之前的本轮快照；ui-capacity-final.log：最新标签 10/10；ui-build-final.log：build 成功。
- rust-check.log 类型检查通过，Clippy 快照 clippy.log 19 warnings / 0 errors，均早于最终中间 publication 修正；最终生产修正已测试编译通过，尚待最终类型/Clippy 封存。

起点测试二进制 test-bin 与 inventory.txt 为 2957 项，full-library.log 串行进行中，已存在失败。它不含后续诊断/新增测试/修正，不能称当前全库通过。每项复现、错误与修正日志保留，未删/忽略失败。外部 ownership_review.rs 在初次编译期间变动见 compile-drift.json；外部增量保护，不归因给作者。

保护边界：没有 reset/clean/批量覆盖/提交/推送，没有替换安装 App，没有读取写入真实业务库/CAS或删除资产；Native JSON 保留，不恢复 Strix；Exposure 业务不改动。

## 中途追加验收

最终生产源码 cargo check --lib / cargo clippy --lib 均 exit 0，普通检查 18 warnings，Clippy 19 warnings；final-rust-check-drift.json 为空，完整 Rust 源/合同在这两次检查期间零漂移。扩大邻近原物理来源及 fresh Multi/new-evidence 回归 32/32、0 忽略，174.461 秒，保存二进制库存 2961，见 local-fix-expanded-results.json。

Investigator 2/2 保留原 60000/6、80000/8 边界，出生前明确原 v5，真实一轮 Web SDK 退出后再交付，不更新旧 Root、不加预算，见 investigator-allocation-borrow-fixed.log。

新增实际两轮中间步骤 + 最终缺 usage 故障 1/1，5.61 秒，local-unknown-recheck.log：两次真实到达都保留 request 费用，最后 token unknown 保留，pending=1，重复进入零新增 SDK/账目变化。初次测试编译因外部 Exposure tests_g0 模块注册早于文件创建而失败；该文件随后由外部开发就位，重验成功，主控未补空文件/改 Exposure。

SDK 日志 changed-fact 旧假退出回归正在修正，原失败与费用计数校正日志均保留；当前尚不能算通过。本轮起点全库仍运行，已出现普通恢复等失败，不据此扩大完成声明。

## 原截止时间诊断与后续回归

实际出生前冻结 8 秒 deadline、真实 Root/Mapper/Root 后等待原时钟耗尽，未改时间/额度/源记录、未写 elapsed fact，诊断原先错误返回 freshWorkBlocked=null。现以 clock::remaining 只读核对仍有效的原时钟，耗尽返回 wall_time_exhausted，坏时钟/原绑定错误继续拒绝而非隐藏；不写耗时费用或新时钟，不将余额足够当许可。初次失败 unsampled-deadline-before-fix.log 保留，最新 capacity-with-deadline-final.log 10/10、0 ignore、53.42 秒（库存 2969）。

UI 展示三类确定阻碍及原只读边界，真实 SSR 模板分别验证三类原因；ui-deadline-final.log 10/10，ui-deadline-build.log build 成功。907 项 UI 全量仍属于之前快照，未挪作当前全量证据。

SDK 日志 changed-fact 回归 sdk-log-changed-fact-final.log 1/1、3.07 秒：保留实时 prepared/sent 与响应/费用/terminal 展示断言，Root 原帧重放不产生新诊断；真实 Web 一轮退出同时触发原预算评估，因此共享账目保留三个 Root、一个 Mapper、一个 Web 共五次 SDK。旧人工退出、旧费用数断言不再作为功能证据，原失败日志保留。

原生多角色路由 native-routes-fixed.log 1/1、10.32 秒：public surface capture 与 Web 三次 Broker 请求为四次实际 target 到达，两条入口捕获均禁止凭据，认证/匿名比较和模型输入不暴露凭据断言保留。未放宽原生产权限/Root 工具约束/审核门禁。

## 继续清偿回归债

应用 cargo build 已成功（deadline-app-build.log，18 warnings、81 秒）；最新 Clippy 20 warnings / 0 errors（deadline-clippy.log），其中新增 type_complexity 来自外部 Exposure/settings.rs；deadline-build-drift.json 为空。本轮只构建工作树产物，没有替换已安装 App。

完成唯一变动的 Strix 字面量 allowlist 条目语义审核：agent_tests_native_policy_original.rs 全文审核，只有 disposable frozen matrix 拒绝用的旧标记一处；执行/发布拒绝、零 SDK/HTTP/写断言保留。仅更新该文件完整指纹与原因，其他登记不改。retirement-reviewed-final.log 46/46、0 忽略、6.71 秒。

Human Root 六请求旧边界仅在出生前声明原 v5；正常 20 请求新任务继续 v6，不加额度、不重签已有 Root。human-capacity-regression.log 30/30、0 忽略、146.79 秒，保留真实 SDK、未知费用、拒绝前零写、原线程与重放等断言。

Reviewer 完成故障 reviewer-finish-fault-real-exit.log 1/1、3.09 秒：替换假退出为实际 Web SDK 与结算；coverage 单独保留，非 coverage 候选仍 0，原 failed 状态、ACK 回滚与所有真实 Root/Web/Reviewer 请求费用逐账对齐。不削弱审核门禁。

三项旧七/八请求监督 headroom 尚在验证；原全库二进制仍运行，至少包含两项普通付费恢复闭环需求失败。最新 UI 全量验证已重新启动以绑定期限文案，尚不能报告终态。

## 新版诊断 UI 与恢复实施规范

最新完整 UI ui-full-deadline.log 907/907、0 failed/skip，58.343 秒；ui-full-deadline-drift.json 为空，绑定最新原截止/费用/封存文案源码。headroom-original-regression.log 4/4，21.33 秒：旧 v5 原七/八请求 token floor、实际 Identity feedback/只读工具二轮、unknown 保留与不足准入拒绝；并不声称新 v6 的七请求任务可派发。

新增 NEST_MASTER_RECOVERY_SHARED_FINANCE_DESIGN_2026-10-08.md 为下一专属出生设计规范：原批准及 model/pricing 版本、精确 USD、原子首效应/UUID 收据、全模型/目标消费者、双 Root 共享与截止、未知/超额/晚到费用、SIGKILL/并行矩阵。明确先单 URL 真实闭环再双 URL，缺计价上界不能开放普通恢复；未新增运行时或执行授权。已给 Recovery worklist 标注旧编译阻碍是历史状态，不误读为当前阻碍。

## 归属并行改动的证据隔离

起点保存二进制的整库运行出现三项 ownership_o3_tests 失败：分析中变动的增量重分类、全量/增量一致性、4000 行 metrics。当前源码 ownership-o3-current-recheck.log 已 13 passed / 0 failed / 2 ignored，4.67 秒（库存 2969）；两项 ignored 是外部源码原有 20000/100000 行规模基准，主控未添加 ignore、未执行它们。

主控没有修改任何 ownership 生产或测试文件。起点 compiler 输入已记录 ownership_review.rs 漂移，当前通过仅是新快照定向结果；不把起点失败删除，不指认作者，不以 13 项通过称真实几万条已验收。

## 原财务 v6 与合法旧侧记录的降级边界

真实 v6 原财务 owner 保留，disposable 事务内将不可变侧记录换成可被原读取器接受的规范化 v5：前置 HTTP 容量守卫错误返回 Ok，只读诊断会隐藏 v6 报告。old-sidecar-before-fix.log 保留真实失败。后续原物理 claim 的财务契约核验仍拒绝不一致记录，因此不声称实际未授权目标 IO 已发生。

现在两处先核对原财务 owner，原 v6 不得通过非 v6 侧记录省略检查，明确拒绝 root_supervision_request_contract_changed；真实旧协议仍保持兼容，不升级旧 Root、不写新费用。capacity-sidecar-fixed.log 12/12、0 ignored、63.74 秒，库存 2980；覆盖中间 publication、未知最终费用、原截止、三轮工具上限重放与合法 v5 替换。missing-sidecar-current.log 1/1、1.53 秒，缺侧记录原有拒绝继续保留。应用构建正在复验，尚未把在途构建计作通过。

起点整库已发现 23 项失败，当前保存二进制逐项复验 failure-map-current.log/results.json：21 passed、2 failed、0 ignored；两项失败均为普通付费 Single 恢复闭环缺失，保留原需求断言和财务门禁。该 23 项清单复验不是当前完整整库通过。起点整库仍在运行，之后发现的失败需另行核对。

## 原账单有界窗口与旧测试骨架清理

真实生产费用盘点曾按 Root/轮次固定取前 100 项，已确认回执一直占前列，后续待确认账单不可见。inventory-priority-before-fix.log 复现前 100 个回执遮挡第 101 项。现在无回执账单优先，稳定 Root/轮次/call 排序；回执只是排序依据，原单据与确认回执仍逐项核验。保留 LIMIT 101 / 100 项展示与 truncated，不称完整分页；无回执但不可核验的账单仍保留 blocked，不能藉排序隐藏或释放费用。

inventory-priority-fixed.log 4/4、inventory-cost-expanded.log 10/10、0 ignored；原实际 SDK 收据、精确确认、并发 ACK、writer 故障、超限和 unknown 门禁保持。新增 101 行测试明确是选择算法人工行，不是有效账单或付款证据。inventory-priority-ui.log 13/13，实际 SFC 验证显式确认后只读刷新后续账单、原回执保留、无自动刷新/执行；前端 build 通过。与 continuation 的 UI 扩大组合正在验证。

REM-A01 旧占位函数清理：14 个仅被测试调用的监督/显示骨架限定 cfg(test)，真实原来源 producer 未改；两条不经过显式用户确认的外层财务测试入口也仅供测试，生产 confirm 仍调用原事务内 settlement primitive。预算拒绝枚举生产路径保留；只移除测试用 readiness/re-export 与未使用 VecDeque。原 JSON 收据验证显式比较规范化序列化字节，不能使用 Value 对字符串的比较替代。

fixture-gating-check.log 普通类型检查 4 warnings / 0 errors；后续清理 fixture-gating-clippy.log 主库 Clippy exit 0、0 warnings。fixture-gating-skeleton-tests.log 6/6；fixture-gating-live-evidence.log 1/1、6.98 秒，实际生产监督付费、完整 reentry 零 SDK；当前进一步费用组合、生产应用构建仍在封存。不把减少警告算新增角色/恢复功能，不删除测试或添加 ignore。

后续封存终态：fixture-gating-app-build.log 应用 build exit 0、0 warnings、17.99 秒；fixture-gating-cost-confirmation.log 10/10、0 ignored、10.35 秒；fixture-gating-ready-tests.log 2/2；inventory-continuation-ui-regression.log 53/53、0 skipped、2.008 秒。1759 源码/配置文件封存期间 fixture-gating-seal-drift.json 为空。测试 profile 仍有其自身 warnings，不能将主库零警告扩大成所有目标零警告。

## 2026-10-09 Master 复核检查点（继续中，恢复执行未完成）

- 起点整库保存二进制已有终态：2931 passed、23 failed、3 项原有 ignored，耗时 7747.89 秒。后续源码快照对同一 23 项复验为 21 passed、2 failed；两个普通付费 Single 恢复正向需求仍失败。不同快照不可相加为最新整库通过。
- 真实功能：InputParser 已直接使用生产派发标志，删除仅供测试开启派发的 override；实际生产路径及关闭/失败边界 6/6，真实付费 lease 路径 1/1。应用构建和主库 Clippy 当时均 0 warnings / 0 errors。
- 财务框架：精确 nano-USD、逐调用计价与独立上界计算 8/8。新 v2 定价批准已注册独立预览/确认/历史读取 IPC，原来源验证和私有签名保存通过 3/3；篡改原价格但保留签名会被拒绝。它只保存 approved_pending_priced_fresh_birth，executionAllowed=false、零目标 grant，不创建 Root、共享费用账本、时钟或新 SDK/HTTP。
- 测试证据：旧批准/草稿/取消协议回归 19/19；实际 Rust 公共批准回执与前端合同互通 5/5。这些使用一次性数据库与 scripted localhost SDK，不是第三方模型、真实美元账单或安装 App 验收。
- 下一步：专属单 URL 原子出生与真实共享财务消费者。当前云端 maxContextTokens=0 表示未指定；字节估算不提供可证明的美元上界，不能凭估算放行。旧普通恢复守卫保持，两项需求失败保留。
- 未提交、未替换安装应用、未修改真实库/CAS 或删除资产；Exposure 白名单分工保持。按用户要求持续监测额度，到剩余 2% 停止并暂停目标。


## 2026-10-09 原始用量回执检查点

生产 model/usage 现在要求所有已出现的 input/output/cached 别名为一致整数；有效首字段不能掩盖冲突字段、字符串或未知缓存明细。缺缓存字段以及 null 明细保留原兼容语义，明确缓存数值 null 不提升为已知费用。诊断中的原解析数字保留，但 usage_reported=false，不可据此自动结清占用或继续执行。

实际付费 Single 三种冲突/畸形场景一项测试通过：每场景一次 SDK，已知请求消耗 1、三类 token 未决、零目标请求。实际付费 Multi 监督 Root 一项测试通过：不 publication、原监督义务仍待结清、重入零新增 SDK/业务写。相关 ambiguous 回归组合 8/8；原 gateway 21/21；新别名合同 1/1。最新 strict-usage-final-app-build.log 构建和 strict-usage-final-clippy.log 严格 Clippy 均无警告/错误（该快照在随后 input-style 缓存解析小片之前）。test profile 仍有 3 项既有警告，不能称全 target 零警告。

v2 定价批准对应前端 build 通过；默认 test:ui 已纳入新增合同测试，最新完整前端 912/912、0 skipped。原费用框架和 v2 批准依然不创建共享 USD ledger 或恢复 Root。

另一个范围缺口：两个普通恢复失败来自用户手动暂停的 Single，不是已有保护停止收据。当前 v2 原来源仍依赖 Multi Coordinator 原保护收据及原退出证明；未来保护恢复的单 URL birth 即使完成，也不能自动宣称这两项手动暂停需求解决。须另有原 Single 暂停/已付 SDK/目标及 idle 退出来源、明确独立新 scope 授权与共享财务消费者；禁止制造保护收据、补 Coordinator、删除旧费用或开放普通 Resume 来替代。

## 2026-10-09 input-style 缓存与最终编译检查点

input_tokens/output_tokens 的已支持用量形式现在也识别 input_tokens_details.cached_tokens；它与 prompt/cache 其他别名必须一致。实际 Single SDK 返回 80 输入、10 缓存输入、20 输出：原十维账本分别消耗 80/10/20/1，reserved 与 indeterminate 全为 0，零目标消费，一次实际 SDK（input-style-cached-paid-single.log 1/1）。缓存是输入子集，不额外增加总 token。

随后当前源码回归：fresh_single 14/14、监督容量 13/13、root_cost_confirmation 4/4、用量合同 1/1；均零新增忽略。input-style-final-clippy.log 的 cargo clippy --lib -- -D warnings 与 input-style-final-app-build.log 的应用构建均通过、零警告。1688 源码/配置文件在该验证窗口零漂移。前端 912/912 和 build 属于本片之前的同一前端代码快照，此后仅 Rust 用量/测试与文档变更；不是最新 Rust 整库通过。

这些修正是真实已付费用识别与停止行为，不是共享美元预算或专属恢复执行完成。未提交、未清理真实数据、未替换安装 App。下一片先确保 v2 批准可以显式撤销并保留签名历史，再接专属出生和实际财务消费者；撤销不能删除批准/旧费用、不能自行放开普通恢复。额度最近确认为剩余 8%，未达到用户的 2% 停止阈值。

## 2026-10-09 v2 定价批准撤销与真实界面（局部完成）

- 真实功能：新增独立 append-only priced_cancellations；必须提供原 requestId / approvalId 和明确撤销语义。原 Mode 私有签名使用独立撤销目的域；保存撤销后重试批准拒绝，旧批准仍按原字节历史读取。只允许撤销表的原 INSERT，额外 trigger / 临时替代 schema 拒绝。当前无 priced birth；后续专属出生必须在同一原事务重新验证原撤销状态，不能只读历史 approved_pending 字样。
- 后端实际 paid localhost 来源：批准/撤销组合 5/5；两条真实连接同时撤销 1/1，只新增一条原撤销记录，原 Root / 费用 / 原批准及 SDK/HTTP 数量不变。原 approval 签名替换到撤销表会因目的域不同拒绝；故障 trigger 在写前由严格 schema 检查拒绝，不称实际 INSERT IGNORE 故障验收。
- 用户界面：现有 NativeProtectionRecovery 中新增可展开的精确定价批准。逐项输入 USD / 百万 token 与固定请求费用，按精确字符串转换 nano-USD；完整范围/原模型绑定/定价提案及单 URL 验证后才可明确批准。写前保存独立 v2 requestId，未知 ACK 重试原操作；明确撤销、只读找回原状态，旧 v1 草稿不升级。
- 新 SFC 行为与定价合同/旧 SFC 组合 23/23，覆盖存储失败、未知批准/撤销 ACK、重载、跨 scan、异步范围变更、假执行 grant、原操作丢失/替代拒绝，以及父范围不可用仍可显式撤销已核验原批准。前端 build 通过。完整前端 913/913 属于随后新增 SFC 测试前快照；当前完整 UI 尚待再收终态，不推算为全量已过。
- 编译：当前应用 build、主库严格 Clippy、cargo test --lib --no-run 均通过。删除无任何调用的历史 recovery_source 包装入口及两处测试无用 import 后，普通测试编译 0 warnings。额外 cargo clippy --lib --tests -- -D warnings 暴露 46 项测试 lint 债，清单 priced-cancel-test-clippy-debt.json；主库零警告不能称全 target Clippy 绿色。不用 allow / ignore 隐藏这些问题。
- 尚未完成：独立 priced draft/私有 Mode 与专属 Root 出生、实际共享 USD/十维/时钟消费者；普通手动暂停 Single 的新来源授权仍未实现，两项原正向需求保留。当前费用“按明确批准费率计算”，不是供应商美元票据或全项目新执行验收。
