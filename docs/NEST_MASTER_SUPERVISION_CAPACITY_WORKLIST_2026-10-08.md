## 2026-10-08 REM-A03 新任务监督请求容量准入（局部完成，Master 未完成）

- 真实功能：Native JSON 新出生 bootstrap v6，按当前共享请求余额保留后续最小容量；每条 Web HTTP 在原 claim 事务内重验，跨 worker 计数，仅原付费 new-evidence publication 解除义务。容量不足在 IO 前拒绝，已付 Web/HTTP 费用与结算保留；旧 v1–v5 不升级、不补账，缺 v6 sidecar 不可降级绕过。
- 测试证据：扩大回归 140/140；后续编译快照的关键入口及旧合同原所有权 21/21，均 localhost 实际 SDK/HTTP、0 忽略。旧预算边界保留原额度，两个派发测试改为真实 SDK 退出后结算。不同源码快照与外部归属并行改动边界详见专项文档，不称整库/第三方/安装 App 通过。
- 编译：最新前端 build、主库测试编译、应用 build、Clippy、最终完整 Rust 类型检查均 exit 0；应用普通构建 18 warnings，Clippy 19 warnings / 0 errors。只读增量/指纹复核确认本片 23 源码路径稳定，未覆盖外部 Asset/归属改动。
- 尚未完成：token/USD/其它维度的完整动态监督容量、任意 Root 多轮/重试、实际并行争抢及全部后续 SDK 准入；dedicated recovery birth 仍待实施，普通恢复禁令不解除。下一片先补各维容量及并发/unknown/撤权组合，再按 Master 继续专属恢复出生；完整 REM-A01–A14 未完成。
- 详见 docs/NEST_MASTER_SUPERVISION_REQUEST_CAPACITY_2026-10-08.md；证据 `/tmp/oviraptor-master-supervision-admission-20261008/`。不提交/推送、不替换安装 App、不改真实库/CAS、不删资产；Native JSON 与 Qoder Exposure 分工保持。

## 实施前的只读基线（历史诊断，以下保留原文）

# REM-A03 多事实监督容量：生产源码复核

当前生产事实：bootstrap.rs v5 rootSupervisionTokenFloor=16000、requestFloor=1；coordinator_executor_allocation.rs 对该原合同保留 31000/2（含另一基础 floor），不是财务 Reserve。coordinator_new_evidence_hook.rs 对 Executor 每个 observed request_record 独立执行真实付费 Root tick，按 rowid,id 顺序，每个原物理事实都要独立核验和重放。execution.rs 顺序仍为结算、new_evidence、ClientSide、InputParser。

因此固定 floor 不能保证任意多个原 HTTP 事实与后续角色监督可用；成功路径不能靠提高测试额度或将事实合并换绿。现有容量不足的拒绝及已结算 Web 支出必须保留。

下一实施应在真实准入时从剩余共享财务确定可调度的监督与目标工作：每次产生新物理请求前保证它对应的真实 Root 付款容量，保留 Reviewer/ClientSide/InputParser 等独立工作必要余额；未知未来事件必须继续准入检查，不预言无界预算。各维度独立，null 原无限语义保留。不得把 retained floor 当已付款或写伪 Reserve。

原 Native JSON 合同是出生冻结且严格校验，不可修改已存在 Root。若需要新创建协议，必须明确新版本及旧版本原字节验证，不在读取时升级。原历史授权/财务链不得靠 cfg(test) backfill。

验收：实际多个同 revision HTTP 事实、不同 identity、ClientSide 和 InputParser 独立完成；重放零新 SDK/费用；余额不足在对应新动作前拒绝；并发容量争抢、未知响应、原物理票据篡改、撤权/过期保留原支出与失败阶段。至少需要实际 run_agent_target 入口证据，不只有直接 helper。

本文件是只读诊断和实施约束，没有完成动态预算功能。Rust 当前受 Qoder Exposure export.rs:164 编译错误阻碍，仍有六项待验源码修正。
