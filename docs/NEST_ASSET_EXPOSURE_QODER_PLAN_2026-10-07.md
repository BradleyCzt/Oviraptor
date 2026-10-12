# Asset 资产暴露面现状、实施分包与 Qoder 并行交接

日期：2026-10-07。分析方式：只读核对当前未提交工作树；本次只新增本文档，没有修改业务代码、运行真实采集、打开或迁移用户数据库、删除资产，也没有提交或推送。

源码根目录：`/Users/swyiic/Desktop/Rust/Oviraptor`。核对时 HEAD 为 `14a78753b95aad65e8fd8611048e6d6ff8ddb9a2`，分支为 `nest/master-checkpoint-2026-10-05`。其他代理正在修改财务恢复代码，所以本文件是资产模块的取证快照，不是对整个工作树已封存的声明。

## 1. 能否让 Qoder 与当前 Master 开发并行

**可以，但需要按文件和运行合同分工。** 当前 Asset 公开暴露面主要是独立的确定性 HTTP 采集与资产台账，REM-A02 正在处理 Native Agent 原模型账单、显式确认、剩余预算迁移和新 Root 执行。两者有独立的可开发区域，不必因为名称里都有“暴露面”就放弃并行。

以下情况会产生功能重叠或代码冲突：

1. Qoder 同时改 `lib.rs`、`commands.rs`、`db.rs`、共享 schema、App 页面装配或全局类型，而 Master 代理也在改这些文件。
2. 把资产的被动采集接到 Native Root 的模型调用、专家派发、预算、恢复、身份会话或授权 Broker；这会进入正在开发的共享合同。
3. 重写资产“发送给调查”的草稿/启动链，或绕过当前 paid-history 恢复守卫。
4. 同一数据库里的资产归属发生变化，而运行中的任务继续沿用启动时的准入列表；即使代码文件不同，也会产生业务竞态。
5. 两个工具同时格式化全库、迁移真实数据库、运行重型 Cargo 构建或修改同一测试门禁。

因此推荐 **Qoder 首先负责 Asset 被动暴露面的真实性、分页、工作项状态与页面；Master 保留 Native Agent 执行和财务链；共享装配由主控串行集成**。文档后面的允许范围是建议分包，不代表 Qoder 已开始运行或已取得这些文件的排他权。开工前主控确认范围清单即可，不需要用户实时参与每个技术选择。

## 2. 文档依据与冲突处理

已读取并对照：

- [最新 Master Plan](/Users/swyiic/Desktop/Rust/Oviraptor/docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md) 顶部 U184 与资产/执行入口约束。
- [progress 顶部](/Users/swyiic/Desktop/Rust/Oviraptor/docs/progress.md)：U184 是生产只读新工作预览，A02/U2 仍 partial。
- [REM 续接计划](/Users/swyiic/Desktop/Rust/Oviraptor/docs/NEST_REM_CONTINUATION_PLAN_2026-10-06.md)：A02 优先，随后监督、角色、证据、并行等；不把分片通过称 Master 完成。
- [续接交接](/Users/swyiic/Desktop/Rust/Oviraptor/docs/NEST_CONTINUATION_HANDOFF_2026-10-01.md)：保留 Native JSON、保护未提交改动、真实清理先盘点备份、不删除 asset。
- [既有暴露面规格](/Users/swyiic/Desktop/Rust/Oviraptor/docs/EXPOSURE_SURFACE_DEVELOPMENT_PLAN.md)：目标是 4,000～100,000 条规模的确定性采集与少量 AI 裁决；文档状态仍为“待实施”。
- [资产草稿原子性审计](/Users/swyiic/Desktop/Rust/Oviraptor/docs/NEST_ASSET_DRAFT_ATOMICITY_AUDIT_2026-09-26.md)：已有草稿修复及其边界。
- [任务/资产/知识蓝图](/Users/swyiic/Desktop/Rust/Oviraptor/docs/NEST_EXECUTION_UI_ASSET_LEARNING_BLUEPRINT_2026-09-25.md)：资产画像、断言与知识治理属于后续合同，不因蓝图存在就算实现。

本文细化资产分包，不替代 Master 优先级，不把历史日期里的“最新/完成”直接当当前事实。既有规格中的 FOFA/规范化、探测、聚类有部分代码基础，下面逐项说明，避免从零重做。

## 3. 当前真实生产路径

### 3.1 用户操作路径

1. 选择工作空间并配置 FOFA，导入主体/域名/IP 等 seeds，启动资产任务；原生 Rust 流水线采集、URL 归并、候选分层、可选存活/内容探测并写入资产中心。
2. 在“资产”查看分页资产、探测分类、人工有效性结论；“确认有效”与“归属门禁的确认归属并授权”是两套操作，不应合并解释。
3. 在“归属门禁”维护主体、域名、地域、排除规则；自动评估只产生线索。人工“确认归属并授权”会将该项目资产设为 exposure eligible。
4. 在 Asset 下的“公开暴露面”点击当前“开始完整采集”，实际执行的是有限被动 Web 采集，加证书透明度和 Wayback 索引读取。按钮措辞超过实际覆盖范围，需要改名。
5. 查看“总览 / 采集任务 / 线索复核 / 数据源”；支持取消、人工录入、复核和 CSV 导出。
6. 资产页“发送调查”走 `create_sentinel_scan`，先创建 Web 草稿，后续确认启动走共享 Native 执行路径。它不是 `start_exposure_scan` 的另一种调用。

入口证据：[App.vue](/Users/swyiic/Desktop/Rust/Oviraptor/src/App.vue:20)、[ExposureSurface.vue](/Users/swyiic/Desktop/Rust/Oviraptor/src/components/ExposureSurface.vue:1)、[AssetWorkspace.vue](/Users/swyiic/Desktop/Rust/Oviraptor/src/components/AssetWorkspace.vue:350)、[AssetOwnershipGate.vue](/Users/swyiic/Desktop/Rust/Oviraptor/src/components/AssetOwnershipGate.vue:90)。本文没有实际打开当前安装 App，以上是生产源代码调用路径证据。

### 3.2 数据流

```text
主体/seeds/config
  → jobs.rs 的 runs 任务
  → native_fofa：分页、缓存、query_log、候选 CSV
  → native_workers：URL 归并
  → native_refinement：P1/P2/P3/Q 分类、证书/名称簇 CSV
  → native_probe：有界并行探测、输入/options 哈希、JSONL journal
  → jobs.rs::import_csv
  → assets（全局资产事实）+ project_assets（项目成员与人工有效性）
  → asset_ownership_profiles / rules / decisions（项目归属与粗粒度准入）
  → exposure_runs + exposure_source_results
  → exposure_findings（当前线索与人工复核状态）
  → 列表/CSV/人工复核

另一路：选资产 → Sentinel Web 草稿 → 显式确认启动 → Native Agent
```

当前 `exposure_findings` 没有 run_id 或不可变观察关联。重复采集按 project + fingerprint 更新同一线索的摘要与最近发现时间；不能从线索行证明某一轮到底观察了什么。全局 `assets` 与项目成员台账是不同粒度，跨项目结果、准入和证据不能只按 asset_id 归并。

### 3.3 逐模块事实表

| 模块 | 已接生产的能力 | 本次确认的限制 | 分类 |
| --- | --- | --- | --- |
| `jobs.rs` + `jobs/native_fofa.rs` | FOFA 预检、分页、查询日志、缓存、候选持久化；失败查询保留候选且拒绝把不完整采集当完成 | 并不等于 Hunter/Quake/搜索引擎已经支持；外部账号/真实配额未实测 | 真实功能，代码证据 |
| `jobs/native_workers.rs` | URL 归并、原生优化、候选分层调用 | 已有规范化，不能建议另建一个互不兼容的 canonical key | 真实功能，代码证据 |
| `jobs/native_refinement.rs` | 确定性分层与 `certificate_clusters.csv`、`name_domain_clusters.csv` | 属于本次资产流水线 CSV 聚合；不是持久数据库簇、代表资产、成员继承与跨轮缓存 | 真实功能 + 规模合同未完成 |
| `jobs/native_probe.rs` / `native_probe_control.rs` | 有界 worker、体积上限、速率/每主机间隔、取消；完整输入/options 指纹的 journal 复用 | 不等于暴露面 run 有 checkpoint；不能把两套恢复机制混为一谈 | 真实功能，代码/夹具断言证据 |
| `commands/assets.rs::list_assets` | 项目/归属/探测/人工结论筛选、总数、服务端分页，pageSize 限 10～200 | `ExposureSurface` 请求 500 仍只取得一页最多 200；绑定资产下拉不是全量选取 | 真实功能，分页已有 |
| `commands/asset_ownership.rs` | 主体档案、规则、自动归属、人工确认、归属事件 | 人工 `confirmed` 同时写 allowed + eligible；无独立 scope/方法/时间窗/撤销版本合同 | 真实粗粒度准入，细粒度授权未完成 |
| `commands/exposure.rs` | 有生产 IPC、独立线程、匿名 GET、自然资源引用、Source Map/疑似凭据/仓库/存储/文档线索、CT/Wayback | 200 资产、20 成功资源等硬截断；无完整工作项、覆盖台账、恢复；跨来源终态不可靠聚合 | 真实有限被动采集 |
| `commands/exposure.rs` 的 list/review/export | 线索去重、状态复核、导出、来源状态、取消请求 | findings 静默 500，runs 最近 20；复核只是改 status/note，无整改/复测链 | 真实功能，产品闭环未完成 |
| `db_initialize_columns.rs` | 启动时把未结束 exposure run 标为 interrupted，并保留线索 | 不是从工作项继续；原线程已丢失，提示只能重新开始 | 真实启动中断归档 |
| `ExposureSurface.vue` | 正式页面/API，2.5 秒轮询活动任务 | 无请求代次隔离，切项目/交叠 load 可能晚到覆盖；sources 固定最近 run；“完整采集”误导 | 真实 UI，存在待补竞态 |
| `scan_lifecycle.rs::create_sentinel_asset_scan_in` | IMMEDIATE 草稿事务、去重、项目/移出/熔断过滤、策略/目标持久化后验、Native mode 登记 | 不检查 ownership confirmed/authorization allowed；只是草稿，不是主动执行授权 | 真实草稿功能，勿重复修原子性 |
| Native `multi_agent_external_surface.rs` | 有单匿名入口 GET、原能力/预算/捕获/证据合同实现及测试 | Root `coordinator_external_surface_gate.rs` 仍 not_ready，prepare flag=false；不是 Asset 完整流水线 | 实现原语/测试证据 + Root 派发骨架 |

关键源文件：[exposure.rs](/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/src/commands/exposure.rs)、[asset_ownership.rs](/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/src/commands/asset_ownership.rs)、[assets.rs](/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/src/commands/assets.rs)、[资产 API](/Users/swyiic/Desktop/Rust/Oviraptor/src/features/assets/api.ts)、[schema](/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/src/db_schema.rs:694)、[原生资产流水线](/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/src/jobs.rs:738)。

## 4. 当前缺口，按影响排序

### 4.1 覆盖范围与完成状态不真实

- 暴露面选择资产的 SQL 在 `exposure.rs:152` 静默 `LIMIT 200`，没有剩余对象工作项；却把选出的长度写成 eligible_assets，隐藏全量差额。
- `list_exposure_findings` 静默 `LIMIT 500`；`list_exposure_runs` 最近 20；返回值没有 cursor/total/truncated。
- 每资产 `resource_count >= 20` 按成功读取次数止步；visited+queue 上限 40；既未记录 budget-exhausted，也未区分本轮漏掉的 HTML/JS/Map。请求失败并不占 resource_count，预算不能只按成功响应计数。
- CT 只查询最多 20 个 approved domains，统计域名数后丢弃发现记录；没有回流 `assets/project_assets` 或 pending ownership。
- Wayback 每域 `limit=200`，只取最多 20 域，没有游标/截断原因；“历史高价值路径”只是线索，不能代表当前可访问。
- CT/Wayback 返回非预期 JSON 时没有显式 parse-error；source 可能仍 completed。
- final run status 主要依据被动 Web 的 failures 与选中资产数，不汇总 CT/Wayback partial；少量失败也可能 completed。run 的 stage 固定 completed 还可能搭配 partial。
- schema test 名称含 `complete_exposure_workflow`，实际只检查列和 source 表；不能作为全流程已完成证据。

### 4.2 授权边界与项目归属需要明确合同

- UI 的人工按钮已经明确写“确认归属并授权”，所以当前不是隐蔽自动授权。但同一 confirmed 状态覆盖了两个维度，无法表达“确属本公司、暂未授权”“仅此 origin 可 GET”“批准到期/撤销”。
- 采集只在读初始列表时检查 eligible + project_assets 未移出，没有每次目标请求重验项目状态、归属版本、撤销、期限和范围。
- 请求前比较 host/子域；client 自动跟随最多 3 次 redirect，没有逐跳验证 Location 的 origin/协议/端口/解析地址。原 host 被批准不能推出任意子域、端口或重定向目标同样批准。
- 初始 URL 允许 HTTP(S)，没有显式拒绝 URL userinfo，也没有这个暴露面通道的私有/回环/link-local 出口策略。Asset Native probe 有自己的 allow_private/探测规则，不能据此认为 exposure client 已受同一保护。
- save finding 对 asset 只查 eligible；不同时检查 active project 与未移出的项目成员。update ownership 接收 selections 时也没有明确逐项成员验证。review/cancel/sources 以 id/run_id 定位，没有 expectedProjectId 的二次 scope 校验。
- 这些是源码可见的合同缺口；本次没有在真实服务上尝试越界，也没有声称已完成多租户权限测试。需要用临时项目、localhost 和明确授权 fixture 验证。

### 4.3 执行可靠性与成本

- `start_exposure_scan` 的 count→insert→spawn 非同一 IMMEDIATE 认领，没有针对活跃任务的唯一约束；并发启动可能产生两条 run。
- 只有线程内计数与 source 汇总，没有逐项 claim/lease、可恢复 cursor、attempt、owner/fence、去重效果凭据；应用重启会 interrupted，但没有 resume exposure API。
- 取消在资源/域名边界读取 cancel_requested；正在 send 的请求受 12 秒 timeout 约束，没有即时终止语义。已发现结果保留是现有功能，应继续保留。
- deterministic 暴露面目前未调用模型，优点是零 Token；没有总请求/总字节/每 origin/每 source 配额和准确 coverage。不能因为零 Token 就称低成本完整。
- 资产 `resume_job` 仅允许 reprobe，并读取当前原配置方案；不能直接拿来承诺暴露面使用冻结原计划恢复。

### 4.4 证据与线索质量

- Source Map 依据公开成功响应、路径 `.map` 标高风险，没有解析 JSON version/sources/sourcesContent 来区分真实业务源码、空图、错误页或第三方公开包。
- 凭据只检查部分字段词、排除 placeholder、保存前三后两字符；这是候选，不证明有效凭据，不得自动调用密钥验证服务。
- 外链仓库/对象存储只做记录，尚未拉取仓库、枚举目录、检查历史或确认归属。文件名含内部语义也不等于文档内容泄漏。
- fingerprint 把整个 URL 小写；URL path/query 大小写可能有语义，存在错误归并。相同 project/category/url/title 跨资产命中也缺少独立 observation 关系。
- 人工录入 evidence_excerpt 只截长，没有后端统一脱敏；导出所有摘要，不应依赖“不要粘贴完整密钥”的提示。CSV 输出也需定义公式字段转义策略。
- response headers/hash/final URL/内容 hash/定位/规则版本/本轮 provenance 没有完整不可变证据包；现在没有被动 response 原文资产清单可以“顺便清理”。
- 每次 upsert 可以更新 evidence/severity/confidence，但已 confirmed/dismissed 的人工状态仍保留；新内容变化后应明确“需复核/上一结论”，不能静默沿用旧人工结论。

## 5. 第一阶段验收目标

首个可交付闭环限定为：**显式启动已授权的被动暴露面任务 → 全计划工作项分页 → 有界匿名采集 → 各来源真实终态/缺口 → 可取消/重启恢复 → 脱敏证据与人工复核 → 分页 UI。**

这一阶段不调用模型，不启用 Native ExternalSurface Root 派发，不执行端口扫描、路径暴力枚举、登录身份复用、上传或受控写，不重做 FOFA/probe；它是资产侧闭环，不是 REM-A02 或整个 Master 完成。

“completed”需要所有计划项和计划来源都有终态且无未解释缺口；达到配额、未配置、停止、失败与待授权分别列出原因。计划未包含的能力仍显示未计划/未支持，不能从“本次任务完成”推导“互联网没有暴露”。

## 6. 分包实施卡与验收场景

下列新目录/文件是建议创建的位置，当前不存在，不是已有生产能力。修改既有共享文件只提供精确补丁，由主控集成。每卡必须报告真实功能、骨架、只读诊断、测试证据四类。

### Q0 — 页面真实性与旧接口限制可见（先做，可独立并行）

范围：`src/components/ExposureSurface.vue`、`src/features/assets/exposure/` 新组件/composable/contracts、必要的暴露面 CSS。保留 `src/features/assets/api.ts` 门面，新增调用需主控确认对应 IPC。

工作：按钮改“开始被动采集”；显示当前采集范围/静默截断的已知限制；把 subjects/历史线索与绑定资产观察分开；允许选择 run 查 sources；generation + capturedProjectId/runId 控制晚到结果，隐藏/卸载停止轮询，刷新失败保留当前可信快照并标 stale。绑定资产选择使用真实分页，不请求 500 后假定拿全量。

依赖：不需要新 Root/模型/财务接口。不得编造 backend total/truncated；旧接口未提供总数时显示“最多前 500 / 最近 20”，新 IPC 集成后替换真实分页信息。

正向：A 项目→B 项目，两个真实组件请求以相反顺序返回，最终全表/summary/sources/form 都属于 B；切历史 run 得到对应 sources；选择第 201 个资产通过分页完成。

负向：旧项目晚到、卸载后回调、重叠轮询、IPC 失败、源响应损坏不覆盖当前数据、不触发采集；点击 fee confirmation 或 Native reconciliation preview 不触发 Asset run。新增真实 Vue 组件/模拟 IPC 测试，不只 source-string pin。

### Q1 — 纯合同与只读 Repository 分页（可与 Master 并行）

范围：`src-tauri/src/commands/exposure/contract.rs`、`repository.rs`、`summary.rs`、同目录临时库 tests；`exposure.rs` 保持命令门面，避免新增大段实现。

工作：设计 findings/runs/work-items 的稳定游标和真实 total/hasMore，固定排序必须有 id tie-break；summary 与实际 active membership、eligible scope 使用同一条件；review/cancel/source 查询接受 expectedProjectId/version 并由后端验证实际所属。未知 enum、非法页长、损坏 cursor 明确拒绝，不把非法条件变空串放行。

依赖：新类型优先放独立 Rust/TS contract 叶子，避免同时改 `models_appsec.rs`/`src/types.ts`。旧调用保留短期门面或由主控一次性更新，不能偷偷改变旧 return shape。

正向：201/501 条边界、相同 last_seen/confidence 的稳定分页、筛选后总数、跨页无重复/遗漏；4,000 条临时资产分页到底。

负向：其他 project 的 finding/run ID、已归档项目、已移出资产、超长页、非法 cursor、并发 review 版本冲突；失败零写。只读查询不得创建 run 或发 HTTP/SDK。

### Q2 — 被动授权与网络出口合同（Qoder asset 叶子，授权共享接口主控集成）

范围：新增 `exposure/scope.rs`、`adapters/http.rs`、`redaction.rs`；局部修改 `asset_ownership.rs` 需主控先分配，首片不动 Agent Broker。

工作：独立表达 ownership 与 exposure authorization；冻结 project、asset membership、origin/method、来源、授权版本、有效期和请求/字节预算。范围明确到 origin 与规则，DNS 后的地址策略也明确；userinfo、非 HTTP(S)、未授权端口/子域/重定向、私有地址默认拒绝，确需内部资产时使用显式配置与批准。关闭自动 redirect 后逐跳检查，跨范围停止并留观察；每次请求前复核撤销/expiry/project 状态。公共 CT/Wayback 是 source endpoint，不能因此授权新发现目标的网络访问。

现有“确认归属并授权”意图保留为 coarse legacy 记录；不批量把旧 allowed 翻成新主动扫描许可。旧 rows 无细粒度合同时显示 legacy coarse scope，必要时要求重新确认具体新操作。授权撤销不删除资产/原证据。

正向：授权 localhost 测试服务、同范围 GET、批准的 redirect 链；允许内部出口的专用 fixture；合法外部索引结果仅入 pending ownership。

负向：请求期间撤销、项目归档、scope 到期、redirect 到不同 port/host/private/link-local、DNS 结果越界、含 userinfo URL、猜测 sibling subdomain。统计真实出站请求，必须在越界 send 前拒绝；取消后的新请求为零。普通 401/403 保存权限边界，明确 challenge/持续限流触发目标停止，不写“安全”。

### Q3 — 持久工作项、原子启动与恢复（先做合同，schema 串行集成）

范围：新 `exposure/orchestrator.rs`、`state.rs`、`work_items.rs`、`schema.sql`、`recovery.rs`；共享 schema/启动装配由主控写。不要改 Native supervisor、RootOwner 或 A02 budget。

建议数据模型：`exposure_jobs`（或版本化扩展 exposure_runs）、`exposure_job_stages`、`exposure_work_items`、`exposure_observations`、`exposure_scope_receipts`。表名最终由主控确认；每项包含 project/run/attempt、asset 或 entity、source、稳定 work key、冻结输入 hash、owner/claim version、状态、原因、游标与计数。新表只服务 Asset，不能写 Agent 财务行。

工作：IMMEDIATE 原子 count/claim/insert + 活跃项目唯一约束；冻结计划资产快照和来源列表，稳定 keyset 分页创建/领取；HTTP 每次尝试包括失败都收费于确定性配额，响应字节也有硬顶；独立 source/quota/rate，单来源失败隔离。事务只做短 DB 操作，不在 SQLite writer transaction 内等待网络。取消、超额、未配置、parse-error、重启孤儿处理明确；重复 start key 返回同一 run；失去 owner 的 worker 不写终态。

恢复：冻结 plan/options 不匹配拒绝复用；已保存确定性观察不重抓，未完成可有界重试，带版本/来源/时间；应用启动只把无法证明仍运行的 owner 标 interrupted，不让 initialize 重写当前活跃 worker。旧 run 保持 `legacy_passive_web` 的投影/provenance，不批量改历史 Native JSON，也不宣称旧历史已覆盖新计划。

正向：4,000 个计划资产分批到终态；零 AI 配额仍跑完；kill 测试子进程后继续，已闭合 work key 不重复；同源有限并发与速率；两请求并发 start 只产生同一活跃 run。

负向：claim 后崩溃、response 保存前后崩溃、DB busy、IGNORE/ABORT/ROLLBACK 故障、snapshot/options/授权变更、取消与 late response 竞争、source parse-error/配额 exhaustion。partial 必须给具体未完项，不能只写 error 字符串或靠线程正常退出判断完成。

### Q4 — CT/Wayback 回流与真实状态聚合

范围：`exposure/adapters/certificate_transparency.rs`、`web_archive.rs`、`observations.rs`、`aggregation.rs`。复用已存在的规范化和项目台账语义，不改 `jobs.rs` 的全局 import 原理。

工作：CT 每条域名保存 source observation/query/time/canonical key，与 assets/project_assets 建立引用或待归并队列；新域名从 unreviewed/unknown 开始，不能自动 eligible。Wayback 保存 indexed_at/currentAccessible 等独立维度，明确索引限制/分页。每个 source 枚举 completed/partial/failed/cancelled/unconfigured/unsupported 与精确 reason；run 汇总计划来源和 work-items，CT failure 不被 Web success 掩盖。

正向：CT 同域/跨轮重复去重但 provenance 保留；同名跨项目独立授权；Wayback 已下线资源仍是历史线索；无 eligible 资产但主体公共索引有计划时也能明确执行范围。

负向：坏 JSON、截断、HTTP 429、空域名、* wildcard、无授权的新域名、混入其他根域、撤销源配置；未配置显示未配置，不能 completed。真实 fixture 没有命中不说明数据源整体有效。

### Q5 — 证据、规则与人工结论版本

范围：`exposure/evidence.rs`、`rules/`、`fingerprint.rs`、`export.rs` 与 evidence detail 组件。

工作：分离 immutable observation 与 current finding；包含 run/source/request/final URL/响应状态、内容/规则 hash、定位、截断、脱敏策略、直接观察/继承来源。URL 只规范化 scheme/host/default port 等已定义部分，保留 path/query 语义；所有后端入口统一脱敏，日志/错误/CSV/预览同样处理。Source Map 正式解析，凭据区分假值/候选/人工结论，不做密钥试用；仓库/存储链接保留“仅引用”标签。新内容/规则变化产生版本和复核需要，旧人工结论保留在历史。

依赖：证据文件若使用 CAS/Native artifact，需要主控提供只新增/只读引用合同；首片可先用 Asset 独立脱敏证据对象。不能把旧 Asset 摘要贴成 Native reviewer 已验证证据。

正向：真实 minimal sourcemap 有 sourcesContent，可定位原代码；空/第三方包降为明确线索；多资产同 URL 各 observation，重复采集不重复 finding；手工 confirmed 后新hash变化标需复核。

负向：HTML 错误页伪 `.map`、placeholder、大小写不同 path、输入粘贴 Bearer/secret、CSV `= + - @` 开头字段、证据超大/损坏/哈希不符、跨项目证据引用。导出和 API 返回不得泄漏完整秘密。

### Q6 — 持久簇、内容缓存与规模验收

范围：新增 `exposure/clusters.rs`、`cache.rs`、`cluster_repository.rs`、簇页面。保留 native_refinement 现有 CSV 输出；将它作为观察输入，不重复实现另一套证书匹配规则。

工作：持久 cluster/member/representative 和规则版本，缓存 key 绑定 project/scope/privacy/content/rule/version；证据继承必须注明 inherited_from_asset_id、直接验证范围与时间。同 IP/CDN/域名簇不能推出同 ownership/授权，聚类不删除资产。

正向：同 hash 同合同一次分析，成员保持完整；增量内容变化只使对应缓存失效；4,000 与 20,000 分页/工作项统计真实，后续 100,000 性能单独记录环境与指标。

负向：跨项目、跨授权、代表被撤销、成员内容变化、共享 CDN/IP、多租户页面不同；不能复用不同授权的证据，也不能把继承结论显示为直接观察。

### Q7 — AI 单次裁决、主动扩展与整改（后续，先不并行改共享执行）

AI：只接受有界脱敏证据包，默认单次裁决 + 一次技术失败重试、缓存、明确候选输入；deterministic 不依赖 AI。先与 Master 确认 SDK/模型账单/原授权/十维预算的新合同，再接适配器。**当前禁止复制一套 reqwest 模型客户端绕开 A02 或翻 ExternalSurface prepare flag。**预算不足只暂停 AI 队列，确定性阶段继续。

主动扩展：端口/目录、文档下载解析/OCR、仓库历史、网盘/社交数据源按既有规格逐 source 加明确授权和配额；缺依赖标未配置。不能把自然链接采集升级成无界 crawler 或 directory brute force。

整改：新增责任人、期限、工单、人工处理、复测关联、复开/关闭历史；当前 `review_exposure_finding` confirmed/dismissed 只是复核结论，不代表整改关闭。复测必须新观察或精确引用可用凭据，不能仅靠前端改 status。

验收：AI SDK 超时/已付未结算不重复发送；授权撤销拒绝主动触碰；回滚/预算耗尽保留原证据；关闭后复测再次命中自动待复核；本卡不在首阶段独立范围，需主控串行接线。

## 7. 当前 Qoder 文件所有权建议

### 7.1 首批可分配的独立范围

- 新增 `src-tauri/src/commands/exposure/`：contract/repository/state/evidence/rules/adapters/fixtures，只写 Asset 表，不引用 Native Root mutation API。
- 既有 `src-tauri/src/commands/exposure.rs` 的门面与局部拆分；该文件当前没有 Master 财务改动，是适合分包的入口。
- 新增 `src/features/assets/exposure/`：独立合同、composable、页面组件。
- `src/components/ExposureSurface.vue` 与 `src/exposure-surface.css`、`src/exposure-runs.css`、`src/exposure-tabs.css`。
- 新增专属 `tools/test_asset_exposure_*.cjs` 和 Asset fixture；测试正常门禁注册交给主控，不能删/skip 原测试。
- 新增 Asset schema SQL 和迁移说明，主控审查后装配到隔离测试库，再集成生产初始化。

`src/features/assets/api.ts` 是 Asset feature 门面，可在主控确认没有另一任务占用后分给 Qoder。`asset_ownership.rs`、`models_asset.rs`、`models_appsec.rs` 的授权/类型增量属于第二批，首片先提供合同/补丁，不自动扩范围。

### 7.2 禁止与 Master 同时编辑的共享区

| 区域 | 当前所有者/原因 | Qoder 做法 |
| --- | --- | --- |
| `src-tauri/src/lib.rs`、`commands.rs`、`db.rs`、`db_schema.rs`、`db_initialize_columns.rs` | 共享 IPC/模块/启动/迁移 | 给精确 insertion patch 与 test fixture，由主控串行集成 |
| `src-tauri/src/commands/native_financial_continuation*`、`native_web_resume_finance.rs`、`native_scan_branches.rs` | 主控 A02 发行/激活/恢复守卫 | 完全不改，不加 bypass |
| `agent_runtime/store.rs`、`store/financial_continuation.rs`、`web_mode.rs`、`web_mode/` 财务出版/签名 | 主控新 Root 出生证明 | 完全不改 |
| `multi_agent/budget/root_definition.rs`、`root/shared.rs`、`limits/contract.rs`、`budget/schema.rs`、`continuation/migration*` | 财务代理精确余额迁移 | 完全不改；Asset 确定性配额用独立合同 |
| `commands/agent_native/coordinator_common.rs`、`coordinator_basis.rs`、`coordinator_changed_request.rs`、`coordinator_evidence.rs` | worker 代理实际新工作请求/证据 | 完全不改 |
| `budget/root/model/tick.rs` 及 purpose/projection 相关叶子、`native_financial_continuation_worker.rs` | 原账单/派发幂等与 actual SDK | 完全不改 |
| `agent_runtime/multi_agent/` 其他调度/lease/supervisor/capability、`commands/multi_agent_external_surface.rs`、`coordinator_external_surface_gate.rs` | Native 专家与 Root 派发合同 | 不翻旗、不复制预算、不接资产主动权限 |
| `scan_startup.rs`、`scan_lifecycle.rs`、`scan_quiescence.rs`、任务 pause/resume/delete/closure | 共享生命周期/保护原付费调用 | 不把 Asset 状态代入 scan attempt，不改 paid-history guard |
| `src/features/sentinel/`，特别 `NativeRootReconciliation.vue` / reconciliation | 主控独立用户批准 UI | 不改，不把 Asset 开始按钮绑定到 fee confirmation |
| `src/App.vue`、`src/api.ts`、`src/types.ts`、`package.json`、锁文件、全局测试配置/allowlist | 共享页面装配/类型/门禁 | 增量建议串行集成；不运行全库格式化、不放宽 guard |
| `jobs.rs`、`jobs/native_workers.rs`、`asset_logs.rs` | 已有未提交改动/其他资产功能 | 首片只读复用；需要改时重新分配确切函数/文件 |
| Master/progress/REM/交接文档 | 主控负责统一阶段事实 | Qoder 只写资产专项记录，主控合并总体进度 |

新目录里的 pure helper 可以共享概念，不能重复实现“授权服务/模型费用账本/Native JSON 导入器”。若必须编辑禁碰文件，暂停该文件的改动，给主控一个小补丁；不需要暂停其他独立 Asset 卡。

## 8. 并行操作与集成规程

1. Qoder 开始时确认真实源码路径、分支/HEAD、dirty 清单和本卡文件哈希；已有 dirty 不等于可覆盖。先读文件/diff，新增文件确认不存在。
2. 当前多个代理共享同一目录；同文件即使改不同行也可能受格式化、include、测试 guard 影响。建议 Qoder 使用主控安排的独立 checkout/worktree，且必须包含或精确移植当前未提交依赖，不能只从旧 HEAD 自行重建。不得重置主工作树。
3. 同目录方案需要独占文件清单，Qoder 只输出这些文件的增量。其他代理变化不能被 Qoder 的备份“恢复”掉。
4. 先冻结接口 shape/错误码/状态图，再写 pure tests，再装配 DB/IPC，最后 UI 调实际接口。交付 schema-only 时标“骨架”，不能对用户说恢复已经可用。
5. 重型 Rust 构建和全量 UI 门禁由主控排队；Qoder 可跑独立纯函数/组件测试，但不要与主控同时启动 Cargo。按项目现行 `nice -n 15`、`-j 1`、`--test-threads=1` 约定；不 `cargo clean`。
6. 每片交付：精确文件表、before/after hash、增量 diff、接口合同、scope 内 whitespace/fmt、真实测试日志与 exit code、未测项。不能把长历史的通过数加成当前全量结果。
7. 主控集成顺序：独立 backend leaves → shared schema/IPC → strict feature contract → Vue → 聚焦回归 → 必要的 normal 门禁。遇到现有 legacy v2/Clippy 债如实区分，不删测试、不改 allowlist 掩盖。
8. 不自动 commit/push，不让 Qoder 联系服务商/发消息，不启动真实 FOFA/CT/Wayback/目标采集作为“自动测试”。先 localhost、临时 SQLite 和离线 fixture；真实数据源验收单独列范围和账号配置。

## 9. 真实资产数据盘点与备份规则

本次没有读取真实 asset 数量或用户数据内容。源码默认数据目录为 `~/oviraptor`，当前用户按该默认路径对应 `/Users/swyiic/oviraptor/oviraptor.sqlite3`；导出默认是用户 Downloads 下的 oviraptor。**这些是源码推导的候选位置，不是已核验当前安装态实际文件。**验收模式、迁移目录或不同安装可能使用其他位置。

真实操作前：

1. 核实当前运行 AppState 的 app_data_dir/db_path/export_dir，盘点 SQLite/WAL/SHM、run output_dir、FOFA cache、probe journal、导出、证据文件及 CAS 引用；不凭目录名称判定“旧数据”。
2. 用 `mode=ro`/只读连接读取表数量、项目归属、共享 asset 引用、dangling references 与 schema；不要调用 `db::initialize` 做只读盘点，因为它会迁移和更新 interrupted 状态。
3. 盘点至少涵盖 `projects/targets/runs/assets/project_assets/asset_events/asset_ownership_profiles/rules/decisions/exposure_runs/source_results/findings`，以及 Sentinel targets 与 Native evidence/artifact 的共享引用。当前 finding 没有 run_id 的事实要保留在迁移映射中。
4. 活跃 SQLite 使用一致性 online backup，或在已确认停止所有 writers 后备份数据库与必要辅助文件；不能只复制主 db 忽略 WAL。备份目录、原 schema、行数/哈希、文件清单、恢复说明与在临时路径的恢复演练齐备。
5. 新 schema 先在副本迁移；旧表保留、旧记录保持原字节，新增 provenance/projection 可做 sidecar。任何历史转换都可重入、可报告失败，不静默丢字段或贴成新 Native 证据。
6. **不删除 asset，也不批量软删除来充当迁移。** 不清空 project_assets、不删全局 assets、不递归删除 run/CAS/cache；授权撤销用状态/version，聚类用引用，历史中断用任务状态。
7. 真正需要清理某种非资产历史材料时另给精确逐行/逐文件预览、共享引用、可恢复备份和恢复路径，依据已有授权边界处理；本文的分析授权不包含直接真实清理。

## 10. 测试证据现状与后续质量门禁

本次只读对照了测试源码，**没有重新运行 Asset 测试或 cargo/npm 全套**，没有把 U184 的 UI 862 或历史 Rust 2624/1 当 Asset 全链通过。

可复用的现有证据：

- `commands/exposure.rs::exposure_tests` 两项：自然资源/Source Map 引用解析、凭据片段脱敏与 placeholder。它们不测 HTTP、run 状态、分页、授权撤销、并发或恢复。
- `asset_ownership.rs::asset_ownership_tests` 四项：DNS label 边界、域名规范化、名称噪声、ASN/CIDR 弱证据；不是人工授权状态机端到端。
- `db_tests.rs::initializes_complete_exposure_workflow_schema`：检查 exposure stage/current_source/cancel_requested 和 source 表，仅 schema 证据。
- `jobs/native_workers.rs` 已有 FOFA plan/归并/分层、探测网络 fixture、取消后 journal 继续等测试；这是 Asset inventory/probe 的证据，不是 exposure resume。
- `commands/tests_asset_draft.rs` 已有五项草稿成功/IGNORE与ABORT/AFTER篡改/错误输入/并发归档回归；历史审计记录通过，但本次未重跑。
- `tools/test_asset_log_panel.cjs` 与 `asset_logs_tests.rs` 覆盖 committed log 快照/项目切换/晚到/脱敏/隐藏等；不证明 `ExposureSurface` 已有相同竞态保护。
- Native `agent_tests_external_surface.rs` 等验证专家原 capture 合同，不能替代 Asset 被动流水线测试。

建议门禁矩阵：

| 门禁 | 最低内容 | 通过后的结论边界 |
| --- | --- | --- |
| pure contract/rules | enum、URL/origin、fingerprint、脱敏、scope/期限、状态聚合 | 规则和合同实现成立 |
| 临时 SQLite | 原子 start、分页到底、项目隔离、版本冲突、失败零写、观察/线索引用 | 持久化链成立 |
| 实际 localhost HTTP | 成功/坏 JSON/redirect/取消/超限/限流/撤销；请求次数与字节计量 | 被动网络生产路径可用 |
| 测试子进程崩溃 | claim/send/save/aggregate 边界，重新打开库恢复 | 指定恢复边界成立 |
| 实际 Vue + 模拟 IPC | 项目/run 切换、晚到、错误保留、分页/复核、无自动采集 | UI 合同和竞态成立 |
| 隔离桌面 App | 同源构建、真实页面路径与临时数据库 | 当前安装交互可用 |
| 用户指定真实数据源 | 明确账号/范围/配额/备份，单独记录结果 | 该数据源该范围实测，不外推全部 |

阶段完成必须给正向与负向日志、不可解释失败清单、总计划/终态/缺口数量及 schema/fixture/安装/真实目标的不同证据级别。没有单个“测试很多，所以全流程完成”的结论。

## 11. 建议实施顺序与 Qoder 首条指令

顺序：Q0 → Q1 → Q2/Q3 合同 → 主控 schema/IPC 集成 → Q4/Q5 → 恢复/并发/4,000 规模验收 → Q6 → Q7 新合同。Q2 与 Q3 可以由 Qoder 内部分工，但同一 schema/repository mutation 文件仍需单所有者。

第一片建议 Q0 + Q1 中的只读分页合同/测试，范围小、能立刻减少误导并提升可用性，也不会干扰 A02 付费恢复。它完成后再把新的持久任务合同交主控评审和集成，不要一口气写所有适配器。

可直接交给 Qoder：

> 继续 `/Users/swyiic/Desktop/Rust/Oviraptor` 的 Asset 被动公开暴露面。先读 `docs/NEST_ASSET_EXPOSURE_QODER_PLAN_2026-10-07.md`、最新 Master/进度与现有 `EXPOSURE_SURFACE_DEVELOPMENT_PLAN.md`，按本文 Q0/Q1 首片和主控分配的独占文件实现。当前 Master 在开发 REM-A02，禁止编辑本文共享禁碰区、Native Root/SDK/预算/恢复/授权 Broker，不翻 ExternalSurface prepare flag。保护未提交改动、Native JSON 和全部 asset，不 reset/clean/批量覆盖/全库格式化/自动 commit/push，不操作真实数据库或发真实采集请求。只读 helper/状态机骨架不能算功能闭环。共享 schema/IPC/types/test registration 输出精确补丁交主控串行集成；测试用临时 SQLite、localhost 和真实组件模拟 IPC，重型 Cargo 由主控排队。交付逐文件增量、正反测试日志、事实分类和剩余缺口，不重做已有 FOFA 分页缓存、probe journal、资产分页、日志或草稿原子性。

本报告的结论是“存在可安全分包的独立范围”，不是“任何两套开发工具都绝不会冲突”。按上述首片和文件所有权做可以并行推进；需要共享执行合同的后续工作由主控协调，届时再接线。

## 附录：本报告核对的关键 Asset 文件快照

以下 SHA256 在报告生成时从当前文件读取，供 Qoder 开工前复核漂移；不代表这些文件已提交，也不代表真实安装包使用同一源码。共享文件正在其他代理开发，不列为冻结资产范围。

| 文件（相对上述源码根目录） | SHA256 |
| --- | --- |
| `src-tauri/src/commands/exposure.rs` | `645ec7b77a197225f5cc3d9b965c2547f4f8d8a6ebe543c74d1a8c4b054e2e36` |
| `src-tauri/src/commands/asset_ownership.rs` | `0a372d9ccb224972bff4c54ed8acbfa186e206c20ad9eae1f2182559b8db5b4a` |
| `src/features/assets/api.ts` | `ce99ea70d5792ed5ece824a70af8c0b10a7d695dd0a5710ce580eb97b357035f` |
| `src/components/ExposureSurface.vue` | `c3f77e39b13fe642a4b924b5408b039cb46be955d36e6ccf030b5c61abd74b6b` |
| `src-tauri/src/jobs/native_probe.rs` | `7117aae4e104e9470b2659571280ebcee3c6bcfec2a3c8b41e3f7131d373c8b8` |
| `src-tauri/src/jobs/native_fofa.rs` | `f5982a6c2d86be205604e1d978553ff6219ef41316277711626e421b8839a8d3` |
