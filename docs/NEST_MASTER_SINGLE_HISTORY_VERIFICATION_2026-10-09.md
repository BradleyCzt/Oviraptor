# Master Single 原费用历史与并发核验检查点

2026-10-09；Master 未完成，额度已实时确认剩余 2%，按用户要求已暂停目标并停止新增开发。所有运行证据在 `/tmp/oviraptor-master-capacity-concurrency-20261008/`。本文把真实生产变化、只读能力、框架和测试分开。

## 已接入生产

- 新定价提案和私有批准精确拒绝单次固定费超过全部 USD 额度；只算零 token 固定费下界，不能据此准入完整请求。历史批准保持旧冻结及签名算法。
- 预算余额读取拒绝未知维度；不再以全零余额掩盖名称错误。
- 原 Single 历史通过 Single owner、原 SDK/HTTP idle inode、SingleExit、原模型 dispatch/终态和四维原费用核验；不误走 Multi Tick。
- 原收到用量必须已报告、字段恰好匹配、缓存为输入子集、总量一致；费用分类及原 reserve/consume/release/unlimited 键逐笔对应，不只核总数。
- 当前 Single 网关只有取消可产生 BeforeTransport。网络失败码不能作为 unsent 零费用历史；保留原账，审计拒绝，不补账、不退款。
- Single 财务指纹包含原 SingleExit 物理行。仅 Single 新分支追加；Multi 旧指纹算法不变。Multi 同 Root 混入 Single 退出回执会拒绝，其他 Root 的记录不影响原事实。
- 原 HTTP 费用核验除原 claim 的三条精确账目，还检查每一追加行的原幂等键；同一实际 HTTP 被换键重复记费不能被认作合法原费用历史。
- 浏览器动作、受控写入、上传字节、并发批次尚无原生物理执行票据消费者；即使插入余额归零的 reserve/release 对，也不能作为已闭合原历史。Single/Multi 读取均明确拒绝，未开放这些动作。

## 已接入的只读诊断

现有预算 IPC/实际 Vue 面板展示 Single 原历史 verified_closed_history 或 not_verified。核验中不写应用表，不发送 SDK/HTTP，不创建或续期时钟，不确认用户同意。Source 不投影为 Single Web 历史；界面拒绝异 Root、执行/请求 grant 或供应商美元账单声明。错误只显示固定文案，不传播原 SQL、路径或用量载荷。历史已经闭合后，原截止时间已过仍可读取；这不能授权执行。

## 框架及仍未完成的能力

精确 nano-USD、RateCard、上界计价及 v2 签名批准/找回/撤销已经存在；v2 仍为 approved_pending_priced_fresh_birth、executionAllowed=false、targetRequestsGranted=0。

缺口仍是专属出生、跨 Root 的共享十维/USD/时钟消费者、完整输入计费上界及人工恢复监督 feed。原普通已付 Single 恢复不因历史读取成功而开放。两项原正向恢复需求在当前 3069 固定镜像复验仍报 web_start_paid_history_requires_explicit_financial_contract；不修改断言、守卫或 ignore 消除失败。

下一片必须推进执行合同与真实消费者，不能把本页诊断或 SDK 并发证据折算为恢复完成。实施规约见 NEST_MASTER_RECOVERY_SHARED_FINANCE_DESIGN_2026-10-08.md。

## 测试证据及适用范围

- 3033 项完整库保存快照：3027 passed / 2 failed / 4 快照已有 ignored；不是当前整库结果。
- 3066 固定镜像：财务续接 32/32、定价保护 6/6、Single 在途 9/9、Single 历史 5/5、预算诊断 2/2，均零忽略。
- 真实 SDK/HTTP 历史：多轮模型和一次目标 HTTP；重读表不变、请求不增加。原回执分类不一致、未报告用量、额外字段均拒绝；截止已过仍仅可读。
- 实际两个独立扫描同时 SDK、暂停隔离；同一数据库四个独立扫描共用一个 provider，前三个同时发送，第四个真正排队后取消零发送；前三个各一次费用保留。
- 同一个 Single Root 在途时，同请求重放、改变原请求及下一轮均无新账目/SDK，原请求仍可完成。不能据此称同一个恢复 scope 的多角色共享预算已验收。
- 完整 UI 929/929、零跳过；实际诊断 SFC 13/13，类型检查/前端 build 通过。
- 3067 固定镜像：原历史 6/6、Single 历史 5/5、Single 在途 9/9、财务续接 32/32、定价保护 6/6 全部通过。
- 最新库存 3069，新增未实现动作的平衡假账拒绝：Single 历史 6/6、Multi 定向 1/1；3069 固定镜像：原历史 7/7、财务续接 32/32、定价保护 6/6、Single 在途 9/9、Single 历史 6/6 全部通过，零忽略。最新 cargo build 与严格 lib+tests Clippy 均通过、零警告。
- 完整 UI 929/929 对应本轮标签细化之前；细化后的实际 SFC 13/13 与前端 build 通过。

失败基线包含原 Single 错用 Multi Tick、未知维度返回零、退出行漏绑、Multi 混入 Single 退出、错误 unsent 码以及原 HTTP 同源换键重复费。冷损坏/换行位置/重复收费仅发生在一次性 fixture；需要移除触发器的测试读取并原样恢复其 DDL，生产 immutable 约束不改。

这些是 scripted localhost SDK/HTTP、实际生产调用与原物理行的证据；不是第三方模型、真实美元账单或安装 App 验收。不同镜像的局部测试不拼为当前整库绿色，不据测试数量计算 Master 百分比。

## 数据和并行边界

未 reset/clean/批量覆盖/提交/推送，未替换或启动安装应用，未修改真实数据库/CAS，未删除资产。Native JSON 保留，不恢复 Strix。Exposure/归属业务仍属于其原分工，主控本片未编辑这些文件。1782 条已选源码/配置路径封存供后续验证，不能当作全部 git dirty 文件的数量。

## 下一执行闭环的具体门槛（尚未实现）

1. 先实现原 Single 未知费用的人工确认：原费用 IPC 只枚举 received 且调用 Multi Tick 读取器；本轮追加 uncertain 的不可确认展示，确认写入仍不支持 Single uncertain。新确认必须单独保存有来源的不可变事实，绑定原 scan/attempt/root/call、原 dispatch/request hash、原 unknown 原因与原费用保留；不能改写 uncertain 为 received，不能从未知用量推导零费用。
2. 确认预览和确认写入应绑定同一原账指纹，使用原子事务拒绝过期、跨 Root、重复不同金额及并发改变；幂等同一事实只返回原收据。费用超过原保留时需要可审计债务事实，不能退款或删除旧账来制造余额。
3. 已核对 SingleExit proof::sources 只绑定原 owner/limits/clock/definition/mode；binding 绑定原运行身份，wall 单独绑定原耗时条目。它没有绑定模型费用表，不能据此假设新增人工确认天然可读或可执行。需要专用确认收据读取与确定性费用核验，不修改旧退出证明算法。
4. 确认完成只解除原费用不确定性，保持 executionAllowed=false。专属恢复出生必须另有显式批准、目标约束、冻结价格及完整计费上界，在一个事务内绑定原费用历史；普通恢复守卫继续保持。
5. v2 已批准 scope 当前是 Multi。不得用自动降为 Single 或仅固定费例外绕过该 scope；USD 精确核验还须区分缓存用量已报告与未报告，现有默认缓存零值不构成第三方美元账单证据。

验收须包含真实原 SDK 未知响应→用户预览→确认→重读，以及过期/并发/跨范围/超保留失败；确认路径零 SDK、零目标请求、零新执行 grant。随后另验专属出生和共享十维/USD/时钟消费者，最终恢复两项原正向需求。以上是后续实施约束，不是本轮完成的功能。

## 未知费用队列可见性修正（已验证）

真实已发送 Single SDK 在暂停后保留未知费用，但原费用队列查询仅 received，导致界面将它漏掉。生产查询现在纳入 uncertain：已有可核对 received 优先，随后未知项，最后已确认收据，保持 101 条截断探测和人工刷新。未知项经原验证入口拒绝，只进入 blocked，不提供确认按钮、退款或执行权限。实际生产 SDK 并发/暂停回归先复现 blocked=0，修复后 blocked=1、items/receipts 为空、所有应用表不变；直接尝试确认被拒绝且应用表不变；原确认/对账组合 10/10、Single 在途 9/9 通过，严格 Clippy 与应用 build 零警告。同一原 call 的多个物理终态只选一个候选，不改写或合并回执；去重失败基线保留，修复后原确认/对账 10/10、Single 在途 9/9，最终 Clippy 与应用 build 通过、零警告。这是只读展示修正，不是未知费用人工结清。

## 原 HTTP 工具授权记录核验（已验证）

实际原 SDK+HTTP 夹具表明：原 tool_invocations 的 policy_decision 冷改为 deny 后，旧 target 历史读取仍接受。现在每个原 HTTP claim 必须关联同一 run/invocation/tool 且 policy_decision=allow 的原记录；缺失或不匹配拒绝，不能凭 HTTP 回执与费用键替代工具调用依据。库存 3070 固定镜像：原历史 8/8、财务续接 32/32、定价保护 6/6、Single 历史 6/6、Single 在途 9/9、原确认/对账 10/10 全部通过；有效原私有提案正向断言已通过，应用 build/严格 Clippy 零警告。两项原普通恢复需求当前镜像仍失败于显式财务合同缺失。选定 1782 路径验证窗口零漂移；完整工作树只读盘点 639 个 dirty/untracked 文件，611 个源码/文档哈希，不将两种范围混为全库验证。所有冷损坏仅发生在一次性夹具，读取全表不变、SDK/HTTP 数不增加；不对运行中的工具授予任何新权限。

## 同来源并发批准与 Single 单笔账单分类（已验证）

原定价表的唯一约束已经阻止重复批准。两个实际连接用不同请求 UUID 同时批准同一真实付费来源，旧结果为一个成功、一个 immutable 冲突；现在同一写事务先读取并核验原签名收据，返回明确 source_already_approved，已撤销来源返回 revoked。相同原请求的两个连接重读得到字节一致的原批准；换请求 UUID 不绕过撤销。最新定价组合 7/7，零新 Root/SDK/HTTP/grant。它验证后台连接并发，不等于两个真实应用窗口或跨进程 SDK 验收。

又复现了已结清 Single 账单被错误送入 Multi 对账器而显示未解决。已提取原单笔读取器，验证冻结 Single owner、规范原 dispatch/派生 call ID、单一终态、已报告用量与四维逐条精确费用。它可以核验前一张已收到账单，不要求另一张在途/未知请求已经结束，也不宣称整个 Root 闭合；完整历史入口仍保留原 SDK/HTTP idle 与 SingleExit 核验。

费用候选流先排除通过核验的 Single 原账单，再截取 101 条需要处理的候选，不能让已知账单占满前 100 条隐藏后面的未知请求。Multi、损坏、不完整或未知账单不被排除，同一原调用去重但保留所有物理回执。实际已付关闭、第一轮已付后第二轮取消、首轮取消组合在 Single 在途 9/9 通过：已付账单无确认候选，混合情况下仅未知原 call 显示 blocked，全部应用表不变。本片不补账、不退款、不授予执行；单笔已知不意味着整段财务历史已闭合。3071 固定镜像：Single 在途 9/9、Single 历史 6/6、原确认/对账 10/10、原费用历史 8/8、定价 7/7、财务续接 32/32 全部通过。随后只将规范 JSON 序列化字符串命名以解决 Clippy cmp_owned，保留字节比较而不采用 Value 字符串值比较；此当前源码的 Single 历史 6/6、应用 build 与严格 lib+tests Clippy 均通过、零警告。镜像源码封存后仅此一个已记录的表达式写法差异，最终源已另行封存。

## 停止交接

2026-10-09，实时查询账户剩余 2%，goal 更新为 paused；不再新增开发，所有已启动验证已结束。Master 未完成，没有可据以计算可信总百分比的统一验收分母，不以局部测试数换算完成度。

真实生产：原历史严格核验及重复批准的明确冲突/撤销检查。只读诊断：Single 已知账单排除、未知账单保留、候选去重与截断。框架：精确定价与 v2 批准仍不创建可执行 Root。证据：localhost SDK/HTTP 与一次性数据库、固定镜像局部回归；不是第三方账单、安装 App 或当前全库绿色。

恢复后优先推进有来源的 Single 未知费用确认、专属恢复原子出生和共享十维/USD/时钟执行消费者；完整输入计费上界、缓存用量报告来源与原债务处理必须明确。两项原普通付费恢复需求保持失败，不放宽守卫、不改断言或新增 ignore。真实数据/资产保持保护，Exposure/归属业务仍遵守原分工。

## 2026-10-09 用户恢复继续：取消 2% 停止线

用户明确要求继续，额度耗尽后由用户重置；不自动重置额度。此前暂停记录保留为历史检查点。当前 Goal 工具没有 resume 参数，旧 Goal 元数据仍为 paused，不通过虚假 complete 或替换目标绕过；当前工作已获得用户继续授权。

- 工作树封存复核：暂停封存的 1782 个路径中，续接复核只发现主控本轮新增测试文件变化；不因此声称所有未跟踪文件均无外部修改。
- 测试证据：真实 creator/原 Single Root/localhost SDK 连续 101 次收到原用量，第 102 次实际在途暂停；原账与 SDK 退出核验后，生产费用列表仍只显示这一未知调用、truncated=false，读取不改变应用任何原表。独立回归 1/1，23.47 秒，3072 项快照。测试容量在原 Root 出生前冻结，不提高已有预算；只使用一次性测试数据库和 localhost 模型。
- 真实功能改动（复验中）：网关响应及逐请求 JSONL 日志新增 cachedUsageReported，区分明确缓存零、明确非零与缺报；重复 usage/cache 键仍使该来源标记为 false。旧 token usageReported 兼容规则保持；旧 Tick 反序列化明确 false，不能从旧默认零数值补造新的缓存来源。
- 边界：该标记不是美元计费消费者，不改原不可变费用协议或原签名；未知 Single 的供应商账单导入/确认、专属定价恢复出生和共享 USD/十维/原时钟执行仍未完成。两项原正向恢复需求未改为负向，也未跳过。
- 证据目录：/tmp/oviraptor-master-capacity-concurrency-20261008/。新测试首编译误用字段 hard_tokens/soft_tokens 的失败日志保留；已改为原真实字段 hard_total_tokens/soft_uncached_tokens，未改生产预算。

续接验收：当前封存源码差异零，库库存 3073；模型 27/27、Single 在途 10/10（含 101 笔已知后第 102 笔未知）、Single 原历史 6/6、费用组 10/10、退役已审查源码 1/1；应用 build 与严格 lib/tests Clippy 零警告。对应 cache-provenance-*.log、cache-provenance-source-seal.json 与保存镜像 test-bin-cache-provenance-3073。退役清单仅更新已人工核对的 model/tests.rs SHA，原唯一旧后端注释及计数保持；新增测试仅 localhost，不引入旧 backend。定价保护及两项原必需恢复需求终态另收，不能把部分组通过视为恢复完成或当前整库绿色。

当前 3073 保存镜像的两项原正向恢复需求已复跑，均仍失败于 web_start_paid_history_requires_explicit_financial_contract；对应 cache-provenance-required-results.json 和两份完整失败日志保留。未将其忽略或更换断言。

定价批准管理当前保存镜像 7/7、零忽略，所有本轮关联组退出码 0；最终选定源码封存比对差异零。该组仍只证明批准/撤销/并发幂等及拒绝门禁，不是定价恢复执行。

## 2026-10-09 后续片：已报告用量的定价前置门禁

精确定价原语新增 quote_reported_response，显式消费实际网关的 usageReported/cachedUsageReported。未报告完整用量拒绝；缓存费率不同且非零输入、未明确报告缓存时拒绝给出确定价格。费率相同或已核数值为零输入时金额不依赖缓存分配，允许算价而不改变来源标记；原 numeric quote 继续负责模型绑定、负值、子集、总量、单次请求和溢出校验。

真实传输测试：四次 localhost SDK 返回缺缓存/明确零/明确非零/重复字段，实际响应直接传入前置算价门禁，分别拒绝或得到精确 nano-USD，不增加请求或写预算。原语测试另覆盖昂贵缓存、未知总用量、相同费率、零输入、错模型绑定与错误总量。

分类：这是带真实网关测试的定价前置原语；目前仍没有恢复结算生产消费者，不授予 Root、SDK、目标请求或 USD 余额。传输来源标记不是经过原 scope/模型/原调用和批准证明核验的不可变供应商账单。Single 未知账单导入、不可变确认、专属恢复出生及共享预算执行仍未完成。未修改两个原恢复需求、普通启动拒绝、原不可变账目或签名。

本片只改 recovery_finance.rs、recovery_finance/tests.rs、model/tests.rs 及该已审查测试的退役 SHA；暂停后源码封存对比未发现其他选定源码变化。证据 reported-price-gate-*.log 与 reported-price-gate-source-seal.json，验收终态另附。

本片终态：当前源码算价 9/9、网关/模型 27/27、退役核验 1/1，零忽略；严格 Clippy lib/tests 与应用 build 通过、零警告，最终选定源码封存差异零。库存 3074，没有运行当前整库。前片两项原付费恢复需求仍未实现，本片不改变普通启动或它们的断言。

## 2026-10-09 原 Root 缓存来源持久化（复验中）

生产写入：新增 agent_root_model_cache_provenance 不可变追加表，保存原 call/Root/owner/round/request/response hash 及缓存明确报告标记；原 received JSON 三字段、旧费用条目、签名及普通预算不改变。Single/旧 Root 在原 received 结算事务追加；v3 Multi 在原 received invoice 独立捕获事务追加，后续费用 writer 失败不会丢掉已捕获来源，也不由历史反序列化默认零补造或覆盖。标记只有完整用量已报告、数值合法且明确缓存来源时才为 true。

当前没有 USD 结算消费者/恢复执行授权；新表仅保存原传输元数据，既不是供应商美元发票，也不代替独立模型、scope、批准、计费上界与债务校验。旧历史不回填，旧 Tick 历史响应仍默认未知缓存来源；未来新合同必须显式读取并审查这份来源，不能将表存在当作准入。

回归新增：实际 Single 缺缓存/明确零回包与原 received 全绑定，UPDATE/DELETE/REPLACE 拒绝；实际 SDK 后新表 writer fault 会回滚本次 received/消费、保留 dispatch 原预留，下一轮拒绝零新 SDK。现有实际 Multi 费用 writer fault 确认测试追加来源保存断言。首编译测试用了需 Debug 的 unwrap，已改为测试内明确 panic；未为测试改生产失败类型，失败证据保留。

工作树外部变化：此前源码封存之外，Exposure 多个 Rust/前端/测试和 lib.rs 注册文件已变化；主控未写这些文件、未归因作者、未回滚。后续测试按当前快照记录，不能把较早库存和当前测试数拼为同一整库。证据 cache-provenance-persistence-*.log 及相应封存。

本片修正与当前证据：两个真实 SDK 新回归通过；旧 102 次调用测试的 30 秒总等待在并发编译时超时，保留该失败镜像，现逐次接收实际 SDK 到达序号（每次 5 秒），保留全部 102 次实际调用及原账/零新增请求断言，不提高生产预算、时钟或请求上限。当前 Single 12/12、严格 Clippy lib/tests 和应用 build 均通过、零警告，固定选定源码封存差异零。

共享接口清理：外部 Exposure 变化移除无版本旧复核入口，前端亦要求旧入口由主控退役；全库仅剩未使用的 ExposureFindingReviewInput 声明，使严格 Clippy 失败。主控只删除 models_appsec.rs 中这一孤立 DTO，未改 Exposure 实现/新带版本复核接口、数据或资产；没有加 allow/expect 压制警告。失败日志与修正后的检查均保留。此项是共享接口残留清理，不计为外部暴露面业务交付。

相同生产缓存写入实现的先前 3094 保存镜像：原历史 6/6、费用 10/10（含真实 Multi writer fault 仍保留来源）、财务续接 32/32；其 Single 11/12 因上述测试等待失败。修正后的 3094 镜像仅改变测试同步和删除无人引用 DTO，Single 12/12/Clippy/build 通过；不将两镜像的结果拼为当前整库绿色。定价批准组和两个原必需恢复需求终态另附。

通道边界：新增持久化只覆盖 Root/Single 与 Coordinator 的原 received 路径，未改 Source/specialist 的 agent_model_cost_facts；不能称全十五角色或全模型通道已具备美元来源证明。当前修正后 3094 镜像的两项原付费恢复正向需求复验仍失败于 web_start_paid_history_requires_explicit_financial_contract，原断言保持，不视为完成。

本片终态收齐：先前相同生产缓存来源写入的 3094 镜像定价批准组 7/7、财务续接 32/32、费用 10/10、原历史 6/6；当前修正镜像 Single 12/12、build/严格 Clippy 零警告、两项原恢复正向仍失败，最终选定源码差异零。没有当前整库绿色声明，没有安装或替换用户 App、提交/推送或真实库/CAS/资产操作。

## 2026-10-09 goal 已恢复：历史缓存来源读取与原捕获认证缺口

Goal 实时状态已恢复为 active，完整 REM-A01 至 REM-A14 目标保留，上一 goal 管理回合没有代码进展；本回合真实改动是历史读取链。SavedDecision.financial_response 现通过原 RootModelCall 读取缓存元数据；核对原 dispatch/received/owner、Root/轮次/请求/响应绑定，并精确检查主库表及三个不可变触发器定义，拒绝异来源和同名空操作触发器。缺表/缺原记录仍返回未知，不补写。原账单、原签名及成本证明序列化不变，无美元余额、SDK/目标权限或时钟增量。

当前生产读取改动固定 3098 镜像：费用 12/12（包含缺历史来源、异 Root/请求/响应、同名空操作触发器实际冷回归）、InputParser 真实原 Root Tick 1/1、监督容量组及原费用历史/Single 历史关联组通过；strict Clippy 与应用 build 零警告，封存选定源码差异零。外部 package.json、Exposure、lib.rs 等变化保持隔离，主控未写。新无签名位翻转正向需求回归另收，不能把这一镜像通过当完整 USD 原账来源证明。

新增认证验收：真实 SDK 缺报缓存 → 原持久化位为 false → 一次性冷 fixture 仅把位改为 true 并恢复原 DDL，要求不可因此恢复为供应商明确报告。元数据目前没有独立原捕获签名，单纯 schema/binding 不能证明该位在原 SDK 时的值。该新需求保留，不能删除/ignore 或改为接受伪造以换绿；下一核心为新独立域、原捕获时的认证封装，完整绑定原作用域/模型请求/收到回执/缓存报告位，拒绝从历史读取或任意 JSON 补签，旧记录按未认证处理。旧签名算法不重写或重新解释；后续 USD 结算必须消费独立核验的原账来源，不信任展示元数据。

Single 未知账单确认、定价恢复专属原子出生、共享 USD/十维/原时钟与其他 Master 项仍未完成。Goal 保持 active，不按本片收窄或标完成。

认证回归已收终态：当前 3099 库存，root_cost_cache_unsigned_bit_cannot_be_promoted_to_provider_reported 实际失败，0/1；只有测试文件较前一 3098 绿色局部镜像变化。日志 cache-provenance-unsealed-bit-required-final.log 保留，首次测试误把 Value 当字符串解析的编译失败亦保留并已修正。该失败实证明确：恢复原 DDL、保持同一原调用/响应绑定，仍能把缺缓存的 false 位改成 true；尚缺原捕获认证封装。新元数据读取不可被称为可授予美元结算的账单证明，现无 USD 结算/执行消费者。新失败需求保持，不 ignore/删除/改成接受；下一回合直接推进原 SDK 捕获时的独立签名协议与只验不签的历史读取，不再增加一般只读展示来消账。

本回合既有读取/费用证据：3098 镜像费用 12/12、InputParser 原 Root 1/1、监督容量 13/13、原费用历史 8/8、Single 历史 6/6，Clippy/build 零警告。3099 仅新增认证需求测试，生产代码相同；未跑当前整库，不称整库绿色。Goal 保持完整目标 active，无自动提交/真实资产数据操作。


### 原捕获私有模式核验入口抽取（2026-10-09，Goal active）

本轮生产改动仅抽取 `original_root_capture_mode`：复用现有 WebBindingDirectory 原文件验证、原模式回执以及 Root 出生签名校验；恢复批准的原模式读取复用该入口，并仍显式匹配原 scan/attempt。未导出私钥、未新增任意 JSON 签名入口、未授予恢复执行。`cargo check --tests --quiet` 当前通过。定向恢复批准冷修改回归已真实命中并通过 1/1（零忽略，10.52 秒），日志位于 `/tmp/oviraptor-master-capacity-concurrency-20261008/root-capture-proof-refactor-test-matched.log`；带裸函数名的 `--exact` 调用不能作为命中证据。缓存来源独立域签名尚未写入，原 unsigned-bit 必需失败回归仍保留，不宣称修复。下一步在首次 received 事务内以私有构造的捕获对象签名，历史路径仅验证、不补签。未改 Exposure 业务或真实数据，未提交。


## 2026-10-09 active goal：Root 缓存来源捕获签名已接入

首次 `received` 事务现原子保存独立 `oviraptor.root-model-cache-capture.v1` HMAC 回执，绑定原模式/Root/owner/轮次/请求/完整 received 与缓存来源位；历史仅验签，无签名返回来源未知，不从旧 JSON 补签。有原模式却私有文件或出生签名失效时拒绝；无 Web 模式的历史数值会计不冒充认证缓存。原冷改 0→1 必需失败回归已修复；当前费用组 14/14，严格 lib+tests Clippy 零警告，Single 缓存与签名写入故障组 2/2（故障同时覆盖元数据与签名写入），最终应用 build 通过；选定源码指纹零漂移。日志与源指纹：`/tmp/oviraptor-master-capacity-concurrency-20261008/cache-capture-signed-*`。这证明本地 SDK 捕获认证，不是第三方美元账单或恢复执行；共享 USD、专属恢复出生与 Master 其他未完成项继续保留。未提交、未改 Exposure 业务或真实数据。

新增坏签名、缺签名不可补签、三种 immutable 操作与签名写入故障回归；保留前一版测试夹具缺快照失败，以及扩大费用组暴露无 Web 模式旧夹具失败日志。修复未弱化原断言；费用组真实命中 14 项。新 seal 表增量建表，不 ALTER/重写历史数据。下一步继续原计划完整输入费用上界、原费用未知确认与专属恢复原子出生，不能以本片替代完整十维共享预算。


## 2026-10-09 active goal：完整请求体传输限制接入

生产网关首次发送前现对真实 request_body 的序列化字节数执行原 request_budget_bytes 限制，包含消息、模型参数和工具 schema；超限归类 BeforeTransport/ContextOverflow，不占 provider 队列、不发送 SDK。localhost 回归覆盖 UTF-8 大消息与工具 schema，零发送后正常请求仍可发送；当前模型 tests 过滤组 26/26、严格 lib+tests Clippy 零警告，应用 build 通过，选定源码指纹零漂移。上一片缓存签名当前历史链复验 Single 6/6、财务历史 8/8。完整日志在 `/tmp/oviraptor-master-capacity-concurrency-20261008/model-complete-payload-*`。此限制是传输大小，chars/token 估计不能作为认证 token/美元上界；云档案输出上限仍为 None，专属恢复出生与共享 USD 尚缺，Goal 保持 active。未提交或修改 Exposure/真实数据。

原请求预算函数此前仅测试调用，完整 body 含工具 schema 后未设发送门禁；现接入原限制，没有增加额度。最初新增测试误写不存在方法名导致编译失败，修正为现有 complete_once_observed，失败日志保留。下一步须给专属恢复冻结并实际发送输出 cap，绑定 tokenizer/消息模板及工具定义的输入上界；不能用现有 cloud 6MB 或 fallback 2048 伪造费用可执行性。


## 2026-10-09 active goal：本地请求预算极值算术修正

原 u64 配置直接转 i64 后减乘可能溢出；现使用 u64 饱和减乘，并限制完整请求体不超过既有 6MB 传输上限。正常 32768/2048 配置仍为 120832 字节，原 4096 token 字节估计下限保留；极值不构成真实上下文可容纳或费用上界证据。极值回归 1/1、当前模型 tests 组 27/27、严格 lib+tests Clippy 零警告、应用 build 通过，选定源码零漂移。日志 `payload-policy-extreme-*` 位于既有 `/tmp/oviraptor-master-capacity-concurrency-20261008/`。Goal active；下一项仍须专属恢复冻结/真实发送输出 cap 与认证输入上界，随后共享 USD 原子预留/结算，未宣称恢复执行完成。未提交，未改 Exposure 或真实数据。



## 2026-10-09 active goal：已配置输出上限的真实 wire 与发送前校验

生产网关拒绝 max_output_tokens=0、超过 i64 最大值、以及有本地窗口时 output>=context；归类 BeforeTransport/Protocol，零 provider 请求。localhost 对本地/云 profile 实际发送均验证 max_tokens=37 原样落入 JSON，请求计数保持 1，非法后续调用零发送。当前模型 tests 组 28/28、严格 lib+tests Clippy 零警告、应用 build 通过，选定源码零漂移。`/tmp/oviraptor-master-capacity-concurrency-20261008/model-output-limit-*` 保留最初新增测试调用错误方法名导致的编译失败与修正后证据。恢复合同仍没有用户冻结的输出 cap，普通云 resolve 仍为 None；本片证明已有配置的派发校验，不代表恢复 cap 绑定、token 输入上界、专属出生或共享 USD 已完成。未提交，未改 Exposure 或真实数据，Goal active。

下步需版本化恢复调用边界，将用户明确输出 cap、完整模型/模板/tokenizer 与工具输入边界冻结进批准签名，再由专属出生和 SDK 原请求读取。不能从没有 max_tokens 的云普通档案推断默认 2048；provider 超量账单必须保留原用量并形成债，不裁剪为批准上限。


### 全量主库回归进行中（2026-10-09）

本轮核对有效输出 cap 的角色差异，详见恢复共享财务设计最后段。启动当前完整 `cargo test --lib`（3103 项），活动 session 19464 已确认 live，原日志 master-full-lib-20261009-output-limits.log 与 1435 文件 source-hashes 已保留。尚无终态，不将上一轮模型 28/28 拼成全库绿色。源码未再改动；Goal active。下一轮继续该句柄，终态后根据实际失败推进回归债与版本化恢复边界。


全量回归观察检查点：同一 exec session 19464 经再次轮询确认仍 live；当前日志 534 项 ok、零 FAILED 标记、没有 test result 终态。原 1435 Rust/SQL/Cargo 源指纹零漂移。此为 verified wait，不是整库通过或 Master 完成；继续同一句柄，不重复启动。


全量继续观察：session 19464 仍 live，日志已到 811 项 ok、1 个 FAILED：`commands::agent_tests::e2e_pause_resume_and_reexecute_keep_the_budget`。测试名属于既有恢复正向需求，但当前错误原因须等待终态 panic 详情，不直接沿用历史错误。无最终 test result，未宣称全库通过；原正向测试保留。下轮继续原句柄，并按终态实际错误修复。


### 全量发现的缓存认证续接启动回归（修复进行中）

同一 3103 项全量镜像新增多项 financial_continuation 失败，不可合并成原 Single 两项需求债。原二进制定向单独复现 discovery_closed 0/1，实际错误 `financial_continuation_discovery_startup_missing`；新 original_root_capture_mode 对 financial born Root 错传 None，选择普通 startup。现原 snapshot 内判断是否已有财务 birth，存在时先原 verify_financial_continuation_birth_on 再传 birth 给原历史私有模式验证；仍保留所有原列/原历史/原 startup HMAC/原 mode/Root 出生签名校验。

修复已触发当前新二进制编译；仍运行的全量主进程是修复前镜像，源指纹已出现本文件的一项变化，其后 subprocess 若按 current_exe 路径启动可能遇到新镜像。因此该长跑仅作诊断基线，不宣称当前源码全量门禁；本次不再在其运行期间继续重建镜像。定向修复证据单独保存 full-regression-discovery-closed-fixed.log，终态后修复组再验。原长跑 session 19464 不重启、不中断。


缓存认证续接启动修复定向已终态：discovery_closed 真实命中 1/1 通过（41.78s），full-regression-discovery-closed-fixed.log 保留；原失败 0/1 日志仍在。当前已编译固定镜像启动 financial_continuation 组，日志 full-regression-financial-continuation-fixed-group.log，未重编译或改测试。完整组、最终应用 build/严格 Clippy 仍待验，不能据 1/1 宣称整个续接修复完成。


## 2026-10-09 active goal：缓存认证对财务续接启动的回归已修复

全量诊断发现新 financial_continuation 回归，原镜像独立复现 discovery_closed 的 startup_missing；不能算入原两项 Single 恢复需求债。修复 original_root_capture_mode 在原 snapshot 中识别已存在的财务 birth，原出生/原来源账目核验后传递原专属 startup 合同；继续原 HMAC、模式与 Root 出生签名验证，未放宽普通 paid Resume。修复版财务续接完整组 32/32（113.72s，零忽略）、严格 lib+tests Clippy 零警告、应用 build 通过，1435 文件固定源码零漂移。日志 `full-regression-financial-continuation-fixed-group.log` 与 `cache-capture-continuation-fix-*` 位于既有临时证据目录。

3103 项全量旧主进程仍为诊断长跑（期间重编译，不能声明单镜像全量门禁），原失败日志保留；最终汇总待终态。该 32 项仅证明已有费用确认后的 Native 财务续接，不是保护恢复专属出生、完整共享 USD 或所有角色完成；原手动暂停 Single 正向需求仍保留。Goal active，未提交、未改真实数据/Exposure 业务。



同一修复镜像追加交叉组已终态：root_cost_ 14/14（22.83s），single_model_inflight_cache_ 2/2（8.58s）；与 financial_continuation 32/32 同为当前修复源，1435 文件指纹零漂移。坏签名/缺签名/冷改来源位、普通 Single SDK、元数据与签名写入故障仍严格通过。日志 cache-capture-continuation-fix-root-cost.log 和 cache-capture-continuation-fix-single-cache.log；不拼成全库绿色。下一步等待诊断全量终态分类其他原债，再推进版本化恢复模型边界与共享账目。


## 2026-10-09 active goal：当前 UI 全组与前端构建复验

当前现有 `npm run test:ui` 929/929、0 fail/skip（106.03s），`npm run build` 的 vue-tsc 与 Vite 均通过；276 个前端/测试/package 文件指纹零漂移。日志 `master-current-ui-20261009.log`、`master-current-frontend-build-20261009.log` 位于既有临时证据目录。这是组件/模拟 IPC 与构建证据，不是实际桌面实时日志、第三方、保护恢复专属出生或最终发布验收。未改前端业务源码，未碰 Exposure 实现或真实数据。

Rust 3103 项诊断长跑仍 live，当前日志 1193 ok、11 FAILED、无最终 test result；修复版财务续接组 32/32、费用 14/14、Single 缓存 2/2 的独立证据保留。不能把修复版组验、UI 通过与旧诊断基线拼成当前整库绿色。Goal active。



## 2026-10-09 active goal：缓存来源签名与删除审计重建兼容修复

全量诊断新增 Multi 原终态删除失败。原镜像独立复现付费保留门禁；给原测试增加只读审计 preflight（原删除/回读/幂等断言保留），取得底层 root_cache_provenance_schema_conflict，纠正先前“非本轮回归”的过早判断。私有审计 arena 现使用仓库可信缓存 schema，保留原列精确核验、不执行存档 SQL；签名表按原 journal call/root 精确捕获，与 metadata 永久保留。旧包没有两张增量表时不补签/不要求迁移为认证来源。原 Multi 正向当前 1/1（17.43s）、应用 build 与严格 lib+tests Clippy 通过；付费删除故障组正在固定镜像复验，日志 cache-audit-reconstruction-paid-deletion-group.log。未操作任何真实删除、asset、CAS 或 Exposure。

全量仍仅诊断基线；出现其他迟到账单失败，待独立定位。Goal active；本片不代表完整删除/旧包所有历史兼容或整体 Master 完成。



## 2026-10-09 active goal：付费删除故障组通过，迟到元数据断言精化

缓存审计重建修复的付费 Single 删除组 14/14（68.94s，零忽略）已通过，包含崩溃/应答丢失/缺 inode/冷改/外 scope 保护；原 Multi 正向 1/1 仍保留删除与回读要求。迟到 Root 账单失败独立复现表明账单/结算成功，但旧“仅 journal/entries 可变”断言漏计新增缓存 metadata。三个相关叶仅增加完整原 tuple（call/root/owner/round/request/response）匹配、确切行数与未知缓存来源 0 的要求，仍比较其余所有表，签名表不被允许改动，旧准入/重放/替代 Coordinator 拒绝与未知子债不变；属于有界财务夹具，不宣称第三方 SDK。

已复现的 closed Root 用例修正后 1/1（2.72s），严格 lib+tests Clippy 零警告；当前固定镜像 root_budget_ 全组正在运行，日志 late-invoice-metadata-root-budget-group.log。本轮生产逻辑未改；原失败日志保留，未删除/ignore 测试。全量旧诊断仍待终态，不称当前整库绿色。Goal active；真实数据、asset、CAS、Exposure 未动，未提交。



## 2026-10-09 active goal：Root 预算与 Single 退出原要求复验

Root 预算组已完整 30/30（50.71s，零忽略）。Single 退出组真实独立复现 2/4，分别为新增缓存元数据/签名不在旧物理不变断言、1 秒夹具初始化时已耗尽；新断言要求精确原 tuple、唯一 metadata/32 字节签名并调用生产只读验签，仍检查其他物理行/原退出 cutoff/原墙钟/旧权限拒绝/零重发。秒边界对齐必须在耗时建库之后、计划持久化插入 Root 之前；首次误放 runtime_open_run 前仍失败，实际插入点已核对纠正。无时间戳重写、无硬限额增加，原 >=1050ms 超限断言保留。

最终 Single 退出 4/4（4.10s）、严格 lib+tests Clippy 零警告；日志 single-finally-exit-cache-and-origin-final.log / final-clippy.log，保留两次失败日志与源码指纹。新增 Root 方法仅 cfg(test) 只读包装，无生产门禁改动。全量仍诊断基线，待终态逐条核对剩余失败；不拼成当前整库绿色。Goal active，未提交/清理真实数据/改 Exposure。

