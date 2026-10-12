# REM-A02 生产闭环逐项验收 · U188 · 2026-10-07

本审计针对用户 Goal 原文中的一个可用生产闭环，不是完整 Master、完整 Web 调查或最终发布验收。验收入口从真实 Vue 按钮，经本地 Tauri IPC 到生产确认/批准/worker、真实 localhost HTTP SDK、账本/发布/FinalClock，再经真实只读 IPC 找回。原 paid call 不重放，新工作是用户明确批准的冻结 Web 证据 Root 评估。当前只有完整、单个历史 Root 的授权链可迁移；未证明的多历史 Root 不恢复、不求和发全额。

## 判定依据

先将完成视为未证明，以下逐项对应原要求。源码存在、mock IPC 或套件数量本身不能证明桌面闭环；独立桌面和私有数据产物补足实际入口证据。U187范围23文件哈希入口复核一致；HEAD仍14a78753b95aad65e8fd8611048e6d6ff8ddb9a2。正常用户数据/旧迁移路径未启动，所有桌面权利与原账单从出生就位于私有acceptance目录。

| 原要求 | 当前权威证据 | 判定及边界 |
| --- | --- | --- |
| 读取最新计划/进度/续接并保护工作树 | U188入口前像与逐文件哈希；六份U187原正文备份；最终scope单列范围外Asset漂移 | 已核对；仅增量接线，不reset/批量覆盖/提交/推送 |
| 原账单身份/任务/attempt/哈希显式确认 | 真实App显示原scan f1e649f6-375a-4e36-ab96-4086f306b78b、attempt1、call与账单/确认hash；用户点击核对、再点击确认，生产IPC返回原immutable receipt | 已证明；原调用实际50000/10000/10/1，总50010 |
| 精确结费，不将估算当费用 | root_cost_ 9/9与root_finite_ 6/6；真实桌面确认前SDK1/回执0，确认后SDK1/回执1/出生0，仍paused1 | 已证明；超顶/缺失/不一致bill仍blocked，费用确认不授权执行 |
| 不可变回执与重复/并发确认 | 原费用atomic writer/binding/concurrent ack测试；新发行concurrent approval/worker测试；冷读/篡改回归 | 已证明；一出生、一新付费请求，重复读取/批准不会补发旧调用 |
| 再核原执行授权和预算 | typed authorization在Immediate内复核原C/fence/有效期、项目/身份、签名模式、模型、原账单与财务原像；当前32财务回归含policy/session/identity/到期与未知义务拒绝 | 已证明；对账回执不能替代原执行授权，不续旧C |
| 原十维/共享剩额与原绝对deadline | 真实预览显示共享149990/19及十维；实际新Root时钟起点18:18:40、硬顶652000ms，截止恰为原18:29:32；新bill31/3/7/1，共享余149952/18；原/新frontend与原task JSON SHA保持 | 已证明；有限零值不转无限，目标/浏览器/写入/上传/批次未执行 |
| 用户独立批准并实际恢复新工作 | 真实App独立checkbox及批准按钮，经真实IPC新建attempt2/新Root，SDK总2，actual received31/3/7/38/1，原JSON SHA保持 | 已证明；第二调用是新Root评估，不是旧SDK/目标重发 |
| 实际发布、真实收口、不伪整任务完成 | 真实状态IPC先返回published/unclosed，再返回original FinalClock验证closed；私有DB Root/attempt终止码assessment_only、scan partial2；冷启动聊天显示已保存38tokens摘要 | 已证明；整个Web剩余目标未完成 |
| 关闭/重启后的可用找回 | 真实关闭面板清空绑定、重开无自动批准，再明确点找回；完整退出/新PID重启后再次真实IPC找回相同approval/proposal/original receipt/closed状态 | 已证明；SDK保持2，已闭合无执行按钮；历史读不授新权限 |
| 跨任务/attempt、撤权/过期、坏账单/额度 | 财务32回归与邻近40回归，具体guards/confirmation/finite/discovery tests；合法原cap50010/50011出生后实付结清，余0/1分别拒绝/回滚 | 已证明；不可用tampered cap替代合法耗尽证明 |
| writer故障、崩溃重入 | 原费用/出生/新费用/finish writer faults；实际exit86在付费发布后、exit87在received commit后terminal前；重入不新增SDK，缺失证明保持未决/review | 已证明；没有用伪paused或改状态假装真实崩溃 |
| 原Native JSON、旧paid facts、asset与Strix边界 | 源frontend JSON的manifest SHA最终相等；原paid rows/proofs的测试frozen snapshots；最终自己的scope无Asset；HEAD未变 | 已证明；不恢复Strix，不清理真实DB/CAS/asset |
| 不夸大历史门禁 | 当前Clippy18/0且消息集合不变；Rust全树未重跑，历史2624过/1 legacy-v2失败保留；全树两处旧空白债保留 | 已记录；不称全树/严格Clippy/完整Master绿 |
| 阶段交接 | 本审计和六文档U188前插；以下给出A03具体切片及未就绪边界 | 已给出 |

## 本轮命令与证据等级

- 当前测试二进制财务集中32/32、0忽略、146.35s：31个财务场景加1个默认无启用env的desktop driver helper。32的数量不能证明桌面；另行explicit exact+双env的桌面driver已实际运行，1/1、900.89s，生命周期按15分钟上限结束，SDK总2。
- 新崩溃用例单独1/1、14.47s；真实子进程exit87。received已独立提交，terminal/decision/publication/exit均缺失，预留保留，重复批准/worker不重发。
- 当前邻近40/40：root_cost_9、web_mode_20、web_fresh_finance_5、root_finite_6，分别exit0。成本/有限预算场景是真实localhost与已明确标注底层金融夹具，不能以总数冒充全部桌面。
- 当前vue-tsc exit0；Clippy18 warnings/0 error，与U187消息集合一致。此前U187全UI893/893和Vite隔离build属该轮快照，本轮未重复全UI/发布打包；实际桌面使用独立名称的debug Tauri App和本地Vite14687，通过真实WebView/IPC。实际App生产代码来自U187，U188只加cfg(test)故障钩子/夹具，release无注入行为。
- 三个新叶rustfmt --check、局部diff/空白通过。未删除/ignore测试；opt-in driver无enable env只返回，有enable必须exact且有效private acceptance root并拒绝预存DB。

日志/原像集中：`/tmp/oviraptor-a02-crash-20261007/`。核心产物：`received-crash.log`、`financial-regression.log`、`adjacent.{log,json}`、`clippy.log`、`types.log`、`desktop-build.log`、`desktop-binary-receipt.json`、`desktop-seed.log`、`desktop-before-confirm.json`、`desktop-after-confirm.json`、`desktop-after-preview.json`、`desktop-after-approve.json`、`desktop-after-restart.json`、`desktop-provider-final.json`、`desktop-deadline-json-preservation.json`，以及各代理scope和final-scope。private profile在desktop-profile.json，原DB/签名/JSON保留供复核；自建App和Vite已停，不影响用户App/Qoder。

### 失败与测量限制

额外Python3.12/SQLite3.54只读探针曾间歇CANTOPEN(14)，系统sqlite3也曾出现一次；未转为可写、未删WAL/SHM、未复制/改权限绕过。相同原路径之后的system sqlite3 -readonly成功读取，并记录实际出生、原回执、费用和收口。生产Rust只读IPC真实UI读取/重启找回均成功。本审计保留该测量问题和失败记录，不声称探针一次全绿，不将尚未查明的原因归为生产缺陷或已修复。

一次测试App冷启动尝试把CFBundleExecutable指向shell、再exec不同名字的原二进制，导致macOS无法关联bundle ID；仅修私有测试bundle Info使其匹配实际Mach-O，并始终用显式acceptance env启动，随后真实CUA识别并完成冷启动找回。主库/产品打包文件未改，二进制SHA不变。

## 下一Master阶段：REM-A03

先复核新evidence、lease到期、预算/压力/保护变化、人工恢复等六触发真实接线与changed-fact去重，不把只读盘点当Root监督已可执行。建议下一最小片选**新证据变更触发Root监督**：同一原活父/C/目标/模型与有限预算下，生产入口捕获新evidence hash，绑定旧→新事实，真实localhost Root SDK写回原费用与监督回执；同hash重复零SDK/零新增动作，跨任务/fence、撤权/过期、证据损坏、超限和writer故障保持义务并拒执行。该片从一个普通新Web Root出生，不借用已闭合A02评估恢复旧模型语义，不把新C当原权限续期。先核验实际frames_not_ready，再决定是否开唯一触发；未接触发/角色仍false/not_ready。

A02更宽的多历史Root授权链、Mapper/目标剩余工作、真实第三方provider质量、安装/发行包和完整Master验收仍未完成；没有把这些包含进已证明的单Root财务恢复。Asset实现继续归Qoder，共享schema/IPC/授权/预算文件协调串行；不改其业务叶或清理资产数据。

## 完成结论

原Goal要求的生产确认→精确回执→原授权/预算复核→独立新工作→真实SDK与收口/找回闭环，以及所列必要正反向，均有当前证据。Goal可标complete；完整REM-A02/Master继续按各自更宽范围追踪，不能据此整体done。上述未扩展支持/测量限制不是被隐藏的生产闭环缺失，也未放松任何原财务/授权守卫。
