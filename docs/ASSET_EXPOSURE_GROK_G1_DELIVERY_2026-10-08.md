# Asset 暴露面 Grok G1 交付：一省/一单位现有来源闭环（G1-0 至 G1-4）— 2026-10-08

- 执行者：Grok
- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`，HEAD `14a7875`。开始、中途（含两次 Mac 掉线/会话中断后）和结束都核对过；未 commit/push/stash/reset/checkout/merge。
- 编译与测试：在隔离副本 `~/.ovi_o3/speccopy` 中进行（rsync 用锚定排除 `/src-tauri/target`、`/target`、`/node_modules`、`/.git`），`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`，构建放在 screen 里跑（掉线不会杀掉构建）。每片在副本中开发、测试、变异，通过后由 `syncback.py` 回写真实仓库：只写白名单文件，而且只有当真实文件的 SHA 等于 T0 快照或上次回写值时才写，否则报 FOREIGN EDIT 并停止。G1 全程没有出现 FOREIGN EDIT。
- 前端测试：副本和真实仓库各跑一次（只读 node_modules，不写数据库）。
- 真实用户库 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 全程未打开。Rust 测试全部使用 tempdir 临时库；网络只连 127.0.0.1 上的 fixture。

## 1. 用户行为与生产调用链

G1 没有新增 IPC，所有行为都挂在 lib.rs 已注册的命令上，只扩展了载荷（见 IR-06 补记）。

**G1-0 设置与限额的真实性**
- 采集轮：`start_exposure_scan` → 后台 `execute_exposure_run` → `recovery::load_plan(...)?` → `frozen_settings_strict` → `exposure_http_limits(&frozen)` / `meter_for(&plan)` → `orchestrator::run_plan`（再次严格读取 project_id、冻结配置、scope_version 和 sources_json）。
  - 计划缺失、空、损坏时，分别以 `[exposure_plan_missing]`、`[exposure_plan_settings_missing]`、`[exposure_plan_settings_corrupt]`、`[exposure_plan_sources_corrupt]` 失败，并且一个请求都不发；`finalize` 把这一轮写成 failed，不会停在 running。
  - 不再静默用 12 秒 / 1MB / 200 行等默认值。
- CT：`certificate_transparency::execute_bounded(…, ct_page_size)` 只保留前 M 个名称；被截断时这一项记为 BlockedQuota + Partial，原因写明“证书透明度返回 N 个名称，按配置只保留前 M 个，其余 K 个未记录”。Wayback 行数和访问队列都读设置（`ARCHIVE_INDEX_LIMIT` 已删）。
- 汇总 `exposure_scope_summary`：
  - 计划数 = min(准入数, 本项目 `max_items`)；`limitsBasis` 是 `project_settings_vN` 或 `defaults`。
  - 配置损坏时 `limitsBasis=settings_corrupt`、`knownLimits=[]`、计划 0，并附上 `limitsError`。
  - 前端删除了写死的 `PASSIVE_ASSET_CAP=200`；旧接口回退时显示“计划数未核实”。
- 设置视图 `get_exposure_settings` 增加 `effective`（concurrency=1、concurrencyImplemented=false 和 7 行 knownLimits）。面板的并行度改为“并行度（未实现，实际串行）”并禁用输入；CT/归档/工作项的单位写成“名称/域名”“行/域名”。实现并行属于 IR-08。
- 附带修复（G1-2 期间由负载下的偶发失败暴露，是真缺陷）：冻结范围指纹包含秒级的 `issuedAt`，所以晚一秒以后的恢复都会被误判为“范围已变化”。现改为 `s2:` 身份指纹（去掉 issuedAt/validUntil）。G1 之前的旧指纹明确报 `exposure_plan_scope_hash_legacy`，不说成范围变化。

**G1-1 聚类改用共享 PSL**
- `clusters::registrable_boundary` 只读调用 `crate::commands::ownership_domain_parts`，边界依据记为 `psl:<OWNERSHIP_PSL_VERSION>`。公共后缀本身和 IP 退为按精确主机分组（簇键 `host:`）。
- 观察写 `root_basis`，簇写 `boundary_basis`/`psl_version`（state.rs 迁移加列，新增索引）。
- 旧簇（依据为空，即旧的“末两段”规则）保持可读，显示“旧版末两段分组（G1 之前，未按公共后缀，可能误合并）”，不会被并入新簇。
- 页面：簇面板显示“同主机”和“分组边界：…”。

**G1-2 AssetUnitProjection（只读）加 unassigned 兜底**
- `exposure_scope_summary.unitProjection` → `units::project_units`，只读，用 total_changes 验证零写。数据来源是 `load_project_org_binding`、`load_org_entities`（只含已确认实体）和 `org_binding_scope`。
- 规则：
  - 未绑定、集团总账、绑定失效时，确认资产全部记为“未分配单位”，不按名称或地区码推断。
  - 单位项目的确认资产才算本单位资产。
  - 父链只经过市/县分公司向上走：遇到专业公司就停止，并记为“专业公司，不归入任何省公司”；父链成环或缺父级都写入 notes。
  - 计划、已执行、未覆盖都按最新一轮计算，撤销准入的资产不能顶替新资产的覆盖。
- 页面：总览新增单位列表，写明“逐资产单位归属：integration_pending（IR-02/IR-11）”。

**G1-3 分类与招标排除（规格 §6/§7）**
- 位置：生产执行器 `run_plan` 在每次 `merge_finding` 之后调用 `classify::page_context`（看页面本身：`<title>`/`<h1>` 招采字样、15 个互不包含的结构字段、公认招采平台 host 按后缀精确匹配、路径或文件名字样）和 `classify::classify`，再由 `classify::record` 写库。
- 只有索引行、没有页面正文的线索（合成观察）只能按地址判断。
- 输出：信息类别 9 种（规格 8 类加 unclassified）；来源渠道（规格 §6 的 7 个，加 unrecognized_source）；处置 4 种（`risk_candidate`、`classification_pending`、`excluded_public_procurement`、`excluded_public_material`）；原因；markers；`policy_version=oviraptor.exposure.classify.v1`。
- 判定顺序：
  1. 混合页面（招采页面里同时出现凭据候选、业务源码图或存储地址）→ 待分类
  2. 公认平台加任一字样，或标题字样加至少 2 个结构字段 → 公开招采排除
  3. 只有标题字样 → 待分类
  4. 只有文件名或路径字样 → 待分类
  5. 第三方 Source Map、空图、非图、占位凭据 → 公开材料排除
  6. 没有信息类别 → 待分类
  7. 其余 → 风险候选
- 存储：`exposure_classifications`（每次观察按策略版本一行，旧行不改写）和 `exposure_finding_dispositions`（当前处置）。
  - 排除是粘性的：后续观察判成别的，也只置 `conflict` 标记并写说明，不自动撤销（§7“已明确属于正常公告的记录不得自动解除排除”）。
  - 人工已确认的线索不会被自动排除，置冲突并保持待分类。
- 影响：
  - `highRiskOpen` 不计入排除线索。汇总新增 `findingsExcluded`、`findingsClassificationPending`、`findingsUnclassified`（G1 之前的旧线索）。
  - taxonomy 新增 `auto_excluded` 行。
  - `save_exposure_remediation` 对排除且未经人工确认的线索返回 `exposure_remediation_excluded`。
  - 证据包附带当前处置和分类记录。
  - 线索分页新增 `disposition` 筛选（未知值报 `[exposure_disposition_invalid]`）和 `dispositions` 旁表。共享的 `ExposureFinding` 在 models_appsec.rs，不能加字段，所以用旁表；页字段保持平铺。
- 排除只减少风险计数，不从覆盖分母里扣除已执行的查询（单位投影里的已执行和未覆盖不受分类影响）。
- 页面：
  - 列表每行显示处置徽标（排除、待分类、风险、旧线索未分类；后端未报告时显示“处置未报告”）。
  - 列表新增处置筛选；总览有一行写明“自动排除 N（…不计入高风险）· 待分类 N · 旧线索未分类 N”。
  - 复核面板对排除线索不显示“登记整改”，改为显示 `integration_pending（IR-07）` 说明。
- 尚缺：人工改判通道（IR-07）；公开领导介绍、正式开源资料等其余“正常公开”规则没有实现，这类线索会落到待分类或风险候选，需要人工判断。

**G1-4 闭环（真实执行路径）**
- `g1_4_the_closed_loop_runs_on_a_real_authorized_site` 把整个闭环跑在 127.0.0.1 的真实 HTTP 站点上，经过生产工作函数 `execute_exposure_run`。站点有首页、招采公告页（带模板凭据行）、第三方 Source Map 和真实凭据配置文件。
  1. 采集：三条线索分别判为风险候选（凭据）、公开招采排除、公开材料排除。高风险待办 1，排除 2。单位投影把绑定到“咪咕浙江杭州分公司”的项目放在专业公司下，不归入浙江。
  2. 复核：`review_scoped` 引用真实观察 id 确认。
  3. 整改：排除的公告被拒绝；凭据整改登记成功；没有复测时关闭被拒绝。
  4. 复测 1：`start_retest` 新开一轮，取回同样字节，结果 `retest_content_identical`，关闭被拒绝。
  5. 复测 2：文件返回 404 新字节，结果 `retest_content_changed`，按依据关闭；线索回到 reviewing，不宣布“已修复”。
  6. 旧历史：第一轮观察字节不变；三轮各有分类记录；招采排除在复测轮里保持；后插入的 G1 之前旧线索在“未分类”筛选和证据包里照常可读。
- 负向：
  - 站点整体下线时复测不能关闭（`g1_4_a_retest_against_a_site_that_went_down_cannot_close`）。
  - 单个地址连接被断开时复测不能关闭（`g1_4_a_retest_that_cannot_fetch_the_url_is_not_a_content_change`）。
- **G1-4 找到并修复的缺陷：** `retest_outcome` 先比较哈希、后判断传输失败。而传输失败的观察 status=0、内容哈希是空字节的 `e3b0c442…`，与原哈希不同，于是被判成 `retest_content_changed / canClose=true`：复测取不到，整改却可以被关闭。这违反 T28–T30。
  - 先用真实 fixture 复现：测试红，日志 `g14b.log`。
  - 再把“取不到”的判断移到字节比较之前：测试绿。
  - 变异 G14-M3 第一次存活，正是指向这个缺口。
- 仍缺：页面上的复测是由用户逐步点击完成的，G1 没有“一键闭环”；版本冲突的复核已由 E5 的 CAS 测试覆盖，G1 没有重复。

## 2. 文件、SHA、白名单检查、共享依赖

**改动文件（SHA-1 前 10 位，取结束时真实仓库的值；全部在白名单内）**

| 文件 | SHA-1 | 片 | 改动 |
|---|---|---|---|
| src-tauri/src/commands/exposure.rs | 720b3321ab | 0/2/3 | 严格计划读取、`exposure_http_limits`、ct_limit、settings.effective；`pub mod classify/units`；`ExposureFindingPageView` 旁表；分页走 `exposure_connection` |
| exposure/adapters/certificate_transparency.rs | 4728cc10b1 | 0 | `execute_bounded`，截断报额度缺口 |
| exposure/settings.rs | 2dd895a905 | 0 | `from_frozen_strict`、默认常量、单测 |
| exposure/contract.rs | e75a3d5faf | 0/3 | `known_limits_for`；分页入参 `disposition`、查询 `disposition` |
| exposure/orchestrator.rs | 0f2af42689 | 0/2/3 | 严格计划、finalize 失败落库、`s2:` 范围指纹、分类挂点 |
| exposure/recovery.rs | 98262033bb | 0/2 | 严格读取、旧指纹明确拒绝 |
| exposure/summary.rs | 2f652bf2b9 | 0/2/3 | limitsBasis/Error、unitProjection、分类计数、highRisk 不计排除 |
| exposure/clusters.rs | e7ddcca2b5 | 1 | PSL 边界 |
| exposure/cluster_repository.rs | 6c5096e9e3 | 1 | 边界字段、旧行隔离 |
| exposure/observations.rs | e19f57b3b5 | 1 | root_basis |
| exposure/state.rs | f58ee3b878 | 1/3 | 迁移列/索引；表清单加两张新表 |
| exposure/schema.sql | 697be2eab3 | 3 | `exposure_classifications`、`exposure_finding_dispositions` |
| exposure/units.rs（新） | 490cbeccb7 | 2 | 单位投影 |
| exposure/classify.rs（新） | 46386a7f0b | 3 | 分类、招采排除、粘性处置、旁表读取 |
| exposure/rules.rs | d49f14b9a8 | 3 | 标题和占位置信度常量化（行为不变） |
| exposure/remediation.rs | 3a5b412a25 | 3/4 | 整改拒绝排除线索；taxonomy auto_excluded；**复测“取不到”先于字节比较** |
| exposure/repository.rs | 3b2ae02901 | 3 | 处置筛选子句（只在有筛选时编译）；证据包分类字段 |
| exposure/tests.rs、tests_e4.rs、tests_q3.rs、tests_g0.rs | dd322195fe / 5850044fef / aaf739a5f5 / 09259cd98c | 0/1/3 | 字面量补字段、冻结配置、legacy 规则测量；不放松断言 |
| exposure/tests_g1.rs（新） | 89e3367076 | 0–4 | 17 个 G1 测试 |
| src/components/ExposureSurface.vue | 70d39de19d | 0/2/3 | 限额行、单位列表、分类行、处置徽标和筛选 |
| src/exposure-surface.css | 04f07df139 | 3 | 处置徽标、筛选、integration_pending 样式 |
| features/assets/exposure/contracts.ts | e0a5cca183 | 0/2/3 | 严格读取器与文案函数 |
| features/assets/exposure/useExposureSurface.ts | e0f42d12bf | 0/2/3 | 去掉假 200；单位/分类/处置状态 |
| features/assets/exposure/api.ts | bbd94c764d | 0/1/3 | 类型扩展 |
| features/assets/exposure/useExposureSettings.ts、ExposureSettingsPanel.vue | e9dc4b5197 / 04bfd7f992 | 0 | 串行说明、禁用并行度 |
| features/assets/exposure/useExposureClusters.ts、ExposureClustersPanel.vue | 0ae47952fb / 9091657961 | 1 | 边界文案 |
| features/assets/exposure/useExposureReview.ts、ExposureReviewPanel.vue | 3a72724642 / a3bc0f4178 | 3 | 排除线索不提供整改 |
| tools/test_asset_exposure_surface.cjs | 18fb1c4218 | 0 | 旧接口回退断言改为“未核实” |
| tools/test_exposure_g1.cjs（新） | 8f74ec56a9 | 0–3 | 9 个前端用例 |
| docs/ASSET_EXPOSURE_GROK_G1_DELIVERY_2026-10-08.md（本文）、docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md（只追加） | — | — | — |

**白名单核对**
- 没有改：lib.rs、commands.rs、db*.rs、models*.rs、src/types、Cargo/package 文件、归属文件、共享 PSL、Ownership*.vue、AssetWorkspace、App.vue、agent_*/native_*/multi_agent、主计划/进度文档、规格原文。
- `git status` 里 package.json、Cargo.toml、agent_runtime 等文件的修改来自并行的 Codex，不是本任务的改动。
- 新表只在 `exposure/schema.sql`，迁移在 `state.rs`。没有批量格式化；旧冻结计划字节未改写。

**共享依赖（只读复用，不复制）**
- `ownership_domain_parts`、`OWNERSHIP_PSL_VERSION`、`org_directory_version`、`load_org_entities`、`load_project_org_binding`、`org_binding_scope`。
- 可见性依靠“exposure 是 commands 的子模块”（见 IR-03），不需要改任何共享文件。

## 3. 来源能力矩阵

| 来源 | 已实现 | 已注册（start/resume） | 生产可达 | 凭据 | G1 验证 |
|---|---|---|---|---|---|
| passive_web | 是 | 是 | 是 | 无 | 生产执行器加 127.0.0.1 真实 HTTP（G1-4 共 3 个测试、6 个真实轮次）；未对外部目标扫描 |
| certificate_transparency（crt.sh） | 是（G1-0 起按 ct_page_size 截断） | 是 | 是 | 无 | 适配器层 localhost 截断测试；执行器层基地址写死，无法指向 fixture（IR-09）；G1 未重新对外部实测（G0：200） |
| web_archive（Wayback） | 是（行数读设置） | 是 | 是 | 无 | 同上（G0：200） |
| GitHub 元数据/代码 | 否 | 否 | 否 | 代码搜索需 token（IR-04） | G2 |
| document_import / document_link / public_intel_import | 否 | 否 | 否 | — | G3 |

分类的来源渠道里，`code_platform`、`document_*`、`public_intel_import` 只有名字，没有生产来源。没有映射的来源键记为 `unrecognized_source` 并进入待分类。

## 4. 测试命令、结果、计数、重复

| 项 | 命令 | 结果 |
|---|---|---|
| Rust 基线（G0 结束） | `cargo test --lib exposure` | 101 passed |
| G1-0 后 | 同上 | 108 passed |
| G1-1 后 | 同上 | 110 passed |
| G1-2 后 | 同上 | 114 passed |
| G1-3 后 | 同上 | 117 passed |
| G1-4 复现缺陷 | `cargo test --lib g1_4_` | 2 passed、1 failed（预期的红，`g14b.log`） |
| G1-4 修复后 | `cargo test --lib exposure` | 120 passed |
| 重复复验 | `cargo test --lib exposure` ×3 | 3/3 次 120 passed（39–41 秒，`rerepeat1–3.log`）。第一次重复是 3 次 119/1：变异工具恢复文件时写回旧 mtime，cargo 没有重编，测的是带最后一个变异（G14-M8）的旧二进制，失败内容与 G14-M8 完全一致。已改工具（恢复时用当前 mtime），touch 后重编复验全绿。变异判定本身不受影响：每个变异都写新 mtime，会触发整 crate 重编 |
| clippy | `cargo clippy --lib --tests` | exit 0；全库 53 个 warning（G0 基线 70，差额来自并行改动的其它模块）；暴露面文件 0 个 |
| 前端（副本和真实仓库） | `npm run test:asset-exposure` | 62/62 |
| 前端 G1 | `node --test --test-concurrency=1 tools/test_exposure_g1.cjs` | 9/9 |
| 类型 | `npx vue-tsc --noEmit -p tsconfig.json` | exit 0 |

- 独立新增用例：Rust 19 个（tests_g1.rs 17 个，settings.rs 和 contract.rs 各 1 个），前端 9 个。
- 失败与修复记录：
  - q3 SIGKILL 测试的计划没有冻结配置，补齐。
  - 一项目一活跃轮的约束导致 G1-0 测试互相阻塞，改为每个用例收尾。
  - finalize 在 sources_json 损坏时停在 running，已修。
  - 负载下 resume 偶发失败，查出 issuedAt 进入范围指纹的真缺陷，已修并加测试。
  - G1-4 复测“取不到”被判成内容变化的真缺陷，已修并加测试。
- 未运行项：真实外部来源的采集（G1 不对外部目标扫描）；规模三档（G4）。

**变异（共 53 个：52 个被杀死，1 个等价）**

方法：在副本中逐一注入变异，跑对应过滤的 Rust 测试或 cjs 文件，然后用 copy2 加 utime 恢复并核对 SHA；每条都记录 restored=true，结束后没有残留的 `.mutbak`。

| 片 | 数量 | 结果 | 说明 |
|---|---|---|---|
| G1-0 | 10 | 9 杀死 | G10-M7（执行器不把 ct_limit 传给适配器）是等价变异：`run_plan` 的严格检查在任何请求之前就拒绝了这一轮，执行器层基地址无法注入（IR-09） |
| G1-1 | 7 | 7 杀死 | 末两段回退、basis 丢失、旧行并入、host 键退化、前端边界文案等 |
| G1-2 | 8 | 8 杀死 | 专业公司继续向上走、集团总账冒充单位、未绑定冒充集团、待授权、未覆盖口径、失效绑定、范围指纹含 issuedAt、前端专业公司文案 |
| G1-3 | 20 | 20 杀死 | Rust 14 个：混合页、结构字段阈值、标题扫全文、仿冒平台、仅文件名排除、粘性、人工优先、高风险口径、整改拒绝、未知筛选、占位凭据、旧线索筛选、旧库缺表、合成观察不分类。前端 6 个：旧线索显示成风险、接受未知处置、不发送筛选、翻页覆盖旁表、整改拦截失效、旧后端显示 0 |
| G1-4 | 8 | 8 杀死（首轮 6/7） | 同字节可关、没看到可关、**取不到可关（首轮存活 → 补测试 → 查出缺陷 → 修复 → 重跑杀死）**、关闭后直接确认、内容变化不可关、生产执行器分类不看正文、真实路径整改不拒排除，以及新增的“传输失败落到字节比较之后” |

日志在 Mac 的 `~/.ovi_o3/logs/g1/`：g12b、g13a–d、g14a–c、mut_g1{0,1,2,3,4,4b}.jsonl、clippy.log、repeat1–3.log。

## 5. 临时库规模、性能与真实库声明

- 规模：G1 用例是功能性规模，每个临时库几十行。G1-4 共 3 个测试、6 个真实采集轮次，每轮经生产执行器向 127.0.0.1 发 4 到 7 个请求。规模三档（4000 / 20000 / 100000）属于 G4。
- 性能：全量 exposure 测试 38–45 秒；`g1_4_` 三个测试合计约 0.4 秒；`g1_3_` 三个测试约 0.3 秒。单个变异 12–40 秒（含增量编译）。
- 取消与恢复：`g1_0_a_resume_in_a_later_second_still_matches_its_frozen_scope` 跨过一个整秒，恢复通过；真实的范围扩大仍被拒绝；旧指纹明确拒绝。G0、Q3 的 SIGKILL 和恢复测试仍然全绿。
- 真实库：`/Users/swyiic/oviraptor/oviraptor.sqlite3` 未打开、未读写。schema 变更只在测试临时库里执行；生产库要等应用下次启动、`ensure_exposure_schema` 运行时才会建两张新表（CREATE IF NOT EXISTS，不改旧表）。
- 磁盘：

| 位置 | 删除前 | 删除后 |
|---|---|---|
| `~/.ovi_o3/spec_target` | 11G | 已删除 |
| `~/.ovi_o3/speccopy` | 36M（node_modules 只是指向真实仓库的符号链接，先删链接，真实 node_modules 完好） | 已删除 |
| `~/.ovi_o3` 合计 | 11G | 2.0M（日志、脚本、T0 指纹） |
| 真实仓库 `src-tauri/target` | 16G | 按规则未清理（Codex 在用） |

## 6. 四类证据

1. **真实功能（生产路径实跑）**：
   - G1-4 闭环经 `execute_exposure_run` 跑在真实 HTTP 上：采集、分类、复核、整改、两轮复测、按依据关闭。
   - 计划损坏时整轮失败且零请求；CT 截断报额度缺口；PSL 分组；单位投影；招采/公开材料排除与粘性；高风险口径。
   - 两个真缺陷已修复：范围指纹含秒级签发时间；复测取不到被当成内容变化。
2. **未接入骨架或待集成**：
   - 人工改判（IR-07）、并行采集（IR-08）、CT/Wayback 端点注入（IR-09）、新测试脚本（IR-10）、逐资产单位边（IR-02/IR-11）。
   - 页面对应位置显示 integration_pending，不显示会失败的按钮。
   - 来源渠道 `code_platform`/`document_*`/`public_intel_import` 只有名字。
3. **只读诊断**：单位投影（零写已断言）；证据包的分类记录；分类计数和“旧线索未分类”；簇的边界说明；`effective` 设置视图。
4. **测试证据**：Rust 101 → 120；重复 3 次；前端 62/62 + 9/9；vue-tsc 0；变异 52/53（1 个等价）；clippy 见 §4。均为 localhost fixture 或临时库证据，不代表外部真实来源验收。

## 7. 下一阶段（G2 GitHub）

1. **G2-0 来源适配器端口与 fixture 注入**：先在白名单内做 `#[cfg(test)]` 的基地址注入（IR-09），让 CT、Wayback、GitHub 都能走“生产执行器到 localhost”的同一路径（T10–T12）。
2. **G2-1 GitHub 仓库元数据搜索（匿名）**：
   - 查询来自单位投影和已确认的注册域；分页和额度与计划一致。
   - 401/403/429/422/503 分别作为来源状态，空结果和失败不混同。
   - 结果渠道 `code_platform`，外部平台不冒充集团资产（T15）。
3. **G2-2 代码搜索**：依赖 IR-04 的 credentialRef 秘密端口。没有凭据时显示 `unconfigured`，不发请求。
   - 恶意 URL 和跨 host 重定向不能泄露 token（T13）；日志、证据、CSV 都不留明文。
4. **G2-3 受控读取与分类**：命中片段进入观察；G1-3 的分类和排除直接复用；第一方源码或凭据组合证据才能成为风险候选。
5. **G2-4 闭环与交付**：localhost 正负向闭环测试和变异。有凭据时另列一次真实 GitHub 验证；没有凭据时不报外部完成。按七节结构交付 G2 文档。

依赖：IR-04（必要）；IR-07、IR-09（建议在 G2 前确认）。
