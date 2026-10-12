## 2026-10-08 当前源码编译恢复与主控验收终态（Master 未完成）

- 原 Exposure 编译阻碍已由 Qoder 在当前源码修复；主控实测 npm build、主库测试编译、应用 cargo build 全 exit 0，应用普通构建 18 warnings（不是 Clippy）。
- 主控补齐 14 个 IPC 注册与七组暴露面测试脚本。Master 六项待验加三邻近 9/9，Exposure Rust 94/94、前端 59/59，均本地定向证据；未整库/第三方/安装 App 验收。
- 详细状态 docs/NEST_CODEX_STATUS_AND_BUILD_2026-10-08.md；Qoder 下一轮 docs/NEST_QODER_NEXT_STEPS_2026-10-08.md。旧 blocked 编译原因已解除，完整 Master 尚未完成。证据 `/tmp/oviraptor-codex-status-20261008/`。未自动提交、未覆盖 Exposure 业务、未删资产。

## 2026-10-08 Master 编译阻碍确认（goal blocked）

- 重验 session 52854 已 exit 101，测试未运行；本轮主控 String/&str 错误不再出现，剩余 8 个编译错误全部属于 Qoder Exposure export.rs/tests_e4.rs，详情见下方及 NEST_QODER_INTEGRATION_BLOCKERS_2026-10-08.md。
- 同一外部编译阻碍已连续多轮确认。六项待验不能运行；当前完整 UI 902/902 已完成。继续生产监督容量/专用恢复出生需要可编译验证基线，遵守不修改 Qoder 业务文件的分工，目标 blocked 等待外部修复，不缩减原 REM-A01–A14 目标。
- 恢复后先重验六项与邻近回归，再继续动态监督容量/专用 Recovery birth；禁止旧二进制伪证、忽略测试、削弱守卫。未提交源码及真实资产完整保留。

## 2026-10-08 编译阻碍终态与主控类型修正（goal active）

- session 20795 exit 101，测试未运行。本轮主控 request_accounting.rs:209 String/&str 断言错误已改为 detail().as_str()，已启动真实重验 session 52854；仍 live，不能报告主控类型验收通过。
- 外部 Exposure 当前编译错误：export.rs:164 `.map(str::to_string())`；tests_e4.rs:111 struct update 尾逗号；tests_e4.rs:15 page_findings 导入不存在；tests_e4.rs:233/235 对日志三元组调用 contains 及 Display。Qoder 修复指示：传函数而非调用、修正 struct update、使用实际分页 API、按实际日志字段检查正文并用 Debug 或正文格式化。主控未修改 Exposure。
- 六项 Rust 待验不变。日志 `/tmp/oviraptor-master-review-stop-revision-20261008/blocker-recheck.log` 与 blocker-owned-fix.log。完整 goal active，先收取 52854 终态，不重复启动。

## 2026-10-08 请求记账夹具保留真实检查点修正（goal active）

- 修正前轮测试中 accounting_native fresh state 覆盖原 Web 检查点的问题：读取真实原检查点，仅写人工 target_requests=3，再读取并归一化该计数后比较完整 as_json。原 evidence/plan hash、模型费用、进度及其他状态不能被测试夹具替换。
- 仅测试代码，语法解析与 diff check 通过；运行待验，Exposure export.rs:164 编译错误仍在。六项待验清单不变，完整 goal active。证据 `/tmp/oviraptor-master-accounting-exit-20261008/`。

## 2026-10-08 当前完整 test:ui 验证终态（goal active）

- npm run test:ui exit 0：902 passed / 0 failed / 0 skipped，77.71s。覆盖当前脚本聊天、历史与事件、授权界面、状态、请求审核/费用、恢复/延续、日志及结果页；本地组件/合同/生产适配器，IPC 模拟，非后端新 Root 实际恢复或第三方验收。
- 证据、脚本测试文件 SHA 与日志 `/tmp/oviraptor-master-ui-full-current-20261008/`。没有测试失败被删除/忽略。此次没有生产代码改动。
- Rust 六项待验与 Qoder Exposure export.rs:164 编译阻碍仍在。REM-A03 多事实监督动态容量及 dedicated recovery birth 尚未实现；完整 goal active，不将 UI 绿色当 Master 完成。

## 2026-10-08 REM-A03 多事实监督容量现场复核（goal active）

- 复核固定保留 31000/2 与逐物理 request_record 付费监督之间的容量缺口，形成 docs/NEST_MASTER_SUPERVISION_CAPACITY_WORKLIST_2026-10-08.md。不是 Web 已结算费用丢失，也不以合并物理事实/预算放宽换绿。
- 本轮只读生产诊断；动态准入尚未实现，六项 Rust 修改未验，Exposure 编译阻碍未消除。完整 Master goal active。

## 2026-10-08 Recovery 专用出生源码边界复核（goal active）

- 五个待验 Rust 文件 rustfmt stdout 语法解析 exit 0，未改写源文件；不等于类型检查/测试通过。
- 生产恢复仍为签名 pending 共享 scope 合同，无执行授权；三个普通启动入口均拒绝。已形成 docs/NEST_MASTER_RECOVERY_BIRTH_WORKLIST_2026-10-08.md 的实际源码锚点/实施及正负向验收清单，尚未实现专用出生。
- Rust 六项待验和 Exposure export.rs:164 编译阻碍保留；Master 完整 goal active。

## 2026-10-08 请求记账终态原 Web 退出补齐待验（goal active）

- request_accounting_terminal_snapshot 测试在冻结前限定一次 Web turn，实际调用 run_native_agent，要求一个原 Web SDK，再保持原 terminal closure/重复只读消费者/Root 不回填/4=3+1 子归属断言。生产记账未改。
- 明确限制：Executor 的 3 次目标请求仍由 accounting_native 人工 fixture 写入，仅模型退出走真实 localhost SDK；不是 HTTP 全链路验收。diff check 通过，运行未验证。证据 `/tmp/oviraptor-master-accounting-exit-20261008/`。
- 当前 Rust 待验六项（前五项加 request_accounting terminal）。Exposure export.rs:164 编译错误仍在；未覆盖 Qoder 业务。前轮 UI 118/118 只证明组件/模拟 IPC。
- 全范围 Master goal active，真实共享十维财务、恢复 Root 执行及最终全量验收尚未完成。

## 2026-10-08 当前恢复/对账/延续 UI 验证终态（goal active）

- 七组当前本地 UI 测试 118 passed / 0 failed / 0 skipped，6.58s；涵盖 ProtectionRecovery、Root reconciliation、continuation preview/issuance/discovery、NativeRunStatus、source pause recovery。组件/适配器实测，IPC 响应模拟，不能证明后端新 Root 出生、共享财务或恢复执行。
- 证据及测试源 SHA `/tmp/oviraptor-master-ui-recovery-current-20261008/`。生产功能本轮未修改，测试中 executionAllowed=false/automaticResumeAllowed=false 不等于执行已恢复。
- Rust 五项待验清单仍保留；Qoder Exposure export.rs:164 编译错误现场仍在，未修改其业务文件。完整 REM-A01–A14 goal active，未全量/第三方/最终交付验收。

## 2026-10-08 审核封印与只读 Gap 原退出补齐待验（goal active）

- gap.rs 两项原失败测试补齐真实 Web 单 turn 及 20/1 结算：审核不足只启动一个只读 Gap 并保留原 evidence 冲突拒绝/ACK 重放；审核封印缺失或变更保留原拒绝码/零任务，并检查零新增 SDK。60000/20 原上限不变，生产守卫未改。
- diff check 通过；未运行，当前 Exposure export.rs:164 编译错误仍在。证据 `/tmp/oviraptor-master-gap-seal-exit-20261008/`。
- 当前待验五项：insufficient revision、executor mailbox failure、failed gap reentry、reviewer insufficiency gap、missing/changed review seal。request_accounting terminal 测试仍需原 Web 退出改造，当前只核对源码，不视为功能完成。
- 本轮为测试证据补齐，完整 goal active，未宣称共享财务/恢复执行/全量验收完成。保留未提交改动和真实资产，未改 Qoder Exposure 业务。

## 2026-10-08 failed-gap 重入原退出补齐待验（goal active）

- failed_gap_round 测试补入冻结前一次 turn 的真实 Web SDK，结算后要求 20 tokens / 1 request，再模拟只读 Investigator 崩溃。原 specialist_received_receipt_required 拒绝、交付快照不变、任务仍仅 1 个断言保留，并增加重入零新 SDK 断言。
- 本轮只修改 gap.rs 测试，相关四个测试/fixture 文件 diff check 通过。运行验收未完成：Qoder Exposure export.rs:164 的 `.map(str::to_string())` 编译错误仍在，未改其业务文件。证据 `/tmp/oviraptor-master-gap-failed-reentry-20261008/`。
- 待编译恢复需重验 revision、executor_mailbox、failed_gap 三项及邻近回归；不得将源修改或旧二进制结果算当前通过。完整 Master goal active，尚未完成生产恢复执行和共享十维预算。

## 2026-10-08 Executor 邮箱故障真实退出补齐待验（goal active）

- scheduler.rs 的 executor_mailbox_failure 测试改为冻结前限一次 Web turn 的原 Root，真实 SDK 退出后注入 execution_result 邮箱故障；原 60000/20 上限与 failed/lane=0/reserved_requests=0 断言保留。生产守卫未修改，单文件 diff check 通过。
- 编译阻碍仍为 Qoder Exposure export.rs:164 `.map(str::to_string())`，本项未运行、未宣称通过。源码与备份证据 `/tmp/oviraptor-master-executor-mailbox-20261008/`。前轮人工停止 1/1 已通过；修订重验仍待编译恢复。
- 完整 REM-A01–A14 goal active；恢复执行、共享十维预算和全量验收仍未完成。资产及 Qoder Exposure 业务文件未改动。

## 2026-10-08 修订重验编译终态（goal active）

- session 59437 已 exit 101，测试未运行：Exposure export.rs:164 `.map(str::to_string())` 引发 E0061/E0277/E0599。该业务文件归 Qoder，主控未修改。修复指示：改为 `.map(str::to_string)` 或等价闭包，并完成证据包导出回归。
- 人工停止测试仍为本轮实测 1/1 通过；修订测试首轮暴露覆盖计数混淆，已修正但尚未重验通过。不能使用旧二进制验证当前源码改动。
- 完整 Master goal active；仅测试证据推进，无全量通过或恢复执行闭环声明。证据 `/tmp/oviraptor-master-review-stop-revision-20261008/`。

## 2026-10-08 人工停止验证终态与修订覆盖计数复核（goal active）

- 人工执行停止及重复审核：1 passed / 0 failed，5.23s。原 Web 真实退出后，Reviewer 记录结论但保留 REQUEST_RECONCILIATION_REQUIRED / EXECUTION_AUTHORIZATION_DENIED，Gap 不自动启动。
- 修订测试首轮真实退出验证失败：旧断言把 Native Web 的覆盖行计为候选发布。已分别断言覆盖记录 1 条、非覆盖发布 0 条；重复 Reviewer、原 evidence 冲突及未验证 artifact 不能开启新修订等断言保留。重验 session 59437 尚未终态，不报告通过。
- 仅修改测试 fixture / review_gate；生产功能、恢复新 Root、共享十维预算、全量及第三方验收尚未完成。证据 `/tmp/oviraptor-master-review-stop-revision-20261008/`。完整 goal active。

## 2026-10-08 coverage-only 验证终态及人工停止/修订接续（goal active）

- coverage-only 单项本地回归已终态通过：1 passed / 0 failed，2.34s，原 Web 实际退出并结算 20 tokens / 1 request；没有制造 Reviewer、审核请求或确认结论。证据 `/tmp/oviraptor-master-coverage-review-20261008/`。
- 人工停止与 insufficient revision 两项正在补齐真实 Web 退出测试证据：冻结前加入原 evidence，保持原 60000/20、80000/30 上限，不修改生产守卫；尚未验证通过。
- 此轮是本地回归证据，未完成新恢复 Root、共享十维财务或第三方/全量验收。全范围 Master goal 保持 active，Exposure 业务仍由 Qoder 负责。

## 2026-10-08 coverage-only 原退出修正待验（goal active）

本轮仅修改 multi_agent/tests/review_gate.rs 的coverage_only_result_never_fabricates_a_reviewer_or_a_confirmed_decision。保留原60,000 tokens/20请求合同、无candidate时Reviewer/request/decision三者为零断言；出生前冻结一轮，实际原Web请求/退出替代虚构outcome，并核对结清20 tokens/1请求。没有修改生产守卫或自动在prepare中执行SDK。

本片diff检查通过，原像/日志在 `/tmp/oviraptor-master-coverage-review-20261008/`。定向进程session72732已确认live、等待构建锁，尚无编译/测试终态；下一轮接续该进程，不重启。原20项定向失败地图仍按当时范围保留，未全库/Clippy/安装App验收。完整goal active，Native JSON、资产、未提交改动和Qoder白名单保护不变。

## 2026-10-08 unsupported-trigger 原退出验证终态（goal active）

上一轮session2757已exit0，不重启。定向1/1、2.45秒、0忽略：unproved-human-directive仍root_trigger_not_enabled，拒绝后仅原三次SDK/零新增Root请求/应用行不变；随后实际原Web一次SDK/20 tokens结清退出。主控仅改agent_tests_root_rust_policy.rs的该夹具，未改生产授权/预算守卫。原像/日志/result.json在 `/tmp/oviraptor-master-unsupported-trigger-20261008/`，本片diff检查通过。

原20项定向失败地图是其当时编译证据，不能直接减一推算当前全库。完整goal active；下一片进入review_gate三条缺真实退出夹具，保留人工stop不自动解除/无candidate不造Reviewer/新revision规则，再继续动态监督容量和恢复预算生产闭环。未重跑全库/Clippy/安装App；Native JSON、未提交改动、真实资产与Qoder白名单保护不变。

## 2026-10-08 unsupported-trigger 原退出修正待验（goal active）

仅修改 agent_tests_root_rust_policy.rs 的unsupported-trigger夹具：出生前冻结一轮，拒绝断言之后实际执行Web并结清20 tokens/1请求，替代虚构outcome。原root_trigger_not_enabled、三次既有SDK/零新增Root请求、应用行不变断言保留；无生产守卫改动。定向验证当前正在编译，session2757仍live，证据目录 `/tmp/oviraptor-master-unsupported-trigger-20261008/`。尚无通过结果，不重启该进程；下一轮接续收取终态。完整goal active，原20项定向失败地图与全库未验限制保持，Native JSON/资产/Qoder边界/未提交改动保护不变。

## 2026-10-08 原63项失败定向复验终态（goal active）

固定当前编译程序副本逐项--exact复验原失败名单：42通过/20失败/1旧名变更；两个v2语义/新协议拒绝替代用例另行通过。没有删除或ignore失败，不从旧名缺失推算通过，也不把只读v2校验当历史执行授权。原全库2806/63仍保留，此处不是当前全库失败数或Master百分比。

20项真实失败分组：原Web退出证明14、显式付费恢复合同2、多事实Root监督容量2、paid SDK检查点1、旧gap请求数1。详细逐项panic、日志与下一步依据见 NEST_MASTER_ORIGINAL_FAILURE_RECHECK_2026-10-08.md；证据目录 `/tmp/oviraptor-master-original-failures-20261008/` 保存原名单、固定binary/sourceSHA、result/summary/failure-map与替代结果。所有进程已终态，不重启已结束runner。部分测试读实时源，漂移名单单列；库存其余用例与全树仍未验。

完整REM-A01—A14 goal active，按新失败地图继续真实执行与恢复合同/容量修复；不缩减scope、不自动提交/推送/清理资产。Native JSON与Qoder暴露面白名单保护保持不变。

## 2026-10-08 Root 原终态证明体积修复（goal active）

本轮仅生产修改 budget/root/model/tick/terminal_proof.rs。复现原三次ReAct调用测试在第三次前root_tick_terminal_proof_too_large：原终态证明再次嵌入并转义已保存的完整请求。新捕获采用version2请求证明，绑定原物理rowid、created与完整canonical fact的stable_hash；完整请求仍在原不可变receipt行，verify_request/原账单/物理费用/源一致性验证全部保留，1MiB上限不提高。读者按原version1重建旧完整证明、按version2重建新hash证明；不读时升级或改写历史Root，不据此宣称历史Root执行迁移通过。

原失败保留baseline.log；定向三请求1/1、6.38秒，ReAct整组11/11、21.98秒，原终态证明负向6/6、35秒，各exit0/0忽略。验证三次实际收费/冻结上限停止/零SDK重放、原角色/工具约束、未知费用不进入第二轮、终态证明/费用物理行篡改拒绝和写入故障不重试。本片diff检查通过；Clippy --lib exit0、22 warnings/0 errors、1m14s，本片文件无警告，不宣称全树无警告。原像、日志/result.json在 `/tmp/oviraptor-master-local-react-20261008/`。

没有新增恢复专属出生、共享多Root/十维/USD执行消费者；动态多事实监督容量成功路径仍未完成。当前全库/安装App/第三方未重验，完整REM-A01—A14 goal active，原2806/63历史整库结果保留。Native JSON、未提交改动、真实资产与Qoder独占暴露面保护不变。

## 2026-10-08 原人工动作回执与关闭矩阵（goal active）

本轮仅修改 agent_tests_directive_closure_fresh.rs 的review_success_is_not_an_applied_action_receipt：取消虚构bounded outcome，执行已有出生前冻结一轮的实际Web，断言一次SDK及结清20 tokens/1请求，再审核和关闭Root。审核返回原实际outcome不代表人工动作完成；没有原动作回执的applied投影仍转deferred/directive_task_ended_execution_receipt_missing，原关闭断言保持。没有修改生产授权或新增恢复执行功能。

定向1/1、2.49秒；directive_closure全组21/21、22.17秒、0忽略、exit0，覆盖真实provider失败、结算/关闭故障、未决费用、取消、终态工具/续租拒绝和动作回执边界。本片diff检查通过，原像/日志/result.json在 `/tmp/oviraptor-master-directive-receipt-20261008/`。源码编译库存2894，仅此过滤组通过；未重跑全库/Clippy或安装App。动态多物理事实Root监督保留额与成功入口仍未完成，不能从矩阵绿宣布完整Master或人类恢复闭环。完整REM-A01—A14 goal active；历史2806/63保留，Native JSON、未提交改动、真实资产与Qoder白名单保持保护。

## 2026-10-08 真实 Web 结算写入故障回归通过（goal active）

本轮仅改 agent_tests_directive_closure_fresh.rs：共享直调夹具在Root出生前冻结一轮、provider识别原Web并保留真实seen；仅结算故障调用者实际执行Web后注入故障，不隐藏执行在prepare中。20 tokens/1请求由原始lifetime回执在事务快照验证，运行行派生计数在coarse结算前为零不能替代它。原事务必需守卫保留，未伪造退出或用量。

结算故障1/1、2.57秒，邻近清理原子性/旧fence拒绝1/1、3.20秒，exit0。验证真实Web已付费退出后coarse结算写入被拒，paused/能力撤销/原预留/通道/未结清账保留；重复清理不退款。此前派生行零值及缺事务两类失败日志均保留。原像、全部日志/result.json在 `/tmp/oviraptor-master-settlement-fixture-20261008/`，本片diff检查通过。未新增生产功能或重跑全库/Clippy/安装App。

同步动态容量只读结论：bootstrap原supervisionRequestFloor=1，而new-evidence退出hook逐条支付每个物理HTTP事实。固定一次保留额不足以证明多事实闭环，不能把真实scope缩成一个节点或删监督。此成功路径仍待生产原合同/动态义务及余额处理，匿名对照成功用例尚未通过。完整REM-A01—A14 goal active，原历史2806/63不代表当前全库；Native JSON、未提交改动、真实资产和Qoder独占暴露面保护不变。

## 2026-10-08 真实后续派发阶段报告修复（goal active）

主控生产修改multi_agent/execution.rs与finalization.rs：在原Web结清之后的新证据/ClientSide/InputParser失败分别保留executor_followup_failed阶段与原cause，避免全部显示为executor_settlement_failed。费用/授权/余额/停止分类不放宽；未决费用cause仍映射REQUEST_RECONCILIATION_REQUIRED。匿名对照测试文件新增实际入口负向，不将后续拒绝改成成功。

首次仅ClientSide阶段的0/1证明错误发生更早，已纠正为new_evidence。最终真实阶段用例1/1、10.13秒、exit0：新证据监督容量拒绝，原Web terminal/12请求/1680 tokens/budget_settled_at保留，四次站点请求真实，无ClientSide SDK。原成功用例仍待动态容量修复，不能宣称ClientSide入口通过。邻近directive_closure_executor_settlement_failure_retains_unsettled_budget当前0/1（1.49秒），修改前保存二进制也同一fixture_settlement_failure断言0/1（1.78秒）；记录既有回归，不隐藏失败。证据/result.json在 `/tmp/oviraptor-master-followup-stage-20261008/`，本片diff检查通过。

本片没有新增共享预算消费者或专属恢复出生；当前整库/Clippy/安装App未重验。下一步处理新证据监督原容量与结算故障夹具，原完整REM-A01—A14 goal active；历史2806/63保留，不用局部通过推算完成度。Native JSON、未提交改动、真实资产和Qoder白名单保护不变。

## 2026-10-08 原请求额度诊断纠正：退出后容量不足（goal active）

本轮仅向原匿名对照测试失败断言补角色/原硬请求额度/实际请求/状态诊断，无生产改动。定向仍0/1、9.85秒、exit101（当前编译库存2893）。原Root硬请求22、全角色SDK21；Mapper/Identity/External各1、Web12且均terminal，Root自身费用仍走独立账，不用coordinator.used_requests=0伪称免费。原Web每轮模型入场检查assignment余额，故前文“派发超用到结算才报”尚未证明，予以纠正。

源码结清顺序先complete_target_child_delivery，再new-evidence/ClientSide/InputParser。SDK数与终态将问题指向退出后新增工作容量不足：剩余总请求约1，ClientSide还要求Reviewer保留额，不能把后续拒绝的executor_settlement_failed标签直接当Web超用。没有删除ClientSide实际调用或费用断言、增加预算、伪造配置证据、改写冻结计划。待验夹具仍不通过；下一步处理真实动态容量/可选后续工作与准确阶段终态，不放宽授权或预算。

原日志original-usage-tests.log、结构结果original-usage-result.json在 `/tmp/oviraptor-master-client-entry-20261008/`。完整goal active；当前全库/Clippy/安装App未验，历史2806/63仍保留。Native JSON、真实资产、他人未提交改动与Qoder白名单保持保护。

## 2026-10-08 Web 收口守卫与冻结计划验证（goal active）

当前真实源码显示finish_target不会接受无证据的not_applicable，未尝试覆盖族/待访问队列仍存在时拒绝提前完成。匿名对照夹具反复发送同一closing_ledger，最终耗到12次请求并暴露派发额度/结算差异。主控尝试在原入口前声明两轮范围，但实际入口重建计划与此前冻结计划不一致，准确拒绝agent_attempt_plan_frozen_conflict，HTTP0/SDK0。此候选改写已精确撤回，未改生产守卫/预算或原被冻结账；上轮真实配置HTTP与诊断修改仍待验。

失败日志bounded-tests.log和frozen-bound-result.json在 `/tmp/oviraptor-master-client-entry-20261008/`。本轮结论是守卫证据与进一步定位，不是通过或新生产功能。下一步必须核对Web实际派发请求额度与执行器停止/扩展条件，再修复实际闭环；不通过放宽收口或改写计划换绿。全库未验、完整goal active；Native JSON、真实资产、未提交改动和Qoder暴露面白名单保持保护。

## 2026-10-08 ClientSide 入口实际失败定位（goal active）

暴露面原四项编译阻挡在本次编译中已解除，主控未代改其业务。ClientSide待验夹具复跑仍失败；增加终态/SDK数诊断后确认先前“缺实际ClientSide SDK”遮蔽更早的Web结束失败：Web达到12次硬请求上限、1680 tokens，整体21 SDK；executor_settlement_failed:child_budget_reservation_exceeded_or_stale。诊断测试0/1、8.59秒、exit101，不是通过或新功能。

本轮只为 agent_tests_multi_anonymous_control_original.rs 的原角色断言增加执行终态/调用数诊断，保留上轮实际匿名配置请求的待验修改；不加预算、不删角色/费用断言、不放宽生产守卫。日志/diagnostic-result.json在 `/tmp/oviraptor-master-client-entry-20261008/`。下一步核对脚本化finish_target回复与真实关闭账为何反复被拒，再验ClientSide与撤权分支。原“只是配置HTTP记录缺失”的归因不充分，以本次真实终态覆盖。全库/Clippy/安装App未重验；完整goal active，Native JSON、未提交改动、真实资产与Qoder暴露面白名单保护保持不变。

## 2026-10-08 恢复 UI 空收据身份拒绝（goal active）

上一轮ClientSide夹具修正仍待验；本轮只读复核暴露面阻挡依然存在，未重复启动已知失败编译，也未代改Qoder代码。独立推进恢复组件：发现approvalMatches/draftMatches接受空approvalId/draftId，可能将不完整回执显示为确认。主控修改 recovery/contract.ts 拒绝这两个空身份；新增真实Vue组件测试确认空批准不能调用createDraft、空草稿不能调用cancel，原requestId与保存操作均保留。这是生产UI收据校验修复，不是恢复专属出生/共享预算执行功能。

变更前恢复/状态/对账/源码暂停组件76/76；新增负向后的最终78/78、0跳过、exit0、4.14秒，本片diff检查通过。测试执行真实组件但IPC模拟，未代表安装App或后端签名/新Root实际执行；原像/日志/result.json在 `/tmp/oviraptor-master-recovery-ui-20261008/`。Rust全库/Clippy未重跑，ClientSide入口待编译依赖恢复再验。完整REM-A01—A14 goal active，原2806/63历史整库结果保留；不恢复Strix、不改Native JSON、暴露面业务、真实资产或他人未提交改动，不提交/推送。

## 2026-10-08 ClientSide 实际入口回归修正待验（goal active）

原失败由上一片已通过的编译程序再次复现：fresh_multi_authenticated_anonymous_control_actual_entry_preserves_account_metadata_and_fees 缺实际ClientSide SDK。只读生产核对发现ClientSide只接受实际Broker HTTP request_record，身份对比本身不是该记录。主控仅修改 agent_tests_multi_anonymous_control_original.rs 的成功夹具：身份对比之外增加一条实际匿名 replay_http 配置采集，保留要求ClientSide真实调用的断言；目标请求/站点总账由3→4。撤权分支不加请求，原零后续SDK/两次目标请求断言不变。未增加伪造配置节点或放宽生产守卫。

修改后编译被Qoder暴露面四项错误阻挡，exit101，尚未执行测试，不能宣布修正通过。当前cache函数随后已出现但未复编；OptionalExtension及两处cursor错误转换仍待白名单所有者修复。给Qoder的具体集成要求见 NEST_QODER_INTEGRATION_BLOCKERS_2026-10-08.md。原像、基线失败、编译日志/input/result.json在 `/tmp/oviraptor-master-client-entry-20261008/`。本片源SHA未漂移，完整goal active；编译依赖恢复后继续此原测试与撤权邻近验收，不据阻挡宣称Master完成或暂停。不改暴露面业务、不重置/提交/清理资产，Native JSON与未提交改动保护保持不变。

## 2026-10-08 四角色独立 SDK/邮箱与 transport 矩阵（goal active）

本轮仅修改 agent_tests_specialist_transport.rs 的四角色链测试。出生前冻结一轮 Web，原provider真实执行/退出后再进入独立审核与提案；移除虚构finish outcome。Mapper/Identity/Reviewer/Investigator加原Web各20 tokens/1请求，子账合计100 tokens/5请求，六次Root SDK独立记账，总计11次SDK请求。身份秘密不泄露、只读角色不获工具、Root工具注册与原请求逐轮对齐、两条gap邮箱确认断言保留。

定向1/1、4.20秒；复制该编译的测试程序并记录SHA后运行全部transport过滤，10/10、33.19秒、0忽略、exit0（含真实剩余时钟、Mapper/Identity失败、gap503/结算/非法响应、Reviewer原子清理/静默跳过/结算）。本片源SHA与验收输入一致，diff检查通过。原像/日志/二进制SHA/result.json在 `/tmp/oviraptor-master-four-roles-20261008/`。这是localhost测试证据修正与邻近回归覆盖，没有新增生产角色能力，不代表当前全库/第三方/安装App验收；未重跑Clippy。原2806/63历史整库结果保留。完整REM-A01—A14 goal active，继续剩余执行证据及人工恢复专属出生/共享十维/USD消费者；Native JSON、真实资产、未提交改动和Qoder暴露面保护保持不变。

## 2026-10-08 Reviewer 故障原子性与真实 Web 退出（goal active）

本轮仅修改 agent_tests_specialist_transport.rs 的共享 Reviewer 故障夹具及三名调用者。夹具出生前冻结一轮 Web，原 provider 实际支付20 tokens/1请求、退出后再注入独立 Reviewer 故障。调用者不再出生后改写 provider；故障 handler 一次真实请求有精确断言。保留503失败不重试、清理错误导致request/child清理整体回滚、三类静默跳过导致后置条件拒绝，以及结算故障保留8000 tokens/1请求未结清占用与收到的20 tokens/1请求。无生产守卫改动或新增生产功能。

Reviewer三项3/3、16.13秒；同文件gap/非法只读响应邻近三项再次通过，合计6/6、0忽略、各exit0。本片SHA与验收输入一致、diff检查通过。原像/完整日志/result.json在 `/tmp/oviraptor-master-reviewer-faults-20261008/`。未重跑全库/Clippy、未改安装App，六项既有lib-test warnings保留。原全库2806/63仅为历史范围；不能从本片宣称全库通过。完整REM-A01—A14 goal active，继续其余原执行证据回归与恢复专属出生/共享十维/USD生产消费者。Native JSON、真实资产、未提交改动和Qoder暴露面白名单均受保护。

## 2026-10-08 原 provider 凭据漂移负向与真实退出（goal active）

本轮仅修改 agent_tests_changed_fact_tick.rs 的凭据归属负向测试。Root 出生前冻结一轮 Web，保留改写 Root 凭据触发 root_tick_original_model_changed、仍三次既有 SDK/零新增请求/应用行不变的全部断言；清理改为用未改写的原执行上下文实际请求 Web、结清20 tokens/1请求并产生原退出。未伪造 outcome，未改生产守卫或暴露面业务。

定向1/1通过、3.99秒、0忽略、exit0（2885过滤），本片SHA与验收输入一致、diff检查通过；原像/日志/result.json在 `/tmp/oviraptor-master-model-owner-20261008/`。六项既有lib-test warnings，未重跑全库/Clippy或安装App。同期只读复核恢复合同与 limits 消费者：签名 scopeBudget 仍 pending_dedicated_fresh_birth，普通 Root 按各自计划冻结限额；共享多历史 Root 总额/时钟/可信USD消费者尚未完成。这不是新增恢复功能或Master完成证据。完整goal active，继续剩余真实退出回归并推进专属恢复出生与共享预算；原2806/63历史全库结果、Native JSON、真实资产和未提交改动保护保持不变。

## 2026-10-08 Investigator 故障费用回归修正（goal active）

本轮只改 agent_tests_specialist_transport.rs 中三项 gap 故障测试及其断言。原夹具出生时固定代理 provider，测试随后改写 api_base，导致模型归属守卫在故障请求前拒绝；删除这三处测试内改写，沿原 provider 转发到原故障 endpoint。生产模型归属/授权/费用守卫未变。新增断言确认 provider 保持出生原像、每项故障 handler 确实只收到一次 Investigator 请求；503 再入仍不重试。已存在的真实 Web 20 tokens/1请求与独立 Reviewer 证据路径保留。

最终2/2 gap故障（6.13秒）及1/1非法只读响应（5.95秒，含空结果/工具调用两分支）通过，0忽略、exit0；结算错误保留原错误与4000 tokens/1请求未结清占用、非法回复保留收到的20 tokens/1请求。源SHA与最终验收起点一致，本片diff检查通过。原像/两轮日志/result.json在 `/tmp/oviraptor-master-specialist-gap-20261008/`。这是localhost回归证据修正，没有新增生产功能；未重跑全库/Clippy或安装App。完整REM-A01—A14 goal active，继续缺原Web退出的剩余夹具和人工恢复专属出生/共享预算；原整库2806/63历史结果仍保留，不按局部通过推算剩余失败数。Native JSON、真实资产、未提交改动、Qoder独占暴露面保持保护。

## 2026-10-08 独立 Reviewer 回归证据迁移（goal active）

本轮仅修改 agent_tests_changed_fact_reviewer.rs 测试。夹具在 Root 出生前冻结一轮 Web，以真实 localhost SDK 支付20 tokens/1请求并产生原退出，再进入独立 Reviewer。拒绝确认无独立控制的 finding、按 Root 建议选择是否派发 Investigator、原事实重放零新增 SDK 的断言均保留。精确费用断言为 Root 四/五请求、所有角色七/九请求；没有修改生产授权/预算守卫、新增功能或暴露面业务。

定向测试1/1通过、7.20秒、0忽略、exit0（含两条是否调查分支；2885过滤）；本片 diff 检查通过。原像、日志及结果 SHA 在 `/tmp/oviraptor-master-reviewer-20261008/`。本轮未重跑全库/Clippy、未更新安装 App，原整库2806通过/63失败仍只代表历史编译范围，不能用局部通过推算当前剩余失败数或 Master 百分比。完整 goal 保持 active，继续剩余执行证据及人工恢复共享预算闭环；Native JSON、未提交改动、真实资产与 Qoder 独占暴露面保持保护。

## 2026-10-08 paid-proposal 与物理原像回归迁移终态（goal active）

本轮主控仅修改agent_tests_paid_proposal_feedback.rs、agent_tests_changed_fact_rowid.rs。没有新增生产功能、修改授权/预算守卫或翻骨架旗。三条paid-proposal监督夹具在Root出生前冻结一轮Web计划，实际支付20 tokens/1请求并退出原Web后再进入独立Reviewer/Investigator/Root提案监督；新增Web与预算Root各一请求，精确断言Root五请求、所有角色九请求。原物理行替换用例保留坏原像拒绝/零SDK、精确恢复后原回执重放零SDK，再以真实Web请求结清原退出。未用虚构outcome伪装已完成执行。

首次3项为2/1：漏更新全角色请求数断言7→实际9，日志保留；纠正后提案/用量/gap邻近9/9、40.91秒。最终两文件一致输入的10/10、42.23秒、0忽略、exit0（编译库存2885，仅选择10）。覆盖原已闭合付费提案回Root并零SDK重放、在途损坏原worker扣留publication/保留第五笔Root费、精确恢复复用原付费响应、模型建议追加Web被拒/零目标授权、物理rowid替换拒绝及精确恢复；邻近未知费用与mailbox篡改/只读v2负向保持通过。文件SHA与最终验收起点一致，本片diff检查通过；lib-test仍六项既有warnings，未重跑Clippy/全库/安装桌面或第三方provider。

原像、失败与最终日志、输入SHA/acceptance-summary.json在 `/tmp/oviraptor-master-paid-proposal-20261008/`。原整库2806/63仍为其历史终态，不能据此或局部绿宣布当前全库通过。完整REM-A01—A14 goal active；下一片继续真实changed-fact/reviewer/executor等缺原Web退出证明的剩余调用，再补人工恢复专属出生、共享十维ledger/clock及可信USD费用消费者。Native JSON、Qoder独占暴露面、真实资产和全部未提交改动保护；未reset/clean/批量覆盖/commit/push或更新用户使用中的App。

## 2026-10-08 原签名 supervisor 回归迁移终态（goal active）

原整库63项失败中的七项缺worker_supervisor_original_signed_issuer_required，已逐项定向复验。三处测试启动点改为已有start_original_fixture_supervisor：只读取原withdrawal issuer，claim原父调用所有权，以start_owned_signed启动；无原协议仍走原路径，不补签、改Root或重发权限。主控只改agent_tests_specialist_inflight_closure.rs、agent_tests_directive_closure.rs、agent_tests_seven_terminal_producers.rs及其三项失效导入，未改生产授权/预算代码或Qoder暴露面。

本轮没有新增生产能力或翻骨架旗；测试现在真正进入原签名SDK/取消/退出链。首批七项7/7、18.72秒；最终连邻近supervisor的30/30、33.05秒、0忽略、exit0（编译库存2885，仅选择30）。覆盖实际SDK在途拒绝Root关闭、原取消在provider释放前返回、原付费保留/不重发、缺/外来锁与hash/role漂移拒绝、无SDK不造退出inode，以及撤权/自然到期/写入故障/异步/源扫描邻近边界。三个代码SHA与最终验收输入一致，本片diff检查通过；lib-test六项既有warnings，无本片新增导入警告，未重复运行lib Clippy，上一片21 warnings仅保留其当时范围。

证据、逐文件原像/输入SHA/终态JSON在 `/tmp/oviraptor-master-signed-issuer-20261008/`。实际localhost SDK与Rust边界测试不等于新安装App或第三方/全平台验收。原整库2806/63结果保留；本片不能宣布当前全库绿，不从定向结果推算Master百分比。下一片处理paid-proposal/changed-fact等仍用虚构Web outcome结清的夹具，逐项保留真实费用与重放断言，再推进人工恢复专属出生与共享十维/USD消费者。完整REM-A01—A14 active，不提交/推送/清理真实资产，Native JSON与Qoder独占暴露面保持保护。

# Master 回归门禁与恢复入口 · 2026-10-08

## 最新用户指令与范围

用户已明确恢复 Master，取消原整库终态后暂停安排；完整 REM-A01—A14 goal active。本轮主控优先处理真实回归债，Qoder只开发暴露面白名单；不覆盖其业务增量。Native JSON保留，不恢复Strix，不重置/clean/批量覆盖/自动提交或推送，不删除真实资产。已安装1.1.61留供用户工作，本轮源码修复尚不代表安装包更新。

## 原整库终态（失败）

`nice -n 15 cargo test -j 1 --lib -- --test-threads=1`，原session13786，**2806 passed / 63 failed / 0 ignored / 0 filtered**，运行7436.86秒，exit101，2869项。日志与最终JSON在 `/tmp/oviraptor-master-full-gate-20261008/`。原二进制SHA256为 `50ad8a18d38af5fc56223455ef0f1a2a6037df3da402fa7c49e994a7f06195e5`，另留原可执行副本。

程序编译在本次急用资产/环境/1.1.61改动之前，运行中观察1366个Rust源文件的SHA采于启动后；不能充当开跑前冻结证明。个别测试读取实时仓库文件，整批不是不可变全树验收。终态源漂移单列 `final-source-drift.json`，当前工作树不能据此称全绿。旧2624/1、Clippy18仅是历史数据，不能覆盖本终态或成为当前Clippy结论。

失败展开归档 `final-panic-map.json`：41项unwrap缺原Web退出证明（另1项直接同错误）、7项缺签名原worker issuer、2项付费历史缺显式财务合同；其余为计数、模型原像、SDK角色/预算/退出证明与残留文件登记等。以上按报错归类，未逐项证明共同根因，不把全部失败称夹具问题。

## 本轮真实生产改动与测试迁移

- `multi_agent/review.rs`：原完整授权守卫拒绝`agent_attempt_not_active`时返回Cancelled；其余授权/签名/冻结/租约错误仍拒绝，不创建Reviewer、不续C、不派发SDK。暂停回归加强全应用行和SDK数量不变断言。
- 授权控制共享夹具在出生时冻结一轮Web预算，实际执行/付费/退出Web后才走三侧控制和独立Reviewer；明确区分原Web coverage与授权漏洞发布。原账单50 tokens/1请求有断言，无测试伪造退出证明。
- 缺失/partial/inconsistent用量测试区分独立原Web/Root端点与子角色端点；保留请求已收1、三token维未决4000、原Web不重发、Root付费重放不新增的断言。
- 两个Root准备/分派测试在原准备断言后执行真实受限Web SDK并结清原退出；保留原Reviewer/Root下限和Rust评分断言，模型夹具补处理真实预算监督请求。
- Gap篡改重放夹具改用实际Web与独立已收Reviewer证明。旧v2测试混淆了JSON语义兼容与原不可变回执：现分别覆盖只读v2语义校验与新协议拒绝改写，不声明历史Root可重新执行。原篡改/单子/不重发断言保留，新协议降级负向另加强零写/SDK断言。
- 残留登记四个文件已人工核对语义上下文，限量更新证据和理由；扫描固定范围、哈希/字面量计数/符号拒绝策略不变，不自动批准新增残留。

## 测试证据与未完成验收

原用量复现1失败、授权复现1失败、准备复现1失败、旧v2复现1失败均保留；首次用量修正后暴露Root实际多一笔付费反馈，失败日志保留。授权正向1/1、残留登记4/4已通过。

首次矩阵37/1（暂停结果错误），第二矩阵42/2（v2夹具冒充执行兼容、准备模型未答预算监督），分别保留。中间两批被外部暴露面编译错误阻挡：orchestrator/recovery类型与新字段、tests_e2闭包生命周期/调用参数；主控未改其业务文件。最终 `acceptance-regression-matrix.log` **45/45、0忽略、146.53秒、exit0**（编译库存2885项，仅选择45项，不是全库）。包括生产暂停结果修复与零写/SDK断言、原费用未知三种矩阵、真实原Web/Root准备、三侧控制/独立Reviewer/ACK和发布故障、gap篡改拒绝、新协议降级拒绝及只读v2语义兼容。旧v2正向现在是明确的只读校验，不能宣称历史Root执行兼容完成。

最终 `cargo clippy -j 1 --lib` **21 warnings / 0 errors、exit0、1m44s**；其中两项位置在暴露面settings.rs（field_reassign_with_default、manual_range_contains），未代改Qoder范围。没有以cargo --fix批量写回。lib-test编译另有6 warnings，不能与Clippy计数混用。七项本片代码/登记SHA与验收开始快照一致，`git diff --check`本片通过，HEAD仍14a78753b95aad65e8fd8611048e6d6ff8ddb9a2。终态JSON为 `acceptance-summary.json`，原像在同目录，外部Rust漂移单列保护。

以上是localhost SDK与真实Rust路径/只读解析证据，未做新安装桌面或第三方provider验收。原整库63项失败仍按原结果保存，不以定向绿自行宣布当前全库或其余回归消除。

## Master 文档差异与下一片

当前源码的wired监督包括new-evidence、lease-expired、protection-changed（实际钩子与生产调用点已存在），human-recovery仍not_ready；旧文档四类均not_ready属于历史检查点。恢复UI已挂到NativeRunStatus并接显式批准、草稿、取消及原requestId只读找回，不再是UI未接；签名共享scopeBudget目前仍pending_dedicated_fresh_birth/executionAllowed=false，仅持久化合同。专属出生、所有Root共享十维ledger/clock、可信USD预约消费及实际执行消费者尚未闭合。不能用UI或合同存在替代执行验收。

本轮定向终态已收，继续按原失败地图逐项修复缺原Web退出/缺签名issuer等剩余簇，并验证生产取消/授权/预算边界；之后推进human-recovery专属出生与共享预算消费者。A03/A11与完整Master均未宣称完成，goal保持active。

## 原整库63项失败名单

- `commands::agent_tests::authorization_control_reaches_an_independent_reviewer_before_publication`
- `commands::agent_tests::authorization_mailbox_failure_releases_target_lane_and_reservation`
- `commands::agent_tests::budget_specialist_missing_provider_usage_keeps_receipt_but_rejects_result`
- `commands::agent_tests::completed_gap_replay_accepts_acknowledged_legacy_v2_without_reissuing_requests`
- `commands::agent_tests::completed_gap_replay_rejects_proposal_that_replaces_the_reviewers_gap`
- `commands::agent_tests::completed_gap_replay_rejects_tampered_or_unacknowledged_assessment`
- `commands::agent_tests::completed_gap_replay_rejects_unattributed_proposal_reference`
- `commands::agent_tests::coordinator_actual_closed_paid_proposal_returns_to_root_and_replay_spends_zero_sdk`
- `commands::agent_tests::coordinator_actual_independent_reviewer_feedback_controls_bounded_gap_dispatch_and_replay`
- `commands::agent_tests::coordinator_actual_root_dispatch_persists_rust_utility_and_merges_model_duplicates`
- `commands::agent_tests::coordinator_changed_fact_cannot_replace_original_provider_credential_before_new_fee`
- `commands::agent_tests::coordinator_changed_fact_physical_rowid_replacement_denies_replay_without_sdk`
- `commands::agent_tests::coordinator_proposal_paid_worker_mutation_rejects_publication_then_exact_restore_reuses_fee`
- `commands::agent_tests::coordinator_proposal_root_advisory_web_step_cannot_issue_another_target_grant`
- `commands::agent_tests::coordinator_tick_actual_multi_prepare_uses_root_sdk_then_mapper_and_original_headroom`
- `commands::agent_tests::coordinator_tick_actual_sigkill_recovers_original_decision_without_new_sdk`
- `commands::agent_tests::coordinator_unsupported_trigger_refuses_before_another_actual_root_sdk_or_write`
- `commands::agent_tests::coverage_only_result_never_fabricates_a_reviewer_or_a_confirmed_decision`
- `commands::agent_tests::denied_cross_object_control_does_not_create_a_candidate_or_finding`
- `commands::agent_tests::directive_closure_executor_settlement_failure_retains_unsettled_budget`
- `commands::agent_tests::directive_closure_review_success_is_not_an_applied_action_receipt`
- `commands::agent_tests::directive_reentry_real_target_does_not_retry_unknown_proposal_or_block_mapper`
- `commands::agent_tests::disconnected_cross_control_is_not_replayed_or_reviewed`
- `commands::agent_tests::disconnected_owner_control_is_not_replayed_or_reviewed`
- `commands::agent_tests::disconnected_tester_control_is_not_replayed_or_reviewed`
- `commands::agent_tests::e2e_pause_resume_and_reexecute_keep_the_budget`
- `commands::agent_tests::every_target_ends_in_exactly_one_terminal_state`
- `commands::agent_tests::executor_allocation_actual_three_sdk_preparation_preserves_original_reviewer_floor`
- `commands::agent_tests::executor_allocation_full_mixed_unlimited_preparation_binds_actual_runtime_windows`
- `commands::agent_tests::executor_mailbox_failure_releases_target_lane_and_reservation`
- `commands::agent_tests::failed_gap_round_is_not_mistaken_for_an_assessed_revision`
- `commands::agent_tests::fresh_multi_authenticated_anonymous_control_actual_entry_preserves_account_metadata_and_fees`
- `commands::agent_tests::gap_refuses_missing_or_changed_review_seal_before_scheduling`
- `commands::agent_tests::gap_review_real_independent_receipt_closes_link_without_rewriting_old_root_or_fabricating_findings`
- `commands::agent_tests::insufficient_finding_remains_reviewable_only_after_a_new_candidate_revision`
- `commands::agent_tests::investigator_allocation_admits_fixed_proposal_when_surplus_exceeds_floors`
- `commands::agent_tests::native_sdk_log_coordinator_changed_fact_real_gate_uses_next_original_call_and_zero_sdk_replay`
- `commands::agent_tests::pause_after_authorization_before_review_cannot_publish`
- `commands::agent_tests::pause_during_candidate_write_cannot_stage_or_publish_authorization`
- `commands::agent_tests::pausing_after_the_control_response_prevents_finding_publication`
- `commands::agent_tests::pausing_after_the_cross_probe_prevents_the_control_request`
- `commands::agent_tests::pausing_after_the_owner_probe_prevents_the_cross_and_control_requests`
- `commands::agent_tests::policy_native_routes_without_runtime_adapter`
- `commands::agent_tests::rejected_review_completes_the_child_without_publishing_a_finding`
- `commands::agent_tests::request_accounting_terminal_snapshot_preserves_aggregate_and_child_ownership`
- `commands::agent_tests::resumed_single_root_refuses_inherited_or_skipped_model_round`
- `commands::agent_tests::retirement_literal_allowlist_matches_reviewed_sources_and_packaged_inputs`
- `commands::agent_tests::reviewer_and_replay_preserve_manual_execution_stop_without_gap_autostart`
- `commands::agent_tests::reviewer_finish_failure_cannot_publish_an_acked_decision`
- `commands::agent_tests::reviewer_insufficiency_starts_one_read_only_gap_run_and_assessment`
- `commands::agent_tests::root_local_react_actual_tool_only_model_stops_at_frozen_three_call_bound`
- `commands::agent_tests::specialist_transport_four_real_roles_have_separate_calls_usage_and_mailbox`
- `commands::agent_tests::specialist_transport_gap_503_is_one_request_and_retains_unknown_usage`
- `commands::agent_tests::specialist_transport_gap_settlement_error_retains_reservation_and_original_error`
- `commands::agent_tests::specialist_transport_invalid_readonly_response_preserves_received_usage`
- `commands::agent_tests::specialist_transport_reviewer_failure_is_atomic_and_preserves_cleanup_errors`
- `commands::agent_tests::specialist_transport_reviewer_settlement_failure_retains_received_usage`
- `commands::agent_tests::specialist_transport_reviewer_silent_cleanup_skip_rolls_back_request_failure`
- `commands::agent_tests::web_inflight_closure_actual_changed_original_hash_dispatch_or_worker_role_keeps_every_row`
- `commands::agent_tests::web_inflight_closure_actual_missing_or_foreign_original_lock_never_creates_proof_on_close_or_reentry`
- `commands::agent_tests::web_inflight_closure_actual_paid_sdk_preserves_original_fee_and_never_accepts_another_round_after_terminal`
- `commands::agent_tests::web_inflight_closure_original_sdk_blocks_root_terminal_until_actual_exit`
- `commands::agent_tests::web_inflight_closure_pristine_no_sdk_does_not_create_an_original_exit_inode`
