# REM-A03 新出生任务的监督请求容量（2026-10-08）

本片只闭合 model_requests 的保留与物理 HTTP 准入，不将 REM-A03、十维动态预算或 Master 全范围标记完成。

## 已接入的真实功能

新建 Multi Root 使用严格的 Native JSON bootstrap v6，增加 supervisionRequestAllocationRule=retain_half_surplus_and_check_each_physical_claim。旧 v1–v5 按原合同验证，既有 Root 不升级、不重签、不补账。

Web Executor 的有限请求额度从当前共享余额计算：先扣除预算评估一次请求和后续最小六次容量，再取剩余的一半作为上限；原有顺序建议额度继续从该上限分配。六次包含 Reviewer/Root 原保留容量与 ClientSide、InputParser 的基础只读路径。保留容量不是付款，不写伪 Reserve；无限请求沿用 null 原语义，token/其它维度仍由各自原准入检查。

每条物理 HTTP 的 claim 在原 IMMEDIATE 事务中重新检查共享 Root 余额。尚未得到原付费 Root new-evidence publication 的物理 claim 按六键计数，跨 Web worker 汇总；图节点、Web 结算或普通 ACK 不替代原付款。只有通过原 Tick 投影（原票据/退出/时间线验证）的对应出版才解除该条监督义务。

容量不足在下一条 HTTP 发送前拒绝，并返回独立容量错误。原 Web SDK 和已发送 HTTP 的费用、退出及结算保留，后续仍走原 new-evidence/ClientSide/InputParser 门禁。新版原财务合同即使缺失 Root sidecar 也拒绝，不能退回旧路径；未声明新版协议的旧 journal-only Root 保留兼容性。

## 本地验证与证据边界

证据目录：/tmp/oviraptor-master-supervision-admission-20261008/。

- 正向：实际 run_agent_target、多身份/匿名对照、三个不同物理 HTTP 事实分别付费监督；独立 ClientSide 与 InputParser 的生产回归保持。
- 负向：一轮提出十二个不同 Web HTTP，容量不足在对应 claim 前停止，只有有容量的子集到达真实 localhost server；所有已准入 claim 都有独立付费监督，实际 Web invoice/terminal 保留。十维 reserved/unknown 最终均零，并将请求费用与实际 SDK/HTTP 到达数量对账。
- 新任务过低余额拒绝派发，只有原 Root/Mapper/Root 三次费用；缺 sidecar 拒绝且零写。旧合同的低额度派发、InputParser 一单位边界和顺序指令仍保留原额度，不提高预算换绿。
- 两个旧派发测试改为实际 SDK 退出后结算，去掉手工伪造 Incomplete 退出；三条 Client 错误断言恢复已存在的生产 executor_followup_failed:client_side: 前缀，费用与零写断言保留。
- 原失败与临时回归日志保留；不忽略、删除失败测试，不把过渡版二进制报告成最终源码通过。

最终证据：扩大邻近回归 140/140、0 忽略、728.377 秒（库存 2913）；另一编译快照的关键入口及旧合同原所有权回归 21/21、0 忽略、176.843 秒（库存 2914）。后者包含 12 项复验和 9 项新增邻近检查，共 149 个不同测试。两次均用保存的本片新编译二进制逐个运行，不引用旧安装 App。

npm run build、cargo test --lib --no-run、cargo build、cargo clippy --lib 及最终 cargo check --lib 全 exit 0。应用普通构建 18 warnings；Clippy 19 warnings / 0 errors，未宣称 warning-free。23 个本片源码路径语法解析通过，增量无尾随空白。

源绑定：140 项对应 source-round5-inputs.json / test-bin-round5；21 项对应 source-current-inputs.json / test-bin-round6。其间外部开发进程更新了共享归属/资产模块与页面；本片 Master 23 路径 SHA 始终与 owned-source-scope.json 一致，未覆盖这些外部改动。后续归属目录与页面又有更新，最终按 source-check-seal-inputs.json / ui-seal-inputs.json 再验 cargo check --lib 与 npm run build，均 exit 0，验收时两组源码均零漂移。应用构建对应 source-build-latest-inputs.json，期间归属文件继续更新，不能把该二进制报告为所有最新归属修改的运行验收。

只读诊断是逐文件增量审查、原容量缺口复核和源码/编译指纹，不是功能交付。localhost 使用真实 SDK/HTTP 传输、原账本与可丢弃数据库，但响应来自脚本；不是第三方模型推理/美元、整库回归或已安装 App 验收。目标仍覆盖完整 REM-A01–A14；旧 blocked 元数据对应的编译原因已解除，不据此伪造 complete。

## 未完成的骨架与后续优先级

这不是完整承诺的预算预留协议。最小六次 headroom 不保证后续任意 Root 多轮/重试、未来撤权/保护变化、USD/token 或其他维度都足够；并行 worker 的完整实际争抢与所有后续模型付款路径仍须专项验证。新 guard 在 HTTP 原事务内计数，但不能把尚未发生的监督成本称已预留/已付。

下一片先补 token/各维可解释容量与后续 SDK 准入、实际并发争抢和 unknown/撤权组合；保持每一物理事实独立监督，不通过合并或提高额度掩盖不足。随后仍按 Master 优先级接 dedicated recovery birth，普通恢复禁令保留。原授权与批准十维、代理/身份/截止时间消费边界见 NEST_MASTER_RECOVERY_BIRTH_WORKLIST_2026-10-08.md。

Asset Exposure 的业务文件不在本片范围。保护所有未提交改动；不提交/推送，不替换已安装 App，不修改真实用户数据库、CAS 或资产，不恢复 Strix。
