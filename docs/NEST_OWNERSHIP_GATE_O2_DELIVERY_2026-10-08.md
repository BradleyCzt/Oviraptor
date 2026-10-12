# 归属门禁 O2 交付：后台聚合复核闭环（2026-10-08）

依据：`docs/NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md`（§6–§10、§12），`docs/NEST_OWNERSHIP_GATE_O0_O1_CLOSURE_2026-10-08.md`。
分支 `nest/master-checkpoint-2026-10-05`，HEAD `14a7875` 未变；未 commit/push/stash/reset/checkout；没有写过真实用户库（见文末“真实库”）。

## 1. 改动范围

| 文件 | 性质 | 说明 |
|---|---|---|
| `src-tauri/src/commands/ownership_public_suffix.rs` | 新增 | 公共后缀（PSL）算法：最长匹配、`*.` 通配、`!` 例外、默认 `*`，区分 ICANN / PRIVATE |
| `src-tauri/src/commands/ownership_public_suffix_list.dat` | 新增 | 由 publicsuffix.org 列表压缩：VERSION `2026-10-07_07-28-19_UTC`、COMMIT `3929462652695bad04f0a27afb600974014a3c8b`，原文件 sha256 `ba7d836c…a32612`；10336 条规则 + 459 条 punycode 变体，保留 MPL-2.0 头 |
| `src-tauri/src/commands/ownership_review.rs` | 新增 | 分析任务、分组、快照、分页、组预览/确认、审计、IPC 与 O2 测试 |
| `src-tauri/src/db_schema.rs` | 追加 5 张表 | 紧接 `org_directory_changes` 之后；不使用 WITHOUT ROWID（已有测试按 `rowid` 快照全表） |
| `src-tauri/src/commands.rs` | +2 行 | `include!` 两个新文件 |
| `src-tauri/src/lib.rs` | +7 行 | 注册 7 个新命令 |
| `src/features/assets/ownershipDirectoryApi.ts` | 追加 | O2 类型与 `ownershipReviewApi`、`OWNERSHIP_LANE_LABELS`、`OWNERSHIP_NAV_LABELS` |
| `src/components/OwnershipReviewGroups.vue` | 新增 | 决策组队列组件 |
| `src/components/AssetOwnershipGate.vue` | +12 行 | 挂载组队列；URL 列表改名为“资产明细（组内详情与逐条修正）”；确认归属卡片说明改为“组确认不含授权” |
| `tools/test_ownership_review_groups.cjs` | 新增 | 组件测试（编译真实 SFC 的 script + template） |

未改：`assets.rs` 准入筛选、`exposure_*`、Native / `agent_runtime`、`asset_ownership.rs`、`ownership_directory.rs`、`OwnershipProjectSetup.vue`、`api.ts`、`package.json`。没有另建 exposure_entities 之外的第二份目录：分类直接读取 O1 的 `org_*` 目录与项目绑定。

## 2. 新表

- `ownership_analysis_runs`：状态 queued/running/completed/cancelled/failed；绑定 `scope_version`（范围 SQL 的 sha16）、`data_revision`、`policy_version`、`directory_version`、`binding_version`、`algorithm_version`（`o2-review-groups-v1`）；`cancel_requested`、`worker_token`、`heartbeat_epoch`、`last_asset_id`（稳定主键游标）、`processed_assets`、`members_pruned`。部分唯一索引 `ux_ownership_runs_active` 保证每个项目最多一个 queued/running。
- `ownership_review_groups`：`UNIQUE(run_id, group_key)`；车道 lane、导航桶、候选实体/标签/来源、关系、地域线索、边界类型/值、支持/反证、人工结论、授权、准入、成员数、不同主机数、样本、理由、优先级、`sort_rank`、`group_version`、`applied_operations`。索引 `(run_id, sort_rank)`、`(run_id, lane, sort_rank)`。
- `ownership_review_group_members`：`PRIMARY KEY(run_id, asset_id)`，同一快照每个资产恰好一个主组；保存快照时的 `decision_state` 与 `evidence_hash`。索引 `(run_id, group_id, asset_id)`。
- `ownership_operations`：`operation_id` 主键、`payload_hash`、`expected_action_hash`、applied/skipped/conflicted/failed、`result_json`。
- `ownership_operation_items`：`PRIMARY KEY(operation_id, asset_id)`，逐条 outcome/reason/`old_json`/`new_json`/`evidence_ref`（`run:X/group:Y/evidence:H`）。

## 3. 新 IPC

| 命令 | 作用 |
|---|---|
| `start_ownership_analysis(projectId)` | 建立或复用活动任务；后台线程分块执行（500/块）；活动任务心跳超时（120s）才会接管 |
| `read_ownership_analysis(projectId)` | 活动任务/最近任务/最新完成快照、当前版本、失效原因、就绪标签、队列计数 |
| `cancel_ownership_analysis(projectId, runId)` | 请求取消；排队中直接取消，运行中由 worker 在下一块前丢弃暂存并标记 cancelled |
| `page_ownership_groups(input)` | 组 cursor 分页（`g{run}:{sort_rank}`），真实 COUNT 总数、快照版本、样本带当前状态；按 lane / navBucket / region 过滤 |
| `page_ownership_group_members(input)` | 成员 cursor 分页（`m{run}:{group}:{asset}`），带快照后是否变化 |
| `preview_ownership_group_decision(input)` | 组预览：可写/跳过/冲突计数、原因、被排除样本、现有授权分布、拟改变、`expectedActionHash` |
| `apply_ownership_group_decision(input)` | 组确认：只收 `groupId` + 版本 + 哈希 + `operationId`，不收资产 ID 列表 |

## 4. 实现要点

**分析任务（§9）**：认领时签发 worker token；分块读取 `pa.asset_id > last_asset_id`，短 IMMEDIATE 事务里 upsert 组计数、插入成员、推进游标（游标、token、未取消三者同时守卫）。worker 丢失后可接管续跑，不会重复成员。续跑时如果版本已变，任务标记 failed。全部写完后发布：成员数 = 已处理数 = 组成员数合计，且每组索引计数一致；对不上就失败，不发布部分结果。完成快照只保留最近 2 份，更早的成员索引删除（`members_pruned=1`）。

**分组（§6）**：主机来自 `normalize_ownership_domain(domain|host|link)`。边界判定：
- IP 主机 → `ip`
- 主机本身是公共后缀 → `public_suffix_host`
- PSL PRIVATE 区的托管平台 → `private_suffix_tenant`，带反证 `hosting_platform`
- 档案或目录里的共享域名 → `exact_host`，带反证 `shared_domain`
- 其余 → 按 PSL 可注册域划分

`com.cn`、`gov.cn` 不会被“最后两段”合并。组键还包括：候选实体、支持强弱、反证种类、人工结论（手工结论保留状态，自动一律记为 auto）、授权状态、暴露面准入。

车道：
- consistent：强支持且无反证
- insufficient：只有弱支持
- counter_evidence：有反证
- conflict：支持与反证并存，或 `entity_mismatch` / `multiple_entities`
- unknown
- manual

反证、冲突、未知各自成组，不和一致组合并。

支持强弱：
- 强：目录官方主机精确命中、目录法定名称完全一致、已保存档案的确认域名。
- 弱：目录别名、档案别名、公司标签候选。
- 项目未绑定目录时，目录命中只算弱（`directory_unbound`）。
- CIDR/ASN 标签记为网络线索（`network_label` 反证），不当作法人。

**组确认（§8）**：
1. 先校验 `operationId` 格式，再做幂等：相同 payload 直接回放已存结果（`replayed=true`），不同 payload 返回 `OWNERSHIP_OPERATION_PAYLOAD_MISMATCH`。
2. 只有本项目最新完成快照可以操作，且当前策略/目录/绑定/范围/算法版本要和快照一致，否则返回 `OWNERSHIP_SNAPSHOT_STALE`。
3. 组版本和策略/目录/绑定版本要和预览一致，`expectedActionHash` 也要一致（哈希覆盖每个成员的当前状态），否则返回 `OWNERSHIP_PREVIEW_STALE`，不写任何东西。
4. 逐成员判定：
   - 删除/解除关联、离开复核范围、结论或授权变化、证据变化 → conflicted，不写
   - 已有人工结论 → skipped
   - 其余 → applicable

快照之后新增的资产不在成员表里，永远不会被写入。

写入规则：
- 确认归属 / 仅关联：保留每条现有的授权与准入，绝不授予。
- 第三方 / 排除：沿用既有的收窄语义，设为 denied/0。
- 写入守卫没命中时计为 failed。

每条写入都记 `asset_events`，并在 `ownership_operation_items` 里写入旧值、新值和证据引用。

动作限制：consistent 组允许全部动作；conflict / counter_evidence / insufficient / unknown 只允许收窄动作（related / third_party / excluded），不能整组确认归属；manual 组不允许组动作，只能逐条修改；违反时返回 `OWNERSHIP_GROUP_ACTION_NOT_ALLOWED`。组动作不提供撤销，撤销仍走逐条路径。

**前端（§7）**：门禁页在项目设置摘要下方新增“归属决策组队列”，原 URL 列表保留为“资产明细（组内详情与逐条修正）”。

组件 `OwnershipReviewGroups.vue` 包含：
- 顶部统计：待复核资产、待复核组、冲突组（含反证组数）、已完成组、未处理增量（快照后新增 + 变化）。
- 快照时间与版本；就绪/旧快照提示；运行中显示进度，每 1s 轮询，并提示“下方仍为旧快照”；开始/取消按钮。
- lane / 导航 / 地域筛选；组和成员都是“加载更多”式 cursor 分页。
- 组卡片：候选单位（标签候选、网络线索、未绑定目录会单独标注）、边界、成员数/主机数、理由、反证、现有权限、3–5 个样本、展开成员。
- 按 `allowedActions` 显示预览按钮；预览面板写明“拟改变：授权保持现状/拒绝，准入保持现状/否”。

请求与重试规则：
- 所有响应都按 projectId + 代次丢弃过期结果。
- 每份预览生成一个 `operationId`（`crypto.randomUUID`）；网络失败重试沿用同一个。
- 遇到 STALE / MISMATCH / NOT_ALLOWED 时清掉预览，提示重新预览，重新预览会生成新 id。
- 结果显示 applied / skipped / conflicted / failed。

门禁页原有的“确认归属并授权”逐条按钮未改。

## 5. 测试

### Rust（临时 SQLite，`cargo test --lib ownership`）

```
test result: ok. 46 passed; 0 failed; 1 ignored; 0 measured; 2892 filtered out; finished in 7.02s
```

46 个里包括 O0/O1/exposure/agent 中名字含 ownership 的既有测试。O2 新增 17 个（另有 1 个 ignore）：

- `ownership_public_suffix_tests::public_suffix_rules_do_not_merge_by_last_two_labels`
- `ownership_public_suffix_tests::private_suffixes_keep_tenant_boundaries_and_wildcards_follow_the_list`
- `ownership_review_o2_tests::analysis_run_binds_versions_and_never_starts_a_duplicate`
- `…::analysis_cancel_discards_staging_and_keeps_the_real_processed_count`
- `…::analysis_resumes_after_a_lost_worker_without_duplicating_members`
- `…::grouping_splits_by_entity_boundary_evidence_decision_and_authorization`
- `…::each_asset_has_exactly_one_primary_group_and_counts_reconcile`
- `…::group_and_member_pages_use_cursors_with_real_totals`
- `…::group_confirmation_writes_ownership_only_and_audits_every_row`
- `…::group_operation_id_is_idempotent_and_rejects_a_different_payload`
- `…::apply_rejects_a_preview_whose_group_or_policy_versions_do_not_match`
- `…::stale_preview_is_rejected_after_delete_revoke_evidence_or_manual_change`（删除、撤权、证据变化、离开范围、人工逐条结论 5 种）
- `…::members_added_after_the_snapshot_are_never_included`
- `…::conflict_counter_unknown_insufficient_and_manual_groups_cannot_be_confirmed`
- `…::snapshot_is_stale_after_policy_directory_or_newer_snapshot`
- `…::page_queries_use_the_snapshot_indexes`（EXPLAIN QUERY PLAN）
- `…::acceptance_4000_rows_reconciles_pages_and_applies_one_group`
- `#[ignore] …::acceptance_on_extracted_project_copy`（真实项目只读抽取副本，手动运行）

合成 4000 行验收（debug 构建）：

```
O2_ACCEPTANCE rows=4019 scope=3974 groups=203 lanes={"conflict": (30, 790), "consistent": (47, 1969), "counter_evidence": (21, 393), "insufficient": (51, 393), "manual": (23, 36), "unknown": (31, 393)} seed_ms=637 analysis_ms=763 group_page_p50_ms=8.22 group_page_p95_ms=10.56 pages=5 member_page_p50_ms=4.60 member_page_p95_ms=6.89 biggest_consistent=786 preview_ms=32 apply_ms=96 applied=786 compression=0.051
```

真实项目抽取副本：用 `immutable=1` 只读 ATTACH 把真实库抽到 /tmp 临时文件，在副本上运行，验收后副本已删除。
- **项目 15**：范围内 3960 条，分成 96 组（其中单例 67 组）。
  - 车道：counter_evidence 93 组 / 3411 条，unknown 3 组 / 549 条。
  - 边界：private_suffix_tenant 49、registrable_domain 29、ip 18。
  - 反证：network_label 3359 条、hosting_platform+network_label 52 条。
  - 后缀分布：ip 3267、icann com 600、private synology.me 24、myds.me、ddns.net 等。
  - 分析 706 ms，组页 p50 13.3 ms。
  - 在副本里对最大组（582 条）执行“仅关联”：写入 582、其余 0；重复提交只回放；准入不变。
- **项目 3**：范围内 14280 条，分成 3475 组（其中单例 2329 组），全部为 insufficient（该项目没有绑定目录也没有保存档案，只有标签弱支持，按可注册域拆分），压缩比 0.24。
  - 分析 3021 ms，组页 p50 8.3 / p95 23.6 ms。
  - 在副本里执行“仅关联”写入 1999 条，耗时 326 ms。

### 前端

- `node --test tools/test_ownership_review_groups.cjs`：pass 7 / fail 0
  - a late overview or group page for the previous project never fills the next project queue
  - groups and members page by backend cursor with the real total, filters restart from the first page
  - the queue shows counter-evidence, conflicts and the unprocessed increment, and conflict groups offer no confirm
  - apply submits only groupId with the preview versions and hash, and a network retry reuses the operationId
  - a stale preview is cleared and the next preview gets a new operationId
  - a late apply result for the previous project neither updates nor reloads the shown project
  - start and cancel go through the backend run; an active run polls and shows the old snapshot
- `tools/test_asset_ownership_gate.cjs`：pass 11 / fail 0；`tools/test_ownership_project_setup.cjs`：pass 6 / fail 0
- `npm run test:asset-exposure`：tests 59 / pass 59 / fail 0
- `npx vue-tsc --noEmit`：通过，无输出

### 变异验证（每项改完都已恢复，并与备份 cmp 一致）

| 变异 | 结果 |
|---|---|
| M1 去掉 operationId payload 不一致检查 | 红：`group_operation_id_is_idempotent_and_rejects_a_different_payload` |
| M2 去掉 expectedActionHash CAS | 红：`stale_preview_is_rejected_after_delete_revoke_evidence_or_manual_change` |
| M3 PSL 改成“最后两段” | 红 6 个：两个 PSL 测试 + grouping / each_asset / conflict / acceptance_4000 |
| M4 去掉车道动作限制 | 红：`conflict_counter_unknown_insufficient_and_manual_groups_cannot_be_confirmed` |
| M5 去掉 worker 的取消检查 | 第一次运行测试挂起（驱动循环无界），被我手动终止；随后把测试改为有界步数，再跑变红：`analysis_cancel_discards_staging_and_keeps_the_real_processed_count` |
| M6 去掉 groupVersion CAS | 第一次全绿（被哈希检查掩盖），据此补了 `apply_rejects_a_preview_whose_group_or_policy_versions_do_not_match`，再跑变红 |
| F1 前端去掉代次/项目守卫 | 红 2 个（项目切换后的过期 overview/page、过期 apply） |
| F2 每次提交都生成新 operationId | 红：retry reuses the operationId |
| F3 STALE 错误后保留预览 | 红：stale preview is cleared |
| F4 渲染全部动作、不按 allowedActions | 红：conflict groups offer no confirm |

### clippy

`cargo clippy --lib --tests`：新文件与本次改动的文件没有 warning。最初有两处 `type_complexity`，已用类型别名消除；其余 19 + 53 条都在既有的 agent / native / multi_agent 文件里。两个新 Rust 文件已用 rustfmt 格式化（只格式化了这两个文件）。

## 6. §12 验收条目状态

| 条目 | 状态 |
|---|---|
| 已确认目录复用、草稿不被覆盖 | O1 已完成；O2 直接读取同一份目录，没有新建目录 |
| 地名不自动归省公司、CIDR/ASN 不当法人 | 分组不按地名合并；地域只作 `region_clue` 展示和筛选；网络标签作为反证单列（测试覆盖） |
| 项目 3 切到 15 后旧请求不写新项目 | 前端代次 + projectId 守卫（测试 + 变异 F1）；后端每个 IPC 都校验 run/group 属于该项目；授权按项目隔离 |
| com.cn、多租户、同主机多单位、第三方平台不整组放行 | PSL 可注册域；private 租户、共享域名精确主机、第三方/供应商都单列；这些组不能整组确认（测试 + 变异 M3/M4） |
| 100000 行可核验、页面不持有全量 ID、样本不隐藏反证、成员可对账 | 4000 行合成与 14280 行真实副本已核验；**20k/100k 尚未测**；页面只持有当前页；反证不并入一致组，计数显示在顶部 |
| 同一批量只应用一次、旧预览拒绝、新增成员不暗中批准 | 已实现，测试 + 变异 M1/M2/M6 |
| 归属确认后主动扫描仍按授权拒绝 | 组确认不改授权/准入（测试断言前后一致）；扫描侧没改。“允许公开主体检索时不要求确认外部平台”属于 exposure 范围，本轮未动 |
| 原资产与历史保留；取消/崩溃不丢成员、不重复赋权 | 只写 `asset_ownership_decisions` + 事件 + 审计；取消丢弃暂存；心跳接管续跑不重复成员（测试）；组操作不赋权。更深的崩溃恢复见遗留 |

## 7. 需要用户决定（第 1 项已决定）

1. ~~`assets.rs` 准入筛选~~：用户已决定保留新口径（只看 `exposure_eligible=1`），本轮没有修改；O2 组确认不设置准入，与该口径一致。
2. 门禁页原有的逐条“确认归属并授权”按钮仍把归属和授权合在一步，是否拆分需要产品决定（O3）。
3. 项目 3 这类没有绑定目录的项目，压缩比较低（14280 条 → 3475 组，全部为“证据不足”）。要提升，需要用户先在项目设置里绑定目录或保存档案；本轮没有放宽成按标签合并。

## 8. O3 前遗留

- 增量水位更新：目前快照之后的新增/变化只计数提示，需要重新分析；没有做增量重分组。
- 崩溃恢复只覆盖心跳超时接管续跑；进程启动时没有自动恢复，用户需要重新点“开始分析”触发接管。
- 20k / 100k 行性能测试，以及 RSS / WAL 体积测量。
- 组操作在一个事务里完成；超大组还没做分块 operation staging。
- 组级撤销（目前只能逐条撤销）。
- 组动作与动作权限（谁可以批量）的集成。
- 前端轮询固定 1s，没有做退避。

## 9. 构建产物清理

确认没有 cargo/rustc 进程后，在 `src-tauri` 执行 `cargo clean`：`du -sh target` 清理前为 7.3G；cargo 报告 “Removed 12567 files, 14.0GiB total”（按表观大小计，包含 APFS 压缩/稀疏差异）；清理后 `target` 目录已不存在。

## 10. 真实库

只用 `file:…/oviraptor.sqlite3?mode=ro&immutable=1` 只读 ATTACH 做过抽取，抽取副本已删除。核对结果：真实库里没有任何 `ownership_*` / `org_*` 表，说明本轮代码从未在真实库上初始化或写入。该库 mtime 从 19:54 变到 20:29，与 `/Applications/Oviraptor.app`（已安装版本，常驻运行）自身的写入一致。
