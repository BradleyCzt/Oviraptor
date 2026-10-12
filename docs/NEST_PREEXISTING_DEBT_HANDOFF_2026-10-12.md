# Nest 存量技术债务交接（2026-10-12）

本文件汇总接手会话（`homogeneous-execution-closure` 分支）实测确认的存量技术债，
供后续会话按优先级处置。所有数字都有可复现命令；「base」指 `8fddee2`
（本分支分叉点，即上游 main 当前提交）。**本分支的改动不是这些债的成因**
（见 §B 对照方法）；本分支自身交付面全绿（见 §D）。

## A. 全量回归快照（本分支，2026-10-12）

命令：`cargo test --manifest-path src-tauri/Cargo.toml --lib -- --skip homogeneous --skip container_parser --skip orphans --skip target_branch --skip review_branch --skip reviewer_same --skip unverified_target --skip tampered_fact --skip replay_or_expired --skip lane_exclusivity --skip target_lane_paid --test-threads=8`
（串行敏感组单独以 `--test-threads=1` 跑，见 §D。）

结果：**3066 passed / 318 failed / 7 只测试挂起**（挂起被终止，未计入失败）。
被跳过的串行组单独结果见 §D，全绿。

挂起清单（全在 exposure/ownership 支线）：

- `commands::exposure::tests_g2::g2_live_github_anonymous_repository_search_once`
- `commands::exposure::tests_g3::g3_live_public_documents_are_read_once_each`
- `commands::exposure::tests_g4_scale::g4_4_scale_tier`
- `commands::exposure::tests_g4_scale::rem_scale_tier_with_per_asset_projection`
- `commands::ownership_o3_tests::scale_100000_rows_reconciles_and_reports_metrics`
- `commands::ownership_o3_tests::scale_200000_rows…`（同簇）
- `commands::ownership_review_o2_tests::acceptance_on_extracted_project_copy`

定性：exposure 支线含**真实外网依赖测试**（GitHub 匿名搜索、公共文档抓取），
网络抖动时无限等待且无超时兜底；ownership 规模测试（10 万/20 万行）单条耗时极长。
建议后续：活网络测试加显式超时或环境开关（如 `OVIRAPTOR_LIVE_NET_TESTS=1` 才启用），
规模测试标注 `#[ignore]` 由门禁显式选择执行。**这两处均在 exposure 冻结区，
按交接黑名单须在独立会话处置，不得混入执行链交付。**

## B. 失败簇归属：base 对照证据

方法：在 `/tmp/oviraptor-base` worktree 检出 `8fddee2`，重跑本分支上失败的
代表性测试。**全部在 base 同样失败**，即存量债：

| 簇 | base 复测结果 |
|---|---|
| `collaboration_events::{pump,signal}`（12 条） | `collaboration_events` 过滤下 base 即 **10 failed / 5 passed** |
| `assignment_attempt_real_http_takeover_records_original_bill…` | base 1 failed |
| `assignment_attempt_real_model_takeover_keeps_original_cost…` | base 1 failed |
| `budget_clock_target_http_transport_obeys_shared_deadline` | base 1 failed |
| `budget_target_http_claim_rechecks_revocation_before_transport` | base 1 failed |
| `coordinator_frame_step_inventory_is_explicit_for_wired_kinds` | base 1 failed |
| `agent_runtime::tests::a_legacy_database_is_upgraded_in_place…` | base 1 failed |

318 条失败未逐条复测（成本过高）；抽样覆盖了与本分支改动最邻近的
broker/budget/coordinator 簇与最远的 collaboration 簇，均属存量。
与文档链自述一致：`NEST_CONTINUATION_HANDOFF_2026-10-01.md` 记载
「325/440 历史失败」与「2446 项 Rust 从未全量」；本次 318 + 挂起 7 与该口径同量级。

后续排查建议：先在 CI 或门禁脚本固化「全量 + 跳过活网络/规模」的基线数，
任何分支只要求**不新增失败**（对照 base），存量红随专项批次逐簇清偿。

## C. 文档链已记载、仍未清偿的存量债（按风险排序）

1. **Exposure 支线三个高危缺陷**（`ASSET_EXPOSURE_STATUS_SUMMARY_2026-10-10` §3.1）：
   U-01 复核面板跨项目写入、U-02 `review_scoped_cas` 不校验归属、
   U-04 观察 URL 凭据参数原文入库；另有 38 条未完善。处置须在冻结区外的
   独立会话进行（黑名单见 DESIGN 附录 D）。
2. **legacy Strix 物理删除未收口**（10-01 交接「未完成项 1」）：
   `result_ingestion_runs.rs` 旧 marker 删除语义、`SentinelVulnerabilitiesPane.vue`
   `stage==='strix'` 分支、ReleaseNotesDialog 文案、约 46 文件活/字面量混合待逐项审。
3. **真实旧数据清理半程**（`NEST_REAL_DATA_RETIREMENT_AUDIT_2026-10-02`）：
   真实库 59 行已清+备份；CAS 59 对象/37 bundle 未证明旧数据独占不删；
   REM-A13 归属盘点/备份/迁移未做。
4. **REM 横切项**：A02（十维对账与授权恢复）、A06（真并行）、A07（取消/强杀恢复）、
   A09（逐路实时日志）、A10（整体 UI）、A14（最终验收，依赖用户提供两个授权 URL）均 active。
5. **回归债本体**：本文件 §A/§B；建议按簇立项，从 collaboration_events
   （与主线耦合最低）开始。
6. **Stage 12 发布验收**未启动（安装包验收流程见 README「macOS 安装产物核对」）。

## D. 本分支已验证绿的面（后续会话的基线 delta）

| 命令 | 结果 |
|---|---|
| `cargo test --lib homogeneous -- --test-threads=1` | 128 passed / 0 failed |
| 新增执行链 9 测试（`agent_tests_homogeneous_execution.rs`，逐一） | 各 1 passed |
| `cargo test --lib container_parser` / `--lib orphans` | 8 passed / 2 passed |
| `cargo test --lib authorization -- --test-threads=1` | 56 passed / 0 failed |
| `cargo test --lib intent_tests -- --test-threads=1` | 20 passed / 0 failed |
| `cargo test --lib frozen_tool_supply` | 7 passed / 0 failed |
| `cargo test --lib tool_execution_readiness` / `execution_readiness` | 3 + 5 passed |
| `cargo clippy --all-targets -- -D warnings` | 0 errors（含清偿 26 条存量） |
| `npm run build` | vue-tsc + Vite 通过 |
| `npm run test:agent-workbench` | 54 passed / 0 fail |

沙箱镜像基线：钉死 digest `sha256:114c7a44…` 已按 digest 从 Hub 拉取就位
（linux/arm64、`/usr/bin/python3` 3.11、无卷）。缺该镜像时 2 条容器测试报
`container_parser_fixed_image_required` 属设计内 coverage gap，不是缺陷。

## E. 移交 main 分支维护者的验收项（非本分支范围，2026-10-12 会话确认）

- 真实模型验证（交接闭环 6）：需配置中心有效 DeepSeek flash 凭据；
  新试验用新捕获目录，保留真实失败回执；禁用旧注入 pro Key。
- 安装测试 App 端到端（闭环 7）：构建安装 + 双合成账号界面全流程 + 唯一发布收官；
  跨平台（Windows/Linux）声明同属上游验收。
- 这两项完成前，整体执行链/Master Plan 的最终验收结论由上游掌握。
- 仍可在后续贡献分支推进的技术项：Root finally/重启回收专项测试；
  解析回调重复验证运行环境的可选优化（不得减少取消/撤销检查）。
