# 本地增量候选交接（2026-10-15）

此候选供后续云端业务模块整合审阅。仅发布独立分支 `candidate/local-incremental-20261015`；没有合并 main 或 `integration/business-modules-20261014`，没有操作用户 Mac。

## 提交与源码来源

- **受测代码提交**：[`b2492bb94ce7aca1b102ffefc7b047dee819a2fe`](https://github.com/swyiic/Oviraptor/commit/b2492bb94ce7aca1b102ffefc7b047dee819a2fe)。
- 直接父提交 / 只读业务拆分快照：`525f1283adae79a13bcf0e07cf50772841911fb0`。受测提交含 55 个源码、配置及必要测试文件的变更，无删除项。
- 本文件及相邻 JSON/审计证据由另一个 **docs-only 子提交**提供；文档提交的 SHA 见本文件 Git 历史或推送交付结果。测试结果归属于上面的受测代码提交，不声称在文档提交上重新测试。
- 本地原始基准：`8fddee2321b313a8a6372d0895dd4c1f533ba57c`。27 个 tracked 增量的基准 blob 均与该真实 Git 基准匹配。
- `Oviraptor-local-source-8fddee2.tar.gz`：10,844,046 字节；SHA256 `fc6448afee2399a988679ff64db8d95a2c5b7439e78ecd0bed05d94535fa24af`；通过 Library 刷新传输后下载，外层及全部 10 个组件校验通过。
- 过滤 bundle 新根 `0d0f995a2b7312c28589c1d952ae33ffcbe045db` 只用于恢复；它不是共同 Git 祖先，其历史没有被合并。
- 已恢复并逐项校验 2,090 个文件；保留 27 个 tracked 修改和 24 个新增源码/测试。候选从完整远端快照构建，只迁移这 51 项增量及必要适配，没有导入过滤树的删除。

## 功能与模块适配

1. 资产查询：高级 OR 条件留在项目/回收站/主队列边界内；修正数值比较、P1/P2/P3 家族过滤及相关 SQL 括号。
2. 响应展示：以完整 HTTP 响应证明进行折叠，保留每个原 URL、成员操作和全量导出；列表、分组数量与分页共享读快照。原 ASSET_SELECT 的 41 列保持，分组列追加在索引 41。
3. 项目页面隔离：预览令牌、备份、人工状态保护、规则目录、停用与恢复；新增 5 个 IPC，分属显式 `commands::page_isolation` / `commands::asset_rule_catalog` owner。
4. 资产采集：断点恢复、失败持久化、发现入口去重与来源合并、响应身份/总网络预算、CSV 写入锁与人工状态保护、托盘并发任务清理。
5. 保留远端 feature API/DTO、导航及插件 workspace 边界。`dashboard_stats` 增量迁至 `commands/application_overview.rs`；资产 Rust DTO 迁至 `assets/model.rs`；前端新增 DTO 归 `features/assets/types.ts`。没有恢复旧聚合命令 include 或全量类型/API facade。
6. App 原有资产刷新与 full/collect/reprobe 原地恢复语义接入 `AssetsPluginWorkspace.vue`；配置导航保留远端 Plugins 并加入 Rules。必要测试适配包括 owner 路径、浏览器/session mock、续跑语义及两个 loopback socket fixture 的完整 HTTP header 读取/阻塞模式。

初步比较发现 18 个 tracked 路径与远端重叠，10 个文件共 21 处文本冲突；均已按符号/模块归属处理。24 个新增路径无同名碰撞。51 项最终映射见 [mapping.json](local-incremental-20261015/mapping.json) 和下表。

## 已执行验证（均归于受测代码提交）

| 检查 | 结果 | 覆盖与边界 |
| --- | --- | --- |
| `npm run build` | 通过 | Vue TypeScript 检查与 Vite 生产构建 |
| 前端组件/workspace 与 IPC mock 回归 | 73/73 | 页面隔离入口、响应组、拖选/会话与陈旧查询、导航、插件恢复与统计；非真实 IPC/数据库端到端验收 |
| 资产/历史导入回归 | 36/36 | 资产归属、日志与历史导入行为 |
| 前端边界与资产暴露回归 | 84/84 | 相对导入、feature owner、shared/启动图、暴露页回归 |
| Rust 业务边界源码守卫 | 21/21 | 14 个显式命令模块、294 个 IPC、窄 owner 依赖 |
| Rust 后端轻量 harness | 20/20 | 页面隔离 9、响应展示 8、查询规则 2、规则目录 1 |
| Rust jobs 轻量 harness | 90/90 | 全部 81 项 jobs 测试与 9 项页面隔离；含 CSV/锁/人工状态 14、恢复并发 8 |
| Rust 语法/构造点静态检查 | 33 个文件 / 15 个构造点通过 | 语法解析不等同 Rust 类型检查 |

Node 合计 **214 项通过**。两个 Rust harness 重叠同一组 9 项页面隔离测试，去重后为 101 项；不能把 20+90 当作 110 个不同测试。逐组命令、结果、保留日志的 SHA256 及与受测提交匹配的源码输入哈希见 [validation.json](local-incremental-20261015/validation.json)。本轮发布工作不重新编译、不补依赖。

Rust harness 使用实际生产域/DTO、原测试及 SCHEMA/WAL；对部分纯查询/导入函数作精确源码切片，数据库通知为空实现，并用默认 profile fixture 初始化。依赖版本/checksum 均与候选 Cargo.lock 对齐（例如 rusqlite 0.40.1）。临时 harness 生成器与编译输出留在云工作目录，未放入此分支，因此不能把本分支文档视为可独立重建该提取 harness 的完整套件。

## 发布前范围与秘密核对

本次发布只包含受测代码提交及本次文档子提交。55 个源码/配置/测试路径严格等于 51 项本地增量迁移后的 52 个实际目标路径，加 3 个必要测试适配文件；没有数据库、真实本地数据、安装/缓存/构建产物或删除项。逐项核对结果见 [scope-audit.json](local-incremental-20261015/scope-audit.json)。

受测提交的全部 55 个 changed blobs 与本次交接文档经过定向启发式秘密扫描和逐条命中人工复核；最终分类、脱敏哈希与限制见 [publication-audit.json](local-incremental-20261015/publication-audit.json)。启发式扫描不构成对所有可能秘密的绝对证明，未放宽任何整个文件的豁免。

## 明确缺失项与后续验收

- 完整 `cargo test --lib --no-run --locked -j 2` 在依赖 `glib-sys` 处因缺 `glib-2.0 >= 2.70` 开发库 / `glib-2.0.pc` 失败，尚未对应用 crate 完成类型检查。轻量测试通过不等同桌面应用完整编译或链接通过。
- 尚未覆盖历史数据库迁移、常规 seeding、真实通知投递、原生 Tauri IPC/runtime/worker 启动、GUI 托盘及平台电源 API。具备系统依赖后，应先 `cargo check --locked --all-targets`，再运行相关原生测试并做实际桌面验收。
- 原归档因敏感疑虑排除了 3 个测试：`src-tauri/src/agent_runtime/tests.rs`、`src-tauri/src/commands/exposure/tests_g1.rs`、`src-tauri/src/commands/runner_log_write_tests.rs`。没有读取、重建或从用户 Mac 补回这些排除内容。
- 原归档还排除了 legacy_strix producer_1_5_3 下 4 个事件/漏洞数据文件（`.state/events.jsonl`、`events.jsonl`、`events.ndjson`、`vulnerabilities.csv`）和 producer_1_6_2 下 `events.jsonl`，共 5 个未验证合成来源的夹具。
- 过滤器额外漏掉 2 个真实生产 Rust 文件：`agent_runtime/multi_agent/budget/root/target/lifetime.rs` 与 `agent_runtime/multi_agent/budget/target/receipt.rs`（相对 `src-tauri/src/`）。候选保留完整远端中的原文件且没有改动；没有用 stub 代替，也没有导入过滤删除。
- 因此原归档不是完整本地测试树。候选包含远端已有文件，不代表排除的本地测试/夹具已完成比对或验证。秘密扫描属于启发式检查，不是所有秘密均不存在的数学证明。

后续主重构任务可从此候选的受测提交取业务增量；如采用文档 tip，则其直接父提交仍是上述受测提交。合并与后续集成决定由后续任务处理，本次只创建独立候选远端分支。

## 51 项增量路径映射

| 原本地路径 | 候选实际归属 |
| --- | --- |
| `package-lock.json` | `package-lock.json` |
| `package.json` | `package.json` |
| `src-tauri/Cargo.lock` | `src-tauri/Cargo.lock` |
| `src-tauri/Cargo.toml` | `src-tauri/Cargo.toml` |
| `src-tauri/src/commands.rs` | `src-tauri/src/commands.rs` |
| `src-tauri/src/commands/asset_query_rule_tests.rs` | `src-tauri/src/commands/asset_query_rule_tests.rs` |
| `src-tauri/src/commands/asset_response_groups.rs` | `src-tauri/src/commands/asset_response_groups.rs` |
| `src-tauri/src/commands/asset_response_groups_tests.rs` | `src-tauri/src/commands/asset_response_groups_tests.rs` |
| `src-tauri/src/commands/asset_rule_catalog.rs` | `src-tauri/src/commands/asset_rule_catalog.rs` |
| `src-tauri/src/commands/assets.rs` | `src-tauri/src/commands/assets.rs` |
| `src-tauri/src/commands/page_isolation.rs` | `src-tauri/src/commands/page_isolation.rs` |
| `src-tauri/src/commands/workspace_projects.rs` | `src-tauri/src/commands/application_overview.rs` |
| `src-tauri/src/db_initialize_budgets.rs` | `src-tauri/src/db_initialize_budgets.rs` |
| `src-tauri/src/db_schema.rs` | `src-tauri/src/db_schema.rs` |
| `src-tauri/src/jobs.rs` | `src-tauri/src/jobs.rs` |
| `src-tauri/src/jobs/asset_identity.rs` | `src-tauri/src/jobs/asset_identity.rs` |
| `src-tauri/src/jobs/asset_import_lock_tests.rs` | `src-tauri/src/jobs/asset_import_lock_tests.rs` |
| `src-tauri/src/jobs/asset_import_tests.rs` | `src-tauri/src/jobs/asset_import_tests.rs` |
| `src-tauri/src/jobs/job_resume.rs` | `src-tauri/src/jobs/job_resume.rs` |
| `src-tauri/src/jobs/job_resume_tests.rs` | `src-tauri/src/jobs/job_resume_tests.rs` |
| `src-tauri/src/jobs/native_collection_resume.rs` | `src-tauri/src/jobs/native_collection_resume.rs` |
| `src-tauri/src/jobs/native_collection_resume_tests.rs` | `src-tauri/src/jobs/native_collection_resume_tests.rs` |
| `src-tauri/src/jobs/native_collection_resume_transport.rs` | `src-tauri/src/jobs/native_collection_resume_transport.rs` |
| `src-tauri/src/jobs/native_config.rs` | `src-tauri/src/jobs/native_config.rs` |
| `src-tauri/src/jobs/native_content.rs` | `src-tauri/src/jobs/native_content.rs` |
| `src-tauri/src/jobs/native_discovery.rs` | `src-tauri/src/jobs/native_discovery.rs` |
| `src-tauri/src/jobs/native_discovery_tests.rs` | `src-tauri/src/jobs/native_discovery_tests.rs` |
| `src-tauri/src/jobs/native_fofa.rs` | `src-tauri/src/jobs/native_fofa.rs` |
| `src-tauri/src/jobs/native_probe.rs` | `src-tauri/src/jobs/native_probe.rs` |
| `src-tauri/src/jobs/native_response_identity.rs` | `src-tauri/src/jobs/native_response_identity.rs` |
| `src-tauri/src/jobs/native_response_identity_tests.rs` | `src-tauri/src/jobs/native_response_identity_tests.rs` |
| `src-tauri/src/jobs/native_workers.rs` | `src-tauri/src/jobs/native_workers.rs` |
| `src-tauri/src/jobs/tray_activity.rs` | `src-tauri/src/jobs/tray_activity.rs` |
| `src-tauri/src/lib.rs` | `src-tauri/src/lib.rs` |
| `src-tauri/src/models_asset.rs` | `src-tauri/src/assets/model.rs` |
| `src-tauri/src/page_isolation.rs` | `src-tauri/src/page_isolation.rs` |
| `src-tauri/src/page_isolation.schema.sql` | `src-tauri/src/page_isolation.schema.sql` |
| `src-tauri/src/page_isolation_tests.rs` | `src-tauri/src/page_isolation_tests.rs` |
| `src-tauri/tauri.conf.json` | `src-tauri/tauri.conf.json` |
| `src/App.vue` | `src/App.vue`<br>`src/features/assets/workspace/AssetsPluginWorkspace.vue` |
| `src/components/AssetRuleSettings.vue` | `src/features/assets/components/AssetRuleSettings.vue` |
| `src/components/AssetWorkspace.vue` | `src/features/assets/components/AssetWorkspace.vue` |
| `src/components/ConfigDialog.vue` | `src/features/configuration/components/ConfigDialog.vue` |
| `src/components/PageIsolationDialog.vue` | `src/features/assets/components/PageIsolationDialog.vue` |
| `src/components/QueryPanel.vue` | `src/features/assets/components/QueryPanel.vue` |
| `src/components/ReleaseNotesDialog.vue` | `src/shell/components/ReleaseNotesDialog.vue` |
| `src/components/TitleRulePopover.vue` | `src/features/assets/components/TitleRulePopover.vue` |
| `src/features/assets/api.ts` | `src/features/assets/api.ts` |
| `src/styles.css` | `src/features/configuration/styles/navigation.css` |
| `src/types.ts` | `src/features/assets/types.ts` |
| `tools/test_asset_workspace_review.cjs` | `tools/test_asset_workspace_review.cjs` |
