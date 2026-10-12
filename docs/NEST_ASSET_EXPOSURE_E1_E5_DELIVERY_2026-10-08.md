# Qoder 交付：公开暴露面 E1–E5（2026-10-08）

按 `docs/NEST_ASSET_EXPOSURE_QODER_FRAMEWORK_2026-10-08.md` 执行，只改第 1 节白名单文件。工作树 HEAD `14a7875`（未提交，未推送）。本轮全程未运行用户真实采集，未结束其他代理进程，未改共享文件。

证据等级标注口径：**生产可达** = 真实入口能走到；**未接入骨架** = 代码存在但生产链未调用；**只读诊断** = 只能查询不能改变状态；**测试证据** = 仅临时 SQLite/localhost 镜像或纯前端模拟。localhost 不等于第三方，临时库不等于用户真实库。

## 0. 开工核对与白名单

- 主控本轮已注册的九个 IPC 复核通过：`src-tauri/src/lib.rs:370-378` 含 `page_exposure_findings`、`page_exposure_runs`、`list_exposure_source_results_for_project`、`read_exposure_finding`、`exposure_scope_summary`、`exposure_known_limits`、`resume_exposure_scan`、`page_exposure_pending_discoveries`、`read_exposure_evidence`。`package.json:9` 有 `test:asset-exposure`。
- 共享文件零改动：`git diff -- lib.rs commands.rs db_schema.rs src/types.ts src/api.ts src/features/assets/api.ts src/App.vue` 中 `cluster|exposure_setting` 命中数为 0；lib.rs 里 exposure 相关行只有上述九项 + 旧的四个写入项，均由主控添加。
- 交付 SHA（`git hash-object`，全量 54 项见 `/tmp/e-delivery-sha.txt`，本文件末尾摘录）：门面 `d3d9db7e…`、页面 `7480169…`、CSS `87bbf40…`、`schema.sql 2b6474f…`、`cluster_repository f2ed130…`、`cache 475c676…`、`remediation dfc9212…`（新文件）、`settings 829d75a…`。

## 1. E1 安装入口与范围合同复核

文件：`src/components/ExposureSurface.vue`、`src/exposure-surface.css`、`src/features/assets/exposure/{api.ts,contracts.ts,useExposureSurface.ts,useExposureEvidence.ts}`、`tools/test_asset_exposure_{entry,page,evidence,surface}.cjs`、`src-tauri/src/commands/exposure.rs`（错误文案）。

实现与结论：
- 页面 IPC 全部改走白名单内的 `exposureApi` 包装（`features/assets/exposure/api.ts`），不再向 `src/features/assets/api.ts` 追加内容（该权限本任务书已撤销）。唯一外部调用 `listAssets` 由页面从共享门面注入，未修改共享文件。
- 首次读取失败不再是空列表：`hasSnapshot` 只在成功读取后置真；失败时模板渲染 `.exposure-error` 并带真实错误码文案（`[code] message` 经 `exposureErrorText` 去码后展示）。未注册命令走 `isUnregisteredCommand` 分支，提示"接口尚未注册"，不会显示"当前筛选没有公开暴露线索"。
- A→B→A：以 projectId + generation 双校验丢弃迟到的旧项目响应；刷新失败保留上一次快照并置 `stale`；组件卸载/隐藏停止轮询（`active()` 门 + `onUnmounted`）。汇总与列表共用同一 project 谓词，`findingsTotal` 与列表 `total` 同口径。

证据等级：只读诊断（前端渲染与状态）+ 测试证据。安装 App 端到端验收仍属主控，未做。

## 2. E2 采集配置冻结与恢复闭环

后端文件：`src-tauri/src/commands/exposure/{settings.rs(新),schema.sql,state.rs,orchestrator.rs,recovery.rs,../exposure.rs,tests_e2.rs}`。
前端文件：`src/features/assets/exposure/{api.ts,useExposureSettings.ts(新),ExposureSettingsPanel.vue(新)}`、`src/components/ExposureSurface.vue`、`src/exposure-surface.css`、`tools/test_asset_exposure_settings.cjs(新)`、`tools/test_asset_exposure_page.cjs`。

生产调用路径（已可达，命令注册待主控）：
```
ExposureSurface.vue → ExposureSettingsPanel.vue → useExposureSettings
  → exposureApi.getExposureSettings/saveExposureSettings/listExposureSettingVersions
  → [tauri] get_exposure_settings / save_exposure_settings / list_exposure_setting_versions
  → settings::load/save/history（exposure_settings + exposure_setting_versions）
start_exposure_scan → exposure_scan_settings(读已保存值) → orchestrator::start_or_reuse
  → ExposureSettings::from_json 严格校验 → to_settings_json 冻结进 exposure_run_plans(settings_version,settings_json)
  → run_plan 读 frozen_settings：max_items 之外的项关闭为 blocked_quota（写明额度与剩余数）
resume_exposure_scan → recovery::load_plan → frozen_settings(计划自身) → verify_frozen → execute_exposure_run(同一份冻结配置)
```
要点：
- 配置字段严格解析：未知字段/非整数/越界一律拒绝并点名字段；`0` 与 `null` 语义分离（`null`=默认，`0` 仅授权天数合法）；单位与硬上界随 `bounds` 返回页面。
- 修改当前配置不改旧轮：`e2_a_later_edit_cannot_move_an_interrupted_round_or_reset_its_spending` 证明冻结 `max_items=1`、`version=1` 与 `requests_used=4/max_requests=10` 在编辑后仍按原值继续；用当前配置复用被拒绝。
- 旧版（无 settings_json）的计划行**不能**按新合同恢复，返回 `exposure_plan_settings_unfrozen`（`e2_a_round_started_before_the_settings_contract_cannot_be_resumed`），不悄悄套用今天的默认额度。
- 不默认放开私网/localhost 出口；`exposureAllowPrivateDestinations` 必须显式写入，且只影响之后的轮次。选来源≠跨范围授权。
- 前端：读取→编辑→保存为新版本；保存带上读到的 `storedVersion`，版本冲突保留草稿并显示"未保存"；未注册时明确"配置接口尚未注册"。面板只提供配置读写，不含任何采集启动能力（`tools/test_asset_exposure_page.cjs` 第 4 用例真实挂载面板并断言只发 `get_exposure_settings`）。

回归记录（真实回归，已修）：E2 的冻结改动最初打破两个既有测试——`tests_q3::q3_resume_refuses_a_moved_plan_and_labels_a_legacy_run`（改用计划自身的冻结配置复用）与 `tests_q6::q6_four_thousand_items…`（规模测试的计划行原来没有配置行，被默认 500 项额度截断；改为在该轮显式冻结 `exposureMaxItems=4000`）。同时把 `verify_frozen` 的文案从"采集配置已变化"改为区分"冻结计划与记录的配置不一致"与"旧轮没有冻结配置"，撤权场景不再混报配置原因（新增断言）。

## 3. E3 生产簇写入、簇视图与缓存维度

后端文件：`clusters.rs`（`host_of/domain_of`）、`observations.rs`（写入即记 `canonical_url/root_domain`）、`cluster_repository.rs`（`save_observed`、`persist_for_observations`、`page_clusters`、`page_cluster_members`）、`orchestrator.rs`（run_plan 内接入 + `cache_classified_verdicts`）、`cache.rs`（键维度 + 期限 + 授权门）、`state.rs`/`schema.sql`（新列、索引、迁移）、`../exposure.rs`（两个只读命令）、`tests_e3.rs(新)`。
前端文件：`api.ts`、`useExposureClusters.ts(新)`、`ExposureClustersPanel.vue(新)`、`ExposureSurface.vue`（"内容簇"页签）、`exposure-surface.css`、`tools/test_asset_exposure_clusters.cjs(新)`，并让 page/entry/evidence 三个测试壳真实编译两个子面板（不再打桩）。

生产路径（观察→簇→缓存→IPC→SFC，全部由一轮真实 run 触发，无手工写入）：
```
run_plan: claim → executor(适配器) → record(observations，含 canonical_url/root_domain)
  → merge_finding → record_pending
  → cluster_repository::persist_for_observations(本项观察 id)   ← 观察之后、终态之前
  → cache_classified_verdicts(按 项目/范围版本/来源/规范化URL/内容哈希/配置版本/规则版本 + 授权期限)
  → finish(工作项终态)
page_exposure_clusters / page_exposure_cluster_members → ClusterSummaryRow / ClusterMemberRow → ExposureClustersPanel.vue
```
要点与证据：
- 代表选择沿用 `(归属置信度 DESC, asset_id ASC)` 的确定规则，并新增记录 `representative_observation_id / representative_run_id / settings_version / rule_version / direct_count / inherited_count / gap_note`；成员行带 `observation_id`、`rule_version`、`verified_at`。自带结论=direct（有自身观察且仍准入），否则 inherited；失去准入=stale 且写入 gap 文案。URL path/query 不做整体小写折叠（`fingerprint::canonical_url` 只折 scheme/host 与默认端口，`e3` 与 `q5` 都断言大小写差异）。
- 崩溃重试幂等：`save_observed` 在同一事务里替换成员行；`e3_the_cluster_pass_is_idempotent_when_a_round_repeats_it` 断言重复执行后簇行/成员行数与代表都不变。
- 失败不假装继承覆盖：簇写失败时 `run_plan` 直接返回 Err（`?` 传播），项不落终态；`e3_a_failed_cluster_write_leaves_the_item_open_instead_of_a_coverage_claim` 断言 `completed < total`、run 不是 completed、且没有任何簇行——单例项在图形成前正常收口不算覆盖声明。
- 分页不外泄全量：`page_clusters` 页长限定 10–200，keyset 游标按簇 id；`total` 用不含游标的同一过滤条件（修掉"第二页总数缩水"的实现错误）；未知 `kind`、超界页长、损坏游标都返回错误码而不是空列表；跨项目读成员返回 `exposure_cluster_not_found`。撤权成员立刻 `admittedNow=false`。前端 `useExposureClusters` 换过滤条件即作废已展开成员，切项目先清行再读。
- 缓存维度与优先级：键绑定 项目/scope 版本/隐私类别/内容哈希/规则版本/来源/规范化 URL/配置版本；`lookup_guarded(..., authorized)` 在未授权时返回未命中且**不**计命中数；`expires_at` 过期同样返回未命中且计数不变，行保留为历史。生产端由 `cache_classified_verdicts` 写入确定性规则结论（不落任何证据原文），期限取冻结的 `authorizationDays`；簇视图经 `attach_cache_times` 显示"缓存于/有效期至"。
- 分类：`e4` 与 `e3` 覆盖 12 簇分页、61 簇规模读、单例不成簇、`solo` 资产不入图。

未接入部分（诚实标注）：缓存的**复用消费方**尚不存在——`lookup_guarded` 目前只有合同与测试（未接入骨架），真实消费方是 E6 的模型裁决结论缓存；本轮没有让任何请求因缓存命中而跳过或获得访问资格。隐私类别固定为 `public`（范围门禁只允许公开资产），若将来要缓存非公开类别需先有真实隐私标签（共享 schema 依赖）。

## 4. E4 规模、故障与恢复矩阵

后端文件：`export.rs`（`write_csv` 加符号链接/覆盖防护 + `verify_written`）、`../exposure.rs`（`verify_exposure_export`）、`tests_e4.rs(新)`、`state.rs`/`schema.sql`（观察与簇读取所需索引）。

测得数字（临时 SQLite、可重现种子、`synchronous=OFF`、`--test-threads=1`，本机 arm64）：
- 100,000 工作项物化 `plan_ms=2591`（三次运行 2591/2631/3228）；600 条线索分 12 页读完 `paging_ms=16`；库文件 `db_bytes=29,421,568`（约 28 MB）。
- 61 个簇的簇页 `clusters_page_ms=1`，成员页 `members_ms=0`。
- 10,000 项 + 冻结 `max_items=200`：`completed=200 / quota=9800 / open=0`，状态 `partial`，缺口行 reason 明确写"本轮工作项额度 200，剩余 9800 项未采集"，`enumerate_gaps(limit=500)` 有界。
- 内存占用未测（本轮无进程级 RSS 采样），列为未做项。

故障点矩阵（本轮新增，均测试证据）：
| 故障点 | 用例 | 结论 |
| --- | --- | --- |
| 计划提交前后 | `e4_a_failed_plan_commit_leaves_no_active_round`（BEFORE INSERT 触发器 `RAISE(ABORT)`） | 返回"计划冻结失败：注入的计划提交故障"；run 行数 0、work_items 行数 0（同一事务回滚），去掉触发器后同项目可再启动 |
| 观察已写、终态未写 | `e4_a_crash_after_the_observation_reuses_it…` | 心跳老化后 `mark_interrupted_if_dead` 判中断；重开复用同一观察行（行数 2 不翻倍），尝试次数≥2，`meter.requests_used=3` 未被重置，簇图收敛；run 状态仍为 interrupted（只有 finalize 有权改终态） |
| 簇持久化前后 | `e3_a_failed_cluster_write…` + 幂等用例 | 失败即项不终态、轮不完成；重复执行图不变 |
| 并发启动/租约过期/迟到 worker/取消/撤权后零新请求/进程死亡 | 沿用 Q2/Q3 既有 11 项（含真实 SIGKILL 子进程） | 本轮复跑全绿，未重复实现 |

磁盘证据：`write_csv` 现在拒绝已存在目标、符号链接与非普通文件（`create_new`），写后再次校验类型；`verify_written` 拒绝跟随链接、校验表头列数与每行列数，回给 `bytes/sha256/rows/ruleVersion/redactionVersion`。`verify_exposure_export` 命令额外要求解析后的父目录等于本应用 `export_dir`（canonicalize 会拆穿借符号链接指向别处的"导出目录"）。页面/日志/CSV 仍统一走 `redaction`，测试断言导出内不含凭据原文且 `'=…` 公式被转义。

## 5. E5 确定性复核、整改与复测

后端文件：`remediation.rs(新)`、`schema.sql`/`state.rs`（`exposure_remediations`、`exposure_remediation_retests`、复核事件加 `expected_revision/references_json`）、`../exposure.rs`（8 个新命令）、`tests_e5.rs(新)`。
前端文件：`api.ts`、`useExposureReview.ts(新)`、`ExposureReviewPanel.vue(新)`、`ExposureSurface.vue`（"复核整改"页签 + 线索选择器）、`exposure-surface.css`、`tools/test_asset_exposure_review.cjs(新)`。

生产路径（写侧命令注册前为未接入骨架，读侧合同与实现已就绪）：
```
ExposureReviewPanel → useExposureReview
  读：read_exposure_evidence(取最大 revision 与最新观察 id) + list_exposure_review_trail
      + page_exposure_remediations + read_exposure_retest_outcome + exposure_review_taxonomy
  写：review_exposure_finding_scoped(projectId, findingId, expectedRevision, status, actor, reason, references)
      save_exposure_remediation / start_exposure_retest / mark_exposure_remediation
```
要点：
- 复核带版本冲突检查：`expected_revision != MAX(revision)` 时整笔拒绝（`exposure_review_conflict：队列已前进到 r{top}…`），并拒绝空操作者、空依据、未知状态、跨项目；结论写入 `exposure_review_events`（from/to、actor、reason 经脱敏、引用观察 id、时间），旧观察一行不动。
- 整改绑定证据：`exposure_remediations(project_id, finding_id, observation_id, run_id, actor, status, note)`，同一证据重复登记走 `ON CONFLICT … DO UPDATE`；未知 observation 或跨项目一律拒绝。
- 复测必是新轮：`start_retest` 先 `ExposureScope::load` 重新验权（撤权或无准入资产 → `exposure_retest_unauthorized`/范围拒绝码），再 `start_or_reuse` 冻结当前配置建新计划，把新 run 与原始 URL 记入 `exposure_remediation_retests`，整改转 `awaiting_retest`；被拒绝时状态与链接都不变。旧观察内容哈希保持不变（测试断言）。
- 关闭必须有依据：`retest_outcome` 只报告 `retest_content_changed / identical / not_seen / unreachable / round_incomplete / without_baseline / no_retest_round`，其中只有 `changed` 置 `canClose=true`；`mark_remediation(..., 'closed')` 在无依据时返回 `exposure_remediation_unclosed`，成功后把线索退回 `reviewing` 并写一条带复测依据的复核事件——绝不把原观察改成"已消失"。
- 分类保持区分：`taxonomy()` 输出已确认暴露/待复核/来源不可达/未覆盖/规则命中/证据不足/已排除，每个数字来自真实行；"没有观察支撑"单列为 `unknown`，测试断言它不会被算进结论数。CSV 与页面同口径（`e5_the_csv_export_reflects_the_reviewed_status_of_each_row`）。

前端诚实性：`tools/test_asset_exposure_review.cjs` 断言未注册≠没有记录、提交必带读到的 r{revision}、匿名/无依据不发 IPC、版本冲突标记"没有写入"、`retest_not_seen` 等四种依据都不允许关闭、被拒复测不改状态、切线索丢弃迟到响应、面板无任何采集控制且只走 `exposureApi`。

## 6. 门禁（本轮全部重跑）

| 命令 | 退出码 | 结果 |
| --- | --- | --- |
| `rustfmt --edition 2021 --check` 仅自有 `.rs`（含门面与全部子模块） | 0 | 无差异 |
| `nice -n 15 cargo test -j 1 --lib -- --test-threads=1 commands::exposure` | 0 | `94 passed; 0 failed; 0 ignored`（E 阶段新增 27：e2 7 / e3 7 / e4 6 / e5 7；其余为 Q1–Q6 与模块内联测试） |
| `nice -n 15 cargo clippy -j 1 --lib --tests` | 0 | `src/commands/exposure*` 诊断 0 条（其他模块的既有告警未触碰） |
| `node --test --test-concurrency=1 tools/test_asset_exposure_*.cjs` | 0 | `59 tests / 59 pass / 0 fail`（surface 12、page 4、evidence 9、entry 7、settings 7、clusters 11、review 9） |
| `npx vue-tsc --noEmit` | 0 | 0 error |
| `npm run build` | 0 | `✓ built`，`dist/assets/ExposureSurface-*.js` 含新面板 |
| `git diff --check`（仅自有文件） | 0 | 无空白错误 |

重型 cargo 任务排队说明：三次过滤运行均为 `-j 1 --test-threads=1 nice -n 15`，开始前确认 `src-tauri/target` 构建锁空闲（他人 PID 35170 的 coordinator 测试进程当时已自行结束）；全程未终止任何他人进程，未做全库格式化，未 reset/clean。

## 7. 失败、未做与外部依赖

未做/未测（不用局部绿称完成）：
1. 安装 App 端到端验收：E1–E5 的九个 + 十一个新命令都未在真实 Tauri 窗口里跑过用户库；临时 SQLite 与纯前端模拟不等于真实验收。
2. E4 的内存指标未采样；20,000/100,000 只在临时库测量，未在真实用户库上评估。
3. 缓存复用消费方未接入（见 §3）；模型裁决（E6）按要求保持 not_ready，本轮没有伪造模型结果、Root 或预算，也没有翻动 Native ExternalSurface 开关。
4. 簇视图与复核面板只做前端 SFC/无头渲染与状态断言，未在浏览器里点选验收。
5. `e4` 规模用例单次约 30–60 秒（100k 物化），如主控认为过重可降到 50k 档；本轮未擅自裁剪。

需要主控处理（不"临时绕过"，逐条列明）：
1. **IPC 注册**（`src-tauri/src/lib.rs`，函数位置均在 `src-tauri/src/commands/exposure.rs`，全部 `#[tauri::command]`）：
   - E2：`get_exposure_settings(projectId) -> JsonValue`（只读）、`save_exposure_settings(projectId, expectedVersion, settings) -> i64`（写）、`list_exposure_setting_versions(projectId, limit) -> {items,truncated}`（只读）。
   - E3：`page_exposure_clusters(projectId, kind, search, pageSize, cursor) -> ExposurePage<ClusterSummaryRow>`（只读）、`page_exposure_cluster_members(projectId, clusterId, pageSize, cursor) -> ExposurePage<ClusterMemberRow>`（只读）。
   - E4：`verify_exposure_export(path) -> JsonValue`（只读，限定本应用 export_dir）。
   - E5：`review_exposure_finding_scoped(projectId, findingId, expectedRevision, status, actor, reason, references) -> i64`（写）、`list_exposure_review_trail(projectId, findingId) -> Vec<JsonValue>`（只读）、`save_exposure_remediation(projectId, findingId, observationId, runId, actor, note, status) -> i64`（写）、`page_exposure_remediations(projectId, status, pageSize, cursor) -> ExposurePage<RemediationRow>`（只读）、`start_exposure_retest(projectId, remediationId) -> i64`（写，后台线程执行新一轮）、`read_exposure_retest_outcome(projectId, remediationId) -> JsonValue`（只读）、`mark_exposure_remediation(projectId, remediationId, status, actor, reason) -> JsonValue`（写）、`exposure_review_taxonomy(projectId) -> Vec<TaxonomyRow>`（只读）。
   错误码约定：读侧统一 `[exposure_*] message`（`ExposureQueryError`），写侧返回原文案字符串；页面已按"未注册 ≠ 空结果"处理，未注册时显示"接口尚未注册"。
2. **`package.json` 脚本补齐**：`test:asset-exposure` 目前只列 surface/page/evidence（25 项），漏掉 entry/settings/clusters/review 共 34 项。建议改为 `node --test --test-concurrency=1 tools/test_asset_exposure_surface.cjs tools/test_asset_exposure_page.cjs tools/test_asset_exposure_evidence.cjs tools/test_asset_exposure_entry.cjs tools/test_asset_exposure_settings.cjs tools/test_asset_exposure_clusters.cjs tools/test_asset_exposure_review.cjs`（我不改共享脚本）。
3. **共享 schema**：本轮全部结构仍走自有附加迁移（`exposure/**/schema.sql` + `EXPOSURE_MIGRATIONS`，`ensure_exposure_schema` 幂等，容忍 duplicate column / already exists）。若要把 `exposure_settings`、`exposure_setting_versions`、`exposure_pending_discoveries`、`exposure_remediations`、`exposure_remediation_retests`、`exposure_observations(canonical_url,root_domain)`、`exposure_analysis_cache(source_key,canonical_url,settings_version,expires_at)`、`exposure_review_events(expected_revision,references_json)` 折进 `db_schema.rs`，表名/列名/迁移顺序与保留历史规则见 `src-tauri/src/commands/exposure/schema.sql` 注释；新增列一律 `NOT NULL DEFAULT`，历史行不做回填抹平（旧计划行按 `exposure_plan_settings_unfrozen` 显式拒绝复用，旧簇行 `direct_count/inherited_count` 默认 0 需重跑一轮才补上）。
4. **全局 TS 类型**：`src/types.ts` 未动；新类型都在 `src/features/assets/exposure/{api.ts,contracts.ts}` 内自持（`ExposureCluster/ExposureClusterMember/ExposureRemediation/ExposureRetestOutcome/ExposureTaxonomyRow/ExposureReviewEvent/ExposureSettingsView/ExposureSettingVersionPage`）。
5. **旧复核命令的迁移依赖**：`review_exposure_finding` 的输入结构在共享 models 里没有 project/revision 字段，因此它无法做跨项目与版本防护；E5 提供 `review_exposure_finding_scoped` 作为替代入口，需要主控决定页面何时切换、以及是否保留旧入口做兼容只读。
6. **E6**：模型裁决的候选输入、结构化输出、原证据引用与费用/授权需求清单可作为下一步单独交付；ExternalSurface/Broker 未就绪前继续 E1–E5 的独立工作，不复制执行框架。

## 8. 可复现验收步骤

```bash
cd /Users/swyiic/Desktop/Rust/Oviraptor/src-tauri
rustfmt --edition 2021 --check $(ls src/commands/exposure.rs src/commands/exposure/*.rs src/commands/exposure/adapters/*.rs)
nice -n 15 cargo test -j 1 --lib -- --test-threads=1 commands::exposure          # 期望 94 passed / 0 failed
nice -n 15 cargo test -j 1 --lib -- --test-threads=1 --nocapture commands::exposure::tests_e4   # 打印 E4 SCALE / CLUSTER PAGING 数字
nice -n 15 cargo clippy -j 1 --lib --tests                                        # 自有文件 0 诊断

cd ..
timeout 240 node --test --test-concurrency=1 --test-timeout=20000 tools/test_asset_exposure_*.cjs   # 期望 59 / 59 / 0
npx vue-tsc --noEmit
npm run build
git diff --check -- src-tauri/src/commands/exposure.rs src-tauri/src/commands/exposure src/features/assets/exposure \
  src/components/ExposureSurface.vue src/exposure-surface.css tools/test_asset_exposure_
git diff -- src-tauri/src/lib.rs src-tauri/src/commands.rs src-tauri/src/db_schema.rs src/types.ts src/api.ts \
  src/features/assets/api.ts src/App.vue | grep -icE "cluster|exposure_setting"                     # 期望 0
```

主控在安装 App 里的建议验收（临时数据，不污染用户库）：注册第 7 节命令后，①新建项目→采集配置改 `max_items=3` 保存→启动→看 run 是否显示"额度 3，剩余 N 项未采集"，再改回 20 并确认旧轮恢复仍按 3；②采完一轮后打开"内容簇"，确认代表/成员、直接 vs 继承、缺口与缓存时间都来自真实行，且撤权某资产后该成员显示"已失效"；③"复核整改"里用旧 revision 提交复核应被拒并保留草稿，登记整改→"另起一轮复测"生成新 run 且旧观察不变；④导出 CSV 后用 `verify_exposure_export` 校验行数与 SHA-256，并把导出目录换成符号链接验证被拒。
