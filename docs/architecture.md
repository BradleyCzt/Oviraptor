# 架构与代码入口

本文是基于 `8fddee2` 源码补充的当前入口说明，不是已删除架构文档的原文恢复，
也不代表所有模块已完成发布验收。设计审计及证据边界见
[`NEST_DESIGN_AUDIT_2026-10-13.md`](NEST_DESIGN_AUDIT_2026-10-13.md)。

2026-10-14 集成分支的最新模块归属、源码来源、验证记录与本地移交阻塞见
[`BUSINESS_MODULE_INTEGRATION_2026-10-14.md`](BUSINESS_MODULE_INTEGRATION_2026-10-14.md)。
该分支保留 `d61454c` WIP，未合并 main，尚不是已验收安装包。

## 主要边界

| 入口 | 责任 |
|---|---|
| `src/App.vue`、`src/shell/`、`src/features/` | 主应用组合、导航与按业务归属的 Vue 视图 |
| `src/shared/components/` | 无业务依赖的基础弹窗、确认组件 |
| `src/features/*/api.ts`、各 owner `types.ts` | 领域 IPC 调用及 TypeScript 数据契约；根 `api.ts`、`types.ts` 保留兼容门面 |
| `src-tauri/src/lib.rs` | Tauri 启动、共享 AppState、命令注册与应用生命周期 |
| `src-tauri/src/commands.rs`、`commands/` | 命令门面与业务编排；混用 include 和真实子模块 |
| `src-tauri/src/agent_runtime/` | Native 运行契约、事件/状态、调度、预算、租约、收据和恢复 |
| `src-tauri/src/native_pipeline/`、`tool_supply/` | Native 分析执行、工具计划、批准与沙箱边界 |
| `src-tauri/src/db.rs`、`db_schema.rs`、`db_*` | 连接、基础 schema、初始化、迁移与修复 |
| `src-tauri/src/commands/exposure/` | 暴露面分析的独立模块树、schema、工作项、恢复和证据 |
| `src-tauri/src/auth_session.rs` | 浏览器身份捕获、持久化、作用域绑定与会话校验 |
| `src-tauri/src/jobs/`、`worker.rs` | 采集/探测任务和远程 Worker 服务 |
| `src-tauri/src/artifact_import/` | 支持的 artifact 导入与材料验证 |

## 命令归属与新增代码

1. 从 `src/api.ts` 的调用进入 `lib.rs` 的 `invoke_handler!` 注册，再定位 Rust 命令及
   其领域 helper。同步检查输入、返回值、错误和 `src/types.ts`；TS 编译不能单独证明
   Rust/TS 契约一致。
2. `include!` 不创建文件级命名空间，但包含内容可能声明子模块或处于 cfg(test) 下。
   不把目录文件数当作模块数。新增业务优先沿现有领域边界设置接口和最小可见性，
   避免为跨文件调用直接扩大为公共 API。
3. Tauri 注册或前端禁用按钮不能替代后端授权。涉及执行、目标网络、费用或身份的
   命令必须复用相应 scope、lease、fencing、预算和收据校验。
4. 数据库变更先定位初始化入口与现有版本标记，检查旧库、部分失败和重入行为。
   `db.rs` 的通用初始化与 exposure 的 feature 初始化不是同一执行器。
5. 测试应跟随真实模块/cfg/平台边界，优先验证行为、不变量和失败恢复。源码 pin
   可保护特定接线/禁用边界，不能代替运行测试，也不能不经评估统一删除。

## Native 生命周期的关键入口

- `contract.rs` 描述状态和协议词汇，`reducer.rs` / `reducer/backend_exit.rs` 归约结果
- `single_projection.rs` 负责 Single 原终态发布；私有连接、事务、受限 authorizer、
  财务收据及后置条件共同保护持久化，不能用测试 helper `reducer::commit` 概括生产入口
- `multi_agent/scheduler/lifecycle.rs` 在事务内校验 Child 状态/租约、结束 assignment/run、
  释放资源并验证预算和事件；必须连同前置查询阅读最后的 UPDATE
- `multi_agent/budget/` 与调度、派发、收据结算调用者共同拥有费用契约；底层 transport
  不直接记账不等于没有预算门禁
- `commands/multi_agent_gap_recovery.rs` 与 `multi_agent_review_recovery.rs` 可在原收据
  和冻结输入匹配后完成本地交付；不得把此类恢复变成新模型执行、重授能力或重复收费

新增状态写入应明确其 owner、原始证据、事务边界、允许转换及完成后可观察的结果。
不要仅按 terminal_state 非空设置统一不可变规则，以免破坏合法的暂停和收据恢复语义。

## 验证入口

基本开发与平台要求见 [README](../README.md)。通常先运行 `npm run build` 与
`cargo check --manifest-path src-tauri/Cargo.toml`，再运行变更相关的 Rust/Node 测试。
完整测试、目标平台测试和最终安装包验收各自报告；过滤测试或某平台零匹配不构成
全量通过。发布包的来源和运行验证须遵循 README 中的构建观察与安装包核对流程。
