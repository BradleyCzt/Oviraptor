# Human Recovery：实现与验收计划（2026-10-08）

最新入口：2026-10-08 恢复草稿取消已接生产，撤权与签名历史分离；专属出生/消费者/UI仍未完成，详见文末及 progress 顶部。

状态：全 scan 历史只读准备、独立新 scope 提案/签名批准及唯一待出生新草稿/action 后端已接并有实际 localhost 测试；专属 Root 出生/实际目标消费者/确认 UI与恢复闭环仍未完成。完整 Master goal 不缩减。Asset Exposure 由 Qoder 独占。

## 已核边界

- `commands/agent_native/coordinator_protection_closed.rs` 能在旧 C 过期后读取 signed WAF 原响应、原 Web 费用、退出与 execution_result ACK，不能授新执行权。
- `commands/agent_native/coordinator_protection_hook.rs` 只在活原 C 上付费评估原停机事实；Root advice、费用确认和新任务批准是不同权力。
- `commands/native_scan_branches/directives.rs` 的聊天确认及 `multi_agent/directive/confirmation.rs` 要求 active attempt/open Coordinator/live 原 fence，不能复活 terminal/paused/expired Root。
- `commands/native_financial_continuation_preview.rs` / `finish.rs` 的合同是 fresh_root_assessment_of_frozen_web_evidence / assessment_only，UI 明示不执行目标剩余工作。
- `commands/scan_control.rs` 普通 Resume 沿用旧 scan；`native_web_resume_finance.rs` 拒 paid history 未有专属财务合同的启动。`agent_runtime/target_protection.rs` immutable stop 以 scan/target 生效，跨 attempt 保留。

不能删除旧 stop、借只读财务 Root 派发 HTTP、把费用确认当新目标批准，也不能只复制旧 plan 到新 scan 后声称 WAF 已解决。旧 task 的真实财务债仍须保存并明确处理；未知 SDK/HTTP 不能获得自动重发许可。

## 下一实现顺序

1. **历史来源与财务准备度**：在原 DB snapshot 中独立读取 signed/closed stop 及全部相关原 Root/child/target SDK/HTTP 费用，拒不确定、重复/错 scope/缺 ACK/文件损坏。保持纯读，不续租、不建新 Root、不修复原历史。输出绑定 hash 和明确未就绪原因；原财务续接的只读评估不得冒充本能力。
2. **恢复提案与专属批准合同**：展示原停机原因、旧成本/原 stop 保留、拟议新任务的 URL/方法/身份引用/模型/十维预算/绝对 deadline。用户在独立确认入口批准新的目标任务 scope；新增不可变 proposal/approval 及原 source ↔ approval 绑定，拒普通聊天/原费用批准替代。批准命令必须前后验证原事实及完全一致的用户选择，并支持同 scope 并发/重启/响应丢失下的幂等查询。不能由后台模型产生用户批准。
3. **新任务出生与授权**：基于真实 URL draft/create/confirm/start 链建立新的 scan 与独立授权、预算和 Root；将 approval 与新 draft/Root 的对应关系在出生时冻结。不得给旧 Root 回填恢复协议或替换私有出生材料。明确新任务是新授权，不退款/重发未知旧工作、不删除旧 stop；旧/new IDs、模式、模型及所有维度都需被验证。
4. **真实消费者与 human-recovery 监督**：生产消费者只接受原批准的新 scope，Root 人工恢复评估使用固定 advisory step 和 changed-fact 去重；真正目标动作使用独立新授权和实际 Tool Broker，保持所有写操作/身份/预算门禁。若新任务再次 WAF，为新 scan 留自己的原 stop 并关闭；不得递归自动建任务。需要恢复的完整获批任务范围必须真实执行并收口，不能以只读诊断或单条夹具 GET 替代 Master 要求。
5. **UI 与债的验收**：用户可看原停机/费用/为何需新任务，以及审批前后的执行范围；原财务确认、恢复批准与执行进度均清楚区分。继续解决 paid Single Resume/Retry 既有失败；不得删除/ignore 测试或削弱历史财务门禁换绿。

## 必要运行证据

实际 localhost 原 WAF → 原费用/退出/ACK → 无批准 SDK/目标请求零增长 → 独立批准 → 唯一新出生/真实新请求 → 原账与新账分别关闭。重复确认、并发、批准前后 source 漂移、模型/身份/预算/deadline 错 scope、撤权/过期、旧财务批准误用、未知目标 outcome、重启和分阶段 SIGKILL 都不能产生多授权或重复目标请求。旧 DB 行、费用、stop 和原文件保持；真实数据不清理。第三方/平台或真实凭据证据若缺失必须明确记录，不能用 localhost 证明覆盖所有平台。

## 实施时不得推断

不能推断用户已经解决 WAF、提供新身份/凭据、允许代理轮换/CAPTCHA 绕过、扩大 URL 或追加预算。实现可自主完成，真实新执行仅在用户专属批准后开始。不存在批准时继续做代码、UI、负向与临时夹具验收，不制造批准。原资产数据及 Qoder 业务文件不参与修改。

## 2026-10-08 所选 Root 恢复准备度只读入口已接；独立批准仍未闭合

真实只读功能：新增 opaque NativeRecoverySource，持原 Web SDK/whole executor 两退出 inode guard 与第三个原 Root SDK guard；核原 signed protection、原 closed ACK/费用、所有原 Root calls 请求/terminal/received/cost/已付语义证明和既有历史费用 helper、原 child 终态/原 owner/十维 reserved与indeterminate零。Root/child 原物理行冻结并复验，不续旧C、不补签、不结账修复、不退款、不给SDK/target权限。独立 source 包含新保护监督原Root费用，不加入旧保护frame dedup basis造成重复收费。

生产入口：preview_native_protection_recovery 已真实 include并注册Tauri，严格READ_ONLY|FULL_MUTEX打开现有数据库，不创建缺失DB；从main原coordinator按Root推导actor并精确scan/attempt/root，原HMAC/target/epoch/fence仍核验。新窄路径接口避免伪造AgentRunContext/环境；旧closed context接口保留原scope/dir检查。source三个guards持到snapshot commit；返回readiness ready仅为所选Root历史准备度，approval/grant/executionGrant false与targetRequestsGranted0，不是全scan历史盘点或恢复执行。

真实localhost测试：recovery-first-tests.log2/2、14.19s；补未知费精确拒后recovery-exact-tests.log2/2、14.21s（compile30.84s）：actual Root/Mapper/Web SDK+WAF GET1+保护Root paid finish后ready、原context与IPC exactfact相等、错scan精确拒、READ_ONLY重开、旧C过期历史read仍ready；success/fail全应用0写、SDK/GET不增。实际保护Root SDK无usage保原input indeterminate与known request1，原Webclosed仍可读但恢复准备连续两次精确budget_indeterminate_requires_reconciliation拒、原账不变，不接受任意Err。原closed-regression.log2/2、11.57s：原ACK/cached/closingevents冷损坏、双原inode占用及关闭前拒仍通过。共4不同测试，库存2842，未全库重跑；recovery-clippy.log lib22warnings/0errors、16.56s。全部Cargo terminal/exit0，源码冻结后主控nice15/-j1/tests1串行。before/hashscope在/tmp/oviraptor-recovery-source-20261008/和agent专属tmp。

边界和下一项：旧financial helper继续拒未原确认的paid-unpublished、unsupported Authorization费用及exhausted elapsed，不伪造结账换ready。只读审计未发现可通过跨scope弱化；内部错误DBpath的canonical核验仍在probes后，最终必拒且IPC不能提供path，可前移小幅硬化。尚欠新source RootSDK占用/Root费用漂移专属实际矩阵、全scan durable-owner UNION历史Root盘点、独立新scope批准/不可变绑定/新scan出生/实际恢复消费者和UI、paid Single Resume/Retry旧债及全Master其它条目。docs/NEST_HUMAN_RECOVERY_IMPLEMENTATION_2026-10-08.md更新为本片准备度入口已实现，批准/恢复仍计划。完整REM-A01—A14 goal active；Asset Exposure Qoder独占，Native JSON/dirty/真实数据资产保护，无reset/clean/commit/push/asset删除，不恢复Strix。

## 2026-10-08 全 scan 恢复历史只读入口与旧恢复回归修复

真实功能（只读诊断）：preview_native_protection_recovery 已由所选 Root source 扩展至 scan 历史盘点。固定 SQL 从原 runs、授权/HTTP/capture claims、budget/mode/birth、elapsed、protection、financial continuation、cost reconciliation 与原签名 JSON 独立收集历史 Root，拒孤儿 owner/错 scope/非终态/未知费用。逐 Root 核原出生和财务历史，持原 parent、SDK/HTTP 退出锁至 snapshot commit；末尾重读私有出生/原保护响应与财务证明，不把 SQLite snapshot 当作文件冻结。Single 按原 Single 退出合同、Multi 按原 Root journal 核验；不续 C、不建 Root、不修账、不补签。返回 grant/approval/executionGrant false、targetRequestsGranted 0。原 database canonical 检查已前移至 probes 前。

测试证据：/tmp/oviraptor-recovery-history-20261008/recovery-history-first.log 首轮广筛 52/60，通过之外 8 项失败保留；recovery-history-fixed-targeted.log 中间 13/14，Reviewer 旧总数断言失败保留。最终 recovery-history-tail-targeted.log 8/8，0 ignored，66.36s，compile 1m12s，库存2847；最终 recovery-history-clippy.log lib 22 warnings/0 errors、17.75s，exit0。真实已付 WAF localhost source/history、Root SDK 占用、原费用/terminal 冷漂移、live parent、scope 隐藏及孤儿 durable owner 拒绝均验证全应用零写、SDK/目标请求不增。孤儿 owner 是临时数据库负向夹具，尚非真实第二付费 Root 正向证据；完整多历史 Root/Single 正向、崩溃矩阵仍欠。

旧回归修复：bootstrap 使用原 signed issuer；Investigator/gap/Reviewer 夹具改走真实 Web SDK/退出/费用，不用假 finish。前一轮相关 13 项中的其余测试已通过；本轮最终 Reviewer 通过，明确原 native-agent-coverage 观察行与唯一人工 Reviewer 候选分开，核原票据、冻结候选、原决定、唯一发布和 ACK，冷篡改仅人工 slot。没有放宽发布或费用守卫，没有删除/ignore 测试。尚未重跑全库，不宣称原全库失败全部清零。

下一优先：独立恢复 proposal/明确新 scope 用户批准、不可变原事实绑定、新 scan 出生及真实消费者/UI；全 scan 历史入口已接不代表用户确认或恢复闭环完成。paid Single Resume/Retry 旧债与其它 REM-A01—A14 继续。完整 Master goal active，不缩减、不报虚构完成百分比；Asset Exposure Qoder 独占，Native JSON、dirty 与真实数据保留，无 reset/clean/自动 commit/push/asset 删除，不恢复 Strix。

## 2026-10-08 独立新 scope 恢复提案与待出生签名批准

真实后端功能：新增并注册 preview_native_protection_recovery_proposal、approve_native_protection_recovery、get_native_protection_recovery_approval。提案独立绑定原 scan/attempt/Root/保护回执、全历史只读证明、用户明确选择的新 project/精确 URL/方法/新草稿身份/模型及 endpoint-key-proxy 绑定（代理仅接受原批准池的 proxyBindingHash，服务器解析 URL，回执不保存代理用户名/密码）/模式/策略/技能/说明/USD 上限/十维限额/绝对 UTC deadline；源任务须已停止且项目 active，旧任务身份不得跨任务复制，当前无 writer 的 browser/write/upload 正额度明确拒绝。预览只读，不发 SDK/HTTP。

专属批准合同：必须提供 exact proposalHash、UUID requestId 与 approve_exact_new_scope_preserve_old_stop_and_costs consent；普通聊天、confirm、原费用确认字符串不能替代。Immediate TX 下重新生成并逐项比对提案，仅直接 INSERT 新 immutable approval 表，原 scan/Root/C/stop/账/身份都不改。有限 authorizer 仅增加 database_list 只读元数据例外供原响应物理 DB 验证，仍拒其他 PRAGMA/ATTACH/DDL/UPDATE/DELETE/触发器写入。schema 完整核验、原 rowid/业务键替换拒绝、插后原 SQL/全历史费用/私有出生与响应文件复验及 guards 至 commit 保留。

批准真实性与重复：opaque FreshProtectionRecoveryApproval 只能由已验证提案和专属 consent 构造，原私有出生材料按独立域签完整批准；不暴露 raw signer、不对冷 JSON 补签。重复列/canonical proposalHash/approvalId/原 mode receipt/HMAC 都核验；same request 原选择返回唯一同记录，改 source/scope/hash 或另 request 对同原停机来源冲突拒绝。历史查询或响应丢失重读只证明原批准存在，不证明当前仍可执行；created_at/物理 rowid 不作为可信批准时间或消费凭证。

实际 localhost 最终证据：approval-proxy-final-tests.log 5/5、0 ignored、63.61s，compile1m23s，库存2852；此前 approval-metadata-tests.log4/4、85.39s 与 canonical 精确串行化改写后的 approval-final-tests.log4/4、54.02s 均保留。原真实 Root/Mapper/Web SDK+WAF GET1→真实付费 finish→原外层 target owner 释放→生产 request/finish pause→proposal 全应用零写→唯一批准落盘，原其余全部应用表不变；READ_ONLY 重开/同 request 重读零写，两个真正线程并发返回同一批准且仅一行，SDK/GET不增；generic consent、预算/方法/model/project/deadline/不支持 write/原 projection 漂移精确拒零写。recursive_triggers OFF 下显式 rowid OR REPLACE、UPDATE、DELETE 拒；仅可丢弃 TX 模拟冷 JSON+proposalHash+approvalId 重算、恢复 exact trigger 后原 HMAC 精确拒，两次不修复，rollback 后原签名记录可读。新增第五项验证真实已批准带用户名/密码代理仅由 hash 选择，proposal/持久 approval_json 不含代理 URL/用户名/密码或模型 key，raw URL/未批准 hash 精确拒零写，SDK/GET 零增长。不是第三方模型/平台或新目标执行证明。

相邻与失败记录：approval-source-regression.log 9/9、0 ignored、105.96s、compile0.92s，原 closed ACK/usage/event 冷损坏、source SDK busy/未知费/terminal-cost 漂移、history live parent/孤儿 owner/隐藏 scope 回归通过。不同相关测试14项；本片主控未重跑全库，不借 Qoder 正在运行的旧源码快照全库进程作当前门禁。approval-first-tests.log 编译 E0433、approval-fixed-tests.log 0/4 缺原外层 target inode、approval-original-target-tests.log 1/4 写限权拒 metadata 的失败全部保留。夹具 target owner 改为首次 SDK 前真实持有、完成后释放，不在 paid 后补建；生产暂停/费用/签名守卫未削弱。中间 approval-clippy.log23warnings/0errors，新增 cmp_owned 已用明确 canonical JSON 串行化变量修复，保持完整 bytes 比对而不改成 JsonValue 对字符串的值比较。approval-final-clippy.log22warnings/0errors、7.45s；代理引用最后变更后的 approval-proxy-final-clippy.log lib22warnings/0errors、14.58s、exit0，新增分支未增加警告。

尚未完成：状态严格 approved_pending_fresh_birth，executionAllowed false/targetRequestsGranted0；独立确认 UI、新 draft/scan/Root 关系、fresh birth、批准消费者、实际目标执行、人为恢复 paid监督 feed 均未完成，human-recovery inventory 仍 not_ready。下一出生需持 scan-control 与全 scan 所有已付 Web 原外层 target 退出 locks（缺 inode 不补建）至提交，重新核来源/model/身份/策略/deadline、精确保留获批 literal URLs，不借普通创建器的尾斜线/拆分规范化改变授权；将批准与唯一新 draft/scan/root 的完整 immutable action/出生签名绑定，处理并发/提交结果未知，再接实际消费者和 UI。真实多历史 Root/Single 正向、全崩溃故障矩阵、paid Single Resume/Retry 旧债与全部其它 REM 条目继续；旧 exhausted elapsed 等 helper 拒绝边界尚保留，不能称全部新授权可恢复。

完整 REM-A01—A14 goal active，不缩减、不标 Master 完成；Asset Exposure Qoder 独占，Native JSON/dirty/真实数据资产保留，无 reset/clean/自动 commit/push/asset 删除，不恢复 Strix。代码前像、SHA/本片日志在 /tmp/oviraptor-recovery-approval-20261008/；确认 source/history 只读证据和批准持久化均不等于实际恢复执行。

## 2026-10-08 恢复授权补齐原外层 target 退出锁

真实授权守卫：新 scope 提案及专属批准在完整历史证明后，另核全 scan 独立库存中每个 Root 的原金融 owner 与原 scan/attempt/literal target。按 (attempt,literal target) 去重，只 probe 原 "target" inode，不 claim、不规范化 URL；忙碌/坏 inode 返回 recovery_history_target_not_idle，缺原 inode 返回 recovery_history_original_target_exit_missing。guards 随 opaque 提案保留至事务 commit 后释放；提案增加 originalTargetExecutorsVerified，纳入原 proposalHash 与 HMAC。此字段不是启动许可。纯历史只读诊断不改变，旧 scan/stop/账/C/身份不变。当前采用严格全 Root 原 inode 门禁，即看似未付费但缺原 inode 的历史也拒绝，不为其补建或推断未派发；未完成真实多历史 Root/Single 正向。

测试证据：/tmp/oviraptor-recovery-target-guards-20261008/tests.log，7/7、0 ignored、107.58s，compile1m14s、库存2854。保留原5项批准回归；新增实际已付 Root/Mapper/Web SDK+WAF+生产暂停后，外层锁重新忙碌拒绝 preview/approve、旧应用全表零写；原 owner 释放后提案持原锁至 commit，批准唯一写入且不产生 SDK/GET。仅可丢弃夹具移除其已释放原 inode，重复预览/批准精确拒绝、文件数不增、不补锁、全表不变、GET保持1。不是新目标执行、第三方或平台证据。Clippy lib22 warnings/0 errors、11.49s、exit0；三个 owned 源文件 SHA 冻结核对、rustfmt --check及 scoped diff检查通过。此次未重跑完整库或相邻9项，不借 Qoder旧源码快照全库结果。前像、逐文件SHA、owned patch和日志保存在同目录。

只读 agent 审计：实际 target key 来源是 run_agent_target 的 prepared.route.url；Single 也须原 target URL，不能借 C 或 HTTP请求子路径。前端尚无 recovery invoke/types/可信原 receipt selection inventory；普通 workbench/confirm不能复用为恢复批准。UI 丢响应需首次批准前持久冻结同 requestId/选择/hash，关闭/重启不生成另一批准；可提供按原来源只读查询作为后续找回入口。本轮没有 UI 编辑或验收。

下一段继续专属唯一新 draft：持源 scan-control 与所有原退出 guards、重核原 source/model/新身份/策略/deadline；逐字保留获批 URL（普通 creator 的拆分/尾斜线处理不可直接用于批准范围）。冻结 approval→唯一新 scan/draft 的 immutable signed action，并在专属 fresh birth/十维绝对时限消费者闭合前拒绝普通 confirm 绕过。随后接真实新 Root/目标消费者、独立确认 UI及 human-recovery paid feed。当前仍 approved_pending_fresh_birth、executionAllowed=false、targetRequestsGranted=0，human-recovery not_ready，完整 REM-A01—A14 goal active；Asset Exposure Qoder独占，无自动提交/推送/重置/asset删除，Native JSON保留。

## 2026-10-08 专属批准到唯一待出生新草稿

真实后端功能：create_native_protection_recovery_draft 与 get_native_protection_recovery_draft 已注册。只接受原批准 requestId，重读原 HMAC 批准，持原 source scan-control、Immediate TX 与全历史原退出 guards，重建完全一致的 source/当前 choice/model/身份/策略/deadline 提案。服务器生成新的 scan UUID，逐字 INSERT 获批 literal URLs（尾斜线、逗号/分号 query 不拆分）；新 scan/context/targets/register_new mode 与唯一关系同 TX。原新草稿身份保持 unowned 的获批 draft scope，本轮不 bind，identitiesBound=false，出生消费者尚待绑定并核 exact new owner；不复制原任务身份。十维/方法/model/deadline 的完整原 scope 保存在签名批准及新关系中，尚未作为执行消费者生效。

出生来源真实性：opaque FreshProtectionRecoveryDraft 只能从当前 FreshModeDraft 与重新验证提案构造，原私有 Mode key 按独立 draft 域签完整批准/newscan/draftId/policyJson/pending 事实；没有冷 JSON 重签或 raw signer。新 immutable native_protection_recovery_drafts 一 approval/request/newscan/draft 一关系，完整 exact schema/TEMP shadow 检查及 rowid/业务键替换守卫；有限 direct INSERT writer 拒 trigger collateral/UPDATE/DELETE/DDL，仅 database_list 元数据读。register_new/freeze_new 前后重新安装或清除限权，最终原 SQL/费用/私有出生和源响应复验至 commit。签名 draft 的完整 policy 从获批 scope 重建，mode ID/值均一致；专属 NewWebAction kind=protection_recovery，固定 request/source/proposalHash/approvalId/literal target set。原恢复关系存在时 action 不允许换 kind 或混入 gap/closure 来源；公开哈希重算不能替代该绑定。action hash 将用于下一 Root 出生；当前不称 Root 已出生。

重复/读取与普通启动：同 request 返回原唯一 signed draft/action，READ_ONLY 重开不写；真正并发竞争 source-control 的一方可以返回忙碌，之后同 request 找回仍是同一 scan/关系，没有第二草稿。仅恢复 pending 来源读取，不保证当前可执行。prepare_web_startup_in 在普通 confirm/resume/retry 的状态/费用/attempt/files 写入前拒 protection_recovery_requires_dedicated_fresh_birth。状态严格 draft_pending_dedicated_fresh_birth、executionAllowed=false/targetRequestsGranted=0，无 SDK、新 Root、worker 或目标请求。

本片 localhost 证据：tests-final.log 4/4、0 ignored、57.82s、库存2858；包括实际原 Root/Mapper/Web SDK+WAF→生产暂停→批准→新 literal 草稿；只改变新6类允许表且所有既有行原样，其余应用表零写、GET保持1、SDK不增；重复/READ_ONLY 找回、原 action 验证、普通 Confirm/Resume/Retry 拒且全表不变；真实两个线程并发仅一个关系；来源 checkpoint 漂移、exact relation schema 异常拒，晚 target INSERT fault 完整回滚、撤 fault 后成功。冷 mode 改值、action 改成 gap 并重算公开hash+假gap关系、冲突来源、policyJson 与实际 context 同步冷改但无新签名均拒，原 guard DDL 恢复后不修复，两次读取零写、rollback 后原样。recursive_triggers=OFF 下 UPDATE/DELETE/显式 rowid OR REPLACE 拒。本片 mode-regression.log 21/21、0 ignored、19.43s；最终 immutable-final-test.log 1/1、14.83s 后又四项全重跑。Clippy 初次24 warnings/0 errors中2项本片新增 cmp_owned/type_complexity 已以命名 canonical 字节序列化与 StoredRow 类型别名修复，保留严格 JSON bytes；clippy-final.log lib22 warnings/0 errors、9.62s、exit0。十个 owned 源路径 SHA 冻结核验，四个叶 rustfmt --check 与 scoped diff检查通过。未运行本片完整库；Qoder旧源码快照全库仍非本片门禁。最初 tests-first.log 是错误工作目录导致测试文件未落盘的编译失败，保留；tests-second.log首两项2/2、40.63s，tests-provenance.log4/4、59.67s保留，最终证据不混用旧代码。前像/逐文件SHA/owned patch/日志在 /tmp/oviraptor-recovery-draft-20261008/。

只读消费者审计（待实施）：不能解除普通启动拒绝后直接 launch 原 pipeline。ordinary runtime/AgentExecutionPlan 会从当前 defaults、local full-power/static fixed budgets 扩大已批准额度；schema2 强制token三类相同、target=modelReq*4、wall=seconds*1000，不能表达独立十维或 Some(0)。须专属 signed born-plan/预算侧合同，模型 descriptor/唯一批准 proxy hash/方法/identity owner 完全一致，截止 min(出生+获批wall,deadlineUtc) 贯穿 queue/C/child/SDK/HTTP及watchdog，重启不续。一个 newScope 多 URL 不得给每个 Root 复制完整十维余额，需要可靠共享总 owner 或可证明总和不扩额的不可变分配。原 frontend recon 在 Root 出生前直接 HTTP/JS/CDP，须先建立实际授权/预算并让第一目标效应走真实 claim；browser=0 不启动 CDP。仅 task metadata 或只测单 GET 不能称完整消费者。当前 action reader 的 pristine draft 检查需与 immutable 历史验签分开，未来消费后以 exact signed birth receipt 验新attempt/root；绑定身份后用原凭据 material/expected new owner 复验，不能调用只支持 unowned 的旧 proposal verifier 或放宽为任意 owner。

下一段：实施专属首次 birth、不可变消费关系及真实 pipeline/十维总预算/method/absolute deadline 门禁，再接确认 UI（可信 signed receipt selection、requestId 持久化/丢ACK查回、原账与新任务分别展示）、human-recovery paid feed与实际闭环。真实多历史/Single、新身份绑定、所有crash/lostcommit矩阵、第三方/平台仍未验；human-recovery not_ready，完整 REM-A01—A14 goal active，不缩减或标 Master 完成。Asset Exposure Qoder独占；Native JSON/dirty/真实数据保留，不恢复Strix，无 reset/clean/自动 commit/push/asset删除。

## 2026-10-08 恢复草稿取消：保留签名历史并撤销未出生授权

真实后端功能：普通 cancel_sentinel_scan 在原 lifecycle control 与 Immediate TX 中识别专属恢复草稿，进入取消专用分支，不物理删除它。新的 immutable native_protection_recovery_cancellations 保存原 scan/draft/approval/request/hash 与独立域 HMAC，固定 authorityRevoked=true、executionAllowed=false、targetRequestsGranted=0；新 scan/targets 同 TX 标 cancelled。原停机、已付费用、批准、草稿关系、私有 Mode、action、身份均保留。opaque FreshProtectionRecoveryCancellation 只从原已验签、未消费草稿构造；限制 writer 只 direct INSERT 新取消记录与 UPDATE 新 scan/targets 的指定生命周期列，拒触发器旁写。晚故障全回滚，重复取消读取原签名且零写，不冷重签或修复。

历史读取与出生资格已分开：get_native_protection_recovery_draft 及 action reader 仍核 immutable literal targets/project/task/policy/mode/HMAC，但不把取消后的生命周期状态当成原创建事实损坏。已注册 get_native_protection_recovery_cancellation 为 READ_ONLY 原撤权事实读取。未来出生必须另外核 pristine snapshot、无旧执行、精确取消 schema，并拒 scan/approval/request 任一撤权记录。创建新草稿在新 UUID 分配前按 approvalId/requestId 检查撤权；即使草稿关系丢失或冷改回 draft/queued，也不能用原批准重发。关系损坏保留原现场，不自动补签。

本片测试证据：tests-first.log 首三项3/3、0 ignored、47.22s，compile2m22s、库存2861；tests-final.log 恢复相关15/15、0 ignored、217.99s、库存2862，其中批准7、草稿4、取消4。startup-regression.log 普通 Web 启动/取消15/15、0 ignored、12.67s。取消四项从实际 localhost 已付 Root/Mapper/Web SDK+WAF、生产暂停、批准、新草稿开始，验证旧全应用行原样、SDK/目标 GET 不增、签名历史找回、重复零写、冷生命周期恢复仍撤权、UPDATE/DELETE/rowid OR REPLACE 拒、恢复精确 DDL 后冷 JSON 原 HMAC 拒且不修复、晚 target 更新故障及触发器旁写全回滚、丢失 draft 关系后两次重发拒且全表零写。首轮 Clippy23 warnings/0 errors、22.40s；其中本片 unnecessary_cast 已去掉冗余 usize cast，cancel-final.log 最终逻辑四项4/4、0 ignored、57.95s、compile30.23s、库存2862；随后只有 rustfmt 排版变化。clippy-corrected.log 最终 lib22 warnings/0 errors、8.88s、exit0，无本片新增警告。四叶 rustfmt --check 通过；十个 owned 路径最终 SHA、逐文件前像与 owned.patch 已保存，最终 docs SHA 同目录。旧22警告包含 Qoder exposure及既有骨架/对账债，本片不接管其业务。

尚未完成：专属 fresh Root 出生、签名实际预算/模型/代理/方法/身份/deadline 消费者、首目标效应前预算授权、多 URL 总额、确认 UI及人工恢复监督 feed；库存仍 not_ready。普通 confirm/resume/retry 继续拒专属恢复草稿，不解除守卫绕过消费者。此片是生产取消/撤权和只读历史能力，localhost测试不是第三方/平台或实际恢复执行验收。完整 REM-A01—A14 goal active；Asset Exposure Qoder 独占，Native JSON/dirty/真实资产保留，无 reset/clean/自动 commit/push/asset删除，不恢复Strix。精确前像、源码 SHA、日志位于 /tmp/oviraptor-recovery-cancel-20261008/。

## 2026-10-08 原 requestId 找回签名取消记录

真实生产入口（只读历史能力）：get_native_protection_recovery_cancellation_by_request 已注册，允许客户端凭批准前持久化的原 UUID requestId 找回取消 ACK，精确取消 schema 检查后按唯一 request 查询 scan，再复用原 signed draft/取消 HMAC 与完整重复字段绑定核验，并额外核 receipt requestId。READ_ONLY 原连接 snapshot；取消前/不存在的合法 request 返回 None，坏 UUID 拒；不补行、重签、修复、重发或授予执行。客户端持久化 UUID/独立确认 UI 尚未实现，此后端入口不能称 UI 已闭合。

测试证据：tests.log 取消四项4/4、0 ignored、62.94s、compile1m00s、库存2862；实际 paid/WAF→暂停→批准→草稿→取消夹具新增核原 request 找回同一签名 receipt、取消前 None、未知合法 request None、坏 UUID 精确拒，raw READ_ONLY 重开后全应用表原样、SDK/目标 GET 不增长；其余原冷篡改/晚故障/触发器旁写/丢失草稿关系不重发回归同轮通过。clippy.log lib22 warnings/0 errors、10.08s、exit0，无取消叶新增 warning。两叶 rustfmt --check 与 scoped lib diff --check 通过；三 owned 路径前像、测试前后 SHA 冻结一致及 owned.patch 在 /tmp/oviraptor-recovery-cancel-lookup-20261008/。未跑本轮完整库，不将 Qoder 外部旧快照全库当作当前验收。

只读预算审计：root_definition 的 schema2 声明与 read 都要求三类 token 同值、target=modelReq*4，limits reserve_slot 依赖该 reader，clock.remaining 依赖各 Root 的原 created_at/硬 wall。需要专属新出生预算合同贯穿声明/读/实际 reserve/clock/运输，不能只增字段或解除普通启动拒绝；绝对批准 deadline、多 URL 总额以及独立十维/Some(0) 必须实际消费。此合同尚未实施，Root 出生/消费者/确认 UI/human-recovery feed 仍未完成。完整 REM-A01—A14 goal active，不缩减。Asset Exposure Qoder 独占，Native JSON与dirty保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 确认端原保护来源枚举（UI尚未接入）

真实生产入口（只读历史来源）：discover_native_protection_recovery_sources 已注册，raw READ_ONLY snapshot 按 scan 枚举原 agent_protection_response_receipts，最多256、257项拒不截断。读原 lease actor 后由 native_original_protection_response_on 逐项核原私有 Mode 出生、签名收据、exact字段/scope、原 HTTP claim/授权/证据与原物理行；重复末尾行复验，不接受日志推断/前端伪造 Root 列表。返回 exact selection、literal target、原状态/factHash，sourceVerified=true，但 readinessAssessed=false、approval=false、executionAllowed=false/targetRequestsGranted0。只证明原来源，不证明已关闭/已付全 scan 或当前可恢复；后续提案继续独立核全历史、原退出 guards/预算/model/identity/deadline。没有补签、修复或新 I/O。

测试证据：tests.log 1/1、0 ignored、4.51s、库存2863，实际 localhost paid Root/Mapper/Web SDK+WAF→生产暂停夹具；raw READ_ONLY 重开、同 exact selection/URL、明确非准入、未知 scan拒、TEMP影子拒、冷 signature置0后恢复精确guard仍两次拒且零修复，整个应用表与SDK/GET保持原样。clippy.log lib22 warnings/0 errors、43.15s、exit0。五 owned 路径前像/最终SHA与owned.patch在 /tmp/oviraptor-recovery-sources-20261008/。没有跑最新全库，也不借Qoder旧快照验收。

下一步确认UI：临时 frontend-drafts/ 的 contract.ts/api.ts/NativeProtectionRecovery.vue 仅草案，尚未进入工作树、类型检查、真实SFC测试或注册显示，不能计UI完成。草案拟先持久化原requestId与完整scope再批准，丢ACK只读找回/同UUID显式重试；需修复Vue proxy structuredClone问题，核十维名与严格完整scope/收据、异步跨scan fencing、持久化失败不发IPC、原取消与来源损坏不重发等真实组件回归后接 NativeRunStatus。草案尚有locale与渲染QA欠缺。专属出生/实际十维与模型代理方法身份deadline消费者、多URL总額与human-recovery feed仍未完成；预算独立合同不能只解除schema2/普通启动守卫。完整REM-A01—A14 goal active，不折算测试数为整体百分比。Asset Exposure Qoder独占，Native JSON/dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 独立恢复确认UI接入（尚不执行恢复）

真实前端接线：NativeRunStatus 挂载 NativeProtectionRecovery，调用独立 recovery/api.ts 注册生产 IPC。按需从原签名来源枚举选择 exact 原 scan/attempt/root/receipt；完整新 scope 表单含 literal URL每行不按query逗号拆分/尾斜线不变、方法、模型、代理hash、新身份草稿ID/scope、Single/Multi、深度、closure、skills/instruction、USD、绝对UTC与独立十维，首三维空为显式null、0为禁止，browser/write/upload固定0。提案由后端重新核全源，前端核完整selection/newScope/hash与非准入事实，scope改变同步清除提案/同意；独立明确新scope同意与原费用确认分开。批准后仅准备唯一待出生草稿、原签名取消/找回；没有普通Confirm/Resume/Retry或目标执行入口。

原操作找回：第一批准写IPC前将原UUID/selection/完整scope/hash存localStorage；存储失败不发送批准。ACK未知保留原操作，不自动换UUID，显式重试仍同UUID/原scope/原hash；按request读原批准/draft/取消，复验重复身份与execution=false，异常读/损坏缓存阻断后续写且不丢旧记录。generation/scan/open/disposed隔离晚异步响应；取消后保留原history/UUID，并拒再发草稿/批准。已修Vue reactive selection不能structuredClone的问题，改仅JSON纯选择复制；read-receipts失败置blocked避免沿旧状态继续写。后端仍负责原HMAC与实际准入，前端不重签、不扩大权限。

证据：build.log vue-tsc/Vite通过，Vite2.54s。新实际SFC脚本+编译模板在DOM宿主运行、脚本化IPC的五项通过：先存后写/丢ACK同UUID重试、存储失败零批准、批准→唯一草稿→签名取消→重复找回只读、坏缓存/错UUID回执拒不替换原操作、scope编辑撤同意与跨scan旧响应拒。rendered-verified-tests.log5/5；首次3项2/3的失败是测试harness每次render创建新组件导致remount，修固定组件身份后通过，原断言保留。模板宿主补addEventListener/options/getRootNode/Document，失败日志保留；最终模板与controller同时执行，不只有静态源码检查。正常test:ui 首893项840/53（新增子组件导致三个旧loader无法识别导入），只扩loader stub支持不删除业务断言；ui-final.log898/898、42.48s；最终模板变更后 ui-rendered-final.log898/898、0ignored、44.32s，新增测试纳入正常package门禁。九owned路径SHA一致、前像/owned.patch/scoped diff通过，日志在 /tmp/oviraptor-recovery-ui-20261008/。本片无Rust改动，未重跑库或借旧全库绿。

边界与继续：这是已接生产IPC的明确批准/待出生/取消确认UI；测试IPC为脚本化，不是安装App实际IPC/真实第三方/平台验收。专属Root出生、实际模型/代理/方法/身份/独立十维/USD/deadline消费者、多URL总额、人工恢复监督feed仍未完成。当前表单文案为中文，locale、视觉/窄屏/暗亮、身份/模型选择器、过期/改变scope后的显式新权力与操作历史管理还待完善，不称整体A10完成。旧批准不可冷重签或悄然改scope，新操作合同须另外实施。完整REM-A01—A14 goal active，不折算局部测试为总体百分比；Asset Exposure Qoder独占，Native JSON/dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 恢复确认回执阶段断言与真实适配器回归

真实前端守卫：approvalMatches 新增 oldScopeRenewed=false 必需，draftMatches 新增 oldScopeRenewed=false/identitiesBound=false 必需；待出生阶段不能显示旧scope已续或新身份已绑定。完整canonical scope/selection/request/hash与execution=false检查保留，不调整后端签名或放开普通启动。

新增实际生产 recovery/api.ts 适配器回归：注入 invoke 捕获精确八路生产命令，批准固定 consent、完整原scope/literal URLquery与原UUID/hash不变，四路批准/草稿/取消读取和创建均同原requestId，取消只指newscan，无普通Confirm/Resume/Retry/start。该证据验证真实适配器序列化，不是安装App真实IPC/HMAC或目标执行。实际SFC脚本+模板负向：原scope续期true的批准与identitiesBound=true的待出生草稿拒显示成功、保留原UUID；此前五项流程/ACK/存储/跨scan/损坏回归保留。

tests.log7/7、0ignored、537.42ms；build.log vue-tsc/Vite通过、Vite2.19s；正常 UI ui.log900/900、0ignored、99.50s。两owned路径前像、SHA与owned.patch在 /tmp/oviraptor-recovery-ui-contract-20261008/，本片没有Rust或Asset改动，无库/第三方/平台新验收。

只读预算审计深化：limits/contract.rs::frozen 先由 Native plan请求推导target，root_definition::read后除schema3财务迁移外仍要求token三类同映射与request字段匹配；initialize_root据此冻结agent_budget_limits，verify再次核同映射。专属十维恢复不能只加声明字段，须新出生权力/原scope binding贯穿reader/frozen/verify/reserve/clock与实际transport，并解决Some(0)/None、独立token、USD、绝对deadline和多URL共享总owner；保留旧schema2/3历史验证。出生/实际消费者、人为恢复feed仍未完成；下一优先是专属实际预算合同，不以本轮UI断言代替恢复执行。完整REM-A01—A14 goal active，Asset Exposure Qoder独占，Native JSON/dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 普通Root发布器拒绝恢复scope默认预算（相邻门禁待完成）

真实后端守卫：persist_frozen_web_execution_plan 在私有Mode读取/新Root/预算声明前检查 main 恢复draft关系、保留取消记录及原native_web_mode_actions.kind=protection_recovery；任一存在即 protection_recovery_requires_dedicated_root_budget。普通发布器不得为恢复scope签发schema2默认/推导额度，未来即使专属启动已有有效私有Mode也不能绕回此路径。关系丢失时保留action/撤权仍拒。普通新Root及历史schema2/3合同不改，不解除startup拒绝。

实际localhost证据：tests.log1/1、0ignored、15.46s、compile2m20s、库存2867；实际paid/WAF→生产暂停→独立批准→新两literal草稿；直接调用真实普通Rootpublisher的attempt1/2均在Mode/Root/预算/worker创建前拒，所有应用表原样、SDK/GET不增。只在可丢弃fixture DB删新draft关系并恢复精确guard后，原action仍拒且不修复，旧paid source保留。本片在依赖aws-lc-sys0.43编译阶段、应用Rust编译尚未启动时核正了测试trigger实际名protection_recovery_draft_no_delete；随后两源码SHA冻结一致。不是专属Root出生或实际恢复执行证明。

活等待：相邻 mode.log/main unified session1963（Cargo PID6332）仍等待build directory锁；当前Qoder Asset cargo test PID463为占锁者，已确认双方进程存活，不重启/终止。下轮先poll原session1963，终态后再串行Clippy。当前不能宣称相邻21项/Clippy或全库通过；旧22warning是历史结果。本片没有Asset业务改动，不动外部Cargo/lock升级。两owned路径SHA/前像/owned.patch与日志在 /tmp/oviraptor-recovery-root-route-20261008/；test叶rustfmt及publisher scoped diff通过。

下一核心：专属born预算/model/代理/方法/身份/UTC期限/多URL全scan总额合同与实际消费者；不能将普通Root拒绝守卫称已实现恢复。human-recovery feed仍not_ready。完整REM-A01—A14 goal active，Native JSON/dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 普通Root恢复路由守卫相邻门禁完成

终态：续接原 unified session1963、未重启，mode.log21/21、0ignored、18.63s、库存2867；此前占锁的外部Asset测试已释放，原进程继续编译依赖并实际运行。随后原串行 clippy.log/main session40739 exit0，lib19 warnings/0errors、58.26s；当前警告总数变化含外部源码变化，不将历史22→19归因本片。本片 publisher无新增警告。上一实际paid/WAF普通publisher恢复draft/丢失关系拒绝1/1、15.46s继续有效；两owned源码SHA仍与已测冻结一致，test叶rustfmt及scoped diff通过。日志保留 /tmp/oviraptor-recovery-root-route-20261008/。未跑当前全库，不借Qoder结果计Master完成。

出生路径只读核对：native_web_mode_plan 普通新Root只发行 NewRootModeDeclaration + schema2预算，已按恢复关系/action/取消早拒；store/financial_continuation.rs 的新Root插入只接受 FinancialContinuationAuthorization，先核verified_financial_source且固定multi，不能借schema3财务新Root评估合同授权恢复目标工作。专属新scope须独立opaque出生授权与新budget reader/frozen/verify/实际reserve/clock/transport，保持原schema2/3与NativeJSON。没有新增可执行恢复Root、预算消费者或human-recovery feed，本轮是完成真实已有生产守卫的相邻验证与出生路线核对。

下一优先仍专属实际出生/十维共享总额与美元/模型代理方法身份deadline消费者，不能持续用确认端或只读片替代恢复闭环；完整REM-A01—A14 goal active。Asset Exposure Qoder独占，dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。

## 2026-10-08 签名草稿绑定不可变全scan预算合同（尚无消费）

真实持久功能：新 FreshProtectionRecoveryDraft 的原私有 Mode HMAC 包含 scopeBudget；从原已验签approved新scope纯派生唯一ownerId、scopeHash、scan/draft/approval/request、exact literal URLs、十维名称及原Option额度、USD与原UTCdeadline。固定合同 one_shared_scan_total_across_all_roots、scan_birth_clock_clipped_by_original_deadline；不将三类token归一，不将Some(0)变None，不给每URL复制总额。新的 immutable native_protection_recovery_scope_budgets 与新scan/targets/mode/action/签名draft同Immediate TX，限制writer只direct INSERT并拒trigger collateral，readback/canonical/hash/重复列/签名内合同全匹配。普通Rootpublisher也检查此总额记录，损失草稿关系时不能绕回schema2默认预算。

读与历史：get/create draft 的原HMAC验签后核签名内合同与精确schema/TEMP shadow、main记录及完整canonical bytes；重复/READ_ONLY不写，取消保留总额和原签名历史。旧原签名draft缺scopeBudget字段时保留历史形状，读取不补行/重签/启动clock；尚无原旧V1 signed样本正向，不称历史迁移已实测。新记录在签名内字段缺失/数据损坏时不会通过消费者，原公开hash重算不能替代原HMAC。此句为合同边界，真实执行消费者尚未实施。

明确边界：这是一份待出生总额的签名约束和实际持久记录，executionAllowed=false/targetRequestsGranted0；allocationPolicy与wallTimePolicy目前不是动态预约/释放、费用消费、多Root竞争守恒或运行时截止证明。没有Root、C、worker、SDK或目标 grant；USD仍原f64 JSON数值，尚无美元reserve/消费。下一实际消费者须签名专属born contract、可信pricing/未知费用处理、全scan共同ledger/clock与实际SDK/HTTP两阶段，不能以本片持久合同当作完整A02或恢复执行。

本片证据：tests.log新增2/2、0ignored、28.16s、compile1m36s、库存2869；实际localhost原paid Root/Mapper/Web SDK+WAF→生产暂停→批准→新两literal草稿，独立input0/cached null/output37及USD3.25/deadline原样、全scan仅一合同、旧全表行保留、重复/READ_ONLY零写、取消后原合同历史可读、SDK/GET不增。recursive_triggers OFF下UPDATE/DELETE/显式rowid OR REPLACE拒；可丢弃TX冷改target额度并重算公开contract_hash、恢复原DDL后二次精确拒且不修复，rollback恢复。recovery.log相关19/19、0ignored、257.43s，含批准7/草稿5/取消4/来源1/总额2；原晚draft关系写故障现在也整笔回滚新总额记录。mode.log21/21、0ignored、18.73s；clippy.log lib19warnings/0errors、9.41s，无本片新增警告。九owned源码SHA冻结一致、四叶rustfmt/scoped diff通过，前像/owned.patch/日志在 /tmp/oviraptor-recovery-scope-budget-20261008/。未跑当前全库，不借Qoder旧或Asset验收计入。

完整REM-A01—A14 goal active。专属出生/实际消费者/human-recovery feed仍未完成；后续核心仍实际执行链，不用更多只读或元数据替代。Asset Exposure Qoder独占，Native JSON/dirty/真实资产保留，无reset/clean/自动commit/push/asset删除，不恢复Strix。


## 2026-10-08 恢复scope拒绝通用frontend首效应路径

真实生产守卫：launch_frontend_recon_producer 在 ScanWorkerOwner::claim、通道/线程、目录/日志和 HTTP/JS/CDP 之前，以 raw READ_ONLY 检查恢复draft/cancellation/scopeBudget/保留action；命中即 protection_recovery_requires_dedicated_frontend_budget。与普通Root发布器共用只读路由分类，不授新权限、不默认分配预算。解决通用采集早于Root预算发布的恢复入口边界；不是专属执行消费者。

运行证据：原实际localhost paid/WAF→生产暂停→批准→新草稿夹具，真实 producer 调用提前拒，全部应用表/SDK/目标GET不增长，工作目录未创建；可丢弃fixture删除draft关系并恢复原guard后保留记录仍拒且零修复。tests.log1/1、0ignored、14.27s、compile1m01s；相邻frontend.log2/2（URL watchdog/身份共享探索预算），不是完整frontend集成验收；clippy.log lib19warnings/0errors、7.23s。三owned源码SHA冻结一致，scoped diff --check通过；前像/日志/patch在 /tmp/oviraptor-recovery-frontend-route-20261008/。当前完整Rust库未重跑。

边界：仍无恢复Root/C/实际预算预约释放及费用消费、人为恢复监督feed。下一核心是专属opaque出生、全scan共享ledger与clock、模型/身份/代理/方法/USD/绝对deadline实际消费者，不以更多守卫或元数据代替执行闭环。完整REM-A01—A14 goal active；Asset Exposure Qoder独占，无Asset业务编辑、reset/clean/自动commit/push/真实数据删除，Native JSON保留。


## 2026-10-08 当前完整Rust主库门禁运行中（非通过结论）

实际门禁：主控启动 nice -n 15 cargo test -j 1 --lib -- --test-threads=1，日志 /tmp/oviraptor-master-full-gate-20261008/tests.log；unified session13786，最后实时核对 Cargo PID42392 / test binary PID42412 均存活，选择2869项，compile0.71s。下轮续poll原session，不因观察超时重启。尚无终态，不将阶段输出或旧2624/1、Qoder快照当作当前全库绿；全库包含Asset测试但不将其功能计入主控Master验收。

源码冻结：本轮没有Rust修改；记录1366个Rust文件的运行中观察SHA到 source-observed-after-start.json，该清单采于测试启动后，不能冒称开跑前全树冻结证明。结束须复核变化与构建快照关系；外部变化另列。测试存活期间继续只读分析和文档，不修改 owned Rust。

消费者核对：root_definition schema2要求三类token同映射及按请求推导target；limits/contract初始化与读验证使用该映射，各Root独立clock，不满足获批独立十维/多URL共享全scan总额。当前源码未找到可信USD计价/美元预约消费实现；maxBudgetUsd的JSON持久化不能充当美元守卫。专属出生需贯穿私有Mode授权、Root插入、独立预算声明/reader、共享ledger/clock与实际SDK/HTTP，当前仍未完成。此段为只读架构结论，非新功能或闭环验收。

完整REM-A01—A14 goal active；先收当前全库终态并处理主控实际回归债，随后推进专属出生/共享预算消费者，不能以门禁或诊断代替执行实现。Asset Exposure Qoder独占，Native JSON/未提交改动/真实资产保留，无reset/clean/自动commit/push/asset删除。
