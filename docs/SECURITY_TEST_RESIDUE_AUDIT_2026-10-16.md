# 安全测试相关内容残留审计（2026-10-16）

**结论：未完全清理。** 当前主线（`main` @ `19756ce`，含 PR #7）的 git 追踪树中，共有 **911 个文件、10,004 行**命中安全测试相关词条（附录 A 全量清单）；另有 `dist/` 构建产物 14 个文件含同类内容。

## 审计口径

- **基准**：`main` @ `19756ce`（2026-10-15 合入 external-asset-accuracy 之后）。
- **范围**：全部 git 追踪文件（`src-tauri/`、`src/`、`tools/`、`docs/`、根配置）；`dist/` 为未追踪构建产物，单列一节；`node_modules/` 不在范围。
- **词条**（不区分大小写）：`strix`、`sentinel`、`appsec`、`src_assurance`、`src-assurance`、`frontend_recon`、`runtime_probe`、`security_rule`、`semgrep`、`codeql`、`recon`（子串，含误命中）、`安全测试`、`安全扫描`、`安全规则`、`渗透`、`pentest`、`penetration`、`exploit`、`漏洞`、`弱口令`、`weak password`、`sensitive`。
- **判定基准**：项目自身的退出改造（README「旧版 Strix 运行时和结果格式不再接受」及 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`）。产品现役功能词（资产采集/分层/存活探测、HackerOne 同步、FOFA）不计入残留。
- **方法**：`git grep -i` 逐词计数与行号定位；附录 A 为全部词条合并后的逐文件去重行数清单。
- 本文档只登记位置，不含处理建议。

## 总量

| 区域 | 文件数 | 命中行数 |
|---|---|---|
| `src-tauri/src/`（含测试模块） | 521 | 3,554 |
| `src/`（前端） | 104 | 1,160 |
| `docs/`（md + json 记录） | 191 | 4,000+ |
| `tools/` | 73 | ~700 |
| `src-tauri/tests/`（fixture） | 6 | 54 |
| 根配置（README、transfer、package.json、.gitignore） | 4 | 43 |
| `dist/`（未追踪构建产物） | 14 | — |

---

## 1. Rust 生产代码（非测试构建会编译执行）

### 1.1 strix（4 处生产命中；其余在测试/fixture）

- `src-tauri/src/db_settings.rs:13` — 设置清理命名空间过滤 `starts_with("strix")`
- `src-tauri/src/commands/retired_data_guards.rs:4,5,8` — 拒绝式守卫（`stage IN ('strix','strix-coverage')`、`stage GLOB 'strix_*'`、`strix-current-attempt:{id}` 等 app_settings 键）
- `src-tauri/src/commands/agent_acceptance.rs:3` — 文件头注释 "no Strix, no Docker…"

### 1.2 appsec 校验域（活动生产全链）

- `src-tauri/src/commands/appsec_validation.rs` — 整文件（110, 293, 300, 432, 471, 531, 540, 543, 547, 568-570, 603, 606, 626-629, 657-663, 665, 675-680, 685, 688, 690, 693, 714, 718, 722, 730, 752, 756, 764-766, 783, 811, 813, 816, 829, 838, 849, 975, 989, 993, 1052, 1055, 1059, 1064, 1066, 1075, 1084, 1093, 1101, 1108, 1111, 1115, 1116, 1119, 1126, 1133, 1137, 1140, 1156-1163, 1169, 1172, 1176, 1177, 1183, 1186, 1190, 1197, 1211, 1212）：`AppSecCandidate`、`refresh_appsec_vulnerabilities`、`appsec_vulnerabilities`/`appsec_vulnerability_sources` 表读写、`evaluate_appsec_gate`、Tauri 命令 `list_appsec_scan_result`（:476）、`sync_investigation_appsec_vulnerability`（:820，engine `'Oviraptor Repeater'`）
- `src-tauri/src/lib.rs:634` — `list_appsec_scan_result` 命令注册
- `src-tauri/src/scanning.rs:244` — `include!("commands/appsec_validation.rs")`
- `src-tauri/src/scanning/model.rs:720,746,760,780-783` — `AppSecVulnerability`/`AppSecVulnerabilitySource`/`AppSecScanContext`/`AppSecScanResult` 结构体
- `src-tauri/src/db_schema.rs:348,372,374,384,385` — `appsec_vulnerabilities`/`appsec_vulnerability_sources` 建表与索引
- `src-tauri/src/db_initialize_columns.rs:312,336,338,348,349` — 同上建表/索引
- `src-tauri/src/commands/workspace_projects.rs:60,124,125,144,161` — 工作台统计 `appsec_vulnerability_count`
- `src-tauri/src/workspaces/model.rs:39` — `pub appsec_vulnerability_count: i64`
- `src-tauri/src/commands/exposure.rs:122`、`src-tauri/src/commands/agent_native/persistence.rs:70`、`src-tauri/src/agent_runtime/multi_agent/homogeneous/decompose.rs:196` — appsec 相关注释
- `src-tauri/src/skills_catalog/learning_model.rs:56`、`patch.rs:197`、`sources.rs:376` — "AppSec 技能工程师/知识工程师" 系统提示词

### 1.3 安全规则包域（活动生产全链）

- `src-tauri/src/commands/rule_packs.rs` — 整文件（:39 `list_security_rule_packs`、:55 `save_security_rule_pack`、:102 `delete_security_rule_pack`、:285 `sync_security_rule_pack`（git 检出规则库）等，共 18 行）
- `src-tauri/src/lib.rs:535-538` — `list/save/delete/sync_security_rule_pack` 四个 Tauri 命令注册
- `src-tauri/src/db_initialize_columns.rs:203-205, 245-270, 270, 593-597, 603` — `security_rule_packs` 建表、索引与内置包种子
- `src-tauri/src/commands/result_ingestion_engines.rs:2` — `SELECT local_path FROM security_rule_packs WHERE engine=?1` 规则包路径解析

### 1.4 src-assurance（活动生产）

- `src-tauri/src/commands/src_assurance.rs:1, 235, 243` — `SRC_ASSURANCE_ADAPTER_NAME`、`stage_builtin_src_assurance`（整文件为该适配器；输出文件名已中性化为 `src-capabilities.json`，函数名保留）
- `src-tauri/src/commands/agent_backend.rs:500, 672` — 读取与调用上述适配器
- `src-tauri/src/scanning.rs:100` — `include!("commands/src_assurance.rs")`

### 1.5 前端侦察 / 运行时探测（活动生产执行链）

- `src-tauri/src/commands/frontend_recon.rs:2-4`、`frontend_recon_artifacts.rs:1,2,12,163,164`、`frontend_recon_compact.rs:1,2`、`frontend_recon_scoring.rs:3` — include 与产物读取
- `src-tauri/src/commands/scan_execution_frontend.rs:35,63,173,193,217,218,254,263,270,337,338,363,372` — 侦察生产执行链（`cached_frontend_recon`、`launch_frontend_recon_producer`、worker kind `"frontend_recon"`）
- `src-tauri/src/commands/scan_execution_runtime.rs:16,29,33,35,36,41,53,61,65` — `FRONTEND_RECON_HARD_TIMEOUT_SECONDS`、`frontend_recon_config`（读 `config/frontend_recon.json`）
- `src-tauri/src/commands/native_frontend_recon.rs:414`（`run_native_frontend_recon`）及 :78,79,82,125,131,217,231-233,240,833,923,927（sensitive 规则接线）
- `src-tauri/src/commands/agent_tools_browser.rs:200` — `native_runtime_probe` 调用；`native_frontend_recon.rs:2,505,604,922` — 定义/调用/`runtime_probe_incomplete`
- `src-tauri/src/commands/native_helper_log_owner.rs:46,48,54`、`native_helper_process.rs:44`、`native_helpers.rs:97,98,134`、`worker_resources.rs:14,17,18,22,27,162,165` — `9_frontend_runtime_probe.cjs` 打包解出与日志归属
- `src-tauri/src/commands/web_investigation_policy.rs:100` — 策略面暴露 `"available": "runtime_probe"`
- `src-tauri/src/commands/investigation_read.rs:21`、`investigation_surface.rs:81`、`result_ingestion_recon.rs:84,122,397`、`scan_lifecycle.rs:745,847`、`scan_quiescence.rs:50`、`scan_execution.rs:68`、`web_dispatch_binding.rs:195`、`web_dispatch_recovery.rs:183`、`code_analysis.rs:314`、`environment.rs:6,62`、`agent_backend.rs:140` — `stage='frontend_recon'` 读写、worker 解析与配置传递
- `src-tauri/src/artifact_import/adapters/frontend_recon.rs:1,101`、`adapters/mod.rs:5,205`、`reconcile.rs:22` — 导入适配器（`ADAPTER_PRIORITY` 含 `frontend_recon`）
- `src-tauri/src/commands/native_recon_contract.rs`（文件级）— 侦察契约

### 1.6 sensitive 规则引擎（活动生产）

- `src-tauri/src/commands/native_sensitive.rs:115,134,339,354,382` — 整模块为 `resources/rules/sensitive.json` 规则扫描引擎
- `src-tauri/src/commands/frontend_recon_compact.rs:143,158,363,466,529,570,609,610,614,763,765,766,811,831`、`frontend_recon_scoring.rs:267-283,359,378,386,394`、`result_ingestion_recon.rs:177,187,215` — sensitive 压缩/计分/入库
- `src-tauri/src/commands/agent_tools_replay.rs:17,24`、`agent_tools_specs.rs:24`、`agent_native/queue.rs:181` — 工具面 `sensitiveCandidates`
- `src-tauri/src/commands/exposure/rules.rs:78,273,286`、`exposure/adapters/http.rs:455` — 敏感泄露规则
- `src-tauri/src/commands/scan_execution.rs:97`、`scan_execution_frontend.rs:47,50`、`conversations/access/files.rs:124,146`、`skill_packages/store.rs:102,131`、`agent_runtime/secrets.rs:361`、`agent_tools_http/evidence.rs:3`、`exposure.rs:788,848` — 泛 sensitive 引用

### 1.7 semgrep / codeql 安全分析（活动生产）

- `src-tauri/src/commands/native_source_scan.rs`（文件级，:103,104,137,246,248,252 等经 `security_rule_packs.local_path` 组 `AnalyzerSpec`）
- `src-tauri/src/commands/result_ingestion_engines.rs:2` — 引擎规则包路径
- `src-tauri/src/native_pipeline/`（`tools.rs`、`analyzer` 相关，文件级）

### 1.8 安全测试 / exploit / 渗透类字样（生产文案与提示词）

- `src-tauri/src/commands/investigation_deep_dive.rs:269` — 「安全测试窗口、可接受速率、最大分页/导出规模和任务配额」
- `src-tauri/src/commands/native_sensitive.rs:2` — "vulnerability is exploitable" 注释
- 「漏洞」生产命中 25 文件 34 行（多为防升级措辞与风险描述）：`agent_runtime/multi_agent/homogeneous/execution_worker.rs:16`、`model_worker.rs:10`、`commands/agent_directive_proposals.rs:255,260`、`agent_gap_review.rs:8`、`agent_homogeneous_execution.rs:16`、`agent_native/business_logic_input_collection.rs:7`、`agent_native/external_surface_output_collection.rs:131`、`agent_native/persistence.rs:230`、`agent_native/queue.rs:12,13,17`、`agent_tools_http/observations.rs:85,187`（jQuery 已知 XSS/公开漏洞）、`agent_tools_identity_diff.rs:227`、`agent_tools_verdict.rs:106`、`exposure/remediation.rs:779`、`investigation_deep_dive.rs:252`、`investigation_persist.rs:115`、`knowledge_learning_candidates.rs:84`、`knowledge_learning_traces.rs:116,268,335,348,357`、`multi_agent/prepare.rs:273`、`multi_agent_external_surface.rs:463`、`native_recon_contract.rs:167`、`native_scan_branches/status.rs:167`、`native_source_coordinator.rs:240`、`result_ingestion_recon.rs:156,235`、`scan_lifecycle.rs:724`（`contains("漏洞")` 逻辑分支）、`web_investigation_policy.rs:122`

### 1.9 sentinel（`sentinel_*` 表名/命令名/类型名层面的生产残留，共 187 个非测试文件、约 1,319 行）

形态：`sentinel_scans`(394)、`sentinel_targets`(144)、`sentinel_scan_attempts`(117)、`sentinel_deleted_scans`(96)、`sentinel_scan_contexts`(63)、`sentinel_findings`(56)、`sentinel_checkpoints`(53)、`sentinel_fuse_zone`(41)、`sentinel_opportunities`(40)、`sentinel_validations`(25)、`sentinel_settings`(16)、`sentinel_processes`(16)、`sentinel_stages`(7)、`sentinel_bundle`(12) 等 SQL 表名与 Tauri 命令名。逐文件行号：

- `db_schema.rs`（73 行，全套 `sentinel_*` 建表）、`db_initialize_columns.rs`（43 行）、`db_initialize_repairs.rs`（37 行）、`db_migrate.rs:88,89,95,115,121,135,141,161,210,216,224`、`db_initialize_budgets.rs:58,67,101,111,114,115,126,134,135,139,148,150,151,155,156`、`db.rs:131,136,258,308`、`db_neutral.rs:31`
- `lib.rs:164,172,173,175,176,180,384,385,501-504,541-564,624,625,635-643` — 注册约 44 个 `*_sentinel_*` Tauri 命令（rescan/confirm/pause/resume/cancel/delete/list/archive_sentinel_scan、list_sentinel_targets/fuse_zone/checkpoints/findings、get/read_sentinel_runner_log 等）
- `commands/scan_control.rs`（127 行）、`scan_lifecycle.rs`（88 行，含 :847 `SENTINEL_RESCAN_RECON_COPY_SQL`）、`scan_startup.rs`（22 行）、`scan_quiescence.rs`（28 行）、`scan_deletion.rs:3,13,16,72,80,86,87,94,99,100,114,128`、`scan_execution.rs`、`scan_execution_frontend.rs`、`scan_execution_routing.rs`、`runtime_config.rs`（12 行，维护 `sentinel_processes`）、`environment.rs:175`
- `commands/agent_backend.rs`、`agent_backend_storage.rs`、`agent_contract.rs`、`agent_dialog_selection.rs`、`agent_dialog_view.rs`、`agent_gap_followup.rs`、`agent_gap_review.rs`、`agent_gap_submission.rs`、`agent_request_accounting.rs`、`agent_request_review.rs`、`agent_runtime.rs`、`agent_terminal_legacy_consumer.rs`、`agent_terminal_legacy_fuse.rs`（`sentinel_fuse_zone`）、`agent_terminal_legacy_writer.rs`、`agent_terminal_report.rs`、`agent_original_terminal_identity.rs`
- `commands/agent_native/`（coordinator_identity_frame、coordinator_lease_closed、coordinator_lease_reviewer、coordinator_protection_original、coordinator_recovery_source、persistence、single_budget、single_finally、web_executor_adapter）
- `commands/native_financial_continuation_*`（approval/authorization/discovery/failure_writer/files/finish/preview/status/worker）、`native_protection_recovery_*`（cancel/contract/draft/signature/sources）、`native_scan_branches.rs` 及子模块（directives/history/root_cost_reconciliation/status/status_budget/status_timeline_page）、`native_single_continuation_model_contract.rs`、`native_single_continuation_preview.rs`、`native_source_*`（analysis/assessment_authority/branch_completion/ci_policy/completion/coordinator/failed_deletion/findings/pause_branch/pause_closure/pause_recovery/pause_recovery_writer/runtime/scan/scope）、`native_branch_dispatch.rs`、`native_execution_history.rs`、`native_helper_live_limits.rs`、`native_helper_log_owner.rs`、`native_helper_process.rs`、`native_helpers.rs`、`native_process_log_replay.rs`、`native_web_mode_binding.rs`
- `commands/web_administrative_closure.rs`、`web_closure_handoff.rs`、`web_dispatch_binding.rs`、`web_dispatch_closure.rs`、`web_dispatch_recovery.rs`、`workbench_retry.rs`、`workbench_startup.rs`、`workspace_projects.rs`（含 `sentinel_fuse_zone` 表计数）、`historical_agent_trace.rs`、`historical_import_views.rs`、`investigation_persist.rs`、`investigation_read.rs`、`investigation_surface.rs`、`knowledge_learning_candidates.rs`、`knowledge_learning_traces.rs`、`multi_agent/findings.rs`、`multi_agent/review_validation.rs`、`multi_agent_external_surface.rs`、`result_ingestion.rs`、`result_ingestion_recon.rs`、`result_ingestion_runs.rs`、`retired_data_guards.rs`、`frontend_recon_artifacts.rs`、`code_analysis.rs`、`assets.rs`、`asset_export.rs`、`asset_ownership.rs`、`asset_response_groups.rs`
- `agent_runtime/`：`contract.rs:518`（`to_sentinel_status`）、`deleted_scan_audit.rs` 及 `rows.rs`/`scope.rs`/`writer.rs`、`model/diagnostics/replay.rs`、`multi_agent/attempts/expiry_proof.rs`、`multi_agent/budget/clock/closure_writer.rs`、`pause_authority.rs`、`financial_continuation_schema.sql:33`、`root.rs`、`root_definition.rs` 及 `guard.rs`、`client_side.rs`、`directive/confirmation.rs`、`directive/human_review/scope.rs`、`directive/reconciliation.rs`、`homogeneous/decompose.rs`、`intent.rs`、`lease.rs`、`mailbox.rs` 及 `send.rs`、`source.rs`、`single_projection.rs` 及 `deletion.rs`、`store.rs:442`、`target_requests.rs`、`web_mode/action.rs`、`draft.rs`、`writer.rs`
- `artifact_import/`：`adapters/sentinel_bundle.rs`（旧 Sentinel bundle 导入适配器，仍编译）、`adapters/sentinel_stages.rs`、`adapters/mod.rs:9,10,206,207`、`reconcile.rs:22,108,193,209`、`service.rs`、`store.rs:213`、`projection.rs`
- 其余：`assets/model.rs`、`auth_session.rs`、`collaboration_events.rs`、`conversations/store.rs`、`conversations/tools.rs`、`native_pipeline/greybox.rs:352`、`native_pipeline/process_log/persist.rs`、`page_isolation.rs`、`scanning.rs:31,37,265`（worker_bridge 重导出 sentinel 命名 API）、`scanning/model.rs`（`SentinelScan` 等模型）、`snapshot_export.rs`、`tool_supply/frozen_plan.rs`、`tool_supply/selection.rs`、`worker.rs`、`worker_resources.rs:32`（`sentinel_runtime_path`）、`workspaces/model.rs`

### 1.10 兼容/历史性质的生产命中（事实标注，非建议）

- strix 4 处生产命中均为「检测即拒绝」守卫或清理过滤（§1.1）。
- `artifact_import` 的 `sentinel_bundle.rs`/`sentinel_stages.rs`、`db_migrate.rs` legacy_transition、`historical_import_views.rs`、`historical_agent_trace.rs`、`deleted_scan_audit/*`：旧数据导入与审计（仍编译、仍可触发）。
- `worker_resources.rs:32` `sentinel_runtime_path`：函数名保留，语义仅为 Node 可执行路径解析。

---

## 2. 前端 `src/`（104 个文件、约 1,160 行）

### 2.1 sentinel 命名（目录 `src/features/sentinel/` 本身 134 个文件；其中 104 个文件内容含 sentinel 字样）

- 入口与注册：`src/plugins/catalog.ts:45,46,48-51`、`src/plugins/preferences.ts:8`、`src/plugins/runtime.ts:18,32,53,56,91,93,108`、`src/plugins/types.ts:3,7,54,65,67,75`、`src/shell/WorkspaceNavigation.vue:8,33`、`src/shell/builtinWorkspaces.ts:8`、`src/shell/moduleCatalog.ts:5,21,38,40,43`、`src/shell/navigation.ts:51`、`src/shell/types.ts:14`、`src/App.vue:15,17,191,226,231,288,299,300,452,509,518`、`src/api.ts:4,10,25`、`src/types.ts:7,8`
- 核心组件（`src/features/sentinel/components/`）：`SentinelBoard.vue`（约 160 行命中）、`SentinelFuseZone.vue`、`SentinelRepeater.vue`、`SentinelTaskCenter.vue`、`SentinelValidationWorkbench.vue`、`SentinelAuthRecoveryPanel.vue`、`AgentDialog.vue`、`AgentWorkbench.vue`、`AgentTraceHub.vue`、`AgentRequestReview.vue`、`AgentIdentityMaterial.vue`、`HistoricalJsonImport.vue`、`NativeBudgetDiagnostics.vue`、`NativeRunStatus.vue`、`NativeSourcePauseRecovery.vue`、`SourceReviewEvidence.vue`、`lazyPanels.ts`
- 结果面板（`results/`）：`SentinelApiPane.vue`、`SentinelEndpointsPane.vue`、`SentinelFingerprintPane.vue`、`SentinelJsFiles.vue`、`SentinelOpportunitiesPane.vue`、`SentinelRuntimeExploration.vue`、`SentinelSourceResults.vue`、`SentinelTraceTimeline.vue`、`SentinelVulnerabilitiesPane.vue`
- 概览（`overview/`）：`SentinelOverviewSummary.vue`、`SentinelTaskOverview.vue`、`SentinelTokenOverview.vue`
- composables / execution / fuse / workbench / workspace / traces 子目录及 `api.ts`（:249-266 `list_sentinel_*` 命令封装）、`types.ts`、`presentation.ts` 等
- 样式：`src/sentinel.css`、`src/styles/sentinel-base.css`、`sentinel-layout.css`、`sentinel-results.css`、`sentinel-trace.css`、`sentinel-workbench.css`、`src/styles.css:20-46`、`src/theme-enhancements.css`（106 行）、`src/asset-workspace-patch.css:360-368`
- 跨功能引用：`src/features/assets/components/AssetWorkspace.vue`（25 行）、`AssetOwnershipGate.vue:162`、`src/features/assets/exposure/useExposureSurface.ts:390`、`src/features/assets/types.ts:42-44,91`、`src/features/configuration/components/ConfigDialog.vue:15,17`、`src/features/conversations/ConversationPluginPanel.vue`、`useConversationPlugins.ts`、`src/features/runtime/components/WorkerSettingsPanel.vue`、`src/features/sentinel/workspace/ScanningPluginWorkspace.vue`（92 行）

### 2.2 appsec

- `src/App.vue:304`（`appsecVulnerabilityCount ... 条漏洞结论`）、`src/features/sentinel/api.ts:5,247,248`（`invoke("list_appsec_scan_result")`）、`src/features/sentinel/types.ts:276,300,312,330-333`（`AppSecVulnerability` 等类型）、`src/features/workspaces/types.ts:34`
- `SentinelBoard.vue:92,289,876-880,937-1005,1438-1455,1804-1826,2669-2686`、`results/SentinelSourceResults.vue:18,55-57,71,754-964`（SAST/DAST/IAST/SCA 关联视图）、`src/styles/sentinel-results.css`（19 行 `.appsec-*`）、`src/theme-enhancements.css:203,220`

### 2.3 安全规则 / semgrep / codeql / sensitive / 侦察词汇

- `src/features/sentinel/rulePacksApi.ts:2,6-9` — `list/save/delete/sync_security_rule_packs` 封装与 `SecurityRulePack` 类型
- `src/features/configuration/components/ConfigDialog.vue:111-115,144,145,193-208,886,1078` — semgrep/codeql 引擎与规则库配置（`builtinRepository: "https://github.com/semgrep/semgrep-rules.git"`、`https://github.com/github/codeql.git`）、「安全规则」文案；:220 漏洞规则文案
- `SentinelBoard.vue:90,343,407-410,523,528,548,700,701,2864,2865,3335,3345,3363`、`results/SentinelApiPane.vue`（12 行 sensitive 表）、`SentinelFuseZone.vue:317,342`、`presentation.ts:78,300,319`、`fuse/presentation.ts:33` — `sensitive_info` 发现类型链
- `presentation.ts:9,18,213,249,288`、`agentDialogLabels.ts:113,114`、`overview/SentinelOverviewSummary.vue:33`、`overview/useTokenUsage.ts:33`、`types.ts:391`、`AgentRequestUsage.vue:10`、`sentinel-results.css:1519` — `frontend_recon`/`recon_only` 状态与「前端与接口侦察/仅完成确定性侦察」文案
- `SentinelBoard.vue:1191,1194`、`InvestigationGraphPanel.vue:416,419` — `runtime_probe_unavailable`
- `src/features/skills/useSkillsCatalog.ts:118` — skill 模板 "Do not report reconnaissance-only observations as vulnerabilities"
- 「漏洞」命中 32 文件 99 行，集中在：`SentinelValidationWorkbench.vue`（整页漏洞验证工作台，12 行）、`results/SentinelVulnerabilitiesPane.vue`、`AgentDialog.vue`（10 行）、`SentinelBoard.vue`（15 行）、`SentinelTaskCenter.vue`、`SentinelRepeater.vue`、`SourceReviewEvidence.vue:146`、`overview/*`、`App.vue:304,512,517`、`ConfigDialog.vue:220,719` 等（完整文件清单见附录 A）

---

## 3. 内置资源 `src-tauri/resources/`

| 文件 | 相关内容 |
|---|---|
| `rules/sensitive.json` | 敏感信息/密钥检测规则全集（`secret-aws-access-key`、`secret-google-api-key`、`secret-oauth-client-secret`、`secret-bearer-token`、`secret-jwt`、`secret-private-key`、`secret-hardcoded-credential`、`crypto-weak-algorithm`、`sensitive-ipv4/email/cn-phone/cn-identity` 等） |
| `rules/exclusions.json` | sensitive 规则排除项（ip_address/bearer_token/jwt/hardcoded_credential 的排除信号与正则） |
| `config/frontend_recon.json` | 前端侦察 worker 超时配置（文件名即 frontend_recon） |
| `skills/business_frontend_deep_analysis.md` | 安全测试执行 skill：候选晋升漏洞、Quick/Standard/Deep 验证器、PoC/CVSS/CWE、修复建议、侦察/目录枚举约束（:11,60,72,78,87,115,123,125,142） |
| `workers/9_frontend_runtime_probe.cjs` | 前端运行时探测（CDP）worker，:1678 `"runtime_probe_error"` |
| `workers/8_js_ast_analyzer.cjs` | :162,191 对 authorization/cookie/api-key 等 header 打 sensitive 标记 |
| `workers/tests/fixtures/runtime_probe.html` | 探测夹具页面 |
| `config/browser-locations.json`、`workers/babel-parser.cjs`、`BABEL_PARSER_LICENSE.txt` | 无命中 |

---

## 4. 根配置与工具脚本

- `.gitignore:50,51` — `strix-jobs/`、`sentinel-tasks/`
- `package.json:28` — `test:ui` 引用 `test_sentinel_refresh.cjs`/`test_sentinel_search.cjs`/`test_sentinel_pagination.cjs`
- `tools/retire_nest_data.py:9,104,107,108,110,113` — strix 表/键退役脚本（生产工具）
- `tools/execution_log_events.cjs:7,29,42,54,67,97` — `readSentinelRunnerLog`/`listSentinelScanAttempts`
- `tools/serve_chat_dom.mjs:8,15` — 指向 `src/features/sentinel/` 路径
- `tools/test_rust_business_boundaries.cjs:84,242,244` — 列出 sentinel/appsec/security_rule 路由名与 `9_frontend_runtime_probe.cjs`（守卫测试）

---

## 5. 测试与夹具

### 5.1 Rust 测试（`src-tauri/src/` 内 307 个 test 文件命中；`src-tauri/tests/` 另有 6 个）

- strix 命中测试 45 个（`tests_database_repair.rs` 37 行、`tests_model_config.rs` 33、`tests_neutral_runtime.rs` 23、`tests_result_attempt_surface.rs` 19、`db_settings_tests.rs` 13、`db_tests.rs` 12、`tests_scan_lifecycle.rs` 12、`agent_tests_persistence.rs` 12、`agent_tests_backend_literal.rs` 12、`agent_tests_backend_residual.rs` 11、`db_neutral_tests.rs` 10、`tests_runner_log.rs` 7、`agent_runtime/tests_backend_retirement.rs` 6、`tests_settings_preservation.rs` 5、`tests_retired_deletion.rs` 5、`agent_tests_backend_retirement.rs` 5、其余 1-4 行 × 29 个文件）
- sentinel 命中测试 282 个（`tests_scan_lifecycle.rs` 83、`tests_bundle_import.rs` 64、`tests_scan_quiescence.rs` 47、`tests_web_startup.rs` 43、`tests_scan_deletion.rs` 42、`tests_findings_read_boundary.rs` 39、`tests_overview_source.rs` 33、`tests_web_dispatch_binding.rs` 29、`tests_result_attempt_surface.rs` 29、`db_tests.rs` 28、`tests_web_dispatch_closure.rs` 27、`tests_asset_draft.rs` 22、`tests_workbench_startup.rs` 21、`agent_tests_gap_followup.rs` 21、`agent_tests_authorization_control.rs` 21、其余 1-20 行 × 约 267 个文件）
- appsec/frontend_recon/runtime_probe/漏洞/sensitive 命中测试另见附录 A（`tests_frontend_recon.rs`、`tests_code_rules.rs`、`tests_historical_source_branch.rs`、`tests_web_dispatch_recovery.rs`、`investigation_tests.rs`、`agent_tests_browser.rs`、`native_helper_live_tests.rs`、`src_adapter_authority_tests.rs` 等）
- fixture：`src-tauri/tests/fixtures/legacy_strix/`（`residual_baseline.json` 33 行、`residual_syntax.json` 12 行、`bundles/producer_1_5_3/findings.sarif`、`bundles/producer_1_6_2/findings.sarif`、`bundles/producer_1_5_3/oviraptor_recon.json`）、`tests/fixtures/page_isolation/legacy_525_schema.sql`（3 行）、`src-tauri/tests/backend_retirement_allowlist.json`（REM-012 允许清单，含 sentinel ×2）

### 5.2 前端/Node 测试（`tools/` 内 56 个 `test_*` 文件 + 约 30 个 harness/夹具脚本命中）

- 代表：`tools/test_builtin_plugins.cjs`（24）、`tools/test_plugin_workspaces.cjs`（28）、`tools/test_historical_json_import.cjs`（16）、`tools/test_sentinel_refresh.cjs`（21）、`tools/test_scan_deletion.cjs`（22）、`tools/test_execution_history.cjs`（19）、`tools/test_retire_nest_data.py`（strix 15）、`tools/test_execution_presentation.cjs`（断言界面不含 /strix/i）、`tools/test_vulnerability_results.cjs`（断言 stage 不含 strix）
- harness：`tools/sentinel_board/`（6 个文件 47 行）、`tools/agent_dialog_harness.cjs`（29）、`tools/agent_workbench_harness.cjs`（13）、`tools/plugin_workspace_harness.cjs`（10）、`tools/execution_history/harness.cjs`（10）、`tools/sentinel_api_harness.cjs`（8）、`tools/qa/chat-ui/*`、`tools/chat_dom/*`、`tools/native_*_harness.cjs` 等

---

## 6. 文档（README、transfer.md、docs/ 共 191 个追踪文件）

### 6.1 README.md（3 行）

- :4「旧版 Strix 运行时和结果格式不再接受」、:13「旧 `strix` 配置值不再作为 Native 别名接受」、:46「Strix 退出……以 Master Plan 为准」

### 6.2 transfer.md（37 行）

:4, 15, 19, 21, 24, 28, 33, 39, 45, 63, 71, 73, 74, 83, 85, 86, 91, 120, 125, 149, 171, 173, 175, 183, 184, 192, 194, 195, 196, 204, 223, 224, 225, 227, 228, 229, 230, 231, 232, 263, 264 — Strix 执行端函数名（`resolve_strix_executable`、`launch_strix_workbench_pipeline`、`start_strix_workbench_scan_impl`）、`strix-jobs` 目录、`StrixWorkbenchInput` 等整段记录。

### 6.3 docs/ 中把安全测试功能当作现存/规划功能描述的文档（逐行）

- `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`（40 行）：:107 `strix_adapter.rs` 目录项、:117 枚举值 `Strix`、:586 `strixLlmProfiles`、:834-851 旧 Tauri 命令名 ABI（`test_strix_llm`、`list_strix_traces`、`start_strix_workbench_scan` 等）、:619-623「Strix 退出/变成可选兼容插件」
- `docs/NATIVE_AGENT_EXECUTOR_TASK.md`（29 行）：:22 `run_adaptive_strix_target`、:41 `AgentBackendKind { Native, Strix }`、:190-194 `agentBackendPolicy: auto | native | strix`、:179 `sentinel_findings` 统一 vulnerability contract、:184 漏洞字段（严重度、CWE/CVSS、修复建议）
- `docs/NATIVE_AGENT_PHASE2_COMPLETION_REQUIREMENTS.md`（46 行）：:24「auto 仍固定选择 Strix」、:235「去除 Strix 命名绑架」、:247 `strixLlmProfiles`、:251「Strix 模型配置」、:361「不删除 Strix 兼容后端」
- `docs/NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md`（33 行）：:58 `StrixAgentBackend`、:850 `strix_adapter.rs`、:993「Stage 9：Native 默认与 Strix 退出」
- `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md:267` — 「不自动进行安全测试」
- `docs/RUST_NATIVE_MIGRATION.md:9,22,24,80` — Strix CLI Python 环境、`native_frontend_recon.rs` 映射
- `docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md:82,90,93,176,202,337` — `ReconAnalyst` 角色、「后渗透」命名
- `docs/NEST_HOST_ASSESSMENT_AND_AGENT_STRATEGY_2026-09-27.md:255,358` — 「不做漏洞利用」「不是自动化渗透平台」定位
- `docs/ASSET_EXPOSURE_GUIDE_2026-24_GAP_ANALYSIS_2026-10-10.md`（19 行）— 弱口令/漏洞/渗透范围裁定
- `docs/architecture.md:28` — `sentinelApi` 门面
- `docs/ARCHIVED_RELEASE_NOTES_2026-08-24.md`（19 行）— 旧 Strix 时代发布说明

### 6.4 docs/ 其余（退出计划、审计、交接、进度、交付记录类）

含 strix/sentinel/appsec/frontend_recon/semgrep/codeql/漏洞 等词的文件共约 180 个，含 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`（208 行，退出计划本体）、`progress.md`（116）、`NEST_CONTINUATION_HANDOFF_2026-10-01.md`（102）、`NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md`（90）、`NEST_REM_CONTINUATION_PLAN_2026-10-06.md`（52）、`NEST_MASTER_REMAINING_EXECUTION_2026-10-07.md`（48）、`NEST_UI_MODULE_BOUNDARY_AUDIT_2026-09-28.md`（62）、`NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md`（19）、`docs/integration/module-moves-2026-10-15.json`（1,781）、`docs/local-incremental-20261015/final-api-contract-map.json`（876）等。完整逐文件清单见附录 A。

---

## 7. `dist/` 构建产物（未追踪）

14 个文件含 sentinel/appsec/漏洞/sensitive/semgrep/codeql/recon 词条（由 `src/` 构建产出）：`dist/assets/index-fXTuLBmD.js`、`index-BEwgFN3A.css`、`SentinelBoard-CJenypNc.js`、`SentinelBoard-23bYnvLO.css`、`SentinelTaskCenter-Dl_t_tXa.js`、`SentinelTaskCenter-Cd4uI-RI.css`、`SentinelSourceResults-D1HvwQHz.js`、`AgentTraceHub-BULfcBBo.js`、`AgentDialog-DQvu06hH.js`、`AgentWorkbench-EiQOi6S-.js`、`AssetWorkspace-BJh7Ypw3.js`、`AssetWorkspace-BgcN5THN.css`、`AssetOwnershipGate-BEBH8bkf.js`、`ExposureSurface-_fmy3KEe.js`。

---

## 附录 A：全量逐文件清单（911 个文件，命中行数为全部词条合并去重行数，降序）

docs/integration/module-moves-2026-10-15.json — 1781 行
docs/local-incremental-20261015/final-api-contract-map.json — 876 行
docs/local-incremental-20261015/final-module-contract-map.json — 249 行
docs/integration/verification-2026-10-15-round2.json — 218 行
src/features/sentinel/components/SentinelBoard.vue — 214 行
docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md — 208 行
src-tauri/src/commands/appsec_validation.rs — 127 行
src-tauri/src/commands/scan_control.rs — 127 行
docs/progress.md — 116 行
docs/integration/verification-2026-10-15.json — 115 行
src/theme-enhancements.css — 106 行
docs/NEST_CONTINUATION_HANDOFF_2026-10-01.md — 102 行
src/features/sentinel/workspace/ScanningPluginWorkspace.vue — 92 行
docs/NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md — 90 行
src-tauri/src/commands/scan_lifecycle.rs — 90 行
src-tauri/src/commands/tests_scan_lifecycle.rs — 90 行
src-tauri/src/db_schema.rs — 78 行
src/features/sentinel/api.ts — 72 行
src-tauri/src/commands/tests_bundle_import.rs — 64 行
docs/NEST_UI_MODULE_BOUNDARY_AUDIT_2026-09-28.md — 62 行
src-tauri/src/db_initialize_columns.rs — 61 行
docs/NEST_REM_CONTINUATION_PLAN_2026-10-06.md — 52 行
src-tauri/src/commands/tests_scan_quiescence.rs — 52 行
src/features/sentinel/components/results/SentinelSourceResults.vue — 51 行
docs/NEST_MASTER_REMAINING_EXECUTION_2026-10-07.md — 48 行
src-tauri/src/lib.rs — 48 行
docs/NATIVE_AGENT_PHASE2_COMPLETION_REQUIREMENTS.md — 46 行
src-tauri/src/commands/tests_database_repair.rs — 45 行
src-tauri/src/commands/tests_web_startup.rs — 45 行
src-tauri/src/commands/tests_scan_deletion.rs — 43 行
src-tauri/src/commands/tests_result_attempt_surface.rs — 42 行
src-tauri/src/commands/tests_model_config.rs — 41 行
docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md — 40 行
src-tauri/src/db_tests.rs — 40 行
src-tauri/src/commands/tests_findings_read_boundary.rs — 39 行
src-tauri/src/db_initialize_repairs.rs — 37 行
transfer.md — 37 行
src/features/sentinel/components/results/SentinelApiPane.vue — 34 行
docs/NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md — 33 行
src-tauri/src/commands/tests_overview_source.rs — 33 行
src-tauri/src/commands/workbench_startup.rs — 33 行
src-tauri/tests/fixtures/legacy_strix/residual_baseline.json — 33 行
tools/sentinel_board/harness.cjs — 33 行
src/features/sentinel/components/SentinelTaskCenter.vue — 32 行
src-tauri/src/commands/tests_web_dispatch_binding.rs — 31 行
src/styles/sentinel-base.css — 31 行
src-tauri/src/commands/tests_frontend_recon.rs — 30 行
docs/NATIVE_AGENT_EXECUTOR_TASK.md — 29 行
src-tauri/src/commands/scan_quiescence.rs — 29 行
tools/agent_dialog_harness.cjs — 29 行
src-tauri/src/commands/tests_native_scan_branches/branch_status.rs — 28 行
src-tauri/src/commands/tests_web_dispatch_closure.rs — 27 行
src/styles/sentinel-results.css — 27 行
src-tauri/src/commands/tests_neutral_runtime.rs — 26 行
src/features/assets/components/AssetWorkspace.vue — 25 行
src/features/configuration/components/ConfigDialog.vue — 25 行
tools/test_builtin_plugins.cjs — 24 行
src-tauri/src/commands/scan_startup.rs — 23 行
src-tauri/src/commands/tests_asset_draft.rs — 22 行
src-tauri/src/commands/tests_runner_log.rs — 22 行
src-tauri/src/commands/tests_web_dispatch_recovery.rs — 22 行
src-tauri/src/commands/workspace_projects.rs — 22 行
tools/test_execution_history.cjs — 22 行
tools/test_scan_deletion.cjs — 22 行
tools/test_sentinel_refresh.cjs — 22 行
docs/integration/module-moves-2026-10-14.json — 21 行
src-tauri/src/commands/agent_tests_authorization_control.rs — 21 行
src-tauri/src/commands/agent_tests_gap_followup.rs — 21 行
src-tauri/src/commands/rule_packs.rs — 21 行
src-tauri/src/commands/tests_runtime.rs — 21 行
src-tauri/src/commands/tests_workbench_startup.rs — 21 行
src/features/sentinel/presentation.ts — 21 行
src/features/sentinel/types.ts — 21 行
src/features/sentinel/components/SentinelValidationWorkbench.vue — 20 行
tools/test_retire_nest_data.py — 20 行
docs/ARCHIVED_RELEASE_NOTES_2026-08-24.md — 19 行
docs/ASSET_EXPOSURE_GUIDE_2026-24_GAP_ANALYSIS_2026-10-10.md — 19 行
docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_BY_QWEN_2026-10-10.md — 19 行
src-tauri/src/commands/agent_tests_persistence.rs — 19 行
src-tauri/src/scanning/model.rs — 19 行
src-tauri/src/commands/tests_web_administrative_closure.rs — 18 行
src-tauri/src/artifact_import/tests_import_idempotence.rs — 17 行
src-tauri/src/commands/agent_backend.rs — 17 行
src-tauri/src/commands/assets.rs — 17 行
src-tauri/src/commands/tests_overview_provenance.rs — 17 行
src-tauri/src/commands/tests_source_known_pause_negative.rs — 17 行
src-tauri/src/commands/tests_source_pause_branch_negative.rs — 17 行
src-tauri/src/commands/tests_source_pause_closed_negative.rs — 17 行
src-tauri/src/commands/tests_web_draft.rs — 17 行
src-tauri/src/commands/tests_workbench_admission.rs — 17 行
docs/MODULARIZATION_AUDIT_2026-10-13.md — 16 行
src-tauri/src/commands/agent_tests_scan_deletion_paid_archive_faults.rs — 16 行
src-tauri/src/commands/scan_execution_frontend.rs — 16 行
src-tauri/src/commands/tests_source_broker_scope.rs — 16 行
src/features/sentinel/components/results/SentinelVulnerabilitiesPane.vue — 16 行
tools/test_historical_json_import.cjs — 16 行
src-tauri/src/commands/agent_tests_backend_residual.rs — 15 行
src-tauri/src/db_initialize_budgets.rs — 15 行
src/features/sentinel/components/overview/SentinelTaskOverview.vue — 15 行
docs/NEST_HUMAN_RECOVERY_IMPLEMENTATION_2026-10-08.md — 14 行
src-tauri/src/commands/native_protection_recovery_draft.rs — 14 行
src-tauri/src/commands/scan_execution.rs — 14 行
src-tauri/src/commands/tests_historical_import_views.rs — 14 行
src-tauri/src/commands/web_dispatch_binding.rs — 14 行
src/App.vue — 14 行
docs/NEST_HISTORICAL_LABEL_NEUTRALIZATION_AUDIT_2026-09-30.md — 13 行
src-tauri/src/commands/agent_gap_followup.rs — 13 行
src-tauri/src/commands/native_scan_branches.rs — 13 行
src-tauri/src/commands/tests_native_routing.rs — 13 行
src-tauri/src/commands/tests_source_reviewer.rs — 13 行
src-tauri/src/db_settings_tests.rs — 13 行
src/features/sentinel/fuse/useFuseDetails.ts — 13 行
src/features/sentinel/results/useApiResults.ts — 13 行
tools/agent_workbench_harness.cjs — 13 行
tools/test_project_deletion.cjs — 13 行
docs/NEST_RESULT_SYNC_ISOLATION_AUDIT.md — 12 行
src-tauri/src/commands/agent_contract.rs — 12 行
src-tauri/src/commands/agent_terminal_legacy_writer.rs — 12 行
src-tauri/src/commands/agent_tests_backend_literal.rs — 12 行
src-tauri/src/commands/agent_tests_scan_deletion_multi_archive_negative.rs — 12 行
src-tauri/src/commands/agent_tests_scan_deletion_single.rs — 12 行
src-tauri/src/commands/agent_tests_surface_persistence.rs — 12 行
src-tauri/src/commands/runtime_config.rs — 12 行
src-tauri/src/commands/scan_deletion.rs — 12 行
src-tauri/src/commands/tests_source_findings.rs — 12 行
src-tauri/src/native_pipeline/analyzer.rs — 12 行
src-tauri/src/native_pipeline/tests_native_code.rs — 12 行
src-tauri/src/tool_supply/selection/tests.rs — 12 行
src-tauri/src/worker.rs — 12 行
src-tauri/tests/fixtures/legacy_strix/residual_syntax.json — 12 行
src/features/sentinel/components/AgentDialog.vue — 12 行
tools/test_module_navigation.cjs — 12 行
src-tauri/src/agent_runtime/tests.rs — 11 行
src-tauri/src/artifact_import/service.rs — 11 行
src-tauri/src/auth_session.rs — 11 行
src-tauri/src/commands/agent_tests_review_recovery.rs — 11 行
src-tauri/src/commands/code_analysis.rs — 11 行
src-tauri/src/commands/knowledge_learning_traces.rs — 11 行
src-tauri/src/commands/tests_results.rs — 11 行
src-tauri/src/commands/tests_source_pause_recovery_negative.rs — 11 行
src-tauri/src/commands/web_dispatch_closure.rs — 11 行
src-tauri/src/db_migrate.rs — 11 行
src-tauri/src/native_pipeline/tests_native_code_results.rs — 11 行
src/features/sentinel/components/SentinelRepeater.vue — 11 行
src/features/sentinel/components/overview/SentinelOverviewSummary.vue — 11 行
src/features/sentinel/components/results/SentinelOpportunitiesPane.vue — 11 行
src/features/sentinel/execution/useTaskExecutionDetails.ts — 11 行
src/features/sentinel/workbench/useWorkbenchTaskCreation.ts — 11 行
src/styles.css — 11 行
tools/test_plugin_workspaces.cjs — 11 行
src-tauri/src/commands/agent_tests_protection_recovery_draft.rs — 10 行
src-tauri/src/commands/agent_tests_tool_selection_deleted_archive.rs — 10 行
src-tauri/src/commands/tests_code_rules.rs — 10 行
src-tauri/src/commands/tests_web_closure_handoff.rs — 10 行
src-tauri/src/db_neutral_tests.rs — 10 行
src-tauri/src/native_pipeline/tests_analyzer_regressions.rs — 10 行
src/features/sentinel/composables/useAgentDialogTasks.ts — 10 行
src/styles/sentinel-workbench.css — 10 行
tools/test_execution_presentation.cjs — 10 行
tools/test_source_findings.cjs — 10 行
docs/NEST_HISTORICAL_SOURCE_BRANCH_RETIREMENT_AUDIT_2026-09-30.md — 9 行
docs/NEST_HOST_ASSESSMENT_AND_AGENT_STRATEGY_2026-09-27.md — 9 行
docs/NEST_PRODUCT_REDESIGN_AND_LEARNING_CONTRACT_2026-09-25.md — 9 行
src-tauri/resources/skills/business_frontend_deep_analysis.md — 9 行
src-tauri/src/agent_runtime/deleted_scan_audit/scope.rs — 9 行
src-tauri/src/commands/agent_tests_gap_submission.rs — 9 行
src-tauri/src/commands/agent_tests_policy.rs — 9 行
src-tauri/src/commands/agent_tests_scan_deletion_retention.rs — 9 行
src-tauri/src/commands/asset_export.rs — 9 行
src-tauri/src/commands/multi_agent/tests/broker.rs — 9 行
src-tauri/src/commands/scan_execution_runtime.rs — 9 行
src-tauri/src/commands/tests_agent_dialog_selection.rs — 9 行
src-tauri/src/commands/tests_native_branch_dispatch.rs — 9 行
src-tauri/src/commands/tests_source_result_original_reads.rs — 9 行
src-tauri/src/commands/tests_workbench_retry.rs — 9 行
src-tauri/src/commands/web_dispatch_recovery.rs — 9 行
src-tauri/src/tool_supply/selection.rs — 9 行
src/asset-workspace-patch.css — 9 行
src/features/conversations/useConversationPlugins.ts — 9 行
src/features/sentinel/components/SentinelFuseZone.vue — 9 行
src/features/sentinel/results/useScanPages.ts — 9 行
tools/execution_history/harness.cjs — 9 行
tools/retire_nest_data.py — 9 行
docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md — 8 行
docs/NEST_ASSET_EXPOSURE_QODER_PLAN_2026-10-07.md — 8 行
docs/NEST_BACKEND_RETIREMENT_AUDIT_2026-09-26.md — 8 行
docs/NEST_JSON_SNAPSHOT_IMPORT_AUDIT.md — 8 行
docs/NEST_OVERVIEW_SOURCE_REVIEW_AUDIT.md — 8 行
docs/NEST_ROOT_HUMAN_CHAT_PROJECTION_AUDIT_2026-10-05.md — 8 行
docs/asset-accuracy-20261015/final-verification.json — 8 行
src-tauri/src/commands/agent_tests_scan_deletion_paid_archive.rs — 8 行
src-tauri/src/commands/agent_tests_terminal_legacy_atomic.rs — 8 行
src-tauri/src/commands/native_process_log_replay_tests.rs — 8 行
src-tauri/src/commands/native_protection_recovery_contract.rs — 8 行
src-tauri/src/commands/result_ingestion.rs — 8 行
src-tauri/src/commands/result_ingestion_recon.rs — 8 行
src-tauri/src/commands/tests_source_pause_branch_original.rs — 8 行
src-tauri/src/commands/web_administrative_closure.rs — 8 行
src-tauri/src/native_pipeline/tests_native_ci.rs — 8 行
src-tauri/src/scanning.rs — 8 行
src-tauri/src/worker_resources.rs — 8 行
src/features/sentinel/components/AgentWorkbench.vue — 8 行
src/features/sentinel/composables/useAgentDialogDirectives.ts — 8 行
tools/sentinel_api_harness.cjs — 8 行
tools/test_agent_workbench.cjs — 8 行
tools/test_native_budget_diagnostics.cjs — 8 行
tools/test_native_root_continuation_issuance.cjs — 8 行
src-tauri/src/agent_runtime/tests_backend_retirement.rs — 7 行
src-tauri/src/agent_runtime/web_mode/draft.rs — 7 行
src-tauri/src/artifact_import/adapters/sentinel_bundle.rs — 7 行
src-tauri/src/commands/agent_acceptance.rs — 7 行
src-tauri/src/commands/agent_tests_backend_retirement.rs — 7 行
src-tauri/src/commands/agent_tests_request_accounting.rs — 7 行
src-tauri/src/commands/agent_tests_single_model_inflight.rs — 7 行
src-tauri/src/commands/investigation_tests.rs — 7 行
src-tauri/src/commands/native_source_scan.rs — 7 行
src-tauri/src/commands/tests_source_failed_deletion_negative.rs — 7 行
src-tauri/src/commands/tests_source_paid_deletion_original_negative.rs — 7 行
src-tauri/src/commands/tests_source_pause_closed_root.rs — 7 行
src-tauri/src/commands/tests_source_quiescence_original.rs — 7 行
src-tauri/src/commands/tests_source_runtime.rs — 7 行
src-tauri/src/commands/web_closure_handoff.rs — 7 行
src/features/sentinel/components/overview/SentinelTokenOverview.vue — 7 行
src/features/sentinel/workbench/useBrowserAuthSessions.ts — 7 行
src/plugins/runtime.ts — 7 行
tools/test_agent_dialog_tasks.cjs — 7 行
tools/test_execution_details.cjs — 7 行
tools/test_native_root_continuation_discovery.cjs — 7 行
docs/EXTERNAL_ASSET_ACCURACY_AND_PROJECT_ANALYSIS_DESIGN_2026-10-15.md — 6 行
docs/NEST_CHAT_ASYNC_AUDIT_2026-09-26.md — 6 行
docs/NEST_NATIVE_SDK_STAGE_LOG_AUDIT_2026-10-03.md — 6 行
docs/asset-accuracy-20261015/coverage-final-34e802a-01.json — 6 行
docs/asset-accuracy-20261015/privacy-final.json — 6 行
docs/asset-accuracy-20261015/screenshots-01-INDEX.md — 6 行
src-tauri/src/artifact_import/adapters/mod.rs — 6 行
src-tauri/src/commands/agent_dialog_selection.rs — 6 行
src-tauri/src/commands/agent_native/persistence.rs — 6 行
src-tauri/src/commands/agent_tests_backend_residual_runtime.rs — 6 行
src-tauri/src/commands/agent_tests_browser.rs — 6 行
src-tauri/src/commands/agent_tests_gap_review.rs — 6 行
src-tauri/src/commands/agent_tests_http_journal.rs — 6 行
src-tauri/src/commands/agent_tests_protection_recovery_cancel.rs — 6 行
src-tauri/src/commands/agent_tests_review_atomic.rs — 6 行
src-tauri/src/commands/agent_tests_seven_terminal_original.rs — 6 行
src-tauri/src/commands/historical_import_views.rs — 6 行
src-tauri/src/commands/knowledge_learning_candidates.rs — 6 行
src-tauri/src/commands/native_financial_continuation_failure_writer.rs — 6 行
src-tauri/src/commands/native_helper_live_tests.rs — 6 行
src-tauri/src/commands/native_helper_log_owner.rs — 6 行
src-tauri/src/commands/native_scan_branches/status.rs — 6 行
src-tauri/src/commands/native_source_ci_policy.rs — 6 行
src-tauri/src/commands/native_source_pause_branch.rs — 6 行
src-tauri/src/commands/scan_execution_routing.rs — 6 行
src-tauri/src/commands/tests_historical_source_branch.rs — 6 行
src-tauri/src/commands/tests_native_source_regressions.rs — 6 行
src-tauri/src/commands/tests_retired_deletion.rs — 6 行
src-tauri/src/commands/tests_source_branch_terminal_original.rs — 6 行
src-tauri/src/commands/tests_source_scope_contract.rs — 6 行
src-tauri/src/commands/tests_web_resume_finance.rs — 6 行
src-tauri/src/commands/web_mode_test_support.rs — 6 行
src-tauri/src/commands/workbench_retry.rs — 6 行
src-tauri/src/db_settings_startup_tests.rs — 6 行
src-tauri/src/native_pipeline/process_log/write_guard_tests.rs — 6 行
src-tauri/src/native_pipeline/process_observer_tests.rs — 6 行
src/features/sentinel/components/NativeRunStatus.vue — 6 行
src/features/sentinel/workbench/WorkbenchOverview.vue — 6 行
src/features/sentinel/workbench/useClosureHandoff.ts — 6 行
src/plugins/catalog.ts — 6 行
src/plugins/types.ts — 6 行
src/sentinel.css — 6 行
tools/execution_log_events.cjs — 6 行
tools/plugin_workspace_harness.cjs — 6 行
tools/test_agent_dialog_events.cjs — 6 行
tools/test_conversation_plugins.cjs — 6 行
tools/test_native_root_reconciliation.cjs — 6 行
tools/test_source_results.cjs — 6 行
tools/test_workbench_tool_selection.cjs — 6 行
docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md — 5 行
docs/NEST_ASSET_TAKEOVER_2026-10-08.md — 5 行
docs/NEST_ENVIRONMENT_RETIREMENT_2026-10-08.md — 5 行
docs/NEST_NO_FINDING_AND_MISSING_MATERIAL_AUDIT_2026-10-04.md — 5 行
docs/NEST_ROOT_HUMAN_OBLIGATION_CHAT_AUDIT_2026-10-05.md — 5 行
docs/NEST_SOURCE_RESULT_EXPORT_AUDIT.md — 5 行
docs/NEST_SOURCE_REVIEW_EXECUTION_AUDIT.md — 5 行
docs/NEST_WEB_DRAFT_HANDOFF_AUDIT_2026-09-26.md — 5 行
docs/NEST_WEB_STARTUP_ATOMICITY_AUDIT_2026-09-26.md — 5 行
src-tauri/src/commands/agent_terminal_legacy_fuse.rs — 5 行
src-tauri/src/commands/agent_tests_directive_routing.rs — 5 行
src-tauri/src/commands/agent_tests_e2e.rs — 5 行
src-tauri/src/commands/agent_tests_frozen_tool_supply_birth.rs — 5 行
src-tauri/src/commands/agent_tests_historical_receipt.rs — 5 行
src-tauri/src/commands/agent_tests_invocation.rs — 5 行
src-tauri/src/commands/agent_tests_protection_recovery_approval.rs — 5 行
src-tauri/src/commands/agent_tests_root_financial_continuation.rs — 5 行
src-tauri/src/commands/agent_tests_scan_deletion_paid_archive_unknown.rs — 5 行
src-tauri/src/commands/agent_tests_single_continuation_preview.rs — 5 行
src-tauri/src/commands/agent_tests_validation.rs — 5 行
src-tauri/src/commands/asset_response_groups_tests.rs — 5 行
src-tauri/src/commands/multi_agent/tests/budget_target.rs — 5 行
src-tauri/src/commands/multi_agent/tests/findings.rs — 5 行
src-tauri/src/commands/native_financial_continuation_approval.rs — 5 行
src-tauri/src/commands/native_financial_continuation_authorization.rs — 5 行
src-tauri/src/commands/native_financial_continuation_preview.rs — 5 行
src-tauri/src/commands/native_frontend_recon.rs — 5 行
src-tauri/src/commands/native_protection_recovery_cancel.rs — 5 行
src-tauri/src/commands/native_source_analysis.rs — 5 行
src-tauri/src/commands/result_ingestion_runs.rs — 5 行
src-tauri/src/commands/tests_historical_trace.rs — 5 行
src-tauri/src/commands/tests_native_scan_branches/history_directives.rs — 5 行
src-tauri/src/commands/tests_settings_preservation.rs — 5 行
src-tauri/src/commands/tests_source_coordinator.rs — 5 行
src-tauri/src/commands/tests_source_known_pause.rs — 5 行
src-tauri/src/commands/tests_source_pause_recovery.rs — 5 行
src/features/sentinel/components/AuthorizationControlForm.vue — 5 行
src/features/sentinel/components/SourceReviewEvidence.vue — 5 行
src/features/sentinel/components/lazyPanels.ts — 5 行
src/features/sentinel/fuse/useFuseDisposition.ts — 5 行
src/features/sentinel/styles/coverage-diagnostics.css — 5 行
src/shell/moduleCatalog.ts — 5 行
src/styles/sentinel-layout.css — 5 行
src/styles/sentinel-trace.css — 5 行
tools/sentinel_board/overview_summary.cjs — 5 行
tools/test_agent_dialog_status.cjs — 5 行
tools/test_source_pause_recovery_ui.cjs — 5 行
tools/web_helper_frontend_harness.cjs — 5 行
docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md — 4 行
docs/FRONTEND_MODULE_OWNERSHIP_2026-10-14.md — 4 行
docs/NEST_ASSET_DRAFT_ATOMICITY_AUDIT_2026-09-26.md — 4 行
docs/NEST_DESIGN_AUDIT_2026-10-13.md — 4 行
docs/NEST_EXECUTION_OVERVIEW_UI_AUDIT_2026-10-03.md — 4 行
docs/NEST_EXECUTION_UI_ASSET_LEARNING_BLUEPRINT_2026-09-25.md — 4 行
docs/NEST_EXTERNAL_SURFACE_ENTRY_AUDIT_2026-09-26.md — 4 行
docs/NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md — 4 行
docs/NEST_ORDERED_HUMAN_EXECUTION_AUDIT_2026-10-03.md — 4 行
docs/NEST_PAUSE_QUIESCENCE_AUDIT.md — 4 行
docs/NEST_SCAN_DELETION_AUDIT.md — 4 行
docs/NEST_SINGLE_IDENTITY_LIVE_AUTHORITY_AUDIT_2026-10-04.md — 4 行
docs/NEST_SOURCE_ATOMIC_BUNDLE_AUDIT.md — 4 行
docs/NEST_SOURCE_DECISION_CONSUMER_AUDIT.md — 4 行
docs/NEST_SOURCE_JOINT_IMPORT_AUDIT.md — 4 行
docs/NEST_SOURCE_RESULT_ORIGINAL_AUDIT_2026-10-04.md — 4 行
docs/RUST_NATIVE_MIGRATION.md — 4 行
src-tauri/src/agent_runtime/model/diagnostics/contract_tests/reads.rs — 4 行
src-tauri/src/agent_runtime/multi_agent/attempts/expiry_proof.rs — 4 行
src-tauri/src/agent_runtime/multi_agent/budget/root_definition/guard.rs — 4 行
src-tauri/src/agent_runtime/multi_agent/homogeneous/decompose.rs — 4 行
src-tauri/src/agent_runtime/multi_agent/source.rs — 4 行
src-tauri/src/agent_runtime/stage1a_tests/evidence_review.rs — 4 行
src-tauri/src/agent_runtime/web_mode/writer.rs — 4 行
src-tauri/src/artifact_import/reconcile.rs — 4 行
src-tauri/src/artifact_import/tests_import_record_limits.rs — 4 行
src-tauri/src/assets/model.rs — 4 行
src-tauri/src/commands/agent_authorization_control.rs — 4 行
src-tauri/src/commands/agent_gap_submission.rs — 4 行
src-tauri/src/commands/agent_native/web_executor_adapter.rs — 4 行
src-tauri/src/commands/agent_tests_authorization_output.rs — 4 行
src-tauri/src/commands/agent_tests_directive_concurrency.rs — 4 行
src-tauri/src/commands/agent_tests_fresh_single_production.rs — 4 行
src-tauri/src/commands/agent_tests_mapper_loop_physical_delete.rs — 4 行
src-tauri/src/commands/agent_tests_single_budget_review.rs — 4 行
src-tauri/src/commands/agent_tests_single_projection_consumers.rs — 4 行
src-tauri/src/commands/agent_tests_tool_selection_policy_source.rs — 4 行
src-tauri/src/commands/asset_response_groups.rs — 4 行
src-tauri/src/commands/exposure/tests_g3.rs — 4 行
src-tauri/src/commands/frontend_recon_artifacts.rs — 4 行
src-tauri/src/commands/multi_agent/findings.rs — 4 行
src-tauri/src/commands/multi_agent/tests/scheduler.rs — 4 行
src-tauri/src/commands/native_helpers.rs — 4 行
src-tauri/src/commands/native_scan_branches/history.rs — 4 行
src-tauri/src/commands/native_source_completion.rs — 4 行
src-tauri/src/commands/native_source_runtime.rs — 4 行
src-tauri/src/commands/tests_recon_diagnostics.rs — 4 行
src-tauri/src/commands/tests_source_branch_unknown_original.rs — 4 行
src-tauri/src/commands/tests_source_ci_policy.rs — 4 行
src-tauri/src/commands/tests_source_financial_exit_owned_stop.rs — 4 行
src-tauri/src/commands/tests_source_paid_deletion_original.rs — 4 行
src-tauri/src/commands/tests_source_specialists.rs — 4 行
src-tauri/src/conversations/tests.rs — 4 行
src-tauri/src/db.rs — 4 行
src-tauri/src/llm_hook_tests.rs — 4 行
src-tauri/src/native_pipeline/process_observer_source_tests.rs — 4 行
src-tauri/src/native_pipeline/tests_native_greybox.rs — 4 行
src-tauri/src/tool_supply/approval/tests.rs — 4 行
src/features/assets/types.ts — 4 行
src/features/conversations/ConversationPluginPanel.vue — 4 行
src/features/runtime/components/WorkerSettingsPanel.vue — 4 行
src/features/sentinel/components/InvestigationGraphPanel.vue — 4 行
src/features/sentinel/components/results/SentinelFingerprintPane.vue — 4 行
src/features/sentinel/components/results/SentinelRuntimeExploration.vue — 4 行
src/features/sentinel/composables/useAgentDialogStatus.ts — 4 行
src/features/sentinel/execution/useRunnerLogPanel.ts — 4 行
src/features/sentinel/fuse/presentation.ts — 4 行
src/features/sentinel/rulePacksApi.ts — 4 行
src/features/sentinel/styles/runner-console.css — 4 行
src/features/sentinel/styles/workbench-layout.css — 4 行
tools/agent_dialog/status_lifecycle.cjs — 4 行
tools/execution_history/search_contract.cjs — 4 行
tools/native_process_log_harness.cjs — 4 行
tools/native_sdk_log_harness.cjs — 4 行
tools/qa/chat-ui/base-fixture.cjs — 4 行
tools/sentinel_board/token_usage.cjs — 4 行
tools/test_native_root_continuation_preview.cjs — 4 行
tools/test_native_sdk_log_api.cjs — 4 行
tools/test_priced_recovery_contract.cjs — 4 行
tools/test_protection_recovery_ui.cjs — 4 行
tools/test_rust_business_boundaries.cjs — 4 行
tools/test_skills_catalog_ownership.cjs — 4 行
README.md — 3 行
docs/BUSINESS_MODULE_BOUNDARIES_2026-10-15.md — 3 行
docs/BUSINESS_MODULE_INTEGRATION_2026-10-14.md — 3 行
docs/NEST_A03_AUTHORIZATION_OUTPUT_IMPLEMENTATION_2026-10-09.md — 3 行
docs/NEST_A03_BUSINESS_LOGIC_OUTPUT_GAP_2026-10-09.md — 3 行
docs/NEST_ASSET_EXPOSURE_QODER_FRAMEWORK_2026-10-08.md — 3 行
docs/NEST_DIRECTIVE_QUEUE_ACTION_AUDIT_2026-09-26.md — 3 行
docs/NEST_FINDINGS_READ_BOUNDARY_AUDIT.md — 3 行
docs/NEST_HOMOGENEOUS_CONTINUATION_HANDOFF_2026-10-12.md — 3 行
docs/NEST_HOMOGENEOUS_COORDINATOR_ARCHITECTURE_2026-10-10.md — 3 行
docs/NEST_HOMOGENEOUS_COORDINATOR_DESIGN_2026-10-10.md — 3 行
docs/NEST_MASTER_REGRESSION_GATE_2026-10-08.md — 3 行
docs/NEST_MASTER_RESUME_AUDIT_2026-10-07.md — 3 行
docs/NEST_ORDERED_HUMAN_PLAN_AUDIT_2026-10-03.md — 3 行
docs/NEST_PAID_SINGLE_ARCHIVE_AUDIT_2026-10-04.md — 3 行
docs/NEST_REAL_DATA_RETIREMENT_AUDIT_2026-10-02.md — 3 行
docs/NEST_ROOT_LOCAL_TOOL_DISPLAY_AUDIT_2026-10-03.md — 3 行
docs/NEST_SOURCE_EXECUTION_HISTORY_AUDIT.md — 3 行
docs/NEST_SOURCE_SARIF_ROUNDTRIP_AUDIT.md — 3 行
docs/NEST_TRACE_HUB_THEME_AUDIT_2026-10-03.md — 3 行
docs/NEST_WEB_MANUAL_RECOVERY_AUDIT_2026-09-26.md — 3 行
docs/NEST_WEB_UNDISPATCHED_CLOSURE_AUDIT_2026-09-26.md — 3 行
src-tauri/src/agent_runtime/deleted_scan_audit/writer.rs — 3 行
src-tauri/src/agent_runtime/model/diagnostics/contract_tests/writes.rs — 3 行
src-tauri/src/agent_runtime/multi_agent/budget/clock/pause_authority.rs — 3 行
src-tauri/src/agent_runtime/multi_agent/directive/human_review/scope.rs — 3 行
src-tauri/src/artifact_import/projection.rs — 3 行
src-tauri/src/artifact_import/tests_sarif_retirement.rs — 3 行
src-tauri/src/commands/agent_gap_review.rs — 3 行
src-tauri/src/commands/agent_native/coordinator_lease_reviewer.rs — 3 行
src-tauri/src/commands/agent_native/queue.rs — 3 行
src-tauri/src/commands/agent_tests_external_surface_output_interruptions.rs — 3 行
src-tauri/src/commands/agent_tests_external_surface_response.rs — 3 行
src-tauri/src/commands/agent_tests_financial_continuation_approval_guards.rs — 3 行
src-tauri/src/commands/agent_tests_financial_continuation_issuance.rs — 3 行
src-tauri/src/commands/agent_tests_missing_side_original.rs — 3 行
src-tauri/src/commands/agent_tests_multi_terminal_owned.rs — 3 行
src-tauri/src/commands/agent_tests_native_policy_original.rs — 3 行
src-tauri/src/commands/agent_tests_ordered_execution_lifecycle.rs — 3 行
src-tauri/src/commands/agent_tests_request_review.rs — 3 行
src-tauri/src/commands/agent_tests_review_attempt_closure.rs — 3 行
src-tauri/src/commands/agent_tests_scan_deletion_multi_archive.rs — 3 行
src-tauri/src/commands/agent_tests_scan_deletion_paid_archive_crash.rs — 3 行
src-tauri/src/commands/agent_tests_single_finally_projection.rs — 3 行
src-tauri/src/commands/agent_tests_specialist_journal.rs — 3 行
src-tauri/src/commands/environment.rs — 3 行
src-tauri/src/commands/frontend_recon.rs — 3 行
src-tauri/src/commands/historical_agent_trace.rs — 3 行
src-tauri/src/commands/investigation_surface.rs — 3 行
src-tauri/src/commands/multi_agent/tests/review_gate.rs — 3 行
src-tauri/src/commands/native_financial_continuation_discovery.rs — 3 行
src-tauri/src/commands/native_financial_continuation_status.rs — 3 行
src-tauri/src/commands/native_helper_live_fixture.rs — 3 行
src-tauri/src/commands/native_process_log_replay.rs — 3 行
src-tauri/src/commands/native_scan_branches/status_budget.rs — 3 行
src-tauri/src/commands/native_single_continuation_model_contract.rs — 3 行
src-tauri/src/commands/native_source_branch_completion.rs — 3 行
src-tauri/src/commands/native_source_coordinator.rs — 3 行
src-tauri/src/commands/native_source_findings.rs — 3 行
src-tauri/src/commands/native_source_pause_closure.rs — 3 行
src-tauri/src/commands/native_source_scope.rs — 3 行
src-tauri/src/commands/native_web_mode_binding.rs — 3 行
src-tauri/src/commands/retired_data_guards.rs — 3 行
src-tauri/src/commands/src_adapter_authority_tests.rs — 3 行
src-tauri/src/commands/src_assurance.rs — 3 行
src-tauri/src/commands/tests_agent_dialog_view.rs — 3 行
src-tauri/src/commands/tests_native_invocation.rs — 3 行
src-tauri/src/commands/tests_native_scan_branches/timeline.rs — 3 行
src-tauri/src/commands/tests_retired_import_commands.rs — 3 行
src-tauri/src/commands/tests_source_broker_historical_negatives.rs — 3 行
src-tauri/src/commands/tests_source_broker_original_candidates.rs — 3 行
src-tauri/src/commands/tests_source_coverage_reviewer.rs — 3 行
src-tauri/src/commands/tests_source_failed_deletion_original.rs — 3 行
src-tauri/src/commands/tests_source_financial_exit_cutover.rs — 3 行
src-tauri/src/commands/tests_source_result_original_retry.rs — 3 行
src-tauri/src/commands/tests_web_mode_creation.rs — 3 行
src-tauri/src/conversations/tools.rs — 3 行
src-tauri/src/db_tool_supply_lease_tests.rs — 3 行
src-tauri/src/native_pipeline/process_log/persist.rs — 3 行
src-tauri/src/workspaces/model.rs — 3 行
src-tauri/tests/fixtures/legacy_strix/bundles/producer_1_5_3/findings.sarif — 3 行
src-tauri/tests/fixtures/page_isolation/legacy_525_schema.sql — 3 行
src/api.ts — 3 行
src/features/sentinel/components/AgentRequestReview.vue — 3 行
src/features/sentinel/composables/useAgentDialogReading.ts — 3 行
src/features/sentinel/results/useHistoricalImportLedger.ts — 3 行
src/features/sentinel/traces/useTraceSelection.ts — 3 行
src/features/sentinel/workbench/useFollowupRecovery.ts — 3 行
src/features/workspaces/types.ts — 3 行
tools/agent_trace_harness.cjs — 3 行
tools/native_status/harness.cjs — 3 行
tools/qa/chat-ui/fixture.cjs — 3 行
tools/sentinel_board/investigation_summary.cjs — 3 行
tools/test_agent_dialog_receipts.cjs — 3 行
tools/test_agent_dialog_timeline.cjs — 3 行
tools/test_priced_recovery_ui.cjs — 3 行
tools/test_sentinel_pagination.cjs — 3 行
tools/test_vulnerability_results.cjs — 3 行
.gitignore — 2 行
docs/ASSET_EXPOSURE_STATUS_SUMMARY_2026-10-10.md — 2 行
docs/EXPOSURE_SURFACE_DEVELOPMENT_PLAN.md — 2 行
docs/NEST_ANONYMOUS_CONTROL_ORIGINAL_ENTRY_AUDIT_2026-10-04.md — 2 行
docs/NEST_ARCHITECTURE_ANALYSIS_2026-10-10.md — 2 行
docs/NEST_ATTEMPT_SURFACE_CHECKPOINT_AUDIT_2026-09-30.md — 2 行
docs/NEST_GAP_REVIEW_CLOSURE_AUDIT_2026-09-26.md — 2 行
docs/NEST_LOCAL_RECEIPT_RECONCILIATION_AUDIT_2026-09-26.md — 2 行
docs/NEST_MAPPER_GENERAL_REACT_CONTRACT_2026-10-09.md — 2 行
docs/NEST_MASTER_CAPACITY_CONCURRENCY_2026-10-08.md — 2 行
docs/NEST_MASTER_RECOVERY_SHARED_FINANCE_DESIGN_2026-10-08.md — 2 行
docs/NEST_MASTER_SINGLE_HISTORY_VERIFICATION_2026-10-09.md — 2 行
docs/NEST_MISSING_SIDE_ORIGINAL_AUDIT_2026-10-04.md — 2 行
docs/NEST_NATIVE_HTTP_STOP_RECONCILIATION_AUDIT.md — 2 行
docs/NEST_ORDERED_UNSENT_CLOSURE_AUDIT_2026-10-03.md — 2 行
docs/NEST_REM_A02_FINANCIAL_CONTINUATION_CONTRACT_2026-10-07.md — 2 行
docs/NEST_REQUEST_OPERATOR_REVIEW_AUDIT.md — 2 行
docs/NEST_RESULT_SURFACE_RETIREMENT_AUDIT_2026-10-01.md — 2 行
docs/NEST_ROOT_BOOTSTRAP_SDK_RECOVERY_AUDIT_2026-10-03.md — 2 行
docs/NEST_SCHEDULER_AUTHORITY_AUDIT_2026-10-01.md — 2 行
docs/NEST_SNAPSHOT_IO_AUDIT.md — 2 行
docs/NEST_SOURCE_ANALYSIS_VIEW_AUDIT.md — 2 行
docs/NEST_SOURCE_KNOWN_PAUSE_ORIGINAL_AUDIT_2026-10-05.md — 2 行
docs/NEST_SOURCE_PARTIAL_TOOL_REENTRY_AUDIT_2026-09-28.md — 2 行
docs/NEST_SOURCE_PAUSE_CLOSED_ROOT_AUDIT_2026-10-05.md — 2 行
docs/NEST_WEB_HELPER_LIVE_LOG_AUDIT_2026-10-03.md — 2 行
docs/NEST_WEB_TOOLCHAIN_BINDING_AUDIT_2026-09-26.md — 2 行
docs/NEST_WORKER_NAMESPACE_AUDIT_2026-10-02.md — 2 行
src-tauri/src/agent_runtime/model/diagnostics/replay.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/budget/clock/closure_writer.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/confirmation/preview.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/directive/confirmation.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/directive/reconciliation.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/lease.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/mailbox.rs — 2 行
src-tauri/src/agent_runtime/multi_agent/mailbox/send.rs — 2 行
src-tauri/src/agent_runtime/single_projection.rs — 2 行
src-tauri/src/agent_runtime/target_requests.rs — 2 行
src-tauri/src/artifact_import/adapters/sentinel_stages.rs — 2 行
src-tauri/src/artifact_import/mod.rs — 2 行
src-tauri/src/artifact_import/tests_import_authority.rs — 2 行
src-tauri/src/artifact_import/tests_import_source_reports.rs — 2 行
src-tauri/src/artifact_import/tests_import_trace.rs — 2 行
src-tauri/src/assets/statistics_tests.rs — 2 行
src-tauri/src/commands/agent_backend_storage.rs — 2 行
src-tauri/src/commands/agent_dialog_view.rs — 2 行
src-tauri/src/commands/agent_directive_proposals.rs — 2 行
src-tauri/src/commands/agent_native/coordinator_identity_frame.rs — 2 行
src-tauri/src/commands/agent_native/coordinator_lease_closed.rs — 2 行
src-tauri/src/commands/agent_native/single_budget.rs — 2 行
src-tauri/src/commands/agent_original_terminal_identity.rs — 2 行
src-tauri/src/commands/agent_terminal_legacy_consumer.rs — 2 行
src-tauri/src/commands/agent_tests_authorization_review_delivery.rs — 2 行
src-tauri/src/commands/agent_tests_authorization_transport.rs — 2 行
src-tauri/src/commands/agent_tests_bootstrap_original.rs — 2 行
src-tauri/src/commands/agent_tests_changed_fact_tick.rs — 2 行
src-tauri/src/commands/agent_tests_client_side_sdk_dispatch.rs — 2 行
src-tauri/src/commands/agent_tests_contracts.rs — 2 行
src-tauri/src/commands/agent_tests_directive_reconciliation.rs — 2 行
src-tauri/src/commands/agent_tests_external_surface.rs — 2 行
src-tauri/src/commands/agent_tests_financial_continuation_crash.rs — 2 行
src-tauri/src/commands/agent_tests_fixtures.rs — 2 行
src-tauri/src/commands/agent_tests_gap_recovery.rs — 2 行
src-tauri/src/commands/agent_tests_identity_root_fixture.rs — 2 行
src-tauri/src/commands/agent_tests_lease_reviewer_control.rs — 2 行
src-tauri/src/commands/agent_tests_multi_exit_receipt_readonly.rs — 2 行
src-tauri/src/commands/agent_tests_native_sdk_logs.rs — 2 行
src-tauri/src/commands/agent_tests_paid_proposal_feedback.rs — 2 行
src-tauri/src/commands/agent_tests_root_supervision_headroom_fixture.rs — 2 行
src-tauri/src/commands/agent_tests_scan_deletion_multi_archive_crash.rs — 2 行
src-tauri/src/commands/agent_tests_scan_deletion_multi_migration.rs — 2 行
src-tauri/src/commands/agent_tests_single_budget_target.rs — 2 行
src-tauri/src/commands/agent_tests_single_continuation_model_contract.rs — 2 行
src-tauri/src/commands/agent_tests_single_finally_entry.rs — 2 行
src-tauri/src/commands/agent_tests_specialist_recovery.rs — 2 行
src-tauri/src/commands/agent_tests_specialist_transport.rs — 2 行
src-tauri/src/commands/agent_tests_tick_authority.rs — 2 行
src-tauri/src/commands/agent_tests_web_mode_root.rs — 2 行
src-tauri/src/commands/agent_tools_http/evidence.rs — 2 行
src-tauri/src/commands/agent_tools_http/observations.rs — 2 行
src-tauri/src/commands/agent_tools_source.rs — 2 行
src-tauri/src/commands/exposure/tests_g4.rs — 2 行
src-tauri/src/commands/investigation_deep_dive.rs — 2 行
src-tauri/src/commands/investigation_persist.rs — 2 行
src-tauri/src/commands/multi_agent/tests/identity.rs — 2 行
src-tauri/src/commands/multi_agent_external_surface.rs — 2 行
src-tauri/src/commands/native_branch_dispatch.rs — 2 行
src-tauri/src/commands/native_execution_history.rs — 2 行
src-tauri/src/commands/native_financial_continuation_finish.rs — 2 行
src-tauri/src/commands/native_financial_continuation_worker.rs — 2 行
src-tauri/src/commands/native_helper_process.rs — 2 行
src-tauri/src/commands/native_protection_recovery_sources.rs — 2 行
src-tauri/src/commands/native_scan_branches/directives.rs — 2 行
src-tauri/src/commands/native_scan_branches/root_cost_reconciliation.rs — 2 行
src-tauri/src/commands/native_scan_branches/status_timeline_page.rs — 2 行
src-tauri/src/commands/native_single_continuation_preview.rs — 2 行
src-tauri/src/commands/native_source_failed_deletion.rs — 2 行
src-tauri/src/commands/native_source_pause_recovery.rs — 2 行
src-tauri/src/commands/ownership_o3_tests.rs — 2 行
src-tauri/src/commands/tests_execution_history.rs — 2 行
src-tauri/src/commands/tests_human_directive_review_faults.rs — 2 行
src-tauri/src/commands/tests_investigation.rs — 2 行
src-tauri/src/commands/tests_legacy_artifact_baseline.rs — 2 行
src-tauri/src/commands/tests_native_budget_diagnostics.rs — 2 行
src-tauri/src/commands/tests_portable_agent_import.rs — 2 行
src-tauri/src/commands/tests_source_analysis_view.rs — 2 行
src-tauri/src/commands/tests_source_coverage_reentry.rs — 2 行
src-tauri/src/commands/tests_source_exhausted_root_reentry.rs — 2 行
src-tauri/src/commands/tests_source_plan_owner.rs — 2 行
src-tauri/src/commands/tests_source_specialist_cancellation.rs — 2 行
src-tauri/src/commands/tests_source_tool_authority.rs — 2 行
src-tauri/src/commands/tests_trace_display_privacy.rs — 2 行
src-tauri/src/commands/tests_web_mode_binding.rs — 2 行
src-tauri/src/commands/tests_workbench_auth.rs — 2 行
src-tauri/src/commands/web_investigation_policy.rs — 2 行
src-tauri/src/page_isolation_migration_tests.rs — 2 行
src-tauri/src/snapshot_export.rs — 2 行
src-tauri/tests/backend_retirement_allowlist.json — 2 行
src/features/sentinel/components/AgentIdentityMaterial.vue — 2 行
src/features/sentinel/components/AgentTraceHub.vue — 2 行
src/features/sentinel/components/HistoricalJsonImport.vue — 2 行
src/features/sentinel/components/NativeBudgetDiagnostics.vue — 2 行
src/features/sentinel/components/NativeSourcePauseRecovery.vue — 2 行
src/features/sentinel/components/SentinelAuthRecoveryPanel.vue — 2 行
src/features/sentinel/components/results/SentinelEndpointsPane.vue — 2 行
src/features/sentinel/components/results/SentinelJsFiles.vue — 2 行
src/features/sentinel/composables/useAgentDialogFollowup.ts — 2 行
src/features/sentinel/composables/useAgentDialogHistory.ts — 2 行
src/features/sentinel/execution/useNativeProcessLog.ts — 2 行
src/features/sentinel/execution/useNativeSdkLog.ts — 2 行
src/features/sentinel/overview/useTokenUsage.ts — 2 行
src/features/sentinel/remoteWorkerApi.ts — 2 行
src/features/sentinel/styles/token-accounting.css — 2 行
src/shell/WorkspaceNavigation.vue — 2 行
src/types.ts — 2 行
tools/agent_dialog/human_review_transport.cjs — 2 行
tools/agent_dialog/navigation_privacy.cjs — 2 行
tools/chat_dom/transport.mjs — 2 行
tools/decision_summary_harness.cjs — 2 行
tools/qa/chat-ui/browser-qa.cjs — 2 行
tools/result_component_harness.cjs — 2 行
tools/serve_chat_dom.mjs — 2 行
tools/test_agent_identity_material.cjs — 2 行
tools/test_agent_request_review.cjs — 2 行
tools/test_api_owner_contracts.cjs — 2 行
tools/test_api_results.cjs — 2 行
tools/test_asset_workspace_review.cjs — 2 行
tools/test_module_boundaries.cjs — 2 行
tools/test_native_runtime.cjs — 2 行
tools/test_runner_log_panel.cjs — 2 行
tools/test_sentinel_search.cjs — 2 行
tools/test_workbench_panels.cjs — 2 行
docs/ASSET_EXPOSURE_GROK_G1_DELIVERY_2026-10-08.md — 1 行
docs/ASSET_EXPOSURE_GROK_G2_DELIVERY_2026-10-09.md — 1 行
docs/ASSET_EXPOSURE_GROK_G4_DELIVERY_2026-10-09.md — 1 行
docs/ASSET_EXPOSURE_GROK_IMPLEMENTATION_SPEC_2026-10-08.md — 1 行
docs/ASSET_EXPOSURE_GROK_P1_REMEDIATION_DELIVERY_2026-10-09.md — 1 行
docs/BUILTIN_PLUGIN_HOST_2026-10-13.md — 1 行
docs/EXTERNAL_ASSET_ACCURACY_2026-10-15.md — 1 行
docs/EXTERNAL_ASSET_ACCURACY_IMPLEMENTATION_2026-10-15.md — 1 行
docs/LOCAL_INCREMENTAL_CANDIDATE_HANDOFF_2026-10-15.md — 1 行
docs/LOCAL_INCREMENTAL_INTEGRATION_2026-10-15.md — 1 行
docs/NEST_A03_EXTERNAL_SURFACE_OUTPUT_GAP_2026-10-09.md — 1 行
docs/NEST_A03_TRIGGER_GAP_AUDIT_2026-10-09.md — 1 行
docs/NEST_ADMINISTRATIVE_CLOSURE_AUDIT.md — 1 行
docs/NEST_BOOTSTRAP_ORIGINAL_REFUSAL_AUDIT_2026-10-04.md — 1 行
docs/NEST_CHILD_COMPLETION_ATOMICITY_AUDIT_2026-09-26.md — 1 行
docs/NEST_CLOSURE_HANDOFF_AUDIT.md — 1 行
docs/NEST_DIRECTIVE_DELIVERY_AUDIT_2026-09-26.md — 1 行
docs/NEST_DIRECTIVE_MODULE_BOUNDARY_AUDIT_2026-09-28.md — 1 行
docs/NEST_DIRECTIVE_PROPOSAL_AUDIT_2026-09-26.md — 1 行
docs/NEST_GAP_FOLLOWUP_HANDOFF_AUDIT_2026-09-26.md — 1 行
docs/NEST_GROUP_ASSET_EXPOSURE_PRODUCT_DESIGN_2026-10-08.md — 1 行
docs/NEST_HISTORICAL_DIRECTIVE_RECEIPT_AUDIT_2026-09-28.md — 1 行
docs/NEST_HOST_BOUNDARY_DECISION_2026-09-26.md — 1 行
docs/NEST_HTTP_BUDGET_SUPERVISION_IMPLEMENTATION_2026-10-09.md — 1 行
docs/NEST_INVESTIGATOR_RECEIPT_RECOVERY_AUDIT_2026-09-26.md — 1 行
docs/NEST_LEGACY_KNOWLEDGE_INVENTORY_AUDIT_2026-09-30.md — 1 行
docs/NEST_MASTER_SUPERVISION_REQUEST_CAPACITY_2026-10-08.md — 1 行
docs/NEST_MULTI_IDENTITY_HTTP_ORIGINAL_SETTLEMENT_AUDIT_2026-10-04.md — 1 行
docs/NEST_MULTI_TERMINAL_ORIGINAL_AUDIT_2026-10-04.md — 1 行
docs/NEST_NATIVE_ORIGINAL_ENTRY_REGRESSION_AUDIT_2026-10-04.md — 1 行
docs/NEST_NATIVE_TICK_IDENTITY_MIGRATION_AUDIT_2026-10-04.md — 1 行
docs/NEST_ORIGINAL_TERMINAL_IDENTITY_AUDIT_2026-10-03.md — 1 行
docs/NEST_READONLY_RECOVERY_AUDIT_2026-09-26.md — 1 行
docs/NEST_REAL_DATA_RETIREMENT_INVENTORY_2026-10-02.md — 1 行
docs/NEST_RECORDED_TRACE_THEME_AUDIT_2026-10-03.md — 1 行
docs/NEST_REM_A02_COMPLETION_AUDIT_2026-10-07.md — 1 行
docs/NEST_REVIEW_RECEIPT_RECOVERY_AUDIT_2026-09-26.md — 1 行
docs/NEST_ROOT_BUDGET_TRIGGER_AUDIT_2026-10-05.md — 1 行
docs/NEST_ROOT_FINITE_COST_AUDIT_2026-10-05.md — 1 行
docs/NEST_ROOT_MAPPER_ALLOCATION_AUDIT_2026-10-05.md — 1 行
docs/NEST_ROOT_UTILITY_AND_PAID_PROPOSAL_AUDIT_2026-10-03.md — 1 行
docs/NEST_SETTINGS_RETIREMENT_AUDIT_2026-10-01.md — 1 行
docs/NEST_SEVEN_TERMINAL_ORIGINAL_AUDIT_2026-10-04.md — 1 行
docs/NEST_SINGLE_TOOL_HTTP_BUDGET_AUDIT_2026-10-02.md — 1 行
docs/NEST_SOURCE_BROKER_ORIGINAL_AUDIT_2026-10-04.md — 1 行
docs/NEST_SOURCE_CI_EXPORT_AUDIT.md — 1 行
docs/NEST_SOURCE_CI_POLICY_AUDIT.md — 1 行
docs/NEST_SOURCE_COMPLETION_PROOF_AUDIT.md — 1 行
docs/NEST_SOURCE_COVERAGE_PREPARATION_AUDIT.md — 1 行
docs/NEST_SOURCE_COVERAGE_REVIEW_EXECUTION_AUDIT.md — 1 行
docs/NEST_SOURCE_DETERMINED_REDUCER_AUDIT_2026-10-03.md — 1 行
docs/NEST_SOURCE_DIFF_MANIFEST_AUDIT.md — 1 行
docs/NEST_SOURCE_EXHAUSTED_FAILURE_DELETION_AUDIT_2026-10-05.md — 1 行
docs/NEST_SOURCE_EXHAUSTED_PENDING_ROOT_REENTRY_AUDIT_2026-10-05.md — 1 行
docs/NEST_SOURCE_FINDINGS_READ_AUDIT.md — 1 行
docs/NEST_SOURCE_INITIAL_ASSESSMENT_DISPATCH_AUDIT.md — 1 行
docs/NEST_SOURCE_MULTI_ENGINE_PROVENANCE_AUDIT.md — 1 行
docs/NEST_SOURCE_PAUSE_BRANCH_ORIGINAL_AUDIT_2026-10-05.md — 1 行
docs/NEST_SOURCE_PAUSE_RESULT_RECOVERY_AUDIT_2026-10-05.md — 1 行
docs/NEST_SOURCE_REVIEW_REENTRY_AUDIT.md — 1 行
docs/NEST_SOURCE_REVIEW_SURFACE_AUDIT.md — 1 行
docs/NEST_SOURCE_RUNTIME_PUBLICATION_AUDIT.md — 1 行
docs/NEST_SOURCE_SCOPE_CONTRACT_AUDIT.md — 1 行
docs/NEST_SOURCE_TRACE_PROJECTION_AUDIT.md — 1 行
docs/NEST_SPECIALIST_RECEIPT_AUDIT_2026-09-26.md — 1 行
docs/NEST_TARGET_REQUEST_ACCOUNTING_AUDIT_2026-09-26.md — 1 行
docs/NEST_TOOLCHAIN_DIGEST_CPU_AUDIT_2026-09-27.md — 1 行
docs/NEST_UNLIMITED_MODEL_EXECUTION_AUDIT_2026-10-05.md — 1 行
docs/NEST_WEB_LEASE_SUPERVISION_IMPLEMENTATION_2026-10-09.md — 1 行
docs/NEST_WORKBENCH_ADMISSION_AUDIT.md — 1 行
docs/architecture.md — 1 行
docs/local-incremental-20261015/final-verification.json — 1 行
package.json — 1 行
src-tauri/resources/workers/9_frontend_runtime_probe.cjs — 1 行
src-tauri/src/agent_runtime/contract.rs — 1 行
src-tauri/src/agent_runtime/deleted_scan_audit.rs — 1 行
src-tauri/src/agent_runtime/deleted_scan_audit/rows.rs — 1 行
src-tauri/src/agent_runtime/model/tests.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/budget/financial_continuation_schema.sql — 1 行
src-tauri/src/agent_runtime/multi_agent/budget/root.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/budget/root_definition.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/client_side.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/homogeneous/execution_worker.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/homogeneous/intent.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/homogeneous/model_worker.rs — 1 行
src-tauri/src/agent_runtime/multi_agent/homogeneous/test_fixture.rs — 1 行
src-tauri/src/agent_runtime/role_config/tests.rs — 1 行
src-tauri/src/agent_runtime/single_projection/deletion.rs — 1 行
src-tauri/src/agent_runtime/stage1a_tests/contract_assignment.rs — 1 行
src-tauri/src/agent_runtime/store.rs — 1 行
src-tauri/src/agent_runtime/web_mode/action.rs — 1 行
src-tauri/src/artifact_import/adapters/frontend_recon.rs — 1 行
src-tauri/src/artifact_import/adapters/source_report.rs — 1 行
src-tauri/src/artifact_import/store.rs — 1 行
src-tauri/src/artifact_import/tests.rs — 1 行
src-tauri/src/artifact_import/tests_coverage_retirement.rs — 1 行
src-tauri/src/artifact_import/tests_import_corruption.rs — 1 行
src-tauri/src/artifact_import/tests_import_regressions.rs — 1 行
src-tauri/src/artifact_import/tests_result_retirement.rs — 1 行
src-tauri/src/artifact_import/tests_sqlite_retirement.rs — 1 行
src-tauri/src/collaboration_events.rs — 1 行
src-tauri/src/commands/agent_authorization_broker.rs — 1 行
src-tauri/src/commands/agent_homogeneous_execution.rs — 1 行
src-tauri/src/commands/agent_native/business_logic_input_collection.rs — 1 行
src-tauri/src/commands/agent_native/coordinator_protection_original.rs — 1 行
src-tauri/src/commands/agent_native/coordinator_recovery_source.rs — 1 行
src-tauri/src/commands/agent_native/external_surface_output_collection.rs — 1 行
src-tauri/src/commands/agent_native/single_finally.rs — 1 行
src-tauri/src/commands/agent_request_accounting.rs — 1 行
src-tauri/src/commands/agent_request_review.rs — 1 行
src-tauri/src/commands/agent_runtime.rs — 1 行
src-tauri/src/commands/agent_terminal_report.rs — 1 行
src-tauri/src/commands/agent_tests_anonymous_control_original.rs — 1 行
src-tauri/src/commands/agent_tests_changed_fact_reviewer.rs — 1 行
src-tauri/src/commands/agent_tests_child_completion.rs — 1 行
src-tauri/src/commands/agent_tests_client_root_feedback.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_closure.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_delivery_fixture.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_delivery_writer.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_paid_task_change.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_production_boundary.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_production_hook.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_production_scope.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_production_writer.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_readonly.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_readonly_negative.rs — 1 行
src-tauri/src/commands/agent_tests_client_side_store_scope.rs — 1 行
src-tauri/src/commands/agent_tests_directive_actions.rs — 1 行
src-tauri/src/commands/agent_tests_directive_closure.rs — 1 行
src-tauri/src/commands/agent_tests_directive_proposals.rs — 1 行
src-tauri/src/commands/agent_tests_financial_continuation_desktop.rs — 1 行
src-tauri/src/commands/agent_tests_financial_continuation_discovery.rs — 1 行
src-tauri/src/commands/agent_tests_financial_continuation_received_crash.rs — 1 行
src-tauri/src/commands/agent_tests_financial_continuation_remaining_limits.rs — 1 行
src-tauri/src/commands/agent_tests_financial_continuation_status.rs — 1 行
src-tauri/src/commands/agent_tests_fresh_web_budget_producer.rs — 1 行
src-tauri/src/commands/agent_tests_harness.rs — 1 行
src-tauri/src/commands/agent_tests_homogeneous_frame.rs — 1 行
src-tauri/src/commands/agent_tests_human_proposal_owned_sdk.rs — 1 行
src-tauri/src/commands/agent_tests_identity_e2e.rs — 1 行
src-tauri/src/commands/agent_tests_mapper_loop_deleted_archive.rs — 1 行
src-tauri/src/commands/agent_tests_mapper_loop_natural_expiry_two_round_faults.rs — 1 行
src-tauri/src/commands/agent_tests_mapper_loop_second_round_unknown.rs — 1 行
src-tauri/src/commands/agent_tests_multi_anonymous_control_original.rs — 1 行
src-tauri/src/commands/agent_tests_multi_terminal_original.rs — 1 行
src-tauri/src/commands/agent_tests_native_plan_owner.rs — 1 行
src-tauri/src/commands/agent_tests_native_sdk_expert_logs.rs — 1 行
src-tauri/src/commands/agent_tests_ordered_primary_error.rs — 1 行
src-tauri/src/commands/agent_tests_original_terminal_consumer.rs — 1 行
src-tauri/src/commands/agent_tests_original_terminal_identity.rs — 1 行
src-tauri/src/commands/agent_tests_protection_original_files.rs — 1 行
src-tauri/src/commands/agent_tests_recovery_history.rs — 1 行
src-tauri/src/commands/agent_tests_root_budget.rs — 1 行
src-tauri/src/commands/agent_tests_root_budget_liveness.rs — 1 行
src-tauri/src/commands/agent_tests_root_controlled_executor.rs — 1 行
src-tauri/src/commands/agent_tests_root_tick_sdk_cancel.rs — 1 行
src-tauri/src/commands/agent_tests_root_v2_slot_fresh_fixture.rs — 1 行
src-tauri/src/commands/agent_tests_root_v2_slot_guards.rs — 1 行
src-tauri/src/commands/agent_tests_scan_deletion_multi_archive_schema.rs — 1 行
src-tauri/src/commands/agent_tests_seven_terminal_producers.rs — 1 行
src-tauri/src/commands/agent_tests_single_budget_contracts.rs — 1 行
src-tauri/src/commands/agent_tests_single_budget_edges.rs — 1 行
src-tauri/src/commands/agent_tests_single_http_inflight.rs — 1 行
src-tauri/src/commands/agent_tests_single_projection_faults.rs — 1 行
src-tauri/src/commands/agent_tests_specialist_inflight_closure.rs — 1 行
src-tauri/src/commands/agent_tests_specialist_original_fixture.rs — 1 行
src-tauri/src/commands/agent_tests_target_budget_preflight.rs — 1 行
src-tauri/src/commands/agent_tests_web_fresh_finance.rs — 1 行
src-tauri/src/commands/agent_tools_browser.rs — 1 行
src-tauri/src/commands/agent_tools_http/identity.rs — 1 行
src-tauri/src/commands/agent_tools_identity_diff.rs — 1 行
src-tauri/src/commands/agent_tools_specs.rs — 1 行
src-tauri/src/commands/agent_tools_verdict.rs — 1 行
src-tauri/src/commands/asset_ownership.rs — 1 行
src-tauri/src/commands/exposure.rs — 1 行
src-tauri/src/commands/exposure/remediation.rs — 1 行
src-tauri/src/commands/exposure/tests_g1.rs — 1 行
src-tauri/src/commands/frontend_recon_compact.rs — 1 行
src-tauri/src/commands/frontend_recon_scoring.rs — 1 行
src-tauri/src/commands/investigation_read.rs — 1 行
src-tauri/src/commands/multi_agent/prepare.rs — 1 行
src-tauri/src/commands/multi_agent/review_validation.rs — 1 行
src-tauri/src/commands/multi_agent/tests/attempt_expired_saved_faults.rs — 1 行
src-tauri/src/commands/multi_agent/tests/budget_final_elapsed.rs — 1 行
src-tauri/src/commands/multi_agent/tests/budget_final_elapsed_ownership.rs — 1 行
src-tauri/src/commands/multi_agent/tests/fixtures.rs — 1 行
src-tauri/src/commands/multi_agent/tests/mailbox.rs — 1 行
src-tauri/src/commands/native_financial_continuation_files.rs — 1 行
src-tauri/src/commands/native_helper_live_limits.rs — 1 行
src-tauri/src/commands/native_protection_recovery_signature.rs — 1 行
src-tauri/src/commands/native_recon_contract.rs — 1 行
src-tauri/src/commands/native_sensitive.rs — 1 行
src-tauri/src/commands/native_source_assessment_authority.rs — 1 行
src-tauri/src/commands/native_source_pause_recovery_writer.rs — 1 行
src-tauri/src/commands/result_ingestion_engines.rs — 1 行
src-tauri/src/commands/tests.rs — 1 行
src-tauri/src/commands/tests_native_root_decision_timeline.rs — 1 行
src-tauri/src/commands/tests_project_deletion.rs — 1 行
src-tauri/src/commands/tests_root_decision_chat_cursor_guards.rs — 1 行
src-tauri/src/commands/tests_source_coverage.rs — 1 行
src-tauri/src/commands/tests_source_dispatch.rs — 1 行
src-tauri/src/commands/tests_source_heartbeat_collateral.rs — 1 行
src-tauri/src/commands/tests_source_recovered_exhausted_original.rs — 1 行
src-tauri/src/commands/tests_source_reentry.rs — 1 行
src-tauri/src/commands/tests_source_result_original_history.rs — 1 行
src-tauri/src/commands/tests_source_review_material.rs — 1 行
src-tauri/src/commands/tests_web_mode_independent_actions.rs — 1 行
src-tauri/src/commands/tests_web_toolchain_binding.rs — 1 行
src-tauri/src/conversations/asset_statistics_tests.rs — 1 行
src-tauri/src/conversations/store.rs — 1 行
src-tauri/src/db_neutral.rs — 1 行
src-tauri/src/db_settings.rs — 1 行
src-tauri/src/native_pipeline/greybox.rs — 1 行
src-tauri/src/native_pipeline/tests.rs — 1 行
src-tauri/src/page_isolation.rs — 1 行
src-tauri/src/skills_catalog/learning_model.rs — 1 行
src-tauri/src/skills_catalog/patch.rs — 1 行
src-tauri/src/skills_catalog/sources.rs — 1 行
src-tauri/src/tool_supply/frozen_plan.rs — 1 行
src-tauri/src/tool_supply/tests/staging.rs — 1 行
src-tauri/tests/fixtures/legacy_strix/bundles/producer_1_6_2/findings.sarif — 1 行
src/features/assets/components/AssetOwnershipGate.vue — 1 行
src/features/assets/exposure/useExposureSurface.ts — 1 行
src/features/sentinel/components/agentDialogLabels.ts — 1 行
src/features/sentinel/components/results/SentinelTraceTimeline.vue — 1 行
src/features/sentinel/components/taskCenterPresentation.css — 1 行
src/features/sentinel/recovery/api.ts — 1 行
src/features/sentinel/results/taskPageContract.ts — 1 行
src/features/sentinel/styles/evidence.css — 1 行
src/features/sentinel/styles/findings.css — 1 行
src/features/sentinel/styles/workspace.css — 1 行
src/features/sentinel/workbench/WorkbenchCicdSettings.vue — 1 行
src/features/sentinel/workbench/WorkbenchTaskNotices.vue — 1 行
src/features/skills/SkillInstructionsCatalog.vue — 1 行
src/features/workspaces/components/ProjectDialog.vue — 1 行
src/plugins/preferences.ts — 1 行
src/shell/builtinWorkspaces.ts — 1 行
src/shell/navigation.ts — 1 行
src/shell/types.ts — 1 行
tools/chat_dom/Fixture.vue — 1 行
tools/live_runner_log_harness.cjs — 1 行
tools/sentinel_board/page_contract.cjs — 1 行
tools/sentinel_board/trace_timeline.cjs — 1 行
tools/test_agent_request_usage.cjs — 1 行
tools/test_conversation_workspace.cjs — 1 行
tools/test_fuse_details.cjs — 1 行
tools/test_fuse_disposition.cjs — 1 行
tools/test_fuse_presentation.cjs — 1 行
tools/test_js_files.cjs — 1 行
tools/test_native_process_log_replay.cjs — 1 行
tools/test_ordered_human_execution_ui.cjs — 1 行
tools/test_request_headers.cjs — 1 行
tools/test_runtime_exploration.cjs — 1 行
tools/test_workbench_web_mode.cjs — 1 行
