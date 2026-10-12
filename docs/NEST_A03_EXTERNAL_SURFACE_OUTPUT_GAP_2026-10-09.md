# Native ExternalSurface 输出：真实来源缺口与最小闭环

2026-10-09，只读设计。本轮阅读最新 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`、REM计划相关角色/Root门禁与当前 Native 源码，未修改生产、未运行 cargo。此处 ExternalSurface 是 Native multi-agent 的匿名目标入口角色，**不属于 Qoder Asset Exposure 业务**；未读取或修改其业务文件。本文件不代表功能完成。

## 当前真实功能

`commands/multi_agent_external_surface.rs` 已有生产链：预算预检→ExternalSurface target_touching 调度→单次匿名入口 GET→实际 specialist SDK分析→费用结算→completed readonly advisory 交付。prepare.rs 在 external_surface配置启用时调用。原token预检为2000 grant+15000 Reviewer留底，共享requests>=3；child模型预留1。这个已有实现不等于独立Root输出评估或Root派发。

匿名合同严格使用当前target URL（去fragment）、GET、anonymousOnly、maxRequests1、followRedirects=false；拒URL账号密码/secret文本。isolated.identities仅anonymous、browser=None；reqwest不使用authenticated helper，不发送session cookie/Authorization/Origin、不跟重定向、不额外CORS探测。proxy使用配置。只允许此入口，不枚举路径、扩域、切身份或浏览器动作。

真实claim在IMMEDIATE中校验原C/worker/frozenplan/cap/taskSlice/熔断/继承target计数，写 `agent_external_surface_captures(state=claimed)` 与 source `public:{assignment}` 的target费用。发送前commit，transport未知不重发。实际响应读取最多AGENT_MAX_RESPONSE_BYTES+1，截断到上限；记录status/contentType/securityRelevantHeaders/truncated/保留bodySHA/bytes和不可信摘录，再写HTTP artifact。

received事务原capture变received，绑定artifact/responseJSON/hash，target::receive结算原请求，写observed RequestRecord并复验文件；429或WAF保护保持真实费用而返回保护错误。specialist实际多agent transport有原SDK receipt/usage，评估JSON严格confirmedFindings=false、必有coverageGaps。readonly_assessment.rs结算核received模型文本/usage，ExternalSurface另核原capture/artifact；终止worker、撤cap/release lane、发送 `evidence_summary`、correlation `public-surface:{assignment}` 并消费ACK。发生已付模型交付失败时保留模型费用、无新GET，现有本地保存结果恢复路径应保留。

`agent_tests_external_surface.rs` 当前源中已有真实loopback GET+specialist SDK、匿名头/no redirects、claim故障零网络、撤权/过期/取消拒发送、receipt写失败不重试、模型坏schema/confirmedFindings拒、Reviewer floor、artifact变动、response暂停、交付故障保费用等测试。它们是现有测试代码的证据范围；本轮没有重跑，不引用旧通过数作为当前绿。

## 当前真实缺口

1. `agent_external_surface_captures` 只有response hash，received更新保护但允许DELETE/CASCADE；原body/record hash与graph事实也非独立私有首次响应签名。当前review reader核表/文件对应，不能把可重建一致JSON/hash当私有原transport证明。
2. capture GET没有保留同原claim writer的private transport witness与首次实际response bytes typed发行能力；需防保存JSON重签、响应后才回填capture的伪来源。
3. 原specialist SDK收费有既有真实receipt路径，仍需将独立响应签名、原SDK、原target与正常闭合交付聚合为typed immutable output，保持原缓存/usage来源，未知不猜零。
4. Root inventory当前external-surface-output仍reserved/notready，step为dispatch:external_surface；`coordinator_external_surface_gate.rs` actual Root选择仍external_surface_dispatch_not_ready。这是角色派发缺口。新输出监督应独立assess-only，不把output事实变成再次target派发。
5. 当前requests>=3预检与Web专属headroom不能保证新Root额外1。ExternalSurface专属claim不经generic Web HTTPguard；后续必须在它的原claim事务复检新原义务。

## 最小新版本实现合同（建议下一原出生版本；未实现）

先在新signed bootstrap冻结 `externalSurfaceOutputRule=original_closed_anonymous_entry_surface_output_advisory_v1` 和 `externalSurfaceOutputRequestCapacityRule=retain_one_original_surface_output_assessment_per_worker_v1`；旧6–11合同原形/算法与门禁不变，不补签旧capture，不read-time升级。Root派发仍notready，单独规划并验收后再开放。

### A. 首次实际匿名响应私有签名

新增 `commands/agent_native/external_surface_transport_receipt.rs` 与可信schema。`claim_public_surface_capture` 改typed私有guard返回，同Box<Connection>原writer：原claim/fee committed后、send前在原事务捕获fresh witness。私有constructor绑定root原birth/mode/C、worker epoch/fence/assignment、contract/URLhash/匿名身份/固定method、原claim rowid/source、原requestHash。不能接受savedJSON成为fresh witness。

真实response headers/body读取后，使用原writer、原claim witness和可信artifact writer结果发行 `FreshExternalSurfaceResponseReceipt`。签名绑定实际status、保留bodybytes/bodySHA、截断/读取上限与truncated语义、recordbytes/recordSHA、artifactID、原target exact费用行与scope、originalphysicalhash。明确只认证保留字节，不宣称截断后未读取的完整正文。body incomplete不得发行完整响应证明；未知claim/费用保留。原首次response reader不调用SDK/目标，不建立新claim，不补签；独立signature表保留，不依graph存在做真来源。

### B. 正常聚合输出/闭合交付

新增collection pure reader：原signedresponse、原SDK received与终态签/usage（四维与真实target费用/原worker时钟）、原匿名taskSlice、artifact/hash、安全评估JSON一致；没有SDK未知费、无runninginvocation。实际异常reader不能吞成missing。用typed constructor从原真实normal completed子交付发行独立 `VerifiedExternalSurfaceOutputReceipt`，source包含responseReceiptId、worker、assignment、SDKcall/usage、summaryHash、完整scope/匿名覆盖缺口、cap声明与撤销集合、原evidence_summary精确Root-only route/correlation/revision/ACK、原固定outbox physical hash。费用已settled或graph节点本身不能假闭合。

历史reader只验原birth/key/source费用/文件/closeddelivery；不require liveC、不能复活旧worker。capture/issue阶段额外核liveC/父alive和原idle锁；paidRoot评估跨过程保留producer锁但reader不重新probe自己持锁。保护/未知/取消失败有独立停止路径，不能伪normal completed output。

### C. Root付费受限输出监督

新增 `coordinator_external_surface_output_frame.rs/hook.rs`，kind/phase external-surface-output、step **assess:external_surface_output**。frame稳定事实为receiptID+完整原typed source，不含Root新增fees/currentclock/C延长TTL。paidSDK前后originalC/父alive/frozenplan/determinate/source全复验，所有suggestions只允许非空此assess；实际tool_calls即使provider附真实usage也拒执行/拒publication，费用保持。禁dispatch/localTools、新targets/newscope、角色或权限发行。

pub(crate)原publication重放桥：首次无输出receipt合法None，其他签/费用/schema/文件错误传播；已有原matching changedFact→Tick::project_publication严格核原paid received/终态/publication且bounded非空无localTools，0SDK/0写，不需新权限。Root预算不足/503/未知不能伪assessed。

### D. 一worker一次Root输出请求义务

每个issued ExternalSurface worker仅1义务，既有specialist的模型request1仍是SDK reserve，两者不同，不能把模型reserve双扣或说已含Root输出。新grant在同原scheduler IMMEDIATE保留：原shared floor+capPending+AuthPending+SurfacePending+newSurface1，原hardcap不提高。真实专属 `claim_public_surface_capture` 每GET在同claim事务复检已issued义务，不再次new1；Web分配/HTTP和Auth调度/claim也共同保留Surface义务。only signed aggregate source对应原受限paidpublication消除1；receipt/settled/revoked/503/unknown/localTools中间步骤均不消。

inventory union assignment/worker/public_surface_get caps、原capture、原target source public:{assignment}、独立response/outputreceipt、specialist原call，强scope/contractowner/worker/key，不让删除claim或冷改role隐藏义务；旧版本Surface source不被回填新合同。原SDKreserved已体现在remaining，额外仅Root未来1。token/USD/deadline各自门禁，不称请求数保留保证完整未来费用。

## 验收与完成边界

- 真实新原birth→匿名GET1→SDK1→原响应签→正常settled/撤cap/独立output→Root实际SDK1→原paid boundedpub；重复0GET/0SDK/0费用/0写。
- 匿名头、fragment、redirect、异URL/identity/proxy/modelbinding、缺签/错key/coldJSON+重算hash/文件替换/删capture/跨worker都精确拒。
- claim/receive/signature/artifact/SDK/ACK/outbox写故障，429/WAF/timeout/incompletebody/SDK未知等保持已付费用，不自动重发、不fake完成。
- 已issued preGET义务1、normalclosedreceipt无pub仍1、真实paidpub后0；不足原headroom调度/claim纯读拒，实际网络0，不raise cap；取消/失效/unknown保持未完成义务。
- toolcall/dispatch/unbounded/空suggestions拒新执行；真实provider账单保留。旧6–11正向/故障合同不降级不修改。

只有这一生产完整链真实验收后才称ExternalSurface Root输出完成；现有匿名GET/specialist/completed advisory属于真实已有功能，Root reserved gate属于未实现骨架，hash复验属于只读来源核验，测试代码与历史记录是有界证据。本轮仅设计文档。
