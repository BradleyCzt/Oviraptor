# Mapper v15 原工作者过期后的本地恢复实施方案

日期：2026-10-10。本文是当前源码审计及下一轮实施规范，**不是已交付功能或运行时验收证据**。不涉及 Asset Exposure，不改变默认 v9，不升级历史出生声明，不删除真实数据。

## 当前真实入口和阻断点

- `src-tauri/src/commands/multi_agent/prepare.rs:145–162`：显式原出生 v15 捕获真实冻结请求和 inventory；184–191 调用共享 core 的真实 Mapper adapter；196–218 交付原结果或保留逐轮发票后报错。
- 同文件 **216–223 当前已有**交付提交后检查原 worker `expired` 并进入 `native_coordinator_lease_expiry_feedback`。不重复新增这一 caller。此前仅读取到 215 行而漏看此段的判断已更正。
- `commands/agent_native/coordinator_lease_hook.rs:12–13`：实际付费 Root 入口先建立 `lease_closed_mapper`，再持有原 SDK inode。Root 评估建议只能是 `assess:lease_expired`，不新增 worker、C、目标权限。
- `commands/agent_native/coordinator_lease_closed.rs:34–40`：仍调用旧 `specialist::verify_received_original`、`received_for_local_delivery`，读取 `agent_specialist_calls.response_json`，强制 `model_requests == 1`。171–181 固定探测 `specialist-sdk` inode。v15 的来源实际是 `agent_mapper_loop_journal`、一或两轮原发票及 `mapper-loop-sdk` inode，不能进入这条旧财务链。
- `commands/multi_agent/readonly_assessment.rs:122` 首先调用 `ExpiredSavedProof::capture`。该 proof 在 `attempts/expired_saved.rs:137` 本身要求旧单轮 specialist 收据，捕获库存也只有 `agent_specialist_calls`，因此 v15 还未到新 final reader 就被旧 proof 拒绝。
- `readonly_assessment.rs:157–177` v15 分支只支持 `running` 正常发行、`completed` 原 final receipt 重放；`paused` 被拒绝。之后 paused 分支又要求旧 `received_for_reconciliation`。不能简单把 paused 临时改 running。
- `specialist/mapper_loop/final_seal.rs:56` 正常 capture 要求 running、未关闭、未取消；原签名 cause 是 `normal_completed_frozen_frontend_assessment`。当前原 final 不能被当成过期恢复发行接口或给已存 JSON 补签。
- `commands/multi_agent/child_accounting.rs:184–211` 已有真实 v15 累计费用校验：从 `verified_original_output_on` 核原 usage，并核四个模型维度 consumed/indeterminate。它只可复用原费用核验和释放原剩余 reserve，不能替代过期授权 proof。

行号是审计时参考位置；实施前按函数名复核，避免其他 agent 的增量修改造成行漂移。

## 来源与状态边界

原出生必须精确 schema 15、`original_frozen_mapper_two_round_one_lookup_v1`，完整继承原 14 规则；原 RootOwner、原签名 birth、原 C epoch/fence、原 assignment 和 first worker 不变。旧 v9–14 保持旧路径，不从旧单请求 grant 推导两轮合同。

财务来源只能是：原四维 reserve 发票、原逐轮 dispatch/full frozen wire/configuration binding、invoice/received 或真实 uncertain/unsent 阶段、原 lookup 审计链、每轮不可变费用条目与原 worker clock samples。累计 requests 是**真实 received 轮数**，不是恒定 2；原 grant 的 2 是上界和预留，不是已消费费用。完整输入/输出/cached/USD/clock 若有未知、外来来源或不一致，必须拒绝完成，不能猜 0、退款或重发 SDK。

`verified_original_output_on` 可提供完整原输出及累计 usage，但它不是过期批准，也不是完成收据。`verify_for_idle_on` 可核全部财务阶段，`has_transport()` 决定是否必须探测已有 `mapper-loop-sdk` inode；不得新建 inode 来假证退出。原 final reader只读核普通 completed 的 signed final/ACK，不需要当前模型配置或 live C。

| 原状态 | 允许行为 | 必须拒绝 |
| --- | --- | --- |
| running，合法 live worker | 原正常 adapter、原 normal final delivery | 绕原权限或提高 2 请求/8000 token 上限 |
| paused，worker 已被真实签名 expiry 撤权，完整原最终输出 known，原 C/Root 尚可执行 | 专属本地结算、关闭和原输出交付；worker 保持 expired；之后 Root advisory | SDK、lookup、网络、新 cap、新 worker、续原 child |
| paused，只有首轮工具调用/缺最终输出 | 只核原费用与保留待恢复状态 | 把中间分析当完整 Mapper 输出或生成 final receipt |
| uncertain / invoice 无法合法完成逐轮费用归属 / 第二维度未知 | 保留原已知发票及未知债，报明确拒绝 | 自动退款、重发请求、普通 completed、Root 成功封套 |
| completed，原 final receipt完整 | 原历史 reader 免费返回旧投影 | 当前配置重新冻结、ACK 重写、重复释放预算或新 Root 评估 |
| C 撤权/父 Root terminal，尚无合法已完成恢复收据 | 纯历史诊断及费用保存 | 发行新本地完成或新 Root SDK；不能靠 historical receipt 恢复 live 权限 |

过期关闭仍是“已付原只读输出的本地交付”，不是重新执行。若实施选择专属 paused financial projection而不普通 completed，则必须同步原 expiry audit 和 typed delivery reader；不能沿用当前 completed reader同时把字段改 paused。优先延续既有六角色 expired-saved 完成语义：assignment/run 完成本地交付，worker 始终 expired，cap 始终 revoked，lane 原子释放。

## 最小依赖顺序及可编码接口

以下为拟实施接口，尚不存在，不得作为当前生产能力引用。

1. **财务/过期 typed proof 叶**：`specialist/mapper_loop/expired_saved.rs`。`capture_expired_mapper_output_on(snapshot, actor, child) -> OriginalExpiredMapperOutput` 私有字段，绑定原签名 withdrawal + 精确 worker deadline/finished/failure + 完整 known 输出 + 全逐轮原费用/lookup/clock + 原 idle inode guard。`require_on` 在同一事务前后复核全部源。不能接 caller usage/JSON/bool 作为授权。只允许原完整最终输出；首轮未完成、unknown、foreign SDK、原源码漂移拒绝。
2. **过期关闭 typed delivery/final 叶**：专属 capture-before/issue-after/read-original，独立 cause/domain，原签名 issuer 只接上述 opaque proof 和实际同事务关闭/ACK。先确定当前 signed v15 继承的 expired-saved 规则是否足以覆盖该专属 cause；若不足，显式新出生合同，不给旧 v15 补 authority。正常 final schema/cause/历史字节不改。拒绝从 saved JSON 或正常 final guard 构造过期签名。
3. **实际交付事务**：在 `readonly_assessment.rs` 原 protocol 选择处先精确识别 v15，避免先调用旧 ExpiredSavedProof。专属 paused 分支持有原 idle guard，复用原累计费用核验/原剩余预留释放；更新 assignment/run 的原允许关闭字段；保持 worker expired 的完整原行，绝不调用普通 worker finish；只撤原 cap、不恢复；精确原 evidence_summary 路由、一次送达及 Root ACK、原两 outbox 事件、发行 typed signed receipt一起提交。失败 rollback 全部关闭/ACK/释放，原此前 paid invoice 不丢。
4. **SDK 返回至本地恢复的真正 caller**：adapter 最后一次 SDK 已 received 后，若 worker 在 finish/check 时自然过期，只允许通过上述纯原输出 proof 提取完整结果并进入 dedicated delivery；不能重进 live core、调用第二轮或把任何 Err 当可恢复。`prepare.rs` 已有的 expired feedback 在交付成功 commit 后执行；未交付成功不建 lease-expired frame。
5. **Root 来源/frame/inode**：在 `coordinator_lease_closed.rs` 显式 v15 分支读专属 signed expired delivery、累计 usage/full journal/lookup/clock和原 expiry receipt。原旧六角色单SDK分支不变。inode 根据原合同精确选择 `mapper-loop-sdk`；帧 stable fact 不含新的 Root 费用、C TTL 或当前时间。`verify_lease_closed_source` 前后重新读同源；Root 步骤只 assess:lease_expired，不 dispatch。
6. **纯历史重放**：completed replay先强签名及完整原来源，再投影已付 Root publication；没有 publication 才按原 live Root/C/预算门禁评估。已发布后免费重放不能 require live C/当前模型配置，不能 SDK/ACK/renew/release。`mapper_historical_projection` 同步识别合法专属 expired final，不以普通 normal final冒认。
7. **归档和关闭库存**：新增 receipt/schema必须 additive初始化、immutable guards、scope retained、可信 restore；旧 Bundle optional缺表不回填；idle/parent关闭库存识别但不自动发行恢复。新增表不意味着历史来源完成，完整 paid archive读取须用对应 typed reader。

可并行分工：财务 agent 独占 typed expired proof与费用/idle；source agent 独占专属 signed delivery及实际运行时测试；主控独占 readonly_assessment/prepare、issuer/schema注册和完整 runtime验收；frame agent 在 typed来源冻结后接 lease_closed/hook。不能在 typed财务和签名来源未稳定时先放宽公共 guards。

## 实际 runtime 验收（不得冷改时钟/状态）

原 scheduler child TTL约600秒；C Supervisor通过真实 `coordinator_heartbeat::renew_if_due` 在到期前120秒窗口续 C，authorizer只允许 C列写，不能续 child。测试必须新出生原 v15、真实 localhost SDK、原合法分配及费用、原 supervisor自然 sweep、完整原签名 withdrawal。不得 UPDATE expires_at/state/fence 来冒充正向自然过期。

- 正向一轮最终输出、正向两轮一次真实 evidence.lookup：真实发票已 persist/received/settled，但在最终交付前持有严格 cfg(test)、完整 scope 的 post-original-fee barrier等待原600秒 child自然过期；父 owner和supervisor仍活、C合法续。SDK不应在 endpoint挂600秒到超时而造成未知费用。释放 barrier后真实 dedicateddelivery成功、worker仍expired/caps撤销，累计模型请求分别1/2，原token/cached/USD/clock精确；原reserve只释放一次；Root真实收到lease-expired frame且仅非空assess建议；新评估先于声明恢复完成。
- 重复完整入口/history：SDK/lookup/新grant/renew/ACK均0增，全表及原档字节无写；同 original paidpublication精确返回。只重复一个hook不等价于完整免费重入。
- unknown第二轮：真实 provider缺 usage 或真实无法确定费用，不冷造账单；第一轮known保存、第二轮unknown保留，无 completed、final/expiry输出receipt、成功Rootpublication、退款或SDK重试。
- 首轮有lookup但无最终round、超frozen bound、真实工具source漂移：不完成，不把 partial文本当final；原费用不丢。
- 原C撤权、父停止、跨scan/attempt/root/worker/epoch/fence、签名/原invoice/lookup/clock冷篡改均负向拒；冷修改仅用于负向且全表0旁写，不算自然过期正向。
- delivery/finalseal/ACK写故障：真实发票先前已提交仍在；本地事务释放/关闭/message/events/receipt整体rollback；下一次只能精确重读原费用，不重新SDK。
- 普通 live v15一/两轮保持正常 final cause，旧 v9–14单轮六角色、原 expiry正向/重放继续通过，不能通过降低出生版本、取消费用断言或抬cap换绿。

完成判定：必须覆盖“真实SDK逐轮费用→自然signed expiry→原输出专属本地关闭/ACK/签源→实际付费Root评估→整入口纯历史零写重放”。当前只具备正常 v15循环/费用/final与旧单SDK expiry框架；本方案不将 frame 单行兼容、结构测试或原normal final签源声称为恢复完成。
