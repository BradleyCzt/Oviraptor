# 架构与代码入口

本文描述第二轮业务迁移后的当前源码边界，不是安装包发布验收声明。行为基线为保留的
WIP `d61454cdf73c52d190d3d4a26d6733356a6495af`，不以 main 的配置默认值替换。
当前源码冻结为 `525f1283adae79a13bcf0e07cf50772841911fb0`。第二轮实现、分批提交、失败尝试和当前验证状态见
[业务模块报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)；第一轮历史记录仍见
[原集成报告](BUSINESS_MODULE_INTEGRATION_2026-10-14.md)。原设计审计见
[NEST_DESIGN_AUDIT_2026-10-13.md](NEST_DESIGN_AUDIT_2026-10-13.md)。

本轮在独立集成分支保留 WIP，未合并 main。本地移交包仍未取得或整合；现有接口 workspace_path=null、仅 signed URL，下载域名放行未确认，本轮无新下载重试，不能把云端回归表述为已覆盖用户本地应急源码。完整 Native、平台和安装包结果须各自核对。

## 当前 owner 与入口

| 入口 | 责任 |
| --- | --- |
| `src/App.vue`、`src/shell/` | 默认 Chat 导航、内置模块和应用组合；允许明确的跨 owner 展示与协调。 |
| `src/features/*/api.ts`、各 owner `types.ts` | 领域 IPC 与数据契约。根 `api.ts`、`types.ts` 保留兼容门面，`sentinelApi` 保留显式聚合。 |
| `src/shared/`、基础 CSS / 通用词条 | 确实复用的 UI、事件协调、设计原语和操作词；不收纳业务 DTO、执行入口或领域流程。 |
| `src-tauri/src/lib.rs` | Tauri 启动、AppState、实际 owner 命令注册和应用生命周期。 |
| `src-tauri/src/commands.rs` | 12 个 owner 的命令索引；不再持有扫描/Agent 的共享执行作用域。 |
| `commands::workspace_projects` | 项目身份、归档、删除与跨业务 impact / 删除保护。 |
| `commands::assets`、`commands::asset_export` | 资产、目标、采集 jobs/logs/events、中断恢复与导出。 |
| `commands::ownership`、`commands::exposure` | 资产归属/目录/复核/授权，以及暴露面采集、证据、轮次和恢复。Exposure 通过明确接口读取归属结果。 |
| `asset_fofa`、`commands::hackerone` | FOFA 请求/配置测试；HackerOne 缓存、同步与显式项目导入。 |
| `commands::application_overview/application_settings` | Shell 指标聚合和应用设置。 |
| `commands::workspace_config_profiles/model_profiles/environment/tool_administrator` | 配置 profile、模型测试、环境准备与管理员工具库存；准备状态不授予任务执行权限。 |
| `src-tauri/src/scanning.rs` | 扫描准入、Web/Code/Greybox/CI、工作台、原 Native Agent、调查、证据、预算、费用、租约、授权、恢复和重放。 |
| `conversations`、`skills_catalog`、`skill_packages` | 独立项目 Chat；已存指令/知识/来源与复核；声明式 Skill 包完整性、导入、资源和撤销。扫描 producer 仍归 Scanning。 |
| 各业务 `model.rs`、`models.rs` | DTO 归 owner，旧 `models.rs` 兼容重导出；移位不改变 wire schema 或 capability。 |
| `model_configuration`、`worker_resources` | 从调用者配置快照解析连接/模型 gateway；随包 helper 定位、Node 选择和完整性机制。 |
| `agent_runtime/`、`native_pipeline/`、`tool_supply/` | 原 Native 状态/协议、调度与执行、工具计划和沙箱边界；整棵 runtime 不是无业务 common。 |
| `db.rs`、`db_schema.rs`、`db_*` | 连接、基础 schema、初始化、迁移与修复；领域授权和持久化不变量继续由 owner 约束。 |
| `auth_session.rs`、`jobs/`、`worker.rs`、`artifact_import/` | 身份会话、采集/探测与远程 Worker、支持的 artifact 导入，各自保留原合同。 |

详细文件归属见 [Rust 指南](RUST_BUSINESS_MODULE_BOUNDARIES_2026-10-14.md)和
[前端指南](FRONTEND_MODULE_OWNERSHIP_2026-10-14.md)。物理文件仍位于 `commands/` 不代表
其属于 `commands` 模块；应以 `mod` / `include!` 接线判断实际 namespace。

## 依赖与新增代码

1. 从视图直接使用的 owner API 开始，沿 `lib.rs` 的 `tauri::generate_handler!` 注册查
   Rust owner、输入/返回 DTO 与 helper。根 API 的 224 个方法和 289 个 IPC 终端名保留；
   不用 TypeScript 编译代替 Rust/TS 契约核对。
2. Scanning 内部保留连贯业务作用域中的原 `include!`，没有重写通用 Agent。
   外部通过八组生产端口或原 typed 模块依赖所需合同；八组端口共导出 25 个命名符号，
   包括 23 个函数、一个结构体和一个常量。四个 owner 的 test_support 中另有 8 个
   `cfg(test)` 薄适配函数，不是 8 个模块；跨 owner 测试不能扩大生产 `pub(super)`、私有 witness 或 writer 的可见性。
3. Chat 使用模型基础设施，但保留自身的项目、recipient/endpoint consent、Skill grant
   和工具循环；不初始化扫描工作台。配置页可以显式组合模型、FOFA、Worker 与规则包。
   原沙箱继续是内部能力，不形成独立用户业务入口。
4. Core `ModelConnection` 仅有 `llm/api_key/api_base/deployment` 四字段。Scan 的六字段
   环境快照、九字段本地资源策略、full_power、prompt audit、材料字节预算和 startup
   idle/hard 时限仍归 Scan。基础机制复用不应把扫描策略移入 common。
5. Tauri 注册或前端禁用按钮不能替代后端授权。执行、目标网络、费用或身份相关入口
   必须保留原 scope、lease、fencing、预算和收据校验。新增领域流程沿其 owner 组合，
   不放回全局 components、types 或共享命令作用域。
6. Shell dashboard、项目列表和 ProjectImpact 可以读取跨业务指标。项目删除必须检查
   已知和其他关联记录，不能为减少跨域依赖而删除保护。StartupStatus 虽由 Shell 展示，
   原 `startup_status` 会将未完成采集 runs 标为 interrupted；`db::initialize` 仍执行
   schema、迁移和 Native 修复/恢复。默认 Chat 不代表全启动路径只读。
7. 数据库变更先核对 owner、初始化入口、版本标记及部分失败/重入。通用 DB 初始化
   与 Exposure feature 初始化不是同一执行器；不可仅以目录迁移删除其中任一阶段。

## Native 生命周期的关键入口

- `agent_runtime/contract.rs` 描述状态和协议词汇，`reducer.rs` /
  `reducer/backend_exit.rs` 归约结果。
- `agent_runtime/single_projection.rs` 负责 Single 原终态发布；私有连接、事务、受限
  authorizer、财务收据及后置条件共同保护持久化，不能用测试 helper `reducer::commit`
  概括生产入口。
- `agent_runtime/multi_agent/scheduler/lifecycle.rs` 在事务内校验 Child 状态/租约、结束
  assignment/run、释放资源并验证预算和事件；必须连同前置查询阅读最后的 UPDATE。
- `agent_runtime/multi_agent/budget/` 与调度、派发、收据结算调用者共同拥有费用契约；
  底层 transport 不直接记账不等于没有预算门禁。
- 物理文件 `commands/multi_agent_gap_recovery.rs` 与 `multi_agent_review_recovery.rs`
  接入 Scanning 内部作用域，可在原收据和冻结输入匹配后完成本地交付；不得把恢复
  变成新模型执行、重授能力或重复收费。

新增状态写入应明确 owner、原始证据、事务边界、允许转换及可观察结果。不要仅按
terminal_state 非空设置统一不可变规则，以免破坏合法暂停和收据恢复。既有付费后
成功 Resume/Retry 仍缺财务续跑合同；安全拒绝回归不能解释为该产品能力已实现。

## 验证与证据边界

开发入口见 [README](../README.md)。先运行生产前端构建和 all-targets check，再执行
变更相关的 Rust/Node 测试。Native 选择器应匹配实际 owner namespace，必须检查真实
case 名字与数量；零匹配退出成功不构成通过。源码 pin 可保护接线/禁用边界，不能
代替运行测试，也不能为过测放宽固定 scopes、类别或 active baseline。

阶段结果必须保留实际工作树来源：Phase 2 的 25 条 Node owner 守卫中有 5 条使用当时
未提交的前端 WIP，不等于全部对应 phase2 clean commit；实际 baseHead/输入 SHA 见
result/快照，关联提交按 body/hash。28 项 rename-aware 变更含 3 项 rename、共 31 个
原/新路径，不以单一计数推断文件丢失。

当前 525f128 的 all-targets 0 / 15.32s、opt1 library build 0 / 88.56s、完整 Node
1,535 pass / 12 skip / 0 fail（exit 0 / 126.26s）已实际完成；80 次原 HTTP 用例执行
全部通过，Exposure + retirement 精确 230 项为 226 pass / 4 既有 ignored / 0 fail。
Phase 9 只修 fixture 的 Connection close 声明，不改生产、断言或 timeout。旧 bb200ca
全量主动中止，outer 143、1,849 rows、2 FAILED、7 ignored，summary=null；新
`final-native-accepted` 新无过滤全量已真实完成：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。
source525f128 / default-feature library / opt_level=1 / 8线程 / debug_assertions和overflow_checks=true；native/audit/tracked 1,800/2,218/2,569 个路径及全部 SHA 在运行前后精确一致，binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c 不变。严格索引再次核对三项 retirement
门禁实际通过及原固定 source scopes / 空 active baseline。旧中止历史不改变。

阶段定向、完整 suite、浏览器合成 IPC、实际 Native、平台和安装包验收分别报告。
第二轮当前结果以[业务模块报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)及最终输入
哈希/日志为准，不将第一轮完整通过数转用为当前代码。Linux 拒绝负例不能代替 Mac
exclusive-publication 正例，真实 Chromium 合成 IPC 不代替真实 Tauri/用户库运行。
发布包来源及运行核对继续遵循 README 的构建观察与安装包流程。
