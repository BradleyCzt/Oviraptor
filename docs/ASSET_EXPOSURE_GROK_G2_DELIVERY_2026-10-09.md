# Asset 暴露面 Grok G2 交付：GitHub（G2-0 至 G2-4）— 2026-10-09

- 执行者：Grok
- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`，HEAD `14a7875`。开始、两次中断恢复和结束时都核对过；未 commit/push/stash/reset/checkout/merge。
- 隔离环境：
  - 在副本 `~/.ovi_o3/speccopy` 中开发、测试、变异，`CARGO_TARGET_DIR=~/.ovi_o3/spec_target`。
  - 长任务放在 screen 里跑，进度写入断点文件 `~/.ovi_o3/g2/CHECKPOINT.md`。
- 回写：由 `~/.ovi_o3/g2/syncback.py` 完成。
  - 只写白名单文件，而且真实文件的 SHA 必须等于 G2 的 T0 快照或上次回写值。
  - G2 全程没有 FOREIGN EDIT。期间 Codex 在真实仓库改了 lib.rs、commands.rs、agent/sentinel 等白名单外文件；Grok 没有碰这些文件，它们与 exposure 相关的行也没有变化。
- 真实用户库 `/Users/swyiic/oviraptor/oviraptor.sqlite3` 全程未打开。
- 网络：Rust 测试全部用 tempdir 临时库和 127.0.0.1 fixture。唯一的外网访问是 §6 列出的 3 次匿名 GitHub 仓库搜索。
- **IR 状态：** 没有任何 IR 获批。需要 IR-04（凭据引用）的部分在生产里显示 `unconfigured / integration_pending`，本文不报“外部完成”。

## 1. 用户行为与生产调用链

G2 没有新增 IPC，所有行为都挂在已注册的 `start_exposure_scan` / `resume_exposure_scan` / `get_exposure_settings` / `save_exposure_settings` 上。

**G2-0 来源端点与 fixture 注入（IR-09 已在白名单内完成）**
- `exposure.rs::SourceEndpoints`：生产版 `source_endpoints()` 只有常量。
  - 端点：`https://crt.sh`、`https://web.archive.org`、`https://api.github.com`。
  - 私网出口关闭；凭据为 `CredentialLookup::production()`，即 `integration_pending`。
- 只有 `#[cfg(test)]` 构建里存在线程局部覆盖 `use_test_endpoints_once`，用一次即清空。发布构建里没有可被外部改写的入口：没有配置项、环境变量或 IPC。
- 顺带修复一个 G0 起就有的计量缺陷：
  - 各适配器把整轮计量器的累计值当作本项用量，`record_usage` 又按项累加，导致从第二个工作项起请求数和字节数被重复记账。
  - 现在执行器统一换成本项增量。测试断言 `requests_used` 等于两个 fixture 实际收到的请求数；变异 G20-M1 改回累计值后被杀死。

**G2-1 匿名仓库元数据搜索（`github_repositories`，capability `repository_metadata`）**
- 查询计划只来自已确认的名称：项目的 approved 根域名，加上 profile 的实体全称 `legal_name`。查询形如 `"<名称>" in:name,description,readme`，引号会被剥掉，防止注入限定符。不枚举员工名或短词。
- 分页：
  - 消费真实的 `Link rel="next"`。只有同 origin、同路径、同查询词、恰好下一页时才接受；否则记 `github_link_rejected`，不访问。
  - 计划上界：每页 30、最多 2 页，接口结果窗口 1000。
  - 读不到的部分记为“未读尾部 K 条”；`incomplete_results` 和超出窗口都报 `partial`，不宣称全平台检索完毕。
- 额度：
  - 每次 HTTP 尝试都扣 exposure 请求额度；额度用尽时保留已读页，状态为 `blocked_quota` / `partial`。
  - 读取 `x-ratelimit-*`、`Retry-After`。`remaining=0` 时停止翻页，同一轮里同一能力的后续工作项不再发请求，报 `rate_limited`。
  - 自动重试 0 次。
- 错误分别显示：

| GitHub 回答 | 来源状态 | 工作项 | 原因码 |
|---|---|---|---|
| 401 | `auth_failed` | failed | `github_auth_failed_401` |
| 403 + 额度信号（remaining=0 / Retry-After / rate limit 文案） | `rate_limited` | blocked_quota | `github_rate_limited_403` |
| 403 其它 | `auth_failed` | failed | `github_forbidden_403` |
| 422 | `failed` | failed | `github_query_rejected_422` |
| 429 | `rate_limited` | blocked_quota | `github_rate_limited_429` |
| 503 | `failed` | failed | `github_unavailable_503` |
| 3xx | `failed` | failed | `github_redirect_not_followed` |
| 200 非 JSON | `parse_error` | failed | `github_parse_error` |
| 200 且 total_count=0 | **`completed_no_matches`** | completed | 不是缺口，但写明“不代表没有泄漏” |

- 每页留一条 `github_search_page` 观察（含状态码、额度头、页号、下一页有无）；每个仓库留一条 `github_repository` 观察和一条线索。
  - 规范地址只由校验过的 `full_name` 构造；API 返回的 `html_url` 不被信任，不一致时标 `providerUrlMismatch`，也从不访问。
  - 非公开仓库跳过。名称不合规的行跳过并在原因里点名（真实验证由此发现了旧式登录名问题，见 §6）。
- 外部平台不冒充集团资产：GitHub 观察的 `asset_id` 为空，不进域名簇，不建 assets，不进待复核发现。

**G2-2 代码搜索 + 受控读取（`github_code`）与 token 防泄露**
- 生产没有凭据（IR-04 未批）：
  - 工作项 `skipped_unconfigured`，来源 `unconfigured`，原因 `integration_pending:凭据引用端口尚未装配（IR-04 待批准）…；未发送任何请求`。
  - fixture 断言零请求；这一轮不报 completed。
- 适配器已实现，只在 fixture 加测试专用 token 下验证：
  - 代码搜索：每页 20、1 页，`text-match` 摘要。
  - 每个查询最多按 blob sha 读 3 个文件，每个 ≤256 KiB。返回的 sha 必须与请求一致，否则丢弃正文（`github_blob_mismatch`）。
  - 摘要命中（`github_code_hit`）和正文读取（`github_blob_read`）分开记录；非公开仓库不收。
- token 防线：
  - `GithubToken` 没有 Display/Serialize，Debug 输出 `<redacted>`；只接受 `[A-Za-z0-9_.-]{8,255}`，防止头注入。
  - token 只转成与 API origin 绑定的 `ProviderAuthorization`；`HeaderValue::set_sensitive(true)`。
  - 发往其它 origin 时在发送前拒绝（`provider_credential_origin_mismatch`，不扣额度、不发请求）。
  - 客户端不跟随任何跳转；token 永不进 URL。
  - 错误文本和回显经过 `scrub` 脱敏。测试扫描整个临时库的每一个文本格子，确认没有 token，也没有凭据原值。

**G2-3 分类与排除（复用 G1-3）**
- `github_*` 映射为渠道 `code_platform`。
- 仓库元数据命中和没有凭据形态的代码命中一律为 `classification_pending`：只出现了集团名称，第一方归属未证实。只有凭据形态值才成为 `risk_candidate`；占位值归公开材料。
- G1-3 的招采排除、混合页留人工、粘性排除、人工结论优先原样生效（`g2_3_*` 有招采仓库和混合页用例）。
- “正式开源账号”名单可人工维护：设置字段 `exposureGithubOfficialOwners`，在 exposure 自有设置面板里编辑。这些账号下的仓库归 `excluded_public_material`；名字像集团的其它账号不会因此被排除或认定为集团所有。名单为空时不写进冻结配置，旧计划的选项哈希不变。
- GitHub 两个来源加入可选来源，但**默认不启用**（`DEFAULT_SOURCES` 仍是 G0 三项），不会给现有项目悄悄增加对外请求。

**G2-4 闭环**
- 生产执行器 → fixture GitHub → 观察、线索、分类 → 人工复核（dismissed）。
- 第二轮内容相同：仍是同一条线索，人工结论不被覆盖。
- 第三轮仓库描述变了：生成新版本，`review_needed=1`，旧结论保留但不被冒用。
- GitHub 线索的“整改/复测”不在 G2 范围：外部平台的下架属于 G3/G4 的流程设计，现有整改流程只对授权站点复测。

**前端**
- 数据源列表用 `sourceStatusView`：来源中文名、状态中文名，加上 GitHub 原因标签（401/403 无权/403 额度/429/422/503/跳转未跟随各不相同）。色调分四种：完成、无命中（蓝）、integration_pending（黄）、缺口（红）。
- 设置面板：
  - 来源中文名。
  - GitHub 能力矩阵：仓库元数据“无需凭据·可用”；默认分支代码搜索和命中文件受控读取“需要凭据·未配置（integration_pending）”。后端不报能力时显示 integration_pending，不假定可用。
  - 正式开源账号编辑框：不合规的条目会点名并拒绝保存，不会被悄悄丢掉。

## 2. 文件、SHA、白名单检查、共享依赖

见附录 A。全部在白名单内：`commands/exposure.rs`、`commands/exposure/**`、`src/features/assets/exposure/*`、`src/components/ExposureSurface.vue`、`src/exposure-surface.css`、`tools/test_asset_exposure*.cjs`、`tools/test_exposure_*.cjs`、`docs/ASSET_EXPOSURE_GROK_*`。
- 新文件：`adapters/github.rs`、`tests_g2.rs`、`tools/test_exposure_g2.cjs`、本文。
- 改动：`exposure.rs`（模块注册、SourceEndpoints、执行器分支、增量记账、设置视图能力矩阵）、`adapters/http.rs`（`fetch_provider_api`、`ProviderAuthorization`、白名单响应头）、`classify.rs`、`orchestrator.rs`（GitHub 计划）、`settings.rs`（来源、默认来源、正式开源账号）、`state.rs`（`rate_limited`/`auth_failed`/`completed_no_matches`）、`fixture.rs`（记录请求头、`start_on`、422/503 文案）。
- 前端改动：`contracts.ts`、`useExposureSettings.ts`、`ExposureSettingsPanel.vue`、`ExposureSurface.vue`、`exposure-surface.css`。
- 测试改动：
  - `tests_g1.rs` 一处：G1-3 拿 `github_code` 当“未知来源”样例，G2 后它是已知来源，改用 `pastebin_feed`，断言不变。
  - `test_asset_exposure_entry/evidence.cjs`：导入白名单加入页面对真实 contracts 模块的导入。
- 共享依赖：没有改任何共享文件；没有新表（`exposure_source_results.status` 是自由文本，新状态无需迁移）。依赖 `base64`、`chrono`、`reqwest` 已在 Cargo.toml。

## 3. 来源能力矩阵

| 来源 / 能力 | 已实现 | 生产可达 | 凭据 | G2 验证 |
|---|---|---|---|---|
| passive_web | 是 | 是 | 无 | G1 起生产执行器 + fixture；G2 回归全绿 |
| certificate_transparency | 是 | 是 | 无 | **G2-0 起生产执行器 → fixture**（IR-09 已解决）；G2 未对 crt.sh 实测 |
| web_archive | 是 | 是 | 无 | 同上 |
| github_repositories · `repository_metadata` | 是 | 是（需在设置里勾选，默认不启用） | 无（匿名） | fixture 正负向 + **3 次真实匿名请求**（§6） |
| github_code · `default_branch_code_search` | 是（适配器） | **否：unconfigured / integration_pending（IR-04）** | 必需 | 只有 fixture + 测试 token；**未做真实验收** |
| github_code · `controlled_blob_read` | 是（适配器） | **否：同上** | 必需 | 只有 fixture + 测试 token；**未做真实验收** |
| document_import / document_link / public_intel_import | 否 | 否 | — | G3 |

## 4. 测试命令、结果、计数、重复

| 项 | 命令 | 结果 |
|---|---|---|
| 基线（G1 结束，副本） | `cargo test --lib exposure` | 120 passed |
| G2 新测试 | `cargo test --lib tests_g2` | 23 passed，1 ignored（真实 GitHub，单独跑） |
| G2 全量 | `cargo test --lib exposure` | 143 passed，1 ignored |
| 重复 | 同上 ×3（`repeat1–3.log`） | 3/3 次 143 passed（43–53 秒） |
| clippy 修复后复跑 | `cargo clippy --lib --tests` + exposure 全量（`clippy2.log`、`repeat4.log`） | exit 0、暴露面 0 warning；143 passed、1 ignored（附录 B） |
| 前端（副本） | `node --test tools/test_asset_exposure*.cjs tools/test_exposure_g1.cjs tools/test_exposure_g2.cjs` | 77/77（原 71 + G2 6） |
| 类型 | `npx vue-tsc --noEmit -p tsconfig.json` | exit 0 |
| 真实 GitHub | `cargo test --lib g2_live -- --ignored --nocapture` | 3 次，均 HTTP 200（§6） |

- 失败与修复记录：
  - G1-3 用例把 `github_code` 当未知来源，改样例。
  - 页面新增对 contracts 的值导入，被两个 SFC 测试的导入白名单拦下，补白名单（映射到真实模块）。
  - `json!` 里的块表达式编译失败，改写。
  - 变异 G21-M1 存活：第 2 页 fixture 没带 Link，页数上限从未成为真正的停止条件。补“第 2 页带第 3 页 Link”后杀死。
  - 真实验证发现旧式登录名被误拒：放宽规则，加回归测试。
  - clippy 报了 6 个暴露面 warning，已修。

**变异（共 42 个，全部杀死）**

方法：在副本中逐一注入，跑 `tests_g2` 或 `test_exposure_g2.cjs`。恢复时用 copyfile 加新 mtime（相当于 touch），再核对 SHA；每条都是 `restored=true`，没有残留的 `.mutbak`。中断恢复后又逐条核对锚点，没有发现残留。

| 片 | 数 | 结果 | 内容 |
|---|---|---|---|
| G2-0 | 3 | 3 杀死 | 增量记账改回累计（证明计量缺陷真实存在）；CT 忽略测试私网开关；生产端点打开私网 |
| G2-1 | 13 | 13 杀死（G21-M1 首轮存活 → 补测试 → 杀死） | 页数上限、空结果报 completed、403 额度识别、401 混同、422/503 混同、额度不记入会话、remaining=0 继续翻页、Link 不查 origin、Link 不查查询词、收私有仓库、信任 html_url、不按 1000 窗口、不点名不合规名称 |
| G2-2 | 9 | 9 杀死 | 凭据发往他源、凭据头没附上、blob sha 不核、回显不脱敏、token Debug 泄露、token 允许任意字符、客户端自动跟随跳转、收私有代码命中、无凭据也发代码搜索 |
| G2-3 | 11 | 11 杀死 | 代码平台命中直接成风险、仓库来源不映射、正式开源不排除、GitHub 默认启用、空名单进冻结配置、登录名全连字符、登录名下划线、有命中+无命中聚合错、无命中算缺口、实体全称不进计划；G23-M6（禁止双连字符）原本也被杀死，但规则因真实验证改变，由 M6b/M6c 替代 |
| 前端 | 6 | 6 杀死 | 无命中显示成成功、integration_pending 显示成缺口、未配置能力显示可用、不合规账号能保存、不合规账号被丢掉（两种规则） |

日志在 Mac 的 `~/.ovi_o3/logs/g2/` 和 `~/.ovi_o3/g2/mut_g2{,b,c}.out`。

## 5. 临时库规模、性能与真实库声明

- 规模：功能性规模。每个临时库几十行；G2 生产执行器用例共 6 个，每轮向 fixture 发 1 到 6 个请求。规模三档属于 G4。
- 性能：`tests_g2` 23 个用例约 0.7 秒；exposure 全量 42–53 秒；单个 Rust 变异 12–25 秒。
- 真实库：未打开、未读写；没有新表。新的来源状态字符串只会在下一轮写入 `exposure_source_results`。
- 磁盘：见附录 B。

## 6. 四类证据

1. **真实外部来源（GitHub，匿名，只读）**：在 Mac 上跑 `g2_live_github_anonymous_repository_search_once`，查询 `"example.com" in:name,description,readme`，计量器上限 1。结果只在内存，不写任何库。

| 时间（UTC+8） | 请求 | HTTP | 记录 | 额度头 | 说明 |
|---|---|---|---|---|---|
| 00:46:22 | 1 | 200 | 29 个仓库 | search 10/分钟，剩 9 | 跳过 1 条，原因未细分 |
| 00:46:52 | 1 | 200 | 29 | 同上 | 细分后点名：`Rob--W/cors-anywhere`（旧式登录名含双连字符，是真实公开仓库）→ 判定为校验过严的真缺陷，放宽规则 |
| 00:49:11 | 1 | 200 | **30/30** | 同上 | 修复确认 |

   三次都是 `blocked_quota` / `partial`：上限只允许 1 次请求，页数上限下的尾部未读，如实报出，不是失败。真实验证覆盖了生产端点、生产解析、额度头、规范地址构造；没有覆盖分页第 2 页、错误码、代码搜索。
2. **fixture（生产执行器同一路径）**：G2-0 到 G2-4 的 23 个用例；token 防泄露全部是 fixture 证据。
3. **导入数据**：无。
4. **未验证 / 等 IR**：
   - 真实 GitHub 代码搜索和受控读取：需要 IR-04 加用户提供的凭据。
   - 正式开源名单的共享治理（IR-13）。
   - 上界是否可配置、是否需要额外的启用确认（IR-12）。

## 7. 下一阶段（G3 文档与情报入口）

1. **G3-0 文件导入（`document_import`）**：UTF-8 文本、JSON/CSV、已提取的文档文本。
   - 保存来源声明、文件 hash、批次、操作者、时间、格式；只写 exposure 观察和线索，不建资产。
   - PDF/DOCX/OCR 在能力表里列为 `unsupported`。
   - 前端上传需要新 IPC（按 IR-06 流程列出）；注册前显示 integration_pending。
2. **G3-1 指定公开文档 URL（`document_link`）**：用户明确提供 URL，经独立外联准入。
   - 复用 G2 的 `fetch_provider_api` 门（origin 白名单、私网开关、不跟随跳转）。
   - 有界读取 HTML/text；PDF/压缩包只存元数据和能力缺口。
   - “用户粘贴的 URL 是声明，不是网络验证”在证据里分开记录。
3. **G3-2 情报导入（`public_intel_import`）**：用户提供的脱敏报告，标 `claim_only` / `sample_supported` / `human_verified`；人工确认必须附可核验样本或单位关联。
4. **G3-3 分类复用**：G1-3 的招采排除和公开材料规则直接用于导入文本和指定 URL；同一原文件的转载先建 `same_document` 候选关系。
5. **G3-4 闭环与交付**：localhost 正负向、变异、真实指定 URL 的有界验证（只读、少量），按七节结构交付。
- 依赖：IR-06（导入需要新 IPC）、IR-07（人工改判）。IR-04 不阻塞 G3，但 GitHub 代码搜索的真实验收仍等 IR-04。


## 附录 A：回写文件 SHA-256（真实仓库，回写后逐字节核对与副本一致）

白名单内 70 个文件都和副本逐字节一致；有 19 个文件内容变化，外加本文。

| 文件 | 类型 | 行数 | SHA-256 前 16 位 |
|---|---|---|---|
| `docs/ASSET_EXPOSURE_GROK_INTEGRATION_REQUEST_2026-10-08.md` | 修改 | 139 | `9d8d4cb74d9b8ed9` |
| `src-tauri/src/commands/exposure.rs` | 修改 | 1241 | `dc7b74021d62c760` |
| `src-tauri/src/commands/exposure/adapters/github.rs` | 新增 | 1096 | `575ff9a84dde48d4` |
| `src-tauri/src/commands/exposure/adapters/http.rs` | 修改 | 581 | `d0d557ba3da7a864` |
| `src-tauri/src/commands/exposure/classify.rs` | 修改 | 632 | `3d9ce8741710a7c7` |
| `src-tauri/src/commands/exposure/fixture.rs` | 修改 | 240 | `e54ab58ac5b92e85` |
| `src-tauri/src/commands/exposure/orchestrator.rs` | 修改 | 863 | `47f850dc42f534c4` |
| `src-tauri/src/commands/exposure/settings.rs` | 修改 | 600 | `df3df549f1b7893b` |
| `src-tauri/src/commands/exposure/state.rs` | 修改 | 446 | `8f2017cc3a207a54` |
| `src-tauri/src/commands/exposure/tests_g1.rs` | 修改 | 1023 | `6ff7303df6602185` |
| `src-tauri/src/commands/exposure/tests_g2.rs` | 新增 | 875 | `2360751d887b8d3f` |
| `src/components/ExposureSurface.vue` | 修改 | 319 | `1a2845680131455a` |
| `src/exposure-surface.css` | 修改 | 96 | `096177adf2575e64` |
| `src/features/assets/exposure/ExposureSettingsPanel.vue` | 修改 | 123 | `b81a2aa8093a3c27` |
| `src/features/assets/exposure/contracts.ts` | 修改 | 780 | `bbf16d759c94ef2a` |
| `src/features/assets/exposure/useExposureSettings.ts` | 修改 | 266 | `36bbe0db9fa9df5f` |
| `tools/test_asset_exposure_entry.cjs` | 修改 | 302 | `c0e4a302ad9c7bba` |
| `tools/test_asset_exposure_evidence.cjs` | 修改 | 357 | `18e687827c31dc8d` |
| `tools/test_exposure_g2.cjs` | 新增 | 157 | `0ccbc6ea073ca7e6` |

## 附录 B：clippy 修复后复跑、集成复核、磁盘

- clippy：暴露面文件里的 6 个 warning 已修：settings.rs 的函数在测试模块之后；tests_g2.rs 的复杂元组类型；4 处 `&[x.clone()]` 改成 `std::slice::from_ref`。复跑 `cargo clippy --lib --tests` exit 0，暴露面 0 warning；全 lib 从 56 降到 49，剩下的都在白名单外。
- 复跑：`cargo test --lib exposure` 143 passed、1 ignored（repeat4，35 秒）。
- 集成复核：把真实仓库里 Codex 新改的白名单外文件同步进副本（lib.rs、commands.rs、agent_runtime、native_protection_recovery_priced* 等 23 个文件）后重跑 exposure 全量：143 passed、1 ignored。真实仓库回写后：node 77/77，vue-tsc exit 0。
- 磁盘：
  - 清理前：`~/.ovi_o3/spec_target` 7.5G，`~/.ovi_o3/speccopy` 38M，`~/.ovi_o3` 7.6G。
  - 清理后：先单独删副本里的 node_modules 软链接（真实 node_modules 完好），再删除副本和独立 target，`~/.ovi_o3` 剩 2.4M（脚本、日志、断点文件）。
  - 真实仓库 `src-tauri/target`（19G）没有动。
