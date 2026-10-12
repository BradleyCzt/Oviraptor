## 2026-10-08 当前接续检查点

专属 Recovery birth 仍未接入；旧 Exposure 编译阻碍已解除，当前应用 build / Clippy 已成功。两项普通 Single 付费恢复正向仍因缺显式财务合同失败，不放宽门禁。完整实施规范见 NEST_MASTER_RECOVERY_SHARED_FINANCE_DESIGN_2026-10-08.md：新的原批准费率/model 绑定、精确 USD、单 URL 有界出生/真实付款后扩至多 Root 共享消费/时钟，旧 pending 草稿不读取升级。该规范为设计，尚未实施。

本轮 REM-A03 容量/诊断及回归证据见 NEST_MASTER_CAPACITY_CONCURRENCY_2026-10-08.md。以下为此前只读基线，文末“六项待验/Exposure 编译错误”仅是当时状态，不代表当前仍不可编译。

# Recovery 专用出生实施清单（当前源码审计）

当前不是执行闭环：scope_budget 保存签名共享预算合同；signature 草案仍 pending；scan_startup、scan_execution_frontend、native_web_mode_plan 显式拒绝通过普通启动路径执行 Recovery。UI 模拟 IPC 118 项通过不能替代后端执行证据。

1. 从原 approval/request/draft 签名、scope hash 和 cancellation 状态加载原授权，使用一致事务重新验证；不要读取时升级历史草案。
2. 实现专用 Fresh birth 原子入口：绑定新 scan/attempt 与签名 scope owner，Root 原始控制与十维限制必须在第一次 INSERT 时存在，失败应不留可执行半成品。
3. frontend、每个 Root 及子任务使用同一 scope 总财务与起点时钟；独立 token 维度及 USD/deadline 必须进入真实 reserve/consume/refuse，不能仅保存 JSON。
4. 将已完成专用出生的原请求作为 human-recovery 监督事实接入；原 UUID 幂等重放只能读取原收据，不能再次生成 Root 或追加预算。
5. 保留普通启动拒绝，直到专用路径具备实际消费者。不要单改 executionAllowed/status/targetRequestsGranted。
6. 正向验收需真实 Root 与独立模型/目标调用、原付款收据和余额；负向覆盖跨 scope、撤权、deadline、并行争抢共享余额、事务故障、丢 ACK、崩溃重入以及原始历史篡改。

源码锚点：src-tauri/src/commands/native_protection_recovery_scope_budget.rs、native_protection_recovery_signature.rs、native_protection_recovery_draft.rs、scan_startup.rs、scan_execution_frontend.rs、native_web_mode_plan.rs。

此清单为只读审计和下一步实施边界，不是新功能、生产验收或 Master 完成声明。当前 Rust 待验六项；外部 Exposure export.rs:164 编译错误仍在。无资产删除、无自动提交。
