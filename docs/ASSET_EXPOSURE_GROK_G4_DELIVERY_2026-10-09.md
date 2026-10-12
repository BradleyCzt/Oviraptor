# Asset 暴露面 Grok G4 交付：事件、跨单位统计、实时页面、分页导出与规模（G4-0 至 G4-5）— 2026-10-09

- 执行者：Grok
- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`，HEAD `14a7875`。开始、每次 Mac 掉线恢复后和回写前都核对过；未 commit/push/stash/reset/checkout/merge。
- 隔离环境：
  - 副本 `~/.ovi_o3/speccopy`，`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`；规模三档用 release 构建，另用 `~/.ovi_o3/spec_target_rel`。
  - 编译和测试都在 screen 里跑，断点写入 `~/.ovi_o3/g4/CHECKPOINT.md`。
- 回写：由 `~/.ovi_o3/g4/syncback.py` 完成。只写白名单文件，真实文件的 SHA 必须等于 G4 的 T0 快照（75 个文件）或上次回写值。全程没有 FOREIGN EDIT。回写后再跑 `--dry` 结果为 `changed 0`，即白名单内所有文件与副本逐字节一致。
- 真实用户库 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 全程未打开。Rust 测试和规模测试全部用 tempdir 临时库，网络只连 127.0.0.1 fixture；G4 没有任何外网请求。
- **处置边界：** 事件处置里的“通知 / 下架 / 整改 / 关闭”只登记人工已经做过的动作。代码里没有任何发信、发请求或调用平台接口的路径，返回值固定 `contactedByApplication=false`；复测结果从不自动关闭（`autoClosed=false`）。
- **IR 状态：** 没有任何 IR 获批。事件台账的读取和人工动作需要 IR-15（IPC 注册）；页面把“事件台账”显示为 `integration_pending（IR-15）`，写明“不是没有事件”，不提供会失败的按钮。本文不把事件台账报为“生产可用”。

## 1. 用户行为与生产调用链

**G4-0 事件与传播位置（自动建议在生产路径上；人工读写待 IR-15）**
- **自动建议（生产可达）：** 已注册的 `start_exposure_scan` / `resume_exposure_scan` → `execute_exposure_run` → `orchestrator::run_plan`，在 finalize 之前调用 `incidents::propose(project, Some(run_id))`。建议失败时，这一轮以 `[exposure_incident_proposal_failed]` 记为基础设施失败，不假装完成。
- **建议规则（`oviraptor.exposure.incidents.v1`）：**
  - 只看处置为 `risk_candidate` / `classification_pending` 的线索；公开招采、公开材料排除的线索永远不进事件。
  - 内容键只用严格依据：`text:<全文规范化哈希>`（G3 文档读取），或 `body:<响应体哈希>`（未截断、有字节，且不是 CT/归档/GitHub 仓库元数据、代码命中摘要、搜索页、文档元数据这类索引或摘要）。截断正文、片段、索引行、同标题、同文件名都不会按内容合并。
  - 同内容至少两个地址，或含风险线索，才成为一个 `candidate` 事件（`stable_key = content:<键>`）；单条风险线索成为 `lead:<线索>` 事件。
  - 新一轮出现相同内容：位置挂到原事件（沿合并链找到存活事件）；原事件已有人工结论时只置 `review_needed` 并写原因，不改结论。本轮内容/规则变化（版本号 > 1）或线索后来被排除的已决事件，同样只标复核。
  - 位置记录 `location_kind`（page/document/archive_index/certificate_index/repository/code_hit/blob/import/intel_report）、`collection_mode`、`relation`、观察与轮次编号。
  - 单位项目里，资产归属已确认 → 自动 `confirmed_link`；attributed/related → `candidate_link`；其余不建关联。集团总账项目没有逐资产单位边，事件计入“单位未知”（IR-02/IR-11）。
- **人工动作（函数已实现，入口待 IR-15）：** `incidents::act(project, action)`，未注册的包装命令为 `act_on_exposure_incident`。
  - 每个动作必须带 `operationId`、`expectedRevision`、`actor`（1–80 字）、`reason`（4–500 字）；未知字段拒绝。
  - 同一 IMMEDIATE 事务里：重放检查 → 读取 → CAS → 写入 → 追加事件日志。同 operationId 同载荷返回原结果（`replayed:true`），异载荷 `[exposure_incident_operation_conflict]`；版本不符 `[exposure_incident_conflict]`；已合并的事件 `[exposure_incident_state]`。所有拒绝零写入。
  - `review`：candidate / reviewing / verified / dismissed / excluded_public，并给出与旧 finding.status 的映射（new / reviewing / confirmed / dismissed）。核实需要至少一个可追溯观察（`[exposure_incident_evidence_required]`），复核会清除 `review_needed`。
  - `merge`：两边都做 CAS，位置复制到目标并在原行标 `merged`，单位关联随合并带入，原事件留档为 `merged`。
  - `split`：拆出新的 `candidate` 事件（`split:<operationId>`），原事件至少保留一个位置；两边各有一条事件日志。
  - `link_entity`：实体必须已在组织目录中（不新建实体）；证据观察必须是本事件有效位置的观察；`confirmed_link` 必须有证据。记录目录版本。
  - `record_action`（只登记）：notification_recorded（必须写明通知对象与渠道）、takedown_requested_recorded、takedown_confirmed_recorded、remediation_recorded、retest_linked（必须给复测轮次）、closed_recorded（候选事件不能关闭；关闭依据记为 manual_with_retest / manual_without_retest）、reopened（只能从已关闭重开）。
  - 复测比对（retest_linked 时执行）：复测轮次必须属于本项目、已结束、且晚于事件位置。每个位置分别给 not_observed / unreachable_or_failed（401/403/404/5xx/传输失败）/ partial_content / same_content / changed_content，**从不自动关闭**。
- **读取（入口待 IR-15）：** `page_exposure_incidents`（按 id 倒序 keyset，筛选 `""`/`open`/具体状态 + 只看待复核；游标 `v1:<筛选哈希>:<id>`，换筛选用旧游标返回 `[exposure_cursor_mismatch]`；每页 1–200）、`read_exposure_incident`（完整 DTO：schemaVersion、projectId、stableKey、revision、policyVersion、findingIds、observationRefs、runIds、directory/bindingVersion、collectionMode、evidenceGrade、单位关联、最近 50 条动作并标 truncated、第一页位置、状态映射）、`page_exposure_incident_locations`。G4 之前的历史轮次可用 `propose_exposure_incidents` 一次性补建议。
- **旧表防护：** 每次读写先 `check_schema`（PRAGMA table_info）；缺列的旧表返回 `[exposure_incident_schema_mismatch]`，不当空结果读。

**G4-1 跨单位统计与覆盖矩阵（生产可达，挂在已注册的 `exposure_scope_summary` 上）**
- `summary.coverage`（`coverage::coverage_matrix`，`oviraptor.exposure.coverage.v1`）：11 行来源能力 × 9 个信息类别。
  - 行状态：unconfigured / not_implemented / not_run / planned / pending_authorization / cancelled_interrupted / executed / failed / partial。
  - 格子状态：verified_incident / candidate / executed_no_hit / partial / not_applicable，或沿用行状态。**只有 executed_no_hit 是绿色**；失败、阻塞、取消、未配置、未实施、超出上限都不是绿色。
  - 每行分母写明单位（资产 / 域名 / 查询 / 地址 / 批次）与 planned、executed、uncovered、failed、blockedQuota、blockedAuthorization、cancelled、open、skippedUnconfigured、outsidePlan；失败留在分母里。
  - `file_import`、`public_intel_import` 标 `integration_pending（IR-14）`；`pdf_docx_ocr`、`netdisk_keyword_search`、`underground_feed` 标 unsupported。证书索引只对“技术网络”类别适用，不算客户信息排查。
  - 单位轴：有投影时给单位项目的计划/执行/未覆盖；投影读取失败时 `not_reported`，页面写“未报告”，不写 0。
- `summary.unitStatistics`（`coverage::unit_statistics`）：
  - 集团唯一事件数（open / all / byStatus，已合并的不计）。
  - 单位行：按每个（事件, 实体）最强关联计 confirmed / candidate / conflict；省、市、县归入省行；专业公司单列，不归入任何省；目录外实体单列。
  - 省行给“唯一事件数”：同一事件涉及同省多个单位只算一次。
  - `sumOfUnitInvolvement` 与集团唯一数并列，页面写明“一起事件涉及多个单位时各计一次，所以大于集团数”。
  - `unknownUnitIncidents`：没有任何关联的未关闭事件。
  - `dedup`：风险线索数 → 去重后事件数 → 有效位置数。
  - 资产数：没有投影时为“未报告”。

**G4-2 实时页面（生产可达）**
- `summary.liveRun`（`live::live_run`）：最近一轮的 runId、状态、active、stage、当前来源、是否已请求取消、是否可恢复；每个来源的完成数 / 总数与状态分布；进度按工作项计（`percent` 只在总数 > 0 时给，并写明“按工作项计数，不代表请求量、字节量或剩余时间”）；实际请求 / 字节与预算；最近落盘时间；未闭合工作项总数与前 20 个。
- 页面复用已有的 2.5 秒单飞轮询（绑定 projectId，切换项目或卸载即停），总览显示上述行；进行中显示“停止这一轮”，可恢复时显示“按冻结计划恢复第 N 轮”（走已注册的 `resume_exposure_scan`）。

**G4-3 分页与导出（生产可达）**
- 线索、轮次、簇、待归并本来就是 keyset 分页（E 系列）；G4 新增事件和位置的 keyset 分页（入口待 IR-15）。
- `export_exposure_findings` 增加可选参数 `kind`：
  - `findings`（默认）：原 14 列 + 13 列（信息类别、来源渠道、处置、处置理由、分类策略版本、事件编号、位置编号、事件状态、事件版本、单位关联、整改状态、采集时间、索引时间）。
  - `ledger`：事件台账，每行声明 `rowKind`（summary / incident / location / entity_link / coverage）。4 条汇总行分别等于本文件同类明细的行数；覆盖矩阵读不到时汇总行写“未导出（不是 0）”。
  - 文件名 `<项目名>_暴露面[_事件台账]_<YYYYMMDD-HHMMSS±zzzz>.csv`（本地时间带显式时区），非法字符替换，同名加 `_2…`，目标已存在时拒绝写，不覆盖。
  - 每个格子先脱敏（URL 账号、Authorization 头、key=value 秘密）再做公式注入防护；导出不含正文。
- 共享 `ExportResult{path, rows}` 不能改（models 不在白名单），所以页面导出后立即调用已注册的 `verify_exposure_export` 读回落盘文件，显示“已导出 N 行（格式）· SHA-256 前 16 位”；读不回时写“未能读回核验”。`verify_exposure_export` 现在识别 findings_v1 / findings_v2 / ledger_v1 三种表头，逐行核对列数，并回给 rowKinds。

**前端（白名单内）**
- 新增 `incidents.ts`（纯文案与校验）、`useExposureIncidents.ts`（事件队列组合函数：generation 与项目守卫、keyset 追加去重、未注册时 integration_pending、重试沿用同一 operationId、版本冲突后刷新详情、详情必须属于当前项目）、`ExposureIncidentsPanel.vue`（列表、详情、位置、单位关联、动作记录、复核/登记/关联表单，顶部固定显示处置边界说明）。
- `contracts.ts`：汇总新增 6 个字段的严格读取；`exportLine`。
- `api.ts`（exposure 私有）：导出 `kind` 参数；`exposureIncidentApi` 四个包装，注释指向 IR-15。
- `useExposureSurface.ts`：导出后读回核验，`lastExport`。
- `ExposureSurface.vue`：新增“事件台账”“覆盖与单位”两个页签，总览显示实时行与停止/恢复按钮，“导出线索”“导出事件台账”两个按钮。`exposure-tabs.css` 追加样式。

**仍缺的步骤**
- IR-15 注册后，事件台账页才能读取和写入；在此之前，自动建议的候选事件只在库里和导出的台账 CSV 里可见。
- 集团总账项目的单位级统计需要 IR-02 的逐资产单位边。
- 覆盖矩阵格子还不能点开看原查询、分页进度和观察（只给了分母和原因）；实时块只列前 20 个缺口。

## 2. 文件、SHA、白名单检查、共享依赖

白名单：`commands/exposure.rs`、`commands/exposure/**`、`src/features/assets/exposure/*`、`src/components/ExposureSurface.vue`、`src/exposure-*.css`、`tools/test_asset_exposure*.cjs`、`tools/test_exposure_*.cjs`、`docs/ASSET_EXPOSURE_GROK_*`。回写的文件加本文全部在白名单内；回写后再跑 `--dry` 结果为 `changed 0`。SHA 见附录 A。

- **新增：** `exposure/incidents.rs`、`exposure/coverage.rs`、`exposure/live.rs`、`exposure/tests_g4.rs`、`exposure/tests_g4_scale.rs`、`src/features/assets/exposure/incidents.ts`、`useExposureIncidents.ts`、`ExposureIncidentsPanel.vue`、`tools/test_exposure_g4.cjs`、本文。
- **修改：**
  - `exposure.rs`：模块注册（coverage、incidents、live、`#[cfg(test)]` 的 tests_g4 与 tests_g4_scale）、`export_exposure_findings` 增加可选 `kind`、`export_exposure_in`、五个未注册的命令包装（IR-15）。
  - `export.rs`：v2 表头与 13 列、台账行、带时区的文件名、同名唯一后缀、`known_header`（findings_v1 / findings_v2 / ledger_v1）、逐行列数核对。原 14 列导出保留。
  - `summary.rs`：liveRun、coverage、unitStatistics 及各自的 error 字段。
  - `orchestrator.rs`：finalize 前调用 `incidents::propose`，失败记 `[exposure_incident_proposal_failed]`。
  - `schema.sql`：四张 exposure 自有表（`exposure_incidents`、`exposure_incident_locations`、`exposure_incident_entities`、`exposure_incident_events`）与观察内容索引，全部 `CREATE IF NOT EXISTS`，只增不改。
- **前端修改：** `contracts.ts`、`api.ts`、`useExposureSurface.ts`、`ExposureSurface.vue`、`exposure-tabs.css`；旧页面 harness（entry/evidence/page 三个 cjs）增加事件面板的真实编译映射。
- **IR 文档：** 追加 G4 段（IR-15、IR-06 补记、IR-11 更新、IR-10 补充、IR-16）。原 171 行前缀与 `~/.ovi_o3/g4/ir_before_g4.md` 逐字节一致。
- **共享依赖：** 没有改任何共享文件。只读调用 `org_directory_version`、`load_org_entities`、`load_project_org_binding`（IR-03 的过渡方式）。`ExportResult{path, rows}` 在共享 models 里，未改，哈希与行数改由已注册的 `verify_exposure_export` 读回。

## 3. 来源能力矩阵

G4 没有新增来源。来源能力矩阵的状态与 G3 交付 §3 相同，G4 只是把这张矩阵做成了可查询的覆盖矩阵（`summary.coverage`）：

| 能力 | 实现 | 生产可达 | 覆盖矩阵里的标注 |
|---|---|---|---|
| first_party_web / certificate_index / archive_index | 是 | 是 | 按最近一轮的实际工作项计算 |
| repository_metadata | 是 | 是（默认不启用） | 同上 |
| code_search / controlled blob read | 是 | 否：IR-04 | unconfigured（凭据端口未装配） |
| explicit_document_url | 是 | 是（默认不启用） | 同上 |
| file_import / public_intel_import | 是 | 否：IR-14 | integration_pending |
| pdf_docx_ocr / netdisk_keyword_search / underground_feed | 否 | — | unsupported，不生成绿色格子 |
| 事件台账读写（不是来源） | 是 | 否：IR-15 | 页面 integration_pending |

## 4. 测试命令、结果、计数、重复

| 项 | 命令 | 结果 |
|---|---|---|
| 基线（G3 结束，副本） | `cargo test --lib exposure` | 162 passed，2 ignored |
| G4 新测试 | `cargo test --lib exposure::tests_g4` | **17 passed**，1 ignored（规模档，单独跑） |
| 全量与重复 | `cargo test --lib exposure` ×3（repeat1–3） | 4/4 次 179 passed、3 ignored（三次重复 35.8–36.6 秒，类型别名修改后再跑一次 37.1 秒） |
| clippy | `cargo clippy --lib --tests` | exit 0。首次有 2 个 `type_complexity`（`coverage.rs` 的命中计数、`live.rs` 的轮次行），改成类型别名后复跑，**暴露面 0 warning** |
| 前端（副本 + 真实仓库） | `node --test` 全部 `tools/test_asset_exposure*.cjs tools/test_exposure_*.cjs` | 副本 93/93；真实仓库 **93/93** |
| 类型 | `npx vue-tsc --noEmit -p tsconfig.json`（副本 + 真实仓库） | 副本 exit 0；真实仓库 exit 0 |
| 规模三档 | `cargo test --release --lib exposure::tests_g4_scale -- --ignored`（独立 target `spec_target_rel`） | 三档全过，见 §5 |

**G4 Rust 用例（17 个）**

| 片 | 用例 |
|---|---|
| G4-0（12） | 两个地址的同一全文合并为一个候选、两个位置；截断片段、索引行和排除线索不按内容合并；新一轮挂位置且只给已决事件打复核标记；重放与异载荷零写入；两个连接同版本只有一个成功；合并与拆分保留历史，后续建议尊重合并链；单位关联引用组织目录，确认关联必须有证据；单位项目绑定只给已确认资产 confirmed_link；处置只登记、复测不关闭；跨项目零读写；缺列旧表拒绝而不是读成空；生产执行器把两个地址的同一文档建议为一个事件 |
| G4-1（2） | 跨单位事件在集团和省各计一次；覆盖矩阵缺口不变绿，失败留在分母 |
| G4-2（1） | 实时进度报告工作项、用量、取消和缺口，没有工作项时不给百分比 |
| G4-3（2） | 事件分页 keyset 完整且游标绑定筛选；导出命名、转义、脱敏，台账汇总与明细对账 |

- 失败与修复记录：
  - `g4_2` 首轮失败：同一项目同时插入 running 与 queued 两轮，撞上单活跃轮唯一索引。改为先把第一轮标为 cancelled 再插入 queued，并断言取消后的轮次不再显示为进行中。
  - 其余首轮编译问题（`province.id` 应为 `entity_id`、规模测试的预算交互、schema 测试缺项目行）在 g4b 之前已修。

**前端用例（11 个，`tools/test_exposure_g4.cjs`）**：绿色格子只认 executed_no_hit；单位统计把集团唯一数与单位之和分开，缺投影写“未报告”；实时进度没有假百分比；汇总读取器不发明字段；每个动作带版本、操作编号和理由，登记通知必须有渠道；导出行报告读回的摘要；未注册时显示 integration_pending 而不是“没有事件”；分页追加不重复、晚到的旧项目响应被丢弃；重试沿用操作编号、冲突后刷新；详情必须属于当前项目；页面接线与“不自动对外联系”的源码扫描。

**变异（共 46 个：46 个杀死，0 个等价）**

方法：逐一注入；Rust 变异跑 `exposure::tests_g4`，前端变异跑 `test_exposure_g4.cjs`。恢复时用 copyfile 加新 mtime（touch），再核对 SHA；每条都是 `restored=true`，没有残留 `.mutbak`。

| 编号 | 结果 | 内容 |
|---|---|---|
| N01 | 杀死 | 后端给 green 就显示绿色 |
| N02 | 杀死 | 缺投影写 0 |
| N03 | 杀死 | 无百分比也显示 % |
| N04 | 杀死 | 登记通知不需渠道 |
| N05 | 杀死 | 确认关联不需证据 |
| N06 | 杀死 | 重试换新操作编号 |
| N07 | 杀死 | 未注册显示为没有事件 |
| N08 | 杀死 | 冲突后不刷新 |
| N09 | 杀死 | 详情不核对项目 |
| N10 | 杀死 | 导出行不报摘要 |
| N11 | 杀死 | 缺覆盖矩阵当空矩阵 |
| N12 | 杀死 | 声称有下一页却无游标时静默结束 |
| R01 | 杀死 | 截断正文也参与同内容合并 |
| R02 | 杀死 | 索引来源也按正文哈希合并 |
| R03 | 杀死 | 排除线索也可成事件 |
| R04 | 杀死 | 单地址待分类也建事件 |
| R05 | 杀死 | 已复核事件新增位置不标复核 |
| R06 | 杀死 | 建议不沿合并链 |
| R07 | 杀死 | CAS 失效 |
| R08 | 杀死 | 同 operationId 异载荷被当重放 |
| R09 | 杀死 | 无观察也可核实 |
| R10 | 杀死 | 复核不清除待复核标记 |
| R11 | 杀死 | 拆分可清空原事件 |
| R12 | 杀死 | 合并不校验目标版本 |
| R13 | 杀死 | 目录外实体可关联 |
| R14 | 杀死 | 确认关联不需证据 |
| R15 | 杀死 | 别的事件的观察也可当证据 |
| R16 | 杀死 | 候选事件可登记关闭 |
| R17 | 杀死 | 声称应用联系了外部 |
| R18 | 杀死 | 复测 404 当内容变化 |
| R19 | 杀死 | 进行中的轮次可当复测 |
| R20 | 杀死 | 跨项目可读写事件 |
| R21 | 杀死 | 游标不绑定筛选 |
| R22 | 杀死 | 候选归属自动成确认关联 |
| R23 | 杀死 | 部分执行显示绿色 |
| R24 | 杀死 | 部分执行的格子记为已执行无命中 |
| R25 | 杀死 | 集团唯一数按单位相加 |
| R26 | 杀死 | 单位轴缺投影写成 0 |
| R27 | 杀死 | 资产数缺投影写成 0 |
| R28 | 杀死 | 失败从未覆盖里扣掉 |
| R29 | 杀死 | 无工作项给假百分比 |
| R30 | 杀死 | 文件名去掉时区 |
| R31 | 杀死 | 同名不加后缀 |
| R32 | 杀死 | 新列不脱敏 |
| R33 | 杀死 | 台账汇总行与明细不符 |
| R34 | 杀死 | 生产执行器不建议事件 |

日志：Mac 的 `~/.ovi_o3/logs/g4/mut_node.out`、`mut_rust.out`。

## 5. 临时库规模、性能、真实库声明、磁盘

三档都在 release 构建（`spec_target_rel`）的临时库上跑，WAL、synchronous=NORMAL（pragma synchronous=1），没有降同步、没有缩小规模。执行器回调是合成的（不发网络），测的是建计划、取消、恢复、事件建议、汇总、分页和导出；设备与阈值没有在 G0 冻结，所以热态分页只报告“实测达到规格给出的初始目标”，不称验收通过。

| 档 | 4000 | 20000 | 100000 |
|---|---|---|---|
| 种子 | 0.29 秒 | 1.1 秒 | 5.4 秒 |
| 默认上限 | 计划 500，超出 3500 | 计划 500，超出 19500 | 计划 500，超出 99500 |
| 建计划（max_items=全量） | 0.11 秒 / 4000 项 | 1.4 秒 / 20000 项 | 44.0 秒 / 100000 项 |
| 取消 | 第 400 项后置标记，0.70 秒返回，之后 0 个新项 | 第 500 项，16.3 秒，0 个新项 | 第 500 项，348 秒，0 个新项 |
| 恢复 | 重跑 3600 = 计划−已完成，0 个重复 | 19500，0 个重复 | 99500，0 个重复（耗时 21.3 分钟） |
| 全量事件建议 | 3.5 秒，建 1600 个事件 / 2000 个位置 | 81.9 秒，8000 / 10000 | 33.1 分钟，40000 / 50000 |
| 汇总 冷/热 | 30 / 25 毫秒 | 119 / 124 毫秒 | 826 / 569 毫秒 |
| 覆盖矩阵 / 单位统计 / 实时 | 6.1 / 3.0 / 6.0 毫秒 | 32 / 6.5 / 21 毫秒 | 157 / 38 / 109 毫秒 |
| 线索分页 首页/热态/后续最大 | 8.2 / 5.1 / 11.2 毫秒 | 24.9 / 29.7 / 33.7 毫秒 | 127 / 120 / 122 毫秒 |
| 事件分页 首页/热态/后续最大（目标 ≤2 秒 / ≤1 秒） | 6.9 / 3.4 / 2.1 毫秒，达标 | 13.4 / 7.1 / 8.2 毫秒，达标 | 33.4 / 30.1 / 30.2 毫秒，达标 |
| 导出 线索 / 台账 | 0.33 秒 4000 行 / 0.05 秒 3615 行 | 7.1 秒 20000 行 / 0.22 秒 18015 行 | 135.5 秒 100000 行 / 1.0 秒 90015 行 |
| 库 / WAL / 证据 JSON | 17.6M / 11.9M / 1.0M | 77.1M / 55.9M / 5.1M | 379M / 279M / 25.4M |
| RSS 峰值 | 56M | 145M | 634M |

- 取消语义：三档 `itemsAfterCancel` 都是 0，即置标记后没有再执行新的工作项。`responseMs` 是“置标记到 `run_plan` 返回”的整段时间，包含循环结束后的事件建议与收尾；100000 档这一段是 348 秒，主要是大库上的收尾，不是又跑了工作项。
- 全量事件建议在 100000 档是 33.1 分钟，是本档最慢的一步，如实报告，没有为它放宽断言或缩小数据。
- 真实库：未打开、未读写。四张新表是 exposure 自有的 `CREATE IF NOT EXISTS`，只会在应用下次打开 exposure 时建出空表；不改任何既有表。
- 磁盘：| 位置 | 清理前 | 清理后 |
|---|---|---|
| 独立编译目录 `~/.ovi_o3/spec_target` | 7.8G | 删除，0 |
| 规模测试的 release 目录 `~/.ovi_o3/spec_target_rel` | 1.4G | 删除，0 |
| 副本 `~/.ovi_o3/speccopy` | 37M（node_modules 是指向真实仓库的软链，先删软链） | 删除，0 |
| 真实仓库 `src-tauri/target` | 30G | **未动**（按规则不清理） |

## 6. 四类证据

1. **真实外部来源：** G4 没有新的外部来源，也没有发任何外网请求。既有来源的真实验证仍以 G2（3 次匿名仓库搜索）和 G3（2 次公开文档）为准。
2. **fixture（生产执行器同一路径）：** `g4_0_the_production_worker_proposes_one_incident_for_the_same_document_at_two_urls` 走 `execute_exposure_run`，两个地址的同一文档成为一个事件、两个位置。
3. **临时库规模证据：** §5 的三档，release 构建，WAL + synchronous NORMAL。
4. **未验证 / 等 IR：**
   - 事件台账的页面读取与人工动作：IR-15。
   - 集团总账项目的单位级事件统计：IR-02/IR-11。
   - 人工改判自动分类：IR-07。
   - 导入与定级入口：IR-14。
   - GitHub 代码搜索真实验收：IR-04。
   - 并行采集：IR-08（T16 无法验证）。
   - 覆盖矩阵格子点开看原查询与观察、实时块 20 个以后的缺口：未做。
   - “公开领导介绍”等其余正常公开规则：未做（T08 部分）。
   - 撤权后复测被拒的专门用例：未做（T30 部分）。
   - claim 前与保存后的真实 SIGKILL：未做（T37 部分，只有 claim 与 close 之间一处是真实 SIGKILL）。

## 7. 下一阶段

规格要求 G4 后停止本专项。没有下一阶段的实现计划，也不启动模型、Agent 或 Master 改造。剩余事项全部在 IR 清单里（见本文 §9），由主控决定是否装配。

## 8. 规格整体验收（G0–G4 全部完成后）

判定口径：**已满足** = 有可运行的代码路径与测试（或实测）证据；**待 IR** = Grok 侧已做完白名单内能做的部分，剩余必须由主控装配或批准；**未满足** = 本期没有做到（含部分做到），写明缺口。fixture 证据不算真实外部验收。

### 8.1 §17 各阶段退出条件

| 条件 | 判定 | 依据 / 原因 |
|---|---|---|
| G0：实测能力矩阵与共享依赖表；缺口不写已完成；无越界源码改动 | 已满足 | G0 交付 §2/§3；五个阶段回写都经 SHA 守卫，无 FOREIGN EDIT |
| G0：复核 P1（设置 CAS 同事务并发、历史 truncated、schema 错误传播） | 已满足 | `tests_g0` 6 个用例（两连接 barrier 并发、251 版本边界、两种结构故障注入） |
| G0：旧的无 project/revision 复核调用 | 待 IR-01 | 页面与 exposure 私有 api 已不再调用（前端测试证明零调用）；`lib.rs` 注册与共享 `api.ts:66` 导出仍在 |
| G1：授权站点→观察→线索→复核→整改→新观察复测 | 已满足 | `g1_4_the_closed_loop_runs_on_a_real_authorized_site`（127.0.0.1 真实 HTTP，生产工作函数） |
| G1：单位未知 / 专业公司不误分省；旧历史可读 | 已满足 | G1-2 单位投影；G4-1 统计沿用同一规则（`g4_1_multi_unit_*`） |
| G1：逐资产单位归属 | 待 IR-02 / IR-11 | 集团总账项目只能记“未分配单位 / 单位未知” |
| G2：同一路径 localhost 正负向 | 已满足 | `tests_g2`（CT/Wayback/GitHub 生产执行器 → fixture） |
| G2：有凭据时另列真实来源验证；没凭据不伪报 | 待 IR-04 | 仓库元数据有 3 次匿名真实请求；代码搜索与受控读取只有 fixture 证据，未伪报 |
| G3：HTTP 读取可追溯 | 已满足 | 生产可达，2 次真实只读请求 |
| G3：实际导入可追溯 | 待 IR-14 | 函数 + 生产 `run_plan` 已验证，入口未注册 |
| G3：PDF / 文库 / 暗网明确缺口；公开材料可排除 | 已满足 | 能力表 unsupported；`g3_3_*` |
| G4：多位置去重 | 已满足（后端与生产采集路径） | `g4_0_identical_full_text_*`、`g4_0_the_production_worker_proposes_*`；人工确认、合并、拆分的页面入口待 IR-15 |
| G4：多省涉及不重复计入集团总数 | 已满足 | `g4_1_multi_unit_incidents_count_once_at_group_and_province_level`；集团总账项目的单位边仍待 IR-02 |
| G4：几万条后端聚合可用 | 已满足 | 4000 / 20000 / 100000 三档临时库实测（§5），分页目标全部达到 |
| G4：缺口不变绿 | 已满足 | `g4_1_coverage_never_turns_a_gap_green_*`；前端 `coverageCellView` 只认 executed_no_hit；变异 R23/R24/N01 均被杀死 |
| G4：导出可复核 | 已满足 | 已注册的 `verify_exposure_export` 读回行数、SHA、格式、rowKinds；台账汇总行与明细对账（`g4_3_exports_*`） |
| 共享合同未就绪时不在页面暴露必然失败的按钮 | 已满足 | IR-07/IR-14/IR-15 对应入口都显示 integration_pending，前端测试覆盖 |
| G4 后停止，不启动模型 / Agent / Master 改造 | 已满足 | 本次汇报后停止 |

### 8.2 §18 测试矩阵 T01–T42

| 编号 | 判定 | 依据 / 原因 |
|---|---|---|
| T01 省 / 专业子公司 / 城市父子关系分开 | 已满足 | G1-2 `units.rs`；G4 `place_unit` 单列专业公司 |
| T02 弱标签与候选组不当确认 | 已满足 | 只用已确认目录；事件确认关联必须有本事件观察作证据（变异 R14/R22 被杀死） |
| T03 目录 / 绑定变更后旧计划不静默换范围 | 已满足 | 冻结计划 `s2:` 范围指纹（G1-0）；事件与关联记录 directory/bindingVersion |
| T04 未授权资产不发请求 | 已满足 | `q3_an_unauthorized_destination_is_named_not_fetched` 等既有用例 |
| T05 跨项目引用拒绝且零写 | 已满足 | G3 导入/定级、G4 `g4_0_another_project_reads_and_writes_nothing`（变异 R20 被杀死） |
| T06 单位绑定 / 外链发现不创建授权 | 已满足 | GitHub、导入、事件都不建资产/实体/归属（行数断言） |
| T07 招标公告与转载排除且可审计 | 已满足 | G1-3 / G3-3 |
| T08 公开领导介绍 / 开源 / vendor 排除 | 未满足（部分） | 正式开源账号名单（G2）与第三方 vendor 资源（G1-3）已实现；“公开领导介绍”等其余正常公开规则未实现，这类线索落到待分类，由人工判断 |
| T09 混合 / 来源冲突保持待分类 | 已满足 | G1-3 混合页、粘性排除的冲突标记 |
| T10 真实请求解析 / 分页与计划一致 | 已满足（元数据与文档）/ 待 IR-04（代码搜索） | GitHub 仓库搜索 3 次、文档 2 次真实请求；代码搜索无凭据 |
| T11 缺凭据 / 401 / 403 / 429 / 422 / 503 分别展示 | 已满足 | G2 状态表与前端标签 |
| T12 空结果与失败 / 未配置不混同 | 已满足 | completed_no_matches 写“不代表没有泄漏”；覆盖矩阵只给 executed_no_hit 绿色 |
| T13 恶意 URL / 跨 host 重定向不泄 token | 已满足（fixture 证据） | G2 token 防线，整库扫描无 token |
| T14 DNS 私网 / metadata / 端口拒绝 | 已满足 | 既有出站门用例 + G3 端口/内网用例 |
| T15 外部平台不冒充集团资产 | 已满足 | GitHub 观察 asset_id 为空，不进簇和资产 |
| T16 两个实际并发 worker 争最后额度 | 待 IR-08 | 执行器严格串行，设置页如实显示“并行未实现”；没有并发 worker 可测 |
| T17 失败 / 重试计实际尝试 | 已满足 | `q2_budget_is_charged_before_the_socket_*`；G2 每次 HTTP 尝试都扣额度 |
| T18 恢复不重置已用请求 / 字节 | 已满足 | `q3_the_budget_is_persisted_so_a_restart_cannot_spend_it_twice` |
| T19 snippet / 正文 / 索引 / 导入分列 | 已满足 | collectionMode、location_kind、evidence_grade 分列 |
| T20 截断不称全文相同 | 已满足 | G3 截断不写全文哈希；`g4_0_truncated_snippets_*`（变异 R01/R02 被杀死） |
| T21 原观察、规则 / 脱敏版本与晚到响应保留 | 已满足 | 观察只增不改；`q3_a_late_worker_cannot_close_*` |
| T22 相同文件多位置合并有依据 | 已满足 | 事件 basis（identical_normalized_full_text / identical_full_response_body）；人工确认入口待 IR-15 |
| T23 同标题不同内容不合并 | 已满足 | `g4_0_truncated_snippets_*` 同标题用例、G3 same_document |
| T24 跨省涉及只计集团唯一事件一次 | 已满足 | `g4_1_multi_unit_*`（变异 R25 被杀死） |
| T25 expectedRevision / CAS 真实并发只有一方成功 | 已满足 | `g4_0_two_writers_with_the_same_revision_one_wins`（两个连接）、E5 线索复核 CAS |
| T26 同 operationId 异载荷拒绝 | 已满足 | `g4_0_cas_replay_*`（变异 R08 被杀死）、G3 导入/定级 |
| T27 内容 / 规则变化保留旧 verdict 并提示复核 | 已满足 | G3-4；`g4_0_a_new_round_attaches_*`（变异 R05 被杀死） |
| T28 新观察关联到原整改 | 已满足 | G1-4 复测；G4 复测比对按位置给结论 |
| T29 来源失败 / 未看见不自动关闭 | 已满足 | G1-4 修复；`g4_0_handling_*`（变异 R18 被杀死） |
| T30 跨项目 / 撤权 / 旧版本拒绝 | 已满足（跨项目、进行中轮次、旧计划）/ 未满足（撤权的专门用例） | 复测轮次必须属于本项目且已结束；旧指纹明确拒绝。复测启动走既有范围校验，但没有“撤权后复测被拒”的专门用例 |
| T31 项目切换 / 晚到响应 / 轮询取消 | 已满足 | 既有 surface 用例 + `g4 incident pages ... stale project answer is dropped` |
| T32 组内分页不拉全量 | 已满足 | 事件、位置 keyset 分页；规模档实测 |
| T33 待配置 / 失败 / 缺口保持真实状态和分母 | 已满足 | 覆盖矩阵分母；实时块缺口列表 |
| T34 项目 + 导出时间命名 | 已满足 | 带显式时区，唯一后缀（变异 R30/R31 被杀死） |
| T35 公式 / 凭据 / 路径防护 | 已满足 | 脱敏 + 公式前缀；`verify_exposure_export` 路径与符号链接校验（变异 R32 被杀死） |
| T36 来源、单位、排除、版本与风险总数可对账 | 已满足 | 台账 4 条汇总行与明细逐类相等（变异 R33 被杀死） |
| T37 claim 前 / 网络后保存前 / 保存后 finish 前实际 SIGKILL | 未满足（部分） | 只有“claim 与 close 之间”一处是真实 SIGKILL 子进程（`q3_sigkill_*`）；另两处是模拟崩溃，没有真实 SIGKILL |
| T38 已收 / 未知响应不冒充零支出重放 | 已满足 | 额度在发送前落库（q2），恢复不重复花（q3） |
| T39 同项目启动 / 恢复不产生双活任务 | 已满足 | `q3_two_simultaneous_starters_get_one_active_run` + 单活跃轮唯一索引 |
| T40 4000 / 20000 / 100000 三档实测 | 已满足（有前提） | §5；执行器回调是合成的（不发网络），测的是编排、落库、建议、汇总、分页、导出；设备与阈值未在 G0 冻结 |
| T41 旧 schema / 冻结计划原字节与历史读取 | 已满足 | G1/G2/G3 旧哈希不变用例；G4 只增表 |
| T42 未知版本 / 损坏表不补账、不假空结果 | 已满足 | G0 结构故障；`g4_0_an_old_table_missing_columns_is_refused_*` |

### 8.3 其它“必须”

| 要求 | 判定 | 原因 |
|---|---|---|
| 只改白名单；共享文件由主控装配 | 已满足 | 五个阶段都没有改 lib.rs、commands.rs、db*、models*、Cargo/package、共享 api.ts、PSL、组织目录 |
| 不碰真实用户库；规模测试只用临时库 | 已满足 | 全程未打开 |
| 通知 / 下架只登记人工动作，不自动对外联系 | 已满足 | 无发送路径；`contactedByApplication=false`（变异 R17 被杀死）；前端测试扫描无对外联系代码 |
| concurrency 真实生效或如实显示 | 如实显示已满足 / 实现待 IR-08 | 面板写“并行未实现，实际串行” |
| PSL 版本写入聚合依据 | 已满足 | G1-1 |
| 覆盖矩阵轴：单位 × 信息类别 × 来源能力 × 时间窗口 | 未满足（部分）/ 待 IR-02 | 信息类别 × 来源能力 × 最近一轮已实现；单位轴只有项目级，集团总账项目为“未报告” |
| 覆盖矩阵点开可见原查询、分页进度、观察和缺口 | 未满足 | 只给分母与原因；缺口在实时块里列前 20 个 |
| 100% 只表示选定配置下计划执行完 | 已满足 | 页面与矩阵 notice 写明 |
| 热态第一页 ≤2 秒、后续页 ≤1 秒 | 已满足（实测值） | 三档都达到；冷态、RSS、建计划只报告实际值（G0 未冻结设备与阈值，不称“通过”） |
| 取消在下一次发送前生效 | 已满足 | 三档 itemsAfterCancel 均为 0 |
| 实时状态：stage、来源、完成/总数、请求/字节、落盘时间、缺口；未知总量不给假百分比 | 已满足 | `live_run`（变异 R29/N03 被杀死） |
| 候选页说明“为什么关联”“读取还是索引/导入”“证据不足在哪里” | 已满足（线索证据包）/ 待 IR-15（事件详情） | 事件详情已有 basis、evidenceGrade、关联依据，但读取入口未注册 |
| 导出文件名 `<项目名>_暴露面_<本地导出时间>.csv`，rowKind，排除原因与缺口 | 已满足 | G4-3 |
| 复测须有新 run / observation；失败不能自动关闭；人工关闭带理由 | 已满足 | G1-4、G4 record_action |
| 新表只在 exposure/schema.sql；旧表缺列能发现 | 已满足 | check_schema |
| 人工改判自动分类 | 待 IR-07 | 排除线索不能人工改回 |
| 文件 / 情报导入与人工定级生产入口 | 待 IR-14 | |
| GitHub 代码搜索真实验收 | 待 IR-04 | |
| 新前端测试进 package 脚本 | 待 IR-10 | g1/g2/g3/g4 四个文件需单独跑 |
| Agent 只预留文档接口，本期不实现 | 已满足 | 未实现任何模型调用 |

## 9. IR 总清单（按优先级）

| 优先级 | IR | 内容 | 不批准的影响 |
|---|---|---|---|
| P0 | IR-15 | 注册 5 个事件台账命令（G4） | 自动建议的事件在页面上读不到，也不能复核、合并、拆分、登记处置 |
| P0 | IR-01 | 退役无版本的 `review_exposure_finding`（lib.rs + 共享 api.ts） | 任意 finding id 可被无 CAS、无理由地改状态（页面已不调用，但入口仍在） |
| P0 | IR-14 | 注册文件/情报导入与人工定级命令（G3） | 导入与定级没有生产入口 |
| P1 | IR-07 | 人工改判自动分类（新命令） | 误排除的线索无法改回，也就进不了事件 |
| P1 | IR-04 | GitHub 凭据引用端口 | 代码搜索与受控读取只能 unconfigured，无真实验收 |
| P1 | IR-02 / IR-11 | 逐资产单位权威边 | 集团总账项目的单位/省级统计全部“单位未知” |
| P1 | IR-08 | 并行采集：实现或从合同移除 | T16 无法验证；设置字段语义悬空 |
| P1 | IR-10 | 把 test_exposure_g1–g4.cjs 加进 package 脚本 | 新前端测试不在统一脚本里，容易漏跑 |
| P2 | IR-16 | “取消”的轮次是否允许恢复（产品决策） | 不决定则保持现状：可按冻结计划恢复 |
| P2 | IR-12 | GitHub 来源启用与额度是否需主控确认 | 保持默认不启用、上界写死 |
| P2 | IR-13 | ExternalSourceScope 完整对象的存放位置 | 现状由代码常量与项目级精确地址清单限定 |
| P2 | IR-03 | PSL / 组织目录升为声明过的 `pub(crate)` 只读端口 | 现状依赖 include! 结构可用，不阻塞 |
| — | IR-06 | 新 IPC 注册流程总条目（各阶段补记） | 具体请求见 IR-07/14/15 |
| — | IR-05 | 已由 IR-10 具体化 | — |
| 已关闭 | IR-09 | CT/Wayback/GitHub 端点注入 | G2 已在白名单内完成，无需主控处理 |

## 附录 A：回写文件 SHA-256（真实仓库，回写后与副本逐字节一致）

| 文件 | 行数 | SHA-256 前 16 位 |
|---|---|---|
| `docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md` | 217 | `bf089f76620e10a9` |
| `src-tauri/src/commands/exposure.rs` | 1422 | `6b16352535f827a7` |
| `src-tauri/src/commands/exposure/coverage.rs` | 511 | `a7e826196be80d7b` |
| `src-tauri/src/commands/exposure/export.rs` | 563 | `f9deff304cee944e` |
| `src-tauri/src/commands/exposure/incidents.rs` | 1332 | `f3a0f8760afab68e` |
| `src-tauri/src/commands/exposure/live.rs` | 116 | `357dff54c78b385d` |
| `src-tauri/src/commands/exposure/orchestrator.rs` | 911 | `11b7a09a307d6f47` |
| `src-tauri/src/commands/exposure/schema.sql` | 431 | `0482b01eb4d3d565` |
| `src-tauri/src/commands/exposure/summary.rs` | 168 | `9f2c4b45d2e59279` |
| `src-tauri/src/commands/exposure/tests_g4.rs` | 895 | `5324992537eda369` |
| `src-tauri/src/commands/exposure/tests_g4_scale.rs` | 285 | `a729138b2a739768` |
| `src/components/ExposureSurface.vue` | 363 | `5c52d0051bde2125` |
| `src/exposure-tabs.css` | 2 | `e744d7848ace0384` |
| `src/features/assets/exposure/ExposureIncidentsPanel.vue` | 140 | `6fde3fa34ef41fd0` |
| `src/features/assets/exposure/api.ts` | 334 | `517c31385a48cb1c` |
| `src/features/assets/exposure/contracts.ts` | 957 | `ae3f0837dea74b3b` |
| `src/features/assets/exposure/incidents.ts` | 330 | `d9aefad5eccb446f` |
| `src/features/assets/exposure/useExposureIncidents.ts` | 202 | `0be18b5a86100f22` |
| `src/features/assets/exposure/useExposureSurface.ts` | 561 | `4447919ffb2c77af` |
| `tools/test_asset_exposure_entry.cjs` | 320 | `694ec7f4618a8ea2` |
| `tools/test_asset_exposure_evidence.cjs` | 375 | `9f944ef19b5ce7fd` |
| `tools/test_asset_exposure_page.cjs` | 273 | `bc548d0a4861d085` |
| `tools/test_exposure_g4.cjs` | 233 | `239d39625181e891` |

本文本身也经 syncback 回写；它的 SHA 记录在 `~/.ovi_o3/g4/synced.sha`。

## P0 落地（2026-10-09）

用户批准 IR 清单后，本轮只落地 P0，未 commit。IR 文档末尾有同日记录。

| IR | 状态 | 落点 |
|---|---|---|
| IR-15 | 已落地 | `lib.rs` 注册 5 个事件台账命令；`exposure/api.ts` 增加 `proposeExposureIncidents`；面板按钮「为历史轮次补建议」。缺命令时仍显示 integration_pending。不自动联系任何人。 |
| IR-01 | 已落地 | 全库确认无其它生产调用方后，删除无版本 `review_exposure_finding` 的注册、共享 `api.ts` 导出和函数体。`review_exposure_finding_scoped` 保留。`ExposureFindingReviewInput` 留在 `models_appsec.rs`（未批准改动），clippy 因此有一条 dead_code，不在 exposure 代码里。 |
| IR-14 | 已落地 | 注册 `import_exposure_documents`、`grade_exposure_intel`；调用在 `exposure/api.ts`；设置页导入表单与证据包定级表单可用。声明 URL 只记录不访问。能力行不再写「入口未注册」。 |

验证：前端测试 97 通过，`vue-tsc --noEmit` 通过，`cargo test --lib exposure` 179 passed / 3 ignored，`cargo check --lib` 通过，clippy 通过（仅上述 models 警告）。变异 8/8 杀死并还原。

P1 未做：IR-07 人工改判、IR-04 GitHub 凭据端口、IR-02/IR-11 逐资产单位边、IR-08 并行度、IR-10 测试脚本（含本轮 `tools/test_exposure_ir.cjs`）。


## P1 交付（2026-10-09）

完整记录见 `ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md` 末尾的「P1 落地记录」。

- 已落地：IR-01 收尾（删除 `ExposureFindingReviewInput`）、IR-10（`npm run test:exposure` 等脚本）、IR-07（人工改判：CAS、operationId、理由、审计表 `exposure_reclassifications`）、IR-08（从合同和面板移除不生效的并行度）、IR-04（钥匙串只读凭据引用；只用 fixture 验证，未宣称外部完成）。
- 阻塞：IR-02/IR-11（逐资产单位边需要归属模块提供只读投影，归属冻结，等待批准）。
- 验证：`cargo test --lib exposure` 181 passed、3 ignored；check 与 clippy 0 warning；`npm run test:exposure` 99/99；vue-tsc 通过；变异 17/17 被杀死。
