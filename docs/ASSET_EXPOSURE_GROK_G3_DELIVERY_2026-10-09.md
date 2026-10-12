# Asset 暴露面 Grok G3 交付：文档与情报入口（G3-0 至 G3-4）— 2026-10-09

- 执行者：Grok
- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`，HEAD `14a7875`。开始和回写前都核对过；未 commit/push/stash/reset/checkout/merge。
- 隔离环境：
  - 副本 `~/.ovi_o3/speccopy`，`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`。
  - 编译和测试都在 screen 里跑，断点写入 `~/.ovi_o3/g3/CHECKPOINT.md`。
- 回写：由 `~/.ovi_o3/g3/syncback.py` 完成。
  - 只写白名单文件，真实文件的 SHA 必须等于 G3 的 T0 快照或上次回写值。
  - 全程没有 FOREIGN EDIT。期间 Codex 改过 package.json 和 agent/multi_agent 测试文件（白名单外，与 exposure 无关），Grok 没有碰。G3 依赖的共享文件（lib.rs、commands.rs、db.rs、db_schema.rs、Cargo.toml、api.ts）副本与真实仓库逐字节一致。
- 真实用户库 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 全程未打开。
- 网络：
  - Rust 测试全部用 tempdir 临时库和 127.0.0.1 fixture。
  - 唯一的外网访问是 §6 列出的 2 次只读 GET，对象是公开、无敏感内容的文档。
  - 情报类只用本机 fixture，没有访问暗网、网盘或文库。
- **IR 状态：** 没有任何 IR 获批。
  - 文件导入、情报导入和人工定级的生产入口需要 IR-14（IPC 注册）；前端把它们显示为“已实现 · 入口待注册（integration_pending）”，不提供会失败的按钮。
  - 本文不把这三项报为“生产可用”。

## 1. 用户行为与生产调用链

**G3-1 指定公开文档 URL（`document_link`，生产可达）**
- **用户操作：** 在 exposure 自有设置面板里填写文档地址，每行一个，最多 20 个；写成 `URL | 理由` 表示人工声明为公开材料（理由必填）。然后勾选“指定公开文档 URL”来源，再开始采集。
- **调用链：**
  - 已注册的 `save_exposure_settings`：字段 `exposureDocumentUrls`，后端 `settings::parse_document_urls` 校验、规范化（去 fragment）、去重、排序。
  - `start_exposure_scan`：`orchestrator` 为每个地址生成一个工作项；`input_hash` 含版本、URL、公开声明和理由。
  - `execute_exposure_run`：执行器 `KIND_DOCUMENT_LINK` 分支调用 `documents::execute_link`。
  - `run_plan` 依次完成：观察、线索、分类（G1-3）、簇、来源结果。
- **出站门：**
  - 地址必须在本轮冻结清单内。
  - 只限 http/https，地址不能带账号信息。
  - 只允许默认端口（80/443），否则返回 `document_port_denied`。
  - 内网地址拒绝（`document_private_denied`），除非项目已开启内网出口。
  - 请求复用 G2 的 `fetch_provider_api`：origin 白名单只含该地址自身的 origin，不跟随跳转，单响应有上界，并扣本轮请求和字节额度。
  - 以上拒绝都发生在发送之前，不扣额度。
- **每种回答单独呈现：**

| 回答 | 工作项 | 来源状态 | 原因码 | 观察 |
|---|---|---|---|---|
| 200 HTML/文本，UTF-8 | completed | completed / completed_no_matches | — | `document_read`：标题、命中词、≤320 字摘录、全文规范化哈希 |
| 200 截断 | completed | **partial** | `document_truncated` | 同上，但**不写全文哈希**（不能用于“同一文档”） |
| 200 PDF/Office/zip/图片（魔数优先于声明类型） | completed | partial | `document_unsupported_type` | `document_metadata`：状态、类型、字节数、哈希；正文未分析 |
| 200 非 UTF-8 声明编码 / 非法 UTF-8 | failed | parse_error | `document_charset_unsupported` / `document_not_utf8` | metadata |
| 3xx | blocked_authorization | partial | `document_redirect_not_followed`（记录 Location，从不访问） | metadata |
| 401/403 | failed | auth_failed | `document_access_denied_{s}` | metadata |
| 404/410 | failed | failed | `document_not_found_{s}` | metadata |
| 429 | blocked_quota | rate_limited | `document_rate_limited_429` | metadata |
| 其它状态 | failed | failed | `document_http_{s}` | metadata |
| 启用了但没填地址 | skipped_unconfigured | unconfigured | `document_link_unconfigured` | 无请求 |

- HTML 只分析可见文本：剔除 script、style 和注释，并解码实体。
- 正文不入库，只存哈希、摘录和命中词。
- 所有观察都标明 `collectionMode=http_read`、`networkObserved=true`。

**G3-0 文件导入（`document_import`，函数已实现，入口待 IR-14）**
- **入口：** `imports::import_batch(project, request)`，未注册的包装命令为 `import_exposure_documents`。
- **请求字段：** `operationId`、`channel`、`operator`、`sourceDeclaration`、可选 `declaredUrl` / `batchLabel` / `publicMaterial(+Note)` / `extractedFrom`、`files[{name, format, contentBase64}]`；未知字段拒绝。
- **格式与上界：** text、json、csv（RFC 4180）、extracted_text；每批 ≤20 个文件，单文件 ≤2 MiB，整批 ≤8 MiB。
- **所有拒绝都在第一次写入之前：**
  - 校验失败返回 `[exposure_import_invalid]`；项目不存在或未激活返回 `[exposure_import_project]`。
  - 同一 operationId：载荷相同则重放原报告（`replayed:true`）；载荷不同返回 `[exposure_import_operation_conflict]`；上次批次未完成返回 `[exposure_import_interrupted]`。
  - 有活动轮次时返回 `[exposure_import_busy]`，事务内的单活动轮唯一索引是第二道防线。
- **执行：** 一个事务写入 `exposure_runs`、冻结计划、工作项和批次行，然后走同一个 `run_plan`（执行器是内存里的已解码文件），所以分类、复核和簇完全复用。
- **逐文件结果：**
  - PDF/Office/图片/压缩包（按魔数识别）：`unsupported_format`。
  - 非法 UTF-8：`invalid_utf8`。
  - JSON/CSV 解析失败：`parse_error`。
  - 无命中：`completed_no_matches`。
  - 最差的文件决定来源状态，不会报“全部完成”。
- **证据标注：**
  - 观察的 method 为 `IMPORT`，status 为 0；`collectionMode` 为 `import` 或 `import_extracted_text`，`networkObserved=false`。
  - 同时记录 operator、sourceDeclaration、batchId、文件 SHA-256。
  - `declaredUrl` 只是声明（`declaredUrlVerified=false`），**从不访问**（fixture 计数 0）。
  - 线索的证据写明“导入正文……不是应用从外网观察到的”。
- **中断：** 正文不在库里，网络执行器遇到导入工作项会返回 `import_not_resumable`，不假装续导。
- **边界：** 不建资产、不建主体、不写归属决定（测试核对 assets、org_entities、asset_ownership_decisions 的行数不变）。

**G3-2 公开情报导入与分级（`public_intel_import`，函数已实现，入口待 IR-14）**
- **输入：** 同一个导入入口，channel 为 `public_intel_import`，只接受 JSON 数组或 CSV；`claim` 必填，可选 sample、source、sourceUrl、publishedAt、entities、grade；每批 ≤500 条。
- **入库：** 每条记录一条 `intel_record` 观察。只有出现集团标识的记录才生成线索 `intel_claim`（中等严重度，置信 40 或 20）；无关记录只留观察。
- **导入时的等级由系统自己判定：**
  - 样本里出现集团标识或凭据形态值：`sample_supported`。
  - 否则：`claim_only`。
  - 文件里自称 `human_verified` 一律降级，并写明原因（`gradeNote`）。
  - 报告来源永远是 `reportedSourceVerified=false`。
- **分类：** 渠道 `public_intel_import`，信息类别 `sold_intel`；**无论内容如何都留人工**，凭据形态值也不会自动成为风险。
- **人工定级：** `imports::grade_intel_finding`，未注册的包装命令为 `grade_exposure_intel`。
  - 理由必须 4–500 字。
  - CAS：`expectedGrade` 必须等于当前等级，否则返回 `[exposure_intel_grade_conflict]`。
  - 定为 `human_verified` 需要可核验的锚点：本线索里支持声称的样本观察（`sampleSupported`），或组织目录中存在的单位；定为 `sample_supported` 需要样本锚点。借用别的线索的样本、目录里没有的单位都会被拒。
  - operationId 幂等；跨项目操作返回 not_found；所有拒绝零写入。
  - 证据包新增 `intelGrade` / `intelGradeError`，页面显示等级行，读取失败时如实显示。

**G3-3 分类与 same_document**
- 三个来源键分列渠道：`document_link`、`document_import`、`public_intel_import`。
- 复用 G1-3：
  - 招采公告排除（链接读取和导入文本都生效）。
  - 招采页里混有凭据时留人工。
- 文档里只出现集团名称，一律 `classification_pending`：出现不等于非公开或属于本集团。
- 人工声明为公开材料，只排除“提及”线索（`excluded_public_material`，理由写在证据里）；同一文档里的凭据线索仍是 `risk_candidate`。
- `same_document`：新观察的全文规范化哈希等于本项目里另一个地址的文档观察时，记为 `sameDocumentCandidates`（relation=same_document，status=candidate）。
  - 只是候选，从不合并线索。
  - 截断正文没有全文哈希，永远不会进入候选。
  - 同名但内容不同的文件不算。

**G3-4 闭环**
- 生产执行器读取 fixture 文档，生成线索后人工复核为 dismissed。
- 第二轮内容相同：仍是同一条线索，人工结论保留。
- 第三轮内容变化：新版本标 `review_needed=1`，旧结论保留但不被冒用。
- 设置视图的能力矩阵：

| 能力 | 状态 |
|---|---|
| explicit_document_url | configured / unconfigured |
| file_import、public_intel_import | integration_pending（IR-14） |
| pdf_docx_ocr、netdisk_keyword_search、underground_feed | unsupported |

  GitHub 能力行照旧。

**前端（白名单内）**
- `contracts.ts`：
  - 三个来源的中文名。
  - 文档和导入原因码标签，带状态码后缀的码也能识别，例如 `document_http_500`。
  - 能力行加 integration_pending、未实现两种表述，并新增 `capabilityTone`。
  - `parseDocumentUrls`、`readDocumentUrls`、`formatDocumentUrls`：不合规的行会被点名，不会被悄悄丢掉。
  - 情报等级标签与 `intelGradeLine`；`readEvidence` 保留 intelGrade。
- `useExposureSettings.ts`：文档 URL 草稿；有不合规地址时拒绝保存。
- `ExposureSettingsPanel.vue`：
  - 文档 URL 编辑框。
  - 能力列表按“来源:能力”区分，四种色调。
  - 写明“导入入口未注册（IR-14），本页不提供导入按钮”。
- `ExposureSurface.vue`：证据包显示情报等级。

## 2. 文件、SHA、白名单检查、共享依赖

白名单：`commands/exposure.rs`、`commands/exposure/**`、`src/features/assets/exposure/*`、`src/components/ExposureSurface.vue`、`src/exposure-*.css`、`tools/test_asset_exposure*.cjs`、`tools/test_exposure_*.cjs`、`docs/ASSET_EXPOSURE_GROK_*`。回写的 18 个文件加本文全部在白名单内；回写后再跑 `--dry` 结果为 `changed 0`，即白名单内所有文件与副本逐字节一致。SHA 见附录 A。

- **新增：** `adapters/documents.rs`、`imports.rs`、`tests_g3.rs`、`tools/test_exposure_g3.cjs`、本文。
- **修改：**
  - `exposure.rs`：模块注册、`SourceEndpoints.document_any_port`（生产为 false）、执行器的文档分支和导入不可恢复分支、same_document 挂接、能力矩阵、两个未注册的命令包装。
  - `settings.rs`：来源列表和 `exposureDocumentUrls`；空列表不写进冻结配置，旧计划哈希不变。
  - `orchestrator.rs`：文档 URL 计划。
  - `classify.rs`：三个渠道和三条规则。
  - `observations.rs`：`method` 列取值来自观察，默认 GET。
  - `passive_web.rs`：结构体补字段。
  - `repository.rs`：证据包情报等级。
  - `schema.sql`：两张 exposure 自有表 `exposure_import_batches`（`UNIQUE(project_id, operation_id)`）和 `exposure_intel_grades`，外加一个索引；都是 `CREATE IF NOT EXISTS`，只增不改。
  - `tests_g2.rs`：fixture 端点补字段。
- **前端修改：** `contracts.ts`、`useExposureSettings.ts`、`ExposureSettingsPanel.vue`、`ExposureSurface.vue`。
- **IR 文档：** 追加 G3 段。原 139 行前缀逐字节未改（与 `ir_before_g3.md` 核对）。
- **共享依赖：** 没有改任何共享文件。读取 `crate::commands::load_org_entities`（组织目录，只读）用于实体锚点；`base64`、`uuid`、`reqwest` 已在 Cargo.toml。

## 3. 来源能力矩阵

| 来源 / 能力 | 已实现 | 生产可达 | G3 验证 |
|---|---|---|---|
| document_link · `explicit_document_url` | 是 | **是**（设置里填地址并勾选来源；默认不启用） | fixture 正负向 + **2 次真实只读请求**（§6） |
| document_import · `file_import`（text/json/csv/extracted_text） | 是（函数 + 未注册命令） | **否：integration_pending（IR-14）** | 临时库 + 生产 `run_plan`；无真实入口验收 |
| public_intel_import · `public_intel_import` + 人工定级 | 是（同上） | **否：integration_pending（IR-14）** | 本机 fixture 报告；无真实情报源 |
| PDF / Office / 图片 OCR | 否 | — | 读取时只记元数据，导入时拒绝，都有测试 |
| 文库/网盘关键词搜索、登录、提取码 | 否（unsupported） | — | 能力表点名 |
| 暗网/地下情报源接入 | 否（unsupported） | — | 能力表点名；未访问 |
| GitHub 代码搜索 / 受控读取 | 是（G2） | 否：IR-04 | 不变 |

## 4. 测试命令、结果、计数、重复

| 项 | 命令 | 结果 |
|---|---|---|
| 基线（G2 结束，副本） | `cargo test --lib exposure` | 143 passed，1 ignored |
| G3 新测试 | `cargo test --lib tests_g3` | **19 passed**，1 ignored（真实 URL，单独跑） |
| 全量 | `cargo test --lib exposure` | **162 passed**，2 ignored |
| 重复 | 同上 ×3（repeat1–3）+ 补测试后第 4 次（repeat4） | 4/4 次 162 passed（35–37 秒） |
| clippy | `cargo clippy --lib --tests`（clippy、clippy2） | exit 0，**暴露面 0 warning**（另有 46 条在 agent/multi_agent 的既有测试文件，与本期无关） |
| 前端（副本 + 真实仓库） | `node --test` 全部 `tools/test_asset_exposure*.cjs tools/test_exposure_*.cjs` | **82/82**（原 77 + G3 5） |
| 类型 | `npx vue-tsc --noEmit -p tsconfig.json`（副本 + 真实仓库） | exit 0 |
| 真实 URL | `cargo test --lib g3_live -- --ignored --nocapture` | 2 次请求，均 HTTP 200（§6） |

**G3 Rust 用例（19 个）**

| 片 | 用例 |
|---|---|
| G3-0（6） | 文本/JSON/CSV 走生产路径、只成线索不成资产、正文不入库（窗口外和上限外两个标记）；PDF/Office/PNG/非法 UTF-8/坏 JSON/坏 CSV/无命中各自成缺口；16 种非法批次零写入；重放、冲突、busy；声明 URL 不访问、已提取文本有标注；中断不可恢复 |
| G3-1（6） | 三份文档经生产执行器读取（剔除 script 和注释、Accept 头、一次一个、请求计量相符）；9 种 HTTP 回答（跳转目标从不访问、魔数优先）；发送前拒绝（内网、端口、额度、取消、未配置、不在清单、带账号）；截断为 partial 且不写全文哈希；设置校验和旧哈希不变；启用但没填地址为 unconfigured 且零请求 |
| G3-2（2） | 情报 JSON+CSV 分级（claim_only / sample_supported / 自称 human_verified 降级 / 无关只留观察 / sold_intel / 留人工）；人工定级的 7 种拒绝零写入、实体锚点、样本锚点、重放、操作冲突、CAS、跨项目、证据包等级 |
| G3-3（3） | 招采排除、招采混凭据留人工、公开声明排除但凭据仍是风险、普通提及留人工、导入的招采文本排除；same_document 只认全文相同，同名不同内容不算，不合并；纯函数渠道映射，情报不自动成风险 |
| G3-4（2） | 复核闭环（dismissed 保留，内容变化 review_needed=1）；设置视图能力矩阵 |

- 失败与修复记录：
  - 编译：`passive_web.rs` 结构体缺 `method` 字段（E0063），已补。
  - 导入的非法 UTF-8 被误判为二进制 unsupported：拆出 `magic_type`，只有魔数才判 unsupported。
  - 首轮测试 3 个失败，都是测试自身问题：列名应为 `evidence_excerpt`；PDF 和伪装 zip 共用同一原因码，计数应为 8。
  - clippy `len_zero` 已修。
  - 变异 R11 / R11b 暴露了“摘录两道上界互为兜底”：补了位于 320 字窗口外、400 字上限内的标记。

**变异（共 31 个：30 个杀死，1 个等价）**

方法：逐一注入；Rust 变异跑 `tests_g3`，前端变异跑 `test_exposure_g3.cjs`。恢复时用 copyfile 加新 mtime（touch），再核对 SHA；每条都是 `restored=true`，没有残留 `.mutbak`。

| 片 | 编号 | 结果 | 内容 |
|---|---|---|---|
| G3-1 | R01–R08 | 8 杀死 | 端口不拒、内网闸门失效、跳转原因码改为成功、script 不剔除、截断也写全文哈希、截断不标 partial、魔数不优先、非 UTF-8 编码照常解析 |
| G3-3 | R09、R10、R19–R21 | 5 杀死 | 公开声明不改标题、same_document 不比哈希、情报可自动判风险、文档提及不留人工、公开声明不排除 |
| G3-0 | R11b、R12、R13、R18、R23 | 5 杀死 | 摘录窗口放大（补测试后杀死）、导入不认魔数、同 operationId 不同载荷被当重放、文件名可带路径、中断导入被“恢复” |
| G3-0 | R11 | **等价存活** | 400 字上限放大到 4000：实际上界是 320 字窗口，400 永远不会触发。保留为纵深防御，由 R11b 证明窗口这道生效 |
| G3-2 | R14–R17 | 4 杀死 | 无样本也判 sample_supported、human_verified 不需锚点、CAS 失效、不支撑声称的样本也算锚点 |
| 设置 | R22 | 1 杀死 | 公开声明可不填理由 |
| 前端 | N01–N07 | 7 杀死 | integration_pending 显示为可用、状态码后缀放宽、公开声明不需理由、接受带账号的地址、不合规地址照常保存、证据包丢掉等级、未实现显示为缺口 |

日志：Mac 的 `~/.ovi_o3/logs/g3/mut_g3.out`、`mut_g3c.out`，以及 `~/.ovi_o3/g3/mut_g3.json`、`mut_g3b.json`。

## 5. 临时库规模、性能、真实库声明、磁盘

- 规模：功能性规模。每个临时库几十行；生产执行器用例 5 个，每轮向 fixture 发 1 到 6 个请求（站点 1 个加文档 0–5 个）；情报批次 5 条记录。规模三档属于 G4。
- 性能：`tests_g3` 19 个用例约 1.0 秒；exposure 全量 35–37 秒；单个 Rust 变异 12–26 秒。
- 真实库：未打开、未读写。两张新表是 exposure 自有的 `CREATE IF NOT EXISTS`，只会在应用下次打开 exposure 时建出空表；不改任何既有表。
- 磁盘：

| 位置 | 清理前 | 清理后 |
|---|---|---|
| 独立编译目录 `~/.ovi_o3/spec_target` | 7.3G | 删除，0 |
| 副本 `~/.ovi_o3/speccopy` | 37M（node_modules 是指向真实仓库的软链） | 删除，0 |
| 真实仓库 `src-tauri/target` | 20G | **未动**（按规则不清理） |

## 6. 四类证据

1. **真实外部来源（只读，2 次请求）**：在 Mac 上跑 `g3_live_public_documents_are_read_once_each`，走生产闸门（不开内网、只允许默认端口、不跟随跳转），每个地址的计量器上限为 1。结果只在内存，不写任何库。

| 时间（UTC+8） | 地址 | HTTP | 类型 / 字节 | 结果 |
|---|---|---|---|---|
| 2026-10-09 01:45:38 | `https://www.rfc-editor.org/rfc/rfc2606.txt`（保留示例域名的公开标准） | 200 | text/plain; charset=utf-8 / 8008 | completed；命中 `example.com`，1 条线索（测试词为 example.com，不是用户集团） |
| 2026-10-09 01:45:39 | `https://example.com/`（IANA 示例页） | 200 | text/html; charset=utf-8 / 577 | completed_no_matches（可见文本没有出现测试词），原因写“不代表没有泄漏” |

   覆盖范围：生产出站门、真实 TLS、文本和 HTML 两种解析、命中与无命中两种状态。未覆盖：真实的跳转、401/404/429、PDF、截断（这些只有 fixture 证据）。
2. **fixture（生产执行器同一路径）**：G3-1 与 G3-3、G3-4 的生产执行器用例共 5 个，加适配器级用例；HTTP 回答全部是 fixture 证据。
3. **导入数据（本机构造，非真实）**：G3-0 和 G3-2 用的文本、JSON、CSV 和情报报告都是测试里构造的样本，经生产 `run_plan` 入库、分类。没有导入任何真实用户材料。
4. **未验证 / 等 IR：**
   - 导入、定级的生产入口：需要 IR-14。
   - 真实情报源、暗网、网盘：不在本期，能力表标为 unsupported。
   - PDF/Office/OCR 解析：未实现。
   - 完整 ExternalSourceScope 对象（IR-13）。
   - GitHub 代码搜索真实验收：仍等 IR-04。

## 7. 下一阶段 G4（事件、统计、实时页面与规模）

1. **G4-0 事件与位置（incident / location）**
   - 把同一泄漏在多个地址或来源的出现（含 same_document 候选和情报声称）归成“事件”候选。
   - 人工确认后才成立，线索本身不合并。
   - 事件有状态机：发现 → 确认 → 通知 → 下架/整改 → 复测 → 关闭，带 CAS 与审计。
   - 外部平台的“下架”只登记人工动作和复测结果，不自动对外联系。
2. **G4-1 跨单位统计**
   - 按 G1-2 的单位投影汇总：处置分布、来源覆盖、未配置/缺口比例、待人工数、事件数。
   - “未覆盖”和“零命中”分开计。
   - 投影缺失的单位显示“未报告”，不显示 0。
3. **G4-2 实时页面**
   - 采集进行中实时刷新工作项和来源状态（轮询或事件，复用已注册的读取命令；需新 IPC 时按 IR 流程）。
   - 取消和恢复的状态即时可见。
4. **G4-3 分页与导出**
   - 线索、观察、事件、情报都用游标分页，列表上限明确，截断时显示“未列出”。
   - 导出带摘要哈希和行数，复用 E4 的导出校验；导出内容不含正文和凭据原值。
5. **G4-4 规模三档**
   - 临时库 4,000 / 20,000 / 100,000 条线索（含观察、版本、分类），测列表、统计、分页、导出、重分类的耗时和内存。
   - 设上界，超界时降级并给出提示。
   - 只用临时库，不碰真实库。
6. **G4-5 闭环与交付**：localhost 正负向、变异、重复 ×3、clippy，按七节结构交付。
- 依赖：
  - IR-14（导入入口）注册后，G4 顺带补导入面板和定级表单。
  - IR-07（人工改判）影响事件确认。
  - 统计口径如需进入共享报表，按 IR 流程申请。

## 附录 A：回写文件 SHA-256（真实仓库，回写后与副本逐字节一致）

| 文件 | 类型 | 行数 | SHA-256 前 16 位 |
|---|---|---|---|
| `docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md` | 修改（追加） | 171 | `f0cbb261f3a425a6` |
| `src-tauri/src/commands/exposure.rs` | 修改 | 1307 | `9c6101307b865d8d` |
| `src-tauri/src/commands/exposure/adapters/documents.rs` | 新增 | 811 | `0b6a2b54dff20e45` |
| `src-tauri/src/commands/exposure/adapters/passive_web.rs` | 修改 | 314 | `cd14bdf7735ad7d4` |
| `src-tauri/src/commands/exposure/classify.rs` | 修改 | 658 | `5836a114e6ffc98b` |
| `src-tauri/src/commands/exposure/imports.rs` | 新增 | 981 | `1d9e6d8c7473afb6` |
| `src-tauri/src/commands/exposure/observations.rs` | 修改 | 426 | `f3bbc63a8efe1975` |
| `src-tauri/src/commands/exposure/orchestrator.rs` | 修改 | 898 | `1f06ca5de4259f71` |
| `src-tauri/src/commands/exposure/repository.rs` | 修改 | 537 | `ac9b80b2b3530f4b` |
| `src-tauri/src/commands/exposure/schema.sql` | 修改（追加） | 337 | `f10c72ceec34d067` |
| `src-tauri/src/commands/exposure/settings.rs` | 修改 | 688 | `be4543e1c80f1308` |
| `src-tauri/src/commands/exposure/tests_g2.rs` | 修改（一行） | 876 | `f664b909570af308` |
| `src-tauri/src/commands/exposure/tests_g3.rs` | 新增 | 1045 | `771027e24163376b` |
| `src/components/ExposureSurface.vue` | 修改 | 320 | `41a862da665ae0e4` |
| `src/features/assets/exposure/ExposureSettingsPanel.vue` | 修改 | 133 | `1752352da64dfdd2` |
| `src/features/assets/exposure/contracts.ts` | 修改 | 934 | `5979a37b0167ca6e` |
| `src/features/assets/exposure/useExposureSettings.ts` | 修改 | 290 | `a5b968d92baffe1c` |
| `tools/test_exposure_g3.cjs` | 新增 | 197 | `dfcc5270c0f0911b` |

本文本身也经 syncback 回写；它的 SHA 记录在 `~/.ovi_o3/g3/synced.sha`。
