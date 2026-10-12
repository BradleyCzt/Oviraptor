# 资产暴露面（Grok）现状总结：已完善 / 未完善 — 2026-10-10

> 用途：后续工作的起点清单。本文只汇总已有交付文档与 `~/.ovi_o3/ir/CHECKPOINT.md` 里写明的事实，不新增结论，也不改代码。时间均为 Asia/Shanghai。
> 来源缩写：
> - **SPEC** = `docs/ASSET_EXPOSURE_GROK_IMPLEMENTATION_SPEC_2026-10-08.md`
> - **G0 / G1 / G2 / G3 / G4** = `docs/ASSET_EXPOSURE_GROK_G{0..4}_DELIVERY_*.md`
> - **IR** = `docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md`
> - **REM** = `docs/ASSET_EXPOSURE_GROK_P1_REMEDIATION_DELIVERY_2026-10-09.md` 的 P1 主体（§0–§5）；**FIX2 / FIX3 / FIX4** = 同文件各追加节
> - **O3** = `docs/NEST_OWNERSHIP_GATE_O3_DELIVERY_2026-10-08.md`（含 IR-02/IR-11 追加段）
> - **CP** = `~/.ovi_o3/ir/CHECKPOINT.md`

---

## 1. 一页概览

| 项 | 现状 |
|---|---|
| 功能范围 | 集团 / 单位项目的**被动**暴露面发现：授权站点被动访问、证书透明（crt.sh）、网页归档（Wayback）、GitHub 仓库元数据与代码搜索（代码搜索需凭据）、显式文档 URL、文件导入、公开情报导入与人工定级；分类 / 排除规则；按资产的单位投影；事件（多位置）合并拆分；复核、整改、复测、关闭；覆盖矩阵与钻取；实时状态；导出与导出复核；脱敏与秘密保护；授权与撤权；崩溃恢复；4000 / 20000 / 100000 三档规模。通知 / 下架只登记人工动作，不对外联系（G4 §8.3）。G4 后本专项按规格停止，不启动模型 / Agent / Master 改造（G4 §7）。 |
| 当前 HEAD | `14a7875`（分支 `nest/master-checkpoint-2026-10-05`，O3）。本文写完后再次核对，仍为 `14a7875`。 |
| 提交状态 | 全部工作**未 commit / push / stash**（REM §4、FIX2–FIX4、CP 每段结尾）。工作区还混有其他会话（归属 O0、agent / multi_agent、db_schema.rs 等）的未提交改动，见 §4。 |
| 最新测试数字（FIX4，2026-10-09 21:22，隔离副本） | `cargo test --lib exposure -- --skip agent_tests` **225 passed / 0 failed**；`npm run test:exposure` **118 pass / 0 fail**；`vue-tsc` exit 0；FIX4 定向 31/31；变异 14/14 杀死；clippy `-D warnings`：本轮 12 个文件无告警，失败只在其他会话的 agent / multi_agent 文件（N7）。 |
| 归属侧最新数字 | O3 追加段：`cargo test --lib ownership` 64 passed / 3 ignored（隔离副本）；FIX3 复跑时 `cargo test --lib ownership` 56 过、2 败，2 败都是他人 agent 表 `agent_http_budget_receipts`（WITHOUT ROWID 上 `ORDER BY rowid`），与本专项无关。 |
| 规模最新数字 | FIX2（release，临时库，10 万档）：全量 propose 11.9 s、单轮 propose 0.43 s；FIX3 / FIX4 **未在最新代码上重验十万档**。 |
| 外部真实验证 | 只有 G0 的 crt.sh / Wayback / GitHub 匿名探测、G2 的 3 次匿名仓库搜索、G3 的 2 次公开文档读取。**GitHub 代码搜索从未真实验证。** |
| 条数 | 已完善 **50** 条（§2）；未完善或仅部分满足 **41** 条（§3）。 |

---

## 2. 已完善的功能

判定口径沿用 G4 §8：有可运行代码路径 + 测试（或实测）证据才算。fixture 证据不算真实外部验收，凡只有 fixture 的已在句末注明。

### 2.1 采集来源

- **C-01 被动网页（passive_web）闭环**：对授权站点做真实 HTTP 采集，走生产执行器 `execute_exposure_run`。证据：G1 §6，`g1_4_the_closed_loop_runs_on_a_real_authorized_site`（127.0.0.1 真实 HTTP）。
- **C-02 证书透明 / 网页归档经生产执行器可测**：CT / Wayback 端点可注入，localhost 正负向走同一路径（IR-09 已关闭）。证据：G2 §3，`tests_g2`（CT / Wayback / GitHub 生产执行器 → fixture）；G0 外部探测 crt.sh 200、Wayback 200。
- **C-03 GitHub 仓库元数据搜索**：默认不启用，需在设置勾选；匿名、只读。证据：G2 §6，`g2_live_github_anonymous_repository_search_once`（3 次真实请求，最终 30/30）。
- **C-04 GitHub 凭据引用端口（IR-04）**：只读 macOS 钥匙串（服务名 `oviraptor.exposure.github`），配置只存引用名，token 不进库 / 日志 / 配置。证据：IR「P1 落地记录 · IR-04」；**只有 fixture 验证**。
- **C-05 GitHub 状态码与额度如实展示**：缺凭据 / 401 / 403 / 429 / 422 / 503 分别展示（T11），每次 HTTP 尝试都扣额度（T17）。证据：G4 §8.2 T11、T17，`q2_budget_is_charged_before_the_socket_*`。
- **C-06 GitHub 末页额度为零与截断标记**：任何成功页报 `remaining=0` 都记额度耗尽、后续零请求；末页前 403 / 429 写明“未读尾部 N 条（截断）”。证据：FIX4 P2 GitHub，`fix4_p2_github_last_page…`、`fix4_p2_github_quota_gone…`（本地假 GitHub）。
- **C-07 显式文档 URL（document_link）**：项目级精确地址准入，走生产出站门（不开内网、默认端口、不跟跳转）。证据：G3 §6，`g3_live_public_documents_are_read_once_each`（2 次真实只读请求）。
- **C-08 文件 / 情报导入与人工定级生产入口（IR-14）**：注册 `import_exposure_documents`、`grade_exposure_intel`，设置页导入表单、证据包定级表单可用；声明 URL 只记录不访问。证据：G4「P0 落地」、IR「P0 落地记录 · IR-14」（前端 97 通过、变异 8/8）。
- **C-09 出站安全**：DNS 私网 / metadata / 非默认端口拒绝（T14），恶意 URL / 跨 host 重定向不泄 token（T13，fixture）。证据：G4 §8.2 T13、T14。
- **C-10 不支持的来源明确标缺口**：PDF / Office / OCR、文库 / 网盘、暗网在能力表标为 unsupported，读取时只记元数据，导入时拒绝。证据：G3 §3；G4 §8.1“G3：PDF / 文库 / 暗网明确缺口”。

### 2.2 分类 / 排除规则

- **C-11 招采公告与转载排除且可审计（T07）**。证据：G4 §8.2 T07（G1-3 / G3-3）。
- **C-12 正式开源账号名单与第三方 vendor 资源排除（T08 前半）**。证据：G4 §8.2 T08（G2、G1-3）。
- **C-13 六类正常公开页面排除（T08 其余规则）**：领导介绍、招聘、客服、新闻、产品文档、广告，需“标题字样 + 正文 ≥2 个结构字段”；混入非公开字样或凭据时仍为待分类；分类策略 v1 → v2。证据：FIX2 §5（变异 F5-M1..M8 全杀死，测试在 `tests_g1.rs`）。
- **C-14 混合页 / 来源冲突保持待分类（T09）**。证据：G4 §8.2 T09。
- **C-15 人工改判（IR-07）**：新命令 `reclassify_exposure_finding`，CAS（expectedClassificationId）、operationId 幂等、4–500 字理由、审计表 `exposure_reclassifications`。证据：IR「P1 落地记录 · IR-07」（变异 17/17）。

### 2.3 归属与单位投影

- **C-16 省 / 专业子公司 / 城市父子关系分开（T01）**，单位未知与专业公司不误分省。证据：G4 §8.1、§8.2 T01，`g4_1_multi_unit_*`。
- **C-17 按资产的只读单位投影（IR-02 / IR-11）**：只认人工确认 + 绑定单位，或已完成的目录组决定；候选组、自动判断不计入，不按名称 / 区划码推断。证据：O3 追加段（归属 4 个测试 + `tests_g4` 端到端 1 个，变异 17/17）。
- **C-18 投影规模修复**：整项目投影改为一次读出、内存判定，消除 O(n²)。证据：REM §2.5，`rem_scale_full_projection_matches_the_per_asset_edges_exactly`（变异 3/3）。
- **C-19 `asset_events(project_id, asset_id, event_type)` 索引**：放在白名单 `exposure/schema.sql`，缺表只警告，不改 db_schema.rs。证据：FIX2 §2。
- **C-20 归属决定晚于事件时单位关联回溯**：propose 末尾按水位对齐，`read_incident` 先对齐本事件再返回，不再成立的关联立即失效。证据：FIX2 §3（变异 F3-M1..M9）。这也覆盖了 IR-02 追加段遗留的“已失效的投影链接在下次建议前仍显示 active”。
- **C-21 回收站资产对增量刷新可见**。证据：FIX3 P2-2，`fix3_p2_2_recycle_bin_moves_are_seen_by_the_incremental_refresh`。
- **C-22 弱标签 / 候选组不当确认，目录 / 绑定变更后旧计划不静默换范围（T02、T03）**。证据：G4 §8.2 T02、T03。
- **C-23 归属门禁 O3 能力**：增量分析水位、崩溃恢复（惰性启动钩子）、大组分块写入与互斥、组回退不恢复授权、撤权两步、确认与授权按钮拆分、轮询退避。证据：O3「实现」「测试结果」，如 `startup_recovery_takes_over_an_interrupted_analysis_with_real_counts`、`group_revert_restores_ownership_facts_but_never_authorization`（变异 17/17）。

### 2.4 事件合并拆分

- **C-24 多位置去重成事件**：同一全文在多个地址合成一个事件、多个位置。证据：G4 §6，`g4_0_the_production_worker_proposes_one_incident_for_the_same_document_at_two_urls`、`g4_0_identical_full_text_*`。
- **C-25 截断不称全文相同，同标题不同内容不合并（T20、T23）**。证据：G4 §8.2，`g4_0_truncated_snippets_*`。
- **C-26 多省涉及只计集团唯一事件一次（T24）**。证据：`g4_1_multi_unit_incidents_count_once_at_group_and_province_level`。
- **C-27 事件台账生产入口（IR-15）**：注册 5 个事件台账命令，面板可读、可补建议。证据：G4「P0 落地」。
- **C-28 页面上的人工拆分 / 合并与位置分页**：勾选位置拆分；先读目标再带双版本合并；第 51 条以后游标分页。证据：REM §1，`P1 UI a split sends exactly the ticked locations…`、`P1 UI a merge reads the target first…`、`P1 UI propagation locations beyond the 51st are paged in by cursor…`。
- **C-29 拆分时单位关联跟着证据走**。证据：REM P1-7，`rem_p1_7_split_moves_unit_links_with_their_evidence`。
- **C-30 propose 单轮成本只随本轮增长**。证据：FIX2 §1（VM 步数比值 14.92 → 1.00，等价哈希 `ecd10d55…` 一致，变异 F2-M1..M9）。

### 2.5 复核 / 整改 / 复测 / 关闭

- **C-31 版本 CAS 与真实并发只有一方成功（T25）**，包括复核版本核对移入 `BEGIN IMMEDIATE`。证据：`g4_0_two_writers_with_the_same_revision_one_wins`；REM P1-2 `rem_p1_2_a_review_checked_against_an_old_revision_writes_nothing`。
- **C-32 同 operationId 异载荷拒绝（T26）**。证据：`g4_0_cas_replay_*`。
- **C-33 两名复核员同版本只有一方成功**。证据：FIX3 R2，`fix3_r2_two_reviewers_on_the_same_version_only_one_wins`。
- **C-34 整改只作用于本线索、终态不可覆盖**：观察 / 轮次归属校验，终态零写，`mark_remediation` 并发守卫。证据：REM P1-1 / P1-2（`rem_p1_1_*`、`rem_p1_2_a_terminal_remediation_is_never_overwritten`）；FIX3 R7 `fix3_r7_a_remediation_rejected_meanwhile_cannot_be_overwritten`。
- **C-35 失败复测不可关闭**：只认 2xx 新字节或 404/410；失败轮、403 复测不能关闭。证据：REM P1-4，`rem_p1_4_a_failed_or_refused_retest_never_lets_a_remediation_close`、`tests_g4::rem_p1_4_an_incident_whose_latest_retest_failed_cannot_be_closed`；G1 `g1_4_a_retest_that_cannot_fetch_the_url_is_not_a_content_change`。
- **C-36 新增位置后旧复测不能关闭**。证据：FIX3 R5，`fix3_r5_a_new_location_after_the_retest_blocks_closing_until_a_new_retest`。
- **C-37 内容 / 规则变化保留旧结论并提示复核（T27），从不自动关闭（T29）**。证据：`g4_0_a_new_round_attaches_*`、`g4_0_handling_*`。
- **C-38 完整业务闭环**：导入 → 归属 → 发现 → 事件 → 核实 / 登记 → 复测 403 不可关 → 复测 404 后人工关闭（`manual_with_retest`）。证据：REM §2.2 `rem_acceptance_full_loop_import_discover_own_incident_retest_close`；FIX3 `fix3_full_business_loop_with_sentinel_secrets_leaves_no_original_anywhere`。
- **C-39 详情 / 项目切换竞态（已修部分）**：同项目详情乱序返回丢弃；证据读取中切项目丢弃；设置草稿不跨项目提交。证据：FIX3 R1、R3（node `FIX3-R1 …`、`FIX3-R3 …`；`fix3_r1_review_reads_ownership_inside_the_write_transaction`）；FIX4 P1／R4（`tools/test_asset_exposure_settings.cjs` 10/10）。复核面板仍有问题，见 U-01。
- **C-40 `cancel_exposure_scan` 校验项目归属**。证据：FIX2 §4（变异 F4-M1..M5）。

### 2.6 覆盖矩阵与钻取

- **C-41 缺口不变绿**：未配置、来源结果 partial、计划外待查项、`classification_pending` 都不再变绿。证据：`g4_1_coverage_never_turns_a_gap_green_*`；REM P1-6 `rem_p1_6_partly_unconfigured_or_partial_source_results_never_turn_green`；FIX4 P1／R3 `fix4_r3_outside_plan_gaps_and_classification_pending_leads_never_turn_green`。
- **C-42 覆盖矩阵钻取**：时间窗口（最近一轮 / 7 / 30 / 90 天）× 单位 × 只看缺口，游标分页，读取失败明示“不是没有缺口”。证据：REM §1，`rem_ui_coverage_drilldown_has_window_unit_and_paged_items`、`P1 UI the coverage drill-down changes window, unit and gap filter…`、`P1 UI a failed drill-down says so…`。
- **C-43 实时状态与真实分母**：stage、来源、完成 / 总数、请求 / 字节、落盘时间、缺口，未知总量不给假百分比（T33）。证据：G4 §8.3 `live_run`（变异 R29 / N03）。

### 2.7 导入

- **C-44 导入元数据不落秘密**：operator、文件名用 `redact_text`，声明 URL 用 `redact_url`；原文只以摘要进入幂等哈希。证据：FIX4 P1／R8 `fix4_r8_import_metadata_secrets_never_reach_storage_responses_or_exports`（修前 52 处，修后 failures=0）。

### 2.8 导出

- **C-45 导出可复核与命名**：`<项目名>_暴露面_<本地导出时间>.csv`、rowKind、公式前缀防护、路径 / 符号链接校验，`verify_exposure_export` 读回行数与 SHA，台账汇总与明细对账（T34–T36）。证据：G4 §8.2，`g4_3_exports_*`。
- **C-46 线索 CSV 三列错位已修**：`splice(9..9)` → `splice(10..10)`。证据：FIX4 P2 CSV，`fix4_p2_csv_every_findings_column_holds_what_its_header_says`。

### 2.9 脱敏与秘密保护

- **C-47 秘密不落库（文本字段、人工录入线索、脱敏碰撞）**：事件动作 / 改判 / 导入声明 / 定级理由写库前 `redact_text`；人工线索标题 / 地址 / 摘录脱敏；脱敏后相同、原文不同的载荷判为冲突而不是重放。证据：REM P1-5 `rem_p1_5_*`；FIX3 R6 `fix3_r6_a_redaction_collision_is_a_different_payload_not_a_replay`、R8 `fix3_r8_a_manual_finding_is_stored_redacted_and_distinct_secrets_stay_distinct`；FIX3 全库扫描（211 表 / 707 格 / 17 响应 / 2 导出 × 6 哨兵 × 11 形态，0 命中）。

### 2.10 授权与撤权

- **C-48 撤权后零新请求，并保留撤权前的观察**：外部来源逐项、逐页重读授权；撤权复测被拒（T30）；撤权前已取得的观察保留，并标 `scopeGrant`，来源记 partial，不自动确认。证据：REM P1-3 `rem_p1_3_external_sources_send_nothing_after_the_scope_is_revoked_mid_round`、`rem_p1_3_t30_a_retest_of_a_revoked_asset_starts_nothing_and_sends_nothing`；FIX3 R4；FIX4 新增 T21 `fix4_new_t21_revoking_before_page_two_keeps_page_one_marked_and_sends_no_page_two`。跨项目引用拒绝且零写（T05），`g4_0_another_project_reads_and_writes_nothing`。

### 2.11 崩溃恢复

- **C-49 三处真实 SIGKILL 与恢复不重复（T37、T38、T39、T18）**：claim 与 close 之间（`q3_sigkill_*`）、首次 claim 前、保存后 finish 前都是真实 `kill -9`；恢复不重置额度，不产生双活任务。证据：REM §2.4 `rem_t37_sigkill_before_the_first_claim_leaves_nothing_half_done`、`rem_t37_sigkill_after_save_before_finish_redoes_the_item_without_duplicates`；`q3_two_simultaneous_starters_get_one_active_run`、`q3_the_budget_is_persisted_so_a_restart_cannot_spend_it_twice`。

### 2.12 规模

- **C-50 三档实测与十万档修复后数字**：G4 三档（release，WAL + synchronous NORMAL）分页都达到初始目标、取消后 0 个新项、恢复 0 个重复；FIX2 十万档全量 propose 11.9 s、单轮 0.43 s，投影约 0.7 s，汇总 1.2–1.5 s，钻取约 0.64 s / 页。证据：G4 §5；REM §2.5 `tests_g4_scale::rem_scale_tier_with_per_asset_projection`；FIX2「10 万档复测」；O3 规模表 `scale_4000_rows_reconciles_and_reports_metrics` / `scale_20000` / `scale_100000`。前提与缺口见 U-09、U-10、U-11。

---

## 3. 未完善或仅部分满足

严重度：**高** = 可能写错数据、泄露秘密或误报安全；**中** = 功能缺口、验收证据不足或性能风险；**低** = 体验、环境、文档或产品待定项。“建议修法”只取文档里已写出的方向；文档没写方向的，标“文档未给出”，并给出最小下一步。

### 3.1 FIX4 新增问题 N1–N7（只记录，未修）

| 编号 | 问题 | 严重度 | 建议修法 |
|---|---|---|---|
| U-01（N1） | 复核面板（`useExposureReview.ts` / `ExposureReviewPanel.vue`）跨项目写入：项目 7 已读后切到 8（读取中），改判发到项目 8 却带项目 7 的分类 5；迟到的旧读取收尾后复核按钮恢复；新项目读取失败后仍可写 | 高 | 按 FIX4 P1／R4 设置草稿的做法：读取成功时把面板数据绑定到项目，加载中或归属不一致时拒绝写。试做的修复与测试存档在 `~/.ovi_o3/ir/f4_s1_shelved/`，可复用 |
| U-02（N2） | `review_scoped_cas` 不校验引用观察的归属：B 的线索带 A 的观察返回 `Ok(0)` 并写入 | 高 | 在写事务里校验每个引用观察都属于本项目本线索，不符则零写。FIX4 试做时只对带 `expected_review_event` 的人工复核生效，原因见 U-03 |
| U-03（N3） | `mark_remediation` 关闭后的系统复核：把 `finding_id` 当观察引用写入；而且在主事务提交后才执行，失败时报错但整改已关闭 | 中 | 改为写正确的观察引用，并把系统复核并入主事务（同一 ImmediateTx）；修好后 U-02 的引用校验可对所有调用生效 |
| U-04（N4） | 被动采集观察 URL 中的凭据型查询参数（如 `?access_token=`）原文写入 observations url / final_url / canonical_url、findings.source_url、analysis_cache、incident_locations 和证据 / 事件响应（修前扫描 38 处） | 高 | 落库前用 FIX4 新增的 `redaction::redact_url` 处理观察 URL；复测要拒绝脱敏后的 URL（FIX4 试做过，已撤回）；用 FIX4 的深度扫描回归 |
| U-05（N5） | Set-Cookie / X-Api-Key 等响应头在被动采集流程里不写入观察 headers，关掉 `redact_json` 键名打码的变异因此存活；头部保存策略未审计 | 中 | 文档未给出；最小下一步：审计头部保存策略，并补一个能杀死该变异的测试 |
| U-06（N6） | `useExposureIncidents.act` 不校验详情所属项目与当前项目是否一致，目前依赖切项目时 `load()` 先 reset | 中 | 文档未给出；可参照 U-01 / FIX4 R4：`act` 前比对详情项目与当前项目 |
| U-07（N7） | 环境：`cargo clippy --lib --tests -D warnings` 在其他会话的 agent / multi_agent 文件上报 dead_code；`cargo test --lib ownership` 有 2 条 agent 表失败（`agent_http_budget_receipts`） | 低 | 不属于本专项，由对应会话处理；本专项门禁需继续用 `--skip agent_tests` 并注明 |

### 3.2 外部验证与规模

| 编号 | 问题 | 严重度 | 建议修法 |
|---|---|---|---|
| U-08 | **真实 GitHub 代码搜索与受控读取从未验证**；G2 的真实请求也没覆盖分页第 2 页和错误码 | 中 | 用户按 IR-04 把 token 存入钥匙串（`security add-generic-password -s oviraptor.exposure.github -a <引用名> -w`），在设置页填引用名后做一次受限真实验收 |
| U-09 | **十万档未在最新代码上重验**：FIX3、FIX4 都沿用 FIX2 数字 | 中 | 在 release 临时库上重跑 `rem_scale_tier_with_per_asset_projection`（含全量 propose） |
| U-10 | **规模测试里工作项用 SQL 收口**，没有跑 10 万档合成执行器；G4 三档执行器回调是合成的（不发网络），设备与阈值未在 G0 冻结，只报告实测值、不称验收通过 | 中 | 用合成执行器跑 10 万档；冻结设备与阈值后再判定 |
| U-11 | **大事件读取对齐成本**：FIX2 后读 200 位置大事件从 4.6 ms 变为 33–40 ms，与位置数成正比。FIX3 P2-1 改为批量取边（目录只读一次，VM 步数比值从约 40 降到 3.0），但没有在十万档重测耗时 | 低 | 随 U-09 重测大事件读取；仍高时继续按 FIX2 的建议做批量投影 |
| U-12 | 十万档其他慢点（G4 §5 的事实）：取消到 `run_plan` 返回 348 s（主要是收尾）；恢复 21.3 分钟；线索导出 135.5 s / 100000 行；RSS 峰值 634 MB（FIX2 全量 propose 690 MB） | 低 | 文档未给出；建议随 U-09 重测后再决定是否优化 |
| U-13 | O3 大组首次调用 1.39 s：开始事务里一次性写入 9810 条 staging，没有分块 | 低 | O3 写明：需新增 `staging` 操作状态，要改 db_schema.rs 的 CHECK，并处理恢复与 CAS，需批准 |
| U-14 | O3 组页单页约 140 KB，有上界但偏大 | 低 | O3：缩小分页载荷，会改 O2 DTO 和前端 |
| U-15 | T16（两个并发 worker 争最后额度）无法验证：IR-08 选择“从合同和面板移除并行度”，执行器严格串行 | 低 | 只有决定实现真并行时才需要做（要处理 T16–T18 的额度争用、取消、连接上限） |
| U-16 | G3 真实文档读取没覆盖跳转、401 / 404 / 429、PDF、截断（只有 fixture） | 低 | 文档未给出；最小下一步：挑公开、可控的地址补真实只读用例 |

### 3.3 秘密保护残余

| 编号 | 问题 | 严重度 | 建议修法 |
|---|---|---|---|
| U-17 | **去重 / 幂等哈希里的短秘密可枚举**：R6 / R8 / FIX4 R8 的幂等哈希和指纹包含原文 SHA-256 摘要，不可逆，但低熵秘密理论上能猜测比对 | 中 | FIX3：改用带本机密钥的 HMAC |
| U-18 | **URL userinfo 口令里未编码的 `/` 不会被 URL 凭据规则识别**（真实 URL 中必须百分号编码，编码形态已覆盖） | 低 | 文档未给出；可在 `redact_url` / URL 凭据规则里补未编码分隔符的容错，并加测试 |
| U-19 | 撤权时已经发出的那一个在途请求无法撤回；FIX4 后它撤权前取得的观察会保留并标 `scopeGrant`，之后零请求 | 低 | 设计限制，不需要修；如产品要求“撤权即丢弃”，需重新决定语义（FIX4「语义变更」） |

### 3.4 待用户 / 主控决定

| 编号 | 问题 | 严重度 | 建议修法 |
|---|---|---|---|
| U-20 | **O4 暂缓**：O3「O4 前遗留」= 决定 U-21、U-22，大组 staging 分块（U-13），缩小页面载荷（U-14） | 中 | 用户决定 U-21 / U-22 后再启动 O4 |
| U-21 | **主动扫描是否读归属授权**：哨兵主动扫描（scan_lifecycle / scan_startup）不读归属授权，“授权”目前只等于暴露面被动采集准入 | 中 | 如需约束主动扫描，要改门禁以外的代码，需用户批准 |
| U-22 | **lib.rs 启动恢复钩子**：恢复现在在第一次归属 IPC 时触发 | 中 | 在 lib.rs 的 setup 闭包里加一行 `ownership_ensure_startup_recovery`；lib.rs 只允许改注册部分，需批准 |
| U-23 | IR-16：取消的轮次是否允许恢复。现状 `resume_exposure_scan` 接受 cancelled，页面有“按冻结计划恢复” | 低 | 产品要求“取消即终态”时，同时改 `resume_exposure_scan` 的状态判断与 `live.rs` 的 resumable |
| U-24 | IR-12：GitHub 来源启用与额度是否需主控确认（上界写死在 `oviraptor.exposure.github.v1`；集团名称会作为查询词发往 GitHub） | 低 | 主控给出约定；不约定则保持默认不启用、上界写死 |
| U-25 | IR-13：ExternalSourceScope 完整对象（providerHost / path / method / 有效期 / 额度 / 审批人）放在哪里 | 低 | 主控决定放 exposure 自有表还是共享表；需新 IPC 时按 IR-06 注册 |
| U-26 | IR-03：PSL / 组织目录升为声明过的 `pub(crate)` 只读端口 | 低 | 不阻塞；现状依赖 include! 结构 |
| U-27 | O3 需决定项其余各条：旧的 `learn_domain_rule` 已改为精确 host；O2 新表没有迁移（尚未发布）；项目 3 这类未绑定目录的项目仍不按标签合并 | 低 | 用户确认这三条口径；O2 表在发布前补迁移 |

### 3.5 规格与交付文档里写明仍未（完全）满足的其他项

| 编号 | 问题 | 严重度 | 建议修法 |
|---|---|---|---|
| U-28 | 覆盖矩阵单位轴只对按资产计划的来源（passive_web）报告；按域名 / 关键词计划的外部来源“不对应单个资产，不按单位分摊” | 低 | 设计口径（REM §1）；需要外部来源按单位统计时，先定义分摊规则 |
| U-29 | 规格要求“覆盖矩阵点开可见原查询、分页进度、观察和缺口”：REM 已做钻取（窗口 × 单位 × 缺口、游标分页），但文档没有逐项写明钻取里是否展示原查询、分页进度和观察，也没说明 G4 实时块“只列前 20 个缺口”是否已被钻取取代 | 中 | 对照 SPEC 逐项核验钻取页面，缺的补上 |
| U-30 | 候选页说明里事件详情部分：G4 §8.3 判为“待 IR-15”；IR-15 在 P0 已注册，但之后没有单独复核这一条 | 低 | 对照 SPEC 复核事件详情中的“为什么关联 / 读取还是索引 / 证据不足在哪里” |
| U-31 | PDF / Office / 图片 OCR 解析未实现 | 中 | 本期范围外（G3）；需要时另立阶段 |
| U-32 | 文库 / 网盘关键词搜索、登录、提取码不支持 | 低 | 本期范围外（G3 能力表 unsupported） |
| U-33 | 暗网 / 地下情报源与真实情报源未接入 | 低 | 本期范围外（G3） |
| U-34 | 导入只用过本机构造样本，没有真实用户材料验收 | 低 | 用户提供可用材料后做一次真实导入验收 |
| U-35 | 分类来源渠道 `code_platform` 等早期只有名字；GitHub、文档、导入落地后，各渠道是否都已映射，文档没有总核对 | 低 | 文档未给出；最小下一步：核对 `unrecognized_source` 是否还会出现 |
| U-36 | O3「需决定」第 3 条：Exposure 每次运行只加载一次 grants。暴露面侧已在 REM P1-3 / FIX3 R4 改为逐项、逐页重读授权，但 O3 文档本身未更新 | 低 | 在 O3 文档追加说明，标为暴露面侧已解决 |
| U-37 | 本专项的 P1（REM）、FIX3 R1–R8 都是**找不到原始评审清单**时自审得出的编号；FIX4 声明 FIX3 自行替换的编号不能作为原问题已关闭的依据 | 中 | 找回或由用户重新提供 R1–R8 / P2 原始清单，逐条对照关闭状态 |
| U-38 | GitHub 正式开源名单的共享治理（G2 §6 列为待 IR-13） | 低 | 随 U-25 一起决定 |
| U-39 | G4 §8.3“新前端测试进 package 脚本”在 IR-10 已落地；但 FIX 系列新增的 node 测试是否都已纳入 `npm run test:exposure`，文档只写了总数 118，没有逐文件核对 | 低 | 对照 `package.json` 的 `test:exposure` 脚本，核对新增测试文件 |
| U-40 | 真实仓库从未做 cargo 构建（按规则），所有 Rust 结果都来自隔离副本 | 低 | 用户决定是否在真实仓库跑一次门禁（注意其他会话在同一工作区） |
| U-41 | 工作区未 commit，并混有其他会话改动，提交时容易混入他人文件 | 中 | 提交前按各轮写回清单（`fix2/3/4_written_manifest.json` 等）挑出本专项文件，由用户决定提交方式 |

---

## 4. 工作规则与环境说明

- **隔离副本**：所有开发、测试、变异都在 `~/.ovi_o3/speccopy` 中进行（O3 时为 `~/.ovi_o3/mutcopy`）；规模测试用副本的副本 `~/.ovi_o3/scalecopy`，避免被变异污染。副本里的 `node_modules` 是指向真实仓库的软链，清理前先 unlink（G4 §5、CP）。
- **独立 target**：`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`（debug）和 `~/.ovi_o3/spec_target_rel`（release 规模）。真实仓库**不做 cargo build、不 cargo clean**，`src-tauri/target` 不动。每轮结束删除副本和独立 target（CP 各段）。
- **白名单文件**：只改暴露面自有文件（`src-tauri/src/commands/exposure*`、`exposure/schema.sql`、`src/features/assets/exposure/*`、`src/components/ExposureSurface.vue`、对应 `tools/test_*` 等）。经批准的例外是归属投影文件 `ownership_unit_projection.rs`。Native / agent / db_schema.rs 不改（FIX4 头部、G4 §8.3）。
- **写回守卫**：每轮记写回截止时间。写回前逐个核对真实文件 mtime 早于截止，或内容与副本相同 / 是副本前缀；否则判为 FOREIGN EDIT，拒绝写回。写回后按 SHA 逐字节核对，并保存写回清单（REM §4、FIX3「写回与边界」、CP）。
- **共享文件行级补丁**：`lib.rs`、`commands.rs`、`src/features/assets/api.ts`、`package.json`、`db_schema.rs` 每轮先记 T0 SHA（如 `fix3_shared_t0.sha256`、`fix4_shared_t0.sha256`）。只有 T0 与当前一致时，才按行打最小补丁（例如 lib.rs +1 行 `exposure_coverage_drilldown`，api.ts 单行 91eab291 → 2044b69a）；FIX4 没有改共享文件。
- **不 commit**：不 commit / push / stash / reset / checkout；HEAD 保持 `14a7875`。
- **不碰真实用户库**：规模测试只用临时库；通知 / 下架只登记，不出站。
- **变异测试**：每个修复都用“把修复改回去”的变异证明测试会红。还原后 touch 更新 mtime（防止 cargo 复用变异二进制，O3 runner 修正），并核对 SHA，不留 `.mutbak`。
- **他人改动**：同一工作区有 Codex 等独立会话。CP 记录过他人改 `commands.rs`、`db_schema.rs`（13:57 加 agent include；FIX4 时为 41014637）、`models_appsec.rs`。本专项未写这些改动，也不回滚。

---

## 5. 证据范围声明

- 本文所有事实都来自文首列出的文档与 CP，没有新的测试或实测；唯一的新动作是写完后核对 HEAD 仍为 `14a7875`。
- 测试数字都是隔离副本中的结果；真实仓库写回后只跑过前端（如 REM `npm run test:exposure` 109/109、vue-tsc 0），没有跑过 Rust。
- 外部真实证据只有：G0 探测（crt.sh 200、Wayback 200、GitHub 仓库搜索匿名 200、代码搜索匿名 401）、G2 3 次匿名仓库搜索、G3 2 次公开文档读取。其余外部来源行为（包括全部 GitHub 代码搜索、FIX3 / FIX4 的撤权与额度场景）都只有本地夹具证据，不算真实外部验收。
- 规模证据是 release 构建 + 临时库；执行器回调合成或工作项 SQL 收口，未冻结设备与阈值，所以只报告实测值，不称验收通过。十万档最新一次是 FIX2。
- 变异结果、扫描结果、规模原始日志都在 `~/.ovi_o3/ir/logs/`（隔离环境，不入库）。
- §2、§3 的分组和编号（C-xx / U-xx）是本文为便于后续引用新加的，不替代各交付文档的原编号（P1-x、R-x、N-x、IR-xx、Txx）。引用时以原编号为准。
