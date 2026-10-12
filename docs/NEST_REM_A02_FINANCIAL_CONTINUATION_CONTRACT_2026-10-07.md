## 2026-10-07 U188 · A02真实桌面闭环与付费后崩溃验收

**本用户Goal的生产闭环已证明**：[逐项完成审计与A03交接](NEST_REM_A02_COMPLETION_AUDIT_2026-10-07.md)。真实独立debug Tauri App+Vite WebView，经真实Vue显式原费用确认→immutable receipt→原授权/十维/共享余量/原deadline预览→独立checkbox批准→实际新Root SDK→发布/原FinalClock收口→关闭面板及完整App重启后的显式只读找回。私有fixture从出生就在acceptance目录，未复制已付费DB、未启动正常用户数据迁移。原费用确认/预览后SDK仍1且未创建新Root；独立批准后SDK总2，原调用不重发，新账单31/3/7/38/1，共享余149952/18。原Root/费用/JSON保存，新attempt2为partial/assessment_only，不是整个Web完成；冷找回没有执行按钮或续授权。

**实际崩溃负向**：新cfg(test)点在真实received invoice独立COMMIT后、费用terminal事务前精确exit87。1/1、14.47s：账单真实31/3/7/1、保留reserve，terminal/decision/publication/exit均0；重复批准与worker重入不再SDK，真实停为review，未伪造收口。U186实际exit86付费发布后、FinalClock前的补收口零SDK回归也重新通过。release不含注入行为；三共享fixture仅抽helper，默认tests语义不变。

**门禁**：当前财务集中32/32、0忽略、146.35s（31场景+1默认未启用的desktop helper，不能用该计数冒充桌面）；显式exact+双env桌面driver另实跑1/1、900.89s，provider按15分钟上限停止，最终SDK2。当前邻近40/40、vue-tsc exit0；Clippy18 warnings/0 error且消息集合与U187相同。Rust全树未重跑，历史2624过/1 legacy-v2失败仍作为历史红项，严格-Dwarnings与整树绿未宣称。U187 UI893/893/build是旧快照，本轮实际桌面/IPC与静态检查分别报告；完整安装/provider/发布验收未做。全树两处旧空白债保留，新增叶/局部格式空白通过。

**测量与失败记录**：额外Python/系统SQLite只读探针出现间歇CANTOPEN，未用可写/删WAL/复制/改权限绕过；后续same-path system sqlite3 -readonly采集原回执、新birth/费用/收口成功，生产Rust只读IPC及冷启动找回成功。测试bundle shell→不同Mach-O名字曾导致macOS识别失败，仅修私有Info匹配原二进制后完成冷启动，主库打包未动。审计保留错误与证据级别，不声称一次全绿。

**范围保护**：原像/scope/hash和日志在 `/tmp/oviraptor-a02-crash-20261007/`；入口U187的23项scope逐文件复核。只改A02测试钩子/夹具及父接线，Asset业务归Qoder，范围外变化单列；自建App/Vite已停、私有证据保留。六文档仅前插本段，U187旧正文完整保留，下面各“partial/next”是当时检查点而非覆盖本段。未reset/clean/批量覆盖/自动commit/push，HEAD未变；未动真实DB/CAS/asset，Native JSON保留，不恢复Strix。

**下一阶段**：本Goal可complete；更宽REM-A02/Master仍partial，多历史Root授权链、Mapper/目标工作、完整第三方/发布验收未完成。按Master序进入REM-A03，先核六触发实际接线/changed-fact去重，建议单片“新证据变化→原活父/C内Root监督SDK→原费用/监督回执”，同事实重复零SDK，跨scope/撤权/过期/超限/故障拒执行。未接A03 frame及BL/AZ/ES骨架不翻旗，不借已闭合财务评估恢复旧模型语义。

## 2026-10-07 U187 · 已批准回执只读找回与合法剩额边界

**真实功能**：新增生产 IPC `discover_native_root_financial_continuation(scanId, attemptNumber)` 并完成 lib/commands 接线。按当前新 attempt 找回原费用确认回执、不可变 birth 中的原 proposal 与19字段严格状态；校验出生列/hash、原账单、双方签名模式、原模式物理证据和新 startup HMAC。SQLite READ_ONLY，读取不发行、不续授权、不启动 worker。原普通确认入口仍拒绝旧 attempt；历史读取复用完整原账单验证，不放松确认或执行守卫。历史 proposal 的 `issuanceImplemented` 原样保留，不改写出生证明。

**真实 UI / 只读诊断**：费用面板新增用户显式“读取已批准评估回执”。关闭/卸载后重开时可找回原绑定；读取不勾选独立批准、不自动执行，只有另行确认已有批准才能走原恢复守卫。重复找回要求原 proposal 全文、原账单和14项不可变状态一致；跨 scan/attempt、A→B→A、关闭/卸载迟到响应拒收。空结果不覆盖已有绑定；review和已闭合结果不启动工作。组件模拟 IPC 证明这些交互，尚不等于真实桌面 IPC 全链验收。

**localhost / 临时库证据**：找回集中 Rust **4/4**（初验27.28s；fixture加入合法额度参数后，最新二进制复验30.67s，均exit0），涵盖实际第二 SDK/闭合后 READ_ONLY 重开、实际 pause/confirm 未发行 null、错误/删除 scope 与12类冷损坏、多 birth 歧义、原 C 过期仅可读 review；全库行、changes和 SDK 数保持。另新增合法原始额度 **1/1**（5.54s、exit0）：cap在原 Root 出生时分别固定50010/50011，真实付费账单结清后共享剩额0/1，分别拒绝发行/请求准入不足回滚；重复点击仍只有原 SDK，原账单/JSON和 attempt 不变。该新测试首次因误用测试 helper 名称编译失败（E0425），改为已有 helper 后通过；未改生产守卫。

**前端与静态门禁**：定向 UI **52/52**，normal UI **893/893、0跳过**。类型检查发现 A02 回调丢失收窄，改用已收窄局部引用，修后专项 **15/15**；再次 `vue-tsc --noEmit` 仅剩范围外 `src/components/ExposureSurface.vue:26` 的 `loadBoundAssets` 未使用（TS6133），**全树类型门禁失败**，未代改 Asset。893为该局部类型修复前快照，修后没有重复全 UI。直接 Vite 隔离临时目录构建通过（3.45s），不能当作带类型检查的 `npm run build` 通过。lib Clippy **18 warnings、0 error**，消息集合与U186一致。Rust全树未重跑，历史legacy v2失败与两处全树空白债保留；本轮17个代码文件局部空白/新叶格式通过。

**范围与协作**：Asset Exposure 实现交给 Qoder，主控未改 Asset 业务文件。旧名资产代理曾复用于A02审计，用户指出混淆后已停用，不再以该代理继续资产或财务工作。入口313 dirty，17项本轮代码 scope、原像/哈希与范围外 Asset 漂移分别保存在 `/tmp/oviraptor-a02-discovery-20261007/`；六份文档只前插本检查点，U186及更早正文逐字保留。未 reset/clean/批量覆盖/自动 commit/push，未操作真实 DB/CAS/asset；Native JSON和旧调用不重放。

**未完成与下一片**：A02/U2仍partial、Goal active。下一片补隔离桌面真实生产 IPC/UI→批准→实际 SDK→状态找回证据；fixture必须从出生就位于显式 acceptance 私有临时目录，不能复制付费DB到新路径伪造原路径/签名/所有权证明。再补实际付费后、terminal/publication前的关键进程退出与重入，核验未决不自动重发，按原完整要求审计后决定完成。当前新工作仍为明确批准的原证据评估，不声称整条Web任务完成；Mapper/目标剩余工作和A03/BL/AZ/ES骨架不翻旗。

日志：`/tmp/oviraptor-a02-discovery-rust{,-final}.log`、`/tmp/oviraptor-a02-discovery-limits{,-final}.log`、`/tmp/oviraptor-a02-discovery-ui-{targeted,full,repair}.log`、`/tmp/oviraptor-a02-discovery-types{,-final}.log`、`/tmp/oviraptor-a02-discovery-build{.log,-path.txt}`、`/tmp/oviraptor-a02-discovery-clippy.log`。

## 2026-10-07 U186 · 独立批准 UI、真实状态、并发与崩溃复验

**真实功能**：生产 Vue 已接独立 checkbox + 用户点击 `approve_native_root_financial_continuation`，费用确认、刷新、预览和事件不批准、不自动重试。后端预览的 `issuanceImplemented` 现为 true，`executionAllowed/automaticResumeAllowed` 仍 false；读取预览本身没有发行权限。新只读 IPC `get_native_root_financial_continuation` 使用 READ_ONLY SQLite，原 scan/attempt/root/receipt/proposalHash 五项绑定及不可变 birth/迁移/原 owner/实际 publication/FinalClock 证明核验；19 字段严格响应区分 issued、assessment_published、review_required 和 attemptClosed。未决/复核或当前授权失效时不 spawn；实际评估发布不等于整个 Web 完成。

**真实 UI 接线修复**：原父级按 attempt 的 key 和瞬时 `state=undefined` 条件都会卸载批准面板。现仅将费用面板移到这些瞬时门之外，以 scanId 稳定挂载；子组件只保留用户已经明确批准、同 scan 且预期新 attempt 的绑定，其他 scan/attempt、关闭和卸载都拒收迟到结果。原概览、其他 key 与执行控制不变。严格 TS 回执绑定十维/共享余额、任务描述、原 deadline 与新 Root/approval；发布/收口证据不能回退，复核原因只接受后端四项枚举，原始错误/密钥不显示。

**localhost 与临时库测试证据**：集中 Rust 首次21/24、104.86s、exit101；两个错误码期望遗漏更早的原预算/未知费用守卫，真实 subprocess 崩溃用例最后读取原 exit 证明缺只读 snapshot。仅修精确期望/测试调用，不削弱任何生产守卫或零写/原记录/请求数断言；复验 **24/24、93.84s、exit0**。涵盖原真实费用故障→生产 pause→确认→独立新发行→新 SDK/发布/收口、并发批准/worker 单一出生与单一新付费请求、跨 scope/坏回执、策略/身份变更、原 C 到期、预算不匹配、合法未知费用、late writer 原子回滚、startup 签名/列绑定篡改、cold status 假完成、已发布但收口 writer 失败。

崩溃用例在独立进程沿实际 SDK 付费并持久发布后、FinalClock 前精确 exit86，不伪造 paused/scanning；父进程看到真实 publication 且缺 closure，重入实际补齐原 exit 证明，SDK 总数仍2（原请求1 + 新评估1）。注入点仅 `cfg(test)`，release 无该行为。

另增 **1/1、9.13s、exit0** 的新评估付费费用 writer 故障：真实第二请求账单保留、review不伪发布/收口、重复批准/worker不重发；用户另行确认新账单仅精确结费，仍不补模型语义或恢复执行。第一次1/1失败是 fixture误将输入31 tokens、低于预留的实际费用注入仅用于扩大预留的 `:actual` 行，未触发故障；改为实际 input consume 的 `:terminal` 写入点，生产未改。不能把两次失败套件声称一次全绿。

最新二进制邻近 **40/40**（费用9、模式20、新 Web 财务5、有限预算6）通过。真实父/子 SFC 与模拟 IPC 定向 **37/37**，最新 normal UI **878/878、0跳过**，`vue-tsc` 与隔离到新临时目录的 Vite build exit0。lib Clippy **18 warnings、0 error**，消息集合与 U185相同。没有重跑 Rust 全树或安装桌面 App；历史 legacy v2失败、全树2处空白债及既存 parent-format 债保留。UI IPC模拟、实际 Rust/local SDK、schema/签名/诊断和真实安装/provider各证据级别分别报告，不以套件数量替代桌面全链验收。

**完整 Goal 尚未通过**：A02/U2 仍 partial，Goal active。本片已完成独立批准按钮与实际后端 SDK/异常证据，但 UI 批准绑定当前仍限面板生命周期：关闭/卸载或重启后，必须补持久化出生回执的只读找回入口，不能从当前余额或状态文字生成新的批准。下一片先做当前 new attempt→原 immutable birth/原账单/原提案/严格状态的显式只读找回、late-scope与cold-tamper负向；任何读取都不自动启动。然后补实际 IPC/隔离桌面与剩余合法耗尽/超限和关键 crash 边界矩阵，再按原完整验收审计决定是否完成。多历史 Root 授权链仍拒绝而不是重发全额；Mapper/子代理/目标剩余工作未开放，A03与BL/AZ/ES骨架未翻旗，Native JSON与原调用不重放。

日志：`/tmp/oviraptor-a02-ui-rust{,-final}.log`、`/tmp/oviraptor-a02-ui-new-cost-fault{,-final}.log`、`/tmp/oviraptor-a02-ui-adjacent.{log,json}`、`/tmp/oviraptor-a02-ui-full-final.log`、`/tmp/oviraptor-a02-ui-types.log`、`/tmp/oviraptor-a02-ui-build{.log,-path.txt}`、`/tmp/oviraptor-a02-ui-clippy.log`；代理定向 UI 与逐文件原像/哈希在 `/tmp/oviraptor-a02-ui-20261007/`。

**并行边界**：入口已有 Asset contract/tests 两处外部漂移，最终另发现 ExposureSurface、资产 API、资产 composable/contracts/CSS 与专项测试六个范围外新 dirty 路径，均独立记录并保护，不覆盖、不代改。上述全 UI/types/build 是验收时快照；随后出现的 Asset 前端增量未再次做全库验收，不能据此宣称当前整工作树绿。本轮恢复自身代码 SHA 保持门禁版本。资产专项计划保持：[Qoder 分工](NEST_ASSET_EXPOSURE_QODER_PLAN_2026-10-07.md)。主控未 reset/clean/批量覆盖/自动 commit/push，未触碰真实 DB/CAS/asset。六份文档仅前插本检查点，U185及更早原文完整保留；其当时“未实现/下一片”不覆盖本检查点。

## 2026-10-07 U185 · 显式新工作后端发行与实际 SDK 评估

**真实功能（后端）**：新增生产 IPC `approve_native_root_financial_continuation`，独立绑定已有费用确认回执、proposalHash 与具体新工作描述。Immediate 事务和 scan/原 worker 静止检查下重核原签名模式、模型、任务身份、项目、C/fence/有效期及原费用证明；私有 typed authorization 只能发行新 attempt、新 Root、不可变出生/迁移事实、新 HMAC 与新 C。十维和共享模型额度来自逐项精确剩额，wall 上限同时受原绝对 deadline 限制；有限零值保持零。受控复制并复核原 frontend JSON 字节，原 Native JSON、原账单/证明和旧调用不重发、不改写。

专用 worker 已沿真实 SDK/Tick/费用与 publication 路径完成一轮新的 Root 评估，并为该新 attempt 写入原始 FinalClock/exit 证明，收口为 `partial / assessment_only`。这是新工作评估，不是整条 Web 任务完成；无普通 branch dispatch、Mapper、子代理或目标工作权限。原付费调用一次、新评估调用一次的 localhost 夹具已验证。发行之前及实际 SDK 之前分别持有新目标的 OS 所有权锁，避免付费后才创建静止证明。

**支持边界与只读 UI**：当前仅支持有完整原证明且仅一个历史 Root 的可证迁移；没有跨 Root 授权链就拒绝，不通过求和重新发完整额度。现有 UI 仍只读预览，`issuanceImplemented:false` 等三旗仍为 false，尚无独立新工作批准按钮；这些是前端当前 rollout 状态，不能据此说后端发行未实现，也不能把后端 IPC 当已交付桌面闭环。A02/U2 和 Goal 仍 **partial / active**。InputParser 生产接线保留；A03、BusinessLogic/Authorization/ExternalSurface 未就绪骨架未翻旗。

**测试证据**：最新 A02 集中 **8/8**（6 项既存预览 + 2 项新发行测试）通过，运行 23.60s、exit0；其中实际 SDK 正向、重复发行/worker 不重发、原剩額/期限/证据字节、错误 proposal 及恶意 late insert trigger 原子回滚均有断言。同一最新二进制邻近 **40/40**（费用9、模式20、新 Web 财务5、有限预算6）通过。lib Clippy **18 warnings、0 error**，消息集合与 U184 相同。日志：`/tmp/oviraptor-a02-issue-acceptance.log`、`/tmp/oviraptor-a02-issue-adjacent-final.{log,json}`、`/tmp/oviraptor-a02-issue-clippy-final.log`。未重跑 Rust 全树或 UI；历史 Rust 2624/1、U184 UI862/type/build 只保留其原证据，不能计入本轮全绿。全树旧空白/格式债保留。之后出现的独立 Asset 并行改动未纳入这些 A02 测试结论。

**下一片验收**：同步前端严格合同并接独立批准，区分 issued、实际 published、attemptClosed 与失败 review；扩充并发发行/激活、跨任务/attempt、撤权/过期、额度耗尽、坏账单/迁移与 startup 绑定篡改、写入故障及 published-before-finish 等崩溃重入负向。当前代码已有部分重入守卫，未实跑的故障点不能算已通过。实际桌面交互与真实 provider 仍未验收，不将 localhost 结果外推。

**资产并行分工**：只读资产代理已交付 [Asset/Qoder 详细计划](NEST_ASSET_EXPOSURE_QODER_PLAN_2026-10-07.md)，首片 Q0/Q1 可做独立 Asset 状态/分页/竞态；共享 schema/IPC/授权/Native SDK/预算/恢复合同由主控串行整合，不删除 asset。发现 `commands/exposure.rs` 与 `commands/exposure/` 的新并行增量，单独保留、标范围外，不覆盖、不纳入 A02 交付。报告附录哈希是报告生成时的快照，Qoder 开工时需复核漂移。

原像与本轮逐文件范围：`/tmp/oviraptor-a02-issue-20261007/`。入口262个 dirty 文件已逐项核对；入口范围外旧改动未改写，新增外部 Asset 文件独立记录。六份总体文档仅前插本检查点，原正文完整保留。未 reset/clean/批量覆盖/自动 commit/push，未操作真实 DB/CAS/asset。下方 U184 及更早内容都是当时快照，其“最新/待实施”不覆盖本检查点。

## U184 状态：只读预览已实现，执行发行仍待完成

原账单确认之后的生产新工作预览已有原始财务/授权/模型/期限复核，具体 taskDescription 与物理证明摘要入 proposalHash。当前权限三旗均 false，只读表示 future fresh Root assessment 的目的和该原Root精确剩余额，不证明跨Root金融授权链或给新工作执行权。新增Rust10项、正常UI862项已通过；精确证据与支持边界见 [U184复核](NEST_MASTER_RESUME_AUDIT_2026-10-07.md)。

下一实施点仍是下述完整合同的真实暂停/静止、独立批准、私有typed迁移、受控原字节复制、新attempt/Root/C/十维和共享剩余额发行、实际新SDK与异常重入。本合同整条闭环和Goal没有完成。下述原U183设计要求原样保留，不能把当前只读阶段替代这些验收。

# REM-A02 · 对账后的显式新工作合同（待实施）

本合同承接 U183 的生产费用确认入口。它是下一片设计与验收要求，不是已有恢复功能、已落地 schema 或运行证据。完整 Goal 保持：用户确认原费用 → 精确本地对账/不可变回执 → 重新核验原授权与剩余额 → 用户另行批准后实际开始新工作。

## 必须形成的闭环

1. 从真实已付 SDK 的费用 writer 故障开始；原 received invoice 保留，旧 call 不重发。用户确认绑定原 scan/attempt/root/call/request/basis/invoice 的费用回执。
2. 用户另行读取新工作预览并批准。费用确认成功、mount、刷新和事件都不自动恢复。预览同时显示新任务描述、使用的历史冻结证据、剩余额和原期限。
3. Immediate 事务内持有 scan control/quiescence，要求当前任务 paused/partial、原 attempt 不变、无活动 worker/调用；核对所有历史 Native Root 的原 owner、账单/证明、逐维消耗，未决或 reserved 非零拒绝。其他 Root 不能通过只核对一个 call 被隐藏。
4. 重新验证原签名 Web mode、冻结 plan/model、授权 session 与目标 host/身份，以及原 C/fence 未过期/撤权。原绝对 deadline 不延长；旧 Root/C/capability 不复活。
5. 事务中构造私有 `VerifiedFinancialContinuation`，IPC JSON、模型叙述和当前余额不能直接构造。绑定物理原回执/费用行、所属金融授权链、逐维剩余硬顶、共享剩余模型额度、新 attempt/target/新任务、原 deadline、单次发行/激活身份。
6. 由 typed migration proof 发行新 attempt、受控 evidence 目录、新 mode/HMAC、新 Root/owner/C 和准确剩余额。原 Native JSON、invoice/proof/owner/计划历史字节不改写。复制并核验原 frontend JSON 字节，明确它是历史冻结证据；不重跑原探测或旧目标请求。
7. 专用 continuation worker 产生新 purpose/basis/new Root/new call 的真实 SDK 工作，沿用实际费用、共享预算、取消及 deadline 门禁。不得加载旧 NativeAgentState 队列、补发旧 Root 语义或用普通 prepare 重新跑 bootstrap/Mapper。
8. 重复/并发批准只得到同一次发行/激活；派发后崩溃不重发新 call。收到未结算的新费用再次保留义务，不能由恢复回执掩盖。

## 已核对的接口与实现约束

| 位置 | 必须处理的约束 |
| --- | --- |
| `commands/scan_startup.rs` / `native_web_resume_finance.rs` | 普通 fresh 路径继续拒绝任何付费历史；新增独立的 typed 金融续接入口，不能把“已对账”当普通 Resume 通行证 |
| `commands/scan_quiescence.rs` | 生产暂停保留原 Root 财务；新合同可以从 paused/partial 发行，无需复活旧 Root |
| `agent_runtime/store.rs` 的 Native 出生证明/plan publisher | 保留 store 独占新 Root 出生能力；抽出事务内 publisher，让金融合同与新 attempt/Root 出生原子提交 |
| `budget/root_definition.rs` / `limits/contract.rs` / `root/shared.rs` | fresh schema 2 把前三模型维绑为同一 token 顶、target 请求绑为 4×模型请求；不能表达精确剩余十维。新增独立 migration definition/reader，旧 schema 和原 JSON 不改 |
| `web_mode/writer.rs` / `web_mode/finance.rs` | 增加接受私有迁移证明的出生 writer，不使用普通 fresh 完整额度发行。验证原行保持、单次激活、新 owner 与十维精确一致 |
| `budget/clock.rs` | 新 origin 仍等于新 Root created_at；在实际 INSERT 时按锁定原绝对 deadline 计算 wall 顶及 C expiry，不能把预览时的 remaining 再加到较晚的出生时间 |
| `budget/admission.rs` | 普通 admission 不放宽。迁移源需要独立“历史财务已结清”只读 validator；历史 finalized 本身不等于欠费，exhausted/unknown 仍拒绝 |
| `commands/agent_contract.rs` / `coordinator_evidence.rs` | 避开 ContinueIncomplete 旧队列和普通 Fresh 清理；新的签名 attempt evidence 必须来自绑定的原字节复制 |
| `commands/agent_native/coordinator_tick_once.rs` | 复用真实 transport/费用保存/publication；新增恢复 frame 的生产者与复核器，不能只翻 not_ready flag |

剩余额按所属原金融授权链计算，防止迁移重复扣费或为旧费用重新授完整顶。多 Root 所属链无法证明时拒绝；不得以当前余额或单张确认回执推断全链结清。若原 session/C/deadline 已失效，入口保留拒绝，不自动续约。

## 完成证据

- 必须贯通真实 paid writer 故障 → 生产费用确认 → 生产 paused/partial → 用户另行批准 → typed 剩余额迁移 → 新 Root/worker → localhost 实际新 SDK 与结算。
- 断言旧 call SDK/目标请求不增加，新 call/attempt/root 身份不同，累积真实费用 + 剩余所有硬顶不超过原合同，原 deadline 不增加。
- 未确认、跨 scan/attempt/target、回执/物理证明篡改、并发/重复、撤权/过期/fence 变化、坏/超限账单、未决/预留、额度不足、发行/回执 writer 故障和崩溃重入均有实际负向证据。
- 拒绝在授权/费用不明时不得新增 SDK/目标 I/O；事务边界核对全表与物理行、文件字节和新目录。支付后故障要保留真实费用，不能用“零写”抹账。
- 生产 UI/IPC、真实 localhost 新工作、财务守恒都证明后才能完成 Goal。只有 schema、静态接线、同 Root 的 still-scanning 单 round、只读诊断或模型叙述均不足。

不需要用户实时答复才能实施常规可逆代码、夹具和文档。禁止自动 commit/push、重置/批量覆盖、恢复 Strix、真实数据清理或删除 asset。
