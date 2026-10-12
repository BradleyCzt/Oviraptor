# REM-A03 BusinessLogic output：原源码缺口与最小闭环

日期：2026-10-09。本文是只读审计与待实施设计，不是新增功能或验收完成证据。未改 production、运行 Cargo、访问真实资产或 Qoder Exposure 业务。以本轮源码为准，历史 Master 状态文字不代替真实入口核验。

## 1. Master 范围

`NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` REM-A03 原要求是六类监督触发、changed-fact 去重和剩余实际角色反馈：真实原事件 → Root SDK → 原回执 → Rust 准入，空重放零新增费用，建议不能直接产生权限。BusinessLogic 是当前剩余 reserved role-output kind；不能把其枚举、只读合同、调度单元测试或显示框架算作该链闭环。Surface 新增真实验收不自动证明 BusinessLogic。

## 2. 当前证据分层

| 层级 | 当前源码 | 真实含义与边界 |
|---|---|---|
| 已存在原语 | `src-tauri/src/agent_runtime/contract.rs` BusinessLogic 角色、`requires_read_only_analysis_lane`、reviewer floor、specialist binding | 明确只读分析，受 Reviewer floor 保护；没有目标 HTTP、身份重放或工具权限。 |
| 已存在原语 | `multi_agent/scheduler/child_contract.rs` BusinessLogic 能力仅 `evidence.read` / `mailbox.write` | 可接受这个受限合同，不证明应用已经执行该角色。 |
| 通用调度与测试 | `scheduler/bootstrap.rs`、`scheduler/supervised_bootstrap.rs`；`agent_tests_business_logic_reviewer_floor.rs`；`multi_agent/tests/replay.rs` | 底层可准备只读 worker；测试验证原 floor/只读 lane。没有真实 BusinessLogic provider completion/Root output 验收。 |
| 明确未就绪门禁 | `commands/agent_native/coordinator_business_logic_gate.rs`、`coordinator_schedule_decision_role_gate.rs` | Root 选中 `dispatch:business_logic` 后仍拒 `business_logic_dispatch_not_ready`，不制造 child/grant/SDK。 |
| 显示骨架 | `coordinator_rust_policy.rs` 的 reserved inventory、try-produce/skeleton | 当前仅 BusinessLogic reserved；只显示 not_ready，不是原 signed output producer。 |
| 缺少生产执行 | production commands 全角色搜索 | 没有类似 `multi_agent_external_surface` 的真实 BusinessLogic 执行函数、严格输入验证、真实模型输出验收与收口。 |
| 缺少真实完成 writer | `commands/multi_agent/readonly_assessment.rs` 的 role→kind/prefix 分派 | 当前没有 BusinessLogic case；不能直接复用后声称已完成，会被 `readonly_assessment_role_invalid` 拒绝。 |
| 缺少历史费用 reader | `multi_agent/specialist/closed_audit.rs` role allow-list | 当前不含 BusinessLogic；新增调用必须专属验原 receipt，而非放宽泛化 live helper。 |
| 缺少 A03 反馈 | frame / hook / birth / obligations | 没有 BusinessLogic 原输出票据、稳定 frame、真实 Root paid publication 与按 worker 消债。 |

上述源码与测试是当前静态核验；本次没有重新运行这些测试，不引用旧测试数量作为本轮绿色门禁。

## 3. 最小产品范围：分析已有原事实

本片只能分析当前 target 已冻结、已验真的事实，产出业务流程候选、证据缺口与待人工核验事项。不得新增网站列表、自动访问 URL、猜测集团资产、读取 Asset 数据、触碰 Qoder Exposure、生成命令或扩大 identity/target scope。

输入应由 Rust 构造：原 Root/scan/attempt/target/plan binding、原 Mapper route/contract 元数据、原只读身份元数据及已 closed 的证据引用。只使用已有可信来源；若没有可用事实，正常返回 `not_scheduled` / 明确缺证，不制造业务流程。每个输入来源有原 scope、receipt/hash、稳定物理行；不把任意 caller JSON 当原事实。

这是优先阶段一的受限只读业务候选分析和 A03 原输出反馈闭环，不能替代完整 BusinessLogic/General ReAct。阶段二若要验证多步业务状态、交易流程或跨身份行为，必须先有独立批准的任务合同、精确目标/身份/动作允许集合、每步硬预算、Broker 与沙箱权限、可停止的实际执行与独立 Reviewer；当前两个只读能力不授权这些动作。本文不建议先加业务工具或自动申请扩大权限，也不把阶段一通过算作 A04 全角色和 A05 General ReAct 完成。

模型输出专属严格 schema，例如 bounded summary、flowCandidates、evidenceGaps、coverageGaps，所有候选带原输入引用；`confirmedFindings=false`、`executionAllowed=false`。未知字段、引用不存在来源、确认漏洞、要求请求/新身份/新路径均拒。它提供候选分析，真实漏洞仍交独立 Reviewer 与既有验证链。

## 4. 实施顺序与边界

1. **原输入 + 真实 worker producer。** 新专属 `business_logic_input.rs` 验 frozen source，调度仍用现只读 lane 和两个原能力。新生产函数先保存严格任务，再走 `multi_agent_child_round_transport` 实际 SDK。原 Reviewer floor、模型硬限额、remaining clock、worker namespace、cancel/expiry 必须保持。独立 birth 合同建议显式 v13，继承 v12 并新增精确 input/output/capacity rules；默认版本不变，旧 JSON 不升级、不回填。
2. **真实模型验收 + typed original collector。** 专属 `business_logic_output_collection.rs` 只接受实际 `agent_specialist_calls` received 原请求/response/usage/event/checkpoint/fees，原请求无 tools，任务/原输入与实际 wire 精确一致；normal running 和 completed 各自核状态。全 child SDK request=1、目标费用=0、已知 invoice breakdown/clock 原来源精确匹配；未知费用/foreign receipt/多 SDK 未纳入合同即拒。不要假造一张零费用 SDK 发票。
3. **同 IMMEDIATE 正常完成。** 新 immutable output table 与私有 typed seal。`capture_before_on(db,actor,child,target_dir,&VerifiedMode)` 捕获 running 的原 collection/worker/event floor；原结算、finish 撤销能力与释放 lane、真实结果 message + Root ACK；`issue_after_on(...,&guard,actual_message_id)` 再核源不漂移并签 seal，失败整个事务 rollback。建议结果 kind 保现只读语义 `evidence_summary`，correlation 专属 `business-logic:<assignment>`；不要使用显示骨架或任意旧完成行重新签。
4. **历史 reader。** `read_original_on(db,actor,child)->Option<VerifiedOutput>` 验原 birth/private seal/完整 source/fees/真实 message/两个 outbox 事件/分别 assignment/run closure。允许原 C 已过期或 scan paused 的纯读；不 live grant、不恢复 worker、不补签缺失记录。通用 mailbox consumed reader 有 live C gate，不能直接拿来做历史 source。
5. **A03 真正付费反馈。** 新 `business-logic-output` frame/hook 读取上述 typed seal，仅 `assess:business_logic_output`，无 scope/dispatch/target grant。首次真实 Root paid 前后仍需 live C、determinate、原完整 model profile 和 source 复核；historical replay 必须原唯一付费 publication、exact frame、无 localTools。实际 SDK request 无 tools，收到 tool calls 在 LocalStep parse 前拒而保原费用。
6. **预算库存。** 每原 BusinessLogic worker 的单次 output 义务仅由其 exact original paid publication 归零，不能由 worker completed/任意费用或其他角色 publication 抵销。若需要免不存在的下一 local round headroom，专属加入私有 one-shot source enum，必须完整原 input/output 与实际 wire/system/step 验真；Generic/Single 仍原规则，不加 caller bool。所有实际 reserve/hard ceiling/child inventory 守卫保持。

不能先移除 reserved gate，再补实际 producer；不能因为底层允许只读 worker 就打开 Root dispatch。闭环可先使用明确受限 Rust producer，A04 Root dispatch 的 prepare/grant 合同是另一个门槛。

## 5. 源事实稳定性

collection pre/post stablefact 包括原请求/模型结果/usage、input 引用、scope、原 worker 身份、实际原费用与 clock 来源、输出语义。状态、settlement timestamp、cap revoked/lane、slot release 分开验证，不能因正常收口变化被误认为新事件。Root 自身评估费用和 TTL 不进入 child changed-fact，避免自身费用再次触发。

原 SDK bill 先保存，semantic invalid 不退款；收到 unknown 不能被零预算/零字段当作已结算。取消若发生在返回完整 response 前，保原 forfeit/indeterminate；完整 known response 已返回后失去 scope，账单保留且禁止新 publication。运行恢复与失败/过期免除义务需独立原终态合同，不能以普通 completed flags 推断。

## 6. 最小实际验收

- 正向真实 born v13 → 原 frozen 输入 → 1 BusinessLogic SDK → 真实费用 → 同事务签输出/ACK/撤权/lane release → 1 Root paid assessment；target HTTP=0，独立 Reviewer floor 不被侵占。
- 多条原输入及缺证输入；错误或未闭合来源拒/不签/不发新 SDK；模型引用其他 target/worker 或确认漏洞严格拒。
- 冷改原输入/模型 JSON/hash/usage/checkpoint/receipt fee/ACK/closure/seal、foreign scope、missing birth 或任何 rule、旧 Root saved JSON 注入均拒且不 backfill。
- 同 source 首次/付费重放全表零写、SDK/HTTP/费用不增；Root publication 失败只保原费用与 pending，不重复 child SDK。
- Root actual tools response 拒前 LocalStep、known invoice 原保存、publication0/pending1；Root paid 期间 pause/cancel、成功后 paused historical replay。
- 原出生低预算近限正向与不足负向，不冷改 hard cap；Generic/Single 不受例外影响；clock 未采样纯读验证。
- shared writer INSERT/trigger 故障保持原已收 SDK 费用，不伪完成；paused/expired 不重新获得两个能力或原 worker。

建议独占：输入/collector、output seal+schema+typed signer、frame/hook+obligation/tests 分叶；shared bootstrap/include/readonly completion/model admission/classifier 与生产 caller 由主控统一接。只有原实际测试通过后的生产闭环才能更新功能状态；本文件本身仅设计与只读证据。
