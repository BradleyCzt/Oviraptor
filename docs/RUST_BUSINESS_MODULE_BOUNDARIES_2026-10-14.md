# Rust 业务模块归属与依赖边界

行为基线为保留的 WIP `d61454cdf73c52d190d3d4a26d6733356a6495af`。原生 owner 迁移提交为 `7cfcea9b7f22f780508e6e528a9216fbaf4e4386`，当前验证源码冻结为 `525f1283adae79a13bcf0e07cf50772841911fb0`；不使用 main 默认值替换 WIP，不重写原 Native Agent。

## 实际 owner

`src-tauri/src/commands.rs` 是包含 12 个 owner 的命令索引，不再承载扫描/Agent 的共享执行作用域。物理上仍位于 `commands/` 的文件，实际所属模块以 module/include 接线为准。

| Owner / 入口 | 责任与允许依赖 |
| --- | --- |
| `commands::workspace_projects` | 项目身份、归档、删除和 impact。跨业务关联计数保护项目删除，包含未分类关联表；不初始化资产、扫描或模型执行。 |
| `commands::assets` | 目标、资产查询与人工判定、采集 jobs/logs/events、中断任务恢复。依赖资产 DTO、DB 和日志机制。 |
| `commands::asset_export` | 资产 CSV 导出，显式复用 assets 的查询条件、SELECT 和 row mapper，保持与列表相同的作用域。 |
| `commands::ownership` | 资产归属 profile、组织目录、分组、复核、持久决策、授权/撤销和只读 unit projection。其 source units 只共享归属领域作用域；Exposure 通过显式读取接口消费结果。 |
| `commands::exposure` | 暴露面采集、source adapters、缓存、证据、轮次、复核、导入/导出与恢复。facade 和 collection kernel 是同一实际 owner；子模块继续按 `commands/exposure/` 正常解析。 |
| `asset_fofa` / `assets/fofa.rs` | FOFA 账户请求与连接测试，供资产采集、配置测试等显式消费者使用；不再借用 Scanning 的私有函数。 |
| `commands::hackerone` | HackerOne 缓存、同步、项目导入和历史。保留显式项目选择；不因外部来源共用入口而授予扫描权限。 |
| `commands::application_overview` | Shell 的只读 dashboard 聚合，读取项目、资产和 run 指标。 |
| `commands::application_settings` | 外观、图标和提醒设置，依赖 DB、文件和图像机制。 |
| `commands::workspace_config_profiles` | 配置 profile 的读写/删除及原有未知字段保留。旧模块名兼容保留，业务归 Configuration。 |
| `commands::model_profiles` | 显式模型配置测试，复用模型基础设施；不是扫描启动入口。 |
| `commands::environment` | 管理员环境准备、日志、准备状态与显式恢复。原 OS guard 和持久 preparation lease 继续约束安装，任务不能自行安装工具。 |
| `commands::tool_administrator` | 签名工具库存批准、撤销和读取，调用 tool_supply 的明确服务；管理员准备状态不等于任务执行授权。 |
| `scanning` | 扫描准入、Web/Code/Greybox/CI 执行、原 Agent、预算与费用、权限、进程、证据、调查、恢复和重放。 |
| `skills_catalog` | 指令 CRUD/便携导入、知识来源/读取、候选复核和显式应用、转换/精炼。17 个原 IPC 由实际模块持有；扫描证据生产、候选生成与聚合仍归 Scanning。 |
| `skill_packages` | 声明式 Skill 包来源、完整性、导入、启停、资源分页与撤销。包或脚本内容不产生可执行 capability。 |
| `conversations` | 独立项目对话、历史、权限和模型工具循环。通过模型基础设施解析 provider，保留自己的项目/recipient/consent/tool-grant 合同。 |

12 个命令索引模块之外，Scanning、Skills、Chat、Skill packages 与 FOFA 均有自己的 crate module。`lib.rs` 使用实际 owner 路径注册；289 个 IPC 终端名称及顺序保持，125 个扫描入口转由 `scanning` 注册。前端命令字符串和参数不因 Rust namespace 迁移而更名。

## Scanning 内部组织与对外接口

`scanning.rs` 保留既有扫描 source units 的内部 `include!`。这些文件共享同一连贯业务作用域，仍是原有 Native Agent 实现；这里没有声称所有 include 都已独立，也没有引入通用 Agent 框架。

外部调用通过八组生产端口：`worker_bridge`、`scope_contract`、`source_deletion`、`financial_continuation`、`publication_replay`、`evidence_bridge`、`recovery_draft`、`log_events`。它们分别服务于远程扫描适配、冻结作用域、删除保护、原财务合同、原 publication 重放、证据、恢复草稿和日志事件，不提供无边界的 root wildcard export。八组端口导出 25 个命名符号：23 个函数、`FinancialContinuationAuthorization` 结构体和 `BUSINESS_LOGIC_READONLY_SYSTEM` 常量；另有两个 `cfg(test)` port 各导出一个测试函数。

原有 12 个 typed 模块继续保留，包括授权、HTTP、原输出收据、业务派发、Mapper 与恢复签名等 owner。迁移没有开放其私有 witness 字段或私有 writer，也没有改变预算、租约、费用和授权的所有权。

跨 owner 回归由四个 owner 的 `test_support` 提供 8 个 `cfg(test)` 薄适配函数，不是 8 个模块。原生产 helper 的 `pub(super)` 可见性及私有证明字段保留；测试端口不进入发行构建，也不能作为新的运行时许可。普通生产 owner 不通过 `use super::*` 恢复旧 commands 共享命名空间。扫描 owner 自身可使用其完整扫描 DTO，其他领域只引入所需契约。

## 模型、DTO 与基础设施

`model_configuration` 从调用者已有 SQLite 连接读取配置快照，统一 profile/endpoint/credential 解析与 gateway 构造。它不新开连接替换调用者未提交的设置，不借用其他 profile 的旧缓存，也不创建扫描或工具权限。

- Core `ModelConnection` 只有 `llm`、`api_key`、`api_base`、`deployment` 四字段。
- 扫描的 `ModelRuntimeEnv` 保留原六字段序列化合同，包括 `full_power` 和 `prompt_audit_mode`。
- 扫描的 `LocalModelRuntimePolicy` 保留原九字段：模型规模、内存、并发、上下文、输出、前端材料字节预算、memory guard、startup idle/hard 时限。基础模型资源计算可以复用，扫描材料与时限策略仍在 Scan owner。
- `worker_resources` 只负责随包 helper 定位、Node 版本选择、运行路径和完整性机制；不承接扫描业务授权。

原 `models_*.rs` 中 95 个 struct 和 7 个私有 default 函数已归 application、assets、scanning、workspaces、configuration、environment、hackerone、skills_catalog 八个 owner。原 serde 属性、字段、默认值和声明内容保持，无额外副本。`models.rs` 只作兼容重导出；DTO 移位不改变 wire schema，也不授予调用 capability。

DB、日志/framing、execution service、worker transport、auth session 和 artifact import 按各自合同提供机制；common/shared 不收纳具体资产、扫描或知识业务。

## 合法的跨业务读取与启动写入

Shell dashboard 和项目 impact 可以读取多个业务表。项目删除必须在存在资产、扫描、知识、认证会话或其他关联记录时拒绝；这些依赖是显式保护合同，不能为减少 import 或跨域 SQL 而删除。

启动路径并非全只读。`startup_status` 原有逻辑会将未完成的采集 runs 标记为 interrupted；`db::initialize` 仍执行 schema、迁移及 Native 状态修复/恢复。Shell 展示 StartupStatus 不代表底层零写入，默认启动 Chat 也不会绕开必要初始化。具体恢复动作仍由原 owner、原授权及持久化合同约束。

## 已执行验证与范围

| 批次 | 提交 | 已记录的原生定向结果 |
| --- | --- | --- |
| 模型连接与 Environment | `fc14c6c` | 102 通过；13 条边界守卫通过。 |
| Skills 生命周期与 native DTO | `0fefa15` | 149 通过；102 个声明内容保持。 |
| Ownership | `6cd48ce` | 284 通过，7 个既有 ignored。 |
| Exposure | `80566e1` | 286 通过，7 个既有 ignored。 |
| FOFA / HackerOne | `9085a9c` | 14 通过。 |
| Scanning 实际 owner | `7cfcea9` | 精确选择 552 项：545 通过、7 个既有 ignored、0 失败；Rust 50.48s。 |

各批集合有重叠，不能相加为独立测试总数。Phase 6 all-targets check exit 0 / 67.50s，library test build exit 0 / 124.91s；50 条 Node owner 守卫通过。17 个迁移后的精确 child selectors 均实际通过；完成收据继续拒绝“选中零测试却退出成功”。案例映射保留原 3,600 项逻辑身份，另加两项 ModelConfiguration 测试；阶段列举不代表执行；当前新无过滤全量另已实际完成全部 3,602 项，见下文独立最终证据。

Phase 2 的 28 项 rename-aware 变更包含 3 项 rename，合计 31 个原/新路径。该阶段 25 条 Node owner 守卫实际通过，其中 5 条读取当时未提交的前端 WIP；实际 baseHead 和输入 SHA 以 result/快照为准，不能把全部 25 条归到 phase2 clean commit。阶段关联按 body/hash；最终 clean 525f128 的完整 Node 独立记录为 1,535 pass / 12 skip / 0 fail。

Phase 6 的 retirement literal/active 和 retired-import 静态守卫已在定向集合实际通过。仅三项 fixture reason/hash 经逐项审查刷新；50 项路径/类别/literal 数量/pathMatch、固定 source scopes 和空 active baselines 保持。静态镜像本身不被当作原生测试通过。

当前 525f128 仅在 `commands/exposure/fixture.rs` 增加一行 `Connection: close`，与 fixture 原本一响应后关闭的生命周期一致；没有修改生产、业务断言、timeout 或客户端重试。旧 bb200ca 全量在两项失败与独立 A/B 实证后主动中止：outer 143，Native 563.00s，1,849 case rows、2 FAILED、7 ignored，summary=null，非完整 suite。Q2 精确诊断实际 exit 101 / Native 0.19s，G2 原全量 body 未取得，不能宣称相同精确断言。独立原 fixture 160 trials 中 24 次 OS104，对照 Connection close 为 160/160；这不代替应用执行。

525f128 当前真实 all-targets exit 0 / 15.32s，default-library opt1 build exit 0 / 88.56s，三类输入路径及 SHA 无变化。原两项 exact 用例 40 轮 / 80 executions 全部通过；完整 Exposure 227 + 3 门禁选中 230 项，226 pass / 4 既有 ignored / 0 fail，outer 0 / 22.85s。新无过滤 `final-native-accepted` 已实际完成：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。source525f128 / default-feature library / opt_level=1 / 8线程 / debug_assertions和overflow_checks=true；native/audit/tracked 1,800/2,218/2,569 个路径及全部 SHA 在运行前后精确一致，binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c 不变。完整全量中的三项 retirement/active/import 门禁均为实际通过，原静态边界在严格索引再次核对；旧不完整 run 与第一轮结果仍独立保留。

批次命令、输入哈希、binary/profile、精确名字集合、失败尝试和修复见第二轮验证记录的 `phase1` 至 `phase6` result/commit 文件。最终构建、完整 Node、Chromium 与完整 Native suite 的当前状态见[第二轮报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)，不将定向结果等同完整验收；Mac/Windows 包、真实模型/扫描目标及可选功能不由这些定向结果覆盖。

第一轮 `496b98f` 的完整验收及 `8c28105` 文档历史见[原集成报告](BUSINESS_MODULE_INTEGRATION_2026-10-14.md)，不把旧完整通过数转用为本版结果。本地源码移交包仍未取得和整合，接口 workspace_path=null、仅 signed URL，域名放行未确认且本轮无新下载重试；云端模块回归不代替未来三方语义整合，也不实现既有付费 Resume/Retry 尚缺的财务续跑合同。
