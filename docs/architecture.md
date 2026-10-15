# 架构与代码入口

本文描述云端业务拆分与本地增量整合后的当前源码边界，不是安装包或真实用户数据库验收声明。
行为基线保留 WIP `d61454cdf73c52d190d3d4a26d6733356a6495af`，不以 main 的默认配置替换。
新受测源码为 `24179702c9d163e1d21bf9a23c4528589355166a`，直接父提交为 `012425b06845f476fda06096d1ade11aa76466ef`。
241 仅修改退休注册测试守卫及其 allowlist 两个文件，生产实现不变；最后实际结果、阶段失败、测试合同替换和剩余风险见
[本地增量整合报告](LOCAL_INCREMENTAL_INTEGRATION_2026-10-15.md)与
[最终验证索引](local-incremental-20261015/final-verification.json)。

本地传输阻塞已解除。27 个 tracked 修改与 24 个新增路径已经按当前 owner 整合，候选与云端两方历史由
`677ad23873d241507395e146aa51a565f95b2d10` 的 merge 保留；后续业务修复、历史夹具闭包与测试精确化分批提交。
原始本地基准为 `8fddee2321b313a8a6372d0895dd4c1f533ba57c`，过滤恢复根不是 Git 共同祖先；
实际 merge base 是 `525f1283adae79a13bcf0e07cf50772841911fb0`。路径和契约见
[51 项原始增量映射](local-incremental-20261015/mapping.json)及
[最终模块与契约映射](local-incremental-20261015/final-module-contract-map.json)。
当前整合在独立分支 `integration/business-modules-20261014`；本地候选曾单独在
`candidate/local-incremental-20261015` 审阅，现已保留历史地整合。没有合并 main、修改用户 Mac 或覆盖其本地源码。

[第二轮云端业务模块报告](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md)保留 `525f128…` 的真实历史结果，
其中“本地尚未取得/整合”仅适用于该旧阶段，不是当前阻塞。[第一轮原集成报告](BUSINESS_MODULE_INTEGRATION_2026-10-14.md)
和 [NEST 设计审计](NEST_DESIGN_AUDIT_2026-10-13.md)继续保留来源；历史通过数不得转作当前源码验收。012 的 full02 完整执行后单项失败与 241 的测试修复亦须分开记账。

## 当前 owner 与入口

| 入口 | 责任 |
| --- | --- |
| `src/App.vue`、`src/shell/` | 默认 Chat 导航、内置模块和应用组合；允许明确的跨 owner 展示与协调。 |
| `src/features/*/api.ts`、各 owner `types.ts` | 领域 IPC 与数据契约。根 `api.ts`、`types.ts` 保留兼容门面，`sentinelApi` 保留显式聚合。 |
| `src/shared/`、基础 CSS / 通用词条 | 确实复用的 UI、事件协调、设计原语和操作词；不收纳业务 DTO、执行入口或领域流程。 |
| `src-tauri/src/lib.rs` | Tauri 启动、AppState、实际 owner 命令注册和应用生命周期。 |
| `src-tauri/src/commands.rs` | 14 个显式命令 owner 的薄索引；扫描、Skills、FOFA 等独立 root owner 另行注册，不混同为这 14 个模块。索引不容纳业务函数或共享实现作用域。 |
| `commands::workspace_projects` | 项目身份、归档、删除与跨业务 impact / 删除保护。 |
| `commands::assets`、`commands::asset_export` | 资产、目标、采集 jobs/logs/events、中断原 run 恢复与原 URL 导出；响应折叠只是展示，精确成员由完整筛选与可选代表锚点限定。 |
| `commands::page_isolation`、`commands::asset_rule_catalog` | Assets 页面隔离的预览/令牌/apply/停用恢复与规则目录；域实现仍在 `page_isolation.rs`，不是配置或扫描的通用规则桶。 |
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

这 14 个 index 模块是 workspace_projects、application_overview、application_settings、workspace_config_profiles、
assets、asset_rule_catalog、page_isolation、asset_export、tool_administrator、environment、model_profiles、
ownership、exposure、hackerone。`exposure` 保留默认 Rust 模块解析以定位自己的子模块。

详细文件归属见 [最终模块与契约映射](local-incremental-20261015/final-module-contract-map.json)、
[Rust 指南](RUST_BUSINESS_MODULE_BOUNDARIES_2026-10-14.md)和[前端指南](FRONTEND_MODULE_OWNERSHIP_2026-10-14.md)。
物理文件仍位于 `commands/` 不代表其属于 `commands` 模块；应以 `mod` / `include!` 接线判断实际 namespace。
本地资产 UI、DTO/API 增量分别落入 `features/assets/`、`assets/model.rs` 等实际 owner，
Rules 由设置导航提供入口但不转移 Assets 规则归属。配置保留 Plugins、完整 profile 及整合基线默认值。

## 依赖与新增代码

1. 从视图直接使用的 owner API 开始，沿 `lib.rs` 的 `tauri::generate_handler!` 注册查
   Rust owner、输入/返回 DTO 与 helper。当前有 229 个前端 API 方法与 294 个 IPC 终端；
   原 224 个方法及原 289 个 IPC 的名称、相对顺序、owner 与 wire 契约保留。恰好增加
   preview_page_rule、apply_page_rule、list_page_rules、disable_page_rule、get_asset_rule_catalog 五项，
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
   页面隔离的 apply/import/恢复沿用原资产事务，保护后续人工决定、归属授权和调查状态。
   公开 list_assets 先 db::open，再由 list_assets_on 首句验证查询，之后才开启查询事务/读取资产快照；
   export_assets 则在 db::open 和输出目录/文件创建之前验证同一 query。不能把两条入口都称为开库前验证。
8. Chat 的 agent_enabled、recipient 同意、技能选择策略由 conversations/access 拥有；
   generation/tombstone 在 conversations/access/schema.sql，skill_packages 负责 package资源/provenance指纹。
   Chat model-facing分页最多4096内容bytes、最终序列化JSON envelope32768bytes；直接资源IPC默认4096/最大8192。
   选择上限128、列表每页12与内置36技能/443文件元数据属于不同合同；不宣称128/129或32768/32769专门边界均已测。
9. full/collect/reprobe 资产恢复使用原 runId 和检查点，未支持的非空 pipeline 在 ACK/start/resume 前拒绝。
   空 pipeline 仅保留合成 fixture 验证的历史 UI fallback，不宣称真实历史迁移。
   前端按项目/示例/操作 generation 拒绝迟到预览与目录结果，响应锚点随分组清除和项目切换同步清除。

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

**历史快照 `525f128…` 的已完成结果（保留原事实，不代表最后 012 源码）：**

该快照的 all-targets 0 / 15.32s、opt1 library build 0 / 88.56s、完整 Node
1,535 pass / 12 skip / 0 fail（exit 0 / 126.26s）已实际完成；80 次原 HTTP 用例执行
全部通过，Exposure + retirement 精确 230 项为 226 pass / 4 既有 ignored / 0 fail。
Phase 9 只修 fixture 的 Connection close 声明，不改生产、断言或 timeout。旧 bb200ca
全量主动中止，outer 143、1,849 rows、2 FAILED、7 ignored，summary=null；新
`final-native-accepted` 新无过滤全量已真实完成：3,602 个唯一 case 全部恰执行一次：3,595 pass、7 个原 ignored、0 fail、0 filtered；outer/command/script 均 exit 0；Rust 1555.55s / Native command 1555.78s / collector 1555.91s。
source525f128 / default-feature library / opt_level=1 / 8线程 / debug_assertions和overflow_checks=true；native/audit/tracked 1,800/2,218/2,569 个路径及全部 SHA 在运行前后精确一致，binary SHA-256 474e15f2d28932a485b020fcdda03204c9d5b3b8cdf1c62485b9e0efd0d0fd1c 不变。严格索引再次核对三项 retirement
门禁实际通过及原固定 source scopes / 空 active baseline。旧中止历史不改变。

**本地整合后的阶段来源与验证边界：** `012425b…` 已实际完成 npm build
（exit 0 / 31.080s）、all-targets check（exit 0 / 19.839s）、优化 library no-run
（exit 0 / 277.342s）和原生定向 140/140（exit 0 / 6.238s）；关键新增 36 项全部实际通过，
含 3 个完整历史初始化与 7 个官方 MockRuntime 生成 handler 测试，前后完整输入不变。
012 的未过滤 full02 实际完整执行 3,725 项：3,717 pass、1 fail、7 原 ignored、0 filtered；
outer/command exit101，Rust1542.76s / command1542.914s，source/全部输入/binary/helpers稳定。
唯一失败为旧退休注册守卫硬码289，而正式五项新入口使registry达到294，并非退休入口重注册。
新源241明确校验五个精确owner各出现一次、294总数和剔除五项后的289保留数；
旧unique/live/退休/隐式根断言tail保留，allowlist仅该fixture的reason/SHA更新，scope/category/count/空baseline不变。
独立1正例+33负例是静态控制，不当作新守卫原生执行。

新源241的build05/Node05/browser05/check05、target04、guard-exact01和full03需依据实际完成结果记录，
不从012或静态控制推定成功。完整最终 Native/Node/Chromium gate 以
[本地增量整合报告](LOCAL_INCREMENTAL_INTEGRATION_2026-10-15.md)和
[最终验证索引](local-incremental-20261015/final-verification.json)中的对应源码、产物和日志为准。

当前原生列名为 3,725：3,601 原名保留、1 项已审必要 optimizer 合同替换、123 项全新。
该替换保留 HTTP/HTTPS 请求原 URL，不能称为旧 Python 折叠合同的等价改名；原始断言比较及唯一替换映射留存证据。
首次 full 预检只因旧名缺失拒绝启动，执行 0 测试，不是原生 full 失败，也不能充当 full 通过。

阶段定向、完整 suite、浏览器合成 IPC、实际 Native、平台和安装包验收分别报告。
不把旧525、第一轮或012的失败full转用为241通过。Linux 拒绝负例不能代替 Mac exclusive-publication 正例；
真实 Chromium 合成 IPC、官方 MockRuntime/临时 SQLite 不代替真实 Wry/桌面 IPC、用户数据库或真实 provider。
付费 Agent Resume/Retry 成功财务合同、旧无哈希有效截断 CSV、永久 DB 写失败下的 durable 恢复及平台实测边界
继续见新报告。发布包来源及运行核对遵循 README 的构建观察与安装包流程。
