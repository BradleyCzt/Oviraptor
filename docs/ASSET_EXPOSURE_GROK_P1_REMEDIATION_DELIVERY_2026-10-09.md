# 暴露面 P1 整改交付（2026-10-09）

> 依据：用户 10:50 指令。时间均为 Asia/Shanghai。仓库 HEAD 14a7875，未 commit / push / stash，未碰用户库。
> 所有开发与测试在隔离副本（独立 CARGO_TARGET_DIR）完成，结束后已删除副本与独立 target。

## 0. P1 清单来源

在 docs/ 里**没有找到**本轮新增的暴露面评审 P1 清单（只有 G4 交付 §8 的自评未满足项）。因此按指令给出的 7 条断言自审，得到下面 7 项 P1。每项都先写能复现错误的业务回归（红，且红因正确），再做最小修复，然后跑变异。

| # | 断言 | 发现的问题（修复前红测表现） | 复现测试 | 修复 |
|---|---|---|---|---|
| P1-1 | 操作对象正确 | 整改可挂别的线索的观察、别的项目的轮次（返回 Ok）；前端复测/关闭取“第一页第一条整改”，可能作用到别的线索 | `tests_g1::rem_p1_1_*`；前端 `P1-1 retest and close never act on another finding's remediation`、`P1-1 … beyond the first remediation page` | `record_remediation` 校验观察属于本线索、轮次属于本项目；`useExposureReview` 只取本线索的整改并翻页（≤25 页） |
| P1-2 | 旧版本零写 | 已关闭/驳回的整改可被重复登记或 mark 改写；复核版本核对在事务外（两连接 TOCTOU 时旧版本写入 Ok(1)） | `rem_p1_2_a_terminal_remediation_is_never_overwritten`、`rem_p1_2_a_review_checked_against_an_old_revision_writes_nothing` | 版本核对移入 `BEGIN IMMEDIATE` 事务；终态整改返回 `exposure_remediation_terminal`，零写入 |
| P1-3 | 撤权后零新请求 | 一轮计划后撤权/归档，证书/归档/代码平台等外部来源仍发出 4 个请求；撤权资产仍能建复测轮（T30） | `tests_g2::rem_p1_3_external_sources_send_nothing_after_the_scope_is_revoked_mid_round`（revoked / archived / one_revoked 三种）、`rem_p1_3_t30_a_retest_of_a_revoked_asset_starts_nothing_and_sends_nothing` | 执行器对外部来源工作项逐项重载授权范围，计划时的被动资产任一不再准入即记 `blocked_authorization`、零请求；`start_retest` 要求原资产仍在授权里，否则 `exposure_retest_unauthorized` 且不建轮 |
| P1-4 | 失败复测不可关闭 | failed 轮的复测给出 canClose=true；事件关联 403 复测后仍可登记关闭 | `rem_p1_4_a_failed_or_refused_retest_never_lets_a_remediation_close`、`tests_g4::rem_p1_4_an_incident_whose_latest_retest_failed_cannot_be_closed` | 只有 2xx 新字节或 404/410 算取得；变化字节必须来自 completed/partial 轮；事件 `closed_recorded` 若关联过复测，最近一次复测每个位置都须为内容变化或 404/410，否则 `exposure_incident_retest_failed`，零写入 |
| P1-5 | 秘密不落库 | 事件动作理由/渠道/外部编号、改判理由、导入声明/批次标签/公开材料说明、情报定级理由里的口令/token 原文落库 | `rem_p1_5_*`（g1/g3/g4 三条，全库扫描原文） | 写库前统一 `redact_text`（脱敏后再算载荷哈希，重放幂等不受影响） |
| P1-6 | 缺口不变绿 | 部分条目未配置的来源被算成 executed；来源结果为 partial 的也变绿 | `rem_p1_6_partly_unconfigured_or_partial_source_results_never_turn_green` | `row_state` 有未配置或来源结果非 completed / completed_no_matches 时为 partial |
| P1-7 | 拆分后单位证据正确 | 拆分后以被拆走位置为证据的单位关联仍留在原事件，新事件没有单位 | `rem_p1_7_split_moves_unit_links_with_their_evidence` | 拆分按证据拷贝/保留/停用单位关联（`movedUnitLinks` 进事件结果） |

每项的变异结果（全部 KILLED）：
- P1-1：REM-M1、M2、F1、F2
- P1-2：REM-M3–M5
- P1-3：REM-M6、M9、M10
- P1-4：REM-M7、M8、M15、M16
- P1-5：REM-M11、M12、M19–M21
- P1-6：REM-M17、M18
- P1-7：REM-M13、M14
- UI：REM-M22–M25、F3–F12

修复中两个旧测试（e5、g4_0）语义冲突时改的是**实现**而不是旧测试：partial 轮算“已结束”；同字节仍报 `retest_content_identical`。

## 1. 页面闭环（真实用户操作路径 + 组件交互测试）

IPC 注册不算闭环；下面每一项都是页面上的真实操作，测试用 `@vue/compiler-sfc` 编译真实 SFC，以自定义渲染器挂到假 DOM 上，触发点击/勾选/选择，断言发出的 IPC 载荷与界面状态（`tools/test_exposure_g4.cjs` 的 7 条 “P1 UI”）。

| 能力 | 页面路径 | 前端实现 | 测试 |
|---|---|---|---|
| 传播位置第 51 条以后分页 | 事件面板 → 位置列表 →「更多位置」 | `useExposureIncidents.loadMoreLocations`（50 条/页，带上一页游标，追加不覆盖） | `P1 UI propagation locations beyond the 51st are paged in by cursor…`；后端 `page_locations` 游标 |
| 人工拆分 | 位置前勾选 `data-location` →「拆分」 | `toggleLocation` / `split`：只发勾选的位置 + 读到的版本 | `P1 UI a split sends exactly the ticked locations…` |
| 人工合并 | 输入目标事件号 →「读取目标」→「合并」 | `readMergeTarget`（拒绝自身与已合并事件）→ `merge` 同时带本事件版本与目标版本 | `P1 UI a merge reads the target first…`、`… payloads are validated before any IPC` |
| 覆盖矩阵钻取 | 覆盖矩阵每行「明细」→ `ExposureCoverageDrill` | 新命令 `exposure_coverage_drilldown`；时间窗口（最近一轮 / 7 / 30 / 90 天）× 单位（含“未归属到单位”）× 只看缺口；游标“加载更多”；读取失败明示“钻取读取失败…（不是“没有缺口”）” | `P1 UI the coverage drill-down changes window, unit and gap filter…`、`P1 UI a failed drill-down says so…`、`P1 UI coverage rows carry their source key and time window…`；后端 `rem_ui_coverage_drilldown_has_window_unit_and_paged_items` |

单位维度只对按资产计划的来源（passive_web）报告，取自归属模块的逐资产只读投影；按域名/关键词计划的外部来源明确显示“不对应单个资产，不按单位分摊”，对它们按单位筛选会报错而不是返回空。

## 2. 验收证据

### 2.1 专项测试（全部在隔离副本，独立 target）

- Rust：`cargo test --lib exposure` **200 passed / 0 failed / 4 ignored**（原 183 + 本轮 17 条）；`cargo test --lib ownership_ -- --skip agent_tests` 55/0；`cargo check` 0；`cargo clippy --lib --tests` 本任务文件 0 警告（全库只有 2 条 `cmp_owned`，在 `agent_native/coordinator_budget_consumption_frame.rs`，不是本任务的文件）。
  - （`cargo test --lib ownership` 不加 skip 时有 2 个 `agent_tests::*ownership*` 失败，属 agent 模块，本任务未动 Native/agent，不处理。）
- 前端：`npm run test:exposure` **109/109**（原 102 + 7 条 P1 UI）；归属前端 14 + 6 + 12 全过；`vue-tsc --noEmit` 0。
- 真实仓库写回后：`npm run test:exposure` 109/109、`vue-tsc` 0（真实仓库未做 cargo 构建，按要求）。

### 2.2 隔离环境完整闭环（生产执行器 + 本机夹具站点）

`tests_g1::rem_acceptance_full_loop_import_discover_own_incident_retest_close`：
导入（document_import，零出站）→ 归属（组确认 → 逐资产单位投影给出杭州分公司）→ 发现（`execute_exposure_run` 真实请求授权站点，得到 `db_password` 线索）→ 事件（执行器收口时自动建议，单位关联 origin=ownership_projection）→ 人工核实 + 登记通知（零出站）→ 复测 1（新一轮真实请求得 403）关联后**不可关闭**，拒绝零写入 → 复测 2（404）关联后人工关闭，`closureBasis=manual_with_retest`，从不自动关闭。

### 2.3 撤权复测 T30

`rem_p1_3_t30_a_retest_of_a_revoked_asset_starts_nothing_and_sends_nothing`：撤销资产 A 的授权后发起复测 → `exposure_retest_unauthorized`，不建新轮，站点请求数 0。另有 `tests_g2::rem_p1_3_*` 覆盖一轮进行中撤权（全部撤权 / 归档项目 / 只撤一个资产）时外部来源零请求、记 `blocked_authorization`。

### 2.4 剩余 2 个真实 SIGKILL 场景（T37）

均为真实子进程 + `kill -9`：
- `rem_t37_sigkill_before_the_first_claim_leaves_nothing_half_done`：父进程持写锁，子进程进入生产认领循环后被杀 → 无遗留认领、无观察；新 owner 两项都做完，轮次 completed。
- `rem_t37_sigkill_after_save_before_finish_redoes_the_item_without_duplicates`：已保存观察和线索、未 finish 时被杀 → 证据仍在；判定中断后新 owner 重做该项，观察/关联/线索都不重复，轮次 completed。

### 2.5 加入逐资产投影后的规模验证（release，临时库）

测试 `tests_g4_scale::rem_scale_tier_with_per_asset_projection`（`#[ignore]`，`REM_SCALE_TIER` 指定档位）：集团台账绑定，4/5 资产有组确认单位依据；另造一个 200 处传播位置的大事件。工作项状态用 SQL 收口（不发网络，不跑合成执行器），测的是读写路径本身。

**规模测试先暴露了一个新问题**：逐资产投影是 O(n²)（`asset_events` 只有 `(project_id, created_at)` 索引，每个资产都扫一遍全项目事件）。修复：整项目投影一次读出归属事件，在内存里按与 SQL 完全相同的规则判定（ASCII 不分大小写前缀、NULL 摘要不算）；等价测试 `rem_scale_full_projection_matches_the_per_asset_edges_exactly` + 变异 3/3 KILLED。

| 指标 | 4000 修前 | 4000 修后 | 20000 修前 | 20000 修后 | 100000 修后 |
|---|---|---|---|---|---|
| 整项目投影（冷/热） | 962 / 966 ms | 20 / 20 ms | 63.9 / 51.1 s | 98 / 97 ms | 475 / 472 ms |
| 投影边 / 未归属 | 3200 / 800 | 3200 / 800 | 16000 / 4000 | 16000 / 4000 | 80000 / 20000 |
| 汇总（含投影，冷/热） | 2.21 / 2.16 s | 55 / 56 ms | 106.9 / 84.0 s | 287 / 253 ms | 2.99 / 1.67 s |
| 单位统计 | 1077 ms | 25 ms | 35.3 s | 115 ms | 619 ms |
| propose 全量（事件 / 投影关联事件） | 4.6 s（1601 / 1201） | 5.7 s（1601 / 1201） | 152.6 s（8001 / 6001） | 149.0 s（8001 / 6001） | 全量：>30 分钟后停止（预估 ≈60 分钟）；只建议 200 条新线索那一轮：602.8 s |
| 覆盖钻取 首页 / 次页 / 单位缺口 / 30 天窗口 | 1048 / 1089 / 1062 / 1115 ms | 24 / 24 / 25 / 24 ms | 35.5 / 35.9 / 44.0 / 36.4 s | 164 / 137 / 122 / 122 ms | 652 / 640 / 648 / 647 ms（单位 5 缺口 10000） |
| 事件分页 首页 / 后续 | 2.2 / ≈2 ms | 2.3 / ≈2 ms | 7.6 / ≈6 ms | 6.7 / ≈6.6 ms | 1.3 / 0.1 ms（只有大事件） |
| 大事件 200 位置：读取 / 每页 50 | 1.5 / 0.4 ms | 1.7 / 0.4–0.6 ms | 4.8 / 1.0 ms | 2.1 / 1.0 ms | 4.6 / 0.3–0.4 ms |
| 拆分 10 位置 / 合并回去 | 3.1 / 0.9 ms（带走 8 条单位关联） | 3.2 / 0.9 ms | 13.3 / 1.5 ms | 2.7 / 0.8 ms | 6.1 / 1.3 ms（带走 8 条） |
| 峰值 RSS | 46 MB | 47 MB | 73 MB | 105 MB | 359 MB |

修前 4000 档的单位缺口钻取为 0 是测试数据分布问题（阻塞项按 id 取模恰好落在别的单位），已改为按资产分布并断言 >0，修后 4000 档 400、20000 档 2000。

10 万档说明：
- 全量 propose 跑到进程 31 分钟时仍未结束，按要求停止。4000→20000 档从 5.7 s 涨到 149 s（约 n²），10 万档预估约 60 分钟。
- 其余各项改用 `REM_SCALE_SKIP_FULL_PROPOSE=1` 测：大事件放在单独一轮，只对这一轮做建议。
- 计划建立 35.9 s，数据库 360 MB，WAL 279 MB。
- **新发现**：即使只对 200 条新线索的那一轮做建议，也要 602.8 s。`propose(Some(run))` 的成本随全项目规模增长，而不是随本轮规模；见 §5。

propose 在 20000 档修前修后几乎一样：瓶颈不在整项目投影，而在 propose 本身（G4 §5 基线 100k 档 33.1 分钟）加上每条线索一次的**单资产** edge 查询（仍逐资产扫描事件，20000 档约 3.5 ms/次）。

## 3. 变异测试

- P1 + UI：37 个变异（Rust 25 + 前端 12）**37/37 KILLED**。
- 规模修复：3/3 KILLED。
- 每个都按 copy + utime 还原并核对 sha，没有留下 .mutbak。
- 清单与结果见 `~/.ovi_o3/ir/logs/muts_rem.out`、`muts_rem_proj.out`（隔离环境日志，不入库）。

## 4. 写回

- 共享文件：只有 `src-tauri/src/lib.rs`，真实文件 = T0 快照（0128ffb8…），按行插入 1 行 `commands::exposure_coverage_drilldown,`。`commands.rs` / `src/features/assets/api.ts` / `package.json` 本任务未改（真实 `commands.rs` 已被他人改过，与本任务无关，未触碰）。
- 白名单 26 个文件（含归属投影文件 ownership_unit_projection.rs），写回前逐个核对：真实文件 mtime 早于 10:51 rsync 截止或内容已相同，无 FOREIGN EDIT。第二批（投影修复 + 两个测试文件）另外核对过：真实文件是副本的前缀或可逆向还原到副本，投影文件与修前快照逐字节相同。
- 未 commit / push / stash；HEAD 仍 14a7875；未碰用户库；Native 和 agent 未动；通知/下架仍只登记、不出站。

## 5. 仍未满足 / 未验证

1. **真实 GitHub 代码搜索：未验证**。没有实际来源证据，测试只用本机夹具。
2. **需要向用户申请：给 `asset_events` 加 `(project_id, asset_id)` 索引**（建议 `(project_id, asset_id, event_type)`）。要改 `db_schema.rs`，不在本次白名单。没有这个索引，单资产单位 edge 每次都要扫全项目事件：10 万档 15.4 ms/次，propose 每条线索调用一次。
3. **propose 在大项目里慢**：全量 10 万档预估约 60 分钟；只建议一轮 200 条新线索也要 602.8 s（成本跟全项目规模走）。需要单独排查 `incidents::propose` 的全项目扫描，本轮未改。
4. **归属决定晚于事件创建时，单位关联不回溯**。完整闭环测试发现：已存在的事件要等下一轮采集再次挂位置时才会挂上单位；本轮未改，测试按“先确认归属再采集”的顺序写，并注明原因。
5. `cancel_exposure_scan` 未校验项目归属（只按 run_id），列为后续项。
6. 规模测试的工作项状态是 SQL 收口，没有跑 10 万档合成执行器（G4 §5 已有执行器档位基线）。
7. **T08 仍部分未满足**：“公开领导介绍”等其余正常公开规则没有实现（沿用 G4 交付 §8），这类线索仍落到待分类，由人工判断。
8. 原始 P1 评审清单没有找到，本轮 P1 是按 7 条断言自审得出的。

---

## 追加：FIX2 未满足项修复（2026-10-09 13:20–14:50 Asia/Shanghai）

用户 13:20 指令“接着修”。在隔离副本 + 独立 target 里完成；每项先写红测，再做最小修复，再跑变异测试；做完一项写回一次。没有 commit，没有动用户库，没有在真实仓库里 cargo build，也没有改 db_schema.rs。

### 1. propose 规模（单轮成本只随本轮增长）
- **热点**（10 万档实测加 EXPLAIN）：
  - 线索查询被规划成“按项目扫观察 × 按项目扫线索”，成本是平方级；
  - 末尾的“需复核”UPDATE 会扫全项目的事件、位置和版本；
  - 单资产投影的 `?2 IS NULL OR` 退化成全项目扫描；
  - asset_events 没有按资产的索引；
  - 自动关联里的失效 UPDATE 走的是项目索引，每条约 1.1 万步。
- **修复**：
  - LEAD_SELECT 用 `CROSS JOIN` 固定连接顺序；
  - “需复核”改为先取本轮候选（按版本和本轮已排除线索），再按主键逐个 UPDATE，条件与原来逐条相同；
  - 单资产投影改用独立 SQL；
  - 新增 `exposure_finding_versions(run_id, finding_id)` 索引；
  - 失效 UPDATE 改用 `+project_id`。
- **等价测试**：同一夹具下，修前和修后的事件、位置、单位关联、事件日志全量形态哈希一致，`ecd10d55…`。修前摘要是把真实仓库的原文件临时换进副本跑出来的，跑了两次，结果一致。夹具包含三个已核实的单线索事件：
  - 内容变化 → 标“需复核”；
  - 线索被排除 → 标“需复核”；
  - 位置已拆出 → 不标。
- **成本测试**（SQLite VM 步数，同一轮 40 条新线索）：

  | 项目规模 | 修前 | 修后 |
  |---|---|---|
  | 1000 资产 | 72,506,100 | 18,900 |
  | 4000 资产 | 1,081,584,300 | 18,900 |
  | 比值 | 14.92 | 1.00 |

  断言阈值是 1.2。
- **变异**：F2-M1..M9 全部杀死。

### 2. asset_events(project_id, asset_id) 索引
- 加在白名单文件 exposure/schema.sql：`CREATE INDEX IF NOT EXISTS idx_asset_events_project_asset ON asset_events(project_id, asset_id, event_type)`。
- 暴露面结构总在主库初始化之后执行，这时 asset_events 已由主库建好。
- 表不存在时，state.rs 只记一条警告并跳过这一条，结构初始化照常成功。
- 测试覆盖两点：逐资产查询走这个索引；DROP 掉 asset_events 后初始化仍然成功、给出警告、不建索引。
- **不需要改 db_schema.rs。**

### 3. 归属决定晚于事件创建时，单位关联回溯
- **建议时**：propose 末尾对齐单位关联，用新表 `exposure_unit_link_watermarks` 记水位（每项目一行）。
  - 以下任一情况，用整项目批量投影对齐全部活跃位置：没有水位、目录或绑定版本变了、项目类型变了、全量 propose；
  - 否则只看水位之后的归属事件和组决定明细涉及的资产；
  - 组操作处于 applying 或 blocked 时，不推进明细水位，操作完成后这些资产会再查一次。
- **读事件时**：`read_incident` 先把本事件的关联对齐，再返回。
- **失效**：不再成立的关联立即失效；项目不再是集团台账时，投影产生的关联全部失效。
- 没有改归属模块的 confirm/authorize。
- 红测覆盖：
  - 先建事件、后做组决定 → 下一轮补上，单位来自目录实体；
  - 之后人工改归属 → 读事件时立即失效；不读的话下一轮也会失效；
  - 进行中的操作完成后补上；
  - 解除绑定 → 全部失效。
- **变异**：F3-M1..M9 全部杀死。

### 4. cancel_exposure_scan 校验项目归属
- 命令改为 `cancel_exposure_scan(projectId, runId)`：
  - 先核对任务属于本项目；
  - 不属于返回 `[exposure_run_project_mismatch]`，不存在返回 `[exposure_run_not_found]`，两种情况都零写入；
  - UPDATE 本身也带上 project_id。
- 前端取消时带当前项目，涉及 exposure/api.ts 和 useExposureSurface.ts。
- 共享的 src/features/assets/api.ts 只改了 1 行；写回前核对过 T0 快照。
- 红测：用 A 项目取消 B 项目的扫描，修前返回 Ok。
- **变异**：F4-M1..M5 全部杀死，其中包括前端漏传项目的变异。

### 5. T08 其余正常公开规则
- 六类页面：公开领导介绍、公开招聘信息、公开客服联系方式、公开新闻、公开产品文档、公开广告。
- **排除条件**：标题（`<title>`/`<h1>`）里有该类字样，且正文至少有 2 个该类结构字段 → `excluded_public_material`，理由写明类别和命中的字样。
- **不排除**：
  - 只有标题，或只有正文字段 → 不排除；
  - 公开页面里出现非公开字样（通讯录、工号、身份证、内部资料等）→ 待分类；
  - 同一页面里有凭据或业务源码图 → 待分类（混合）。
- 招采规则和文件名规则仍然优先。
- 分类策略版本 v1 → v2。
- **变异**：F5-M1..M8 全部杀死。

### 6. GitHub 真实代码搜索
仍然**未验证**，本轮没有动。

### 门禁（隔离副本）
- cargo clippy -D warnings：0
- cargo test exposure：207/207
- npm test:exposure：110/110
- vue-tsc：0 错误
- `cargo test ownership`：55 过、2 败。2 个失败都是他人的 agent 表 `agent_http_budget_receipts`（WITHOUT ROWID 表上 `ORDER BY rowid`），与本任务无关。

### 10 万档复测（release）

| 项目 | 修前 | 修后 |
|---|---|---|
| 全量 propose | 超过 30 分钟（已中止） | **11.9 s**：40,001 事件、50,200 位置，30,001 个事件由投影关联单位 |
| 单轮 propose（200 条新线索那一轮） | 602.8 s | **0.43 s** |

修后其余各项：
- 投影：冷 763 ms，热 668 ms；单资产 edge 0.2–0.4 ms；
- 汇总：1.2–1.5 s；单位统计 0.5–0.6 s；
- 钻取：约 0.64 s/页；
- 事件分页：40,001 个事件时约 31 ms/页；位置页约 5 ms；
- 拆分 3.5 ms，合并 0.8 ms；
- 计划构建约 35 s；
- 测试进程 RSS：690 MB（全量 propose），538 MB（单轮）。

**新增代价**：读一个大事件（200 个位置）从 4.6 ms 变为 33–40 ms，原因是读取时要逐位置对齐单位关联。

### 仍未满足 / 需留意
- GitHub 真实代码搜索：未验证。
- 读大事件时的单位关联对齐与位置数成正比；如果需要，可以改成批量投影。
- 资产被移入回收站不产生归属事件，增量刷新看不到；靠读事件时对齐和全量 propose 收口。
- 规模测试的工作项状态仍由 SQL 收口，没有跑合成执行器。
- 原始 P1 清单仍未找到。
- 他人在 13:57 修改了 db_schema.rs，增加了一行 agent include。本任务没有写这个文件，也不冲突。

## FIX3（2026-10-09 17:29–18:10 Asia/Shanghai）：R1–R8 场景修复、两项 P2、秘密原值全量扫描、完整闭环

**原始清单**：在仓库根和全仓 14:40 之后新增/修改的 md/txt、docs/progress.md、git 未跟踪文件、~/Desktop 和 ~/Downloads 当天评审文件里都**没有找到** R1–R8 / P2 原始清单。下表的 R1–R6 按用户点名的六个场景，R7/R8 为自查补充，P2 取上一轮未满足项。

**方法**：每项先写红测，再做最小修复，然后用变异（把修复改回去）证明红测会失败。变异共 15 个，全部 KILLED。另有 2 个扫描器灵敏度变异，也全部 KILLED。全部工作在隔离副本和独立 target 里进行；真实仓库只做写回，不在其中构建。

### 逐项验收表

| 项 | 场景 | 红测 | 最小修复 | 修后证据 |
|---|---|---|---|---|
| R1 | 同项目详情竞态 | node：`FIX3-R1 opening incident 5 then 6…`、`FIX3-R1 opening finding 3 then 4…`（乱序返回）；rust：`fix3_r1_review_reads_ownership_inside_the_write_transaction`（另一连接持锁改归属） | useExposureIncidents.open / useExposureEvidence.openEvidence 加专用票号，旧响应丢弃；remediation `review_scoped` 的归属/原状态读取移进 ImmediateTx | 通过；变异 F3-R1a/b/c KILLED |
| R2 | 两名复核员同版本提交 | rust：`fix3_r2_two_reviewers_on_the_same_version_only_one_wins`（后者冲突，事件数与 total_changes 不变）；node：`FIX3-R2 …newest review event…`、`…same-version conflict is shown as not saved` | `review_scoped_cas(expected_review_event)` 在事务内比对最新复核事件；命令 `review_exposure_finding_scoped` 新增 `expectedReviewEvent`；复核轨迹返回 `eventId`；前端带上读到的最新事件 | 通过；变异 F3-R2b/R2f KILLED。事件动作的 CAS 原本已在同一写事务内（rem_p1 系列覆盖） |
| R3 | 项目切换 | node：`FIX3-R3 switching project while the evidence read is in flight…`、`FIX3-R3 the legacy source fallback also carries the current project…` | useExposureReview 的丢弃判断比较 projectId；旧命令 `list_exposure_source_results` 必须带 `expected_project_id`，走 `read_exposure_source_results` 归属校验；前端回退和共享 `src/features/assets/api.ts`（单行补丁）都带上项目 | 通过；变异 F3-R3a/b KILLED |
| R4 | 项内撤权 | rust：`fix3_r4_revoking_while_an_external_request_is_in_flight_discards_it_and_sends_nothing_more`（夹具在回复前撤权） | 外部来源适配器的停止条件改为“取消或授权失效”（每页/每请求前重读）；本项结束后再核一次，失效则丢弃本项在途结果、记 `blocked_authorization`，请求照实记账；passive_web 同样复核；夹具新增 `start_hooked` | 撤权后外部请求只有那 1 个在途请求，之后为 0；外部来源观察/待归并为 0；全部外部工作项显示“授权阻塞”，理由含“本项在途结果已丢弃”。变异 F3-R4 KILLED |
| R5 | 新增位置后的关闭 | rust：`fix3_r5_a_new_location_after_the_retest_blocks_closing_until_a_new_retest` | `closable_after_retest` 要求当前每个活跃位置都在最近一次复测的覆盖内，且没有复测轮之后的新观察；否则返回 `[exposure_incident_retest_stale]` | 旧复测关闭被拒、零写入；新复测覆盖两个位置（404）后可以关闭。变异 F3-R5 KILLED |
| R6 | 脱敏碰撞重放 | rust：`fix3_r6_a_redaction_collision_is_a_different_payload_not_a_replay`（事件动作）、`fix3_r6_an_import_with_a_colliding_redacted_declaration…`（导入） | `redaction::idempotency_hash`：脱敏改写过文本时，把原文的不可逆摘要并入幂等哈希；未改写时与旧哈希完全一致，已有 operationId 的重放不受影响。用于事件动作、改判、导入批次、情报改级 | 碰撞时冲突、零写入；真重放返回首次结果；首次、重放、冲突响应都不含原值。变异 F3-R6 KILLED |
| R7（自查） | 整改状态并发 | rust：`fix3_r7_a_remediation_rejected_meanwhile_cannot_be_overwritten` | `mark_remediation` 的终态检查、复测依据、写入放进同一 ImmediateTx；UPDATE 带“状态仍是读到的值”守卫 | 已驳回的终态不被覆盖。变异 F3-R7 KILLED |
| R8（自查） | 人工录入线索的秘密落库 | rust：`fix3_r8_a_manual_finding_is_stored_redacted_and_distinct_secrets_stay_distinct` | `save_exposure_finding` 拆出 `save_exposure_finding_in`，标题/地址/摘录落库前脱敏；指纹用 `idempotency_hash` | 库内无原值；两条秘密不同的线索不被合并；同一输入仍是同一条。变异 F3-R8a/b KILLED |
| P2-1 | 大事件读取对齐批量化 | rust：`fix3_p2_1_reading_a_large_incident_loads_the_directory_once` | 新增 `ownership_asset_unit_edges`（目录/实体/绑定只读一次）；read_incident 对齐与增量 propose 改用批量取边 | 40 个位置的事件、目录增加 3000 个实体时：读事件的 VM 步数增量 135000，单次取边增量 45000，比值 3.0（修复前约 40 倍）；批量结果与逐个取边一致。变异 F3-P2-1 KILLED |
| P2-2 | 回收站资产对增量刷新不可见 | rust：`fix3_p2_2_recycle_bin_moves_are_seen_by_the_incremental_refresh` | 增量水位并入回收站集合摘要（走 `idx_project_assets_project_deleted`），集合变化时整项目对齐一次 | 进回收站后单位关联失效，恢复后补回。变异首次 SURVIVED：所选资产被第二轮夹具重新触碰，改选未被触碰的资产后 KILLED |

### 秘密原值扫描

测试 `fix3_full_business_loop_with_sentinel_secrets_leaves_no_original_anywhere` 放了 6 个哨兵秘密，来源分别是：站点内容（JSON 口令、Bearer）、人工理由/渠道/外部编号、脱敏后相同的孪生秘密、导入来源声明、导入文件正文。

每个秘密检查 11 种形态：明文、URL 编码（大小写）、hex（大小写）、标准与 URL 安全 base64 各 3 种字节对齐。

扫描范围：
- 临时库全部 211 张表的每一个文本/BLOB 字段，共 707 格；
- 17 份首次、重放、冲突响应；
- findings 和 ledger 两份导出文件。

结果：**0 命中**。

扫描器灵敏度：去掉事件理由的脱敏（F3-SCAN），或让 `redact_text` 放过哨兵（F3-SCAN2），扫描都会失败（KILLED）。

### 完整业务闭环（隔离环境）

上面这条测试同时就是闭环，全部经真实入口，真实 localhost 授权站点，生产 worker `execute_exposure_run`，流程依次为：
1. 采集 → 线索 → 事件建议 → 读事件/证据；
2. 事件复核（重放、碰撞冲突）；
3. 两名复核员同版本复核（一成一败）；
4. 整改 → 复测轮（404）→ 按依据关闭整改；
5. 事件关联复测 → 登记关闭（manual_with_retest）；
6. 导入（重放）；
7. 人工改判（重放）；
8. 导出 findings 和 ledger。

### 复跑结果（18:04–18:06）

- `cargo clippy --lib --tests -D warnings` 通过；
- `cargo test --lib exposure --skip agent_tests` 218 通过；
- `cargo test --lib fix3_` 11 通过；
- `rem2_` 6 通过；
- `npm run test:exposure` 116/116；
- `vue-tsc` 0 错。
- `cargo test --lib ownership`：56 通过、2 失败，失败的是 tests_scan_deletion 两条，原因是 agent 表 `agent_http_budget_receipts`。这是已知的无关问题，本轮没有改动。

### 写回与边界

- 共写回 22 个白名单文件，写回前逐一核对：文件在 17:30 之后没有被他人修改，否则拒绝写回。
- 共享文件只改了 `src/features/assets/api.ts` 一行：先按 T0 sha 91eab291 校验，新 sha 2044b69a。
- lib.rs、commands.rs、package.json、db_schema.rs 与 T0 一致，没有改动。
- 没有新增命令，没有动 Native/agent，没有 commit，没有碰用户库。

### 证据范围说明

- **真实 GitHub 未验证**：本轮的外部来源都是本地夹具。
- **十万档规模本轮未重验**：沿用 FIX2 的结果（全量 11.9s，单轮 0.43s）。本轮的规模类证据只有 P2-1 的 VM 步数对比，以及 rem2_ 规模回归。

### 剩余说明

- R4 无法撤回已经发出的那一个在途请求，只能保证它的结果不落库、之后零新请求。
- R6/R8 的幂等哈希和指纹里含原文的 SHA-256 摘要。摘要不可逆，原值不落库；但低熵秘密理论上可被猜测比对。如需更强保护，可改用带本机密钥的 HMAC（本轮未做）。
- URL userinfo 口令里未编码的 `/` 不会被 URL 凭据规则识别。真实 URL 中它必须百分号编码，编码形态已覆盖。

## FIX4（2026-10-09 20:37 指令；21:07 起按用户贴出的原六项清单执行，Asia/Shanghai）

- 编号严格沿用原清单。FIX3 自行替换的编号不作为原问题已关闭的依据。
- 环境与约束：
  - 隔离副本 + 独立 target；
  - 未 commit；未碰用户库；未在真实仓库 cargo build；
  - Native / agent / db_schema.rs 未改。
- 每项都是“修前失败输出 → 最小修复 → 回归 + 变异”。

| 原编号 | 失败复现（修前输出） | 修复 | 回归证据 |
|---|---|---|---|
| P1／R4 设置草稿跨项目提交 | `useExposureSettings` 测试：A=7 已读并开私网出口、填文档 URL，切到 B=8（读取中）后保存。实际提交 `[8, 3, true, [{url:'https://a.example.test/public.pdf',…}]]`，即把 A 的版本 3、私网出口和文档配置写到了 B。切换后尚未读取或 B 读取失败时，`save()` 仍返回 `true` | `useExposureSettings.ts`：读取成功时把草稿绑定到项目（`draftProject`）。重新读其他项目时先解绑；加载中或草稿归属与当前项目不一致时，`save()` 拒绝并提示，不发请求 | 设置测试 10/10 绿（新增 2 个）：加载中提交为空；B 读完后只提交 `[8, 9, false]`（B 自己的版本和配置）；切换后未读、读取失败都拒绝。<br>变异：R4a 去掉守卫、R4b 只查加载中不查归属 → 均杀死 |
| P1／R8 导入元数据保存秘密原文 | 全库 + 响应 + 导出扫描（每表每列按声明类型；TEXT/BLOB 解码：gzip/zlib/brotli、两层百分号、JSON/HTML 转义；JSON 递归取键和值；导出文件名）。<br>修前 52 处命中，例如：<br>• `exposure_import_batches.operator`（operator 原文）；<br>• `.declared_url` 和 `.report_json`（URL `access_token` 原文）；<br>• `.files_json`、`exposure_work_items.entity/reason`、`exposure_source_results.error`、`exposure_observations.url/canonical_url`、`exposure_findings.source_url`、`exposure_analysis_cache.canonical_url`、`export findings/ledger`（文件名原文）；<br>• `response import_first`（URL token、operator、文件名） | `imports.rs` 校验阶段：<br>• operator 用 `redact_text`；<br>• 文件名用 `redact_text`；<br>• 声明 URL 规范化后用新增的 `redaction::redact_url` 打码，例如 `access_token=SNT***h8(len=25)`；<br>• 三者原文只以摘要进入 `idempotency_hash`，原文不同、脱敏后相同的载荷不会互相重放 | 修后 `FIX4_R8_SCAN tables=214 columns_by_type={TEXT:1669, INTEGER:579, BLOB:21, REAL:2, (none):2} text_cells=326 json_cells=27 nested_strings=550 responses=4 secrets=5 failures=0`。<br>首次和重放响应都已扫描；脱敏碰撞的另一 operator 判为冲突；FIX3 闭环扫描仍是 0。<br>扫描器自检测试：gzip BLOB、嵌套 JSON、百分号、JSON 转义都能发现。<br>变异：R8a operator、R8b 声明 URL、R8c 文件名、R8scan 关 URL 查询串打码（扫描器灵敏度）→ 均杀死 |
| P1／R3 覆盖矩阵错误变绿 | `coverage_matrix(…, beyond_plan=900)`，完成 1 项：`完成 1 项、计划外 900 项仍有 8 个绿格子：[internal_business, technical_network, source_code, credential, employee_data, customer_data, storage_misexposure, unclassified]（行状态 "executed"）`。<br>一条 `classification_pending` 的凭据线索，其格子是 `executed_no_hit` | `coverage.rs`：<br>• 被动网页有计划外待查项时，行状态由 `executed` 改为 `partial`，并在原因里写明“另有计划外 N 项未执行，是缺口”；<br>• `classification_pending` 与 `risk_candidate` 一并计入候选，格子显示“候选”（amber） | `fix4_r3_…` 绿：0 个绿格子，行非 executed，原因含 900；对照组（计划外 0）仍是 executed；待分类凭据格不是 `executed_no_hit`，也非绿。<br>原 `g4_1_coverage…` 仍绿。<br>变异：R3a 待分类不计入、R3b 计划外不参与 → 均杀死 |
| P1／新增 撤权时丢弃整个工作项的观察 | 本地假 GitHub 两页，第 1 页请求到达时撤权（发出时授权有效）。<br>修前：`第 1 页观察保留: left 0 right 30`（FIX3 用空结果替换了整个工作项） | `exposure.rs`：撤权后不再替换结果。<br>• 已取得的观察保留（落库仍走原有脱敏），逐条在证据里加 `scopeGrant{verifiedAt, withdrawnDetectedAt, basis}`；<br>• 请求和费用照实记账；<br>• 新发现不入队；<br>• 本项 `blocked_authorization`，来源 `partial`；<br>• stop 闭包在每页前复核授权，撤权后零请求 | `fix4_new_t21_…` 绿：第 2 页零请求（总请求 1）；31 条观察都带授权时间点；工作项原因含“已保留”；`source_results=partial`（不变绿）；线索无 `confirmed/verified`，事件无 `verified`（禁止自动确认）。<br>FIX3-R4 测试按新语义更新后仍绿。<br>变异：T21a 恢复丢弃、T21b 来源标 completed → 均杀死 |
| P2 GitHub 末页未记额度耗尽 | 原分页函数 `walk_search` 接本地假 GitHub：末页 `x-ratelimit-remaining: 0`，下一查询仍发出（`短末页: 末页已报额度 0，后续不再发请求 left 1 right 0`：两页之后下一查询又发 1 次；修后两页即止）。<br>末页前 403 + reset 时，修前没有截断标记：`github_rate_limited_403:…额度重置于 2026-10-03 12:00:00…；已读 1 页，记录 30 个仓库（已保存）` | `adapters/github.rs`：<br>• 额度更新移到提前退出之前：任何成功页报 `remaining=0`，都记下该能力额度耗尽（含重置时间）；当前项仍按完整结束，不标截断；<br>• 末页前被拒（403/429）时写明“未读尾部 N 条（截断）” | `fix4_p2_github_last_page…` 绿：短末页、满末页都是 `completed/completed`，结果全保留，无截断或限流标记；下一查询 0 请求，`BlockedQuota/RateLimited`，带重置时间。<br>`fix4_p2_github_quota_gone…` 绿：第 1 页 remaining=0、第 2 页 403、第 2 页 429 三种都保留 30 条，`BlockedQuota/RateLimited`，含“未读尾部 15 条”和重置时间，之后零请求。<br>变异：GHa、GHb → 均杀死 |
| P2 CSV 三列错位 | 实际生成的线索 CSV，按表头取列：`("需复核","oviraptor.exposure.rules.v1"), ("规则版本","oviraptor.exposure.redaction.v2"), ("脱敏版本","2")`（2 为需复核计数） | `export.rs`：`splice(9..9)` 改为 `splice(10..10)` | `fix4_p2_csv_…` 绿：解析真实导出文件，14 个旧表头逐列与库值或常量相等，包括需复核=2、规则版本、脱敏版本、首/末次发现。<br>变异：CSVa 回到 9..9、CSVb 改 11..11 → 均杀死 |

### 汇总

- 定向测试 31/31 绿；变异 14/14 杀死。
- `cargo test --lib exposure -- --skip agent_tests`：225 passed / 0 failed。
- `npm run test:exposure`：118 pass / 0 fail。
- `vue-tsc`：exit 0。
- clippy `-D warnings`：本轮 12 个文件无告警；失败项都在其他会话进行中的 agent / multi_agent 文件，见 N7。
- 写回 12 个文件：useExposureSettings.ts、test_asset_exposure_settings.cjs、imports.rs、redaction.rs、coverage.rs、export.rs、exposure.rs、adapters/github.rs、fixture.rs、tests_g1.rs、tests_g2.rs、tests_g4.rs。
- 共享文件（lib.rs、commands.rs、api.ts、package.json、db_schema.rs）未改。

### 语义变更（需知）

- 撤权：FIX3-R4 的“在途结果整项丢弃”改为“撤权前已按授权取得的观察保留并标注授权时间点”。FIX3“剩余说明”中“只能保证它的结果不落库”一句由本节取代。
- 覆盖矩阵：单元格的 `riskLeads` 计数现在包含 `classification_pending`。
- 导入：`operator`、`declared_url`、文件名以打码形态保存和返回。导入声明 URL 不再保留可直接访问的 token。

### 新增问题（只记录，未修）

- N1 复核面板（`useExposureReview.ts` / `ExposureReviewPanel.vue`）跨项目写入。组件测试修前输出：
  - 项目 7 已读后切到 8（读取中），改判发出 `["reclassify",8,{findingId:41, expectedClassificationId:5}]`；
  - 迟到的旧读取收尾后恢复按钮，复核发出 `["review",8,42,2,…,[107],0]`；
  - 新项目读取失败后仍可写。
  
  本轮试做的修复和测试已撤回，存档在 `~/.ovi_o3/ir/f4_s1_shelved/`。
- N2 `review_scoped_cas` 不校验引用观察的归属：B 的线索带 A 的观察，返回 `Ok(0)` 并写入。
- N3 `mark_remediation` 关闭后的系统复核有两个问题：
  - 把 `finding_id` 当观察引用写入；
  - 在主事务提交后才执行，失败时返回错误，但整改已经关闭。
- N4 被动采集的观察 URL 里，凭据型查询参数（如 `?access_token=`）以原文写入多处：observations url/final_url/canonical_url、findings.source_url、analysis_cache、incident_locations，以及证据/事件响应（扫描修前 38 处）。不在 R8（导入元数据）范围内，本轮未改。
- N5 Set-Cookie / X-Api-Key 等响应头在被动采集流程里没有写入观察 headers，所以关掉 `redact_json` 键名打码的变异在该流程里存活。头部保存策略本轮未审计。
- N6 `useExposureIncidents.act` 未校验详情所属项目与当前项目是否一致；目前依赖切项目时 `load()` 先 reset。
- N7 环境：`cargo clippy --lib --tests -D warnings` 在同步进来的 agent / multi_agent 文件上报 dead_code（其他会话进行中的改动，非本轮文件）。本轮文件无 clippy 告警。

### 证据范围

- GitHub 只用本地假服务器验证，真实 GitHub 未验证。
- 十万档本轮未重验。
- 日志在 `~/.ovi_o3/ir/logs/f4_*`。
