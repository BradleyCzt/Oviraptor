# Master Recovery 专属出生与共享财务实施设计

2026-10-08；设计规范，未接入生产执行。不能据本文件宣称恢复可执行、共享 USD 账本已经存在或 Master 完成。当前普通付费 resume/retry 禁令继续保留。

实施起点：src-tauri/src/recovery_finance.rs 已实现纯金额/费率原语，recovery-exact-money-tests.log 6/6；整数 nano-USD、严格十进制上限、字符串 wire、缓存输入子集、逐原调用合计向上取整、固定请求费、原 model hash/费率版本及未知协议拒绝。它不验证原批准/原物理用量，不创建账本/预约/费用/Root，也没有生产消费者；分类为财务框架原语与算法测试，不是恢复执行功能。新增公开库 API 不接受或签发执行权力。旧浮点批准/旧 pending 草稿/真实数据库未改变。

## 1. 当前源码事实

- native_protection_recovery_scope_budget.rs 签名保存 pending_dedicated_fresh_birth 合同；executionAllowed=false、targetRequestsGranted=0。
- native_protection_recovery_draft.rs 分离历史 immutable scope 与 pristine 首次出生检查，取消记录保留签名历史。
- native_web_mode_plan.rs、scan_startup.rs、scan_execution_frontend.rs 拒绝走普通恢复启动；不能只改状态或预算默认值绕过。
- NativeProtectionRecoveryScope 有十维 limits、maxBudgetUsd、deadlineUtc、模型/身份/代理绑定；没有冻结的模型费率合同和真实 USD 消费者。
- 现有原 Root ledger 是每 Root 十维，不含 USD。其 input_tokens 包含 cached_input_tokens；费用计算必须对 uncached=input-cached、cached、output 分别计价，不能重复计缓存输入。
- Root/model、Web/model、specialist、source-round 均已有不同的原费用/物理票据路径；仅改 Root/model 不能构成全模型费用消费者。
- 本輪两项旧普通 Single 付费恢复正向仍被 web_start_paid_history_requires_explicit_financial_contract 拒绝。独立 financial continuation 当前仅 Multi 只读 advisory Root，不含工具、目标或旧队列，不替代它们。

## 2. 本规范的有界验收单位

首片只交付一个新批准 scope、一个 URL、一个原模型绑定的完整专属出生/实际模型/目标/结算/停止闭环，再扩大到两个 URL Root 共享同一限额。单 URL 通过不能称全 scan 共享完成；只读准备、签名草稿和余额接口不能称执行完成。

允许来源是用户独立确认的全新 scope，不沿用旧执行授权。保护响应历史仅证明为什么申请恢复，不赋予访问新目标的权限。保留每个物理 HTTP 和每轮模型独立费用及监督，禁止用合并旧事实减少成本。

## 3. 必须冻结的合同

新协议单独版本化；旧 v1 草稿按原字节验证，不读取时补字段、签名、授权、费率或时钟。新增含费率的协议需要新的明确用户确认。

新 birth 合同至少包含：

| 项目 | 冻结来源/语义 |
| --- | --- |
| scopeOwnerId | 原批准/request/draft 身份与合同 hash，目的域隔离 |
| approvalHash / scopeHash | 原批准与 scope 原字节，不能仅信任 UI 字段 |
| requestId / draftId / scanId | 唯一待出生关系；原 UUID 幂等 |
| urls / methods | 批准 URL 与方法，完整 canonical 列表；不扩大域名或路径 |
| modelBindingHash | 冻结模型、endpoint、deployment/adapter 及凭据身份绑定的摘要；不向公共 UI 暴露密钥 |
| pricingContract | 原模型绑定适用的费率版本、单位、缓存语义、每请求额外成本、支持的计费方式 |
| tenDimensionalLimits | 十个独立 Some(0)/Some(n)/null；null 不等于 0，0 不等于无限 |
| currencyLimit | 精确整数 USD 最小单位；不是另一个默认美元预算 |
| bornAt / deadlineUtc | 唯一 scan 起点；截止时间为原绝对截止与 bornAt+wall-time 上限中的更早者 |
| identityBinding / proxyBinding | 仍属于批准的 scope/project，仍有效；绑定缺失不能猜默认 |
| permittedConsumers | frontend 与所有模型、目标、lane 等消费者明确列举；未接入者拒绝 |

不能从当前 settings 重新取额度/模型/代理去执行旧批准。当前 provider 配置变化时须拒绝或重新批准，不能将同名模型等同于相同费用合同。

## 4. USD 精度与可证明边界

采用整数 nano-USD（1 USD = 10^9）账目；费率可用每百万 token 的 nano-USD 整数。中间乘法使用 checked i128，最终转回有界整数；收费向上取整，不能舍入释放未证明余额。限额转换不能向上扩大批准上限。

新 UI 应输入十进制字符串，由后端严格解析；JSON 浮点、展示四舍五入值、字符/token 估计都不能成为付款凭据。旧 maxBudgetUsd 浮点字段仍按旧签名读取，不能读取时转为执行授权。过细精度、溢出、不支持的计费模型均拒绝出生。

支持第一片的模型必须具有明确且冻结的可计费 adapter：实际 usage 结构、缓存是否输入子集、可证明的完整输入上界、原输出上界、每请求成本和 provider/model 匹配规则均明确。仅 request_bytes/4 的历史估计不能承诺硬美元上限；缺 usage 或不支持定价不能假定费用为 0。

按冻结本地费率计算的金额，应明确显示“按批准费率记账”；除非另有真实原 provider 美元账单，不称第三方实际账单已对账。不引用临时查询的公开价格为历史付款，不伪造当前供应商费率。没有可证明上界/适用计费方式时，新恢复执行保持拒绝。

## 5. 唯一首效应与原子出生

专属入口持有同一 IMMEDIATE 事务完成：

1. 加载并核对原审批链、私有签名、schema、scope/model/identity/proxy、project、未取消、原 deadline。
2. 原 request 已有 birth receipt 时只核验原绑定并返回原结果；不再新增 scan/attempt/Root/clock/额度。
3. 首次请求要求 pristine：draft、attempt_count=0、无 Root/branch/attempt/任务目录消费、无取消记录。
4. 在首效应前验证价格合同及全部支持维度；不支持 browser/write/upload 的批准不能假装可执行。
5. 创建 scope 财务/唯一 clock owner、原 attempt、born Root 与原 C1 及完整限额/能力/冻结计划，最后写不可变 birth receipt。
6. 验证 INSERT 实際生效及所有原行身份；任何 ABORT/IGNORE/越界触发器/坏签名均全回滚，不能留下可执行半成品。

文件副效应不能在上述验证前产生。工作目录等必要材料使用明确的 staging/publish 协议，记录可恢复原收据；不能仅用数据库事务承诺文件原子性。出现不确定发布，保持原持有关系并拒绝二次出生。

普通 confirm/resume/retry 入口仍拒绝这类草稿。只有明确携带已验证原 birth receipt 的专属内部路径可以进入 frontend/Root 出版；不能用 executionAllowed=true 字段充当私有批准证明。

## 6. 多 Root 共享财务

scope 是唯一总 owner，各 URL Root 只拥有其来源证明和局部支出，不获得一份额外的 scope 总额。局部 limits 可限制子任务，但每次准入还必须在同一事务核对共享 scope。

共享余额按 consumed、reserved、indeterminate 分列；同一物理请求/模型 call 只有一个原来源键。不能把 Root/child 对同一 grant 的 input/output class 持有简单相加成双重占用，也不能在下级结算前释放 scope 义务。

新 scope journal 与原 Root journal 的 reserve/receive/consume/release/forfeit 必须绑定同一物理原来源、owner、call/claim、attempt、fence 与原票据。任一侧写入失败不得只提交另一侧。每条路径的 authorizer 白名单必须显式扩展并实测，不能为接新表允许任意业务写入。

两个 URL Root 同时争抢最后可用额度时只能有一个获准；第二个不能复制余额或换 Root 绕过。新 attempt/Root/child retry 不能重置 scan 时钟或共享消费。所有已付来源删除前仍按原历史账本保护，不能靠删除隐藏费用。

## 7. 消费者接入顺序

| 接入点 | 必须具备的原验证与写入 |
| --- | --- |
| 专属 frontend | 首次网络前验证 birth/private scope；GET/重定向/代理逐物理准入；共享目标及 USD 适用维度 |
| Root/model | 原请求、完整输入/输出上界、共享 reserve、收到 usage 的精确结算、付款/terminal/publication |
| Web/model | 每轮原 dispatch/physical result 与共享 reserve，不能仅在退出时补总数 |
| specialist | Mapper/Identity/Client/Reviewer/Investigator 等每个真实 SDK 路径逐笔共享消费 |
| source-round | 非 Web 不得沿普通默认额度进入 scope；实现前明确拒绝 |
| Tool Broker HTTP | 每个物理重定向/身份对照请求原 claim、原保护、撤权/截止检查与共享目标消费 |
| Browser/write/upload | 当前拒绝维度保持 0；有完整消费者/退出/恢复证据后才允许新批准 |
| lane/batch | 不把现有 lane 占位称批次派发已实现；批次另有原 scope consumer |

每接一条路径都需要真实生产入口、原费用和跨 scope/过期/故障证据。只新增 scopeBudget JSON、writer helper 或 cfg(test) 原行不算已接入。

## 8. 不确定费用与晚到费用

已准入但没有可验证收到/未发送证明的动作保留 reserve/indeterminate；禁止自动 retry、补 Reserve、释放费用或新出生获得第二份预算。收到证明但缺 usage，已知请求次数可记账，token/USD 未知仍持有。

取消、撤权、截止后禁止新发送，但原迟到费用仍能按原授权来源结算；结算不依赖恢复执行权限，不能把一次 late receipt 当新 grant。

超出原持有/上限的真实账单不得截断为估计、不提高上限。保存原账单与明确 overrun/debt，并冻结新工作；结算协议必须明确定义已知超额与未知占用，不能删实际费用换绿。

用户确认对账只确认确切原金额，不隐式创建新 scope、不自动重启。将新 birth 作为 human-recovery 监督事实接入，也不能跳过独立付费 Root 原来源验证。

## 9. 必需验收矩阵

- 单 URL：真实原批准、专属出生、frontend/Root/Web/Reviewer 或无候选退出、实际 SDK/HTTP、原价格/usage/目标/时钟费用逐账对齐。
- 双 URL：同 scope 下独立 Root 共享余额，第二 Root 不能再获得全额；最后额度的真实并发消费者只准入一个。
- 幂等：丢 commit reply、SDK received 前后 SIGKILL、进程重启、原 UUID 重入，均读原物理收据，不二次 SDK/HTTP/出生。
- 隔离：其他 scan/project/request/approval/model/provider/fence/identity/proxy、撤权、取消、原 deadline 及零维度全拒绝，失败前无 target/SDK/业务写。
- 事务：每一个出生/财务/clock/receipt INSERT 的 ABORT/IGNORE、side effect 和坏 hash/签名，不能产生半成品。
- 财务：缓存输入/非缓存输入/输出、缺 usage、不一致 usage、费用超过原持有、未知回调、晚到真实费用、并行占用、浮点/精度/乘法溢出。
- 历史：旧 v1 pending 草稿原字节不改变，旧 Root/费用/保护来源不回填；普通恢复门禁仍拒绝。
- 安装与第三方：本地 scripted localhost 通过单列；真实模型/美元账单和安装 App 验收另列，缺凭据/平台不伪称通过。

## 10. 实施顺序与接续边界

先冻结 pricing/model/独立维度的新批准协议并实现精确金额与可证明上界；随后单 URL 原子 birth + 一个真实 SDK/目标闭环；之后所有模型/target consumer；最后双 URL 共享/真实争抢/取消及 SIGKILL。不能先开放普通恢复，再补财务。

此项为主控 Master 范围，Qoder/Grok 的 Exposure 白名单不得修改这些控制/财务/Root/共享 schema/IPC。任何需要共享资源的暴露面接口只能消费已公开的只读合同；不存在的执行能力不能由其自行补齐。

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

## 2026-10-09 恢复来源与出生依赖的具体分离

| 来源 | 当前原证明 | 当前能批准/执行的范围 | 下一项缺口 |
| --- | --- | --- | --- |
| Multi 原保护停止 | 原 protection receipt、Coordinator 原授权、所有已付 provider 账目、原 SDK/目标退出 | v1/v2 只批准；均无目标执行 grant | v2 独立 draft/Mode 私有出生、共享财务真实消费者 |
| Single 用户手动暂停 | 原 Single financial exit、原 SDK/HTTP idle 证明、single projection 的原暂停发布证明 | 原费用/退出及暂停历史已验证；普通 Resume 拒绝 | 新的暂停来源目的域和明确新 scope 批准，不能沿用保护 receipt 来源类型 |
| 普通 Retry/已付历史 | 原 attempt/Root/模式、模型与目标物理收据 | 不能由新默认限额重复续权或销账 | 明确重执行授权与新的财务 scope；原历史永久保留 |
| 未知/冲突用量、超限账单 | 原 received/unknown/forfeit 与已知请求收费 | 仅盘点/对账/停止，不允许以新 scope 遮蔽旧债 | 确切原账单证明和显式结清；不能用估算或用户“继续”确认未知价格 |

Single 手动暂停的来源验证必须同时覆盖：原执行模式 Single、原 Native Root budget owner、原 Single financial exit reference、原 single projection proof 的 paused=true、原 snapshot/source/event/collaboration 行、原 SDK/HTTP 进程退出及 inode、原十维未决余额。仅 sentinel_scans.status='paused'、空 finished_at 或零进程数量均不充分。历史读取不得 close_single_finance 或 close projection 补造退出收据。

现有 single_projection.rs 的 Proof 为私有发布证明，不能从 SQL JSON 反序列化后视作新批准或执行权力。下一实现应增加限定只读的 typed 原暂停来源读取，并在专属新批准事务再次校验；不要公开私有签名密钥、放宽 RootOwner::require_live，或把公开暂停事实当作新 Mode 签发依据。旧普通 Single 两项测试应保留失败，待新的明确恢复入口可真实运行后再补完整正向；保护恢复单 URL 的通过不能替代它们。

v2 原批准 modelBinding 中的 maxContextTokens / maxOutputTokens 为配置冻结值。当前云端分别可能为 0 / null，且常规 Root transport 会显式采用默认 output=8192；后续 birth 必须冻结实际发送的有效输出上限与请求/工具模式，不能批准 null 后由实现读取新默认值而默默改变价格适用合同。配置的非零 context 也不等于已验证 tokenizer / provider 对完整输入的计费上界。

实施顺序保持先一个来源/一个 URL 真正执行，再扩大来源和多 URL；本节是额外缺口及安全依赖，不新增执行能力，不将原目标压缩为保护恢复。

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


## 2026-10-09 批准历史找回与测试债最新终态

真实功能：新增 `find_native_priced_protection_recovery_approval`，按 scan/attempt/Root/protection 四个原来源字段精确查询，并在同一只读快照通过原保存读取器核验 canonical bytes/各列/私有签名。历史读取不要求当前模型配置或新 scope 仍有效，撤销后原批准仍保留。实际 SFC 的显式“按原保护来源找回”从原回执恢复原 requestId、范围和费率，另读原撤销；不创建替代 UUID、不重新批准、不调用执行入口。缓存出现不同记录、篡改回执、来源变化、跨 scan 迟到或保存失败时保留原状态。页面批准前也检查已有缓存，避免可观察到的另一窗口记录被覆盖；localStorage 检查不提供跨窗口原子锁，后台独立来源唯一约束继续负责阻止重复批准。

测试证据：源查询扩展了实际付费来源/四字段串线/冷改价保留旧签名/撤销历史断言，后端 priced_recovery 6/6、99.61 秒；实际 SFC/合同/旧 SFC 组合 26/26；完整前端 925/925、零跳过。严格库与测试 Clippy 零警告；前端类型检查和 build 通过。SDK 为 scripted localhost，不是第三方模型或安装 App 端到端验收。

测试债：46 项仅整理测试借用、类型、参数封装、测试模块位置与错误提取，原断言/预算/批准守卫保留，没有新增 allow/ignore。相关 33 项实际运行回归全部通过，退休 allowlist 只更新已审查 E2E 测试文件 SHA，对应原扫描测试通过。源码清单 3033 项的整库保存二进制复验另行运行，它早于本次来源找回改动；不同快照不可相加为整库绿色。

下一核心保持第 2–8 节专属出生与真实消费者；上述属于批准管理和只读历史找回，不是恢复执行。普通手动暂停 Single 的两个既有正向需求仍保留失败，不能改为守卫拒绝负向来消账，也不能由本保护来源批准替代。恢复出生尚缺 scope 私有权力、十维独立映射与共享总 owner、原 scan 时钟、USD 原预约及结算、模型/代理/身份/方法/UTC 期限消费者；未知模型输入计费上界不能由字节估算或 maxContextTokens=0 填补。


## 2026-10-09 原始账单结构与执行入口复核

真实生产修正：OpenAI compatible gateway 在普通 JSON 对象解析之外检查原响应 usage 结构，拒绝由同名字段覆盖掩盖的账单歧义；覆盖顶层重复 usage、嵌套重复 cached_tokens 与转义同名键。它保留 ModelResponse 的诊断数字，但 usageReported=false；实际 Root/Single 原接收路径仍记一次物理模型请求、保留未决 token 义务并停止后续工具/目标/模型，而不改为已知账单或 SDK 重试。只检查 usage 及其子树，非费用 metadata 不受此约束。原始响应文本仍不新增持久化；这是账单可信性门禁，不能称供应商美元账单已经导入。

证据：Single 原付费测试扩展至 5 场景；监督 Root 扩展至 3 场景，逐次实际 localhost SDK、旧 HTTP 一次、新目标零次、出版不增加、重入原行不变且零新 SDK。固定当前测试二进制库存 3054：wire 1/1、gateway 21/21、fresh_single 14/14、supervision_capacity 13/13、root_cost_confirmation 4/4、priced_recovery 6/6。严格 lib/tests Clippy 与应用 build 零警告。20 项新 Exposure 测试属于外部业务快照变化，不是本片交付；未运行其显式 ignored 公网探测，不改其业务。旧 3033 整库快照正在单独收终态。

下一执行合同的源码阻碍已复核：root_definition.rs 的普通 schema2 仍从 Native plan 推导同一 token 三类、请求及 target 默认映射；limits/contract.rs、root/shared.rs 校验这一原映射。continuation/migration.rs 的 schema3 只支持 Multi 原剩余额度的只读 advisory Root，不是新保护恢复或普通手动暂停 Single。native_web_resume_finance.rs 在原费用、未决义务与原 publication 验证后，仍拒绝普通已付新 attempt。pricing.quote/quote_upper_bound 当前没有生产付款/预约消费者；生产引用只有新批准中的金额与价格合同冻结。这些拒绝必须保留，不能靠放宽 schema/改默认 token/重置时钟取得表面成功。核对输出为 recovery-finance-production-consumer-audit.log。


## 2026-10-09 完整库快照终态及实际日志修正

3033 项保存二进制全组终态：3027 passed、2 failed、4 ignored、2740.63 秒。四项为快照已有的 Github 公网 opt-in、归属 2 万/10 万规模及提取真实项目副本验收，主控未新增或调整其 ignore。两个普通 Single 付费恢复正向仍因 `web_start_paid_history_requires_explicit_financial_contract` 失败。较早 2957 快照的 2931/23/3 保留；当前 3057 库存含后续代码和外部 Exposure 20 项，不能把不同快照局部通过相加为当前全库绿色。

真实日志修正：native-agent-usage.jsonl 以前只保存数字，未标注 usageReported；并发 8 线程/32 个诊断 append 实际复现 JSON 帧写串（基线失败，key must be a string）。现在先编码完整 JSON 帧、追加换行，再持有该文件 OS advisory lock 完成 append，并显示 usageReported 与 provider_reported/unverified。锁竞争最多等待约 100ms，之后不写未锁定的半帧；这个次级文件依旧 best-effort，可能因错误/竞争缺行，不是预算权力、完整收费账本或全部实时日志验收。原费用与原 native_sdk_log_rows 才能另行核验。本片不声称真实多 worker SDK 同时运行；并发 append fixture 只取一次真实 localhost gateway 响应并写 32 条诊断，实际模型请求仍 1。

证据：原写串基线 usage-log-frame-before-fix.log 保留；修复后并发完整帧与真实 unknown/known 两行标记 2/2、持锁不无限阻塞及不重发 1/1；当前 gateway 24/24、retirement 50/50、specialist_transport 10/10、source_tool 15/15、监督容量 13/13。严格 Clippy lib/tests 和应用 build 零警告；Web USD 参考文案原面板 9/9、完整 UI 925/925、零跳过、类型检查/build 通过。日志测试文件的退役 allowlist 只更新该已审查文件 SHA；唯一旧词仍是既有“没有旧进程”的注释，不引入旧 backend。

只读复核：native SDK cost_saved 不是无条件自报；ModelLog 与 persist 会回读原 owner 的物理费用 phase，前端显示“原费用记录已保存”，不宣称已结清。没有据 caller 的无条件 cost_saved 方法调用误改其生产消费者。新增用量标记及附属文件框架不替代专属出生、共享十维/USD/clock 或来源合法性验证。


## 2026-10-09 固定请求费最低准入与实际并发边界

真实新提案门禁：`freeze_for_new_proposal` 在预览及私有批准验证中，以原模型绑定的 RateCard 精确计算零 token 时单次固定费这个下界；超过原精确 USD limit 就拒绝，未创建批准/Root/账本或新 SDK。零 token 只用于最低固定费可行性，不是完整输入预约，不证明请求可以执行；恰好等于 limit 仍需将来消费者证明完整输入/输出费用。保存的历史批准继续通过原 `freeze` 读取，不因新增当前准入规则失去原历史。actual-paid priced 6/6 包含超过 1 nano-USD 的 preview/approve 拒绝及原表/SDK 不变；实际组件解释错误并允许修改费率后重新核验，组合 27/27；完整 UI 926/926，零跳过。第一条不带完整限定名的 `--exact` 命令实际选中 0 项，保留日志但不计成功证据；随后限定过滤的实际测试才构成证据。

实际 SDK 并发：同一应用测试数据库内，两个由真实 creator/startup/Mode/原 Root 建立的独立 Single scan 同时进入 localhost SDK；两个原 dispatch 已存在、received 均未产生，再由生产暂停入口停止第一个。第二个仍保持在途，释放其响应后收到真实用量并记录一次模型费用；第一个保留未知 token 义务。每个 provider 实际只收到一请求、无自动 retry、零目标 HTTP。这是两个独立扫描的传输及暂停隔离证据，不是同一恢复 scope 多 worker 调度、跨 Root 共享 USD 或第三方模型验收。首次断言误用了 input_tokens 这个非配置名，已纠正为原十维中的 model_input_tokens，保留该测试基线。

真实余额读取修正：这个误名同时揭示 balance 会对不认识的维度返回零，容易成为错误的“无费用”观察。新增实际已付 Root 下的空名/input_tokens/usd/尾随空格拒绝断言，先保存返回全零的失败基线，再让生产 balance 只接受 DIMENSIONS 中的十个原维度，未知名明确 budget_dimension_unknown，零写入。余额函数本身仍不是 owner、scope 或快照验证器；合法但无行的余额不能单独用于获得授权。当前 Single in-flight 7/7、Root budget 30/30、监督容量 13/13，其他关联组终态另收。

下一步仍为专属恢复出生及实际共享十维/USD/clock。普通手动暂停 Single 的来源不能用 Multi 保护签名替代；它需原 SingleExit/原 SDK 和 HTTP 退出/原 projection 与显式剩余合同。当前两个原正向仍保留失败，不把新诊断或传输并发证据当恢复实现。


## 2026-10-09 Single 原历史闭合审计与预算只读投影

- 2026-10-09 Single 原历史协议修正与只读入口：原读取器误用 Multi Tick，真实 Single 已付回归曾报 root_tick_requires_multi_coordinator；现按原 Single owner、SDK/HTTP idle、SingleExit、每轮原 dispatch/终态与四维原费用逐项核验，不补账、不更新时钟、不授予执行。实际多轮模型+一次 HTTP、分配篡改/未报告/额外字段拒绝、原截止已过但闭合历史可读 3/3；预算 IPC 已接入只读结果，Source 不冒充 Single，界面严格拒绝执行 grant/异 root/美元账单声明，实际 SFC 13/13，前端 build 通过。3060 固定快照关联 Single in-flight 7/7、fresh Single 14/14、财务续接 32/32、旧保护协议 19/19、定价保护 6/6、Root budget 30/30、退役 50/50；两项普通付费 Single 恢复需求仍因显式财务合同缺失失败。当前新增截止回归后的库存 3061，不拼成整库绿色。恢复专属出生与共享 USD 执行仍未完成；额度最近确认为 5%，未达 2% 停止线。

生产消费者变化：continuation/proofs.rs 的 Single 分支调用 root/model/single_history.rs，Multi 分支仍读取原 Tick。Single 不可夹带 Multi Tick；已接收回执必须 canonical、已报告、恰好五类原用量，按 input/cache/output/request 重建原 reserve/consume/release，与源 call 的原追加账目完全一致。未实际退出、未知费用或原账冲突均拒绝；过去的真实截止不影响已经闭合历史的读取。private remaining=ZERO 仅用于构造只读原调用核验，不进行准入、模型派发或时钟续期。

IPC 在现有 attempt snapshot 中读取，投影仅 schemaVersion/root/state/不可执行 flags/basis；原审计错误和 SQLite 路径不进入界面，失败只显示 not_verified，不能据此断言损坏或批准恢复。实际界面明确不是供应商美元账单，不增加同意/重试按钮。相关测试数据库为一次性 fixture；损坏回执测试暂移除并原样恢复该 fixture immutable trigger，以验证原退出前已坏的记录，生产触发器保持。

这里仍缺普通 Single 显式确认及定价恢复专属出生/共享 USD/时钟消费者，不通过放宽普通新 attempt 来替代。


## 2026-10-09 原退出指纹与实际 provider 准入证据

- 2026-10-09 后续历史/并发证据：完整 UI 929/929、零跳过；应用 build、严格 lib+tests Clippy 零警告（随后退出指纹小片另验）。真实网关排队取消保留前一付费轮/零发送首轮，Single 历史组合 5/5；同一数据库四扫描共用一个云端 provider 时实测前三个同时发送、第四个排队后暂停零发送，且不影响前三个；同 Root 在途时同请求重放、改变请求和下一轮都拒绝新增原账目或 SDK。新实际失败基线暴露退出物理行漏绑：Single 财务指纹现追加原 SingleExit 物理行，Multi 原指纹算法保持；Multi 混入 Single 退出回执现拒绝，异 scope 原记录不改变本 Root 原指纹，实际 Multi 1/1。当前保存二进制 3066 项，财务续接/定价/在途/历史/预算诊断组合复验中；仍不实现恢复专属出生或共享 USD 执行，两项普通恢复正向需求当前复验仍失败。最近额度 4%，未达停止线。

指纹真实修正：Single 原历史新增时，physical_hashes 沿用旧 Multi 表列表，遗漏原 SingleExit。一次性真实 SDK fixture 移动原退出行 rowid 后，原指纹实际未改变；失败日志 single-history-exit-binding-before-fix.log 保留。现仅已验证的 Single 分支追加原退出物理行类型/列/值摘要；Multi 分支不追加空表摘要，不改变旧签名算法。原 SingleExit immutable trigger 在生产未改；cold fixture 读取并原样恢复其原 DDL。

协议混用真实基线：原 Multi 付费已闭合 source 曾无视插入的同 Root SingleExit。新 verify_calls 在已核 actor 的 Multi 分支拒绝该原范围混用；其它 Root 的 SingleExit 不改变原 source fact，不引入全库任意历史依赖。这不是迁移/补签/修复，也没有新预算。组合以保存的当前 3066 测试镜像串行复验，不能将较早不同镜像通过拼成整库绿色。

并发证据限定：前三个独立扫描实际同时进入同一 localhost provider；第四个仅到真实 admission 队列，生产暂停使其获得 BeforeTransport/unsent 原终态，零物理发送、零消耗、无未知费用；前三个各一次请求，已知原账独立结清。另有同 Root 原 SDK 仍在途时的三类重入原行完全不变与零新发送。它们不证明同 scope 多角色并行、跨 Root 共享 USD、专属恢复执行或第三方服务付费成功。

## 2026-10-09 原账核验边界补充

原 HTTP 追加费用现在逐行绑定原 source 幂等键；错误网络失败码不再被接受为 Single unsent；未实现动作的平衡 reserve/release 对无法证明物理执行。Single/Multi 均拒绝这些伪闭合历史。最新库库存 3069，不表示全部通过。Single 未知费用仍缺人工确认入口，不能用 Multi received/Tick 确认入口冒充覆盖；原 SingleExit 费用表未参与 sources 哈希，必须新增专用确认事实读取，而非改写退出证明。详见 NEST_MASTER_SINGLE_HISTORY_VERIFICATION_2026-10-09.md 的下一执行闭环门槛。

## 原工具账据与未知费用队列补充

原 HTTP 历史必须同时保留匹配 run/invocation/tool 的 allow 记录；只有 HTTP 状态及精确费用键还不足。原费用队列现在显示 uncertain 为 blocked：已可核对账单、未知项、已确认项依次排序，按原 call 去重候选而不合并回执，仍有 101 条截断探测。它不接受未知账单确认，不提供退款或新执行授权。实际 SDK 暂停/读取/确认拒绝回归、原确认及对账 10/10、Single 在途 9/9 已验证。真实未知费用确认仍需要有来源的原供应商账单、独立不可变确认事实、原子绑定及债务处理；不允许以人工猜测价格或估算 token 结清原费用。

## 2026-10-09 恢复续接：缓存来源标记的生产边界

OpenAI-compatible 实际响应和逐请求日志现保留 cachedUsageReported。已报告一致的 input/output/total 且明确存在一致缓存子集字段时才为 true，明确零亦为 true；缺缓存字段、冲突/无效用量或重复 wire 键为 false。旧 usageReported 保留 token 协议兼容，不能把它单独当成不同缓存价格下的确切 USD 来源。旧不可变 Tick/原 invoice 未保存该来源，历史反序列化默认 false，不补写或改旧哈希/签名。当前日志标记仍不是经过独立原 scope、请求、原模型绑定审查的美元账单；新专属结算消费者必须读取其自己的完整原出处，不使用展示日志授予费用权限。真实 localhost 四场景及模型 27/27、严格 Clippy/build 已通过。

## 2026-10-09 后续片：已报告用量的定价前置门禁

精确定价原语新增 quote_reported_response，显式消费实际网关的 usageReported/cachedUsageReported。未报告完整用量拒绝；缓存费率不同且非零输入、未明确报告缓存时拒绝给出确定价格。费率相同或已核数值为零输入时金额不依赖缓存分配，允许算价而不改变来源标记；原 numeric quote 继续负责模型绑定、负值、子集、总量、单次请求和溢出校验。

真实传输测试：四次 localhost SDK 返回缺缓存/明确零/明确非零/重复字段，实际响应直接传入前置算价门禁，分别拒绝或得到精确 nano-USD，不增加请求或写预算。原语测试另覆盖昂贵缓存、未知总用量、相同费率、零输入、错模型绑定与错误总量。

分类：这是带真实网关测试的定价前置原语；目前仍没有恢复结算生产消费者，不授予 Root、SDK、目标请求或 USD 余额。传输来源标记不是经过原 scope/模型/原调用和批准证明核验的不可变供应商账单。Single 未知账单导入、不可变确认、专属恢复出生及共享预算执行仍未完成。未修改两个原恢复需求、普通启动拒绝、原不可变账目或签名。

本片只改 recovery_finance.rs、recovery_finance/tests.rs、model/tests.rs 及该已审查测试的退役 SHA；暂停后源码封存对比未发现其他选定源码变化。证据 reported-price-gate-*.log 与 reported-price-gate-source-seal.json，验收终态另附。

本片终态：当前源码算价 9/9、网关/模型 27/27、退役核验 1/1，零忽略；严格 Clippy lib/tests 与应用 build 通过、零警告，最终选定源码封存差异零。库存 3074，没有运行当前整库。前片两项原付费恢复需求仍未实现，本片不改变普通启动或它们的断言。

本片修正与当前证据：两个真实 SDK 新回归通过；旧 102 次调用测试的 30 秒总等待在并发编译时超时，保留该失败镜像，现逐次接收实际 SDK 到达序号（每次 5 秒），保留全部 102 次实际调用及原账/零新增请求断言，不提高生产预算、时钟或请求上限。当前 Single 12/12、严格 Clippy lib/tests 和应用 build 均通过、零警告，固定选定源码封存差异零。

共享接口清理：外部 Exposure 变化移除无版本旧复核入口，前端亦要求旧入口由主控退役；全库仅剩未使用的 ExposureFindingReviewInput 声明，使严格 Clippy 失败。主控只删除 models_appsec.rs 中这一孤立 DTO，未改 Exposure 实现/新带版本复核接口、数据或资产；没有加 allow/expect 压制警告。失败日志与修正后的检查均保留。此项是共享接口残留清理，不计为外部暴露面业务交付。

相同生产缓存写入实现的先前 3094 保存镜像：原历史 6/6、费用 10/10（含真实 Multi writer fault 仍保留来源）、财务续接 32/32；其 Single 11/12 因上述测试等待失败。修正后的 3094 镜像仅改变测试同步和删除无人引用 DTO，Single 12/12/Clippy/build 通过；不将两镜像的结果拼为当前整库绿色。定价批准组和两个原必需恢复需求终态另附。

本片终态收齐：先前相同生产缓存来源写入的 3094 镜像定价批准组 7/7、财务续接 32/32、费用 10/10、原历史 6/6；当前修正镜像 Single 12/12、build/严格 Clippy 零警告、两项原恢复正向仍失败，最终选定源码差异零。没有当前整库绿色声明，没有安装或替换用户 App、提交/推送或真实库/CAS/资产操作。

## 2026-10-09 goal 已恢复：历史缓存来源读取与原捕获认证缺口

Goal 实时状态已恢复为 active，完整 REM-A01 至 REM-A14 目标保留，上一 goal 管理回合没有代码进展；本回合真实改动是历史读取链。SavedDecision.financial_response 现通过原 RootModelCall 读取缓存元数据；核对原 dispatch/received/owner、Root/轮次/请求/响应绑定，并精确检查主库表及三个不可变触发器定义，拒绝异来源和同名空操作触发器。缺表/缺原记录仍返回未知，不补写。原账单、原签名及成本证明序列化不变，无美元余额、SDK/目标权限或时钟增量。

当前生产读取改动固定 3098 镜像：费用 12/12（包含缺历史来源、异 Root/请求/响应、同名空操作触发器实际冷回归）、InputParser 真实原 Root Tick 1/1、监督容量组及原费用历史/Single 历史关联组通过；strict Clippy 与应用 build 零警告，封存选定源码差异零。外部 package.json、Exposure、lib.rs 等变化保持隔离，主控未写。新无签名位翻转正向需求回归另收，不能把这一镜像通过当完整 USD 原账来源证明。

新增认证验收：真实 SDK 缺报缓存 → 原持久化位为 false → 一次性冷 fixture 仅把位改为 true 并恢复原 DDL，要求不可因此恢复为供应商明确报告。元数据目前没有独立原捕获签名，单纯 schema/binding 不能证明该位在原 SDK 时的值。该新需求保留，不能删除/ignore 或改为接受伪造以换绿；下一核心为新独立域、原捕获时的认证封装，完整绑定原作用域/模型请求/收到回执/缓存报告位，拒绝从历史读取或任意 JSON 补签，旧记录按未认证处理。旧签名算法不重写或重新解释；后续 USD 结算必须消费独立核验的原账来源，不信任展示元数据。

Single 未知账单确认、定价恢复专属原子出生、共享 USD/十维/原时钟与其他 Master 项仍未完成。Goal 保持 active，不按本片收窄或标完成。

认证回归已收终态：当前 3099 库存，root_cost_cache_unsigned_bit_cannot_be_promoted_to_provider_reported 实际失败，0/1；只有测试文件较前一 3098 绿色局部镜像变化。日志 cache-provenance-unsealed-bit-required-final.log 保留，首次测试误把 Value 当字符串解析的编译失败亦保留并已修正。该失败实证明确：恢复原 DDL、保持同一原调用/响应绑定，仍能把缺缓存的 false 位改成 true；尚缺原捕获认证封装。新元数据读取不可被称为可授予美元结算的账单证明，现无 USD 结算/执行消费者。新失败需求保持，不 ignore/删除/改成接受；下一回合直接推进原 SDK 捕获时的独立签名协议与只验不签的历史读取，不再增加一般只读展示来消账。

本回合既有读取/费用证据：3098 镜像费用 12/12、InputParser 原 Root 1/1、监督容量 13/13、原费用历史 8/8、Single 历史 6/6，Clippy/build 零警告。3099 仅新增认证需求测试，生产代码相同；未跑当前整库，不称整库绿色。Goal 保持完整目标 active，无自动提交/真实资产数据操作。


## 2026-10-09 有效输出派发差异与全量回归启动

当前源核对：`agent_native/root_model.rs` Single 使用 profile.max_output_tokens 或 8192，并将有效值写回 bounded profile 与请求哈希；`agent_native/coordinator_common.rs` 使用配置或 1024，封顶 2048；`multi_agent/child_transport.rs` 分支使用配置/512 封顶 512，以及固定 256、512 等。旧定价批准 modelBinding 的 cloud maxOutputTokens=null 不等于这些实际发送值，不能只给旧 null 补一个默认值后直接出生。

专属恢复新协议应把每个获准消费者的有效输出上限列入已批准合同，出生与每次 dispatch 对照原值；不同角色不可借用 Single 或 Coordinator 的 cap。模型、工具模板和 tokenizer 输入上界须一起绑定；存在未列举消费者、不支持完整输入上界或实际 cap 不匹配时，首效应前拒绝，不假定不收费。旧 v2 历史继续只读验证，不迁移成可执行合同。此段是实施约束，尚未实现新批准协议/专属出生。

当前 `cargo test --lib` 全量已启动，库存 3103，统一日志 `/tmp/oviraptor-master-capacity-concurrency-20261008/master-full-lib-20261009-output-limits.log`，活动 exec session 19464 已实测仍运行。源码 Rust/SQL 与 Cargo 指纹 1435 文件已保存；未有终态前不报整库通过。下一轮优先观察同一活动句柄，不重复启动或按超时判失败；终态后逐项分类原正向恢复债、新回归与并发夹具问题，不删除/ignore 原测试。


## 2026-10-09 active goal：缓存认证对财务续接启动的回归已修复

全量诊断发现新 financial_continuation 回归，原镜像独立复现 discovery_closed 的 startup_missing；不能算入原两项 Single 恢复需求债。修复 original_root_capture_mode 在原 snapshot 中识别已存在的财务 birth，原出生/原来源账目核验后传递原专属 startup 合同；继续原 HMAC、模式与 Root 出生签名验证，未放宽普通 paid Resume。修复版财务续接完整组 32/32（113.72s，零忽略）、严格 lib+tests Clippy 零警告、应用 build 通过，1435 文件固定源码零漂移。日志 `full-regression-financial-continuation-fixed-group.log` 与 `cache-capture-continuation-fix-*` 位于既有临时证据目录。

3103 项全量旧主进程仍为诊断长跑（期间重编译，不能声明单镜像全量门禁），原失败日志保留；最终汇总待终态。该 32 项仅证明已有费用确认后的 Native 财务续接，不是保护恢复专属出生、完整共享 USD 或所有角色完成；原手动暂停 Single 正向需求仍保留。Goal active，未提交、未改真实数据/Exposure 业务。

