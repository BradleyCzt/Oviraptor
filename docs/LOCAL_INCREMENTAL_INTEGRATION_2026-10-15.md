# 本地增量与业务模块整合报告（2026-10-15）

> **云端回归已完成。** 受测源码为 clean `24179702c9d163e1d21bf9a23c4528589355166a`；完整未过滤原生、完整 Node、真实 Chromium 合成 IPC、构建与 check 均以实际完成结果核验，见第 6.4 节及最终验证索引。012/full02 等失败、来源与必要测试合同替换完整保留；结论范围为 Linux/default-feature library 与合成桌面接口，不代表真实安装包、用户数据库、平台 GUI 或 live provider 验收。

## 1. 本次整合结果与范围

本地移交包的传输阻塞已解除，原始 51 项本地增量已经按云端业务 owner 整合，并保留候选与云端双方的 Git 历史。当前新增能力包括资产高级筛选、完整响应展示分组及成员操作、项目页面隔离与规则目录，以及采集/重探测的原地恢复、输入保全和状态持久化修复。随后一次 30 文件修复提交补齐了前后端契约、历史初始化、人工状态保护与真实并发缺口。

本次沿用云端资产、扫描、聊天、技能、项目空间的模块拆分，不恢复旧 `commands.rs` 共享实现作用域，不将业务 DTO 塞入 common/shared，也没有重写通用 Agent。启动默认聊天、项目感知、内置资产与扫描、技能导入、配置及内部沙箱能力继续按原 owner 提供。

历史报告 [BUSINESS_MODULE_BOUNDARIES_2026-10-15.md](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md) 中“本地传输受阻、尚未整合”的结论属于先前云端快照。本报告记录其后已完成的传输和整合；旧报告保留为阶段记录，其旧测试数字不能替代本次最终验收。

当前整合发生在云端独立分支 `integration/business-modules-20261014`；本地候选此前在 `candidate/local-incremental-20261015` 独立审阅，现保留双方历史地整合。没有改动用户 Mac、覆盖用户本地源码或合并 main。

## 2. 可审查的来源与提交链

| 节点 | 精确引用及用途 |
| --- | --- |
| 原云端 WIP 基线 | `d61454cdf73c52d190d3d4a26d6733356a6495af`；保留此前项目感知聊天、技能及配置工作，不用 main 默认配置替换 |
| 原本地真实基准 | `8fddee2321b313a8a6372d0895dd4c1f533ba57c`；27 个 tracked 增量的基准 blob 均与此基准匹配 |
| 过滤恢复根 | `0d0f995a2b7312c28589c1d952ae33ffcbe045db`；仅用于恢复源码，**不是共同 Git 祖先**，未盲合并其无关历史 |
| 云端业务拆分代码 / 实际共同祖先 | `525f1283adae79a13bcf0e07cf50772841911fb0` |
| 本地候选代码 | `b2492bb94ce7aca1b102ffefc7b047dee819a2fe`；直接父提交为上述云端代码 |
| 本地候选文档 tip | `1df794453310247d2aca94214de24e5898cfb097`；交接、映射及候选验证记录 |
| 合并前云端文档 tip | `0d1ba51483f2379551446ab5a05fd61f21c7a01c` |
| 保留双方历史的整合提交 | `677ad23873d241507395e146aa51a565f95b2d10`；父提交分别为上述云端与候选 tip |
| 本轮 30 文件业务修复 | `f51226a8268764c073a09bd1242a799304ac7ed8`；直接父提交为 `677ad23…` |
| 资产边界守卫适配阶段 | `0f5856f6ced064de9d2f433b43b483ca8e375e57`；只改 `tools/test_rust_business_boundaries.cjs`，生产源码与父提交相同；新增共享校验/顺序/窄导入断言 |
| 历史夹具外键闭包修复 | `67b57d011c096a3acf08954f1bcd944bfb8285cf`；补齐精确旧 DDL 的 config_profiles/runs 两个 FK parent，生产实现及测试断言不变 |
| 取消复用断言修订 | `012425b06845f476fda06096d1ade11aa76466ef`；只改 cfg(test) native_workers 取消复用断言，生产与 optimizer 不变 |
| 新受测源码 / 退休注册守卫修复 | `24179702c9d163e1d21bf9a23c4528589355166a`；直接父提交为 012，仅改 `commands/tests_retired_import_commands.rs` 与 `tests/backend_retirement_allowlist.json`；生产未动，最后 gate 必须绑定此源 |
| 最终报告提交、远端交付 tip | 实际 docs-only SHA、远端分支/readback 及证据归档 SHA 由后续独立外部发布收据绑定；本文不预填自身未来提交或归档哈希，不能改变受测源码 |

移交包 `Oviraptor-local-source-8fddee2.tar.gz` 为 10,844,046 字节，SHA256 为 `fc6448afee2399a988679ff64db8d95a2c5b7439e78ecd0bed05d94535fa24af`。交接任务完成下载、脚本检查和独立目录恢复，外层及全部 10 个组件校验通过，恢复并逐项校验 2,090 个文件。本报告不包含原始签名下载 URL、凭据或用户数据。

51 项增量由 27 个 tracked 修改和 24 个新增源码/测试路径组成。按业务 owner 迁移后得到 52 个目标路径：原 `App.vue` 增量拆入 App 与 `AssetsPluginWorkspace.vue`。候选变更严格为这 52 个目标加 3 个必要测试适配，共 55 个路径，只有新增/修改，没有导入过滤树的删除。初步重叠为 18 个 tracked 路径，10 个文件的 21 处文本冲突按真实基准与业务语义解决。

逐项原路径、目标、类别与 SHA 见 [mapping.json](local-incremental-20261015/mapping.json)；候选范围见 [scope-audit.json](local-incremental-20261015/scope-audit.json)。最终 owner/wire 对照见 [final-module-contract-map.json](local-incremental-20261015/final-module-contract-map.json)，最后验证索引见 [final-verification.json](local-incremental-20261015/final-verification.json)，其中 source SHA 必须与实际最后 gate 一致。本轮独立 Git 对象审计核对全部 51 行、52 个目标和 55 个候选 blob，确认均进入 `677ad23…`；`f51226a…` 的后续修复另由 `source-freeze.json` 记录 30 个路径及其哈希，不将后续修复误称为候选原样未改。

## 3. 业务归属与保留能力

| 业务职责 | 当前实际 owner 与边界 |
| --- | --- |
| 资产查询、数值/OR/队列过滤、响应展示、人工操作 | 前端 `src/features/assets/{api.ts,types.ts,components/}`；Rust `commands/assets.rs`、私有响应分组实现 `commands/asset_response_groups.rs`；DTO 在 `assets/model.rs` |
| 页面隔离、预览、备份、停用和恢复 | Assets 业务；命令入口 `commands/page_isolation.rs`、`commands/asset_rule_catalog.rs`，域规则 `page_isolation.rs` 与 `page_isolation.schema.sql` |
| 资产采集、导入、检查点与原任务恢复 | 资产 jobs 实现及 `features/assets/workspace/AssetsPluginWorkspace.vue`；不转交聊天或通用 Agent |
| 设置中的 Rules 入口 | 配置导航提供入口，内容与规则 API 仍由 Assets 拥有；原 Plugins、模型 profile 与配置字段继续保留 |
| Shell 导航与只读总览 | `App.vue` 负责启动和入口协调；`commands/application_overview.rs` 负责跨项目/资产/任务的只读聚合，不机械移归 Assets |
| 扫描、聊天、技能与项目空间 | 沿用已拆分 owner；聊天仍感知当前项目，技能导入有独立入口，项目切换/权限与运行时边界不回退到旧全局桶 |
| common/shared 与基础设施 | 继续承载实际复用的基础组件、类型和工具；本次未加入页面隔离、扫描 DTO 或具体业务规则 |

本地 `models_asset.rs` 的增量落到 `assets/model.rs`，本地全局 `types.ts` 的业务字段落到 `features/assets/types.ts`，配置导航样式落到 configuration 的样式文件，发布说明组件保留在 Shell。前端内部消费 feature API，没有恢复旧聚合 API。配置保留本次整合基线的 `workers=96`、`probeTotalTimeout=8` 及完整 profile/Plugins 字段，没有从 main 取默认值覆盖 WIP。

## 4. IPC、DTO 与响应组契约

### 4.1 旧入口兼容与 5 个新增入口

不可变 Git 对象审计确认：旧 289 个 handler 的名称、相对顺序、实际 owner 和参数/返回签名保留；当前 registry 共 294 个唯一入口，恰好新增下列 5 个。TypeScript AST 审计还确认旧 224 个 API 方法的名称、顺序、owner、wire 调用及属性字节保留，当前增加至 229 个。静态兼容审计不替代运行测试。

| 新 IPC | wire 输入 | 主要返回内容 |
| --- | --- | --- |
| `preview_page_rule` | `{ input: { projectId, sourceAssetId, kind, value } }` | rule、匹配/可操作/受保护/已隔离数量、sample、previewToken |
| `apply_page_rule` | `{ input: { rule, previewToken } }` | ruleId、isolated、protected、backupPath |
| `list_page_rules` | `{ projectId }` | 项目规则列表 |
| `disable_page_rule` | `{ projectId, ruleId }` | restored、retained、backupPath |
| `get_asset_rule_catalog` | 无参数 | projectRules、legacyRules |

规则类型为 `title_contains` / `body_sha256`，前端 camelCase 与 Rust serde 契约对应。`AssetPage.memberTotal` 在 Rust 为 `i64`，前端为兼容旧快照保留可选字段并回退 `total`。原 `ASSET_SELECT` 的 41 列保持，展示分组字段追加在索引 41；展示折叠不会删除原 URL 或把组代表误作唯一导出对象。

### 4.2 展开成员使用可选代表资产锚点

审查发现旧 fingerprint-only 展开可能包含同指纹、不同人工状态或响应证明的另一展示分区，导致“2 URL”按钮展开 3 条以上。修复新增可选 `responseRepresentativeAssetId`：TS 为可选 number，Rust 为 `Option<i64>` 且 `serde(default)` 为 None，旧 fingerprint-only 请求继续兼容。

存在锚点时，必须是正 ID，同时提供项目及有效 fingerprint。后端在**完整当前筛选结果**中找到代表行，再以分组使用的全部 20 项 tuple/状态字段作 null-safe 匹配；锚点已过期或被当前筛选排除时返回空，不扩大到其他分区。列表与 CSV 导出共享验证和筛选，但入口时序须分别说明：公开 `list_assets` 先 `db::open`，随后 `list_assets_on` 的首句执行 `validate_asset_query`，再开启查询事务或读取资产快照；`export_assets` 则在打开数据库、创建目录/输出文件之前执行同一验证。导出仍输出精确组内的原始 URL。

前端展开发送项目、fingerprint 和代表 ID；清除分组、重置及项目切换同步移除两项分组条件，会话恢复保留完整有效锚点并清理孤立 ID。打开代表 URL 后，seen 状态会改变分区，因此重新读取当前锚点下的行与计数；迟到的 opener/标记响应不能回写新的查询或卸载视图。

### 4.3 真实 handler 测试与边界

`src-tauri/src/local_incremental_ipc_tests.rs` 使用官方 Tauri `MockRuntime`、生产 `generate_handler!` 生成的实际命令 wrapper、真实 AppState 和临时 SQLite，覆盖参数解码、5 个入口、只读预览、令牌失效、恢复、分组/原始导出及取消任务的终态保护。它不是直接调用前端 API mock 或 SQL 切片 harness。这些测试已进入实际原生执行：`native-targeted-01` 为 137 pass / 3 个历史 fixture fail；补齐外键闭包后的 `native-targeted-02` 在 `67b57d0…` 上 140/140 通过，包含本文件全部 7 个真实 handler 用例和本轮新增关键原生 36 项。012 全量实际记录为 3,717 pass / 1 个旧注册守卫 fail / 7 ignored；新源码 241 的最后结论由第 6.4 节记录。MockRuntime 不等同真实桌面 Wry/event loop、托盘或平台 IPC 验收。

## 5. 本轮审查及修复

| 文件组 | 已落实的行为修复与回归目的 |
| --- | --- |
| `page_isolation.rs`、`db_initialize_budgets.rs` 与迁移测试 | 显式人工重新确认即使文本与隔离注释相同也受保护；历史分类迁移不覆盖人工备注/删除/事件、归属授权或调查状态；非空导入与人工恢复要求沿用外层真实事务 |
| `page_isolation_migration_tests.rs`、`legacy_525_schema.sql` | 首次原生运行在初始 seed 缺 FK parent runs、尚未进入初始化时失败。`67b57d0…` 从同一旧 schema 补齐 config_profiles/runs，成为 10 表闭包；原 8 表完整前缀与全部 15 条外键保留，FK 始终开启。随后 3 个历史迁移测试真实通过完整 `db::initialize` 与正常 seeding，不删原断言 |
| `jobs.rs`、`jobs/job_lifecycle.rs` 及测试 | 终态写入须满足 affected-row=1、同事务读回及 commit 后才发布完成；忽略更新/触发器改写转入失败路径；持久化失败不伪报成功；取消使用 active 状态 CAS，不能将已完成任务改回 cancel_requested |
| `jobs/reprobe_source_tests.rs`、`native_fofa.rs` 与输入发布 | 临时文件 flush/sync 后再原子发布；重探测先 P2/P3 后 P1 提交标记，失败不留下可被误复用的半份主输入；保留有效冻结文件字节与 mtime，CSV 以真实记录计数支持多行字段；Windows sync 使用可写句柄 |
| `jobs/native_probe.rs` 与私网字面量测试 | 带方括号 IPv6 与 IPv4-mapped IPv6 在实际发网前经过既有私网分类；保留显式 allow_private；不扩大成通用 DNS/重定向授权重写 |
| `PageIsolationDialog.vue` | 项目/示例/草稿变化使预览失效；generation 阻止 ABA 规则读取和旧写入完成覆盖新操作，卸载后不隐藏刷新或 emit |
| `AssetRuleSettings.vue` | 停用成功后的目录读取取代尚未完成的旧读取；迟到成功/错误不能把已停用规则显示回 Enabled |
| `AssetWorkspace.vue` 与 Assets DTO | 项目上下文同步持久化，避免同 tick 把 A 查询存入 B；切项目关闭旧隔离 UI；成员锚点、分页、导出与 seen 后重读保持一致 |
| `App.vue`、`AssetsPluginWorkspace.vue` 与 shell/plugin 测试 | 从 Projects/Settings/Skills 点击当前模块可返回业务页；full/collect/reprobe 恢复原 runId；host receipt 跨卸载且阻止重复，迟到动作不跳转当前新项目 |

资产导入继续在 IMMEDIATE 事务取得写入准入后读取快照/规则/历史，再原子写入资产、项目关联与事件，保护人工状态。发现入口以实际 scheme/host/port/path/query 去重并合并来源证据，保留不同原始 URL；公司/域名显示偏好不推导授权归属。采集恢复校验种子、计划、endpoint/credential/schema/options 绑定及页哈希，失败或取消保留输入、页状态与日志。探测预算继续覆盖实际重试、响应体和协议/TCP fallback，排队节流时间按既有设计单独计算。

取消与结束以数据库提交顺序决定唯一终态：取消提交先到，原本成功的任务落 cancelled；结束先取得写入准入并完成，之后取消返回错误；真实执行错误仍为 failed。原子标记的单独写入时刻不构成持久化顺序保证。RAII 清理 registry/托盘活动不依赖 SQL 成功。

资产续跑的三个受支持 pipeline 为 `full|collect|reprobe`，使用 `resume_job(runId)`，不 ACK 旧提醒、不创建替代任务。**非空 unsupported pipeline 在 ACK/start/resume 之前拒绝**；仅空 pipeline 保留原 UI 的 ACK→startJob(full) 兼容分支，该分支目前由合成历史 fixture 验证，没有据此声称实际旧数据库迁移已验证。这些修复不构成付费 Agent Resume/Retry 成功合同。

### 5.1 原 WIP 六项疑点的现状与边界

下表据 `contract-review/original-six-wip-concerns-source-and-compiled-tests.json` 核对实际 owner、生产路径和编译后的 exact 名称。该 JSON 本身只作源码/列名映射，不能当执行证明；另与 `native-full-accepted-02-result.json` 逐名对照，其所列关键名称均有实际 ok 记录，但 full02 整体仍因另一注册守卫失败。后续独立 `original-six-wip-concerns-final-execution.json` 与新源 full03 精确核对 35 个去重关键用例全部 ok；原012映射保留，12 个源码 blob 已证明完全继承至241。这些用例属于 full 子集，不再累加，也不把已测代表情形扩大成全部边界覆盖。

| 原疑点 | 实际 owner 与当前合同 | 已有断言 / 明确保留的边界 |
| --- | --- | --- |
| `agent_enabled=false` 可能拒绝普通聊天 | **Chat/conversations** 的 `access.rs`/`reply.rs`：scope 校验项目、会话快照与 revision；开关关闭工具定义/执行，不禁止普通聊天携带当前项目文字上下文 | 已有 disabled plain-chat、stale/wrong-project 拒绝及 callback 收到无 tools 的断言；callback 是替代 completion，非真实 provider/Wry |
| provider 同意未绑定 endpoint | **Chat access/consent.rs** 持有 recipient 同意与 in-flight lease；模型配置基础设施只解析 gateway。身份 hash 绑定完整解析 endpoint（含 path/query）、model、local deployment flag；显示仅暴露 origin，旧 boolean 不自动升级为新同意 | 已有 endpoint-path 变更、发请求前/付费响应后失效、可撤销与旧 schema 断言；没有据此宣称 query-only/model-only/local-flag-only 逐一边界已测，API-key rotation 不在 recipient identity 内；不等同付费续跑成功合同 |
| 技能撤销 / 内容指纹不完整 | **Chat `conversations/access/skills.rs` 与 `conversations/access/schema.sql`** 拥有选择、generation/tombstone 与同意失效；**Skills `skill_packages/store.rs`** 提供 package provenance、entry 和实际可访问资源 bytes/hash 的指纹。generation/tombstone 不归 skill_packages | 已有同 timestamp 停用再启用/改回文本、删同 ID 再建、资源损坏/来源变化及 in-flight 撤销断言；仅参考文本读取同意，不授权执行脚本，也不证明未取得的本地排除测试等同 |
| 24 选择限制与 36 内置技能冲突 | **Chat access** 选择上限现为 128 个唯一正 ID，model skills_list 每页 12；**Skills package** 保留 36 技能/443 文件的来源与 license 元数据 | 已有选择 36 并三页遍历、拒绝 list limit 13、36/443 完整元数据与冲突注册断言；没有专门 128/129 边界断言，8 次工具调用/4 轮限制仍在，不能说一次回复读取全部技能 |
| 16 KiB 整包读取与 12 KiB 工具输出不一致 | **Chat access/tools** 通过 UTF-8 byte offset 分页，model-facing instruction/resource 内容每页最多 **4,096 bytes**；最终序列化 JSON envelope 上限 **32,768 bytes**。**Skills 直接 resource IPC** 默认 4,096、最大 **8,192 bytes**，不是模型工具的页上限 | 已有 20,000 bytes 多字节内容重组、偏移/未选中 ID/超限拒绝、资源 0/8193 限制、JSON escaping 包络、整份资源先扫描再分页断言；未专测 exact 32768/32769 包络边界，包络过大可拒绝，不保证实时模型会读完所有页 |
| 两个测试模块缺失 / Rust 未实际编译 | **Skills `skill_packages/tests.rs`** 与 **Chat `conversations/access/tests.rs`** 已跟踪并 cfg(test) 接线；012 的实际库列表含 16+14 项，真实 locked all-targets check 与 library no-run 均 exit 0 | 这 30 项在 full02 各有实际 ok；列名/构建原本只证明编译，实际运行证据须分开。012 full 的另一守卫失败不抹去已运行事实，也不使整套被接受；Linux/MockRuntime 不证明 macOS/Windows 桌面、托盘、电源或 live provider |

## 6. 验证账本

### 6.1 已执行的阶段检查

表中每行均绑定其实际 code HEAD，不能把不同提交下的通过数字合成同一次最终验收。稳定运行记录检查 native/audit/tracked/untracked 的完整路径集合和 SHA，工具与审计 policy 也绑定哈希。核心输入计数保持为 1,828 native、2,249 audit、2,609 tracked、0 untracked，聚合清单 2,614 项；最后 gate 仍须引用其自身前后清单，不能只凭计数或 HEAD 相同认定稳定。

| 阶段 / 实际源码 | 实际执行结果 | 结果文件与限制 |
| --- | --- | --- |
| `f51226a…` 前端构建 | `npm run build` exit 0，18.868 秒 | `frozen-frontend-build-01-result.json`；clean，全部输入前后不变 |
| `f51226a…` all-targets check | exit 0，20.779 秒 | `frozen-cargo-check-01-result.json`；真实应用类型检查，非测试执行 |
| `f51226a…` 首次 Node 全量 | 1,588 total / 1,575 pass / 1 fail / 12 skip；exit 1，148.053 秒 | `node-final-01/result.json`；源码稳定，剩余失败为守卫未适配共享 query validator |
| `0f5856f…` Node 全量 | 97 文件，1,588 total / 1,576 pass / 0 fail / 12 skip；exit 0，139.891 秒 | `node-final-02/result.json`；守卫修订后通过，非最后 012 阶段结论 |
| `0f5856f…` Chromium | 28/28，outer exit 0，无过滤、输入前后不变 | `browser/final-02/`；合成 Tauri IPC，非真实用户 DB/桌面 |
| `0f5856f…` 原生定向首次 | 140 项，137 pass / 3 fail / 0 ignored，3,585 filtered；exit 101，6.837 秒 | `native-targeted-01-result.json`、`.log`；3 个新增历史 fixture 在 seed 缺 main.runs，尚未进入初始化 |
| `67b57d0…` all-targets check | exit 0，20.543 秒 | `frozen-cargo-check-03-result.json`；补齐 fixture 闭包后，完整输入前后不变 |
| `67b57d0…` 原生 lib no-run | exit 0，134.538 秒 | `frozen-native-build-03-result.json`；真实构建/链接，非执行 |
| `67b57d0…` 原生定向重跑 | 140/140、0 fail，exit 0，6.532 秒；包含本轮关键新增 36、历史迁移 3、真实 handler 7 | `native-targeted-02-result.json`、`native-targeted-02-verification.json`；全部 140 恰好一次、原生二进制/输入稳定；属于定向接受 |
| `67b57d0…` Node 全量 | 1,588 total / 1,576 pass / 0 fail / 12 skip；exit 0，171.628 秒 | `node-final-03/result.json`；独立于最后 012 阶段 |
| `67b57d0…` Chromium | 28/28，无过滤，outer exit 0、输入前后不变 | `browser/final-03/`；runtime error / unknown IPC / external fetch / blocked request 均 0 |
| `012425b…` 前端构建 | `npm run build` exit 0，31.080 秒 | `frozen-frontend-build-04-result.json`；该次 clean，全部输入前后不变 |
| `012425b…` all-targets check | exit 0，19.839 秒 | `frozen-cargo-check-04-result.json`；完整输入稳定 |
| `012425b…` 优化 library no-run | exit 0，277.342 秒 | `frozen-native-build-04-result.json`；实际产物/完整输入稳定，非测试执行 |
| `012425b…` 原生定向 | 140/140、0 fail / 0 ignored，exit 0，6.238 秒；critical36 全实际 pass | `native-targeted-03-result.json`、`native-targeted-03-verification.json`；定向接受，不代替 full |
| `012425b…` 未过滤 full02 | 完整 3,725 项：3,717 pass / 1 fail / 7 原 ignored / 0 filtered；outer/command exit 101；Rust 1,542.76 秒，command 1,542.914 秒 | `native-full-accepted-02-result.json`；所有列名恰执行一次，source/全部输入/binary/helpers 稳定；唯一失败是旧注册守卫 289 vs 294，accepted=false |
| `2417970…` 两文件测试修复 | 提交已完成，生产未动 | build05/Node05/browser05/check05、target04、guard-exact01 与 full03 均已实际完成；结果与输入绑定见第 6.4 节 |

浏览器 28 场景保留此前 18 场景，并覆盖 scoped OR/数值过滤、52 成员的 50+2 分页、单成员操作与全量导出、预览编辑后失效/停用保护、项目切换迟到读取、Rules 旧目录读取覆盖、三种原地恢复/重复点击/错误后核对、窄屏布局及响应碰撞。碰撞场景实际验证初始仅 401/402；点击 401 标记 seen 后同锚点重读变为 401/403，旧 402 不残留。截图与布局记录不另算测试用例。

### 6.2 用例迁移账本、预检拒绝与合同替换

旧云端基准有 3,602 个原生用例名；当前实际列名为 3,725 个唯一用例，**净增 123**。其组成是 **3,601 个原名保留 + 1 个必要合同替换 + 123 个全新用例**。不能写成“3,602 原名全部保留”或“124 项全新”；列表中的 124 个新名字包括该 1 个替换名。

`native-full-accepted-01` 只运行 `--list`（列名命令 exit 0），收集守卫发现旧 optimizer 名缺失后拒绝启动 full。`native-full-accepted-01-collection-error.json` 明确 `fullExecutionStarted=false`、实际 full 退出码为 null：**该 attempt 执行了 0 个测试，没有原生 full 测试失败结果或 completed summary**。拒绝证据保留，随后独立审查替换合同，不为启动 full 绕过未知缺项。

| 原用例 | 当前替换用例 | 必要语义变化与保留断言 |
| --- | --- | --- |
| `jobs::native_workers::tests::optimizer_matches_python_contract` | `jobs::native_workers::tests::optimizer_preserves_distinct_schemes_and_quarantines_failed_endpoints` | 旧 Python 身份忽略 scheme/port/query，示例会把健康 HTTP 作为重复而仅保留 HTTPS；本地授权行为保留不同请求原 URL。原三行输入、真实 optimizer 调用、HTTPS 仍存在与坏端点隔离的合同保留；kept 从 1→2、excluded 从 2→1 是行为合同更改，不能称字节等价改名 |

独立证据 `jobs-review/optimizer-contract-replacement.json` 保存旧/新函数、Git blob 与定义 SHA、生产调用及断言逐项比较，确认没有移除无关旧 optimizer 断言。新原 URL 保留、完整响应分组、原始全量导出等已在 `67b57d0…` 的 140 项定向中实际通过；这不意味着旧 Python 折叠合同继续成立，也不能替代未过滤 full。

`012425b…` 是另一个明确的 **cfg(test) 修订**：取消可能发生在第一个或第二个请求完成后，测试先独立解析未注入损坏前的 journal，核对 P1/web_alive、合法 URL、无重复及真实记录数，再要求 `reused_results` 精确等于该持久化数；服务器请求总数和最终 CSV 两行断言仍保留。它替代宽松的 1-or-2 复用断言，生产与 optimizer 函数完全未改。不能把本次测试精确化混作 optimizer 的合同替换。

`native-full-accepted-02` 与上述 list-only attempt 不同：它在 012 源码上**真实完整运行**，统计为 3,717 pass / 1 fail / 7 ignored / 0 filtered，所有 3,725 名各执行一次、summary 与逐例一致，source、全部输入、二进制、helper 均稳定；Rust 1,542.76 秒，command 1,542.914 秒，outer/command exit 101。唯一失败是 `scanning::tests::implicit_history_import_commands_are_not_registered` 仍硬编码 registry 长度 289，而五项正式新增入口使长度为 294；独立实际 registry/owner 与所有旧退休/隐式根检查未发现退休入口重新注册。该次不是“只列名拒绝”，也不是“全绿”；失败原样留存。

修复提交 `2417970…` 的标题为 `test: validate added asset IPC owners in retirement guard`，只改原守卫及 retirement allowlist。新守卫要求总数 294，五个新增入口分别在精确 Assets owner 下恰好一次，剔除五项后保留计数 289；原 uniqueness/live/退休入口/已删除 include/隐式根负例断言 tail 逐字节保留。allowlist 只更新该 fixture 的 reason 和全文件 SHA；固定 source scopes、category/count 与空 active baseline 不变（50 条：47 fixture / 2 migration / 1 importer），没有新豁免。独立静态解释的 1 个正例、33 个负例验证丢新入口、错 owner、补数、重复、退休入口及隐式根等拒绝；**这些静态控制本身不是原生测试执行**；其后新源的 exact 守卫与完整 full 已实际执行通过，分别列在第 6.4 节，不把静态负控计入 full 总数。

原守卫没有钉住任意未来编辑下全部 289 条 qualified path 清单，新守卫也不冒称新增此能力；旧 289 全路径与顺序未变另由独立 Git/registry 审计证明。此测试修复不改变生产注册或任何执行授权边界，不能解释成重新启用旧隐式导入。

最后原生 gate 必须核对：3,725 唯一名称、3,601 原名及显式替换映射、123 新用例集合、预期 ignored 集合、逐用例 occurrence、真实退出码、完整 summary、对应二进制 SHA 和前后输入清单。列名通过、局部选择通过或预检拒绝都不是未过滤 full 接受。

### 6.3 真实失败、修复和历史验证的使用范围

`677ad23…` 的最初 Node baseline 实际为 1,564 tests、1,549 pass、3 fail、12 skip，exit 1，117.346 秒；运行期间 native 输入变化，不能作为冻结验收。三类失败均保留：

1. Linux Chromium SUID sandbox 不可用，测试既有 opt-in 未设置。只对测试子进程设置 `CHROME_BINARY=/usr/bin/chromium`、`OVIRAPTOR_TEST_NO_SANDBOX=1` 后实际 1/1 通过，没有修改生产浏览器参数或 HOME。
2. App 同模块 early return 导致共享 Projects 不能返回 Chat，是真实生产回归；已修生产逻辑并保留原断言。
3. 旧 host resume fixture 期待 full ACK/startJob，新合同已支持原 runId。旧替代分支改为空 pipeline 正例，新增支持三类恢复的成功/失败、跨卸载、重复阻断、状态核对及 unsupported 的 0 ACK/0 start/0 resume；没有允许未知 pipeline 或删除旧兼容正例。

shell/plugin/navigation 定向实际 75/75，前端新增 SFC 行为回归先出现 5 项失败、修复后最终 20/20，失败日志未覆盖。该测试使用真实 SFC setup、编译 template 与 Vue host renderer 事件，替代 DOM 指令/图标与 IPC，不当作真实 Chromium/Tauri 验收。

`f51226a…` 首次冻结 Node 剩余的 1 fail 来自旧 import 字面断言。`0f5856f…` 按实际 4 个具名符号核对窄导入，继续禁止 alias/glob/重复实现，并新增 export 在 DB/output 前验证、inventory 在事务/快照前验证的断言；没有改变生产代码或放宽行为断言。随后完整 Node 在 0f 和 67 两阶段均通过。

首次原生 no-run `frozen-native-build-01` 实际 exit 0、283.493 秒，但运行中守卫文件变化，完整 audit/tracked 输入不稳定，不能接受为最终全输入冻结证据。随后 `frozen-native-build-02` 在 clean 0f 阶段增量确认 exit 0、0.475 秒，输入不变；不能把该增量时长称为全量重编译。67 阶段实际重新构建记录另见上表。

3 个新增迁移测试首次都在 seed 缺 main.runs、尚未进入 `db::initialize` 时失败，不是“初始化后生产迁移已失败”，更不是迁移已通过。67 从原 `525f128…` 同一 schema 补齐 config_profiles/runs：10 个 CREATE 逐字节来自旧源、原 8 表完整前缀不变、15 条 FK 保留、始终 foreign_keys=ON，未删除 Rust 断言或修改生产。随后 3 项真实初始化/正常 seeding 回归通过。fixture SHA 为 `90fe63e7c30cc4f0b6fead31cecc7e1d3c43cd0940f887c5674025416fa596d3`，完整依赖闭包与源 blob 见 `db-review/historical-schema-provenance.json`；旧缺项 fixture 的 SHA/失败原样留存。

候选交接阶段 Node 214 项通过；Rust 轻量 harness 20 与 90 重叠 9，去重 101，不是 110。当时完整 Cargo 因 glib 开发依赖失败，33 文件/15 构造点语法检查也不等于类型检查。后来真实 all-targets check、no-run 与 140 项原生执行各自补齐对应覆盖；任何 mutable 编译、切片 harness、先前云端 full 或 012 的完整失败结果都不能替代新源 241 的最终 gate。

### 6.4 最终实际 gates

当前状态为 **ACCEPTED_CLOUD_REGRESSION**，以下均为 clean `24179702c9d163e1d21bf9a23c4528589355166a` 的实际已完成 gate；原生二进制 SHA256 为 `3e45d49035145d33d416f5ce3a914af8535e722ed56a19d9796ae1a17355e572`。后续仅文档提交及发布身份由独立外部收据核对；若实现源码再改，须重新冻结并运行必要检查。

| Gate | 最后实际结果 | 必须同时记录 |
| --- | --- | --- |
| 原生 full | **ACCEPTED**；`native-full-accepted-03-result.json` / `native-full-accepted-03.log`；3,725 唯一用例各恰一次，3,718 pass / 0 fail / 7 原 ignored / 0 filtered；command 与独立 `native-full-accepted-03-outer-receipt.json` outer 均实际 exit 0；Rust 1595.82s / command 1596.040s。3601 原名 + 1 已审合同替换 + 123 全新；20 项实际检查全 true，源/全部输入/binary/helpers 稳定 | 实际 attempt/result/日志、tests/pass/fail/ignored/filtered、exit/时长、3725 名称及合同映射、逐例完整性、二进制 SHA、前后全部输入哈希、accepted；未接受须明确阻塞 |
| 完整 Node | **ACCEPTED**；`node-final-05/result.json` / `all-node.tap` / `case-occurrences.json`；97 文件，1,588 total / 1,576 pass / 0 fail / 12 原 skip；node/collector exit 0，171.001s；与04 ledger精确核对，原skip/完整输入闭包不变 | Node05 以04 ledger核对，97 文件、实际 total/pass/fail/skip、exit/时长、完整 occurrence/保留 skip 集合、collector SHA、最后源码与输入闭包 |
| Chromium | **ACCEPTED**；`browser/final-05/browser-results.json`、`execution-metadata.json`、`qa-review.json`；实际 Chromium 28/28、无过滤，runtime/unknownIPC/externalFetch/blocked均0；build 9.990s / browser 81.861s / outer exit0；283源与298全输入前后一致（manifest SHA `1802a49cc68b7efc1ed6a46e34424ed3c64b3cc6c41776b4b04a3a376df412c2`）。QA记录含44张本地截图，图片不收入文本证据包；合成IPC范围保持 | 28 场景完整集合、无过滤、runtime/IPC/外联错误计数、outer exit、源 manifest SHA、视觉复核及合成 IPC 限制 |
| 构建与 check | **ACCEPTED**；`frozen-frontend-build-05` exit0 / 20.290s（`frozen-frontend-build-05.log`）；`frozen-cargo-check-05` exit0 / 18.696s（`frozen-cargo-check-05.log`）；`frozen-native-build-05` exit0 / 285.965s（`frozen-native-build-05.log`）。真实build产物绑定同一full二进制，各命令源/输入/日志SHA一致；target04 140/140、0 fail/0 ignored，5.850s；exact guard 1/1，0.199s，均为full子集不重复累加 | 最后 npm build、all-targets check、优化 no-run、target04/guard-exact01 的实际命令/exit/时长、日志 SHA 与输入前后一致性；注明增量与实际产物 |
| 发布与总体结论 | **ACCEPTED_CLOUD_REGRESSION**；受测code `24179702c9d163e1d21bf9a23c4528589355166a`，独立分支 `integration/business-modules-20261014`，没有合并main。回归接受与发布身份分开：实际docs-only SHA、push/readback/远端tip、文本证据归档SHA及Library位置由后续独立外部 `publication-receipt.json`/交付收据绑定；本文不自引用或预填未来docs/归档SHA，也不声称生成报告已完成推送。模块/API契约见 [final-module-contract-map.json](local-incremental-20261015/final-module-contract-map.json) 与 [final-verification.json](local-incremental-20261015/final-verification.json) | 整体接受/阻塞、受测 code 与 docs-only tip、仅文档差异证明、推送分支、用户可获取证据位置/归档 SHA；无 main 合并 |

## 7. 排除内容与完整远端源码的保留

原归档不含完整本地测试树，以下内容没有向用户索取，也未从 Mac 补读。不可变 blob 审计确认候选及 `677ad23…` 保留了完整云端版本；这不等于取得或比较了对应本地排除版本。

| 类别 | 路径/内容与处理 |
| --- | --- |
| 3 个主动排除测试 | `src-tauri/src/agent_runtime/tests.rs`、`src-tauri/src/commands/exposure/tests_g1.rs`、`src-tauri/src/commands/runner_log_write_tests.rs`；保留远端原文件，不重建疑似敏感本地内容 |
| 5 个排除旧夹具 | `src-tauri/tests/fixtures/legacy_strix/bundles/producer_1_5_3/` 下 `.state/events.jsonl`、`events.jsonl`、`events.ndjson`、`vulnerabilities.csv`，以及 `producer_1_6_2/events.jsonl`；保留远端原 blob |
| 2 个被过滤器误漏的真实生产文件 | `src-tauri/src/agent_runtime/multi_agent/budget/root/target/lifetime.rs`、`src-tauri/src/agent_runtime/multi_agent/budget/target/receipt.rs`；完整远端版本保持，没有用 stub 替代或套用过滤删除 |

源码发布范围及定向启发式秘密检查沿用候选 [publication-audit.json](local-incremental-20261015/publication-audit.json) 的明确范围与限制；它不是所有潜在秘密均不存在的证明，也未把整个文件列为豁免。交付不附用户移交包、签名 URL、凭据、真实用户数据库或依赖/编译缓存。新历史迁移数据使用可审查的 schema 来源和合成行。

## 8. 剩余边界与风险

- **平台与真实桌面**：本轮为 Linux 云端。没有验证用户 Mac、Windows、ARM64 安装包、真实 Wry/桌面 IPC、平台托盘、电源与通知 API。Windows 可写句柄修复有源码依据，但没有 Windows 执行记录；Linux 平台特定负例不证明 macOS 正例成功。
- **真实外部服务**：Chromium 使用严格合成 IPC，不连接真实 provider、账单、远程扫描目标或用户数据库。真实第三方同意/计费及部署环境仍需相应独立验收，不能由 mock 成功推定。
- **付费 Agent Resume/Retry**：本轮资产 `full|collect|reprobe` 原地恢复与付费 Agent 的财务续跑合同是两条业务路径。此前付费成功续跑合同的缺口仍保留；没有因资产修复宣称通用 Agent 或付费 Resume/Retry 已实现。
- **旧有效截断 CSV**：历史 P1 若恰好是语法有效的截断前缀，且没有旧哈希/行数权威，当前只能校验结构，无法证明它原本完整。不能用当前项目数据重建并冒称原冻结输入，现有有效字节继续保留。
- **永久数据库写失败**：两次终态写均失败时明确报告持久化错误，不发布完成；原 run 与输入保留，但 durable 状态可能暂时仍为 active，需正常启动 reconciliation 才转 interrupted。不能承诺永久存储故障下立即可恢复或状态已落盘。
- **历史迁移范围**：67 阶段已在 10 表精确旧 DDL 与合成行上完成 3 项真实初始化回归；这不意味着已迁移或检查用户实际数据库。空 pipeline UI fallback 也没有真实历史样本的迁移证明。
- **网络规则范围**：IPv6 字面量与 IPv4-mapped 修复只补齐现有私网谓词入口，没有建立 DNS rebinding 或重定向全过程授权边界。
- **页面隔离恢复**：预览令牌用于清单新鲜度校验，不是独立授权凭据；归档项目仍只读。后续数据库错误可留下安全备份文件，备份存在不能单独证明业务事务已提交。
- **移交包缺项**：3 个排除测试、5 个旧夹具未取得本地版本，2 个误漏生产源仅保留远端版本；当前整合不声明覆盖未知本地差异。

## 9. 证据索引与交付说明

本轮仓外证据根为 `/workspace/validation/local-integration-20261015/`；正式仓内映射/索引为 [final-module-contract-map.json](local-incremental-20261015/final-module-contract-map.json) 与 [final-verification.json](local-incremental-20261015/final-verification.json)。用户可取回的证据载体、大小与SHA由独立外部发布收据及最终交付消息记录；本文件不预填尚未产生的交付身份。源码来源与模块归属详见本稿第 2—5 节，最终实际结果见第 6.4 节及正式验证索引。

| 证据 | 内容 |
| --- | --- |
| `baseline.json`、`source-freeze.json`、各 attempt 的 before/after manifest | 共同祖先、候选/merge、30 文件修复与实际受测输入；最初 freeze/012 不冒充最后 241 闭包 |
| `contract-review/immutable-provenance-ipc-review.json`、`api-contract-at-merge.json` | 51 映射、52 目标/55 blob、10 个排除/漏源远端保留文件、旧 289/新 294 handler 与 TS API 兼容 |
| `contract-review/response-group-anchor-source-ready.json`、`independent-review.json` | 可选锚点、精确分区、列表/导出合同与独立复核 |
| `db-review/{review.json,historical-schema-provenance.json}`、`fix-prepared/` 的保留诊断 | 人工状态/外层事务、原始失败、10 表来源与 15 FK 闭包；review 中旧 pending 状态以真实 targeted-02 结果补充，不篡改旧记录 |
| `jobs-review/{source-review-manifest.json,independent-review.json,cancel-cas-sql-proof.json,optimizer-contract-replacement.json}` | 状态 CAS/输入发布/IPv6/平台边界、唯一 optimizer 合同替换与断言比较 |
| `frontend-review/frontend-review-result.json` 与日志 | 失败→修复后 20 项实际 SFC 回归、SHA 与测试桩边界 |
| `node-baseline/`、`node-failure-analysis.json`、`node-shell-resume-targeted/`、`node-native-runtime-ci/` | 3 个最初失败及定向修复记录 |
| `node-final-01/`、`node-final-02/`、`node-final-03/` 与最后 actual attempt | 首次守卫 1 fail、0f/67 全绿、最后 gate 的完整记录，不覆盖历史 |
| `frozen-frontend-build-*`、`frozen-cargo-check-*`、`frozen-native-build-*` | 每阶段实际命令/产物/日志和完整输入前后哈希，显式标出首构建的输入漂移 |
| `native-targeted-01*`、`native-targeted-02*`、`native-targeted-03*` 与最后 actual attempt | 140 项首次 3 fixture fail、修复后 67/012 分别 140/140；关键 36/迁移 3/handler 7 的真实 occurrence；241 另行核对 |
| `native-full-accepted-01-{collection-error,migration,list-result}.json`、`native-full-accepted-02-result.json` 与后续 actual full | full01只列名拒绝0测试；full02完整运行1个旧计数守卫fail；241 full03最后结果须独立记录，不能混写 |
| `contract-review/original-six-wip-concerns-source-and-compiled-tests.json`、`retired-import-guard-review/independent-review.json` | 六疑点owner/编译断言/边界映射；两文件守卫/allowlist审查与1pos+33neg静态控制，原JSON旧状态保留 |
| `browser/final/`、`browser/final-02/`、`browser/final-03/` 与最后 actual attempt | 各源码对应的真实 Chromium 28 场景、输入稳定性与合成 IPC 边界 |
| `run-command.py`、`snapshot-inputs.py`、`collect-all-node.py`、`run-native-full.py` 及审查/升级记录 | 执行/输入闭包/用例收集规则与实际 SHA；未知缺项继续 fail-closed，唯一已审合同替换显式入账 |

最终gate仅来自实际已完成结果；后续docs-only差异、独立分支推送、远端回读和证据归档由外部发布收据绑定。历史失败、预检拒绝和可变输入诊断原样保留，不与当前接受结果累加。
