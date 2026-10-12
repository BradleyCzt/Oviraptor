# Asset 暴露面（Grok）共享集成请求 — 2026-10-08

> 状态：G0 起草（2026-10-08 22:36 UTC+8）。条目编号固定，后续各片只追加不改号。

由 G0 起草，后续各片追加。本文件只提请求；共享文件（lib.rs、commands.rs、db_schema.rs、共享 api.ts/models、package.json、归属模块）一律由主控串行装配。Grok 没有修改任何共享文件。

## IR-01 旧复核入口 `review_exposure_finding` 退役（P1-4）
- **现状：** lib.rs:410 仍注册该命令；`src/features/assets/api.ts:66` 仍导出 `reviewExposureFinding`。它没有 projectId、expectedRevision、operator 或 reason，任意 finding id 都能改状态。
- **Grok 已做（白名单内）：**
  - 暴露面页面（ExposureSurface.vue、useExposureSurface.ts）不再调用它。列表按钮改为打开 scoped 复核面板。
  - 前端测试证明旧入口一次都不会被调用。
  - 后端命令保留原行为，只是先校验 schema。
  - 暴露面私有 `src/features/assets/exposure/api.ts` 的 `reviewExposureFinding` 导出已删除，并加注释指向 IR-01。共享的 `src/features/assets/api.ts:66` 不在白名单内，未动。
- **请求：**
  1. 从 lib.rs 的 invoke_handler 移除 `commands::review_exposure_finding`。
  2. 从 `src/features/assets/api.ts` 移除 `reviewExposureFinding`。
  3. 之后由 Grok 删除 exposure.rs 中的函数体。删除前后都保留 `review_exposure_finding_scoped`（已注册）。
- **调用方：** 无剩余生产调用方（G0 用 grep 核对：src/、tools/ 只剩测试桩）。
- **错误：** 退役后调用会得到 Tauri 的 unknown command。页面已改走 scoped，不受影响。
- **测试：** tools/test_asset_exposure_surface.cjs 的 "never writes a review through the legacy unversioned entry"；主控装配后跑 `npm run test:asset-exposure` 和 vue-tsc。
- **风险：** 历史复核记录只读，不受影响；不会制造授权绕过（移除的正是无版本入口）。

## IR-02 AssetUnitProjection：资产→单位的权威边（G1 依赖）
- **现状：** `asset_ownership_decisions` 没有 entity 列。`project_org_bindings` 只把整个项目绑定到集团或单位（`unit_entity_id`）。`ownership_review_groups.candidate_entity_id` 只是候选。
- **影响：** 规范要求的"确认单位资产数"目前只能按"单位项目（unit_entity_id 非空）× 该项目已确认资产"推导。集团项目里的资产没有逐资产的单位边，只能显示 `unassigned`（单位未确定）。
- **请求：** 由主控/归属模块提供只读投影：每个资产的 ownerEntityId、关系类型、证据和版本。名称与字段待主控定。在此之前，Grok 在 G1 只用项目级绑定，并把其余标 `unassigned`，不从候选组或地名推断。

## IR-03 共享只读端口：PSL 与组织目录（DomainBoundaryPort / DirectoryProjection）
- **现状：** G0 测试 `g0_shared_psl_and_directory_are_reachable_read_only` 证明暴露面模块可以只读调用下列函数，不复制、不写库：
  - `crate::commands::ownership_domain_parts`、`OWNERSHIP_PSL_VERSION`
  - `org_directory_version`、`load_org_entities`、`load_project_org_binding`
- **为什么能调用：** 它们是 `commands` 命名空间里的私有函数，exposure 是 `commands` 的子模块。这依赖 include! 结构，不是声明过的合同。
- **请求（不阻塞）：** 主控择机把它们标成 `pub(crate)` 只读端口，或在 asset_ownership 侧提供 facade 并写明版本语义。现状下 G1 可以直接复用，不需要改任何共享文件。

## IR-04 GitHub 凭据的秘密引用端口（G2）
- **现状：** 2026-10-08 22:14（UTC+8）实测：
  - `api.github.com/search/repositories` 匿名 200
  - `/search/code` 匿名 401 "Requires authentication"
- **请求：** 一个 credentialRef → token 的只读秘密端口。配置、日志、证据、CSV 都不保存明文。
- **缺少时：** G2 的代码搜索显示 `unconfigured`，只做匿名元数据搜索。具体字段在 G2 前补充。

## IR-05 新测试脚本进 package.json（后续）
- G0 没有新增测试文件，用例都加进了 `test:asset-exposure` 已包含的文件。
- 后续新增的 `tools/test_exposure_*.cjs` 先用 `node --test --test-concurrency=1 <文件>` 单跑，由主控合入脚本。

## IR-06 新 IPC 注册（G1 起逐片追加）
- G0 没有新增 IPC。
- 后续每片列出：命令名、参数、返回、错误码、调用方、测试。
- 注册前，前端对应入口显示 `integration_pending`，不显示会失败的按钮。

---

> G1 追加（2026-10-08 23:59 UTC+8）。IR-01 至 IR-06 原文未改。G1 没有新增 IPC，也没有修改任何共享文件；下列条目是 G1 落地后剩余的集成请求。

## IR-06 补记：G1 只扩展了已注册命令的载荷（不需注册）
| 命令（lib.rs 已注册） | G1 新增字段 | 向后兼容 |
| --- | --- | --- |
| `exposure_scope_summary` | `limitsBasis`、`limitsError`（G1-0）；`unitProjection`、`unitProjectionError`（G1-2）；`findingsExcluded`、`findingsClassificationPending`、`findingsUnclassified`、`classifyPolicyVersion`（G1-3）；`highRiskOpen` 不再计入自动排除的线索 | 只增字段；前端缺字段时显示“未报告”，不显示 0 |
| `get_exposure_settings` | `effective`：`{concurrency:1, concurrencyImplemented:false, …, knownLimits}`（G1-0） | 只增字段 |
| `page_exposure_findings` | 入参 `disposition`（serde default，空串 = 不筛选；未知值 `[exposure_disposition_invalid]`）；出参平铺页字段之外增加 `dispositions`（findingId → 处置）和 `classifyPolicyVersion`（G1-3） | 页字段仍平铺；旧调用方不传 `disposition` 行为不变 |
| `read_exposure_evidence` | `disposition`、`classifications`、`classificationError`、`classifyPolicyVersion`（G1-3） | 只增字段 |
| `save_exposure_remediation` | 对自动排除且未经人工确认的线索返回 `exposure_remediation_excluded：…`（G1-3） | 新错误码；页面已不对这类线索显示“登记整改” |
| `page_exposure_clusters` | 行内 `boundaryBasis`、`pslVersion`、`boundaryLabel`（G1-1） | 只增字段 |

## IR-07 线索处置的人工改判入口（G1-3 遗留，需要新 IPC）
- **现状：** G1-3 的自动分类（`oviraptor.exposure.classify.v1`）把线索分成风险候选、待分类、公开招采排除、公开材料排除四种处置。排除是粘性的：后续观察只会置冲突标记，不会自动撤销；人工确认的线索不会被自动排除。但目前没有“人工把排除改回风险/把待分类定为排除”的通道。
- **请求：** 注册新命令（名称由主控定，建议 `reclassify_exposure_finding`）：
  - 参数：`projectId`、`findingId`、`expectedClassificationId`（CAS，防止覆盖并发改判）、`disposition`（四选一）、`infoCategory`、`actor`、`reason`。
  - 行为：写一条 `exposure_classifications` 行（`policy_version = manual:<actor>`），更新 `exposure_finding_dispositions`，清除冲突标记；跨项目或 CAS 不符返回零写错误。
  - 函数体由 Grok 在 `commands/exposure/classify.rs` 实现；主控只负责 lib.rs 注册和共享 api.ts 导出。
- **注册前：** 复核面板对排除线索显示 `integration_pending（IR-07）` 说明，不显示会失败的按钮（`ExposureReviewPanel.vue` 的 `data-state="integration_pending"`）。
- **测试：** 注册后补 Rust 用例（CAS 冲突、跨项目零写、改判后可登记整改），以及前端用例。

## IR-08 并行采集：实现或改合同（G1-0 遗留）
- **现状：** `exposure_concurrency` 可保存（兼容旧配置），但执行器严格串行。G1-0 已把面板改成“并行度（未实现，实际串行）”并禁用输入，设置视图返回 `effective.concurrency = 1`。
- **请求（二选一，由主控定）：** ① 在执行器中实现多 worker 的额度争抢（规格 T16–T18 要求两个真实并发连接争最后额度）；② 从设置合同中移除该字段。在决定之前 Grok 不扩大该字段的语义。

## IR-09 CT / Wayback 端点注入，用于 localhost 正负向测试（G1-0 / G2 前置）
- **现状：** `certificate_transparency::execute_bounded` 已按 `ct_page_size` 截断，并以 BlockedQuota + Partial 报出“其余 K 个未记录”，适配器层有 localhost 测试。但 `execute_exposure_run` 里 crt.sh / Wayback 的基地址写死，生产执行器路径无法指向 fixture，因此“执行器把 ct_limit 传给适配器”这一步只能靠 `run_plan` 层的测试间接覆盖（变异 G10-M7 是等价变异，已在 G1 交付文档说明）。
- **请求（不阻塞）：** 允许在测试构建里注入来源基地址。可以由 Grok 在白名单内用 `#[cfg(test)]` 覆盖，但如果主控希望统一成来源配置端口（G2 的 GitHub 也需要），请在 G2 前给出约定。

## IR-10 package.json 测试脚本（IR-05 的具体化）
- 新文件：`tools/test_exposure_g1.cjs`（9 个用例，覆盖 G1-0 到 G1-3 的前端逻辑与组件行为）。
- 请求：把它加进 `test:asset-exposure`，或新增 `test:exposure-g1`：`node --test --test-concurrency=1 tools/test_exposure_g1.cjs`。

## IR-11 逐资产单位边（IR-02 的 G1 落地状态）
- G1-2 已按 IR-02 的过渡方案实现只读单位投影（`commands/exposure/units.rs`，`oviraptor.exposure.units.v1`），挂在 `exposure_scope_summary.unitProjection` 上：
  - 只用项目级绑定和已确认目录。
  - 集团总账、未绑定、绑定失效时，确认资产全部记为“未分配单位”。
  - 专业公司及其分公司不归入任何省公司。
- 投影带 `perAssetEdge: "integration_pending"`；页面写明“逐资产单位归属：integration_pending”。IR-02 提供权威边之后，Grok 再把集团项目里的资产按边分配。

---

> G2 追加（2026-10-09 UTC+8）。IR-01 至 IR-11 原文未改。G2 没有新增 IPC，没有修改任何共享文件；下列条目是 G2 落地后的状态更新和新的集成请求。

## IR-09 更新：已在白名单内完成（G2-0），无需主控处理
- **做法：** `commands/exposure.rs` 新增 `SourceEndpoints`。生产版本 `source_endpoints()` 只返回常量：`https://crt.sh`、`https://web.archive.org`、`https://api.github.com`，私网出口关闭，凭据端口为 `integration_pending`。
- **测试覆盖：** 只在 `#[cfg(test)]` 构建里有一个线程局部覆盖 `use_test_endpoints_once`，用一次即清空；发布构建里这段代码不存在。没有配置项、环境变量或 IPC 能改写端点。
- **效果：** CT、Wayback、GitHub 都能走“生产执行器 → localhost fixture”的同一路径（`tests_g2::g2_0_*`、`g2_1_*_production_worker_*`、`g2_2_*worker*`）。G1 里等价的变异 G10-M7 现在也可以直接覆盖。
- 如主控以后要统一成来源配置端口（规格 §11 的 ExternalSourceScope），见 IR-13。

## IR-04 补充：G2 已备好凭据端口的接口形状，只差主控装配
- **现状：** `adapters/github.rs` 定义了 `CredentialLookup { Unconfigured{code,reason} | Available(GithubToken) }`。
  - `GithubToken` 没有 Display/Serialize，Debug 输出 `GithubToken(<redacted>)`；只接受 `[A-Za-z0-9_.-]{8,255}`，防止头注入。
  - token 只会变成与 `api.github.com` origin 绑定的 `ProviderAuthorization`。
  - 生产 `CredentialLookup::production()` 固定返回 `integration_pending`。
- **请求：** 主控提供一个只读秘密引用端口，签名建议：
  ```
  fn exposure_secret(project_id: i64, credential_ref: &str, capability: &str) -> Result<Option<String>, String>
  ```
  - `capability` ∈ `default_branch_code_search` / `controlled_blob_read`。
  - 返回值只在内存中使用，不落库。
  - `credential_ref` 存在 exposure 自有配置里，只是引用名，不是明文。
- **装配点（Grok 侧，端口到位后一处改动）：** `SourceEndpoints::production()` 的 `github_credential` 改为调用该端口；设置视图的 `sourceCapabilities` 同步显示 `configured`。
- **未装配时（现状）：**
  - 代码搜索工作项为 `skipped_unconfigured`，来源 `unconfigured`，原因 `integration_pending:凭据引用端口尚未装配（IR-04 待批准）…；未发送任何请求`。fixture 断言零请求。
  - 设置页显示“默认分支代码搜索 · 需要凭据 · 未配置（integration_pending）”。
- **验收缺口：** 代码搜索和受控读取目前只有 localhost fixture + 测试专用 token 的证据（证据等级 `fixture`）。没有真实凭据，就没有真实 GitHub 代码搜索的验收，交付文档不报外部完成。

## IR-10 补充：新增前端测试文件
- 新文件 `tools/test_exposure_g2.cjs`（6 个用例）。
- 请求：一并加入 `test:asset-exposure`，或新增 `test:exposure-g2`：`node --test --test-concurrency=1 tools/test_exposure_g2.cjs`。
- G2 还给 `tools/test_asset_exposure_entry.cjs`、`tools/test_asset_exposure_evidence.cjs` 的导入白名单加了一条，把页面对 `../features/assets/exposure/contracts` 的导入映射到同一个真实编译的 contracts 模块（不是桩）。

## IR-12 GitHub 来源的启用与额度是否需要主控层面的确认（新，不阻塞）
- **现状：** `github_repositories`、`github_code` 已加入可选来源（`settings::SOURCES`），但不在默认来源（`DEFAULT_SOURCES` 仍是 G0 的三项）。启用需要在 exposure 自有设置面板勾选。
- **计划上界写死在适配器版本里**（`oviraptor.exposure.github.v1`）：
  - 仓库搜索：每页 30、最多 2 页。
  - 代码搜索：每页 20、1 页；每个查询最多读 3 个 blob，每个 ≤256 KiB。
  - 自动重试 0 次。
- **请求：** 如主控希望这些上界可在设置里调整，或要求启用第三方平台前有额外的项目级确认（“集团名称会作为查询词发往 GitHub”），请给出约定。Grok 不擅自扩大默认外联。

## IR-13 ExternalSourceScope（规格 §11 设计名）的落地方式（新，不阻塞）
- **现状：** 第三方外联由代码常量限定：只允许 `api.github.com` 一个 origin（`fetch_provider_api` 的 origin 白名单）。
  - 路径只由适配器构造：`/search/repositories`、`/search/code`、`/repos/{owner}/{repo}/git/blobs/{sha}`；owner/repo/sha 都经过校验。
  - API 返回的 `html_url`、Link 下一页不作为出站授权：Link 只在同 origin、同路径、同查询词、恰好下一页时才消费。
  - 跳转一律不跟随。
- **请求：** 如果主控要把 providerHost/path/method/有效期/额度做成可审计的配置对象（含版本与审批人），需要确定它放在 exposure 自有表还是共享表；若放共享表或需要新 IPC，按 IR-06 流程注册。现状不阻塞 G2/G3。

---

> G3 追加（2026-10-09 UTC+8）。以上原文未改。G3 没有修改任何共享文件（lib.rs、api.ts、package.json、db_schema.rs、PSL、组织目录都未动）；下列是 G3 落地后剩余的集成请求。用户尚未批准任何 IR。

## IR-14 文件/情报导入与人工定级的 IPC 注册（新，G3 的生产入口依赖它）
- **现状：** 函数与 `#[tauri::command]` 包装已在 `src-tauri/src/commands/exposure.rs` 写好，未在 `lib.rs` 注册；前端没有调用入口，设置面板把这两项能力显示为“已实现 · 入口待注册（integration_pending）”。

| 命令 | 参数 | 返回 | 错误码 | 调用方 | 测试 |
| --- | --- | --- | --- | --- | --- |
| `import_exposure_documents` | `projectId: i64`，`request`：`{operationId, channel: "document_import"\|"public_intel_import", operator, sourceDeclaration, declaredUrl?, batchLabel?, publicMaterial?, publicMaterialNote?, extractedFrom?, files:[{name, format: text\|json\|csv\|extracted_text, contentBase64}]}`（未知字段拒绝） | `{batchId, runId, runStatus, replayed, declaredUrl, declaredUrlVerified:false, files:[{name, format, sha256, bytes, url, result:{sourceStatus, reason, findings, observations}}]}` | `[exposure_import_invalid]`、`[exposure_import_project]`、`[exposure_import_operation_conflict]`、`[exposure_import_interrupted]`、`[exposure_import_busy]` | 待建的导入面板（本期未建，避免出现会失败的按钮） | `tests_g3::g3_0_*`、`g3_2_*`、`g3_3_*` |
| `grade_exposure_intel` | `projectId: i64`，`request`：`{operationId, findingId, expectedGrade, grade, actor, reason, sampleObservationId?, entityId?}` | `{eventId, findingId, grade, previousGrade, replayed}` | `[exposure_intel_not_found]`、`[exposure_intel_grade_conflict]`（CAS）、`[exposure_intel_grade_operation_conflict]`、`[exposure_intel_anchor_required]`、`[exposure_intel_anchor_invalid]`、`[exposure_intel_grade_invalid]` | 证据包里的情报等级行（目前只读显示） | `tests_g3::g3_2_a_manual_grade_*` |

- **请求：**
  1. 在 `lib.rs` 的 `generate_handler!` 中注册这两个命令。
  2. 在共享 `src/features/assets/exposure/api.ts` 里加 `importExposureDocuments` / `gradeExposureIntel` 两个 invoke 包装。
  3. 注册后由 Grok 在下一片补导入面板与定级表单。
- **约束：**
  - 导入在同一事务里建一轮 `exposure_runs`，有活动轮次时拒绝（`busy`），中断后不按网络任务恢复（`import_not_resumable`）。
  - 文件正文不入库，只存哈希、摘录（≤400 字）与命中词。

## IR-10 补充（G3）：新增前端测试文件
- 新文件 `tools/test_exposure_g3.cjs`（5 个用例）。
- 请求：加入 `test:asset-exposure`，或新增 `test:exposure-g3`：`node --test --test-concurrency=1 tools/test_exposure_g3.cjs`。

## IR-13 补充（G3）：文档 URL 清单是项目级精确地址准入
- `exposureDocumentUrls` 是 exposure 自有设置里的项目级清单（≤20 个），进入冻结计划。
- 准入规则：
  - 只读清单内的精确 URL，每个一次；不跟随跳转。
  - 只限 http/https、默认端口；地址不能带账号信息，不能指向内网地址（除非项目开启内网出口）。
  - 清单之外的地址（包括导入时声明的来源 URL）一律不访问。
- 这是 ExternalSourceScope 的最小落地，不含有效期、审批人或额度对象；完整对象的存放位置仍按 IR-13 原文待定，不阻塞。

---

> G4 追加（2026-10-09 UTC+8）。以上原文未改。G4 没有修改任何共享文件（lib.rs、commands.rs、db*.rs、models*.rs、Cargo/package 文件、共享 `src/features/assets/api.ts`、PSL、组织目录都未动）。用户尚未批准任何 IR。

## IR-15 事件台账 IPC 注册（新，G4-0/G4-3 的生产入口依赖它）
- **现状：** 函数与 `#[tauri::command]` 包装已写在 `src-tauri/src/commands/exposure.rs`（`#[allow(dead_code)]`），未在 `lib.rs` 注册。exposure 私有 `src/features/assets/exposure/api.ts` 的 `exposureIncidentApi` 已有对应 invoke 包装；未注册时调用得到 unknown command，`useExposureIncidents` 据此把“事件台账”页显示为 `integration_pending（IR-15）`，写明“不是没有事件”，不提供任何动作按钮。
- 事件候选会在每轮采集结束前由已注册的采集路径自动建议（`orchestrator::run_plan` → `incidents::propose`），所以注册前库里就会有候选事件；只是页面读不到、人也改不了状态。

| 命令 | 参数 | 返回 | 错误码 | 调用方 | 测试 |
| --- | --- | --- | --- | --- | --- |
| `page_exposure_incidents` | `projectId: i64`、`status?: ""\|"open"\|<状态>`、`reviewNeededOnly?: bool`、`pageSize: 1..200`、`cursor?` | `{items[{id, stableKey, title, status, legacyFindingStatus, revision, basis, evidenceGrade, reviewNeeded, handlingState, activeLocations, units{confirmed,candidate,conflict,unknown}, …}], total, pageSize, hasMore, nextCursor, policyVersion}`；按 id 倒序 keyset，游标 `v1:<筛选哈希>:<id>` | `[exposure_page_size_invalid]`、`[exposure_cursor_invalid]`、`[exposure_cursor_mismatch]`、`[exposure_incident_schema_mismatch]` | 事件台账列表 | `tests_g4::g4_3_incident_pages_*`、`test_exposure_g4.cjs` |
| `read_exposure_incident` | `projectId`、`incidentId` | 完整 DTO：schemaVersion、projectId、stableKey、revision、policyVersion、findingIds、observationRefs、runIds、directoryVersion/bindingVersion、entities、events（最近 50 条 + truncated）、locations（第一页）、statusMapping | `[exposure_incident_not_found]`（跨项目同此） | 事件详情 | `g4_0_*`、`g4_0_another_project_reads_and_writes_nothing` |
| `page_exposure_incident_locations` | `projectId`、`incidentId`、`pageSize`、`cursor?` | 位置分页（同上 keyset） | 同上 | 详情“更多位置” | `g4_3_*` |
| `act_on_exposure_incident` | `projectId`、`action`：`{operationId, action: review\|merge\|split\|link_entity\|record_action\|retest_outcome, incidentId, expectedRevision, actor, reason, status?, targetIncidentId?, targetExpectedRevision?, locationIds?, entityId?, linkLevel?, evidenceObservationIds?, handling?, channel?, externalReference?, retestRunId?}`（未知字段拒绝） | 动作结果 JSON（含 `eventId`；重放时 `replayed:true` 并原样返回首次结果） | `[exposure_incident_invalid]`、`[exposure_incident_conflict]`（CAS）、`[exposure_incident_operation_conflict]`、`[exposure_incident_state]`、`[exposure_incident_evidence_required]`、`[exposure_incident_evidence_invalid]`、`[exposure_incident_entity_invalid]`、`[exposure_incident_retest_invalid]`、`[exposure_incident_not_found]` | 详情里的复核/登记/关联表单 | `g4_0_cas_*`、`g4_0_two_writers_*`、`g4_0_merge_and_split_*`、`g4_0_unit_links_*`、`g4_0_handling_*` |
| `propose_exposure_incidents` | `projectId` | `{created, attachedLocations, reviewFlagged, groupsConsidered}` | 同 schema 错误 | G4 之前的历史轮次补建议（可选按钮，本期未放） | `g4_0_*`、`g4_4_scale_tier` |

- **请求：** 在 `lib.rs` 的 `generate_handler!` 注册以上 5 个命令。前端包装已在 exposure 私有 api.ts，无需改共享 `src/features/assets/api.ts`。
- **约束（已实现，注册后不变）：**
  - 状态只由人工改；自动建议只建 `candidate`、只挂位置；已有人工结论的事件只会被标 `review_needed`，不改结论。
  - “通知/下架/整改/关闭”只登记人工已经做过的动作：`contactedByApplication=false`，代码里没有任何对外发送路径；复测结果从不自动关闭事件（`autoClosed=false`）。
  - 每个动作带 projectId、expectedRevision、operationId、actor、reason；同 operationId 同载荷重放原结果，异载荷拒绝；CAS 与写入同一 IMMEDIATE 事务。

## IR-06 补记（G4）：已注册命令的载荷扩展（不需注册）

| 命令（lib.rs 已注册） | G4 变化 | 向后兼容 |
| --- | --- | --- |
| `exposure_scope_summary` | 新增 `liveRun`/`liveRunError`（G4-2）、`coverage`/`coverageError`、`unitStatistics`/`unitStatisticsError`（G4-1） | 只增字段；前端缺字段时显示“后端未报告（integration_pending）”，不显示 0 或绿色 |
| `export_exposure_findings` | 新增可选参数 `kind`（`findings` 默认 / `ledger`，其它值 `[exposure_export_kind_invalid]`）；文件名改为 `<项目名>_暴露面[_事件台账]_<YYYYMMDD-HHMMSS±zzzz>.csv`，同名加 `_N`，从不覆盖；线索 CSV 在原 14 列后追加 13 列（信息类别、处置、事件编号、单位关联级别、整改、版本、采集/索引时间等） | 旧调用方不传 `kind` 仍得到线索 CSV；返回类型仍是共享 `ExportResult{path, rows}`（未改 models） |
| `verify_exposure_export` | 识别 `findings_v1` / `findings_v2` / `ledger_v1` 三种表头，逐行核对列数，回给 `format` 与 `rowKinds` | 只增字段 |
| `start_exposure_scan` / `resume_exposure_scan`（执行路径） | 每轮 finalize 前自动建议事件；建议失败时这一轮以 `[exposure_incident_proposal_failed]` 记为基础设施失败，不假装完成 | 只增写 exposure 自有表 |

## IR-11 更新（G4）：单位级事件统计仍受逐资产单位边限制
- G4-1 的 `unitStatistics` 与事件的单位关联只用两种依据：单位项目绑定 + 已确认资产（自动 `confirmed_link`），以及人工 `link_entity`（`confirmed_link` 必须引用本事件的观察作为证据）。
- 集团总账项目里没有逐资产单位边（IR-02），这类项目的事件全部计入“单位未知”，省/单位行不会从名称或地区码推断。IR-02 提供权威边后，Grok 在 `incidents::auto_link` 一处改动即可按边关联。
- 覆盖矩阵的 `unitAxis` 在这类项目里为 `not_reported`，页面写“单位维度未报告”，不写 0。

## IR-10 补充（G4）：新增前端测试文件
- 新文件 `tools/test_exposure_g4.cjs`（11 个用例）。
- 请求：加入 `test:asset-exposure`，或新增 `test:exposure-g4`：`node --test --test-concurrency=1 tools/test_exposure_g4.cjs`。
- G4 还给 `tools/test_asset_exposure_entry.cjs`、`test_asset_exposure_evidence.cjs`、`test_asset_exposure_page.cjs` 的导入映射加了事件台账面板及其两个模块（真实编译，不是桩）。

## IR-16 “取消”的轮次是否允许恢复（新，不阻塞，产品决策）
- **现状（G4 之前就有的行为，G4 未改）：** `resume_exposure_scan` 接受 `interrupted`、`cancelled`、`queued`、`running` 四种状态；取消的轮次可以按冻结计划继续，已完成项不重跑（规模测试实测重跑量 = 计划 − 已完成）。
- G4-2 的实时块据此把 `cancelled` 也标为 `resumable`，页面提供“按冻结计划恢复”按钮。
- **请求：** 如果产品要求“取消即终态”，请主控确认；届时需同时改 `resume_exposure_scan` 的状态判断（exposure.rs，白名单内）与 `live.rs` 的 resumable，Grok 可在一处完成。不确认则保持现状。

## P0 落地记录（2026-10-09）

用户批准落地后，本轮只做 P0。下面三项已改入代码，未 commit。更早的 IR 条目原文未改。

### IR-15 已落地

- `src-tauri/src/lib.rs` 的 `generate_handler!` 已注册 `page_exposure_incidents`、`read_exposure_incident`、`page_exposure_incident_locations`、`act_on_exposure_incident`、`propose_exposure_incidents`。
- `src/features/assets/exposure/api.ts` 增加 `proposeExposureIncidents`。事件面板增加「为历史轮次补建议」。
- 命令在运行时缺失时，面板仍显示 integration_pending，不把缺接口显示成「没有事件」。
- 处置仍只登记人工动作，不自动联系任何人。

### IR-01 已落地

- 动手前全库检索：生产调用方只有 `lib.rs` 注册行、`commands/exposure.rs` 函数体、共享 `src/features/assets/api.ts` 导出。`src/` 没有其它调用。`tools/` 里是测试桩，未删。
- 已删除这三处。`review_exposure_finding_scoped` 保留。
- `models_appsec.rs` 的 `ExposureFindingReviewInput` 不在本轮批准范围内，未改。因此 `cargo check --lib` 与 clippy 各有一条 `dead_code`（该结构体不再被构造），不在 exposure 代码里。

### IR-14 已落地

- `lib.rs` 已注册 `import_exposure_documents`、`grade_exposure_intel`。
- 调用加在 `src/features/assets/exposure/api.ts`，没有加进共享 `api.ts`。
- 导入表单在 `ExposureSettingsPanel.vue`，定级表单在证据包（仅情报线索）。校验在 `contracts.ts` 的 `buildImportRequest` / `buildGradeRequest`。声明的 URL 只记录，不访问。PDF/Office/OCR 仍不支持。
- 设置页这两项能力为 `configured`；覆盖矩阵 `integration=available`（项目未启用且没有轮次时，状态是 `unconfigured`，不是 `not_implemented`）。

### 本轮验证

- `node --test` exposure 与 asset-exposure 前端测试 97 通过（含新文件 `tools/test_exposure_ir.cjs`）。`vue-tsc --noEmit` 通过。
- `cargo test --lib exposure`：179 passed，3 ignored。`cargo check --lib` 通过。`cargo clippy --lib --tests` 通过，exposure 代码 0 warning。
- 变异 8 个全部被测试杀死；每个还原后 touch 并核对 sha，无残留 `.mutbak`。
- `package.json` 未改。`tools/test_exposure_ir.cjs` 进入测试脚本仍属于 IR-10（P1）。

P1 未做：IR-07、IR-04、IR-02/IR-11、IR-08、IR-10。P2 未做：IR-16、IR-12、IR-13、IR-03。IR-09 此前已关闭。

## P1 落地记录（2026-10-09）

### IR-01 收尾：已完成

- 全库检索后，`ExposureFindingReviewInput` 只剩 `models_appsec.rs` 里的定义。已删除，经用户同意。`dead_code` 警告消失。
- 注：真实仓库的 `models_appsec.rs` 在 08:28 已被其他人改成同样内容（删掉了这个结构体）。本轮回写时内容一致，没有再写。

### IR-10：已落地

- `package.json` 新增 `test:exposure-g1`~`g4`、`test:exposure-ir`，以及汇总脚本 `test:exposure`（g1~g4、ir 加上 7 个 `test_asset_exposure_*.cjs`）。原有的 `test:asset-exposure` 不变。

### IR-07：已落地（人工改判）

- 新命令 `reclassify_exposure_finding(projectId, request)`，已在 `lib.rs` 注册；`exposure/api.ts` 和共享 `api.ts` 都有对应的调用封装。
- 请求必须带 `operationId`、`findingId`、`expectedClassificationId`（CAS：对照处置行当前的 `classification_id`）、`disposition`、`infoCategory`、`actor`，以及 4~500 字的 `reason`。
- 写入方式：追加一条 `source_channel=manual`、`policy_version=manual:<操作人>` 的分类记录；只有当 `classification_id` 仍等于 expected 时，才更新处置行。审计写入新表 `exposure_reclassifications`（同项目内 operationId 唯一，并保存 payload hash 和结果）。
- 拒绝情况：同一 operationId、同一内容时重放第一次的结果；同一 operationId、不同内容时返回 `exposure_reclassify_operation_conflict`；跨项目返回 `exposure_reclassify_not_found`；CAS 不符返回 `exposure_reclassify_conflict`。以上拒绝都不写库。
- 改判为风险后，可以登记整改；对未改判且已排除的线索，拦截保留。
- 页面：证据包里，已排除的线索显示「人工改判」表单。操作人和理由沿用复核栏输入。只写库，不联系任何人。

### IR-08：已落地，采用「从合同和面板里移除不生效的并行度」

- 选这个方案的原因：真实并行要处理多 worker 争用额度、取消、连接上限（规格 T16~T18）等问题，风险高、改动大。执行器本身是严格串行的，保存的数值从来没有生效过。
- 后端：默认并行度为 1。`exposureConcurrency` 仍是已知字段，旧的冻结配置可以照常解析；非整数仍然报错。任何整数都会被忽略，强制为 1，也不再写回 JSON。历史轮次的 options_hash 是对原始冻结文本算的，不受影响。用新配置恢复旧轮次会因为配置不一致而要求新开一轮，这是预期行为。
- 前端：删除了并行度输入框；页面说明改为「并行度已从采集合同移除，执行器串行」。

### IR-04：端口已落地；只用 fixture 验证，没有真实 token，不宣称外部完成

- 仓库里没有统一的秘密存储（没有 keyring 类 crate，也没有 secrets 表）。agent/Native 代码没有改动。
- `exposure/adapters/github.rs` 新增只读的 macOS 钥匙串读取：服务名为 `oviraptor.exposure.github`，账户名为配置里的凭据引用。调用方式是 `security find-generic-password -w`，参数直接传递，不经过 shell；stdout/stderr 不进入错误信息。
- 配置里只保存引用名（`exposureGithubCredentialRef`，必须匹配 `^[A-Za-z0-9._:-]{1,80}$`；为空时不写进 JSON）。token 不进数据库、日志或配置，也没有写 token 的 IPC。引用为空或不合规时，不读钥匙串。读到形态不合法的值时直接丢弃，原因说明里不包含原文。
- 状态码由 `integration_pending` 改为 `unconfigured`。release 构建只在设置了引用时才读钥匙串；测试构建始终使用注入的 fixture，不会读登录钥匙串。
- 用户需要自己把 token 存入钥匙串：`security add-generic-password -s oviraptor.exposure.github -a <引用名> -w`，然后在设置页填写引用名。真实 GitHub 代码搜索尚未端到端验证。

### IR-02 / IR-11：阻塞，未改代码，等待用户批准

- `asset_ownership_decisions` 没有实体列（只有 status/confidence/authorization_status/exposure_eligible/source/reason/evidence_json）。`project_org_bindings` 只到项目级。`ownership_review_groups.candidate_entity_id` 只是候选，不是结论。
- 逐资产单位边需要归属模块新增一个只读投影，按资产给出 `ownerEntityId`、关系、证据和版本。这需要修改归属门禁的表或代码，而归属目前是冻结的，所以本轮停止，没有动。
- 投影提供之后，exposure 只在 `units.rs`（place_unit/project_units）和 `incidents::auto_link` 里只读消费。在此之前 `per_asset_edge` 仍为 `integration_pending`，也不会根据名称或区划码推断。

### P1 验证

- `cargo test --lib exposure`：181 passed，3 ignored。`cargo check --lib` 通过。`cargo clippy --lib --tests` 0 warning。
- `npm run test:exposure`：99 通过。`vue-tsc --noEmit` 通过。
- 变异 17 个全部被杀死（CAS、跨项目、审计、重放冲突、空理由/空操作人、token 原文回显、不合规引用读钥匙串、并行度重新生效或写回、前端 0 号分类、空理由、并行度回草稿、IR-07 提示回退、脚本缺失、结构体回来、面板不调接口）。第一轮「空理由」存活，补了后端非法请求零写测试后被杀死。
- 共享文件：`lib.rs`、`api.ts`、`package.json` 回写前与 P1 T0 一致，各 1 处行级补丁；`models_appsec.rs` 已被他人改成同样内容，没有写。

P2 未做：IR-16、IR-12、IR-13、IR-03。

## IR-02 / IR-11 落地（2026-10-09，用户 09:40 批准）

用户原话：“批准：在归属模块加按资产的只读单位投影，暴露面只读使用”。

### 归属模块：按资产的只读单位投影

- 新文件 `src-tauri/src/commands/ownership_unit_projection.rs`，在 `commands.rs` 里加了 1 行 include（回写前与快照一致）。新增两个函数：`ownership_asset_unit_projection(conn, project_id)` 和 `ownership_asset_unit_edge(conn, project_id, asset_id)`。每条边包含 `assetId / ownerEntityId / relation / evidence / version`。投影整体带 16 位版本摘要，摘要覆盖目录版本、绑定版本、所有边和未分配原因。
- **没有新增列或表，也不写库。** 投影完全由现有的 decisions、operations、items、review_groups、asset_events 推导，旧数据不受影响。确认归属、授权、准入、分组和恢复的语义都没有改。
- 只有两类权威依据会计入：
  - 单位项目：资产状态为人工确认（`confirmed` 且来源 `manual`），单位取自项目绑定（relation `project_binding`）。
  - 集团总账：资产状态为人工确认，并且本项目内该资产最近一次已应用的组操作满足以下全部条件：
    - 是已完成的 `confirm_ownership` 组决定；
    - 所在组为 `consistent` 车道，候选来源是 `directory`；
    - 实体在已确认目录里，属于绑定集团，类型是单位；
    - 存在对应的决定事件；
    - 之后没有改判。
  - 满足后 relation 为 `group_decision`，证据里含 operationId、groupId、runId、support。
- 其余已确认资产一律记为未分配，并给出原因：`project_unbound`、`binding_broken`、`no_unit_decision`、`operation_in_progress`、`group_without_directory_entity`、`entity_retired`、`entity_outside_binding`、`entity_not_a_unit`、`decision_event_missing`、`changed_after_group_decision` 等。候选组和自动判断都不计入，也不按名称或区划码推断。
- 版本：边的版本格式为 `d{目录}.b{绑定}.op:{操作}`。组撤回、组决定后的人工改判、目录退役、改绑和目录升版本都会改变投影。
- **口径：** 授权的授予或撤销不改变单位边。这是 O3 的既有语义，归属和授权分开。这里的“撤销”指组撤回或人工改判，两者都会让边立即消失。

### 暴露面：只读使用

- `units.rs`：`per_asset_edge` 改为 `ownership_projection`。集团总账的 `confirmedUnit` 从“未报告”（null）变为真实数字，`unassigned` 等于确认资产减去有单位的资产。新增 `perUnit`（专业公司单列，不归省）、`unassignedReasons` 和 `ownershipVersion`。
- `incidents::auto_link`：集团总账项目改为读取逐资产投影，生成 `confirmed_link`（origin 为 `ownership_projection`，依据里写明投影版本）。对同一资产重新建议时，投影已经不支持的旧链接会被置为失效。单位项目原有的路径不变。
- `coverage.rs`：单位统计在读取时过滤掉当前投影已不支持的投影链接，所以撤销立即反映到统计。没有单位依据的事件进入“单位未知”。
- 前端：去掉集团总账的 `integration_pending`，显示“其中有单位 X、未分配单位 Y”和逐单位资产行。旧后端返回 null 时显示“后端未报告”。
- 遗留：已失效的投影链接在下次建议之前，可能仍以 active 的状态出现在事件详情的实体列表里；统计已经在读取时排除了它们。10 万资产规模下的耗时没有实测（逐资产走预编译查询）。

### 验证

见 O3 交付文档的同日追加段落（测试、变异、闸门结果）。
